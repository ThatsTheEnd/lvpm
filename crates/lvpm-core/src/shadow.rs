//! Packages a venv holds that its LabVIEW installation holds too.
//!
//! A venv package's files mirror the installation's tree: what lands in
//! `.lv-venv/packages/<pkg>/1/vi.lib/Delacor/...` would be `vi.lib/Delacor/...`
//! in LabVIEW itself. When the installation has its own copy at the same
//! place (installed by VIPM, by `lvpm install --global`, or by hand), LabVIEW
//! loads that copy, whatever version the venv installed, and its Tools menu
//! shows both copies' entries. Nothing fails; the venv just is not what it
//! says. So lvpm looks for it and says so: after an install into a venv, at
//! `lvpm launch`, and in `lvpm venv status`.
//!
//! Only LabVIEW files count (VIs, controls, libraries, classes): a `dir.mnu`
//! or a readme two packages both ship says nothing about which copy loads.
//! Ported from upstream lvpm 8fee2d9 and 1faffff.

use crate::install::{self, Manifest};
use crate::relink;
use crate::target::Roots;
use crate::venv::Venv;
use serde::Serialize;
use std::path::Path;

/// A venv package the LabVIEW installation has its own copy of.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Shadowed {
    pub name: String,
    /// The version the venv installed.
    pub version: String,
    /// How many of the package's LabVIEW files the installation has too, of
    /// how many it has.
    pub found: usize,
    pub total: usize,
    /// One of them, relative to the installation, forward-slashed.
    pub example: String,
    /// The installation's copy came from `lvpm install --global`, at this
    /// version (`None` inside: a record lvpm cannot read). `None`: anything
    /// else put it there, VIPM, NI's installers, a copy by hand.
    pub by_lvpm: Option<Option<String>>,
}

/// The packages among `records` (the venv's install records) that `v`'s
/// LabVIEW installation shadows.
pub fn find(v: &Venv, records: &[Manifest]) -> Vec<Shadowed> {
    let global = Roots::labview(&v.target);
    scan(records, &v.target.path, |name| {
        install::is_installed(&global, name).then(|| {
            install::read_manifest(&global, name)
                .ok()
                .map(|m| m.version)
        })
    })
}

/// Tell `sink` which of `records` the installation shadows; nothing when none.
pub fn emit(sink: &dyn crate::events::EventSink, v: &Venv, records: &[Manifest]) {
    let packages = find(v, records);
    if !packages.is_empty() {
        sink.emit(crate::events::Event::VenvShadowed {
            labview_year: v.target.year(),
            packages,
        });
    }
}

fn scan(
    records: &[Manifest],
    labview: &Path,
    by_lvpm: impl Fn(&str) -> Option<Option<String>>,
) -> Vec<Shadowed> {
    let mut out = Vec::new();
    for m in records {
        let files: Vec<String> = (m.files.iter())
            .filter_map(|f| in_tree(f, &m.name))
            .collect();
        let found: Vec<&String> = (files.iter())
            .filter(|rel| labview.join(rel).is_file())
            .collect();
        if let Some(example) = found.first() {
            out.push(Shadowed {
                name: m.name.clone(),
                version: m.version.clone(),
                found: found.len(),
                total: files.len(),
                example: example.to_string(),
                by_lvpm: by_lvpm(&m.name),
            });
        }
    }
    out
}

/// `file`'s path inside the LabVIEW tree its package mirrors, when it is a
/// LabVIEW file there: the part after `packages/<pkg>/1/`, below a folder.
/// Files outside the addon (`<OS ...>` targets) and the addon's own root
/// files (`lvaddoninfo.json`) have none.
fn in_tree(file: &str, pkg: &str) -> Option<String> {
    let file = file.replace('\\', "/");
    let marker = format!("/packages/{pkg}/1/").to_ascii_lowercase();
    let at = file.to_ascii_lowercase().find(&marker)?;
    let rel = &file[at + marker.len()..];
    (rel.contains('/') && relink::is_lv_item(Path::new(rel))).then(|| rel.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(name: &str, files: &[&str]) -> Manifest {
        serde_json::from_value(serde_json::json!({
            "name": name, "version": "1.0", "installed_at": 0, "target": "t",
            "files": files, "dependencies": [], "skipped_hooks": [],
        }))
        .unwrap()
    }

    #[test]
    fn paths_map_into_the_labview_tree() {
        let f = "C:/Temp/p/.lv-venv/packages/delacor_lib_qmh/1/vi.lib/Delacor/QMH/A.vi";
        assert_eq!(
            in_tree(f, "delacor_lib_qmh").as_deref(),
            Some("vi.lib/Delacor/QMH/A.vi")
        );
        // Recorded with other separators or case than the venv's own path.
        let f = r"c:\temp\P\.LV-VENV\Packages\Delacor_Lib_QMH\1\project\Delacor\B.vi";
        assert_eq!(
            in_tree(f, "delacor_lib_qmh").as_deref(),
            Some("project/Delacor/B.vi")
        );
        // Not LabVIEW files, the addon's own root, and files outside the addon.
        assert_eq!(in_tree("C:/v/packages/p/1/vi.lib/P/readme.txt", "p"), None);
        assert_eq!(in_tree("C:/v/packages/p/1/lvaddoninfo.json", "p"), None);
        assert_eq!(in_tree("C:/v/packages/p/1/Top.vi", "p"), None);
        assert_eq!(in_tree("C:/Users/me/Documents/p/A.vi", "p"), None);
    }

    #[test]
    fn finds_packages_the_installation_also_has() {
        let root = std::env::temp_dir().join(format!("lvpm-shadow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let lv = root.join("LabVIEW 2026");
        for rel in [
            "vi.lib/Delacor/QMH/A.vi",
            "vi.lib/Delacor/QMH/dir.mnu",
            "vi.lib/Mine/M.vi",
        ] {
            std::fs::create_dir_all(lv.join(rel).parent().unwrap()).unwrap();
            std::fs::write(lv.join(rel), b"x").unwrap();
        }
        let venv = "C:/p/.lv-venv/packages";
        let records = [
            record(
                "qmh",
                &[
                    &format!("{venv}/qmh/1/vi.lib/Delacor/QMH/A.vi"),
                    &format!("{venv}/qmh/1/vi.lib/Delacor/QMH/B.vi"),
                    &format!("{venv}/qmh/1/vi.lib/Delacor/QMH/dir.mnu"),
                ],
            ),
            record("mine", &[&format!("{venv}/mine/1/vi.lib/Mine/M.vi")]),
            record("clean", &[&format!("{venv}/clean/1/vi.lib/Clean/C.vi")]),
        ];
        let got = scan(&records, &lv, |name| {
            (name == "mine").then(|| Some("0.9".into()))
        });
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(
            (got[0].name.as_str(), got[0].found, got[0].total),
            ("qmh", 1, 2)
        );
        assert_eq!(got[0].example, "vi.lib/Delacor/QMH/A.vi");
        assert_eq!(got[0].by_lvpm, None);
        assert_eq!(got[1].by_lvpm, Some(Some("0.9".into())));
    }
}
