//! [`InstallSession`]: what an install or an uninstall carries from its first
//! package to its last.
//!
//! One HTTP client and cache for the downloads, one VI Server connection for
//! the hooks, and the work that waits for the end: the folders to relink and
//! the PostInstall hooks to run once the files are in place. [`finish`] spends
//! it, and an uninstall feeds the same session removals instead of installs.
//!
//! [`finish`]: super::install::finish

use super::env::{Destination, OpEnv};
use super::hooks::{HookCall, PostInstallHook, run_reported_hook};
use super::resave::Resaver;
use super::staging::Staging;
use crate::cache::{Cache, Stored};
use crate::cancel::CancelToken;
use crate::fetch;
use crate::github::{self, GitHubAuth};
use crate::index::{self, Entry};
use crate::target::{LvTarget, Roots};
use crate::venv::Venv;
use crate::viserver::Connection;
use anyhow::Result;
use reqwest::blocking::Client;
use std::path::PathBuf;

/// What fetching an archive needs, opened when the first archive is wanted:
/// an uninstall never gets here. Locking has no destination and so no session;
/// it fetches through the same thing, as `info` does on a cache miss.
pub(super) struct Downloads {
    client: Client,
    cache: Cache,
    cancel: CancelToken,
    github: GitHubAuth,
}

impl Downloads {
    pub(super) fn open(env: &OpEnv) -> Result<Downloads> {
        Ok(Downloads {
            client: Client::builder()
                .user_agent(concat!("lvpm/", env!("CARGO_PKG_VERSION")))
                .build()?,
            cache: Cache::open(&env.config.cache)?,
            cancel: env.cancel.clone(),
            github: env.github.auth.clone(),
        })
    }

    /// The archive's bytes, from the cache, a local file or the entry's feed.
    /// A GitHub entry is sent the GitHub token; no other entry is. When the
    /// environment's token is cancelled, [`Cancelled`](crate::errors::Cancelled).
    pub(super) fn fetch(&self, entry: &Entry) -> Result<Vec<u8>> {
        let repo = index::github_repo(entry);
        let token = || repo.and_then(|_| self.github.token());
        let bytes = fetch::package(&self.cache, &self.client, entry, token, &self.cancel);
        match repo {
            Some(repo) => {
                bytes.map_err(|why| github::download_error(why, &entry.name, repo, &self.github))
            }
            None => bytes,
        }
    }

    /// The releases of a GitHub dependency's repo, from the cache unless
    /// `refresh` says to ask GitHub; see [`github::releases_cached`].
    pub(super) fn releases(
        &self,
        api: &str,
        repo: &str,
        name: &str,
        refresh: bool,
    ) -> Result<github::Listing> {
        let (cache, client, auth) = (&self.cache, &self.client, &self.github);
        github::releases_cached(cache, client, api, repo, name, auth, refresh)
    }

    /// What the cache holds under this SHA-256, without removing anything.
    pub(super) fn stored(&self, sha256: &str) -> Stored {
        self.cache.lookup(sha256)
    }

    /// Keep bytes in the cache under their own SHA-256, so that a later read
    /// of that SHA-256 finds them.
    pub(super) fn keep(&self, bytes: &[u8]) -> Result<String> {
        self.cache.store(bytes)
    }
}

pub struct InstallSession {
    dest: Destination,
    store: Roots,
    downloads: Option<Downloads>,
    /// Opened by the first hook and shared by the rest, so a LabVIEW is asked
    /// once however many packages have hooks.
    hook_connection: Option<Connection>,
    /// Per package, in install order: the folders its files landed in.
    /// Relinking happens after every package is on disk, so no package can be
    /// relinked against a dependency that is not there yet.
    pub(super) relink_work: Vec<(String, Vec<PathBuf>)>,
    /// PostInstall hooks extracted so far, executed only after the relink
    /// pass has made them runnable.
    pub(super) post_install_hooks: Vec<PostInstallHook>,
    /// The LabVIEW a venv install resaves its staged packages in.
    pub(super) resaver: Resaver,
}

impl InstallSession {
    pub fn new(dest: Destination) -> InstallSession {
        InstallSession {
            store: dest.store(),
            dest,
            downloads: None,
            hook_connection: None,
            relink_work: Vec::new(),
            post_install_hooks: Vec::new(),
            resaver: Resaver::default(),
        }
    }

    pub fn destination(&self) -> &Destination {
        &self.dest
    }

    /// The store is one per destination: what reads or removes installed
    /// packages goes through it.
    pub fn store(&self) -> &Roots {
        &self.store
    }

    /// The LabVIEW installation the destination belongs to, if it has one: a
    /// scratch tree has none.
    pub fn target(&self) -> Option<&LvTarget> {
        self.store.target.as_ref()
    }

    pub fn venv(&self) -> Option<&Venv> {
        match &self.dest {
            Destination::Venv(v) => Some(v),
            _ => None,
        }
    }

