//! `publish`: copy the package `lvpm build` wrote for the current manifest
//! into a folder feed, never replacing a file there; or push it to an OCI
//! registry (see [`oci`]).
//!
//! The copy is staged beside the target and hard-linked to its final name,
//! which fails if that name exists, so two publishes of one version cannot
//! both land and a reader never sees half a package. `std::fs::rename` would
//! replace an existing file on Windows.

mod oci;

use super::env::OpEnv;
use crate::oci::{FeedUrl, Reference, UrlError};
use crate::paths;
use anyhow::{Context, Result, bail};
use std::fmt::Display;
use std::path::{Path, PathBuf};

/// Where `lvpm publish --to` puts the package.
#[derive(Debug, Clone)]
pub enum PublishTarget {
    Folder(PathBuf),
    Oci(FeedUrl),
}

impl PublishTarget {
    /// `--to` as given. A value that starts with `oci://` is an OCI feed and
    /// is never read as a path (Windows would collapse its `//`); anything
    /// else is a folder. An error never repeats the value: it may carry a
    /// secret.
    pub fn from_arg(arg: &Path) -> Result<PublishTarget> {
        let url = arg
            .to_str()
            .filter(|text| text.starts_with(crate::oci::reference::SCHEME));
        let Some(url) = url else {
            return Ok(PublishTarget::Folder(arg.to_path_buf()));
        };
        match FeedUrl::parse(url) {
            Ok(feed) => Ok(PublishTarget::Oci(feed)),
            Err(UrlError::Credentials { registry }) => bail!(
                "--to: credentials do not belong in the target; remove the user:password@ part \
                 and run docker login {registry}"
            ),
            Err(other) => bail!("--to: {other}"),
        }
    }
}

/// What a publish wrote.
#[derive(Debug)]
pub enum Published {
    Folder {
        path: PathBuf,
    },
    Oci {
        name: String,
        version: String,
        reference: Reference,
        /// The digest of the OCI manifest, which a signature of the package signs.
        digest: String,
    },
}

pub fn publish(env: &OpEnv, to: &PublishTarget) -> Result<Published> {
    let (root, d, _) = super::packaged(env, "publish")?;
    let package = super::package_path(&root, &d);
    built(&package)?;
    match to {
        PublishTarget::Folder(to) => folder(env, to, &d.file_name(), &package),
        PublishTarget::Oci(feed) => oci::publish(env, feed, &package),
    }
}

fn folder(env: &OpEnv, to: &Path, name: &str, package: &Path) -> Result<Published> {
    let to = paths::resolve(&env.cwd, to);
    if !to.is_dir() {
        bail!(
            "{} is not a folder: lvpm publish puts packages into an existing folder",
            to.display()
        );
    }
    let target = to.join(name);
    if target.symlink_metadata().is_ok() {
        bail!(already_published(&target.display(), ""));
    }
    Ok(Published::Folder {
        path: stage_and_link(package, &to, name)?,
    })
}

/// `package` is a file lvpm build wrote. The file itself is not followed: a
/// link in a fresh checkout could copy any file onto the share.
fn built(package: &Path) -> Result<()> {
    match package.symlink_metadata() {
        Ok(m) if m.is_file() => Ok(()),
        Ok(m) if m.is_symlink() => bail!(
            "{} is a link, not a package lvpm build wrote: run lvpm build",
            package.display()
        ),
        _ => bail!("{} does not exist: run lvpm build first", package.display()),
    }
}

/// `<what> is already published<at>; a published version is never replaced`.
fn already_published(what: &dyn Display, at: &str) -> String {
    format!("{what} is already published{at}; a published version is never replaced")
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
    let placed = copied.and_then(|()| link_unless_exists(&temp, &dest));
    // shortcut: a share that allows create but not delete keeps the staged
    // name (feeds skip `.tmp`); clean up there if one ever shows up in a feed.
    let _ = std::fs::remove_file(&temp);
    placed.map(|()| dest)
}

/// Give `temp` the name `dest` unless `dest` exists. A hard link, so it fails
/// on a file system without them; `MoveFileExW` without
/// `MOVEFILE_REPLACE_EXISTING` is the upgrade if a feed needs that.
fn link_unless_exists(temp: &Path, dest: &Path) -> Result<()> {
    match std::fs::hard_link(temp, dest) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            bail!(already_published(&dest.display(), ""))
        }
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
    fn a_target_that_is_not_an_oci_url_is_a_folder_however_it_is_spelled() {
        for arg in [
            "feed",
            "../feed",
            r"C:\feeds\a",
            "//server/share",
            "oci:/x",
            "OCI://h/p",
        ] {
            let target = PublishTarget::from_arg(Path::new(arg)).unwrap();
            assert!(
                matches!(&target, PublishTarget::Folder(p) if p == Path::new(arg)),
                "{arg}"
            );
        }
    }

    #[test]
    fn an_oci_target_is_a_feed_and_keeps_its_double_slash() {
        let target = PublishTarget::from_arg(Path::new("oci://127.0.0.1:5000/team")).unwrap();
        let PublishTarget::Oci(feed) = target else {
            panic!("not an OCI feed");
        };
        assert_eq!(feed.to_string(), "oci://127.0.0.1:5000/team");
    }

    #[test]
    fn a_target_with_credentials_or_no_prefix_is_refused_without_repeating_it() {
        let refused = |arg: &str| {
            let why = PublishTarget::from_arg(Path::new(arg)).unwrap_err();
            let why = format!("{why:#}");
            assert!(!why.contains("7f3a9c"), "{why}");
            why
        };
        assert_eq!(
            refused("oci://u-4c1d2e:pw-7f3a9c-secret@ghcr.io/org"),
            "--to: credentials do not belong in the target; remove the user:password@ part and \
             run docker login ghcr.io"
        );
        assert_eq!(
            refused("oci://ghcr.io"),
            "--to: an OCI feed is oci://host/prefix"
        );
        assert_eq!(
            refused("oci://pw-7f3a9c:x/y@ghcr.io/org"),
            "--to: the registry must be host or host:port"
        );
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
        let why = format!(
            "{:#}",
            stage_and_link(&dir, &feed, "p-1.0.0.0.vip").unwrap_err()
        );
        assert!(why.contains("copying"), "{why}");
        assert!(names(&feed).is_empty(), "{:?}", names(&feed));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_linked_package_is_refused() {
        let dir = folder("symlink");
        let (real, link) = (dir.join("secret.txt"), dir.join("p-1.0.0.0.vip"));
        std::fs::write(&real, "secret").unwrap();
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(&real, &link);
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&real, &link);
        if made.is_err() {
            // Windows without Developer Mode cannot make one; nothing to check.
            return std::fs::remove_dir_all(&dir).unwrap_or(());
        }
        let why = format!("{:#}", built(&link).unwrap_err());
        assert!(why.contains("is a link"), "{why}");
        assert!(built(&real).is_ok());
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
            link_unless_exists(&temp, &missing.join("p.vip")).unwrap_err()
        );
        assert!(why.contains(&missing.display().to_string()), "{why}");
        assert!(why.contains("hard links"), "{why}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
