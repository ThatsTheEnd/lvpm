//! The wording of query results: what `targets`, `search`, `search --all`,
//! `list` and `cache add` print for the data core returns. Each function returns the whole
//! text, line ends included, for the caller to write to stdout.

use lvpm_core::index::{Entry, Import};
use lvpm_core::install::Manifest;
use lvpm_core::ops::query::Catalog;
use lvpm_core::target::LvTarget;

/// How many search hits are shown; the rest are counted.
const SEARCH_ROWS_SHOWN: usize = 40;
/// The name and version columns of `search` and `search --all`.
const NAME_WIDTH: usize = 52;
const VERSION_WIDTH: usize = 14;

pub fn targets(targets: &[LvTarget]) -> String {
    if targets.is_empty() {
        return "no LabVIEW installations detected\n".to_string();
    }
    targets
        .iter()
        .map(|t| format!("{:<34} {}\n", t.label(), t.path.display()))
        .collect()
}

pub fn search(query: &str, hits: &[Entry]) -> String {
    if hits.is_empty() {
        return format!("no packages matching {query:?}\n");
    }
    let mut text: String = hits
        .iter()
        .take(SEARCH_ROWS_SHOWN)
        .map(|e| {
            printable(format!(
                "{:<NAME_WIDTH$} {:<VERSION_WIDTH$} {}\n",
                e.name,
                e.version.to_string(),
                e.display_name.as_deref().unwrap_or("")
            ))
        })
        .collect();
    if hits.len() > SEARCH_ROWS_SHOWN {
        text.push_str(&format!(
            "... and {} more\n",
            hits.len() - SEARCH_ROWS_SHOWN
        ));
    }
    text
}

/// One row per entry, every one of them, for stdout.
pub fn catalog(query: Option<&str>, catalog: &Catalog) -> String {
    if !catalog.entries.is_empty() {
        return catalog.entries.iter().map(catalog_row).collect();
    }
    match (query, catalog.sources.as_slice()) {
        (Some(query), _) => format!("no packages matching {query:?}\n"),
        (None, []) => "no package sources are configured\n".to_string(),
        (None, sources) => {
            let named: String = sources.iter().map(|s| format!("  {s}\n")).collect();
            format!("no packages found in:\n{named}")
        }
    }
}

/// The sources the catalog could not read, for stderr.
pub fn unread_sources(catalog: &Catalog) -> String {
    let line =
        |(source, why): &(String, String)| format!("warning: could not read {source}: {why}\n");
    catalog.failed.iter().map(line).collect()
}

fn catalog_row(e: &Entry) -> String {
    let mut row = format!(
        "{:<NAME_WIDTH$} {:<VERSION_WIDTH$} {}",
        e.name,
        e.version.to_string(),
        e.display_name.as_deref().unwrap_or("")
    );
    if !e.requires.is_empty() {
        let needs: Vec<String> = e
            .requires
            .iter()
            .map(|r| match &r.min {
                Some(min) if r.min_exclusive => format!("{}>{min}", r.name),
                Some(min) => format!("{}>={min}", r.name),
                None => r.name.clone(),
            })
            .collect();
        row.push_str(&format!("  needs {}", needs.join(", ")));
    }
    if let Some(lv) = e.lv_min {
        row.push_str(&format!("  LabVIEW>={lv}"));
    }
    row.push_str(&format!("  from {}\n", e.source));
    printable(row)
}

/// `row` with every control character but the line end shown as `?`, so a
/// feed's names cannot send escape sequences to the terminal.
fn printable(row: String) -> String {
    let shown = |c: char| if c.is_control() && c != '\n' { '?' } else { c };
    row.chars().map(shown).collect()
}

pub fn list(installed: &[Manifest]) -> String {
    if installed.is_empty() {
        return "lvpm has installed nothing into this target\n".to_string();
    }
    installed
        .iter()
        .map(|m| {
            format!(
                "{:<52} {:<14} {:>5} files  {:<12}{}\n",
                m.name,
                m.version,
                m.files.len(),
                if m.relinked {
                    "relinked"
                } else {
                    "NOT relinked"
                },
                if m.skipped_hooks.is_empty() {
                    String::new()
                } else {
                    format!(
                        "  (hooks skipped: {})",
                        super::hooks::hook_names(&m.skipped_hooks)
                    )
                }
            )
        })
        .collect()
}

