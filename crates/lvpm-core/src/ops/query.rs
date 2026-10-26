//! Questions about what lvpm knows, answered as data: the front end decides
//! how many rows to show and what to call them.

use super::env::OpEnv;
use crate::config;
use crate::index::{self, Entry, Import};
use crate::install::{self, Manifest};
use crate::paths;
use crate::target::LvTarget;
use anyhow::Result;
use std::path::Path;

/// The LabVIEW installations detected on this machine, newest first.
pub fn targets<'e>(env: &'e OpEnv) -> &'e [LvTarget] {
    &env.targets
}

/// Every package whose name or display name contains `query`, newest version
/// of each, by name. All of them: a front end that shows fewer says so itself.
pub fn search(env: &OpEnv, query: &str) -> Result<Vec<Entry>> {
    let manifest = env.manifest_for(None, None)?;
    let index = env.load_index(manifest.as_ref())?;
    Ok(index.search(query).into_iter().cloned().collect())
}

/// Every version of every package the sources list, by name and then newest
/// first, narrowed to names or display names containing `query` in any case.
/// A source that cannot be read is named in `failed` and the others are still
/// listed; when every source fails, so does this.
pub fn catalog(env: &OpEnv, query: Option<&str>) -> Result<Catalog> {
    let manifest = env.manifest_for(None, None)?;
    let mut failed = Vec::new();
    let loaded = env.load_index_each(manifest.as_ref(), |source, error| {
        failed.push((source.to_string(), format!("{error:#}")));
        Ok(())
    })?;
    if !failed.is_empty() && failed.len() == loaded.sources.len() {
        let why: Vec<String> = failed.iter().map(|(s, e)| format!("{s}: {e}")).collect();
        anyhow::bail!("no package source could be read:\n  {}", why.join("\n  "));
    }
    let mut entries: Vec<Entry> = loaded
        .index
        .matching(query.unwrap_or(""))
        .cloned()
        .collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name).then(b.version.cmp(&a.version)));
    Ok(Catalog {
        entries,
        sources: loaded.sources,
        failed,
    })
}

/// The answer to [`catalog`].
#[derive(Debug)]
pub struct Catalog {
    pub entries: Vec<Entry>,
    /// Every source asked: each local folder, then each feed's index URL.
    pub sources: Vec<String>,
    /// The sources that could not be read, with why.
    pub failed: Vec<(String, String)>,
}

/// What lvpm has installed into the selected destination.
pub fn list(env: &OpEnv) -> Result<Vec<Manifest>> {
    install::list_installed(&env.destination()?.store())
}

/// The effective value of a setting, one string per value.
pub fn config_get(env: &OpEnv, key: &str) -> Result<Vec<String>> {
    config::get(&env.config, key)
}

/// Change a user-level setting. Writes the user's config file; `env.config`
/// is what the command started with and is not updated. A relative path is
/// taken from the directory the command was run from.
pub fn config_set(env: &OpEnv, key: &str, values: &[String]) -> Result<()> {
    config::set(key, values, &env.cwd)
}

