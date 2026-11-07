//! [`IndexFiles`]: the files of community index repos, as one run reads them.

use crate::cache::Cache;
use crate::errors::{GitHubFault, IndexRepoUnreachable};
use crate::github::{self, GitHub};
use crate::index::IndexFeed;
use anyhow::Result;
use reqwest::blocking::Client;
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::Path;

/// Reads `packages/<key>.toml` of index repos through the cache, and checks an
/// index repo is there before it believes the first 404 from it.
///
/// A 404 on a file is also what a misspelled repo, or a private one without a
/// token, answers; read as "not listed", it would let a later source supply
/// the name in its place. So the first 404 from an index in a run is followed
/// by one request about the repo itself, and an index that is missing or
/// cannot be seen fails as a source (see [`IndexRepoUnreachable`]). The repos
/// found are remembered for this value's life, and not in the cache.
pub struct IndexFiles {
    client: Client,
    cache: Cache,
    github: GitHub,
    refresh: bool,
    found: RefCell<HashSet<String>>,
}

impl IndexFiles {
    /// Reads through the cache in \`cache\`, as \`github\` says.
    pub fn open(cache: &Path, github: &GitHub, refresh: bool) -> Result<IndexFiles> {
        Ok(IndexFiles {
            client: Client::builder()
                .user_agent(concat!("lvpm/", env!("CARGO_PKG_VERSION")))
                .build()?,
            cache: Cache::open(cache)?,
            github: github.clone(),
            refresh,
            found: RefCell::default(),
        })
    }

    /// The text of `packages/<key>.toml` in `feed`'s repo, or `None` when the
    /// repo has no such file.
    pub fn read(&self, feed: &IndexFeed, key: &str) -> Result<Option<String>> {
        let repo = format!("{}/{}", feed.owner, feed.repo);
        let path = format!("packages/{key}.toml");
        let (api, auth) = (&self.github.api, &self.github.auth);
        let limit = github::MAX_ENTRY_BYTES;
        let (cache, client) = (&self.cache, &self.client);
        let text =
            github::contents_cached(cache, client, api, &repo, &path, auth, self.refresh, limit)?;
        if text.is_none() && !self.found.borrow().contains(&repo) {
            github::repo_visible(client, api, &repo, auth).map_err(|fault| {
                IndexRepoUnreachable {
                    index: feed.source(),
                    why: match fault {
                        // The fault's own words about "the releases" do not fit a repo.
                        GitHubFault::Unreadable { cause } => cause,
                        other => other.to_string(),
                    },
                }
            })?;
            self.found.borrow_mut().insert(repo);
        }
        Ok(text)
    }
}
