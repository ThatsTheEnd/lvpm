//! GitHub releases as a source of one package's files.
//!
//! `release_file` picks the package file from a release's assets by name;
//! `api` lists a repo's releases and maps GitHub's failures to errors; `auth`
//! finds the token; `contents` reads one file of a repo; `attestation` asks
//! whether a release has one and `gh` runs `gh attestation verify`. Nothing here reads the process environment:
//! the front end hands [`GitHubAuth`] the lookup and the `gh` runner.

use std::fmt;

mod api;
pub mod attestation;
mod auth;
mod contents;
pub mod gh;
pub mod release_file;

pub use api::{Listing, ReleaseFile, download_error, releases, releases_cached};
pub use auth::{GitHubAuth, TokenSource, gh_auth_token};
pub use contents::{MAX_ENTRY_BYTES, contents_cached, repo_visible};

/// The API GitHub itself answers on.
pub const DEFAULT_API: &str = "https://api.github.com";
/// The variable that points lvpm at another API (GitHub Enterprise, a test).
pub const API_VARIABLE: &str = "GITHUB_API_URL";

/// Where GitHub calls go and who they are made as: what [`OpEnv`] carries.
///
/// [`OpEnv`]: crate::ops::OpEnv
#[derive(Clone)]
pub struct GitHub {
    pub api: String,
    pub auth: GitHubAuth,
    /// What runs `gh attestation verify`; see [`GitHub::with_gh_runner`].
    pub runner: gh::Runner,
}

impl fmt::Debug for GitHub {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitHub")
            .field("api", &self.api)
            .field("auth", &self.auth)
            .field("runner", &format_args!("<gh runner>"))
            .finish()
    }
}

impl Default for GitHub {
    /// `https://api.github.com`, anonymous, with no `gh` to run: what a test gets.
    fn default() -> GitHub {
        GitHub {
            api: DEFAULT_API.to_string(),
            auth: GitHubAuth::anonymous(),
            runner: gh::missing(),
        }
    }
}

impl GitHub {
    /// What the front end builds from the process: the API from
    /// `GITHUB_API_URL` (blank counts as unset) and the token chain over the same
    /// `lookup`, with `gh` run only when a token is needed.
    ///
    /// The runner of `gh attestation verify` is not set here: the front end must
    /// add [`gh::real_runner`] with [`GitHub::with_gh_runner`], or no attestation
    /// is ever checked.
    pub fn from_lookup(
        lookup: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
        gh: impl Fn() -> Option<String> + Send + Sync + 'static,
    ) -> GitHub {
        let api = lookup(API_VARIABLE).map(|api| api.trim().to_string());
        let api = api.filter(|api| !api.is_empty());
        GitHub {
            api: api.unwrap_or_else(|| DEFAULT_API.to_string()),
            auth: GitHubAuth::new(lookup, gh),
            runner: gh::missing(),
        }
    }

    /// The site's address, where release files are served: `github.com` for
    /// the public API, the host of a GitHub Enterprise `/api/v3`, and any other
    /// API (a test's) itself.
    pub fn web_base(&self) -> String {
        let api = self.api.trim_end_matches('/');
        if api == DEFAULT_API {
            return "https://github.com".to_string();
        }
        api.strip_suffix("/api/v3").unwrap_or(api).to_string()
    }

    /// This, with `runner` running `gh attestation verify`.
    pub fn with_gh_runner(mut self, runner: gh::Runner) -> GitHub {
        self.runner = runner;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_api_is_github_dot_com_unless_the_variable_names_another() {
        let api = |value: Option<&'static str>| {
            let lookup = move |name: &str| {
                (name == API_VARIABLE)
                    .then(|| value.map(str::to_string))
                    .flatten()
            };
            GitHub::from_lookup(lookup, || None).api
        };
        assert_eq!(api(None), DEFAULT_API);
        assert_eq!(api(Some("  ")), DEFAULT_API);
        assert_eq!(api(Some(" http://127.0.0.1:1 ")), "http://127.0.0.1:1");
        assert_eq!(GitHub::default().api, DEFAULT_API);
    }

    #[test]
    fn the_web_base_follows_the_api() {
        let web = |api: &str| {
            GitHub {
                api: api.to_string(),
                ..GitHub::default()
            }
            .web_base()
        };
        assert_eq!(web("https://api.github.com"), "https://github.com");
        assert_eq!(web("https://ghe.corp/api/v3"), "https://ghe.corp");
        assert_eq!(web("https://ghe.corp/api/v3/"), "https://ghe.corp");
        assert_eq!(web("http://127.0.0.1:5000"), "http://127.0.0.1:5000");
    }

    #[test]
    fn gh_is_not_run_until_the_front_end_installs_a_runner() {
        use crate::verify::NotChecked;
        use std::path::Path;
        let not_installed = gh::GhRun::NotRunnable(NotChecked::GhMissing);
        let run = |github: &GitHub| {
            (github.runner)(gh::GhVerify {
                artifact: Path::new("a.lvpkg"),
                repo: "o/r",
                token: "tok",
                bundle: None,
            })
        };
        assert_eq!(run(&GitHub::default()), not_installed);
        assert_eq!(run(&GitHub::from_lookup(|_| None, || None)), not_installed);

        let github = GitHub::default().with_gh_runner(std::sync::Arc::new(|_| {
            gh::GhRun::NotRunnable(NotChecked::GhTimedOut)
        }));
        assert_eq!(run(&github), gh::GhRun::NotRunnable(NotChecked::GhTimedOut));
    }
}
