//! A desktop shortcut that starts LabVIEW on a venv.
//!
//! The shortcut runs `lvpm launch`, not `LabVIEW.exe -pref <ini>`: a `.lnk`
//! cannot set environment variables, so a direct one would lose the venv's
//! own `TEMP`/`TMP`, and it would skip the manifest check and the pid record
//! that keeps a second LabVIEW off the same venv. It carries LabVIEW's icon,
//! so it still looks like what it starts.

use crate::venv::Venv;
#[cfg(windows)]
use anyhow::Context;
use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

/// The file name a shortcut gets when the caller names only a folder:
/// `<project> (LabVIEW 2026 64-bit).lnk`, the project being the `.lvproj`
/// it opens or else the repo folder.
pub fn file_name(v: &Venv, lvproj: Option<&Path>) -> String {
    let stem = lvproj
        .and_then(|p| p.file_stem())
        .or_else(|| v.repo.file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "venv".to_string());
    format!(
        "{stem} (LabVIEW {} {}-bit).lnk",
        v.target.year(),
        v.target.bitness
    )
}

/// The command line after `lvpm.exe`: the venv by its repo, so the shortcut
/// works from any folder, and `--pause-on-error` so a refused launch stays on
/// screen instead of flashing past in a console that closes.
pub fn arguments(v: &Venv, lvproj: Option<&Path>) -> String {
    let mut args = format!("--project {} launch", quote(&v.repo));
    if let Some(p) = lvproj {
        args.push(' ');
        args.push_str(&quote(p));
    }
    args.push_str(" --pause-on-error");
    args
}

/// The Windows app id a venv's LabVIEW windows carry, so the taskbar groups
/// them apart from the user's own LabVIEW and pins relaunch the venv. Stable
/// for a repo path across lvpm versions, or a pinned button stops matching.
pub fn app_id(v: &Venv) -> String {
    let key = v.repo.to_string_lossy().to_lowercase().replace('/', "\\");
    format!(
        "lvpm.venv.{:016x}",
        crate::venv_icon::fnv1a(key.trim_end_matches('\\'))
    )
}

/// The full command line that starts this venv's LabVIEW again: what a
/// taskbar pin runs.
pub fn relaunch_command(v: &Venv) -> Result<String> {
    let lvpm = std::env::current_exe().map_err(|e| anyhow::anyhow!("locating lvpm.exe: {e}"))?;
    Ok(format!("{} {}", quote(&lvpm), arguments(v, None)))
}

/// What the taskbar calls a pinned venv: `<project> (LabVIEW 2026)`.
pub fn display_name(v: &Venv) -> String {
    let name = v
        .repo
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "venv".into());
    format!("{name} (LabVIEW {})", v.target.year())
}

/// A path as one Windows command-line argument. Windows paths cannot hold
/// `"`, so quoting is enough — but a trailing backslash would escape the
/// closing quote, and is dropped.
fn quote(p: &Path) -> String {
    let s = p.to_string_lossy();
    format!("\"{}\"", s.trim_end_matches(['\\', '/']))
}

/// Write the shortcut to `lnk`, overwriting one that is there, with `icon`.
#[cfg(windows)]
pub fn create(v: &Venv, lvproj: Option<&Path>, lnk: &Path, icon: &Path) -> Result<()> {
    use mslnk::{ShellLink, ShowCommand};
    let lvpm = std::env::current_exe().context("locating lvpm.exe")?;
    let mut sl = ShellLink::new(&lvpm).with_context(|| format!("reading {}", lvpm.display()))?;
    // mslnk's `./lvpm.exe` is relative to the .lnk, not to lvpm — wrong
    // wherever the shortcut is not beside lvpm.exe. The ID list suffices.
    sl.set_relative_path(None);
    sl.set_arguments(Some(arguments(v, lvproj)));
    sl.set_working_dir(Some(v.repo.to_string_lossy().into_owned()));
    sl.set_icon_location(Some(icon.to_string_lossy().into_owned()));
    sl.set_name(Some(format!(
        "{} on the venv in {}",
        v.target.label(),
        v.repo.display()
    )));
    sl.header_mut().set_show_command(ShowCommand::ShowNormal);
    sl.create_lnk(lnk)
        .with_context(|| format!("writing {}", lnk.display()))?;
    Ok(())
}

#[cfg(not(windows))]
pub fn create(_v: &Venv, _lvproj: Option<&Path>, _lnk: &Path, _icon: &Path) -> Result<()> {
    bail!("venv shortcuts are Windows-only for now")
}

/// The user's desktop, wherever it was redirected to (OneDrive moves it).
#[cfg(windows)]
pub fn desktop() -> Result<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Shell Folders");
    match key.and_then(|k| k.get_value::<String, _>("Desktop")) {
        Ok(dir) => Ok(PathBuf::from(dir)),
        Err(e) => bail!("cannot find the desktop folder ({e}): pass --out instead"),
    }
}

#[cfg(not(windows))]
pub fn desktop() -> Result<PathBuf> {
    bail!("venv shortcuts are Windows-only for now")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_drops_a_trailing_backslash() {
        assert_eq!(quote(Path::new(r"C:\a b\")), r#""C:\a b""#);
        assert_eq!(quote(Path::new(r"C:\x.lvproj")), r#""C:\x.lvproj""#);
    }
}
