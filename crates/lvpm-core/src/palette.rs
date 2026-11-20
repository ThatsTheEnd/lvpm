//! Merging the palettes installed into a project venv.
//!
//! Package menu trees are kept under the venv's private `palettes` directory,
//! outside `LVAddons.AdditionalLocations`. The one reserved addon under
//! `packages/` is rebuilt from those owned contributions, so uninstall and
//! upgrade can reconstruct shared categories without relying on another
//! package's copy. Differing `dir.mnu` files use the first package in
//! case-insensitive name order; every other differing same-path file is an
//! error.
//!
//! Two trees are merged: `<menus>` and the `.mnu` files under
//! `<vi.lib>\addons`. Both have category folders several packages add to, and
//! LabVIEW shows only one addon's copy of a folder.
//!
//! Ported from upstream lvpm e59e344. Here a package's install still writes
//! its `menus/` and `vi.lib/addons/**.mnu` into its own addon, staged like the rest; the install's
//! refresh step ([`crate::ops::install::finish`]) then runs [`reconcile`],
//! which moves them into the private store, records that in the package's
//! install record, and publishes the merged addon. An uninstall goes through
//! the same step.

use crate::install::{self, Manifest, PaletteFile, PaletteTree};
use crate::target::Roots;
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// This addon folder and identity are reserved for lvpm's merged palettes.
pub const ADDON_NAME: &str = "venv-palettes";

#[derive(Debug)]
struct Entry {
    package: String,
    /// Relative to the addon root: `menus/...` or `vi.lib/addons/...`.
    relative: String,
    source: PathBuf,
}

#[derive(Debug)]
struct LegacyFile {
    package: String,
    path: PathBuf,
    tree: PaletteTree,
    /// Relative to the tree's folder.
    relative: String,
    contribution: PathBuf,
    source: PathBuf,
}

#[derive(Debug)]
struct MergedFile {
    relative: String,
    bytes: Vec<u8>,
    package: String,
}

fn venv_path(roots: &Roots) -> Result<&Path> {
    roots
        .venv()
        .context("palette reconciliation requires a project venv")
}

pub(crate) fn validate_package_name(package: &str) -> Result<()> {
    ensure!(
        !package.is_empty()
            && package != "."
            && package != ".."
            && package
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-&".contains(c)),
        "package name {package:?} cannot be an addon folder name"
    );
    Ok(())
}

fn palette_state_dir(roots: &Roots) -> Result<PathBuf> {
    Ok(venv_path(roots)?.join("palettes"))
}

fn managed_marker(roots: &Roots) -> Result<PathBuf> {
    Ok(palette_state_dir(roots)?.join("managed"))
}

fn stage_dir(roots: &Roots) -> Result<PathBuf> {
    Ok(palette_state_dir(roots)?.join("addon-stage"))
}

fn backup_dir(roots: &Roots) -> Result<PathBuf> {
    Ok(palette_state_dir(roots)?.join("addon-backup"))
}

fn addon_dir(roots: &Roots) -> Result<PathBuf> {
    Ok(venv_path(roots)?
        .join("packages")
        .join(ADDON_NAME)
        .join("1"))
}

fn contribution_base(roots: &Roots, package: &str) -> PathBuf {
    roots
        .venv()
        .expect("palette contributions only belong to a venv")
        .join("palettes")
        .join("contributions")
        .join(package.to_ascii_lowercase())
}

/// Where a package's palette files of one tree are kept.
pub fn contribution_tree(roots: &Roots, package: &str, tree: PaletteTree) -> PathBuf {
    contribution_base(roots, package).join(tree.dir())
}

#[cfg(test)]
pub fn contribution_root(roots: &Roots, package: &str) -> PathBuf {
    contribution_tree(roots, package, PaletteTree::Menus)
}

/// Whether a file is a palette menu file.
fn is_mnu(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("mnu"))
}

fn safe_palette_path(relative: &str) -> Result<PathBuf> {
    let normalized = relative.replace('\\', "/");
    ensure!(
        !normalized.starts_with('/') && !normalized.contains(':'),
        "refusing absolute palette path: {relative:?}"
    );
    let mut path = PathBuf::new();
    for part in normalized.split('/') {
        match part {
            "" | "." => {}
            ".." => bail!("refusing palette path traversal: {relative:?}"),
            p => path.push(p),
        }
    }
    ensure!(!path.as_os_str().is_empty(), "palette path is empty");
    Ok(path)
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn normalized(path: &str) -> String {
    path.replace('\\', "/").to_ascii_lowercase()
}

fn recorded_file(manifest: &Manifest, path: &Path) -> bool {
    let want = normalized(&path_string(path));
    manifest.files.iter().any(|file| normalized(file) == want)
}

/// A palette file an earlier lvpm left in the package's own addon: anything
/// under its `menus`, and the `.mnu` files under its `vi.lib/addons`.
fn legacy_relative(
    roots: &Roots,
    manifest: &Manifest,
    file: &str,
) -> Result<Option<(PaletteTree, String)>> {
    let package_root = venv_path(roots)?
        .join("packages")
        .join(&manifest.name)
        .join("1");
    let file = file.replace('\\', "/");
    let file_lower = file.to_ascii_lowercase();
    for tree in [PaletteTree::Menus, PaletteTree::Addons] {
        let expected = normalized(&path_string(&package_root.join(tree.dir())));
        let Some(tail) = file_lower.strip_prefix(&(expected.clone() + "/")) else {
            continue;
        };
        let relative = &file[expected.len() + 1..];
        ensure!(
            tail.len() == relative.len(),
            "invalid recorded palette path {file:?}"
        );
        let safe = safe_palette_path(relative)?;
        if tree == PaletteTree::Addons && !is_mnu(&safe) {
            return Ok(None);
        }
        return Ok(Some((tree, path_string(&safe))));
    }
    Ok(None)
}

fn read_regular_file(path: &Path) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("reading palette contribution {}", path.display()))?;
    ensure!(
        metadata.file_type().is_file(),
        "palette contribution is not a regular file: {}",
        path.display()
    );
    fs::read(path).with_context(|| format!("reading palette contribution {}", path.display()))
}

