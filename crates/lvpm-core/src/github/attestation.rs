//! Whether a release archive has an attestation, and who built it.
//!
//! [`outcome`] asks GitHub's API first, with the archive's SHA-256; only when
//! GitHub holds an attestation does it run gh, which checks the signature, and
//! reads gh's JSON for the signer and the repository. Every answer is an
//! [`Outcome`]; what the policy makes of it is `verify`'s.

use super::GitHub;
use super::api::request;
use super::gh::GhRun;
use crate::verify::{Attestation, Outcome};
use reqwest::blocking::Client;
use serde::Deserialize;
use serde::de::IgnoredAny;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

/// A list of attestations larger than this is not one GitHub sent.
const MAX_BODY_BYTES: u64 = 8 * 1024 * 1024;
const NO_TOKEN: &str = "lvpm has no GitHub token";

/// What asking for the attestation of `archive`, the file of `repo`
/// (`owner/repo`) whose SHA-256 is `sha`, came to. The lookup is made with or
/// without a token; gh needs one.
pub fn outcome(client: &Client, github: &GitHub, repo: &str, sha: &str, archive: &Path) -> Outcome {
    if let Some(settled) = lookup(client, github, repo, sha) {
        return settled;
    }
    let Some(token) = github.auth.token() else {
        return Outcome::Unchecked(NO_TOKEN.to_string());
    };
    classify((github.runner)(archive, repo, token), sha)
}

/// Ask GitHub's API for the attestations of the archive with SHA-256 `sha`.
/// `None` means there is at least one, so gh is to be asked; any other answer
/// is the outcome.
pub fn lookup(client: &Client, github: &GitHub, repo: &str, sha: &str) -> Option<Outcome> {
    let api = github.api.trim_end_matches('/');
    let url = format!("{api}/repos/{repo}/attestations/sha256:{sha}");
    let Ok(response) = request(client, &url, &github.auth).send() else {
        return unchecked("could not reach GitHub");
    };
    let status = response.status();
    let none_left = response
        .headers()
        .get("x-ratelimit-remaining")
        .map(|v| v.as_bytes())
        == Some(b"0");
    let rate_limited = || match github.auth.token() {
        Some(_) => unchecked("GitHub rate limit reached"),
        None => unchecked("GitHub rate limit reached (60/h without a token)"),
    };
    match status.as_u16() {
        404 => Some(Outcome::Unattested),
        429 => rate_limited(),
        403 if none_left => rate_limited(),
        403 => unchecked("GitHub refused the attestation lookup (HTTP 403)"),
        _ if !status.is_success() => unchecked(format!("GitHub returned HTTP {}", status.as_u16())),
        _ => {
            #[derive(Deserialize)]
            struct List {
                attestations: Vec<IgnoredAny>,
            }
            let mut body = Vec::new();
            if response
                .take(MAX_BODY_BYTES)
                .read_to_end(&mut body)
                .is_err()
            {
                return unchecked("could not reach GitHub");
            }
            match serde_json::from_slice::<List>(&body) {
                Ok(list) if list.attestations.is_empty() => Some(Outcome::Unattested),
                Ok(_) => None,
                Err(_) => unchecked("GitHub returned an unreadable answer"),
            }
        }
    }
}

fn unchecked(reason: impl Into<String>) -> Option<Outcome> {
    Some(Outcome::Unchecked(reason.into()))
}

/// What a run of gh says of the archive with SHA-256 `sha`.
fn classify(run: GhRun, sha: &str) -> Outcome {
    match run {
        GhRun::NotRunnable(reason) => Outcome::Unchecked(reason),
        GhRun::Spawned {
            status: Some(0),
            stdout,
            ..
        } => parse_gh_json(&stdout, sha),
        GhRun::Spawned { stderr, .. } => {
            let stderr = String::from_utf8_lossy(&stderr);
            if stderr.contains(r#"unknown command "attestation""#) {
                Outcome::Unchecked("gh is too old to verify attestations".to_string())
            } else {
                Outcome::Invalid(stderr.into_owned())
            }
        }
    }
}

