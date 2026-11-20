//! lvpm's settings, resolved the way npm resolves its own: `LVPM_CONFIG_<KEY>`
//! in the environment (`lvpm_config_<key>` works too), else the built-in default.

// This is one of the two loaders that read the environment (the other is
// `target`); the rest of core is handed what it needs.
#![allow(clippy::disallowed_methods)]

use crate::paths;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub(crate) const DEFAULT_VENV_DIR: &str = ".lv-venv";

/// The keys `config set` and `config get` accept, as the errors list them.
const SUPPORTED_KEYS: &str = "cache, hooks.run, log.dir, sources.local, venv.dir, venv.title";

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Where downloaded indexes (and later, packages) are cached.
    pub cache: PathBuf,
    /// Names of the packages whose install hooks may run in a global install with no project.
    pub hooks_run: Vec<String>,
    /// Where lvpm writes its logs: one per venv, `lvpm.log`, `tui.log`.
    pub log_dir: PathBuf,
    /// Local directories scanned for package files.
    pub local_sources: Vec<PathBuf>,
    /// Project-relative venv directory, unless configured as an absolute path.
    pub venv_dir: PathBuf,
    /// Where `lvpm launch` puts the venv's name in LabVIEW's window titles.
    pub venv_title: TitlePosition,
}

/// Where the venv's name goes in a venv LabVIEW's window titles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TitlePosition {
    /// `[name] x.vi Front Panel …`
    Start,
    /// `x.vi Front Panel … [name]`, before LabVIEW's `*` when there is one.
    #[default]
    End,
    /// Titles are left as LabVIEW writes them.
    Off,
}

impl TitlePosition {
    pub const VALUES: &str = "end, start, off";

    pub fn as_str(self) -> &'static str {
        match self {
            TitlePosition::Start => "start",
            TitlePosition::End => "end",
            TitlePosition::Off => "off",
        }
    }
}

impl std::str::FromStr for TitlePosition {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "start" => Ok(TitlePosition::Start),
            "end" => Ok(TitlePosition::End),
            "off" => Ok(TitlePosition::Off),
            _ => bail!(
                "venv.title {s:?}: expected one of {}",
                TitlePosition::VALUES
            ),
        }
    }
}