fn walk_files(root: &Path, current: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let metadata = match fs::symlink_metadata(current) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("reading {}", current.display())),
    };
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "refusing non-directory palette tree: {}",
        current.display()
    );
    for entry in fs::read_dir(current).with_context(|| format!("reading {}", current.display()))? {
        let path = entry?.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            bail!("refusing symlink in palette tree: {}", path.display());
        }
        if metadata.is_dir() {
            walk_files(root, &path, out)?;
        } else if metadata.is_file() {
            ensure!(
                path.starts_with(root),
                "palette file escaped {}",
                root.display()
            );
            out.push(path);
        }
    }
    Ok(())
}

fn validate_addon_tree(
    root: &Path,
    files: &[MergedFile],
    target: &crate::target::LvTarget,
) -> Result<()> {
    let info_path = root.join("lvaddoninfo.json");
    ensure!(
        fs::read_to_string(&info_path)? == install::addon_info(ADDON_NAME, target),
        "palette addon information did not validate"
    );
    let mut actual = Vec::new();
    walk_files(root, root, &mut actual)?;
    let mut expected = HashSet::from([normalized(&path_string(&info_path))]);
    expected.extend(
        files
            .iter()
            .map(|file| normalized(&path_string(&root.join(&file.relative)))),
    );
    ensure!(
        actual.len() == expected.len(),
        "palette addon has an unexpected number of files"
    );
    for file in files {
        let path = root.join(&file.relative);
        ensure!(
            read_regular_file(&path)? == file.bytes,
            "palette file did not validate: {}",
            path.display()
        );
    }
    for path in actual {
        ensure!(
            expected.contains(&normalized(&path_string(&path))),
            "palette addon contains an unexpected file: {}",
            path.display()
        );
    }
    Ok(())
}

fn collect(roots: &Roots, manifests: &[Manifest]) -> Result<(Vec<Entry>, Vec<LegacyFile>)> {
    let mut entries = Vec::new();
    let mut legacy = Vec::new();
    for manifest in manifests {
        validate_package_name(&manifest.name)?;
        ensure!(
            !manifest.name.eq_ignore_ascii_case(ADDON_NAME),
            "package name {ADDON_NAME:?} is reserved for lvpm's palette addon"
        );
        let mut recorded_legacy = HashSet::new();

        for palette in &manifest.palette_files {
            let relative = path_string(&safe_palette_path(&palette.relative)?);
            let source = contribution_tree(roots, &manifest.name, palette.tree).join(&relative);
            ensure!(
                recorded_file(manifest, &source),
                "{}'s palette record does not own {}",
                manifest.name,
                source.display()
            );
            entries.push(Entry {
                package: manifest.name.clone(),
                relative: format!("{}/{relative}", palette.tree.dir()),
                source,
            });
        }

        for file in &manifest.files {
            let Some((tree, relative)) = legacy_relative(roots, manifest, file)? else {
                continue;
            };
            recorded_legacy.insert(normalized(file));
            let old = PathBuf::from(file);
            let contribution = contribution_tree(roots, &manifest.name, tree).join(&relative);
            let mounted = addon_dir(roots)?.join(tree.dir()).join(&relative);
            let source = if contribution.is_file() {
                if old.is_file() {
                    ensure!(
                        read_regular_file(&contribution)? == read_regular_file(&old)?,
                        "palette migration found different copies for {} at {} and {}",
                        relative,
                        contribution.display(),
                        old.display()
                    );
                }
                contribution.clone()
            } else if old.is_file() {
                old.clone()
            } else if mounted.is_file() {
                mounted
            } else {
                bail!(
                    "cannot migrate {}'s palette file {}: the package copy and merged addon copy are both missing",
                    manifest.name,
                    old.display()
                );
            };
            entries.push(Entry {
                package: manifest.name.clone(),
                relative: format!("{}/{relative}", tree.dir()),
                source: source.clone(),
            });
            legacy.push(LegacyFile {
                package: manifest.name.clone(),
                path: old,
                tree,
                relative,
                contribution,
                source,
            });
        }

        let old_root = venv_path(roots)?
            .join("packages")
            .join(&manifest.name)
            .join("1")
            .join("menus");
        let mut disk_files = Vec::new();
        walk_files(&old_root, &old_root, &mut disk_files)?;
        for path in disk_files {
            ensure!(
                recorded_legacy.contains(&normalized(&path_string(&path))),
                "unrecorded menu file cannot be safely migrated: {}",
                path.display()
            );
        }
    }
    Ok((entries, legacy))
}