/// The attestation in the JSON `gh attestation verify --format json` printed
/// on success: the first entry whose subjects include the archive's `sha`. The
/// signer and the repository are kept as gh reports them. Anything lvpm cannot
/// read, or no entry for this archive, is [`Outcome::Invalid`]: gh vouched for
/// something lvpm cannot tie to the archive.
pub fn parse_gh_json(json: &[u8], sha: &str) -> Outcome {
    let Ok(entries) = serde_json::from_slice::<Vec<Entry>>(json) else {
        return Outcome::Invalid("unrecognised gh output".to_string());
    };
    if entries.is_empty() {
        return Outcome::Invalid("unrecognised gh output".to_string());
    }
    let for_archive = |entry: &&Entry| {
        (entry.result.statement.subject.iter()).any(|subject| {
            subject
                .digest
                .get("sha256")
                .is_some_and(|d| d.eq_ignore_ascii_case(sha))
        })
    };
    match entries.iter().find(for_archive) {
        Some(entry) => {
            let certificate = &entry.result.signature.certificate;
            Outcome::Attested(Attestation {
                signer: certificate.signer.clone(),
                subject: certificate.repository.clone(),
            })
        }
        None => Outcome::Invalid("gh's attestation does not match the archive".to_string()),
    }
}

// gh's shape, down to the fields lvpm reads.
#[derive(Deserialize)]
struct Entry {
    #[serde(rename = "verificationResult")]
    result: VerificationResult,
}

#[derive(Deserialize)]
struct VerificationResult {
    signature: Signature,
    statement: Statement,
}

#[derive(Deserialize)]
struct Signature {
    certificate: Certificate,
}

#[derive(Deserialize)]
struct Certificate {
    #[serde(rename = "subjectAlternativeName")]
    signer: String,
    #[serde(rename = "sourceRepositoryURI")]
    repository: String,
}

#[derive(Deserialize)]
struct Statement {
    subject: Vec<Subject>,
}

#[derive(Deserialize)]
struct Subject {
    digest: BTreeMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::GitHubAuth;
    use crate::github::gh::Runner;
    use serde_json::{Value, json};
    use std::io::Write;
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    /// `gh attestation verify gh_2.103.3_windows_arm64.zip --repo cli/cli
    /// --format json`, recorded with gh 2.100.0 on the cli/cli v2.103.3 release.
    const RECORDED: &str = include_str!("../../tests/fixtures/gh-attestation-verify.json");
    /// The SHA-256 of that archive.
    const ARCHIVE: &str = "197211c98687d7770ac331fe8a3b6c198c389108c1536cdbedf659fe4703e484";
    const OTHER: &str = "0000000000000000000000000000000000000000000000000000000000000000";
    const RECORDED_SIGNER: &str =
        "https://github.com/cli/cli/.github/workflows/deployment.yml@refs/heads/trunk";

    fn unrecognised() -> Outcome {
        Outcome::Invalid("unrecognised gh output".into())
    }

    fn recorded() -> Value {
        serde_json::from_str(RECORDED).unwrap()
    }

    /// The recorded entry attested as `signer` for `repository`, covering `sha`.
    fn entry(signer: &str, repository: &str, sha: &str) -> Value {
        let mut entry = recorded();
        let mut entry = entry[0].take();
        let result = &mut entry["verificationResult"];
        result["signature"]["certificate"]["subjectAlternativeName"] = json!(signer);
        result["signature"]["certificate"]["sourceRepositoryURI"] = json!(repository);
        result["statement"]["subject"] = json!([{"name": "a", "digest": {"sha256": sha}}]);
        entry
    }

    fn attested(signer: &str, subject: &str) -> Outcome {
        Outcome::Attested(Attestation {
            signer: signer.into(),
            subject: subject.into(),
        })
    }

    #[test]
    fn ghs_json_gives_the_workflow_as_signer_and_the_repository_as_subject() {
        assert_eq!(
            parse_gh_json(RECORDED.as_bytes(), ARCHIVE),
            attested(RECORDED_SIGNER, "https://github.com/cli/cli")
        );
    }

