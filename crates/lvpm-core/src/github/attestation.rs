//! Whether a release archive has an attestation, and who built it.
//!
//! [`outcome`] asks GitHub's API first, with the archive's SHA-256; only when
//! GitHub holds an attestation does it run gh, which checks the signature, and
//! reads gh's JSON for the signer and the repository. Every answer is an
//! [`Outcome`]; what the policy makes of it is `verify`'s.

use super::GitHub;
use super::api::{MAX_BODY_BYTES, failure, request, unix_now};
use super::gh::{GhRun, GhVerify};
use crate::errors::GitHubFault;
use crate::verify::{Attestation, Invalid, NotChecked, Outcome};
use reqwest::blocking::Client;
use serde::Deserialize;
use serde::de::IgnoredAny;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

/// What asking for the attestation of `archive`, the file of `repo`
/// (`owner/repo`) whose SHA-256 is `sha`, came to. The lookup is made with or
/// without a token; gh needs one.
pub fn outcome(client: &Client, github: &GitHub, repo: &str, sha: &str, archive: &Path) -> Outcome {
    if let Some(settled) = settled_by_api(client, github, repo, sha) {
        return settled;
    }
    run_gh(github, repo, sha, archive, None)
}

/// What gh says of the OCI manifest in the file `manifest` (its SHA-256 is
/// `sha`), checked against the Sigstore `bundle` and built by `repo`
/// (`owner/repo`). GitHub's API is not asked: the bundle came from the
/// registry. gh needs a token.
pub fn verify_bundle(
    github: &GitHub,
    repo: &str,
    sha: &str,
    manifest: &Path,
    bundle: &Path,
) -> Outcome {
    run_gh(github, repo, sha, manifest, Some(bundle))
}

/// What `gh attestation verify` makes of `artifact` (SHA-256 `sha`), when
/// there is a token to run it with.
fn run_gh(
    github: &GitHub,
    repo: &str,
    sha: &str,
    artifact: &Path,
    bundle: Option<&Path>,
) -> Outcome {
    let Some(token) = github.auth.token() else {
        return Outcome::Unchecked(NotChecked::NoToken);
    };
    let ask = GhVerify {
        artifact,
        repo,
        token,
        bundle,
    };
    classify((github.runner)(ask), sha)
}