/// The records under `roots`, and the files among them that do not read as
/// one, with why. [`install::list_installed`] skips those silently.
type Unreadable = Vec<(PathBuf, anyhow::Error)>;

fn list_lenient(roots: &Roots) -> Result<(Vec<Manifest>, Unreadable)> {
    let dir = roots.store_dir();
    let (mut good, mut bad) = (Vec::new(), Vec::new());
    if !dir.exists() {
        return Ok((good, bad));
    }
    for entry in fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let read = fs::read_to_string(&path)
            .map_err(anyhow::Error::from)
            .and_then(|text| Ok(serde_json::from_str::<Manifest>(&text)?));
        match read {
            Ok(m) => good.push(m),
            Err(why) => bad.push((path, why)),
        }
    }
    good.sort_by(|a, b| a.name.cmp(&b.name));
    Ok((good, bad))
}

fn manifests_for_reconcile(roots: &Roots) -> Result<(Vec<Manifest>, bool)> {
    let (manifests, unreadable) = list_lenient(roots)?;
    for (record, error) in &unreadable {
        let package = record
            .file_stem()
            .context("unreadable installation record has no package name")?
            .to_string_lossy()
            .into_owned();
        validate_package_name(&package)?;
        let old_root = venv_path(roots)?
            .join("packages")
            .join(&package)
            .join("1")
            .join("menus");
        let contribution = contribution_base(roots, &package);
        let mut files = Vec::new();
        walk_files(&old_root, &old_root, &mut files)?;
        walk_files(&contribution, &contribution, &mut files)?;
        ensure!(
            files.is_empty(),
            "cannot reconcile palette files for unreadable install record {}: {error:#}",
            record.display()
        );
    }
    Ok((manifests, !unreadable.is_empty()))
}

/// A `dir.mnu` that differed between contributors: whose copy was published
/// and whose were left out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DirMnuChoice {
    pub relative: String,
    pub kept: String,
    pub dropped: Vec<String>,
}

/// What a [`reconcile`] that rebuilt the addon published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merged {
    pub files: usize,
    pub packages: usize,
    /// Palette files an earlier lvpm left in a package's own addon, moved out.
    pub moved: usize,
    pub dir_mnu: Vec<DirMnuChoice>,
}

fn merge(entries: &[Entry]) -> Result<(Vec<MergedFile>, Vec<DirMnuChoice>)> {
    let mut ordered: Vec<&Entry> = entries.iter().collect();
    ordered.sort_by(|a, b| {
        a.package
            .to_ascii_lowercase()
            .cmp(&b.package.to_ascii_lowercase())
            .then_with(|| a.package.cmp(&b.package))
            .then_with(|| normalized(&a.relative).cmp(&normalized(&b.relative)))
            .then_with(|| a.relative.cmp(&b.relative))
    });

    let mut files: BTreeMap<String, MergedFile> = BTreeMap::new();
    let mut choices: BTreeMap<String, DirMnuChoice> = BTreeMap::new();
    for entry in ordered {
        let relative = path_string(&safe_palette_path(&entry.relative)?);
        let key = normalized(&relative);
        let bytes = read_regular_file(&entry.source)?;
        if let Some(existing) = files.get(&key) {
            if existing.bytes == bytes {
                continue;
            }
            if relative
                .rsplit(['/', '\\'])
                .next()
                .is_some_and(|name| name.eq_ignore_ascii_case("dir.mnu"))
                && !existing.package.eq_ignore_ascii_case(&entry.package)
            {
                choices
                    .entry(key)
                    .or_insert_with(|| DirMnuChoice {
                        relative: existing.relative.clone(),
                        kept: existing.package.clone(),
                        dropped: Vec::new(),
                    })
                    .dropped
                    .push(entry.package.clone());
                continue;
            }
            bail!(
                "palette conflict at {relative:?}: packages {} and {} provide different files; \
                 the published palettes are unchanged. Uninstall one of the two, or report the \
                 clash to their maintainers",
                existing.package,
                entry.package
            );
        }
        files.insert(
            key,
            MergedFile {
                relative,
                bytes,
                package: entry.package.clone(),
            },
        );
    }
    Ok((
        files.into_values().collect(),
        choices.into_values().collect(),
    ))
}

/// Whether the published addon already holds exactly `files`.
fn is_current(
    roots: &Roots,
    files: &[MergedFile],
    target: &crate::target::LvTarget,
) -> Result<bool> {
    Ok(managed_marker(roots)?.exists()
        && validate_addon_tree(&addon_dir(roots)?, files, target).is_ok())
}

fn recover(roots: &Roots) -> Result<()> {
    let backup = backup_dir(roots)?;
    let output = addon_dir(roots)?;
    if backup.exists() {
        if output.exists() {
            fs::remove_dir_all(&backup)
                .with_context(|| format!("removing {}", backup.display()))?;
        } else {
            fs::create_dir_all(output.parent().context("palette addon has no parent")?)?;
            fs::rename(&backup, &output).with_context(|| {
                format!(
                    "restoring the previous palette addon from {}",
                    backup.display()
                )
            })?;
        }
    }
    let stage = stage_dir(roots)?;
    if stage.exists() {
        fs::remove_dir_all(&stage).with_context(|| format!("removing {}", stage.display()))?;
    }
    Ok(())
}

