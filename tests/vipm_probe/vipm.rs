//! Driving `vipm.exe`: the commands the probe uses, a verdict for an install,
//! and the checks that run before the first one. Only `vipm install -y`,
//! `uninstall`, `info` and `--version` are ever run.

use crate::support::process;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

pub const VIPM_ENV_VAR: &str = "LVPM_TEST_VIPM";
pub const PILOT_A_ENV_VAR: &str = "LVPM_TEST_PILOT_A";

/// The packages a probe may install: pilot A, and what a `Requires` variant
/// makes VIPM fetch with it, in the order to uninstall them.
pub const PILOT_A: &str = "hse_lib_hse_logger";
pub const DATA_MANIPULATION: &str = "labview_open_source_lib_data_manipulation";
pub const PALETTE: &str = "labview_open_source_project_lib_labview_open_source_palette";
pub const TOUCHED: [&str; 3] = [PILOT_A, DATA_MANIPULATION, PALETTE];

/// How a command ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Ran {
    Exited {
        code: i32,
        output: String,
    },
    /// Still running at its deadline, and killed; what it had written.
    TimedOut(String),
    CannotStart(String),
}

/// How long a command may run before it is killed.
#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    pub install: Duration,
    pub other: Duration,
}

impl Timeouts {
    pub const REAL: Timeouts = Timeouts {
        install: Duration::from_secs(600),
        other: Duration::from_secs(120),
    };
}

/// What a command line is run through: arguments and a deadline in, how it ended out.
pub type Runner<'a> = &'a dyn Fn(&[String], Duration) -> Ran;

/// `vipm.exe` as a command closure, so tests run on a fake.
pub fn real_runner(exe: PathBuf) -> impl Fn(&[String], Duration) -> Ran {
    move |args, timeout| {
        let mut command = Command::new(&exe);
        command.args(args);
        match process::run_capturing(command, None, timeout) {
            Ok(out) => Ran::Exited {
                code: out.exit_code,
                output: format!("{}{}", out.stdout, out.stderr),
            },
            Err(failed) if failed.timed_out => {
                Ran::TimedOut(format!("{}{}", failed.stdout, failed.stderr))
            }
            Err(failed) => Ran::CannotStart(failed.message),
        }
    }
}

/// What the probe makes of an install.
#[derive(Debug, Clone, PartialEq)]
pub struct Judged {
    pub accepted: bool,
    pub exit: Option<i32>,
    /// The first line of VIPM's output, or why the probe rejected it.
    pub note: String,
    /// All of VIPM's output.
    pub output: String,
}

pub fn timed_out_note(timeout: Duration) -> String {
    format!("timed out after {}s", timeout.as_secs_f64())
}

pub struct Vipm<'a> {
    run: Runner<'a>,
    timeouts: Timeouts,
}