/// What GitHub's API says of the attestations of the archive with SHA-256
/// `sha`, when that settles the outcome. `None` means there is at least one,
/// so gh is to be asked.
fn settled_by_api(client: &Client, github: &GitHub, repo: &str, sha: &str) -> Option<Outcome> {
    let unchecked = |reason| Some(Outcome::Unchecked(reason));
    let api = github.api.trim_end_matches('/');
    let url = format!("{api}/repos/{repo}/attestations/sha256:{sha}");
    let Ok(response) = request(client, &url, &github.auth).send() else {
        return unchecked(NotChecked::Unreachable);
    };
    let status = response.status();
    let headers = response.headers().clone();
    let mut body = Vec::new();
    if (response.take(MAX_BODY_BYTES).read_to_end(&mut body)).is_err() {
        return unchecked(NotChecked::Unreachable);
    }
    if status.as_u16() == 404 {
        return Some(Outcome::Unattested);
    }
    if !status.is_success() {
        // Sorted as every other GitHub call sorts a failure.
        return unchecked(
            match failure(status, &headers, &body, &github.auth, unix_now()) {
                GitHubFault::RateLimited { token_source, .. } => NotChecked::RateLimited {
                    anonymous: token_source.is_none(),
                },
                GitHubFault::TokenRejected { .. } => NotChecked::TokenRejected,
                GitHubFault::Refused { .. } if status.as_u16() == 403 => NotChecked::LookupRefused,
                _ => NotChecked::Http {
                    status: status.as_u16(),
                },
            },
        );
    }
    #[derive(Deserialize)]
    struct List {
        attestations: Vec<IgnoredAny>,
    }
    match serde_json::from_slice::<List>(&body) {
        Ok(list) if list.attestations.is_empty() => Some(Outcome::Unattested),
        Ok(_) => None,
        Err(_) => unchecked(NotChecked::Unreadable),
    }
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
        // A signal (a Ctrl-C, a kill) ended gh: it said nothing of the attestation.
        GhRun::Spawned { status: None, .. } => Outcome::Unchecked(NotChecked::GhInterrupted),
        GhRun::Spawned { stderr, .. } => {
            let stderr = String::from_utf8_lossy(&stderr);
            // Anchored: gh's usage error leads its stderr, and a verify failure must not pass for it.
            if (stderr.trim_start()).starts_with(r#"unknown command "attestation" for "gh""#) {
                Outcome::Unchecked(NotChecked::GhTooOld)
            } else {
                Outcome::Invalid(Invalid::Rejected {
                    stderr: stderr.into_owned(),
                })
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
        return Outcome::Invalid(Invalid::UnreadableOutput);
    };
    if entries.is_empty() {
        return Outcome::Invalid(Invalid::UnreadableOutput);
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
        None => Outcome::Invalid(Invalid::OtherArchive),
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
        Outcome::Invalid(Invalid::UnreadableOutput)
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
                Outcome::Invalid(Invalid::OtherArchive),
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
        let runner: Runner = Arc::new(move |ask| {
            seen.lock().unwrap().push((
                ask.artifact.to_path_buf(),
                ask.repo.to_string(),
                ask.token.to_string(),
            ));
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

    /// One answer of the API: what it sends, and what lvpm makes of it.
    struct Row {
        status: &'static str,
        headers: &'static str,
        body: &'static str,
        token: Option<&'static str>,
        want: Outcome,
        gh_runs: bool,
    }

    #[test]
    fn the_api_lookup_decides_whether_gh_runs() {
        const RATE: &str = "x-ratelimit-remaining: 0\r\n";
        let unchecked = Outcome::Unchecked;
        let row = |status, headers, body, want| Row {
            status,
            headers,
            body,
            token: Some("tok"),
            want,
            gh_runs: false,
        };
        let table = [
            row("404 Not Found", "", "{}", Outcome::Unattested),
            row("200 OK", "", r#"{"attestations":[]}"#, Outcome::Unattested),
            row(
                "429 Too Many Requests",
                "",
                "{}",
                unchecked(NotChecked::RateLimited { anonymous: false }),
            ),
            row(
                "403 Forbidden",
                RATE,
                "{}",
                unchecked(NotChecked::RateLimited { anonymous: false }),
            ),
            Row {
                token: None,
                ..row(
                    "403 Forbidden",
                    RATE,
                    "{}",
                    unchecked(NotChecked::RateLimited { anonymous: true }),
                )
            },
            row(
                "403 Forbidden",
                "retry-after: 60\r\n",
                "{}",
                unchecked(NotChecked::RateLimited { anonymous: false }),
            ),
            row(
                "403 Forbidden",
                "x-ratelimit-remaining: 4999\r\n",
                "{}",
                unchecked(NotChecked::LookupRefused),
            ),
            row(
                "500 Internal Server Error",
                "",
                "{}",
                unchecked(NotChecked::Http { status: 500 }),
            ),
            row(
                "401 Unauthorized",
                "",
                "{}",
                unchecked(NotChecked::TokenRejected),
            ),
            Row {
                token: None,
                ..row(
                    "401 Unauthorized",
                    "",
                    "{}",
                    unchecked(NotChecked::Http { status: 401 }),
                )
            },
            row(
                "200 OK",
                "",
                "<html>a captive portal</html>",
                unchecked(NotChecked::Unreadable),
            ),
            Row {
                gh_runs: true,
                ..row(
                    "200 OK",
                    "",
                    ONE_ENTRY,
                    attested(RECORDED_SIGNER, "https://github.com/cli/cli"),
                )
            },
        ];
        for Row {
            status,
            headers,
            body,
            token,
            want,
            gh_runs,
        } in table
        {
            let (api, _) = serve(status, headers, body);
            let (runner, calls) = recording(spawned(0, RECORDED, ""));
            let github = github(&api, token, runner);
            assert_eq!(ask(&github), want, "{status} {headers:?} {body}");
            assert_eq!(
                calls.lock().unwrap().len(),
                usize::from(gh_runs),
                "{status} {body}"
            );
        }
    }

    #[test]
    fn a_connection_that_is_refused_is_unchecked_and_gh_is_not_run() {
        // Port 1 is never served here, and no other test can be handed it.
        let (runner, calls) = recording(spawned(0, RECORDED, ""));
        assert_eq!(
            ask(&github("http://127.0.0.1:1", Some("tok"), runner)),
            Outcome::Unchecked(NotChecked::Unreachable)
        );
        assert!(calls.lock().unwrap().is_empty());
    }

    fn rejected(stderr: &str) -> Outcome {
        Outcome::Invalid(Invalid::Rejected {
            stderr: stderr.into(),
        })
    }

    #[test]
    fn gh_failures_are_classified() {
        let table = [
            (
                "is not installed",
                GhRun::NotRunnable(NotChecked::GhMissing),
                Outcome::Unchecked(NotChecked::GhMissing),
            ),
            (
                r#"says unknown command "attestation""#,
                spawned(1, "", "unknown command \"attestation\" for \"gh\"\n"),
                Outcome::Unchecked(NotChecked::GhTooOld),
            ),
            (
                r#"fails verification quoting unknown command "attestation""#,
                spawned(
                    1,
                    "",
                    "error: ref unknown command \"attestation\" for \"gh\"",
                ),
                rejected("error: ref unknown command \"attestation\" for \"gh\""),
            ),
            (
                "times out",
                GhRun::NotRunnable(NotChecked::GhTimedOut),
                Outcome::Unchecked(NotChecked::GhTimedOut),
            ),
            (
                r#"exits 1 with "verification failed""#,
                spawned(1, "", "verification failed"),
                rejected("verification failed"),
            ),
            (
                "is ended by a signal",
                GhRun::Spawned {
                    status: None,
                    stdout: Vec::new(),
                    stderr: b"killed".to_vec(),
                },
                Outcome::Unchecked(NotChecked::GhInterrupted),
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
            Outcome::Unchecked(NotChecked::NoToken)
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

    #[test]
    fn a_bundle_goes_to_gh_with_the_manifest_and_the_repo_and_github_is_not_asked() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let runner: Runner = Arc::new(move |ask| {
            let bundle = ask.bundle.map(Path::to_path_buf);
            log.lock()
                .unwrap()
                .push((ask.artifact.to_path_buf(), bundle, ask.repo.to_string()));
            spawned(0, RECORDED, "")
        });
        // The API is unreachable: a call to it would make the outcome unchecked.
        let github = github("http://127.0.0.1:1", Some("tok"), runner);
        let outcome = verify_bundle(&github, "cli/cli", ARCHIVE, Path::new("m"), Path::new("b"));
        assert_eq!(
            outcome,
            attested(RECORDED_SIGNER, "https://github.com/cli/cli")
        );
        assert_eq!(
            *seen.lock().unwrap(),
            [(
                PathBuf::from("m"),
                Some(PathBuf::from("b")),
                "cli/cli".to_string()
            )]
        );
    }

    #[test]
    fn a_bundle_is_not_verified_without_a_token_and_a_digest_gh_did_not_vouch_for_is_invalid() {
        let (runner, calls) = recording(spawned(0, RECORDED, ""));
        let no_token = github("http://127.0.0.1:1", None, runner);
        let outcome = verify_bundle(&no_token, "o/r", ARCHIVE, Path::new("m"), Path::new("b"));
        assert_eq!(outcome, Outcome::Unchecked(NotChecked::NoToken));
        assert!(calls.lock().unwrap().is_empty());

        let (runner, _) = recording(spawned(0, RECORDED, ""));
        let github = github("http://127.0.0.1:1", Some("tok"), runner);
        let outcome = verify_bundle(&github, "o/r", OTHER, Path::new("m"), Path::new("b"));
        assert_eq!(outcome, Outcome::Invalid(Invalid::OtherArchive));
    }
}
