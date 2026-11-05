//! `[application]`: which LabVIEW project and build specification
//! `lvpm build --app` builds.
//!
//! ```toml
//! [application]
//! project    = "app/Main.lvproj"      # relative to lvpm.toml, inside its folder
//! build-spec = "My Application"       # the build specification's name in that project
//! ```
//!
//! The unknown-key check runs when any command reads `lvpm.toml`
//! ([`crate::project`]); everything else runs here, so `lvpm lock` works while
//! the `.lvproj` is still being set up. The SBOM names the application after
//! `project.name` and `project.version`.

use crate::project::{APPLICATION_KEYS, Project};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

/// A manifest checked for `lvpm build --app`.
#[derive(Debug, PartialEq)]
pub struct Application {
    /// The `.lvproj`, absolute.
    pub project: PathBuf,
    /// The build specification's name, as written.
    pub build_spec: String,
    /// `[project] name`.
    pub name: String,
    /// `[project] version`.
    pub version: String,
}

const EXTENSION: &str = ".lvproj";

/// `p`'s `[application]`, checked against the project folder `root`.
pub fn validate(p: &Project, root: &Path) -> Result<Application> {
    let Some(table) = &p.application else {
        bail!("lvpm.toml has no [application] table: lvpm build --app needs one");
    };
    let [project_key, build_spec_key] = APPLICATION_KEYS;
    let project = string(
        table,
        project_key,
        "the LabVIEW project, a .lvproj path relative to lvpm.toml",
    )?;
    let build_spec = string(
        table,
        build_spec_key,
        "the build specification's name in the LabVIEW project",
    )?;
    let project = project_file(root, &project)?;
    let (Some(name), Some(version)) = (non_blank(&p.name), non_blank(&p.version)) else {
        bail!(
            "an application build needs project.name and project.version in lvpm.toml, for the SBOM"
        );
    };
    Ok(Application {
        project,
        build_spec,
        name: name.to_string(),
        version: version.to_string(),
    })
}

fn non_blank(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|v| !v.trim().is_empty())
}

/// The key's value: present, a string and not blank.
fn string(table: &toml::Table, key: &str, what: &str) -> Result<String> {
    match table.get(key) {
        Some(toml::Value::String(s)) if !s.trim().is_empty() => Ok(s.clone()),
        None | Some(toml::Value::String(_)) => bail!("[application] needs {key} ({what})"),
        Some(_) => bail!("[application] {key} must be a string"),
    }
}