fn ensure_managed(
    roots: &Roots,
    files: &[MergedFile],
    target: &crate::target::LvTarget,
) -> Result<()> {
    let marker = managed_marker(roots)?;
    if marker.exists() {
        return Ok(());
    }
    let parent = marker.parent().context("palette state has no parent")?;
    fs::create_dir_all(parent)?;
    let output = addon_dir(roots)?;
    let addon_parent = output.parent().context("palette addon has no parent")?;
    if addon_parent.exists() {
        ensure!(
            output.exists(),
            "reserved palette addon path already exists and is not managed by lvpm: {}",
            addon_parent.display()
        );
        validate_addon_tree(&output, files, target).with_context(|| {
            format!(
                "reserved palette addon is not the expected tree: {}",
                output.display()
            )
        })?;
    }
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker)
    {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e).with_context(|| format!("creating {}", marker.display())),
    }
}

fn publish(roots: &Roots, files: &[MergedFile], target: &crate::target::LvTarget) -> Result<()> {
    recover(roots)?;
    let output = addon_dir(roots)?;
    let addon_parent = output.parent().context("palette addon has no parent")?;
    let stage = stage_dir(roots)?;
    let backup = backup_dir(roots)?;
    ensure_managed(roots, files, target)?;

    fs::create_dir_all(&stage)?;
    fs::write(
        stage.join("lvaddoninfo.json"),
        install::addon_info(ADDON_NAME, target),
    )
    .with_context(|| format!("writing palette addon information in {}", stage.display()))?;
    for file in files {
        let destination = stage.join(&file.relative);
        fs::create_dir_all(destination.parent().context("palette file has no parent")?)?;
        fs::write(&destination, &file.bytes)
            .with_context(|| format!("staging palette file {}", destination.display()))?;
    }
    validate_addon_tree(&stage, files, target)?;

    fs::create_dir_all(addon_parent)?;
    if output.exists() {
        fs::rename(&output, &backup).with_context(|| {
            format!(
                "staging the previous palette addon from {}",
                output.display()
            )
        })?;
    }
    if let Err(error) = fs::rename(&stage, &output) {
        if backup.exists()
            && let Err(restore) = fs::rename(&backup, &output)
        {
            bail!(
                "publishing the merged palettes failed: {error}; restoring the previous addon also failed: {restore}"
            );
        }
        return Err(error)
            .with_context(|| format!("publishing palette addon to {}", output.display()));
    }
    if let Err(error) = validate_addon_tree(&output, files, target) {
        if output.exists() {
            fs::remove_dir_all(&output)
                .with_context(|| format!("removing invalid palette addon {}", output.display()))?;
        }
        if backup.exists() {
            fs::rename(&backup, &output).with_context(|| {
                format!(
                    "restoring the previous palette addon from {}",
                    backup.display()
                )
            })?;
        }
        return Err(error).context("validating the published palette addon");
    }
    if backup.exists() {
        fs::remove_dir_all(&backup).with_context(|| format!("removing {}", backup.display()))?;
    }
    Ok(())
}

/// Remove the managed addon; whether there was a published one to remove.
fn publish_empty(roots: &Roots) -> Result<bool> {
    recover(roots)?;
    let marker = managed_marker(roots)?;
    if !marker.exists() {
        let output = addon_dir(roots)?;
        let addon_parent = output.parent().context("palette addon has no parent")?;
        ensure!(
            !addon_parent.exists(),
            "reserved palette addon path already exists and is not managed by lvpm: {}",
            addon_parent.display()
        );
        return Ok(false);
    }
    let output = addon_dir(roots)?;
    let backup = backup_dir(roots)?;
    let had_output = output.exists();
    if had_output {
        fs::rename(&output, &backup).with_context(|| {
            format!(
                "staging the previous palette addon from {}",
                output.display()
            )
        })?;
    }
    if backup.exists()
        && let Err(error) = fs::remove_dir_all(&backup)
    {
        if !output.exists()
            && let Err(restore) = fs::rename(&backup, &output)
        {
            bail!(
                "removing the palette addon failed: {error}; restoring it also failed: {restore}"
            );
        }
        return Err(error).with_context(|| format!("removing {}", backup.display()));
    }
    if marker.exists() {
        fs::remove_file(&marker).with_context(|| format!("removing {}", marker.display()))?;
    }
    let parent = output.parent().context("palette addon has no parent")?;
    if parent.exists() && fs::read_dir(parent)?.next().is_none() {
        fs::remove_dir(parent)?;
    }
    Ok(had_output)
}

fn copy_legacy(legacy: &[LegacyFile]) -> Result<()> {
    for file in legacy {
        if file.source == file.contribution {
            continue;
        }
        let bytes = read_regular_file(&file.source)?;
        if file.contribution.exists() {
            ensure!(
                read_regular_file(&file.contribution)? == bytes,
                "palette contribution for {} changed during migration: {}",
                file.package,
                file.contribution.display()
            );
            continue;
        }
        fs::create_dir_all(
            file.contribution
                .parent()
                .context("palette contribution has no parent")?,
        )?;
        fs::write(&file.contribution, bytes).with_context(|| {
            format!("migrating palette file to {}", file.contribution.display())
        })?;
    }
    Ok(())
}

