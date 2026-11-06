//! The registry's bearer-token login: read the challenge a `401` carries, ask
//! its realm for a token. The realm is asked anonymously, or with the login the
//! Docker config gave: Basic auth for a user name and password, and for an
//! identity token the OAuth2 refresh-token grant.

use super::Error;
use super::logins::Credential;
use reqwest::Url;
use reqwest::blocking::Client;
use reqwest::header::{CONTENT_TYPE, HeaderMap, WWW_AUTHENTICATE};

/// A token reply is a few hundred bytes; more is not one.
const MAX_TOKEN_REPLY_BYTES: u64 = 64 * 1024;
/// What lvpm calls itself to a token service in a refresh-token grant.
const CLIENT_ID: &str = "lvpm";

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

/// A token for `wanted` scope from the challenge's realm, logged in with
/// `login` when there is one. A realm that refuses a login says so as
/// [`Error::LoginRefused`]; one that refuses an anonymous ask is a plain status.
/// `client` follows no redirect: a `3xx` is the realm's answer like any other, so
/// that an identity token in the body is never sent on to another host.
pub(super) fn fetch_token(
    client: &Client,
    registry: &str,
    challenge: &Challenge,
    wanted: &str,
    login: Option<&Credential>,
) -> Result<String, Error> {
    let request = match login.and_then(Credential::identity_token) {
        Some(token) => client
            .post(checked_realm(registry, challenge)?)
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(refresh_form(challenge, wanted, token)),
        None => {
            let request = client.get(realm_url(registry, challenge, wanted)?);
            match login.and_then(Credential::basic_login) {
                Some((user, secret)) => request.basic_auth(user, Some(secret)),
                None => request,
            }
        }
    };
    let reply = request
        .send()
        .map_err(|e| Error::unreachable(registry, &e))?;
    let status = reply.status().as_u16();
    if let (401, Some(login)) = (status, login) {
        return Err(Error::LoginRefused {
            registry: registry.to_string(),
            origin: login.origin().to_string(),
        });
    }
    match status {
        200 => {}
        // The realm saying no to who asked is a refusal, as the registry's own.
        401 | 403 => return Err(Error::status(registry, status, "the token request")),
        // Any other status is the token service's, not the registry's word on
        // the package: a `404` here must not read as "no such repository".
        _ => {
            let problem = format!("answered {status} to the token request");
            return Err(Error::protocol(registry, &problem));
        }
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

/// The body of the refresh-token grant that trades an identity token for an
/// access token.
fn refresh_form(challenge: &Challenge, wanted: &str, token: &str) -> String {
    // Only to write the pairs the way a form is: `Url` has the encoder.
    let mut form = Url::parse("http://form.invalid/").expect("a URL");
    {
        let mut pairs = form.query_pairs_mut();
        pairs.append_pair("grant_type", "refresh_token");
        if let Some(service) = &challenge.service {
            pairs.append_pair("service", service);
        }
        pairs.append_pair("scope", wanted);
        pairs.append_pair("client_id", CLIENT_ID);
        pairs.append_pair("refresh_token", token);
    }
    form.query().unwrap_or_default().to_string()
}

/// The realm with `service` and `scope` as query parameters.
fn realm_url(registry: &str, challenge: &Challenge, wanted: &str) -> Result<Url, Error> {
    let mut url = checked_realm(registry, challenge)?;
    {
        let mut query = url.query_pairs_mut();
        if let Some(service) = &challenge.service {
            query.append_pair("service", service);
        }
        query.append_pair("scope", wanted);
    }
    Ok(url)
}

/// The realm the challenge names. It is the registry's word, so it must be
/// `https` too, but for a loopback test fake: a login is never sent in the clear.
fn checked_realm(registry: &str, challenge: &Challenge) -> Result<Url, Error> {
    let url = Url::parse(&challenge.realm)
        .map_err(|_| Error::protocol(registry, "named a token realm that is not a URL"))?;
    // A remote registry must not point lvpm at a service on this machine, nor
    // send a login in the clear.
    let realm_host = url.host_str().unwrap_or_default();
    let registry_host = registry.split(':').next().unwrap_or_default();
    if !super::scheme_allowed(url.scheme(), realm_host)
        || (super::is_loopback(realm_host) && !super::is_loopback(registry_host))
    {
        return Err(Error::protocol(
            registry,
            "named a token realm that is not https",
        ));
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
        for realm in [
            "http://127.0.0.1:9/t",
            "https://127.0.0.1:9/t",
            "https://localhost/t",
        ] {
            assert!(
                realm_url("ghcr.io", &named(realm), "own").is_err(),
                "{realm}"
            );
        }
    }

    #[test]
    fn an_identity_token_is_traded_in_a_refresh_token_grant_form() {
        let challenge = Challenge {
            realm: "https://a.example/token".into(),
            service: Some("svc one".into()),
        };
        assert_eq!(
            refresh_form(&challenge, "repository:o/a:pull,push", "r1-3c8f2b-token"),
            "grant_type=refresh_token&service=svc+one&scope=repository%3Ao%2Fa%3Apull%2Cpush\
             &client_id=lvpm&refresh_token=r1-3c8f2b-token"
        );
    }
}