/// The `.lvproj` `rel` names below `root`, absolute. A link is followed before
/// the folder is checked, so one that leads out of the folder is refused.
fn project_file(root: &Path, rel: &str) -> Result<PathBuf> {
    if !rel.to_ascii_lowercase().ends_with(EXTENSION) {
        bail!("[application] project \"{rel}\" must end in {EXTENSION}");
    }
    let path = crate::lvlib::normalize(&root.join(rel));
    if !path.is_file() {
        bail!("[application] project \"{rel}\" is not a file");
    }
    let real = path
        .canonicalize()
        .with_context(|| format!("resolving {}", path.display()))?;
    let real_root = root
        .canonicalize()
        .with_context(|| format!("resolving {}", root.display()))?;
    if !real.starts_with(&real_root) {
        bail!("[application] project \"{rel}\" lies outside the folder with lvpm.toml");
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project;

    const GOOD: &str = r#"
[project]
name = "demo_app"
version = "1.2.3"

[application]
project = "app/Main.lvproj"
build-spec = "My Application"
"#;

    /// A project folder, `project` in a scratch folder of its own, with
    /// `app/Main.lvproj` and `app/Main.vi`. What lies beside it is outside it.
    fn folder(tag: &str) -> PathBuf {
        let scratch =
            std::env::temp_dir().join(format!("lvpm-application-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let root = scratch.join("project");
        std::fs::create_dir_all(root.join("app")).unwrap();
        std::fs::write(root.join("app/Main.lvproj"), "<Project/>").unwrap();
        std::fs::write(root.join("app/Main.vi"), "a vi").unwrap();
        root
    }

    /// Delete the scratch folder `root` is in.
    fn remove(root: &Path) {
        std::fs::remove_dir_all(root.parent().unwrap()).ok();
    }

    fn check(root: &Path, text: &str) -> Result<Application> {
        validate(&project::parse(text).unwrap(), root)
    }

    fn error(root: &Path, text: &str) -> String {
        format!("{:#}", check(root, text).unwrap_err())
    }

    fn with_application(lines: &str) -> String {
        let (head, _) = GOOD.split_once("[application]").unwrap();
        format!("{head}[application]\n{lines}")
    }

    #[test]
    fn a_complete_table_gives_the_absolute_project_the_spec_and_the_names() {
        let root = folder("good");
        let app = check(&root, GOOD).unwrap();
        assert_eq!(app.project, root.join("app").join("Main.lvproj"));
        assert!(app.project.is_absolute());
        assert_eq!(
            (
                app.build_spec.as_str(),
                app.name.as_str(),
                app.version.as_str()
            ),
            ("My Application", "demo_app", "1.2.3")
        );
        remove(&root);
    }

    #[test]
    fn a_path_that_goes_up_and_comes_back_is_kept_without_the_detour() {
        let root = folder("detour");
        let text = GOOD.replace("app/Main.lvproj", "app/../app/Main.lvproj");
        assert_eq!(
            check(&root, &text).unwrap().project,
            root.join("app").join("Main.lvproj")
        );
        remove(&root);
    }

    #[test]
    fn a_detour_through_a_folder_that_is_not_there_is_resolved_before_the_file_is_looked_for() {
        let root = folder("detour-missing");
        let text = GOOD.replace("app/Main.lvproj", "nowhere/../app/Main.lvproj");
        assert_eq!(
            check(&root, &text).unwrap().project,
            root.join("app").join("Main.lvproj")
        );
        remove(&root);
    }

    #[test]
    fn a_manifest_without_the_table_has_none_and_validate_says_so() {
        let none = project::parse("[project]\nname = \"x\"\n").unwrap();
        assert_eq!(none.application, None);
        let root = folder("none");
        assert_eq!(
            format!("{:#}", validate(&none, &root).unwrap_err()),
            "lvpm.toml has no [application] table: lvpm build --app needs one"
        );
        remove(&root);
    }

    #[test]
    fn each_problem_says_what_to_write_in_its_own_words() {
        let needs_project = "[application] needs project (the LabVIEW project, a .lvproj path relative to lvpm.toml)";
        let needs_spec = "[application] needs build-spec (the build specification's name in the LabVIEW project)";
        let root = folder("errors");
        let spec = "build-spec = \"S\"\n";
        let proj = "project = \"app/Main.lvproj\"\n";
        let cases = [
            (spec.to_string(), needs_project.to_string()),
            (format!("{spec}project = \"\"\n"), needs_project.to_string()),
            (proj.to_string(), needs_spec.to_string()),
            (format!("{proj}build-spec = \"\"\n"), needs_spec.to_string()),
            (
                format!("{proj}build-spec = \"   \"\n"),
                needs_spec.to_string(),
            ),
            (
                format!("{spec}project = \"  \"\n"),
                needs_project.to_string(),
            ),
            // Neither key: the project is asked for first.
            (String::new(), needs_project.to_string()),
            (
                format!("{spec}project = 5\n"),
                "[application] project must be a string".to_string(),
            ),
            (
                format!("{proj}build-spec = 5\n"),
                "[application] build-spec must be a string".to_string(),
            ),
            (
                format!("{spec}project = \"app/Missing.lvproj\"\n"),
                "[application] project \"app/Missing.lvproj\" is not a file".to_string(),
            ),
            (
                format!("{spec}project = \"app/Main.vi\"\n"),
                "[application] project \"app/Main.vi\" must end in .lvproj".to_string(),
            ),
            (
                format!("{spec}project = \"../other/X.lvproj\"\n"),
                "[application] project \"../other/X.lvproj\" lies outside the folder with lvpm.toml"
                    .to_string(),
            ),
        ];
        let outside = root.parent().unwrap().join("other");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("X.lvproj"), "<Project/>").unwrap();
        for (lines, said) in cases {
            assert_eq!(error(&root, &with_application(&lines)), said, "{lines}");
        }
        remove(&root);
    }

    #[test]
    fn a_missing_name_or_version_is_refused_for_the_sbom() {
        let root = folder("sbom");
        let said = "an application build needs project.name and project.version in lvpm.toml, for the SBOM";
        for text in [
            GOOD.replace("version = \"1.2.3\"\n", ""),
            GOOD.replace("name = \"demo_app\"\n", ""),
            GOOD.replace("\"1.2.3\"", "\"\""),
            GOOD.replace("\"1.2.3\"", "\"  \""),
            GOOD.replace("\"demo_app\"", "\" \t\""),
        ] {
            assert_eq!(error(&root, &text), said, "{text}");
        }
        remove(&root);
    }

    #[test]
    fn an_absolute_path_outside_the_folder_is_refused_and_one_inside_is_not() {
        let root = folder("absolute");
        let outside = root.parent().unwrap().join("X.lvproj");
        std::fs::write(&outside, "<Project/>").unwrap();
        let said = error(
            &root,
            &GOOD.replace(
                "app/Main.lvproj",
                &outside.to_string_lossy().replace('\\', "/"),
            ),
        );
        assert!(
            said.ends_with("lies outside the folder with lvpm.toml"),
            "{said}"
        );
        let inside = root.join("app").join("Main.lvproj");
        let text = GOOD.replace(
            "app/Main.lvproj",
            &inside.to_string_lossy().replace('\\', "/"),
        );
        assert_eq!(check(&root, &text).unwrap().project, inside);
        remove(&root);
    }

    #[test]
    fn a_directory_named_like_a_project_is_not_a_file() {
        let root = folder("directory");
        std::fs::create_dir_all(root.join("app/X.lvproj")).unwrap();
        let said = error(&root, &GOOD.replace("Main.lvproj", "X.lvproj"));
        assert_eq!(said, "[application] project \"app/X.lvproj\" is not a file");
        remove(&root);
    }

    #[test]
    fn the_extension_is_read_in_any_case() {
        let root = folder("case");
        std::fs::copy(root.join("app/Main.lvproj"), root.join("app/Other.LVPROJ")).unwrap();
        assert!(check(&root, &GOOD.replace("Main.lvproj", "Other.LVPROJ")).is_ok());
        remove(&root);
    }

    #[test]
    #[allow(clippy::print_stderr)] // the skip notice: libtest shows it
    fn a_link_that_leads_out_of_the_folder_is_refused() {
        let root = folder("link");
        let outside = root.parent().unwrap().join("linked-to");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("X.lvproj"), "<Project/>").unwrap();
        let link = root.join("linked");
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(&outside, &link).or_else(|_| {
            // Without Developer Mode a junction needs no privilege.
            let junction = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&link)
                .arg(&outside)
                .output()?;
            match junction.status.success() {
                true => Ok(()),
                false => Err(std::io::Error::other("mklink /J failed")),
            }
        });
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&outside, &link);
        #[cfg(unix)]
        made.as_ref().expect("a symlink can be made");
        if made.is_err() {
            eprintln!("skipped: neither a symlink nor a junction can be made here");
            remove(&root);
            return;
        }
        assert!(link.join("X.lvproj").is_file(), "the link leads there");
        let said = error(&root, &GOOD.replace("app/Main.lvproj", "linked/X.lvproj"));
        assert_eq!(
            said,
            "[application] project \"linked/X.lvproj\" lies outside the folder with lvpm.toml"
        );
        remove(&root);
    }
}