    #[test]
    fn gh_output_lvpm_cannot_use_is_invalid() {
        let no_certificate = json!([{"verificationResult": {
            "signature": {},
            "statement": {"subject": [{"digest": {"sha256": ARCHIVE}}]}
        }}]);
        let table = [
            ("not JSON", b"not JSON".to_vec(), ARCHIVE, unrecognised()),
            ("an empty list", b"[]".to_vec(), ARCHIVE, unrecognised()),
            (
                "an entry missing the certificate",
                no_certificate.to_string().into_bytes(),
                ARCHIVE,
                unrecognised(),
            ),
            (
                "entries whose digest is not the archive",
                RECORDED.as_bytes().to_vec(),
                OTHER,
                Outcome::Invalid("gh's attestation does not match the archive".into()),
            ),
        ];
        for (output, bytes, sha, want) in table {
            assert_eq!(parse_gh_json(&bytes, sha), want, "{output}");
        }
    }

    #[test]
    fn several_entries_pick_the_first_one_for_this_archive() {
        let list = json!([
            entry(
                "https://github.com/o/a/w.yml@v1",
                "https://github.com/o/a",
                OTHER
            ),
            entry(
                "https://github.com/o/b/w.yml@v1",
                "https://github.com/o/b",
                ARCHIVE
            ),
            entry(
                "https://github.com/o/c/w.yml@v1",
                "https://github.com/o/c",
                ARCHIVE
            ),
        ]);
        assert_eq!(
            parse_gh_json(list.to_string().as_bytes(), ARCHIVE),
            attested("https://github.com/o/b/w.yml@v1", "https://github.com/o/b")
        );
    }

