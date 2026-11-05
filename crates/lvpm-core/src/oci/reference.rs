//! What an `oci://` URL names: a feed (`oci://host[:port]/prefix`) or one
//! package version in it (`oci://host[:port]/repo:tag`, `oci://host[:port]/repo@sha256:...`).
//!
//! A URL that carries `user:password@` is refused, and no error here holds any
//! part of the URL but the registry host: a secret in a URL must not reach
//! output.

use std::fmt;

/// What an `oci://` URL starts with.
pub const SCHEME: &str = "oci://";
const MAX_TAG_LEN: usize = 128;
const SHA256_PREFIX: &str = "sha256:";
const SHA256_HEX_LEN: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UrlError {
    /// The authority has `user:password@`. Only the registry is kept.
    #[error("credentials do not belong in the URL; remove the user:password@ part")]
    Credentials { registry: String },
    #[error("{0}")]
    Invalid(&'static str),
}

/// The registry and repository prefix of an OCI feed. Package `X` lives in
/// repository `<prefix>/<name_key(X)>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedUrl {
    registry: String,
    prefix: String,
}

impl FeedUrl {
    /// `oci://host[:port]/prefix`; the prefix may have several `/` parts.
    pub fn parse(text: &str) -> Result<FeedUrl, UrlError> {
        let (registry, path) = split(text)?;
        let prefix = path.trim_end_matches('/');
        if prefix.is_empty() {
            return Err(UrlError::Invalid("an OCI feed is oci://host/prefix"));
        }
        Ok(FeedUrl {
            registry,
            prefix: valid_repository(prefix)?,
        })
    }

    /// The registry, `host` or `host:port`.
    pub fn registry(&self) -> &str {
        &self.registry
    }

    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// The repository package `name` is in.
    pub fn repository(&self, name: &str) -> String {
        format!("{}/{}", self.prefix, crate::project::name_key(name))
    }
}

impl fmt::Display for FeedUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{SCHEME}{}/{}", self.registry, self.prefix)
    }
}

/// What a reference points at in its repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Tag(String),
    /// `sha256:<64 hex>`.
    Digest(String),
}

/// One OCI manifest: where it is and by which name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub registry: String,
    pub repository: String,
    pub target: Target,
}

impl Reference {
    pub fn tagged(registry: &str, repository: &str, tag: &str) -> Reference {
        Reference {
            registry: registry.to_string(),
            repository: repository.to_string(),
            target: Target::Tag(tag.to_string()),
        }
    }

    /// `oci://host/repo:tag` or `oci://host/repo@sha256:<hex>`.
    pub fn parse(text: &str) -> Result<Reference, UrlError> {
        let (registry, path) = split(text)?;
        let (repository, target) = match path.split_once('@') {
            Some((repository, digest)) if valid_digest(digest) => {
                (repository, Target::Digest(digest.to_string()))
            }
            Some(_) => {
                return Err(UrlError::Invalid(
                    "the digest is not sha256:<64 hex digits>",
                ));
            }
            None => match path.rsplit_once(':') {
                Some((repository, tag)) if valid_tag(tag) => {
                    (repository, Target::Tag(tag.to_string()))
                }
                Some(_) => return Err(UrlError::Invalid("the tag is not a valid OCI tag")),
                None => return Err(UrlError::Invalid("a reference ends in :tag or @digest")),
            },
        };
        Ok(Reference {
            registry,
            repository: valid_repository(repository)?,
            target,
        })
    }

    /// The tag or the digest.
    pub fn name(&self) -> &str {
        match &self.target {
            Target::Tag(name) | Target::Digest(name) => name,
        }
    }
}

impl fmt::Display for Reference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let joint = match self.target {
            Target::Tag(_) => ':',
            Target::Digest(_) => '@',
        };
        write!(
            f,
            "{SCHEME}{}/{}{joint}{}",
            self.registry,
            self.repository,
            self.name()
        )
    }
}

