//! The fake of a Docker credential helper.
//!
//! A helper is a program called `docker-credential-<name>`; Docker runs it as
//! `docker-credential-<name> get` with the registry host on its standard
//! input. A test cannot make one from a script: Rust's `Command` does not
//! find a `.cmd` through `PATHEXT`, so it must be an executable. [`install`]
//! puts a copy of the running test program in a directory under that name,
//! with a JSON file beside it that says what it does, and a test program that
//! calls [`run_if_helper`] first thing in its `main` turns into the helper when
//! it is started under such a name.
//!
//! Every run of the helper adds the host it was asked for to a `.runs` file
//! beside it, so a test can count them.

use serde_json::{Value, json};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// What every credential helper's name starts with.
pub const PREFIX: &str = "docker-credential-";
/// What a real helper prints, and exits 1 on, when it holds nothing for the host.
pub const NOT_FOUND: &str = "credentials not found in native keychain";
/// How long a helper that never answers sleeps: far more than any test waits.
const HANG: Duration = Duration::from_secs(600);

/// What the fake does when it is asked for a login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Behavior {
    /// Answers with this user name and secret. A user name of `<token>` makes
    /// the secret an identity token, as it does for a real helper.
    Answers { user: String, secret: String },
    /// Holds no login for the host: [`NOT_FOUND`] on stdout, exit 1.
    HoldsNothing,
    /// Exits with `code`, printing `stderr` on its standard error.
    Fails { code: i32, stderr: String },
    /// Never answers.
    Hangs,
}

impl Behavior {
    fn to_json(&self) -> Value {
        match self {
            Behavior::Answers { user, secret } => {
                json!({ "does": "answers", "user": user, "secret": secret })
            }
            Behavior::HoldsNothing => json!({ "does": "holds nothing" }),
            Behavior::Fails { code, stderr } => {
                json!({ "does": "fails", "code": code, "stderr": stderr })
            }
            Behavior::Hangs => json!({ "does": "hangs" }),
        }
    }

    fn from_json(value: &Value) -> Option<Behavior> {
        let text = |key: &str| value[key].as_str().map(str::to_string);
        Some(match value["does"].as_str()? {
            "answers" => Behavior::Answers {
                user: text("user")?,
                secret: text("secret")?,
            },
            "holds nothing" => Behavior::HoldsNothing,
            "fails" => Behavior::Fails {
                code: i32::try_from(value["code"].as_i64()?).ok()?,
                stderr: text("stderr")?,
            },
            "hangs" => Behavior::Hangs,
            _ => return None,
        })
    }
}

/// What a helper writes and how it ends.
#[derive(Debug, PartialEq, Eq)]
struct Response {
    stdout: String,
    stderr: String,
    code: i32,
    hang: bool,
}

/// What the helper does when run with `args` and `stdin`.
fn respond(behavior: &Behavior, args: &[String], stdin: &str) -> Response {
    let said = |stdout: &str, stderr: &str, code: i32| Response {
        stdout: stdout.to_string(),
        stderr: stderr.to_string(),
        code,
        hang: false,
    };
    let host = stdin.trim();
    if args != ["get"] || host.is_empty() {
        return said(
            "",
            "usage: docker-credential-<name> get, with the host on stdin",
            2,
        );
    }
    match behavior {
        Behavior::Answers { user, secret } => {
            let answer = json!({ "ServerURL": host, "Username": user, "Secret": secret });
            said(&answer.to_string(), "", 0)
        }
        Behavior::HoldsNothing => said(NOT_FOUND, "", 1),
        Behavior::Fails { code, stderr } => said("", stderr, *code),
        Behavior::Hangs => Response {
            hang: true,
            ..said("", "", 0)
        },
    }
}

/// The file name of the helper `name`: with `.exe` where programs have it.
fn program_name(name: &str) -> String {
    format!("{PREFIX}{name}{}", std::env::consts::EXE_SUFFIX)
}

fn behavior_file(program: &Path) -> PathBuf {
    program.with_extension("json")
}

fn runs_file(program: &Path) -> PathBuf {
    program.with_extension("runs")
}

/// Make `docker-credential-<name>` in `dir` act as `behavior`: the running
/// test program under that name, and a file that tells it what to do. A helper
/// of that name already there is replaced.
pub fn install(dir: &Path, name: &str, behavior: &Behavior) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let program = dir.join(program_name(name));
    let me = std::env::current_exe()?;
    std::fs::remove_file(&program).ok();
    // A link costs no copy of a test program that can be tens of megabytes.
    std::fs::hard_link(&me, &program).or_else(|_| std::fs::copy(&me, &program).map(drop))?;
    std::fs::write(behavior_file(&program), behavior.to_json().to_string())?;
    std::fs::remove_file(runs_file(&program)).ok();
    Ok(program)
}

