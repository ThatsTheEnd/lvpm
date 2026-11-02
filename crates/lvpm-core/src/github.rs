//! GitHub releases as a source of one package's files.
//!
//! `release_file` picks the package file from a release's assets by name;
//! `api` lists a repo's releases and maps GitHub's failures to errors; `auth`
//! finds the token; `attestation` asks whether a release has one and `gh`
//! runs `gh attestation verify`. Nothing here reads the process environment:
//! the front end hands [`GitHubAuth`] the lookup and the `gh` runner.

use std::fmt;

mod api;
pub mod attestation;
mod auth;
pub mod gh;
pub mod release_file;

pub use api::{Listing, ReleaseFile, download_error, releases, releases_cached};
pub use auth::{GitHubAuth, TokenSource, gh_auth_token};

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
    fn gh_is_not_run_until_the_front_end_installs_a_runner() {
        use std::path::Path;
        let not_installed = gh::GhRun::NotRunnable("gh is not installed".into());
        let run = |github: &GitHub| (github.runner)(Path::new("a.lvpkg"), "o/r", "tok");
        assert_eq!(run(&GitHub::default()), not_installed);
        assert_eq!(run(&GitHub::from_lookup(|_| None, || None)), not_installed);

        let github = GitHub::default().with_gh_runner(std::sync::Arc::new(|_, _, _| {
            gh::GhRun::NotRunnable("gh timed out".into())
        }));
        assert_eq!(run(&github), gh::GhRun::NotRunnable("gh timed out".into()));
        assert!(format!("{github:?}").contains("runner: <gh runner>"));
    }
}