pub fn load() -> Result<Config> {
    let file = read_file()?;
    resolve(&file, |k| std::env::var(k).ok())
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct FileConfig {
    cache: Option<String>,
    hooks: HooksConfig,
    log: LogConfig,
    sources: SourcesConfig,
    venv: VenvConfig,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct HooksConfig {
    run: Vec<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct LogConfig {
    dir: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct SourcesConfig {
    local: Vec<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct VenvConfig {
    dir: Option<String>,
    title: Option<String>,
}

fn resolve(file: &FileConfig, env: impl Fn(&str) -> Option<String>) -> Result<Config> {
    let cache = match env_value(&env, "cache") {
        Some(v) => std::path::absolute(v)?,
        None => match &file.cache {
            Some(v) => std::path::absolute(v)?,
            None => default_cache(),
        },
    };
    // An empty value is an empty list, not "unset": it switches the file's list off.
    let hooks_run = match env_raw(&env, "hooks.run") {
        Some(v) => split_names(&v),
        None => file.hooks.run.clone(),
    };
    let log_dir = match env_value(&env, "log.dir").or_else(|| file.log.dir.clone()) {
        Some(v) => std::path::absolute(v)?,
        None => default_log_dir(),
    };
    let local_sources = match env_value(&env, "sources.local") {
        Some(v) => std::env::split_paths(&v).collect(),
        None => file.sources.local.iter().map(PathBuf::from).collect(),
    };
    let venv_dir = env_value(&env, "venv.dir")
        .or_else(|| file.venv.dir.clone().filter(|v| !v.is_empty()))
        .unwrap_or_else(|| DEFAULT_VENV_DIR.to_string());
    let venv_title = match env_value(&env, "venv.title")
        .or_else(|| file.venv.title.clone().filter(|v| !v.is_empty()))
    {
        Some(v) => v.parse()?,
        None => TitlePosition::default(),
    };
    Ok(Config {
        cache,
        hooks_run,
        log_dir,
        local_sources,
        venv_dir: PathBuf::from(venv_dir),
        venv_title,
    })
}

/// Write one setting to the user's config file. The local source directories
/// are stored absolute: a relative one is taken from `cwd`, the directory the
/// command was run from.
pub fn set(key: &str, values: &[String], cwd: &Path) -> Result<()> {
    let mut file = read_file()?;
    apply(&mut file, key, values, cwd)?;
    let path = config_file();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(&file)?;
    std::fs::write(&path, text).with_context(|| format!("writing config {}", path.display()))
}

fn apply(file: &mut FileConfig, key: &str, values: &[String], cwd: &Path) -> Result<()> {
    match key {
        "cache" if values.len() == 1 => file.cache = Some(values[0].clone()),
        "hooks.run" => file.hooks.run = values.iter().flat_map(|v| split_names(v)).collect(),
        "log.dir" if values.len() == 1 => {
            file.log.dir = Some(
                paths::resolve(cwd, Path::new(&values[0]))
                    .to_string_lossy()
                    .into_owned(),
            )
        }
        "sources.local" => {
            file.sources.local = values
                .iter()
                .map(|v| {
                    paths::resolve(cwd, Path::new(v))
                        .to_string_lossy()
                        .into_owned()
                })
                .collect();
        }
        "venv.dir" if values.len() == 1 => file.venv.dir = Some(values[0].clone()),
        "venv.title" if values.len() == 1 => {
            let pos: TitlePosition = values[0].parse()?;
            file.venv.title = Some(pos.as_str().to_string())
        }
        "cache" => bail!("`config set cache` expects one value"),
        "log.dir" => bail!("`config set log.dir` expects one value"),
        "venv.dir" => bail!("`config set venv.dir` expects one value"),
        "venv.title" => bail!(
            "`config set venv.title` expects one value: {}",
            TitlePosition::VALUES
        ),
        _ => bail!("unknown config key {key:?}; supported keys: {SUPPORTED_KEYS}"),
    }
    Ok(())
}

pub fn get(cfg: &Config, key: &str) -> Result<Vec<String>> {
    match key {
        "cache" => Ok(vec![cfg.cache.to_string_lossy().into_owned()]),
        "hooks.run" => Ok(cfg.hooks_run.clone()),
        "log.dir" => Ok(vec![cfg.log_dir.to_string_lossy().into_owned()]),
        "sources.local" => Ok(cfg
            .local_sources
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect()),
        "venv.dir" => Ok(vec![cfg.venv_dir.to_string_lossy().into_owned()]),
        "venv.title" => Ok(vec![cfg.venv_title.as_str().to_string()]),
        _ => bail!("unknown config key {key:?}; supported keys: {SUPPORTED_KEYS}"),
    }
}

fn read_file() -> Result<FileConfig> {
    let path = config_file();
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            toml::from_str(&text).with_context(|| format!("parsing config {}", path.display()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(FileConfig::default()),
        Err(e) => Err(e).with_context(|| format!("reading config {}", path.display())),
    }
}

fn config_file() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);

    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(std::env::temp_dir);

    base.join("lvpm").join("config.toml")
}

/// `LVPM_CONFIG_<KEY>`, else `lvpm_config_<key>`; empty counts as unset.
fn env_value(env: &impl Fn(&str) -> Option<String>, key: &str) -> Option<String> {
    env_raw(env, key).filter(|v| !v.is_empty())
}

/// [`env_value`], but an empty value is kept.
fn env_raw(env: &impl Fn(&str) -> Option<String>, key: &str) -> Option<String> {
    let upper = format!("LVPM_CONFIG_{}", key.replace('.', "_").to_uppercase());
    let lower = format!("lvpm_config_{}", key.replace('.', "_").to_lowercase());
    env(&upper).or_else(|| env(&lower))
}

/// Package names separated by commas, whitespace, or both; empty names dropped.
fn split_names(text: &str) -> Vec<String> {
    text.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

/// `%LOCALAPPDATA%\lvpm\logs`; `$XDG_STATE_HOME/lvpm/logs` elsewhere.
#[cfg(windows)]
fn default_log_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("lvpm")
        .join("logs")
}

#[cfg(not(windows))]
fn default_log_dir() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("state")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("lvpm").join("logs")
}

/// Per-user, not per-target — the feeds are the same for every LabVIEW.
#[cfg(windows)]
fn default_cache() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("lvpm")
        .join("cache")
}

#[cfg(not(windows))]
fn default_cache() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("lvpm").join("cache")
}

#[cfg(test)]
mod tests {
    #[test]
    fn log_dir_defaults_beside_the_config_and_reads_the_environment_first() {
        let none = |_: &str| None;
        assert_eq!(
            resolve(&FileConfig::default(), none).unwrap().log_dir,
            default_log_dir()
        );
        let file = FileConfig {
            log: LogConfig {
                dir: Some("from-file".into()),
            },
            ..FileConfig::default()
        };
        assert!(resolve(&file, none).unwrap().log_dir.ends_with("from-file"));
        let env = |k: &str| (k == "LVPM_CONFIG_LOG_DIR").then(|| "from-env".to_string());
        assert!(resolve(&file, env).unwrap().log_dir.ends_with("from-env"));
    }

    #[test]
    fn venv_title_defaults_to_end_reads_the_environment_first_and_refuses_others() {
        let file = FileConfig {
            venv: VenvConfig {
                dir: None,
                title: Some("start".into()),
            },
            ..FileConfig::default()
        };
        let none = |_: &str| None;
        assert_eq!(
            resolve(&FileConfig::default(), none).unwrap().venv_title,
            TitlePosition::End
        );
        assert_eq!(
            resolve(&file, none).unwrap().venv_title,
            TitlePosition::Start
        );
        let env = |k: &str| (k == "LVPM_CONFIG_VENV_TITLE").then(|| "off".to_string());
        assert_eq!(resolve(&file, env).unwrap().venv_title, TitlePosition::Off);
        let bad = |k: &str| (k == "LVPM_CONFIG_VENV_TITLE").then(|| "middle".to_string());
        assert!(resolve(&file, bad).is_err());
    }

    use super::*;

    #[test]
    fn default_when_nothing_is_set() {
        let cfg = resolve(&FileConfig::default(), |_| None).unwrap();
        assert_eq!(cfg.cache, default_cache());
        assert!(cfg.local_sources.is_empty());
        assert!(cfg.hooks_run.is_empty());
        assert_eq!(cfg.venv_dir, PathBuf::from(DEFAULT_VENV_DIR));
    }

    #[test]
    fn env_overrides_default_in_either_case_and_empty_is_unset() {
        let upper = |k: &str| (k == "LVPM_CONFIG_CACHE").then(|| "/from/env".to_string());
        assert_eq!(
            resolve(&FileConfig::default(), upper).unwrap().cache,
            std::path::absolute("/from/env").unwrap()
        );

        let lower = |k: &str| (k == "lvpm_config_cache").then(|| "/lower".to_string());
        assert_eq!(
            resolve(&FileConfig::default(), lower).unwrap().cache,
            std::path::absolute("/lower").unwrap()
        );

        let empty = |k: &str| (k == "LVPM_CONFIG_CACHE").then(String::new);
        assert_eq!(
            resolve(&FileConfig::default(), empty).unwrap().cache,
            default_cache()
        );
    }

    #[test]
    fn file_settings_are_overridden_by_environment_values() {
        let file = FileConfig {
            cache: Some("from-file".into()),
            hooks: HooksConfig::default(),
            log: LogConfig::default(),
            sources: SourcesConfig {
                local: vec!["packages-a".into()],
            },
            venv: VenvConfig {
                title: None,
                dir: Some(".from-file".into()),
            },
        };
        let from_file = resolve(&file, |_| None).unwrap();
        assert_eq!(from_file.venv_dir, PathBuf::from(".from-file"));
        let cfg = resolve(&file, |key| match key {
            "LVPM_CONFIG_CACHE" => Some("from-env".into()),
            "LVPM_CONFIG_SOURCES_LOCAL" => Some("packages-b".into()),
            "LVPM_CONFIG_VENV_DIR" => Some(".custom-venv".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(cfg.cache, std::path::absolute("from-env").unwrap());
        assert_eq!(
            cfg.local_sources,
            std::env::split_paths("packages-b").collect::<Vec<_>>()
        );
        assert_eq!(cfg.venv_dir, PathBuf::from(".custom-venv"));
    }

    #[test]
    fn a_relative_local_source_is_stored_from_the_directory_the_command_ran_in() {
        let cwd = std::env::temp_dir().join("lvpm-config-cwd");
        let mut file = FileConfig::default();
        let elsewhere = std::env::temp_dir().join("lvpm-config-elsewhere");
        let values = vec!["packages-a".to_string(), elsewhere.to_string_lossy().into()];
        apply(&mut file, "sources.local", &values, &cwd).unwrap();
        assert_eq!(
            file.sources.local,
            [
                cwd.join("packages-a").to_string_lossy(),
                elsewhere.to_string_lossy()
            ]
        );
        apply(&mut file, "sources.local", &[], &cwd).unwrap();
        assert!(file.sources.local.is_empty());
        assert!(apply(&mut file, "cache", &[], &cwd).is_err());
        assert!(apply(&mut file, "nothing", &[], &cwd).is_err());
    }

    #[test]
    fn config_serializes_local_sources_as_a_list() {
        let file = FileConfig {
            cache: None,
            hooks: HooksConfig::default(),
            log: LogConfig::default(),
            sources: SourcesConfig {
                local: vec!["one".into(), "two".into()],
            },
            venv: VenvConfig::default(),
        };
        let encoded = toml::to_string_pretty(&file).unwrap();
        let decoded: FileConfig = toml::from_str(&encoded).unwrap();
        assert_eq!(decoded.sources.local, ["one", "two"]);
    }

    fn hooks_run(file: &FileConfig, env_value: Option<&str>) -> Vec<String> {
        let env = |k: &str| {
            (k == "LVPM_CONFIG_HOOKS_RUN")
                .then(|| env_value.map(str::to_string))
                .flatten()
        };
        resolve(file, env).unwrap().hooks_run
    }

    fn file_running(names: &[&str]) -> FileConfig {
        FileConfig {
            hooks: HooksConfig {
                run: names.iter().map(|n| n.to_string()).collect(),
            },
            ..FileConfig::default()
        }
    }

    #[test]
    fn hooks_run_is_read_from_the_file() {
        assert_eq!(hooks_run(&file_running(&["a", "b"]), None), ["a", "b"]);
        let decoded: FileConfig = toml::from_str("[hooks]\nrun = [\"pilot_a\"]").unwrap();
        assert_eq!(hooks_run(&decoded, None), ["pilot_a"]);
    }

    #[test]
    fn hooks_run_from_the_environment_overrides_the_file() {
        let file = file_running(&["from_file"]);
        assert_eq!(hooks_run(&file, Some("from_env")), ["from_env"]);
        let lower = |k: &str| (k == "lvpm_config_hooks_run").then(|| "lower".to_string());
        assert_eq!(resolve(&file, lower).unwrap().hooks_run, ["lower"]);
    }

    #[test]
    fn hooks_run_names_split_on_commas_and_whitespace_and_drop_empty_ones() {
        let none = FileConfig::default();
        assert_eq!(hooks_run(&none, Some("a,,b")), ["a", "b"]);
        assert_eq!(hooks_run(&none, Some(" a ")), ["a"]);
        assert_eq!(hooks_run(&none, Some("a b,\tc\n")), ["a", "b", "c"]);
        assert!(hooks_run(&file_running(&["from_file"]), Some(" , ")).is_empty());
    }

    #[test]
    fn an_empty_hooks_run_environment_value_is_an_empty_list_that_overrides_the_file() {
        assert!(hooks_run(&file_running(&["from_file"]), Some("")).is_empty());
    }

    #[test]
    fn config_set_hooks_run_replaces_the_list_and_splits_each_value() {
        let cwd = std::env::temp_dir();
        let mut file = file_running(&["old"]);
        let values = ["a,b".to_string(), " c ".to_string()];
        apply(&mut file, "hooks.run", &values, &cwd).unwrap();
        assert_eq!(file.hooks.run, ["a", "b", "c"]);
        apply(&mut file, "hooks.run", &[], &cwd).unwrap();
        assert!(file.hooks.run.is_empty());
    }

    #[test]
    fn config_get_hooks_run_lists_the_names_and_unknown_keys_list_the_supported_ones() {
        let keys = "supported keys: cache, hooks.run, log.dir, sources.local, venv.dir, venv.title";
        let cfg = resolve(&file_running(&["a", "b"]), |_| None).unwrap();
        assert_eq!(get(&cfg, "hooks.run").unwrap(), ["a", "b"]);
        let why = get(&cfg, "hooks.allow").unwrap_err().to_string();
        assert!(why.ends_with(keys), "{why}");
        let cwd = std::env::temp_dir();
        let mut file = FileConfig::default();
        let why = apply(&mut file, "hooks.allow", &[], &cwd).unwrap_err();
        assert!(why.to_string().ends_with(keys), "{why}");
    }
}