fn record_migration(
    roots: &Roots,
    manifests: &mut [Manifest],
    legacy: &[LegacyFile],
) -> Result<()> {
    for manifest in manifests.iter_mut() {
        let files: Vec<&LegacyFile> = legacy
            .iter()
            .filter(|file| file.package == manifest.name)
            .collect();
        if files.is_empty() {
            continue;
        }
        for file in &files {
            let palette = PaletteFile {
                relative: file.relative.clone(),
                tree: file.tree,
            };
            if !manifest.palette_files.contains(&palette) {
                manifest.palette_files.push(palette);
            }
            let source = path_string(&file.contribution);
            if !recorded_file(manifest, &file.contribution) {
                manifest.files.push(source);
            }
        }
        install::write_manifest(roots, manifest)?;
    }

    for file in legacy {
        if file.path.exists() && file.path != file.contribution {
            fs::remove_file(&file.path).with_context(|| {
                format!("removing migrated palette file {}", file.path.display())
            })?;
            prune_legacy_dirs(roots, &file.path)?;
        }
    }

    for manifest in manifests.iter_mut() {
        let old: HashSet<String> = legacy
            .iter()
            .filter(|file| file.package == manifest.name)
            .map(|file| normalized(&path_string(&file.path)))
            .collect();
        if old.is_empty() {
            continue;
        }
        manifest
            .files
            .retain(|file| !old.contains(&normalized(file)));
        install::write_manifest(roots, manifest)?;
    }
    Ok(())
}

fn prune_legacy_dirs(roots: &Roots, path: &Path) -> Result<()> {
    let mut current = path.parent();
    let stop = path
        .ancestors()
        .find(|p| p.file_name().is_some_and(|name| name == "1"))
        .map(Path::to_path_buf);
    while let Some(dir) = current {
        if Some(dir) == stop.as_deref() {
            break;
        }
        if !dir.starts_with(venv_path(roots)?)
            || !dir.exists()
            || fs::read_dir(dir)?.next().is_some()
        {
            break;
        }
        fs::remove_dir(dir)?;
        current = dir.parent();
    }
    Ok(())
}

fn cleanup_orphans(roots: &Roots, manifests: &[Manifest]) -> Result<()> {
    let mut referenced = HashSet::new();
    for manifest in manifests {
        for palette in &manifest.palette_files {
            referenced.insert(normalized(&path_string(
                &contribution_tree(roots, &manifest.name, palette.tree)
                    .join(safe_palette_path(&palette.relative)?),
            )));
        }
        for file in &manifest.files {
            if let Some((tree, relative)) = legacy_relative(roots, manifest, file)? {
                referenced.insert(normalized(&path_string(
                    &contribution_tree(roots, &manifest.name, tree).join(relative),
                )));
            }
        }
    }
    let contribution_base = palette_state_dir(roots)?.join("contributions");
    let mut files = Vec::new();
    walk_files(&contribution_base, &contribution_base, &mut files)?;
    for file in files {
        if !referenced.contains(&normalized(&path_string(&file))) {
            fs::remove_file(&file)
                .with_context(|| format!("removing unowned palette source {}", file.display()))?;
        }
    }
    Ok(())
}

/// Migrate package-local menus if needed and atomically rebuild the reserved
/// addon. A conflict or failed stage leaves the currently published tree in
/// place; file ownership remains with the package manifests.
///
/// An addon that already holds the merged tree is left alone, so a reconcile
/// with nothing to do never renames a folder a running LabVIEW has mounted.
/// Returns what was published, or `None` when nothing changed.
pub fn reconcile(roots: &Roots) -> Result<Option<Merged>> {
    if roots.venv().is_none() {
        return Ok(None);
    }
    recover(roots)?;
    let (mut manifests, has_unreadable) = manifests_for_reconcile(roots)?;
    let (entries, legacy) = collect(roots, &manifests)?;
    let (merged, choices) = merge(&entries)?;

    copy_legacy(&legacy)?;
    let changed = match (&merged[..], roots.target.as_ref()) {
        ([], _) => publish_empty(roots)?,
        (_, Some(target)) if is_current(roots, &merged, target)? => false,
        (_, Some(target)) => {
            publish(roots, &merged, target)?;
            true
        }
        (_, None) => bail!("palette addon cannot be created without a bound LabVIEW target"),
    };
    record_migration(roots, &mut manifests, &legacy)?;

    let packages: HashSet<String> = entries
        .iter()
        .map(|e| e.package.to_ascii_lowercase())
        .collect();
    let merged = changed.then_some(Merged {
        files: merged.len(),
        packages: packages.len(),
        moved: legacy.len(),
        dir_mnu: choices,
    });

    if !has_unreadable {
        let updated = install::list_installed(roots)?;
        cleanup_orphans(roots, &updated)?;
    }
    Ok(merged)
}

