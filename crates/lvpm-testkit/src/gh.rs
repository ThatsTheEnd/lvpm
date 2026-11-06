//! The fake of `gh attestation verify <file> --bundle <file> --repo <owner/repo>
//! --format json`, and of the Sigstore bundle it is given.
//!
//! `lvpm` runs `gh` as a program, so a scenario cannot hand it a closure: a
//! copy of the running test program called `gh` is the fake, as
//! [`helper`](super::helper) does for a credential helper. [`install`] puts it
//! in a directory with a JSON file that says what it does; a test program that
//! calls [`run_if_gh`] first thing in its `main` turns into gh when it is
//! started under that name. Every run adds a line to a log beside it, which
//! says what gh was asked and the SHA-256 of the files it was given, so a test
//! can tell which bytes lvpm handed over.
//!
//! The fake verifies as gh does, in what a test can see: the file's SHA-256 is
//! the subject it vouches for, and `--repo` must be the repository that built
//! it, or it exits 1 with the message gh gives.

use base64::Engine as _;
use lvpm_core::oci::{BUNDLE_TYPE, digest_of};
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};

const GITHUB: &str = "https://github.com/";
/// What real gh says when `--repo` is not the repository that built the artifact.
const WRONG_REPO: &str = "verifying with issuer \"sigstore.dev\"";
/// What real gh says when it has no login.
const NOT_LOGGED_IN: &str = "To get started with GitHub CLI, please run:  gh auth login";
/// The Fulcio extension `SourceRepositoryURI` as DER: tag, length and OID.
const SOURCE_REPOSITORY_OID: [u8; 12] = [
    0x06, 0x0a, 0x2b, 0x06, 0x01, 0x04, 0x01, 0x83, 0xbf, 0x30, 0x01, 0x0c,
];

/// What the fake does when it is asked to verify.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Behavior {
    /// The bundle verifies: the artifact was built by this repository, as its
    /// URL (`https://github.com/o/r`).
    Verifies { repository: String },
    /// Verification fails: gh exits 1 and says this.
    Rejects { stderr: String },
}

impl Behavior {
    fn to_json(&self) -> Value {
        match self {
            Behavior::Verifies { repository } => {
                json!({ "does": "verifies", "repository": repository })
            }
            Behavior::Rejects { stderr } => json!({ "does": "rejects", "stderr": stderr }),
        }
    }

    fn from_json(value: &Value) -> Option<Behavior> {
        let text = |key: &str| value[key].as_str().map(str::to_string);
        Some(match value["does"].as_str()? {
            "verifies" => Behavior::Verifies {
                repository: text("repository")?,
            },
            "rejects" => Behavior::Rejects {
                stderr: text("stderr")?,
            },
            _ => return None,
        })
    }
}

/// One run of the fake: what it was given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub args: Vec<String>,
    /// The SHA-256 (hex) of the file it was asked to verify, if it could be read.
    pub artifact_sha256: Option<String>,
    /// The SHA-256 (hex) of the file after `--bundle`, if there was one.
    pub bundle_sha256: Option<String>,
    /// Whether `GH_TOKEN` was set: the token itself is never kept.
    pub token_given: bool,
}

impl Run {
    fn to_json(&self) -> Value {
        json!({
            "args": self.args,
            "artifact_sha256": self.artifact_sha256,
            "bundle_sha256": self.bundle_sha256,
            "token_given": self.token_given,
        })
    }

    fn from_json(value: &Value) -> Option<Run> {
        let text = |key: &str| value[key].as_str().map(str::to_string);
        Some(Run {
            args: (value["args"].as_array()?.iter())
                .map(|arg| arg.as_str().map(str::to_string))
                .collect::<Option<_>>()?,
            artifact_sha256: text("artifact_sha256"),
            bundle_sha256: text("bundle_sha256"),
            token_given: value["token_given"].as_bool()?,
        })
    }

    /// The value after `flag`.
    fn flag(&self, flag: &str) -> Option<&str> {
        let at = self.args.iter().position(|arg| arg == flag)?;
        self.args.get(at + 1).map(String::as_str)
    }
}

/// What the fake prints and how it ends.
#[derive(Debug, PartialEq, Eq)]
struct Response {
    stdout: String,
    stderr: String,
    code: i32,
}

fn said(stdout: &str, stderr: &str, code: i32) -> Response {
    Response {
        stdout: stdout.to_string(),
        stderr: stderr.to_string(),
        code,
    }
}

