//! Whether a login is a public member of an organisation: the one fact the
//! community index's bot asks of GitHub about a PR's author.

use super::api::get;
use super::auth::GitHubAuth;
use crate::errors::GitHubFault;
use reqwest::blocking::Client;

/// Whether `login` publicly belongs to the organisation `org`, on the API at
/// `api`: `GET /orgs/{org}/public_members/{login}` is a 204 for a member and a
/// 404 for anyone else, and for a name that is no organisation. Any other answer
/// is a fault, not a "no".
///
/// Both names come from a PR, so one that GitHub could not have as a login (or
/// that would climb out of the path) is no member, and asks nothing.
pub fn public_member(
    client: &Client,
    api: &str,
    org: &str,
    login: &str,
    auth: &GitHubAuth,
) -> Result<bool, GitHubFault> {
    if !is_login(org) || !is_login(login) {
        return Ok(false);
    }
    let url = format!(
        "{}/orgs/{org}/public_members/{login}",
        api.trim_end_matches('/')
    );
    match get(client, &url, auth) {
        Ok(_) => Ok(true),
        Err(GitHubFault::NotFound { .. }) => Ok(false),
        Err(fault) => Err(fault),
    }
}

/// Whether `name` is made of what a GitHub login is: letters, digits and
/// hyphens, and not empty.
fn is_login(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_what_a_login_can_be_is_asked_about() {
        for name in ["alice", "Some-One", "a1"] {
            assert!(is_login(name), "{name}");
        }
        for name in [
            "",
            "..",
            "a/b",
            "a?b",
            "a b",
            "a%2fb",
            "dependabot[bot]",
            "a.b",
        ] {
            assert!(!is_login(name), "{name}");
        }
    }
}