/// Verify that the merged addon matches recorded package contributions.
pub fn verify(roots: &Roots) -> Result<()> {
    if roots.venv().is_none() {
        return Ok(());
    }
    let manifests = install::list_installed(roots)?;
    let (entries, legacy) = collect(roots, &manifests)?;
    ensure!(
        legacy.is_empty(),
        "package-local palette files need migration; run `lvpm sync`"
    );
    let (merged, _) = merge(&entries)?;
    let output = addon_dir(roots)?;
    let marker = managed_marker(roots)?;
    if merged.is_empty() {
        ensure!(
            !marker.exists() && !output.exists(),
            "the managed palette addon is stale; run `lvpm sync`"
        );
        return Ok(());
    }
    ensure!(
        marker.exists(),
        "the merged palette addon is not managed; run `lvpm sync`"
    );
    validate_addon_tree(
        &output,
        &merged,
        roots
            .target
            .as_ref()
            .context("venv has no LabVIEW target")?,
    )
    .context("the merged palette addon is stale; run `lvpm sync`")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::LvTarget;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn a_palette_path_that_leaves_the_tree_is_refused() {
        for bad in [
            "/abs/x.mnu",
            "\\abs\\x.mnu",
            "C:x.mnu",
            "a/../../x.mnu",
            "..",
            "",
            "./",
        ] {
            assert!(safe_palette_path(bad).is_err(), "{bad:?} was accepted");
        }
        assert_eq!(
            safe_palette_path("a\\.\\b//c.mnu").unwrap(),
            Path::new("a").join("b").join("c.mnu")
        );
    }

    #[test]
    fn a_package_name_must_be_a_plain_folder_name() {
        for bad in ["", ".", "..", "a/b", "a\\b", "a b", "a:b", "ä"] {
            assert!(validate_package_name(bad).is_err(), "{bad:?} was accepted");
        }
        for good in ["oglib_array", "JKI_State-Machine.2", "R&D"] {
            validate_package_name(good).unwrap();
        }
    }

    fn roots(label: &str) -> (PathBuf, Roots) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("lvpm-palette-{label}-{unique}"));
        let target = LvTarget {
            version: 26.3,
            bitness: 64,
            path: dir.join("LabVIEW"),
        };
        (dir.clone(), Roots::project_store(&dir, &target))
    }

    fn add_package(roots: &Roots, name: &str, files: &[(&str, &[u8])]) {
        let mut manifest = Manifest {
            name: name.into(),
            version: "1".into(),
            display_name: None,
            installed_at: 0,
            target: "test".into(),
            files: Vec::new(),
            palette_files: Vec::new(),
            dependencies: Vec::new(),
            skipped_hooks: Vec::new(),
            relinked: false,
            relink_folders: Vec::new(),
            post_install_vi: None,
            pre_install_vi: None,
            pre_uninstall_vi: None,
            post_uninstall_vi: None,
            skipped_groups: Vec::new(),
            archive_sha256: None,
        };
        for (relative, bytes) in files {
            // `vi.lib/addons/...` is the Addons tree; anything else is `menus`.
            let (tree, relative) = match relative.strip_prefix("vi.lib/addons/") {
                Some(rest) => (PaletteTree::Addons, rest),
                None => (PaletteTree::Menus, *relative),
            };
            let relative = path_string(&safe_palette_path(relative).unwrap());
            let source = contribution_tree(roots, name, tree).join(&relative);
            fs::create_dir_all(source.parent().unwrap()).unwrap();
            fs::write(&source, bytes).unwrap();
            manifest.files.push(path_string(&source));
            manifest.palette_files.push(PaletteFile { relative, tree });
        }
        install::write_manifest(roots, &manifest).unwrap();
    }

    #[test]
    fn merges_root_menus_controls_and_categories_without_dir_mnu() {
        let (dir, roots) = roots("nested");
        add_package(
            &roots,
            "alpha",
            &[
                ("Categories/Computer/alpha.mnu", b"computer"),
                ("Root.mnu", b"root"),
            ],
        );
        add_package(&roots, "beta", &[("Controls/MGI/control.mnu", b"control")]);

        reconcile(&roots).unwrap();
        verify(&roots).unwrap();
        let menus = addon_dir(&roots).unwrap().join("menus");
        assert_eq!(
            fs::read(menus.join("Categories/Computer/alpha.mnu")).unwrap(),
            b"computer"
        );
        assert_eq!(fs::read(menus.join("Root.mnu")).unwrap(), b"root");
        assert_eq!(
            fs::read(menus.join("Controls/MGI/control.mnu")).unwrap(),
            b"control"
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(
                &fs::read(addon_dir(&roots).unwrap().join("lvaddoninfo.json")).unwrap()
            )
            .unwrap()["AddonName"],
            ADDON_NAME
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn packages_sharing_an_addons_category_all_show_in_it() {
        // The HSE packages: each ships the same `addons\HSE\dir.mnu` and its
        // own palette beside it. Merged, the category lists every one.
        let (dir, roots) = roots("addons");
        add_package(
            &roots,
            "hse_lib_hse_logger",
            &[
                ("vi.lib/addons/HSE/dir.mnu", b"hse"),
                (
                    "vi.lib/addons/HSE/functions_hse_lib_hse_logger.mnu",
                    b"logger",
                ),
            ],
        );
        add_package(
            &roots,
            "hse_lib_hse_layout_manager",
            &[
                ("vi.lib/addons/HSE/dir.mnu", b"hse"),
                (
                    "vi.lib/addons/HSE/functions_HSE_lib_hse_layout_manager.mnu",
                    b"layout",
                ),
            ],
        );
        add_package(
            &roots,
            "hse_lib_hse_framework",
            &[("Controls/controls_hse.mnu", b"controls")],
        );

        assert!(reconcile(&roots).unwrap().is_some());
        verify(&roots).unwrap();
        let addon = addon_dir(&roots).unwrap();
        let hse = addon.join("vi.lib/addons/HSE");
        assert_eq!(fs::read(hse.join("dir.mnu")).unwrap(), b"hse");
        assert_eq!(
            fs::read(hse.join("functions_hse_lib_hse_logger.mnu")).unwrap(),
            b"logger"
        );
        assert_eq!(
            fs::read(hse.join("functions_HSE_lib_hse_layout_manager.mnu")).unwrap(),
            b"layout"
        );
        assert_eq!(
            fs::read(addon.join("menus/Controls/controls_hse.mnu")).unwrap(),
            b"controls"
        );

        install::uninstall(&roots, "hse_lib_hse_logger").unwrap();
        assert!(reconcile(&roots).unwrap().is_some());
        assert!(!hse.join("functions_hse_lib_hse_logger.mnu").exists());
        assert_eq!(fs::read(hse.join("dir.mnu")).unwrap(), b"hse");
        verify(&roots).unwrap();
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn migrates_addons_mnu_files_from_the_package_and_leaves_its_vis() {
        let (dir, roots) = roots("addons-migration");
        let package = roots
            .venv()
            .unwrap()
            .join("packages/hse_lib_hse_logger/1/vi.lib/addons/HSE");
        fs::create_dir_all(&package).unwrap();
        let menu = package.join("functions_hse_lib_hse_logger.mnu");
        let vi = package.join("Helper.vi");
        fs::write(&menu, b"logger").unwrap();
        fs::write(&vi, b"vi").unwrap();
        let manifest = Manifest {
            name: "hse_lib_hse_logger".into(),
            version: "1".into(),
            display_name: None,
            installed_at: 0,
            target: "test".into(),
            files: vec![path_string(&menu), path_string(&vi)],
            palette_files: Vec::new(),
            dependencies: Vec::new(),
            skipped_hooks: Vec::new(),
            relinked: false,
            relink_folders: Vec::new(),
            post_install_vi: None,
            pre_install_vi: None,
            pre_uninstall_vi: None,
            post_uninstall_vi: None,
            skipped_groups: Vec::new(),
            archive_sha256: None,
        };
        install::write_manifest(&roots, &manifest).unwrap();

        assert!(reconcile(&roots).unwrap().is_some());
        verify(&roots).unwrap();
        let migrated = contribution_tree(&roots, "hse_lib_hse_logger", PaletteTree::Addons)
            .join("HSE/functions_hse_lib_hse_logger.mnu");
        assert_eq!(fs::read(&migrated).unwrap(), b"logger");
        assert!(!menu.exists());
        assert!(vi.is_file());
        assert_eq!(
            fs::read(
                addon_dir(&roots)
                    .unwrap()
                    .join("vi.lib/addons/HSE/functions_hse_lib_hse_logger.mnu")
            )
            .unwrap(),
            b"logger"
        );
        let updated = install::read_manifest(&roots, "hse_lib_hse_logger").unwrap();
        assert_eq!(
            updated.palette_files,
            [PaletteFile {
                relative: "HSE/functions_hse_lib_hse_logger.mnu".into(),
                tree: PaletteTree::Addons,
            }]
        );
        assert!(recorded_file(&updated, &migrated));
        assert!(recorded_file(&updated, &vi));
        assert!(!recorded_file(&updated, &menu));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn deduplicates_identical_files_and_selects_dir_mnu_by_package_name() {
        let (dir, roots) = roots("ordering");
        add_package(
            &roots,
            "zeta",
            &[
                ("Categories/Shared/dir.mnu", b"zeta"),
                ("Categories/Shared/same.mnu", b"same"),
            ],
        );
        add_package(
            &roots,
            "alpha",
            &[
                ("Categories/Shared/dir.mnu", b"alpha"),
                ("Categories/Shared/same.mnu", b"same"),
            ],
        );

        reconcile(&roots).unwrap();
        let menus = addon_dir(&roots).unwrap().join("menus/Categories/Shared");
        assert_eq!(fs::read(menus.join("dir.mnu")).unwrap(), b"alpha");
        assert_eq!(fs::read(menus.join("same.mnu")).unwrap(), b"same");
        verify(&roots).unwrap();

        let manifests = install::list_installed(&roots).unwrap();
        let (entries, _) = collect(&roots, &manifests).unwrap();
        let (_, choices) = merge(&entries).unwrap();
        assert_eq!(
            choices,
            [DirMnuChoice {
                relative: "menus/Categories/Shared/dir.mnu".into(),
                kept: "alpha".into(),
                dropped: vec!["zeta".into()],
            }]
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_current_addon_is_not_republished() {
        let (dir, roots) = roots("current");
        add_package(&roots, "alpha", &[("Categories/Shared/a.mnu", b"a")]);
        assert!(reconcile(&roots).unwrap().is_some());

        // Publishing renames the addon folder away and back; a reconcile with
        // nothing to do must not, or it fights a LabVIEW that has it mounted.
        let addon = addon_dir(&roots).unwrap();
        let held = addon.join("menus/Categories/Shared/a.mnu");
        let before = fs::metadata(&held).unwrap().modified().unwrap();
        assert!(reconcile(&roots).unwrap().is_none());
        assert_eq!(fs::metadata(&held).unwrap().modified().unwrap(), before);

        add_package(&roots, "beta", &[("Categories/Shared/b.mnu", b"b")]);
        assert!(reconcile(&roots).unwrap().is_some());
        verify(&roots).unwrap();
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_conflicting_menu_file_leaves_the_published_tree_untouched() {
        let (dir, roots) = roots("conflict");
        add_package(
            &roots,
            "alpha",
            &[("Categories/Shared/functions.mnu", b"original")],
        );
        add_package(
            &roots,
            "beta",
            &[("Categories/Shared/functions.mnu", b"original")],
        );
        reconcile(&roots).unwrap();

        let changed = contribution_root(&roots, "beta").join("Categories/Shared/functions.mnu");
        fs::write(&changed, b"conflict").unwrap();
        let error = reconcile(&roots).unwrap_err();

        assert!(format!("{error:#}").contains("palette conflict"));
        assert_eq!(
            fs::read(
                addon_dir(&roots)
                    .unwrap()
                    .join("menus/Categories/Shared/functions.mnu")
            )
            .unwrap(),
            b"original"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn migrates_package_local_menus_and_updates_ownership_records() {
        let (dir, roots) = roots("migration");
        let old = roots
            .venv()
            .unwrap()
            .join("packages")
            .join("legacy")
            .join("1/menus/Categories/Old.mnu");
        fs::create_dir_all(old.parent().unwrap()).unwrap();
        fs::write(&old, b"legacy").unwrap();
        let manifest = Manifest {
            name: "legacy".into(),
            version: "1".into(),
            display_name: None,
            installed_at: 0,
            target: "test".into(),
            files: vec![path_string(&old)],
            palette_files: Vec::new(),
            dependencies: Vec::new(),
            skipped_hooks: Vec::new(),
            relinked: false,
            relink_folders: Vec::new(),
            post_install_vi: None,
            pre_install_vi: None,
            pre_uninstall_vi: None,
            post_uninstall_vi: None,
            skipped_groups: Vec::new(),
            archive_sha256: None,
        };
        install::write_manifest(&roots, &manifest).unwrap();

        reconcile(&roots).unwrap();
        verify(&roots).unwrap();

        let migrated = contribution_root(&roots, "legacy").join("Categories/Old.mnu");
        assert!(migrated.is_file());
        assert!(!old.exists());
        let updated = install::read_manifest(&roots, "legacy").unwrap();
        assert_eq!(
            updated.palette_files,
            [PaletteFile {
                relative: "Categories/Old.mnu".into(),
                tree: PaletteTree::Menus,
            }]
        );
        assert!(recorded_file(&updated, &migrated));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn migrates_missing_package_copies_from_an_exact_existing_merged_addon() {
        let (dir, roots) = roots("adopt");
        let old = roots
            .venv()
            .unwrap()
            .join("packages")
            .join("legacy")
            .join("1/menus/Categories/Old.mnu");
        let merged = addon_dir(&roots).unwrap();
        let old_menu = merged.join("menus/Categories/Old.mnu");
        fs::create_dir_all(old_menu.parent().unwrap()).unwrap();
        fs::write(old_menu, b"merged").unwrap();
        fs::write(
            merged.join("lvaddoninfo.json"),
            install::addon_info(ADDON_NAME, roots.target.as_ref().unwrap()),
        )
        .unwrap();
        let manifest = Manifest {
            name: "legacy".into(),
            version: "1".into(),
            display_name: None,
            installed_at: 0,
            target: "test".into(),
            files: vec![path_string(&old)],
            palette_files: Vec::new(),
            dependencies: Vec::new(),
            skipped_hooks: Vec::new(),
            relinked: false,
            relink_folders: Vec::new(),
            post_install_vi: None,
            pre_install_vi: None,
            pre_uninstall_vi: None,
            post_uninstall_vi: None,
            skipped_groups: Vec::new(),
            archive_sha256: None,
        };
        install::write_manifest(&roots, &manifest).unwrap();

        reconcile(&roots).unwrap();
        verify(&roots).unwrap();
        assert_eq!(
            fs::read(contribution_root(&roots, "legacy").join("Categories/Old.mnu")).unwrap(),
            b"merged"
        );
        assert!(managed_marker(&roots).unwrap().exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn uninstalling_one_contributor_keeps_the_other_and_removes_the_addon_when_empty() {
        let (dir, roots) = roots("ownership");
        add_package(
            &roots,
            "alpha",
            &[
                ("Categories/alpha.mnu", b"a"),
                ("Categories/shared.mnu", b"shared"),
            ],
        );
        add_package(
            &roots,
            "beta",
            &[
                ("Controls/beta.mnu", b"b"),
                ("Categories/shared.mnu", b"shared"),
            ],
        );
        reconcile(&roots).unwrap();

        install::uninstall(&roots, "alpha").unwrap();
        reconcile(&roots).unwrap();
        let menus = addon_dir(&roots).unwrap().join("menus");
        assert!(!menus.join("Categories/alpha.mnu").exists());
        assert_eq!(
            fs::read(menus.join("Categories/shared.mnu")).unwrap(),
            b"shared"
        );
        assert_eq!(fs::read(menus.join("Controls/beta.mnu")).unwrap(), b"b");

        install::uninstall(&roots, "beta").unwrap();
        reconcile(&roots).unwrap();
        assert!(!addon_dir(&roots).unwrap().exists());
        assert!(!managed_marker(&roots).unwrap().exists());
        let _ = fs::remove_dir_all(dir);
    }
}
