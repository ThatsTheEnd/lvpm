//! GitHub releases as a source of one package's files.
//!
//! `release_file` picks the package file from a release's assets by name;
//! `api` lists a repo's releases and maps GitHub's failures to errors; `auth`
//! finds the token. Nothing here reads the process environment: the front end
//! hands [`GitHubAuth`] the lookup and the `gh` runner.

mod api;
mod auth;
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
#[derive(Debug, Clone)]
pub struct GitHub {
    pub api: String,
    pub auth: GitHubAuth,
}

impl Default for GitHub {
    /// `https://api.github.com`, anonymous: what a test gets.
    fn default() -> GitHub {
        GitHub {
            api: DEFAULT_API.to_string(),
            auth: GitHubAuth::anonymous(),
        }
    }
}

impl GitHub {
    /// What the front end builds from the process: the API from
    /// `GITHUB_API_URL` (blank counts as unset) and the token chain over the same
    /// `lookup`, with `gh` run only when a token is needed.
    pub fn from_lookup(
        lookup: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
        gh: impl Fn() -> Option<String> + Send + Sync + 'static,
    ) -> GitHub {
        let api = lookup(API_VARIABLE).map(|api| api.trim().to_string());
        let api = api.filter(|api| !api.is_empty());
        GitHub {
            api: api.unwrap_or_else(|| DEFAULT_API.to_string()),
            auth: GitHubAuth::new(lookup, gh),
        }
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
}