/// What the fake does for `run`.
fn respond(behavior: &Behavior, run: &Run) -> Response {
    let verify = matches!(run.args.as_slice(), [a, b, ..] if a == "attestation" && b == "verify");
    if !verify || !run.token_given {
        return said("", NOT_LOGGED_IN, 4);
    }
    // As real gh: it reads a bundle by its extension, and refuses any other.
    let bundle_extension = run
        .flag("--bundle")
        .map(|path| Path::new(path).extension().and_then(|e| e.to_str()));
    if matches!(bundle_extension, Some(extension) if !matches!(extension, Some("json" | "jsonl"))) {
        return said(
            "",
            "Error: bundle file extension not supported, must be json or jsonl",
            1,
        );
    }
    match behavior {
        Behavior::Rejects { stderr } => said("", stderr, 1),
        Behavior::Verifies { repository } => {
            let built_by = repository.strip_prefix(GITHUB).unwrap_or(repository);
            let (Some(sha), Some(_)) = (&run.artifact_sha256, &run.bundle_sha256) else {
                return said("", "no bundle or no file to verify", 1);
            };
            if run.flag("--repo") != Some(built_by) {
                return said("", WRONG_REPO, 1);
            }
            let workflow = format!("{repository}/.github/workflows/release.yml@refs/tags/v1.0.0");
            let entry = json!({ "verificationResult": {
                "signature": { "certificate": {
                    "subjectAlternativeName": workflow,
                    "sourceRepositoryURI": repository,
                }},
                "statement": { "subject": [{ "digest": { "sha256": sha } }] },
            }});
            said(&json!([entry]).to_string(), "", 0)
        }
    }
}

fn program_name() -> String {
    format!("gh{}", std::env::consts::EXE_SUFFIX)
}

fn behavior_file(program: &Path) -> PathBuf {
    program.with_extension("json")
}

fn runs_file(program: &Path) -> PathBuf {
    program.with_extension("runs")
}

/// Make `gh` in `dir` act as `behavior`: the running test program under that
/// name, and a file that tells it what to do. A gh already there is replaced.
pub fn install(dir: &Path, behavior: &Behavior) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let program = dir.join(program_name());
    let me = std::env::current_exe()?;
    std::fs::remove_file(&program).ok();
    // A link costs no copy of a test program that can be tens of megabytes.
    std::fs::hard_link(&me, &program).or_else(|_| std::fs::copy(&me, &program).map(drop))?;
    std::fs::write(behavior_file(&program), behavior.to_json().to_string())?;
    clear_runs(dir);
    Ok(program)
}

/// What the `gh` in `dir` was asked, one run each, in order.
pub fn runs(dir: &Path) -> Vec<Run> {
    let text = std::fs::read_to_string(runs_file(&dir.join(program_name())));
    (text.unwrap_or_default().lines())
        .map(|line| {
            let value = serde_json::from_str(line).expect("a line of the gh log is JSON");
            Run::from_json(&value).expect("a line of the gh log is a run")
        })
        .collect()
}

/// Forget the runs so far.
pub fn clear_runs(dir: &Path) {
    std::fs::remove_file(runs_file(&dir.join(program_name()))).ok();
}

fn sha256_of(path: &str) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(digest_of(&bytes).trim_start_matches("sha256:").to_string())
}