/// The hosts `docker-credential-<name>` in `dir` was asked for, one for each
/// run, in order.
pub fn runs(dir: &Path, name: &str) -> Vec<String> {
    let text = std::fs::read_to_string(runs_file(&dir.join(program_name(name))));
    text.unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// Call first thing in `main`: when this program was started as a credential
/// helper, be one, and exit.
pub fn run_if_helper() {
    let Ok(program) = std::env::current_exe() else {
        return;
    };
    let named = program.file_stem().and_then(|stem| stem.to_str());
    if !named.is_some_and(|stem| stem.starts_with(PREFIX)) {
        return;
    }
    let mut stdin = String::new();
    std::io::stdin().read_to_string(&mut stdin).ok();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let behavior = std::fs::read_to_string(behavior_file(&program))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .and_then(|value| Behavior::from_json(&value));
    let Some(behavior) = behavior else {
        eprintln!("this helper has no behavior file");
        std::process::exit(2);
    };
    if let Ok(mut runs) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(runs_file(&program))
    {
        writeln!(runs, "{}", stdin.trim()).ok();
    }
    let response = respond(&behavior, &args, &stdin);
    if response.hang {
        std::thread::sleep(HANG);
    }
    print!("{}", response.stdout);
    eprint!("{}", response.stderr);
    std::io::stdout().flush().ok();
    std::process::exit(response.code);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn the_helper_answers_get_for_the_host_it_is_given_and_nothing_else() {
        let answers = Behavior::Answers {
            user: "u1".into(),
            secret: "s1-7f3a9c-secret".into(),
        };
        let got = respond(&answers, &args(&["get"]), "reg:5000\n");
        let json: Value = serde_json::from_str(&got.stdout).unwrap();
        assert_eq!(
            json,
            json!({ "ServerURL": "reg:5000", "Username": "u1", "Secret": "s1-7f3a9c-secret" })
        );
        assert_eq!((got.code, got.hang), (0, false));
        for wrong in [
            respond(&answers, &args(&["store"]), "reg:5000"),
            respond(&answers, &args(&["get"]), ""),
        ] {
            assert_eq!(wrong.code, 2, "{wrong:?}");
            assert_eq!(wrong.stdout, "");
        }
    }

    #[test]
    fn a_helper_that_holds_nothing_fails_or_hangs_does_so() {
        let get = |behavior: Behavior| respond(&behavior, &args(&["get"]), "r");
        let nothing = get(Behavior::HoldsNothing);
        assert_eq!((nothing.stdout.as_str(), nothing.code), (NOT_FOUND, 1));
        let failed = get(Behavior::Fails {
            code: 3,
            stderr: "token s7 locked".into(),
        });
        assert_eq!(
            (failed.stdout.as_str(), failed.stderr.as_str(), failed.code),
            ("", "token s7 locked", 3)
        );
        assert!(get(Behavior::Hangs).hang);
    }

    #[test]
    fn a_behavior_survives_its_file() {
        for behavior in [
            Behavior::Answers {
                user: "u".into(),
                secret: "s".into(),
            },
            Behavior::HoldsNothing,
            Behavior::Fails {
                code: 3,
                stderr: "e".into(),
            },
            Behavior::Hangs,
        ] {
            assert_eq!(Behavior::from_json(&behavior.to_json()), Some(behavior));
        }
        assert_eq!(Behavior::from_json(&json!({ "does": "sings" })), None);
    }

    #[test]
    fn an_installed_helper_is_a_program_with_a_behavior_file_and_no_runs_yet() {
        let dir = std::env::temp_dir().join(format!("lvpm-helper-test-{}", std::process::id()));
        let program = install(&dir, "lvpmtest", &Behavior::Hangs).unwrap();
        assert!(program.is_file());
        assert!(
            program
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("docker-credential-lvpmtest")
        );
        let file = std::fs::read_to_string(behavior_file(&program)).unwrap();
        assert_eq!(
            Behavior::from_json(&serde_json::from_str(&file).unwrap()),
            Some(Behavior::Hangs)
        );
        assert_eq!(runs(&dir, "lvpmtest"), Vec::<String>::new());
        std::fs::write(runs_file(&program), "a\nb\n").unwrap();
        assert_eq!(runs(&dir, "lvpmtest"), ["a", "b"]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