/// The registry and the path of an `oci://` URL; `user:password@` is refused.
fn split(text: &str) -> Result<(String, &str), UrlError> {
    let rest = text
        .strip_prefix(SCHEME)
        .ok_or(UrlError::Invalid("not an oci:// URL"))?;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    if let Some((_, registry)) = authority.rsplit_once('@') {
        return Err(UrlError::Credentials {
            registry: registry.to_string(),
        });
    }
    if !valid_registry(authority) {
        return Err(UrlError::Invalid("the registry must be host or host:port"));
    }
    Ok((authority.to_string(), path))
}

/// `host` or `host:port`: letters, digits, `.` and `-`, then 1-5 digits.
///
/// shortcut: no IPv6 literal; a registry is reached by name.
fn valid_registry(text: &str) -> bool {
    let (host, port) = match text.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (text, None),
    };
    let host_ok = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    let port_ok =
        port.is_none_or(|p| (1..=5).contains(&p.len()) && p.chars().all(|c| c.is_ascii_digit()));
    host_ok && port_ok
}

/// `path` when each of its `/` parts is an OCI repository name component.
fn valid_repository(path: &str) -> Result<String, UrlError> {
    match path.split('/').all(valid_component) {
        true => Ok(path.to_string()),
        false => Err(UrlError::Invalid(
            "repository names are lowercase letters and digits, separated by . _ - or /",
        )),
    }
}

/// `[a-z0-9]+((\.|_|__|-+)[a-z0-9]+)*`, the OCI distribution spec's component.
fn valid_component(component: &str) -> bool {
    let mut rest = component;
    loop {
        let after = rest.trim_start_matches(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit());
        if after.len() == rest.len() {
            return false;
        }
        if after.is_empty() {
            return true;
        }
        rest = match (after.strip_prefix("__"), after.strip_prefix(['_', '.'])) {
            (Some(next), _) | (None, Some(next)) => next,
            (None, None) if after.starts_with('-') => after.trim_start_matches('-'),
            (None, None) => return false,
        };
    }
}