pub fn cache_add(imports: &[Import]) -> String {
    let mut text: String = imports
        .iter()
        .map(|import| {
            format!(
                "{} {} sha256:{}\n",
                if import.reused { "cached" } else { "imported" },
                import.path.display(),
                import.sha256
            )
        })
        .collect();
    text.push_str(&format!("{} package(s) processed\n", imports.len()));
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use lvpm_core::version::Version;
    use std::path::PathBuf;

    fn entry(name: &str, display_name: Option<&str>) -> Entry {
        Entry {
            name: name.into(),
            version: Version::parse("1.0"),
            url: String::new(),
            md5: None,
            display_name: display_name.map(str::to_owned),
            requires: Vec::new(),
            lv_min: None,
            source: String::new(),
        }
    }

    fn manifest(name: &str, relinked: bool, skipped_hooks: &[&str]) -> Manifest {
        Manifest {
            name: name.into(),
            version: "1.0".into(),
            display_name: None,
            installed_at: 0,
            target: String::new(),
            files: vec!["a".into(), "b".into()],
            skipped_hooks: skipped_hooks.iter().map(|h| h.to_string()).collect(),
            relinked,
            relink_folders: Vec::new(),
            post_install_vi: None,
            pre_install_vi: None,
            pre_uninstall_vi: None,
            post_uninstall_vi: None,
            skipped_groups: Vec::new(),
            archive_sha256: None,
        }
    }

    #[test]
    fn no_installations_and_no_hits_and_nothing_installed_are_each_said_so() {
        assert_eq!(targets(&[]), "no LabVIEW installations detected\n");
        assert_eq!(search("zzz", &[]), "no packages matching \"zzz\"\n");
        assert_eq!(list(&[]), "lvpm has installed nothing into this target\n");
    }

    #[test]
    fn a_search_shows_forty_hits_and_counts_the_rest() {
        let hits: Vec<Entry> = (0..43)
            .map(|n| entry(&format!("pkg{n:02}"), Some("A package")))
            .collect();
        let text = search("pkg", &hits);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 41);
        assert!(lines[0].starts_with("pkg00 "), "{}", lines[0]);
        assert!(
            lines[0].ends_with(" 1.0            A package"),
            "{}",
            lines[0]
        );
        assert_eq!(lines[39].split(' ').next(), Some("pkg39"));
        assert_eq!(lines[40], "... and 3 more");

        let exactly_forty = search("pkg", &hits[..40]);
        assert_eq!(exactly_forty.lines().count(), 40);
    }

    #[test]
    fn a_catalog_row_shows_requirements_minimum_labview_and_source_after_search_columns() {
        use lvpm_core::index::Requirement;

        let mut beta = entry("beta", Some("Beta"));
        beta.requires = vec![
            Requirement {
                name: "alpha".into(),
                min: Some(Version::parse("1.0")),
                min_exclusive: false,
            },
            Requirement {
                name: "gamma".into(),
                min: Some(Version::parse("2")),
                min_exclusive: true,
            },
        ];
        beta.lv_min = Some(20.0);
        beta.source = "http://feed/index.vipr".into();
        let mut plain = entry("plain", None);
        plain.source = "packages".into();
        let catalog = Catalog {
            entries: vec![beta, plain],
            sources: Vec::new(),
            failed: Vec::new(),
        };
        let text = super::catalog(None, &catalog);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[0],
            format!(
                "{:<NAME_WIDTH$} {:<VERSION_WIDTH$} Beta  needs alpha>=1.0, gamma>2  LabVIEW>=20  from http://feed/index.vipr",
                "beta", "1.0"
            )
        );
        assert_eq!(
            lines[1],
            format!(
                "{:<NAME_WIDTH$} {:<VERSION_WIDTH$}   from packages",
                "plain", "1.0"
            )
        );
        assert_eq!(lines.len(), 2);
    }

    fn empty_catalog(sources: &[&str]) -> Catalog {
        Catalog {
            entries: Vec::new(),
            sources: sources.iter().map(|s| s.to_string()).collect(),
            failed: Vec::new(),
        }
    }

    #[test]
    fn an_empty_catalog_names_the_sources_it_asked_or_says_there_are_none() {
        assert_eq!(
            catalog(None, &empty_catalog(&["packages", "http://feed"])),
            "no packages found in:\n  packages\n  http://feed\n"
        );
        assert_eq!(
            catalog(None, &empty_catalog(&[])),
            "no package sources are configured\n"
        );
    }

    #[test]
    fn an_empty_catalog_for_a_query_names_the_query() {
        assert_eq!(
            catalog(Some("zzz"), &empty_catalog(&["packages"])),
            "no packages matching \"zzz\"\n"
        );
    }

    #[test]
    fn each_unread_source_is_a_warning() {
        let mut unread = empty_catalog(&[]);
        unread.failed = vec![("http://feed".into(), "refused".into())];
        assert_eq!(
            unread_sources(&unread),
            "warning: could not read http://feed: refused\n"
        );
    }

    #[test]
    fn control_characters_from_a_feed_never_reach_the_terminal() {
        let mut hostile = entry("evil\x1b[2K", Some("Evil\x07\u{9b}"));
        hostile.source = "packages".into();
        let catalog = Catalog {
            entries: vec![hostile.clone()],
            sources: Vec::new(),
            failed: Vec::new(),
        };
        for text in [
            super::catalog(None, &catalog),
            search("evil", std::slice::from_ref(&hostile)),
        ] {
            assert!(
                !text.chars().any(|c| c.is_control() && c != '\n'),
                "{text:?}"
            );
            assert!(text.contains("evil?[2K"), "{text:?}");
        }
    }

    #[test]
    fn a_listed_package_says_whether_it_is_relinked_and_which_hooks_were_skipped() {
        let text = list(&[
            manifest("alpha", true, &[]),
            manifest(
                "beta",
                false,
                &["PostInstall=C:\\build\\Post.vi", "PreInstall=x"],
            ),
        ]);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(
            lines[0].ends_with("    2 files  relinked    "),
            "{}",
            lines[0]
        );
        assert!(
            lines[1].ends_with("NOT relinked  (hooks skipped: PostInstall, PreInstall)"),
            "{}",
            lines[1]
        );
    }

    #[test]
    fn imports_are_listed_then_counted() {
        let imports = [
            Import {
                path: PathBuf::from("a.vip"),
                sha256: "ab".into(),
                reused: false,
            },
            Import {
                path: PathBuf::from("b.vip"),
                sha256: "cd".into(),
                reused: true,
            },
        ];
        assert_eq!(
            cache_add(&imports),
            "imported a.vip sha256:ab\ncached b.vip sha256:cd\n2 package(s) processed\n"
        );
        assert_eq!(cache_add(&[]), "0 package(s) processed\n");
    }
}