impl<'a> Vipm<'a> {
    pub fn new(run: Runner<'a>, timeouts: Timeouts) -> Vipm<'a> {
        Vipm { run, timeouts }
    }

    fn exec(&self, args: &[&str], timeout: Duration) -> Ran {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        (self.run)(&args, timeout)
    }

    pub fn install(&self, file: &Path) -> Ran {
        let file = file.to_string_lossy();
        self.exec(&["install", &file, "-y"], self.timeouts.install)
    }

    pub fn uninstall(&self, name: &str) -> Ran {
        self.exec(&["uninstall", name], self.timeouts.other)
    }

    /// Whether `vipm info <name>` lists the package: it exits 0 and its output
    /// holds the name and, when given, the version. Broad on purpose: a false
    /// "listed" only stops the run. An error means the command gave no answer.
    pub fn info(&self, name: &str, version: Option<&str>) -> Result<bool, String> {
        match self.exec(&["info", name], self.timeouts.other) {
            Ran::Exited { code, output } => Ok(code == 0
                && output.contains(name)
                && version.is_none_or(|v| output.contains(v))),
            Ran::TimedOut(_) => Err(format!(
                "`vipm info {name}` timed out after {}s",
                self.timeouts.other.as_secs_f64()
            )),
            Ran::CannotStart(why) => Err(format!("`vipm info {name}`: {why}")),
        }
    }

    pub fn version(&self) -> Result<String, String> {
        match self.exec(&["--version"], self.timeouts.other) {
            Ran::Exited { code: 0, output } => Ok(crate::table::first_line(&output)),
            other => Err(format!("`vipm --version` gave {other:?}")),
        }
    }

    /// Install `file` and say whether VIPM took it: exit 0 and listed is
    /// accepted; exit 0 and not listed, a non-zero exit, a timeout are not.
    pub fn judge_install(&self, file: &Path, name: &str, version: &str) -> Judged {
        let rejected = |exit, note: String, output: String| Judged {
            accepted: false,
            exit,
            note,
            output,
        };
        match self.install(file) {
            Ran::Exited { code: 0, output } => match self.info(name, Some(version)) {
                Ok(true) => Judged {
                    accepted: true,
                    exit: Some(0),
                    note: crate::table::first_line(&output),
                    output,
                },
                Ok(false) => rejected(Some(0), "not listed by vipm info".into(), output),
                Err(why) => rejected(Some(0), why, output),
            },
            Ran::Exited { code, output } => {
                rejected(Some(code), crate::table::first_line(&output), output)
            }
            Ran::TimedOut(output) => rejected(None, timed_out_note(self.timeouts.install), output),
            Ran::CannotStart(why) => rejected(None, format!("could not start: {why}"), why),
        }
    }

    /// Uninstall `name`, or say why the run must stop.
    pub fn remove(&self, name: &str) -> Result<(), String> {
        let command = uninstall_command(name);
        match self.uninstall(name) {
            Ran::Exited { code: 0, .. } => Ok(()),
            Ran::Exited { code, output } => {
                Err(format!("`{command}` failed (exit {code}): {output}"))
            }
            Ran::TimedOut(_) => Err(format!(
                "`{command}` timed out after {}s and was killed; {name} may still be installed",
                self.timeouts.other.as_secs_f64()
            )),
            Ran::CannotStart(why) => Err(format!("`{command}`: {why}")),
        }
    }
}

/// The command that removes `name` by hand.
pub fn uninstall_command(name: &str) -> String {
    format!("vipm uninstall {name}")
}

/// Stop, naming each of `names` that is installed and how to uninstall it.
pub fn preflight(vipm: &Vipm, names: &[&str]) -> Result<(), String> {
    let mut installed = Vec::new();
    for name in names {
        if vipm.info(name, None)? {
            installed.push(format!(
                "{name} is installed; run `{}`",
                uninstall_command(name)
            ));
        }
    }
    match installed.is_empty() {
        true => Ok(()),
        false => Err(format!(
            "stopping before any install: {}",
            installed.join("; ")
        )),
    }
}

/// What the run needs from the environment.
#[derive(Debug)]
pub struct Env {
    pub vipm: PathBuf,
    pub pilot_a: PathBuf,
}

pub fn check_env(get: &dyn Fn(&str) -> Option<OsString>) -> Result<Env, String> {
    let named = |var: &str, what: &str, exists: fn(&Path) -> bool| -> Result<PathBuf, String> {
        let value = get(var).ok_or_else(|| format!("{var} must name {what}; it is not set"))?;
        let path = PathBuf::from(value);
        match exists(&path) {
            true => Ok(path),
            false => Err(format!("{var} must name {what}: {} is not", path.display())),
        }
    };
    Ok(Env {
        vipm: named(VIPM_ENV_VAR, "vipm.exe", Path::is_file)?,
        pilot_a: named(
            PILOT_A_ENV_VAR,
            "a checkout of pilot A (a folder)",
            Path::is_dir,
        )?,
    })
}

/// The line before the first install.
pub fn start_line(variants: usize) -> String {
    format!(
        "probing {variants} variants through VIPM; none of pilot A, Data Manipulation or the \
         palette may be installed on this machine; do not interrupt: if you must, \
         the next start tells you what to uninstall"
    )
}

/// `[7/14] ID=random: installing...`
pub fn progress(index: usize, total: usize, variant: &str, doing: &str) -> String {
    format!("[{index}/{total}] {variant}: {doing}...")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const SHORT: Timeouts = Timeouts {
        install: Duration::from_millis(50),
        other: Duration::from_millis(20),
    };

    #[cfg(windows)]
    const LONG: Duration = Duration::from_secs(60);

    fn exited(code: i32, output: &str) -> Ran {
        Ran::Exited {
            code,
            output: output.into(),
        }
    }

    /// A fake `vipm` that answers by its first argument and logs every call.
    struct Fake {
        calls: RefCell<Vec<String>>,
        /// The deadline each command was given.
        deadlines: RefCell<Vec<(String, Duration)>>,
        install: Ran,
        info: Ran,
        uninstall: Ran,
    }

    impl Fake {
        fn new(install: Ran, info: Ran) -> Fake {
            Fake {
                calls: RefCell::default(),
                deadlines: RefCell::default(),
                install,
                info,
                uninstall: exited(0, ""),
            }
        }

        fn run(&self, args: &[String], timeout: Duration) -> Ran {
            self.calls.borrow_mut().push(args.join(" "));
            self.deadlines.borrow_mut().push((args[0].clone(), timeout));
            let answer = match args[0].as_str() {
                "install" => &self.install,
                "info" => &self.info,
                "uninstall" => &self.uninstall,
                _ => return exited(0, "vipm synthetic 1.0"),
            };
            answer.clone()
        }
    }

    fn with<R>(fake: &Fake, check: impl FnOnce(&Vipm) -> R) -> R {
        let run = |args: &[String], timeout: Duration| fake.run(args, timeout);
        check(&Vipm::new(&run, SHORT))
    }

    #[test]
    fn listed_needs_exit_zero_name_and_version() {
        let cases = [
            (exited(0, "pkg 1.2.3 installed"), true),
            (exited(0, "pkg 9.9.9 installed"), false),
            (exited(0, "other 1.2.3 installed"), false),
            (exited(1, "pkg 1.2.3 installed"), false),
            (exited(0, ""), false),
        ];
        for (info, listed) in cases {
            let fake = Fake::new(exited(0, ""), info.clone());
            let got = with(&fake, |v| v.info("pkg", Some("1.2.3")));
            assert_eq!(got, Ok(listed), "{info:?}");
        }
    }

    #[test]
    fn without_a_version_the_name_is_enough() {
        let fake = Fake::new(exited(0, ""), exited(0, "pkg 9.9.9"));
        assert_eq!(with(&fake, |v| v.info("pkg", None)), Ok(true));
        let fake = Fake::new(exited(0, ""), Ran::TimedOut(String::new()));
        assert!(with(&fake, |v| v.info("pkg", None)).is_err());
    }

    #[test]
    fn an_install_that_exits_zero_and_is_listed_is_accepted() {
        let fake = Fake::new(exited(0, "done\nbye"), exited(0, "pkg 1.0.0.1"));
        let judged = with(&fake, |v| {
            v.judge_install(Path::new("p.vip"), "pkg", "1.0.0.1")
        });
        assert!(judged.accepted, "{judged:?}");
        assert_eq!(judged.exit, Some(0));
        assert_eq!(fake.calls.borrow()[0], "install p.vip -y");
    }

    #[test]
    fn exit_zero_not_listed_is_rejected() {
        let fake = Fake::new(exited(0, "done"), exited(0, "nothing here"));
        let judged = with(&fake, |v| {
            v.judge_install(Path::new("p.vip"), "pkg", "1.0.0.1")
        });
        assert!(!judged.accepted);
        assert_eq!(judged.exit, Some(0));
        assert_eq!(judged.note, "not listed by vipm info");
    }

    #[test]
    fn nonzero_exit_is_rejected_with_its_output() {
        let fake = Fake::new(
            exited(3, "\nboom here\nsecond line"),
            exited(0, "pkg 1.0.0.1"),
        );
        let judged = with(&fake, |v| {
            v.judge_install(Path::new("p.vip"), "pkg", "1.0.0.1")
        });
        assert!(!judged.accepted);
        assert_eq!(judged.exit, Some(3));
        assert_eq!(judged.note, "boom here");
        assert_eq!(judged.output, "\nboom here\nsecond line");
        assert_eq!(
            fake.calls.borrow().len(),
            1,
            "no info after a failed install"
        );
    }

    #[test]
    fn a_command_past_its_timeout_is_killed_and_rejected() {
        let fake = Fake::new(Ran::TimedOut(String::new()), exited(0, "pkg 1.0.0.1"));
        let judged = with(&fake, |v| {
            v.judge_install(Path::new("p.vip"), "pkg", "1.0.0.1")
        });
        assert_eq!(
            *fake.deadlines.borrow(),
            [("install".to_string(), SHORT.install)],
            "an install gets the install deadline"
        );
        assert!(!judged.accepted);
        assert_eq!(judged.exit, None);
        assert_eq!(judged.note, "timed out after 0.05s");
        assert_eq!(
            fake.calls.borrow().len(),
            1,
            "no info after a killed install"
        );
    }

    #[test]
    fn a_timed_out_install_keeps_what_it_wrote() {
        let fake = Fake::new(Ran::TimedOut("half done".into()), exited(0, "pkg 1.0.0.1"));
        let judged = with(&fake, |v| {
            v.judge_install(Path::new("p.vip"), "pkg", "1.0.0.1")
        });
        assert_eq!(judged.output, "half done");
        assert_eq!(judged.note, "timed out after 0.05s");
    }

    /// A stand-in for `vipm.exe`: `cmd` takes the arguments as its own.
    #[cfg(windows)]
    fn through_cmd(args: &[&str], timeout: Duration) -> Ran {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        real_runner(PathBuf::from("cmd"))(&args, timeout)
    }

    #[cfg(windows)]
    #[test]
    fn the_real_runner_maps_an_exit_a_timeout_and_a_missing_program() {
        let Ran::Exited { code, output } = through_cmd(&["/c", "echo", "synthetic"], LONG) else {
            panic!("the command did not exit");
        };
        assert_eq!((code, output.trim()), (0, "synthetic"));
        let Ran::Exited { code, .. } = through_cmd(&["/c", "exit", "3"], LONG) else {
            panic!("the command did not exit");
        };
        assert_eq!(code, 3);

        let started = std::time::Instant::now();
        let ran = through_cmd(
            &[
                "/c",
                "echo",
                "partial",
                "&",
                "ping",
                "-n",
                "30",
                "127.0.0.1",
            ],
            Duration::from_millis(1500),
        );
        assert!(started.elapsed() < Duration::from_secs(10));
        match ran {
            Ran::TimedOut(output) => assert!(output.contains("partial"), "{output}"),
            other => panic!("{other:?}"),
        }

        let missing = real_runner(PathBuf::from("no-such-vipm-lvpm-test"));
        match missing(&["--version".to_string()], LONG) {
            Ran::CannotStart(why) => assert!(why.contains("could not start"), "{why}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn version_is_the_first_line_of_vipm_version() {
        let fake = Fake::new(exited(0, ""), exited(0, ""));
        assert_eq!(
            with(&fake, |v| v.version()),
            Ok("vipm synthetic 1.0".to_string())
        );
        assert_eq!(fake.calls.borrow()[0], "--version");
    }

    #[test]
    fn a_version_that_fails_says_what_vipm_gave() {
        let run = |_: &[String], _: Duration| exited(2, "no such flag");
        let why = Vipm::new(&run, SHORT).version().unwrap_err();
        assert!(
            why.contains("vipm --version") && why.contains("no such flag"),
            "{why}"
        );
    }

    #[test]
    fn a_command_that_cannot_start_is_said_for_each_command() {
        let boom = || Ran::CannotStart("boom".into());
        let fake = Fake::new(boom(), boom());
        let judged = with(&fake, |v| {
            v.judge_install(Path::new("p.vip"), "pkg", "1.0.0.1")
        });
        assert!(!judged.accepted);
        assert_eq!(judged.exit, None);
        assert_eq!(judged.note, "could not start: boom");
        assert_eq!(judged.output, "boom");
        let why = with(&fake, |v| v.info("pkg", None)).unwrap_err();
        assert_eq!(why, "`vipm info pkg`: boom");

        let mut fake = Fake::new(exited(0, ""), exited(0, ""));
        fake.uninstall = boom();
        let why = with(&fake, |v| v.remove("pkg")).unwrap_err();
        assert_eq!(why, "`vipm uninstall pkg`: boom");
        let run = |_: &[String], _: Duration| boom();
        assert!(Vipm::new(&run, SHORT).version().is_err());
    }

    #[test]
    fn an_uninstall_past_its_timeout_stops_the_run() {
        let mut fake = Fake::new(exited(0, ""), exited(0, ""));
        fake.uninstall = Ran::TimedOut(String::new());
        let why = with(&fake, |v| v.remove("pkg")).unwrap_err();
        assert!(why.contains("vipm uninstall pkg"), "{why}");
        assert!(why.contains("timed out after 0.02s"), "{why}");
        assert_eq!(
            *fake.deadlines.borrow(),
            [("uninstall".to_string(), SHORT.other)]
        );
    }

    #[test]
    fn an_uninstall_that_fails_names_the_package_and_vipms_output() {
        let mut fake = Fake::new(exited(0, ""), exited(0, ""));
        fake.uninstall = exited(4, "cannot remove it");
        let why = with(&fake, |v| v.remove("pkg")).unwrap_err();
        assert!(
            why.contains("vipm uninstall pkg") && why.contains("exit 4"),
            "{why}"
        );
        assert!(why.contains("cannot remove it"), "{why}");
        assert_eq!(
            with(&Fake::new(exited(0, ""), exited(0, "")), |v| v
                .remove("pkg")),
            Ok(())
        );
    }

    #[test]
    fn preflight_refuses_when_a_touched_package_is_installed() {
        let run = |args: &[String], _: Duration| match args[1].as_str() {
            "a" | "c" => exited(0, &format!("{} 1.0", args[1])),
            _ => exited(1, ""),
        };
        let vipm = Vipm::new(&run, SHORT);
        let why = preflight(&vipm, &["a", "b", "c"]).unwrap_err();
        assert!(
            why.contains("a is installed") && why.contains("`vipm uninstall a`"),
            "{why}"
        );
        assert!(
            why.contains("c is installed") && why.contains("`vipm uninstall c`"),
            "{why}"
        );
        assert!(!why.contains("b is installed"), "{why}");
        assert_eq!(preflight(&vipm, &["b"]), Ok(()));
    }

    #[test]
    fn missing_env_names_the_variable() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("vipm.exe");
        std::fs::write(&exe, b"x").unwrap();
        let env_of = |vipm: Option<&Path>, pilot: Option<&Path>| {
            let vars: Vec<(&str, OsString)> = [(VIPM_ENV_VAR, vipm), (PILOT_A_ENV_VAR, pilot)]
                .into_iter()
                .filter_map(|(name, path)| Some((name, path?.as_os_str().to_os_string())))
                .collect();
            move |name: &str| {
                vars.iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, value)| value.clone())
            }
        };

        let why = check_env(&env_of(None, Some(dir.path()))).unwrap_err();
        assert!(
            why.contains(VIPM_ENV_VAR) && why.contains("vipm.exe"),
            "{why}"
        );
        let missing = dir.path().join("nope.exe");
        let why = check_env(&env_of(Some(&missing), Some(dir.path()))).unwrap_err();
        assert!(
            why.contains(VIPM_ENV_VAR) && why.contains("nope.exe"),
            "{why}"
        );
        let why = check_env(&env_of(Some(&exe), None)).unwrap_err();
        assert!(why.contains(PILOT_A_ENV_VAR), "{why}");
        let why = check_env(&env_of(Some(&exe), Some(&exe))).unwrap_err();
        assert!(
            why.contains(PILOT_A_ENV_VAR),
            "a file is not a checkout: {why}"
        );
        let env = check_env(&env_of(Some(&exe), Some(dir.path()))).unwrap();
        assert_eq!((env.vipm, env.pilot_a), (exe, dir.path().to_path_buf()));
    }

    #[test]
    fn progress_and_the_start_line_say_what_is_happening() {
        assert_eq!(
            progress(7, 14, "ID=random", "installing"),
            "[7/14] ID=random: installing..."
        );
        let line = start_line(14);
        assert!(
            line.contains("14 variants") && line.contains("do not interrupt"),
            "{line}"
        );
        assert!(
            line.contains("none of pilot A, Data Manipulation or the palette"),
            "{line}"
        );
    }
}
