//! Which of a release's files is the package: `<name>-<version>.lvpkg`, or
//! `.vip` when the release holds no `.lvpkg`. Pure; it looks at names only.

/// A package file's extension, in order of preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Extension {
    Lvpkg,
    Vip,
}

/// The version `<name>-<version>.<ext>` carries, as written, and its kind.
#[derive(Debug, PartialEq, Eq)]
pub struct Parsed<'a> {
    pub version: &'a str,
    pub extension: Extension,
}

/// `file` read as `<name>-<version>.lvpkg` or `.vip`; `None` when it is
/// neither. The name matches without case, as package names do everywhere. A
/// version starts with a digit and holds letters, digits, `.` and `-`: it is
/// what the lock will say, so nothing else is let through.
pub fn parse<'a>(file: &'a str, name: &str) -> Option<Parsed<'a>> {
    let (stem, ext) = file.rsplit_once('.')?;
    let extension = match ext.to_ascii_lowercase().as_str() {
        "lvpkg" => Extension::Lvpkg,
        "vip" => Extension::Vip,
        _ => return None,
    };
    let (head, version) = stem.split_at_checked(name.len())?;
    let version = version.strip_prefix('-')?;
    let well_formed = version.starts_with(|c: char| c.is_ascii_digit())
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    (head.eq_ignore_ascii_case(name) && well_formed).then_some(Parsed { version, extension })
}

/// The index in `files` of the one to use for `name`, and its version: the
/// first `.lvpkg` that parses, else the first `.vip`.
pub fn choose<'a>(files: &[&'a str], name: &str) -> Option<(usize, &'a str)> {
    files
        .iter()
        .enumerate()
        .filter_map(|(at, file)| Some((at, parse(file, name)?)))
        .min_by_key(|(_, parsed)| parsed.extension)
        .map(|(at, parsed)| (at, parsed.version))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_name_dash_version_dot_lvpkg_or_vip() {
        use Extension::{Lvpkg, Vip};
        type Row = (
            &'static str,
            &'static str,
            Option<(&'static str, Extension)>,
        );
        let table: &[Row] = &[
            ("pkg-1.0.lvpkg", "pkg", Some(("1.0", Lvpkg))),
            ("pkg-1.0.0.4.vip", "pkg", Some(("1.0.0.4", Vip))),
            ("PKG-1.0.LVPKG", "pkg", Some(("1.0", Lvpkg))),
            ("pkg-1.0.lvpkg", "Pkg", Some(("1.0", Lvpkg))),
            ("pkg-1.0-rc1.vip", "pkg", Some(("1.0-rc1", Vip))),
            ("my-pkg-2.vip", "my-pkg", Some(("2", Vip))),
            (
                "labview_open_source_lib_serializer-1.2.1.16.vip",
                "labview_open_source_lib_serializer",
                Some(("1.2.1.16", Vip)),
            ),
            // Not this package.
            ("other-0.8.lvpkg", "pkg", None),
            ("pkg2-1.0.lvpkg", "pkg", None),
            ("pkg-extra-1.0.lvpkg", "pkg", None),
            ("Pkg_Release.lvpkg", "pkg", None),
            // Not a package file.
            ("pkg-1.0.zip", "pkg", None),
            ("pkg-1.0.lvpkg.sig", "pkg", None),
            ("pkg-1.0", "pkg", None),
            ("notes.txt", "pkg", None),
            // No version, or one that is not what a lock may hold.
            ("pkg-.lvpkg", "pkg", None),
            ("pkg.lvpkg", "pkg", None),
            ("pkg-v1.0.lvpkg", "pkg", None),
            ("pkg-1.0 beta.lvpkg", "pkg", None),
            ("pkg-1.0/../x.lvpkg", "pkg", None),
            // Multi-byte text where the name ends is no panic.
            ("päckage-1.0.lvpkg", "pack", None),
            ("", "pkg", None),
        ];
        for (file, name, want) in table {
            let got = parse(file, name).map(|p| (p.version, p.extension));
            assert_eq!(got, *want, "{file} for {name}");
        }
    }

    #[test]
    fn the_lvpkg_wins_over_the_vip_whatever_the_order_and_the_first_of_a_kind_wins() {
        let files = [
            "notes.txt",
            "pkg-1.0.vip",
            "other-1.0.lvpkg",
            "pkg-1.0.lvpkg",
        ];
        assert_eq!(choose(&files, "pkg"), Some((3, "1.0")));
        let vips = ["pkg-1.0.vip", "pkg-2.0.vip"];
        assert_eq!(choose(&vips, "pkg"), Some((0, "1.0")));
        assert_eq!(choose(&["other-1.0.vip", "notes.txt"], "pkg"), None);
        assert_eq!(choose(&[], "pkg"), None);
    }
}