    /// One request answered with `status` and `headers` (each line ended `\r\n`)
    /// and `body`; the API's URL, and the head of the request when it comes.
    fn serve(status: &str, headers: &str, body: &str) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        let reply = format!(
            "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                std::io::Read::read_exact(&mut stream, &mut byte).unwrap();
                head.push(byte[0]);
            }
            stream.write_all(reply.as_bytes()).unwrap();
            String::from_utf8_lossy(&head).to_lowercase()
        });
        (api, thread)
    }

    /// What a runner was asked: the archive, `owner/repo` and the token.
    type Calls = Arc<Mutex<Vec<(PathBuf, String, String)>>>;

    /// A runner that answers `answer` and records what it was asked.
    fn recording(answer: GhRun) -> (Runner, Calls) {
        let calls = Calls::default();
        let seen = calls.clone();
        let runner: Runner = Arc::new(move |archive, repo, token| {
            seen.lock()
                .unwrap()
                .push((archive.to_path_buf(), repo.to_string(), token.to_string()));
            answer.clone()
        });
        (runner, calls)
    }

    fn github(api: &str, token: Option<&'static str>, runner: Runner) -> GitHub {
        GitHub {
            api: api.to_string(),
            auth: GitHubAuth::new(
                move |name| token.filter(|_| name == "GH_TOKEN").map(str::to_string),
                || None,
            ),
            runner,
        }
    }

    const ONE_ENTRY: &str = r#"{"attestations":[{"bundle":{}}]}"#;

    fn spawned(status: i32, stdout: &str, stderr: &str) -> GhRun {
        GhRun::Spawned {
            status: Some(status),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    fn ask(github: &GitHub) -> Outcome {
        outcome(&Client::new(), github, "o/r", ARCHIVE, Path::new("a.lvpkg"))
    }

    #[test]
    fn the_api_lookup_decides_whether_gh_runs() {
        const RATE: &str = "x-ratelimit-remaining: 0\r\n";
        let rate_limit = |reason: &str| Outcome::Unchecked(reason.into());
        let table: [(&str, &str, &str, &str, Outcome, usize); 10] = [
            ("404 Not Found", "", "{}", "tok", Outcome::Unattested, 0),
            (
                "200 OK",
                "",
                r#"{"attestations":[]}"#,
                "tok",
                Outcome::Unattested,
                0,
            ),
            (
                "429 Too Many Requests",
                "",
                "{}",
                "tok",
                rate_limit("GitHub rate limit reached"),
                0,
            ),
            (
                "403 Forbidden",
                RATE,
                "{}",
                "tok",
                rate_limit("GitHub rate limit reached"),
                0,
            ),
            (
                "403 Forbidden",
                RATE,
                "{}",
                "",
                rate_limit("GitHub rate limit reached (60/h without a token)"),
                0,
            ),
            (
                "403 Forbidden",
                "x-ratelimit-remaining: 4999\r\n",
                "{}",
                "tok",
                rate_limit("GitHub refused the attestation lookup (HTTP 403)"),
                0,
            ),
            (
                "500 Internal Server Error",
                "",
                "{}",
                "tok",
                rate_limit("GitHub returned HTTP 500"),
                0,
            ),
            (
                "401 Unauthorized",
                "",
                "{}",
                "tok",
                rate_limit("GitHub returned HTTP 401"),
                0,
            ),
            (
                "200 OK",
                "",
                "<html>a captive portal</html>",
                "tok",
                rate_limit("GitHub returned an unreadable answer"),
                0,
            ),
            (
                "200 OK",
                "",
                ONE_ENTRY,
                "tok",
                attested(RECORDED_SIGNER, "https://github.com/cli/cli"),
                1,
            ),
        ];
        for (status, headers, body, token, want, ran) in table {
            let (api, _) = serve(status, headers, body);
            let (runner, calls) = recording(spawned(0, RECORDED, ""));
            let github = github(&api, (!token.is_empty()).then_some("tok"), runner);
            assert_eq!(ask(&github), want, "{status} {headers:?} {body}");
            assert_eq!(calls.lock().unwrap().len(), ran, "{status} {body}");
        }
    }

    #[test]
    fn a_connection_that_is_refused_is_unchecked_and_gh_is_not_run() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let (runner, calls) = recording(spawned(0, RECORDED, ""));
        assert_eq!(
            ask(&github(&api, Some("tok"), runner)),
            Outcome::Unchecked("could not reach GitHub".into())
        );
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn gh_failures_are_classified() {
        let table = [
            (
                "is not installed",
                GhRun::NotRunnable("gh is not installed".into()),
                Outcome::Unchecked("gh is not installed".into()),
            ),
            (
                r#"says unknown command "attestation""#,
                spawned(1, "", "unknown command \"attestation\" for \"gh\"\n"),
                Outcome::Unchecked("gh is too old to verify attestations".into()),
            ),
            (
                "times out",
                GhRun::NotRunnable("gh timed out".into()),
                Outcome::Unchecked("gh timed out".into()),
            ),
            (
                r#"exits 1 with "verification failed""#,
                spawned(1, "", "verification failed"),
                Outcome::Invalid("verification failed".into()),
            ),
            (
                "is ended by a signal",
                GhRun::Spawned {
                    status: None,
                    stdout: Vec::new(),
                    stderr: b"killed".to_vec(),
                },
                Outcome::Invalid("killed".into()),
            ),
        ];
        for (does, run, want) in table {
            let (api, _) = serve("200 OK", "", ONE_ENTRY);
            let (runner, _) = recording(run);
            assert_eq!(ask(&github(&api, Some("tok"), runner)), want, "gh {does}");
        }
    }

    #[test]
    fn no_token_means_unchecked_and_gh_is_not_run() {
        let (api, _) = serve("200 OK", "", ONE_ENTRY);
        let (runner, calls) = recording(spawned(0, RECORDED, ""));
        assert_eq!(
            ask(&github(&api, None, runner)),
            Outcome::Unchecked("lvpm has no GitHub token".into())
        );
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn gh_gets_the_token_the_archive_and_the_repo_after_a_lookup_by_sha() {
        let (api, server) = serve("200 OK", "", ONE_ENTRY);
        let (runner, calls) = recording(spawned(0, RECORDED, ""));
        let github = github(&api, Some("tok"), runner);
        outcome(&Client::new(), &github, "o/r", ARCHIVE, Path::new("blob"));
        assert_eq!(
            *calls.lock().unwrap(),
            [(PathBuf::from("blob"), "o/r".to_string(), "tok".to_string())]
        );
        let head = server.join().unwrap();
        assert!(
            head.starts_with(&format!("get /repos/o/r/attestations/sha256:{ARCHIVE} ")),
            "{head}"
        );
        assert!(head.contains("authorization: bearer tok"), "{head}");
    }
}
