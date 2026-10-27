//! `publish`: copy the package `lvpm build` wrote for the current manifest
//! into a folder feed, never replacing a file there.
//!
//! The copy is staged beside the target and hard-linked to its final name,
//! which fails if that name exists, so two publishes of one version cannot
//! both land and a reader never sees half a package. `std::fs::rename` would
//! replace an existing file on Windows.

use super::env::OpEnv;
use crate::paths;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

/// What a publish wrote.
#[derive(Debug)]
pub struct Published {
    pub path: PathBuf,
}

pub fn publish(env: &OpEnv, to: &Path) -> Result<Published> {
    let to = paths::resolve(&env.cwd, to);
    let (root, d) = super::packaged(env, "publish")?;
    let name = d.file_name();
    let package = root.join("dist").join(&name);
    if !package.is_file() {
        bail!("{} does not exist: run lvpm build first", package.display());
    }
    if !to.is_dir() {
        bail!(
            "{} is not a folder: lvpm publish puts packages into an existing folder",
            to.display()
        );
    }
    let target = to.join(&name);
    if target.symlink_metadata().is_ok() {
        bail!(already(&target));
    }
    Ok(Published {
        path: stage_and_link(&package, &to, &name)?,
    })
}

fn already(target: &Path) -> String {
    format!(
        "{} is already published; a published version is never replaced",
        target.display()
    )
}

/// Copy `src` to a staged file in `dir`, then give it the name `name`. Makes
/// no check of its own that `name` is free: the link refuses if it is not.
fn stage_and_link(src: &Path, dir: &Path, name: &str) -> Result<PathBuf> {
    let dest = dir.join(name);
    let temp = paths::staging_path(&dest);
    let mut out = std::fs::File::options()
        .write(true)
        .create_new(true)
        .open(&temp)
        .with_context(|| format!("creating {}", temp.display()))?;
    let copied = std::fs::File::open(src)
        .and_then(|mut from| std::io::copy(&mut from, &mut out))
        .and_then(|_| out.sync_all())
        .with_context(|| format!("copying {} to {}", src.display(), temp.display()));
    drop(out);
    let placed = copied.and_then(|()| no_replace_move(&temp, &dest));
    // shortcut: once linked, the publish stands even if the staged name cannot
    // be removed (a share that allows create but not delete); feeds skip `.tmp`.
    let _ = std::fs::remove_file(&temp);
    placed.map(|()| dest)
}

/// Give `temp` the name `dest` unless `dest` exists. A hard link, so it fails
/// on a file system without them; `MoveFileExW` without
/// `MOVEFILE_REPLACE_EXISTING` is the upgrade if a feed needs that.
fn no_replace_move(temp: &Path, dest: &Path) -> Result<()> {
    match std::fs::hard_link(temp, dest) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => bail!(already(dest)),
        Err(e) => bail!(
            "could not place {} in {} (its file system may not support hard links): {e}",
            dest.file_name().unwrap_or_default().to_string_lossy(),
            dest.parent().unwrap_or(dest).display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lvpm-publish-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("feed")).unwrap();
        dir
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut n: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        n.sort();
        n
    }

    #[test]
    fn a_link_that_finds_the_target_taken_leaves_no_staged_file() {
        let dir = folder("taken");
        let src = dir.join("src.vip");
        std::fs::write(&src, "new").unwrap();
        let feed = dir.join("feed");
        let target = feed.join("p-1.0.0.0.vip");
        std::fs::write(&target, "earlier").unwrap();
        let why = format!(
            "{:#}",
            stage_and_link(&src, &feed, "p-1.0.0.0.vip").unwrap_err()
        );
        assert!(why.contains("already published"), "{why}");
        assert!(why.contains(&target.display().to_string()), "{why}");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "earlier");
        assert_eq!(names(&feed), ["p-1.0.0.0.vip"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_failed_copy_leaves_no_staged_file() {
        let dir = folder("copy");
        let feed = dir.join("feed");
        // A folder cannot be read as a file, so the copy fails after the staged file exists.
        assert!(stage_and_link(&dir, &feed, "p-1.0.0.0.vip").is_err());
        assert!(names(&feed).is_empty(), "{:?}", names(&feed));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_link_error_names_the_folder() {
        let dir = folder("link");
        let temp = dir.join("staged.tmp");
        std::fs::write(&temp, "x").unwrap();
        let missing = dir.join("missing");
        let why = format!(
            "{:#}",
            no_replace_move(&temp, &missing.join("p.vip")).unwrap_err()
        );
        assert!(why.contains(&missing.display().to_string()), "{why}");
        assert!(why.contains("hard links"), "{why}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