/// `[A-Za-z0-9_][A-Za-z0-9._-]{0,127}`, the OCI distribution spec's tag.
pub fn valid_tag(tag: &str) -> bool {
    let mut chars = tag.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
    first_ok
        && tag.len() <= MAX_TAG_LEN
        && chars.all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

/// `sha256:` and 64 lower-case hex digits.
pub fn valid_digest(digest: &str) -> bool {
    digest.strip_prefix(SHA256_PREFIX).is_some_and(|hex| {
        hex.len() == SHA256_HEX_LEN && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a";

    #[test]
    fn a_feed_url_has_a_registry_and_a_prefix_and_names_a_packages_repository() {
        let feed = FeedUrl::parse("oci://127.0.0.1:5000/team/sub/").unwrap();
        assert_eq!(feed.registry(), "127.0.0.1:5000");
        assert_eq!(feed.prefix(), "team/sub");
        assert_eq!(feed.repository("Alpha_Lib"), "team/sub/alpha_lib");
        assert_eq!(feed.to_string(), "oci://127.0.0.1:5000/team/sub");
    }

    #[test]
    fn a_url_with_credentials_is_refused_naming_only_the_registry() {
        for text in [
            "oci://u-4c1d2e:pw-7f3a9c-secret@ghcr.io/org",
            "oci://pw-7f3a9c-secret@ghcr.io/org",
            "oci://u-4c1d2e:p@ss-7f3a9c@ghcr.io/org",
        ] {
            let error = FeedUrl::parse(text).unwrap_err();
            assert_eq!(
                error,
                UrlError::Credentials {
                    registry: "ghcr.io".into()
                }
            );
            let shown = format!("{error} {error:?}");
            assert!(
                !shown.contains("7f3a9c") && !shown.contains("4c1d2e"),
                "{shown}"
            );
        }
        let reference = Reference::parse(&format!("oci://u:s3cret-7f3a9c@ghcr.io/o/a@{DIGEST}"));
        assert!(matches!(reference, Err(UrlError::Credentials { .. })));
    }

    #[test]
    fn a_url_that_is_not_a_feed_is_refused_without_echoing_it() {
        for (text, why) in [
            ("https://ghcr.io/org", "not an oci:// URL"),
            ("oci://ghcr.io", "an OCI feed is oci://host/prefix"),
            ("oci://ghcr.io/", "an OCI feed is oci://host/prefix"),
            ("oci:///org", "host or host:port"),
            ("oci://ghcr.io:99999x/org", "host or host:port"),
            (
                "oci://user-7f3a9c:pw/with/slash@ghcr.io/org",
                "host or host:port",
            ),
            ("oci://ghcr.io/Org", "lowercase"),
            ("oci://ghcr.io/a//b", "lowercase"),
            ("oci://ghcr.io/a..b", "lowercase"),
            ("oci://ghcr.io/org?x=1", "lowercase"),
        ] {
            let error = FeedUrl::parse(text).unwrap_err().to_string();
            assert!(error.contains(why), "{text}: {error}");
            assert!(!error.contains("7f3a9c"), "{text}: {error}");
        }
    }

    #[test]
    fn repository_components_follow_the_distribution_spec() {
        for good in [
            "a", "a1", "my-lib", "my--lib", "my_lib", "my__lib", "my.lib", "a/b.c-d",
        ] {
            assert!(FeedUrl::parse(&format!("oci://h/{good}")).is_ok(), "{good}");
        }
        for bad in ["-a", "a-", "a___b", "a._b", "a.-b", "a b", "A", ".a"] {
            assert!(FeedUrl::parse(&format!("oci://h/{bad}")).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_reference_is_a_tag_or_a_digest_and_prints_as_it_was_written() {
        for text in [
            "oci://127.0.0.1:5000/team/alpha:1.0.0".to_string(),
            format!("oci://ghcr.io/org/alpha@{DIGEST}"),
        ] {
            assert_eq!(Reference::parse(&text).unwrap().to_string(), text);
        }
        let tagged = Reference::parse("oci://h:1/t/a:1.0.0").unwrap();
        assert_eq!(tagged, Reference::tagged("h:1", "t/a", "1.0.0"));
        assert_eq!(tagged.name(), "1.0.0");
        let by_digest = Reference::parse(&format!("oci://h/t/a@{DIGEST}")).unwrap();
        assert_eq!(by_digest.target, Target::Digest(DIGEST.into()));
    }

    #[test]
    fn a_reference_without_a_valid_tag_or_digest_is_refused() {
        for text in [
            "oci://h/t/a",
            "oci://h/t/a:",
            "oci://h/t/a:1.0.0+build",
            "oci://h/t/a@sha256:abc",
            "oci://h/t/a@md5:00",
        ] {
            assert!(
                matches!(Reference::parse(text), Err(UrlError::Invalid(_))),
                "{text}"
            );
        }
    }

    #[test]
    fn a_tag_is_up_to_128_of_letters_digits_underscore_dot_and_dash() {
        assert!(valid_tag("1.0.0"));
        assert!(valid_tag("_x-1.2"));
        assert!(valid_tag(&"a".repeat(128)));
        assert!(!valid_tag(&"a".repeat(129)));
        for bad in ["", ".1", "-1", "1.0.0+build.7", "a b", "a/b", "a:b"] {
            assert!(!valid_tag(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_digest_is_sha256_and_64_lower_case_hex_digits() {
        assert!(valid_digest(DIGEST));
        assert!(!valid_digest(&DIGEST.to_uppercase()));
        assert!(!valid_digest(&DIGEST[..DIGEST.len() - 1]));
        assert!(!valid_digest(&DIGEST.replace("sha256", "sha512")));
    }
}