/// Import a package archive, or the packages directly inside a directory, into
/// the shared cache. A relative `path` is taken from the directory the command
/// was run from.
pub fn cache_add(env: &OpEnv, path: &Path) -> Result<Vec<Import>> {
    index::cache_add(
        &env.config.cache,
        &paths::resolve(&env.cwd, path),
        env.refresh,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::events::NullSink;
    use std::path::PathBuf;

    fn env_with(cache: &str, venv_dir: &str, targets: Vec<LvTarget>) -> OpEnv<'static> {
        let config = Config {
            cache: PathBuf::from(cache),
            local_sources: vec![PathBuf::from("packages-a"), PathBuf::from("packages-b")],
            venv_dir: PathBuf::from(venv_dir),
        };
        OpEnv::new(&NullSink, config, targets, PathBuf::from("."))
    }

    #[test]
    fn the_detected_installations_are_the_ones_the_environment_was_given() {
        let labview = LvTarget {
            version: 26.3,
            bitness: 64,
            path: PathBuf::from("LabVIEW 2026"),
        };
        let env = env_with("cache", ".lv-venv", vec![labview.clone()]);
        assert_eq!(targets(&env).len(), 1);
        assert_eq!(targets(&env)[0].path, labview.path);
        assert!(targets(&env_with("cache", ".lv-venv", Vec::new())).is_empty());
    }

    #[test]
    fn cache_add_takes_a_relative_path_from_the_environments_directory_not_the_processs() {
        use std::io::Write;

        let cwd = super::super::testing::scratch_dir("cache-add-cwd");
        std::fs::create_dir_all(cwd.join("pkgs")).unwrap();
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        writer
            .start_file("spec", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer
            .write_all(b"[Package]\nName=alpha\nVersion=1.0\n")
            .unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        std::fs::write(cwd.join("pkgs").join("alpha-1.0.vip"), bytes).unwrap();

        let config = Config {
            cache: cwd.join("cache"),
            local_sources: Vec::new(),
            venv_dir: PathBuf::from(".lv-venv"),
        };
        let env = OpEnv::new(&NullSink, config, Vec::new(), cwd.clone());
        let imports = cache_add(&env, Path::new("pkgs")).unwrap();
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].path, cwd.join("pkgs").join("alpha-1.0.vip"));
        let _ = std::fs::remove_dir_all(&cwd);
    }

    /// A project that reads only `folders`, each holding (name, version,
    /// display name) packages, in a scratch directory the caller removes.
    fn catalog_env(tag: &str, folders: &[&[(&str, &str, &str)]]) -> (PathBuf, OpEnv<'static>) {
        use std::io::Write;

        let cwd = super::super::testing::scratch_dir(tag);
        let manifest = "[project]\nlabview = \"2024\"\n\n[sources]\ndefaults = false\n";
        std::fs::write(cwd.join("lvpm.toml"), manifest).unwrap();
        let mut local_sources = Vec::new();
        for (i, packages) in folders.iter().enumerate() {
            let dir = cwd.join(format!("folder-{i}"));
            std::fs::create_dir_all(&dir).unwrap();
            for (name, version, display) in packages.iter() {
                let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
                writer
                    .start_file("spec", zip::write::SimpleFileOptions::default())
                    .unwrap();
                let spec = format!(
                    "[Package]\nName={name}\nVersion={version}\nDisplay Name=\"{display}\"\n"
                );
                writer.write_all(spec.as_bytes()).unwrap();
                let bytes = writer.finish().unwrap().into_inner();
                std::fs::write(dir.join(format!("{name}-{version}.vip")), bytes).unwrap();
            }
            local_sources.push(dir);
        }
        let config = Config {
            cache: cwd.join("cache"),
            local_sources,
            venv_dir: PathBuf::from(".lv-venv"),
        };
        let env = OpEnv::new(&NullSink, config, Vec::new(), cwd.clone());
        (cwd, env)
    }

    fn rows(catalog: &Catalog) -> Vec<String> {
        let row = |e: &Entry| format!("{} {}", e.name, e.version);
        catalog.entries.iter().map(row).collect()
    }

    #[test]
    fn the_catalog_lists_every_version_from_every_source_by_name_then_newest_first() {
        let (cwd, env) = catalog_env(
            "catalog-all",
            &[
                &[("beta", "0.1", "Beta"), ("alpha", "1.0", "Alpha")],
                &[("alpha", "2.0", "Alpha")],
            ],
        );
        let catalog = catalog(&env, None).unwrap();
        assert_eq!(rows(&catalog), ["alpha 2.0", "alpha 1.0", "beta 0.1"]);
        let folder = |i: usize| {
            cwd.join(format!("folder-{i}"))
                .to_string_lossy()
                .into_owned()
        };
        assert_eq!(catalog.entries[0].source, folder(1));
        assert_eq!(catalog.entries[1].source, folder(0));
        assert_eq!(catalog.sources, [folder(0), folder(1)]);
        assert!(catalog.failed.is_empty());
        let _ = std::fs::remove_dir_all(&cwd);
    }

    #[test]
    fn a_query_narrows_the_catalog_by_name_or_display_name_in_any_case() {
        let packages = [("alpha", "1.0", "Alpha Tools"), ("beta", "0.1", "Beta")];
        let (cwd, env) = catalog_env("catalog-query", &[&packages]);
        for query in ["alp", "ALP", "tools"] {
            let catalog = catalog(&env, Some(query)).unwrap();
            assert_eq!(rows(&catalog), ["alpha 1.0"], "{query}");
        }
        assert!(catalog(&env, Some("zzz")).unwrap().entries.is_empty());
        let _ = std::fs::remove_dir_all(&cwd);
    }

    #[test]
    fn a_source_that_cannot_be_read_is_named_and_the_others_are_still_listed() {
        let (cwd, mut env) = catalog_env("catalog-failed", &[&[("beta", "0.1", "Beta")]]);
        let missing = cwd.join("missing");
        env.config.local_sources.insert(0, missing.clone());
        let catalog = catalog(&env, None).unwrap();
        assert_eq!(rows(&catalog), ["beta 0.1"]);
        assert_eq!(catalog.failed.len(), 1);
        assert_eq!(catalog.failed[0].0, missing.to_string_lossy());
        assert!(catalog.failed[0].1.contains("is not a directory"));
        let _ = std::fs::remove_dir_all(&cwd);
    }

    #[test]
    fn the_catalog_fails_when_no_source_can_be_read() {
        let (cwd, mut env) = catalog_env("catalog-none-read", &[]);
        env.config.local_sources = vec![cwd.join("missing")];
        let why = catalog(&env, None).unwrap_err().to_string();
        assert!(why.starts_with("no package source could be read"), "{why}");
        assert!(why.contains("missing"), "{why}");
        let _ = std::fs::remove_dir_all(&cwd);
    }

    #[test]
    fn an_empty_catalog_still_names_the_sources_it_asked() {
        let (cwd, env) = catalog_env("catalog-empty", &[&[]]);
        let catalog = catalog(&env, None).unwrap();
        assert!(catalog.entries.is_empty());
        assert_eq!(catalog.sources.len(), 1);
        assert!(catalog.failed.is_empty());
        let _ = std::fs::remove_dir_all(&cwd);
    }

    #[test]
    fn a_setting_is_read_from_the_environments_config_not_the_machines() {
        let env = env_with("the-cache", ".the-venv", Vec::new());
        assert_eq!(config_get(&env, "venv.dir").unwrap(), [".the-venv"]);
        assert_eq!(
            config_get(&env, "sources.local").unwrap(),
            ["packages-a", "packages-b"]
        );
        let cache = config_get(&env, "cache").unwrap();
        assert_eq!(cache, [PathBuf::from("the-cache").to_string_lossy()]);
        let why = config_get(&env, "nothing").unwrap_err().to_string();
        assert!(why.starts_with("unknown config key \"nothing\""), "{why}");
    }
}
