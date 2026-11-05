//! The registry's bearer-token login: read the challenge a `401` carries, ask
//! its realm for a token. This slice asks anonymously.

use super::Error;
use reqwest::Url;
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, WWW_AUTHENTICATE};

/// A token reply is a few hundred bytes; more is not one.
const MAX_TOKEN_REPLY_BYTES: u64 = 64 * 1024;

/// What `WWW-Authenticate: Bearer ...` says: where to get a token. Its scope
/// is not kept: a caller asks for the access it needs, as its repository says.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Challenge {
    realm: String,
    service: Option<String>,
}

/// The Bearer challenge among the response's `WWW-Authenticate` headers.
pub(super) fn challenge(headers: &HeaderMap) -> Option<Challenge> {
    headers
        .get_all(WWW_AUTHENTICATE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(parse)
}

/// `Bearer realm="https://auth.example/token",service="registry",scope="..."`.
///
/// shortcut: every value must be quoted and holds no `\"`; every registry lvpm
/// targets writes them so. Upgrade when one does not.
fn parse(header: &str) -> Option<Challenge> {
    let (scheme, mut rest) = header.trim().split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("Bearer") {
        return None;
    }
    let (mut realm, mut service) = (None, None);
    while !rest.is_empty() {
        let (key, after) = rest.split_once('=')?;
        let (value, tail) = after.strip_prefix('"')?.split_once('"')?;
        let value = Some(value.to_string());
        match key.trim().to_ascii_lowercase().as_str() {
            "realm" => realm = value,
            "service" => service = value,
            _ => {}
        }
        rest = tail.trim_start_matches([',', ' ']);
    }
    Some(Challenge {
        realm: realm?,
        service,
    })
}

/// A token for `wanted` scope from the challenge's realm.
pub(super) fn fetch_token(
    client: &Client,
    registry: &str,
    challenge: &Challenge,
    wanted: &str,
) -> Result<String, Error> {
    let url = realm_url(registry, challenge, wanted)?;
    let reply = client
        .get(url)
        .send()
        .map_err(|e| Error::unreachable(registry, &e))?;
    let status = reply.status().as_u16();
    if status != 200 {
        return Err(Error::status(registry, status, "the token request"));
    }
    let body = super::read_limited(registry, reply, MAX_TOKEN_REPLY_BYTES)?;
    let json: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|_| Error::protocol(registry, "answered the token request with no JSON"))?;
    json["token"]
        .as_str()
        .or_else(|| json["access_token"].as_str())
        .map(str::to_string)
        .ok_or_else(|| Error::protocol(registry, "answered the token request with no token"))
}

/// The realm with `service` and `scope` as query parameters. The realm is the
/// registry's word, so it must be `https` too, but for a loopback test fake.
fn realm_url(registry: &str, challenge: &Challenge, wanted: &str) -> Result<Url, Error> {
    let mut url = Url::parse(&challenge.realm)
        .map_err(|_| Error::protocol(registry, "named a token realm that is not a URL"))?;
    // A plain-http realm is for a loopback registry only: a remote registry
    // must not point lvpm at a service on this machine.
    let registry_host = registry.split(':').next().unwrap_or_default();
    if !super::scheme_allowed(url.scheme(), url.host_str().unwrap_or_default())
        || (url.scheme() == "http" && !super::is_loopback(registry_host))
    {
        return Err(Error::protocol(
            registry,
            "named a token realm that is not https",
        ));
    }
    {
        let mut query = url.query_pairs_mut();
        if let Some(service) = &challenge.service {
            query.append_pair("service", service);
        }
        query.append_pair("scope", wanted);
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    fn challenge_of(header: &str) -> Option<Challenge> {
        let mut headers = HeaderMap::new();
        headers.append(WWW_AUTHENTICATE, HeaderValue::from_str(header).unwrap());
        challenge(&headers)
    }

    #[test]
    fn a_bearer_challenge_names_the_realm_and_the_service_and_skips_a_scope_with_its_comma() {
        let got = challenge_of(
            "Bearer realm=\"https://ghcr.io/token\",service=\"ghcr.io\",scope=\"repository:o/a:pull,push\"",
        )
        .unwrap();
        assert_eq!(
            got,
            Challenge {
                realm: "https://ghcr.io/token".into(),
                service: Some("ghcr.io".into()),
            }
        );
    }

    #[test]
    fn a_challenge_needs_a_bearer_scheme_and_a_realm() {
        assert_eq!(challenge_of("Basic realm=\"x\""), None);
        assert_eq!(challenge_of("Bearer service=\"x\""), None);
        assert_eq!(challenge_of("Bearer realm=x"), None);
        assert!(challenge_of("bearer realm=\"https://a/t\"").is_some());
    }

    #[test]
    fn the_realm_gets_the_service_and_scope_and_must_be_https_but_on_loopback() {
        let named = |realm: &str| Challenge {
            realm: realm.to_string(),
            service: Some("svc".into()),
        };
        let url = realm_url("r", &named("https://a.example/token"), "s:1").unwrap();
        assert_eq!(
            url.as_str(),
            "https://a.example/token?service=svc&scope=s%3A1"
        );
        let url = realm_url("127.0.0.1:5", &named("http://127.0.0.1:9/t"), "own:pull").unwrap();
        assert_eq!(
            url.as_str(),
            "http://127.0.0.1:9/t?service=svc&scope=own%3Apull"
        );
        for bad in ["http://a.example/token", "ftp://127.0.0.1/t", "not a url"] {
            assert!(realm_url("r", &named(bad), "own").is_err(), "{bad}");
        }
        // A remote registry cannot send lvpm to a service on this machine.
        assert!(realm_url("ghcr.io", &named("http://127.0.0.1:9/t"), "own").is_err());
    }
}