    /// Where a package's own files go. In a venv every package is an addon of
    /// its own; elsewhere the store is the destination.
    pub fn roots_for(&self, package: &str) -> Result<Roots> {
        match self.venv() {
            Some(v) => v.roots_for_pkg(package),
            None => Ok(self.store.clone()),
        }
    }

    /// The archive's bytes, from the cache, a local file or the entry's feed.
    pub fn fetch(&mut self, env: &OpEnv, entry: &Entry) -> Result<Vec<u8>> {
        if self.downloads.is_none() {
            self.downloads = Some(Downloads::open(env)?);
        }
        self.downloads.as_ref().expect("opened above").fetch(entry)
    }

    /// Run one hook on the session's VI Server connection, opening it if no
    /// hook has yet, and report how it went. Whether it ran to the end.
    pub fn run_hook(&mut self, env: &OpEnv, target: &LvTarget, call: &HookCall) -> bool {
        run_reported_hook(env, &mut self.hook_connection, target, call)
    }

    /// Resave a staged venv package; anywhere else there is nothing to do.
    pub(super) fn resave(&mut self, env: &OpEnv, staging: &Staging, package: &str) {
        if let Destination::Venv(v) = &self.dest {
            self.resaver
                .resave(v, env.headless, env.sink, staging, package);
        }
    }

    /// Let go of the hook connection: the hooks that follow open their own.
    pub fn close_hook_connection(&mut self) {
        if let Some(connection) = self.hook_connection.take() {
            connection.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::NullSink;
    use crate::github::GitHubAuth;
    use crate::version::Version;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// The head of the one request a server answered with `bytes`, as `status`.
    fn serve_once(
        status: &'static str,
        bytes: &'static [u8],
    ) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/package.vip", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                head.push(byte[0]);
            }
            let reply = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            );
            stream.write_all(reply.as_bytes()).unwrap();
            stream.write_all(bytes).unwrap();
            String::from_utf8_lossy(&head).to_lowercase()
        });
        (url, thread)
    }

    fn entry(url: String, source: String) -> Entry {
        Entry {
            name: "pkg".into(),
            version: Version::parse("1.0.0"),
            url,
            md5: None,
            display_name: None,
            requires: Vec::new(),
            lv_min: None,
            source,
        }
    }

    #[test]
    fn the_github_token_is_sent_for_a_github_entry_and_for_no_other() {
        let cache = std::env::temp_dir().join(format!("lvpm-session-{}", std::process::id()));
        let mut env = OpEnv::new(
            &NullSink,
            crate::config::Config {
                cache: cache.clone(),
                local_sources: Vec::new(),
                venv_dir: "venv".into(),
            },
            Vec::new(),
            PathBuf::from("."),
        );
        env.github.auth = GitHubAuth::new(
            |name| (name == "GH_TOKEN").then(|| "tok".to_string()),
            || None,
        );
        let downloads = Downloads::open(&env).unwrap();

        for (source, sent) in [
            ("github:o/r", true),
            ("http://feed.example/index.json", false),
        ] {
            let (url, server) = serve_once("200 OK", b"bytes");
            downloads.fetch(&entry(url, source.into())).unwrap();
            let head = server.join().unwrap();
            assert_eq!(
                head.contains("authorization: bearer tok"),
                sent,
                "{source}: {head}"
            );
            assert_eq!(
                head.contains("accept: application/octet-stream"),
                sent,
                "{source}: {head}"
            );
        }
        std::fs::remove_dir_all(cache).ok();
    }

    #[test]
    fn a_github_file_that_cannot_be_downloaded_fails_in_githubs_words_and_a_feed_file_in_its_own() {
        let cache = std::env::temp_dir().join(format!("lvpm-session-fail-{}", std::process::id()));
        let config = crate::config::Config {
            cache: cache.clone(),
            local_sources: Vec::new(),
            venv_dir: "venv".into(),
        };
        let env = OpEnv::new(&NullSink, config, Vec::new(), PathBuf::from("."));
        let downloads = Downloads::open(&env).unwrap();
        let table = [
            (
                "404 Not Found",
                "not found. If it is private, run gh auth login or set GH_TOKEN",
            ),
            (
                "401 Unauthorized",
                "GitHub refused access (401 Unauthorized)",
            ),
            (
                "502 Bad Gateway",
                "cannot read the releases; check your network and try again: HTTP 502",
            ),
        ];
        for (status, said) in table {
            let (url, server) = serve_once(status, b"");
            let why = downloads
                .fetch(&entry(url.clone(), "github:o/r".into()))
                .unwrap_err();
            server.join().unwrap();
            assert_eq!(format!("{why:#}"), format!("pkg (github.com/o/r): {said}"));

            let (url, server) = serve_once(status, b"");
            let why = downloads
                .fetch(&entry(url, "http://feed.example/index.json".into()))
                .unwrap_err();
            server.join().unwrap();
            assert!(!format!("{why:#}").contains("github.com"), "{why:#}");
        }
        std::fs::remove_dir_all(cache).ok();
    }
}