/// Call first thing in `main`: when this program was started as `gh`, be it,
/// and exit.
pub fn run_if_gh() {
    let Ok(program) = std::env::current_exe() else {
        return;
    };
    if program.file_stem().and_then(|stem| stem.to_str()) != Some("gh") {
        return;
    }
    let behavior = std::fs::read_to_string(behavior_file(&program))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .and_then(|value| Behavior::from_json(&value));
    let Some(behavior) = behavior else {
        eprintln!("this gh has no behavior file");
        std::process::exit(2);
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut run = Run {
        artifact_sha256: args.get(2).and_then(|file| sha256_of(file)),
        bundle_sha256: None,
        token_given: std::env::var_os("GH_TOKEN").is_some_and(|token| !token.is_empty()),
        args,
    };
    run.bundle_sha256 = run.flag("--bundle").and_then(sha256_of);
    if let Ok(mut log) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(runs_file(&program))
    {
        writeln!(log, "{}", run.to_json()).ok();
    }
    let response = respond(&behavior, &run);
    print!("{}", response.stdout);
    eprint!("{}", response.stderr);
    std::io::stdout().flush().ok();
    std::process::exit(response.code);
}

/// A Sigstore bundle (as a registry holds it) whose certificate claims
/// `repository` (`https://github.com/o/r`) as its source repository, or none.
/// Only what lvpm reads of a bundle is real: the certificate is the one
/// extension that names the repository, not an X.509 certificate.
pub fn bundle(repository: Option<&str>) -> Vec<u8> {
    let mut der = Vec::new();
    if let Some(repository) = repository {
        der.extend(SOURCE_REPOSITORY_OID);
        let string = tlv(0x0c, repository.as_bytes());
        der.extend(tlv(0x04, &string));
    }
    let raw = base64::engine::general_purpose::STANDARD.encode(tlv(0x30, &der));
    json!({
        "mediaType": BUNDLE_TYPE,
        "verificationMaterial": { "certificate": { "rawBytes": raw } },
    })
    .to_string()
    .into_bytes()
}

/// `tag`, the DER length of `content`, and `content`.
fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let length = match content.len() {
        n @ 0..=0x7f => vec![n as u8],
        n @ 0x80..=0xff => vec![0x81, n as u8],
        n => [&[0x82][..], &(n as u16).to_be_bytes()].concat(),
    };
    [&[tag][..], &length, content].concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str], token: bool) -> Run {
        Run {
            args: args.iter().map(|arg| arg.to_string()).collect(),
            artifact_sha256: Some("ab".repeat(32)),
            bundle_sha256: Some("cd".repeat(32)),
            token_given: token,
        }
    }

    const VERIFY: [&str; 9] = [
        "attestation",
        "verify",
        "m",
        "--bundle",
        "b.json",
        "--repo",
        "o/r",
        "--format",
        "json",
    ];

    fn verifies() -> Behavior {
        Behavior::Verifies {
            repository: "https://github.com/o/r".into(),
        }
    }

    #[test]
    fn a_bundle_that_verifies_vouches_for_the_sha_of_the_file_and_names_its_builder() {
        let got = respond(&verifies(), &run(&VERIFY, true));
        assert_eq!(got.code, 0, "{got:?}");
        let entries: Value = serde_json::from_str(&got.stdout).unwrap();
        let result = &entries[0]["verificationResult"];
        let certificate = &result["signature"]["certificate"];
        assert_eq!(certificate["sourceRepositoryURI"], "https://github.com/o/r");
        assert_eq!(
            certificate["subjectAlternativeName"],
            "https://github.com/o/r/.github/workflows/release.yml@refs/tags/v1.0.0"
        );
        assert_eq!(
            result["statement"]["subject"][0]["digest"]["sha256"],
            "ab".repeat(32)
        );
    }

    #[test]
    fn gh_refuses_what_real_gh_refuses() {
        let mut other_repo = VERIFY;
        other_repo[6] = "o/other";
        let mut no_bundle = run(&VERIFY, true);
        no_bundle.bundle_sha256 = None;
        let mut no_extension = VERIFY;
        no_extension[4] = "b";
        let table = [
            (run(&other_repo, true), WRONG_REPO),
            (run(&VERIFY, false), NOT_LOGGED_IN),
            (run(&["auth", "token"], true), NOT_LOGGED_IN),
            (no_bundle, "no bundle or no file to verify"),
            (
                run(&no_extension, true),
                "Error: bundle file extension not supported, must be json or jsonl",
            ),
        ];
        for (asked, says) in table {
            let got = respond(&verifies(), &asked);
            assert_eq!(
                (got.code != 0, got.stderr.as_str()),
                (true, says),
                "{asked:?}"
            );
            assert_eq!(got.stdout, "");
        }
        let rejected = Behavior::Rejects {
            stderr: "nope".into(),
        };
        assert_eq!(respond(&rejected, &run(&VERIFY, true)), said("", "nope", 1));
    }

    #[test]
    fn behaviors_and_runs_survive_their_files() {
        for behavior in [verifies(), Behavior::Rejects { stderr: "e".into() }] {
            assert_eq!(Behavior::from_json(&behavior.to_json()), Some(behavior));
        }
        let asked = run(&VERIFY, true);
        assert_eq!(Run::from_json(&asked.to_json()), Some(asked));
    }

    #[test]
    fn a_bundle_names_its_repository_in_a_certificate_lvpm_reads() {
        for repository in [Some("https://github.com/o/r"), None] {
            let found = lvpm_core::oci::sigstore::source_repository(&bundle(repository));
            let wanted = repository.map(|url| url.trim_start_matches(GITHUB).to_string());
            assert_eq!(found, wanted);
        }
    }
}
