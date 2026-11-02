//! `check`: every VI under a folder, read as bytes, and each link in its linker
//! tables that leads to a file that is not there. The installation a venv is
//! bound to is a folder to look in, never a program to run.
//!
//! A link is looked for where LabVIEW would look. A relative one is stored
//! relative to the VI file itself, so its first `Up` leaves the file. A
//! `<vilib>`, `<userlib>` or `<instrlib>` one is looked for in the bound
//! installation first, then in each venv package's `1/` tree. Existence is
//! what the file system says, or, for a link through an `.llb`, what the
//! `.llb` lists.
//!
//! A file that a venv install record lists belongs to that package, and a VI
//! may link into it only if the manifest names the package in a table the VI
//! may draw on. A file the bound installation holds is LabVIEW's own, since it
//! is looked for first, and so is one that no record lists.

use crate::errors::{Cancelled, CheckError};
use crate::linker::{self, Component, Kind, LinkPath, LinkerError, ViFacts};
use crate::manifest_edit::DependencyTable;
use crate::ops::OpEnv;
use crate::project::{Project, same_name};
use crate::venv::{self, Bound};
use crate::{distribution, install, lvlib, paths, project, target};
use anyhow::{Context, Result, anyhow};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// One rule a VI breaks. Declared in the order the rules are listed in, which
/// is the order findings are sorted in.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Problem {
    /// The file is not an RSRC container: text, a git-LFS pointer, a cut-short
    /// copy.
    NotResource,
    /// A linker table cannot be read: `block` is its four-character name and
    /// `offset` the file byte where the section or record that failed starts.
    Unreadable { block: &'static str, offset: usize },
    /// The VI was saved in a newer LabVIEW (`saved`, a year) than the
    /// project's minimum (`minimum`).
    TooNew { saved: u32, minimum: u32 },
    /// The VI keeps its compiled code in the file, not apart from the source.
    NotSeparated,
    /// A link to a file that is not there. `link` is shown as stored for a
    /// symbolic root (`<vilib>/x.vi`), and otherwise as the resolved path
    /// relative to the project root, with `/`.
    NotFound { link: String },
    /// A link into `package`, which the venv holds, that the manifest does not
    /// let the VI use: `only_in` is the table that names it, if one does (a
    /// library member may draw on `[dependencies]` only). `link` is as in
    /// `NotFound`.
    Undeclared {
        link: String,
        package: String,
        only_in: Option<DependencyTable>,
    },
}

/// A VI and a rule it breaks.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Finding {
    /// The VI, relative to the project root, with `/`.
    pub vi: String,
    pub problem: Problem,
}

/// Why links were not looked for.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Unchecked {
    /// A link into `folder` of the bound installation, which is not on this
    /// machine at `installation`, and no package holds the file.
    NoInstallation {
        folder: &'static str,
        installation: PathBuf,
    },
    /// A symbolic root lvpm does not resolve, as written: `<resource>`.
    Root(String),
}

/// What a check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    /// How many VIs were read, readable or not.
    pub vis: usize,
    /// How many links were not looked for, for each cause.
    pub not_checked: BTreeMap<Unchecked, usize>,
    /// Whether the manifest names a LabVIEW to compare save versions with.
    pub saves_checked: bool,
    /// Sorted by VI, then rule, then link; each once.
    pub findings: Vec<Finding>,
}

impl Checked {
    /// All the links that were not looked for, whatever the cause.
    pub fn not_checked_total(&self) -> usize {
        self.not_checked.values().sum()
    }
}

/// Read every `.vi .vit .ctl .ctt` under `dir` (the project root when None; a
/// relative one is taken from the directory the command ran in) and say which
/// links lead nowhere. Findings are a result, not an error.
pub fn check(env: &OpEnv, dir: Option<&Path>) -> Result<Checked, CheckError> {
    let repo = match &env.flags.project {
        Some(repo) => env.cwd.join(repo),
        None => venv::locate(&env.cwd, &env.config.venv_dir)?.ok_or_else(|| {
            anyhow!(
                "no {} at or above {}",
                project::FILE_NAME,
                env.cwd.display()
            )
        })?,
    };
    // Every path below is taken from these two, so a `..` in either is
    // settled here, once: `join` and `shown` work on names, not on `..`.
    let repo = lexical(&repo);
    let bound = venv::binding(&repo, &env.config.venv_dir)?;
    let (_, manifest) = env
        .manifest_for(None, Some(&bound.repo))?
        .ok_or_else(|| anyhow!("no {} in {}", project::FILE_NAME, bound.repo.display()))?;
    let minimum = manifest
        .labview
        .as_deref()
        .map(project::labview_year)
        .transpose()
        .map_err(anyhow::Error::from)?;

    let start = dir.map_or_else(
        || bound.repo.clone(),
        |dir| lexical(&paths::resolve(&env.cwd, dir)),
    );
    if !start.is_dir() {
        return Err(CheckError::NoDir(dir.map_or(start, Path::to_path_buf)));
    }
    let members = members(&manifest, &bound.repo)?;
    let mut files = Vec::new();
    walk(&start, &bound.dir, &mut files)?;

    let search = Search::of(&bound)?;
    let mut checked = Checked {
        vis: files.len(),
        not_checked: BTreeMap::new(),
        saves_checked: minimum.is_some(),
        findings: Vec::new(),
    };
    for file in &files {
        if env.cancel.is_cancelled() {
            return Err(anyhow::Error::from(Cancelled).into());
        }
        let bytes = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
        let vi = shown(&bound.repo, file);
        let uses = Uses {
            project: &manifest,
            member: members.contains(&key(file)),
        };
        let broken = problems(
            &bytes,
            file,
            &search,
            &uses,
            &bound.repo,
            minimum,
            &mut checked.not_checked,
        );
        checked
            .findings
            .extend(broken.into_iter().map(|problem| Finding {
                vi: vi.clone(),
                problem,
            }));
    }
    in_order(&mut checked.findings);
    Ok(checked)
}

/// The files of the library `[distribution]` names, as [`key`]s; none when the
/// manifest has no such table.
fn members(manifest: &Project, repo: &Path) -> Result<HashSet<PathBuf>> {
    let Some(dist) = &manifest.distribution else {
        return Ok(HashSet::new());
    };
    let library = repo.join(distribution::top_level_library(dist, repo)?);
    Ok(lvlib::members(&library)?.iter().map(|p| key(p)).collect())
}

/// A path as files are compared by: separators and `.` settled and, on
/// Windows, case folded, as the file system does.
fn key(path: &Path) -> PathBuf {
    let path = paths::clean(path);
    match cfg!(windows) {
        true => PathBuf::from(path.to_string_lossy().to_lowercase()),
        false => path,
    }
}

/// The one place the listing is put in order (VI, rule, link) and a repeat is
/// dropped.
fn in_order(findings: &mut Vec<Finding>) {
    findings.sort();
    findings.dedup();
}

/// `path` with `.` and `..` settled by name, as [`target::within`] does but
/// keeping case. A `..` that leads a relative path stays; one at a root goes.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if out
                    .components()
                    .all(|c| c == std::path::Component::ParentDir)
                {
                    out.push("..");
                } else {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The VIs, controls and templates under `dir`, in name order, leaving out the
/// venv, dot-folders and links, which could lead out of the project or round
/// in a loop.
fn walk(dir: &Path, venv: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort();
    for path in entries {
        let hidden = path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'));
        let link = std::fs::symlink_metadata(&path)?.file_type().is_symlink();
        if hidden || link || target::within(venv, &path) {
            continue;
        }
        if path.is_dir() {
            walk(&path, venv, out)?;
        } else if is_vi_file(&path) {
            out.push(path);
        }
    }
    Ok(())
}

fn is_vi_file(path: &Path) -> bool {
    path.extension().is_some_and(|ext| {
        ["vi", "vit", "ctl", "ctt"].contains(&ext.to_string_lossy().to_ascii_lowercase().as_str())
    })
}

/// What one VI's bytes break. A link that is looked for and not found is a
/// problem; one that cannot be looked for is counted in `unchecked`. `minimum`
/// is the oldest LabVIEW the project is for, when it says.
fn problems(
    bytes: &[u8],
    vi: &Path,
    search: &Search,
    uses: &Uses,
    repo: &Path,
    minimum: Option<u32>,
    unchecked: &mut BTreeMap<Unchecked, usize>,
) -> Vec<Problem> {
    match ViFacts::read(bytes) {
        Ok(facts) => {
            let mut found = save_problems(&facts, minimum);
            found.extend(link_problems(
                &facts.links,
                vi,
                search,
                uses,
                repo,
                unchecked,
            ));
            found
        }
        // A header or directory that runs out is a cut-short file.
        Err(LinkerError::NotResource | LinkerError::Truncated { .. }) => {
            vec![Problem::NotResource]
        }
        Err(LinkerError::Unreadable { block, offset }) => {
            vec![Problem::Unreadable { block, offset }]
        }
    }
}

/// What the way a VI was saved breaks. A save too old to be a year (LabVIEW 8.x
/// and older) is not too new. A VI whose compiled-code flag cannot be read is
/// unreadable already, so it is neither separated nor not.
fn save_problems(facts: &ViFacts, minimum: Option<u32>) -> Vec<Problem> {
    let mut found = Vec::new();
    if let (Some(minimum), Some(saved)) = (minimum, facts.save_year)
        && saved > minimum
    {
        found.push(Problem::TooNew { saved, minimum });
    }
    if !facts.separate_compiled {
        found.push(Problem::NotSeparated);
    }
    found
}

fn link_problems(
    links: &[LinkPath],
    vi: &Path,
    search: &Search,
    uses: &Uses,
    repo: &Path,
    unchecked: &mut BTreeMap<Unchecked, usize>,
) -> Vec<Problem> {
    let mut found = Vec::new();
    for link in links {
        match resolve(link, vi, search) {
            Resolved::File(file) => {
                if let Some(package) = search.owner(&file)
                    && let Some(only_in) = uses.barred(package)
                {
                    found.push(Problem::Undeclared {
                        link: shown_link(link, vi, repo),
                        package: package.to_string(),
                        only_in,
                    });
                }
            }
            Resolved::Skipped => {}
            Resolved::NotFound => found.push(Problem::NotFound {
                link: shown_link(link, vi, repo),
            }),
            Resolved::NotChecked(why) => *unchecked.entry(why).or_default() += 1,
        }
    }
    found
}

/// Where the places a symbolic link can lead are.
struct Search {
    /// The folder of the LabVIEW the venv is bound to.
    installation: PathBuf,
    /// Whether it is on this machine.
    present: bool,
    /// Each venv package's `<pkg>/1` tree, in name order.
    packages: Vec<PathBuf>,
    /// The package each file an install record lists belongs to, by [`key`].
    owners: HashMap<PathBuf, String>,
}

impl Search {
    fn of(bound: &Bound) -> Result<Search> {
        let mut packages = Vec::new();
        // A venv is not always made yet: its binding is, but its folder may hold only that.
        for entry in std::fs::read_dir(&bound.dir)
            .with_context(|| format!("reading {}", bound.dir.display()))?
        {
            let tree = entry?.path().join("1");
            if tree.is_dir() {
                packages.push(tree);
            }
        }
        packages.sort();
        let mut owners = HashMap::new();
        for record in install::list_installed(&bound.store())? {
            for file in &record.files {
                owners.insert(key(Path::new(file)), record.name.clone());
            }
        }
        Ok(Search {
            installation: bound.target.path.clone(),
            present: bound.target.path.is_dir(),
            packages,
            owners,
        })
    }

    /// The package that owns `file`. A member of an `.llb` is owned by whoever
    /// owns the `.llb`, which is the file a record lists.
    fn owner(&self, file: &Path) -> Option<&str> {
        let held = self.owners.get(&key(file));
        let llb = || file.parent().and_then(|dir| self.owners.get(&key(dir)));
        held.or_else(llb).map(String::as_str)
    }
}

/// What the manifest lets one VI link into.
struct Uses<'a> {
    project: &'a Project,
    /// Whether the VI is a member of the distribution library, which is held to
    /// `[dependencies]`; any other VI (a test) may draw on every table.
    member: bool,
}

impl Uses<'_> {
    /// `None` when the VI may use `package`. Otherwise `Some(only_in)`: the
    /// first table that names it, if any does.
    fn barred(&self, package: &str) -> Option<Option<DependencyTable>> {
        let named: Vec<_> = (self.project.tables())
            .filter(|(_, lines)| lines.iter().any(|(name, _)| same_name(name, package)))
            .map(|(table, _)| table)
            .collect();
        let may =
            named.contains(&DependencyTable::Dependencies) || (!self.member && !named.is_empty());
        (!may).then(|| named.into_iter().next())
    }
}

/// Where a link leads. The file of a `File` is looked up among those the
/// venv's packages own.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Resolved {
    /// The file it names, or, through an `.llb`, the path of the member.
    File(PathBuf),
    NotFound,
    /// It could not be looked for, and why.
    NotChecked(Unchecked),
    /// LabVIEW's "not a path": a subVI saved while missing. It names nothing.
    Skipped,
}

/// Where `link`, found in the VI file `vi`, leads.
fn resolve(link: &LinkPath, vi: &Path, search: &Search) -> Resolved {
    let found = |path: PathBuf| match exists(&path) {
        true => Resolved::File(path),
        false => Resolved::NotFound,
    };
    match &link.kind {
        Kind::NotAPath => Resolved::Skipped,
        Kind::Relative => found(join(vi.to_path_buf(), &link.components)),
        Kind::Absolute => found(PathBuf::from(link.to_path_string())),
        Kind::Symbolic(root) => {
            let Some(folder) = root.folder() else {
                return Resolved::NotChecked(Unchecked::Root(root.text().to_owned()));
            };
            let places = std::iter::once(&search.installation).chain(&search.packages);
            let held = places
                .map(|place| join(place.join(folder), &link.components))
                .find(|path| exists(path));
            match (held, search.present) {
                (Some(path), _) => Resolved::File(path),
                (None, true) => Resolved::NotFound,
                (None, false) => Resolved::NotChecked(Unchecked::NoInstallation {
                    folder,
                    installation: search.installation.clone(),
                }),
            }
        }
    }
}

/// `base` with `parts` followed: an `Up` leaves the last name.
fn join(mut base: PathBuf, parts: &[Component]) -> PathBuf {
    for part in parts {
        match part {
            Component::Up => {
                base.pop();
            }
            Component::Name(name) => base.push(name),
        }
    }
    base
}

/// Is there a file at `path`, or a member of an `.llb` of that name?
fn exists(path: &Path) -> bool {
    if path.is_file() {
        return true;
    }
    let (Some(llb), Some(name)) = (path.parent(), path.file_name()) else {
        return false;
    };
    let is_llb = llb
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("llb"));
    if !is_llb || !llb.is_file() {
        return false;
    }
    let name = name.to_string_lossy();
    // An `.llb` that cannot be read lists nothing.
    let members = std::fs::read(llb).map_or_else(
        |_| Vec::new(),
        |b| linker::llb_members(&b).unwrap_or_default(),
    );
    members.iter().any(|member| match cfg!(windows) {
        true => member.eq_ignore_ascii_case(&name),
        false => *member == *name,
    })
}

/// A path relative to the project root, with `/`; one outside it as it is.
fn shown(repo: &Path, path: &Path) -> String {
    match target::within(repo, path) {
        true => {
            let below = path.components().skip(repo.components().count());
            let names: Vec<_> = below.map(|c| c.as_os_str().to_string_lossy()).collect();
            names.join("/")
        }
        false => path.to_string_lossy().replace('\\', "/"),
    }
}

/// A link as the finding names it.
fn shown_link(link: &LinkPath, vi: &Path, repo: &Path) -> String {
    match &link.kind {
        Kind::Symbolic(root) => {
            let names = link.components.iter().map(|c| match c {
                Component::Up => "..",
                Component::Name(name) => name,
            });
            std::iter::once(root.text())
                .chain(names)
                .collect::<Vec<_>>()
                .join("/")
        }
        Kind::Relative => shown(repo, &join(vi.to_path_buf(), &link.components)),
        Kind::Absolute | Kind::NotAPath => shown(repo, Path::new(&link.to_path_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cancel::CancelToken;
    use crate::events::NullSink;
    use crate::linker::{Block, Rsrc, decode_pth0, encode_pth0};
    use crate::ops::testing;
    use crate::target::LvTarget;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/check");

    /// A project folder with an `lvpm.toml`, a venv bound to an installation
    /// that is not made yet, and no LabVIEW detected. Removed when dropped.
    struct Sandbox {
        root: PathBuf,
        repo: PathBuf,
        installation: PathBuf,
        venv_dir: &'static str,
    }

    impl Sandbox {
        fn new(tag: &str, labview: Option<&str>) -> Sandbox {
            Sandbox::with_venv_dir(tag, labview, ".lv-venv")
        }

        fn with_venv_dir(tag: &str, labview: Option<&str>, venv_dir: &'static str) -> Sandbox {
            let root = testing::scratch_dir(&format!("check-{tag}"));
            let (repo, installation) = (root.join("repo"), root.join("LabVIEW 2026"));
            std::fs::create_dir_all(&repo).unwrap();
            let key = labview.map_or(String::new(), |v| format!("labview = \">={v}\"\n"));
            std::fs::write(repo.join("lvpm.toml"), format!("[project]\n{key}")).unwrap();
            let target = LvTarget {
                version: 26.3,
                bitness: 64,
                path: installation.clone(),
            };
            let made = venv::create(
                &repo,
                Some("2026"),
                None,
                Path::new(venv_dir),
                &[target],
                &NullSink,
            );
            made.unwrap();
            Sandbox {
                root,
                repo,
                installation,
                venv_dir,
            }
        }

        /// A fixture VI (or `.llb`, or `.lvlib`) copied to `to` under the project.
        fn put(&self, fixture: &str, to: &str) -> &Sandbox {
            let from = Path::new(FIXTURES).join("project").join(fixture);
            self.write(to, &std::fs::read(from).unwrap())
        }

        fn write(&self, to: &str, bytes: &[u8]) -> &Sandbox {
            let to = self.repo.join(to);
            std::fs::create_dir_all(to.parent().unwrap()).unwrap();
            std::fs::write(to, bytes).unwrap();
            self
        }

        fn mkdir(&self, rel: &str) -> &Sandbox {
            std::fs::create_dir_all(self.repo.join(rel)).unwrap();
            self
        }

        /// The files a library VI is made of, as they are in the project.
        fn library(&self) -> &Sandbox {
            self.put("Lib.lvlib", "Lib.lvlib")
                .put("Good.vi", "Good.vi")
                .put("Helper.vi", "Helper.vi")
        }

        /// A file in the bound installation, which is made if it is not.
        fn installed(&self, rel: &str) -> &Sandbox {
            let to = self.installation.join(rel);
            std::fs::create_dir_all(to.parent().unwrap()).unwrap();
            std::fs::write(to, "").unwrap();
            self
        }

        /// Lines added to the manifest.
        fn declare(&self, toml: &str) -> &Sandbox {
            let path = self.repo.join("lvpm.toml");
            let text = std::fs::read_to_string(&path).unwrap();
            std::fs::write(path, format!("{text}\n{toml}\n")).unwrap();
            self
        }

        /// A library `Lib.lvlib` that lists exactly `members`, which are in the project.
        fn library_of(&self, members: &[&str]) -> &Sandbox {
            let items: String = (members.iter())
                .map(|m| format!("<Item Name=\"{m}\" Type=\"VI\" URL=\"../{m}\"/>"))
                .collect();
            self.write(
                "Lib.lvlib",
                format!("<Library>{items}</Library>").as_bytes(),
            )
        }

        /// A package the venv holds: the fixture `rel` below `packages/` as
        /// `rel` in its `vi.lib`, and the install record that lists it.
        fn package(&self, name: &str, rel: &str) -> &Sandbox {
            let venv = self.repo.join(self.venv_dir);
            let to = venv.join(name).join("1/vi.lib").join(rel);
            std::fs::create_dir_all(to.parent().unwrap()).unwrap();
            std::fs::copy(Path::new(FIXTURES).join("packages").join(rel), &to).unwrap();
            let record = serde_json::json!({
                "name": name, "version": "1.0", "installed_at": 0,
                "target": self.installation.to_string_lossy(),
                "files": [to.to_string_lossy().replace('\\', "/")], "skipped_hooks": [],
            });
            let record_path = venv.join(format!(".lvpm/installed/{name}.json"));
            std::fs::create_dir_all(record_path.parent().unwrap()).unwrap();
            std::fs::write(record_path, record.to_string()).unwrap();
            self
        }

        /// A project with `Lib.lvlib` as its distribution, `toml` after it in
        /// the manifest, and both fixture packages in its venv.
        fn with_packages(tag: &str, toml: &str) -> Sandbox {
            let sandbox = Sandbox::new(tag, None);
            sandbox
                .declare("[distribution]\ntop-level-library = \"Lib.lvlib\"")
                .declare(toml)
                .package("check_fixture_dep", DEP)
                .package("check_fixture_other", DEV);
            sandbox
        }

        fn env(&self) -> OpEnv<'static> {
            let mut env = testing::env(&self.repo);
            env.targets.clear();
            env.config.venv_dir = PathBuf::from(self.venv_dir);
            env
        }

        fn check(&self) -> Checked {
            check(&self.env(), None).unwrap()
        }

        fn check_in(&self, dir: &str) -> Result<Checked, CheckError> {
            check(&self.env(), Some(Path::new(dir)))
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn not_found(vi: &str, link: &str) -> Finding {
        Finding {
            vi: vi.into(),
            problem: Problem::NotFound { link: link.into() },
        }
    }

    /// The fixture packages' files, as they are below `packages/` and `vi.lib`.
    const DEP: &str = "Check Fixture/Dep.vi";
    const DEV: &str = "Check Fixture Dev/DevDep.vi";

    /// What `UsesDep.vi` and `UsesDevOnly.vi` link to.
    const DEP_LINK: &str = "<vilib>/Check Fixture/Dep.vi";
    const DEV_LINK: &str = "<vilib>/Check Fixture Dev/DevDep.vi";

    static NO_MANIFEST: std::sync::LazyLock<Project> = std::sync::LazyLock::new(Project::default);

    /// A VI no manifest table names anything for, and that is no library member.
    fn nobody() -> Uses<'static> {
        Uses {
            project: &NO_MANIFEST,
            member: false,
        }
    }

    const NI_LINK: &str = "<vilib>/Utility/Get Current LV Bitness.vi";
    const NI_FILE: &str = "vi.lib/Utility/Get Current LV Bitness.vi";

    #[test]
    fn a_clean_project_has_no_findings() {
        let sandbox = Sandbox::new("clean", Some("2026"));
        sandbox.library();
        let checked = sandbox.check();
        assert_eq!(checked.vis, 2);
        assert_eq!(checked.findings, []);
        assert_eq!(checked.not_checked_total(), 0);
        assert!(checked.saves_checked);
    }

    #[test]
    fn a_manifest_without_labview_says_save_versions_are_not_checked() {
        let sandbox = Sandbox::new("no-labview", None);
        sandbox.library();
        assert!(!sandbox.check().saves_checked);
    }

    #[test]
    fn a_missing_subvi_is_named_with_its_vi() {
        let sandbox = Sandbox::new("missing", None);
        sandbox.put("Broken.vi", "Broken.vi").mkdir("Sub");
        let checked = sandbox.check();
        assert_eq!(checked.vis, 1);
        assert_eq!(checked.findings, [not_found("Broken.vi", "Sub/Gone.vi")]);
    }

    #[test]
    fn a_link_through_an_llb_needs_the_member_to_be_listed() {
        let sandbox = Sandbox::new("llb", None);
        sandbox
            .put("UsesLlb.vi", "UsesLlb.vi")
            .put("Archive.llb", "Archive.llb");
        assert_eq!(
            sandbox.check().findings,
            [not_found("UsesLlb.vi", "Archive.llb/Removed.vi")]
        );
    }

    #[test]
    fn a_missing_llb_fails_every_link_through_it() {
        let sandbox = Sandbox::new("no-llb", None);
        sandbox.put("UsesLlb.vi", "UsesLlb.vi");
        assert_eq!(
            sandbox.check().findings,
            [
                not_found("UsesLlb.vi", "Archive.llb/InLlb.vi"),
                not_found("UsesLlb.vi", "Archive.llb/Removed.vi"),
            ]
        );
    }

    #[test]
    fn an_llb_that_is_not_one_lists_nothing() {
        let sandbox = Sandbox::new("bad-llb", None);
        sandbox
            .put("UsesLlb.vi", "UsesLlb.vi")
            .write("Archive.llb", b"not an llb");
        assert_eq!(sandbox.check().findings.len(), 2);
    }

    #[test]
    fn two_vis_with_the_same_missing_link_give_two_findings() {
        let sandbox = Sandbox::new("twice", None);
        sandbox
            .put("Broken.vi", "Broken.vi")
            .put("Broken.vi", "Broken2.vi");
        let checked = sandbox.check();
        assert_eq!(checked.vis, 2);
        assert_eq!(
            checked.findings,
            [
                not_found("Broken.vi", "Sub/Gone.vi"),
                not_found("Broken2.vi", "Sub/Gone.vi"),
            ]
        );
    }

    #[test]
    fn an_unreadable_linker_table_is_a_finding_at_its_offset() {
        let mut bytes = std::fs::read(Path::new(FIXTURES).join("project/Good.vi")).unwrap();
        let rsrc = Rsrc::read(&bytes).unwrap();
        let at = rsrc
            .blocks
            .iter()
            .find(|b| &b.kind == b"LIvi")
            .unwrap()
            .offset;
        bytes[at..at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        let sandbox = Sandbox::new("unreadable", None);
        sandbox.write("Good.vi", &bytes);
        let want = Finding {
            vi: "Good.vi".into(),
            problem: Problem::Unreadable {
                block: "LIvi",
                offset: at,
            },
        };
        assert_eq!(sandbox.check().findings, [want]);
    }

    #[test]
    fn text_named_vi_is_not_a_resource_file_and_neither_is_a_cut_short_one() {
        let sandbox = Sandbox::new("not-a-vi", None);
        let good = std::fs::read(Path::new(FIXTURES).join("project/Good.vi")).unwrap();
        sandbox
            .write("x.vi", b"not a VI")
            .write("cut.vi", &good[..20]);
        let vi = |vi: &str| Finding {
            vi: vi.into(),
            problem: Problem::NotResource,
        };
        assert_eq!(sandbox.check().findings, [vi("cut.vi"), vi("x.vi")]);
    }

    #[test]
    fn a_vilib_link_is_looked_for_in_the_bound_installation() {
        let sandbox = Sandbox::new("vilib-here", None);
        sandbox.put("UsesNi.vi", "UsesNi.vi").installed(NI_FILE);
        let checked = sandbox.check();
        assert_eq!((checked.vis, checked.findings.len()), (1, 0));
        assert_eq!(checked.not_checked_total(), 0);
    }

    #[test]
    fn a_vilib_file_missing_from_the_installation_is_a_broken_link() {
        let sandbox = Sandbox::new("vilib-gone", None);
        sandbox
            .put("UsesNi.vi", "UsesNi.vi")
            .installed("vi.lib/other.vi");
        let checked = sandbox.check();
        assert_eq!(checked.findings, [not_found("UsesNi.vi", NI_LINK)]);
        assert_eq!(checked.not_checked_total(), 0);
    }

    #[test]
    fn with_the_installation_absent_vilib_links_are_not_checked_and_do_not_fail() {
        let sandbox = Sandbox::new("no-installation", None);
        sandbox.put("UsesNi.vi", "UsesNi.vi");
        let checked = sandbox.check();
        assert_eq!(checked.findings, []);
        let cause = Unchecked::NoInstallation {
            folder: "vi.lib",
            installation: sandbox.installation.clone(),
        };
        assert_eq!(checked.not_checked, BTreeMap::from([(cause, 1)]));
    }

    #[test]
    fn links_not_checked_do_not_hide_real_findings() {
        let sandbox = Sandbox::new("not-hidden", None);
        sandbox
            .put("UsesNi.vi", "UsesNi.vi")
            .put("Broken.vi", "Broken.vi")
            .mkdir("Sub");
        let checked = sandbox.check();
        assert_eq!(checked.vis, 2);
        assert_eq!(checked.findings, [not_found("Broken.vi", "Sub/Gone.vi")]);
        assert_eq!(checked.not_checked_total(), 1);
    }

    #[test]
    fn a_venv_package_holds_a_vilib_file_whether_or_not_the_installation_is_there() {
        let sandbox = Sandbox::new("package", None);
        sandbox.put("UsesNi.vi", "UsesNi.vi");
        sandbox.write(&format!(".lv-venv/pkg/1/{NI_FILE}"), b"");
        let checked = sandbox.check();
        assert_eq!(checked.findings, []);
        assert_eq!(checked.not_checked_total(), 0);
    }

    #[test]
    fn without_a_venv_the_hint_is_the_one_a_manifest_alone_gets() {
        let root = testing::scratch_dir("check-no-venv");
        std::fs::write(root.join("lvpm.toml"), "[project]\n").unwrap();
        let why = check(&testing::env(&root), None).unwrap_err();
        assert!(why.to_string().contains("but no venv"), "{why}");
        assert!(
            why.to_string().contains("`lvpm venv create` makes one"),
            "{why}"
        );

        let bare = testing::scratch_dir("check-no-manifest");
        let why = check(&testing::env(&bare), None).unwrap_err();
        assert!(
            why.to_string().starts_with("no lvpm.toml at or above"),
            "{why}"
        );
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&bare);
    }

    #[test]
    fn a_project_flag_names_the_repo_from_anywhere() {
        let sandbox = Sandbox::new("project-flag", None);
        sandbox.put("Broken.vi", "Broken.vi");
        let mut env = sandbox.env();
        env.cwd = sandbox.root.clone();
        env.flags.project = Some(PathBuf::from("repo"));
        assert_eq!(check(&env, None).unwrap().findings.len(), 1);

        env.flags.project = Some(PathBuf::from("."));
        let why = check(&env, None).unwrap_err();
        assert!(why.to_string().contains("no venv in"), "{why}");
    }

    #[test]
    fn dir_limits_the_walk_and_names_paths_from_the_project_root() {
        let sandbox = Sandbox::new("dir", None);
        sandbox.put("Broken.vi", "Old/Broken.vi");
        sandbox
            .put("Lib.lvlib", "New/Lib.lvlib")
            .put("Good.vi", "New/Good.vi")
            .put("Helper.vi", "New/Helper.vi");
        let new = sandbox.check_in("New").unwrap();
        assert_eq!((new.vis, new.findings.len()), (2, 0));
        let old = sandbox.check_in("Old").unwrap();
        assert_eq!(
            old.findings,
            [not_found("Old/Broken.vi", "Old/Sub/Gone.vi")]
        );
        assert_eq!(sandbox.check().vis, 3);
    }

    #[test]
    fn a_dir_that_is_not_there_is_named_as_it_was_written() {
        let sandbox = Sandbox::new("no-dir", None);
        sandbox.put("Good.vi", "Good.vi");
        let why = sandbox.check_in("Nope").unwrap_err();
        assert!(matches!(&why, CheckError::NoDir(dir) if dir == Path::new("Nope")));
        assert_eq!(why.to_string(), "folder Nope does not exist");
    }

    #[test]
    fn no_vis_is_a_check_of_nothing() {
        let sandbox = Sandbox::new("empty", None);
        let checked = sandbox.check();
        assert_eq!((checked.vis, checked.findings.len()), (0, 0));
    }

    #[test]
    fn the_venv_dot_folders_and_other_files_are_not_walked() {
        // A venv folder that is not a dot-folder is skipped by being the venv.
        let sandbox = Sandbox::with_venv_dir("walk", None, "venv");
        sandbox.library();
        sandbox
            .put("Broken.vi", "venv/pkg/1/vi.lib/Broken.vi")
            .put("Broken.vi", ".hidden/Broken.vi")
            .write("notes.txt", b"x")
            .write(
                "Upper.VI",
                &std::fs::read(Path::new(FIXTURES).join("project/Helper.vi")).unwrap(),
            );
        let checked = sandbox.check();
        assert_eq!(checked.vis, 3, "Good, Helper and Upper.VI");
        // Upper.VI is Helper.vi's bytes: it links to Lib.lvlib only.
        assert_eq!(checked.findings, []);
    }

    #[test]
    fn a_linked_folder_is_not_followed() {
        let sandbox = Sandbox::new("symlink", None);
        let outside = sandbox.root.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::copy(
            Path::new(FIXTURES).join("project/Broken.vi"),
            outside.join("Broken.vi"),
        )
        .unwrap();
        let link = sandbox.repo.join("linked");
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
        made.expect("a link to a folder can be made");
        assert!(link.join("Broken.vi").is_file(), "the link leads there");
        assert_eq!(sandbox.check().vis, 0);
    }

    /// Broken.vi's bytes with its link `Sub/Gone.vi` rewritten, in the same
    /// number of bytes, as `../../Missing.vi`: two `Up`s, as a VI two levels
    /// below a file it calls stores it.
    fn two_ups_vi() -> Vec<u8> {
        let bytes = std::fs::read(Path::new(FIXTURES).join("project/Broken.vi")).unwrap();
        let from = b"\x03Sub\x07Gone.vi";
        let to = b"\x00\x0aMissing.vi";
        let mut out = Vec::new();
        let mut rest = &bytes[..];
        while let Some(at) = rest.windows(from.len()).position(|w| w == from) {
            out.extend(&rest[..at]);
            out.extend(to);
            rest = &rest[at + from.len()..];
        }
        out.extend(rest);
        let links = ViFacts::read(&out).unwrap().links;
        let ups = [Component::Up, Component::Up];
        assert_eq!(links[0].components[..2], ups);
        out
    }

    #[test]
    fn a_dotdot_in_the_project_flag_or_dir_changes_neither_what_is_found_nor_how_it_is_shown() {
        let sandbox = Sandbox::new("dotdot", None);
        sandbox.write("Top.vi", &two_ups_vi()).mkdir("Sub");
        std::fs::write(sandbox.root.join("Missing.vi"), "").unwrap();
        let mut env = sandbox.env();
        // From `repo/Sub`, the project is `..`; the VI's two `Up`s lead to the
        // file beside the repo, not into `repo/Sub`.
        env.cwd = sandbox.repo.join("Sub");
        env.flags.project = Some(PathBuf::from(".."));
        let checked = check(&env, None).unwrap();
        assert_eq!((checked.vis, checked.findings), (1, vec![]));
        std::fs::remove_file(sandbox.root.join("Missing.vi")).unwrap();
        assert_eq!(check(&env, None).unwrap().findings.len(), 1);

        // From `repo/Old`, DIR `../New` is `New`, and is shown so.
        let sandbox = Sandbox::new("dotdot-dir", None);
        sandbox.put("Broken.vi", "New/Broken.vi").mkdir("Old");
        let mut env = sandbox.env();
        env.cwd = sandbox.repo.join("Old");
        let checked = check(&env, Some(Path::new("../New"))).unwrap();
        assert_eq!(
            checked.findings,
            [not_found("New/Broken.vi", "New/Sub/Gone.vi")]
        );
    }

    #[test]
    fn a_pilot_control_is_read_and_is_not_unreadable() {
        use std::io::Read;
        let vip = Path::new(FIXTURES)
            .join("../pilots/labview_open_source_lib_data_manipulation-1.6.0.16.vip");
        let mut zip = zip::ZipArchive::new(std::fs::File::open(vip).unwrap()).unwrap();
        let mut bytes = Vec::new();
        let name = "File Group 0/vi.lib/LabVIEW Open Source Project/Data Manipulation/StringFormatter/Localization Decimal Separator -- enum.ctl";
        zip.by_name(name).unwrap().read_to_end(&mut bytes).unwrap();
        ViFacts::read(&bytes).expect("a typedef is readable");
        let sandbox = Sandbox::new("pilot-ctl", None);
        sandbox.write("T.ctl", &bytes);
        let unreadable = |f: &Finding| matches!(f.problem, Problem::Unreadable { .. });
        let checked = sandbox.check();
        assert_eq!(checked.vis, 1);
        assert!(!checked.findings.iter().any(unreadable));
    }

    #[test]
    fn a_cancelled_check_stops_before_it_reads_a_vi() {
        let sandbox = Sandbox::new("cancel", None);
        sandbox.put("Broken.vi", "Broken.vi");
        let mut env = sandbox.env();
        env.cancel = CancelToken::new();
        assert!(env.cancel.cancel());
        let why = check(&env, None).unwrap_err();
        assert!(
            matches!(&why, CheckError::Other(why) if why.is::<Cancelled>()),
            "{why}"
        );
    }

    /// A search with an installation of its own, which is `present` if it is
    /// made, and no packages.
    fn search_in(dir: &Path) -> Search {
        Search {
            installation: dir.join("LabVIEW"),
            present: dir.join("LabVIEW").is_dir(),
            packages: Vec::new(),
            owners: HashMap::new(),
        }
    }

    fn link(kind: Kind, parts: &[&str]) -> LinkPath {
        let components = parts.iter().map(|p| match p {
            &".." => Component::Up,
            name => Component::Name(name.to_string()),
        });
        LinkPath {
            kind,
            components: components.collect(),
        }
    }

    #[test]
    fn a_relative_link_starts_from_the_vi_file_and_each_up_leaves_one_level() {
        let dir = testing::scratch_dir("check-resolve-rel");
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::write(dir.join("a/x.vi"), "").unwrap();
        std::fs::write(dir.join("a/b/Helper.vi"), "").unwrap();
        let (vi, search) = (dir.join("a/b/V.vi"), search_in(&dir));
        // `Up` leaves the file, so `[Up, Helper.vi]` is the VI's own folder.
        let sibling = link(Kind::Relative, &["..", "Helper.vi"]);
        assert_eq!(
            resolve(&sibling, &vi, &search),
            Resolved::File(dir.join("a/b/Helper.vi"))
        );
        let parent = link(Kind::Relative, &["..", "..", "x.vi"]);
        assert_eq!(
            resolve(&parent, &vi, &search),
            Resolved::File(dir.join("a/x.vi"))
        );
        let gone = link(Kind::Relative, &["..", "..", "gone.vi"]);
        assert_eq!(resolve(&gone, &vi, &search), Resolved::NotFound);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_absolute_link_is_checked_as_it_is() {
        let dir = testing::scratch_dir("check-resolve-abs");
        let here = dir.join("x.vi");
        std::fs::write(&here, "").unwrap();
        let absolute = |path: &Path| decode_pth0(&encode_pth0(path).unwrap()).unwrap().0;
        let vi = dir.join("V.vi");
        let search = search_in(&dir);
        assert_eq!(
            resolve(&absolute(&here), &vi, &search),
            Resolved::File(here)
        );
        assert_eq!(
            resolve(&absolute(&dir.join("gone.vi")), &vi, &search),
            Resolved::NotFound
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_root_lvpm_does_not_resolve_is_not_checked_and_a_not_a_path_is_skipped() {
        let dir = testing::scratch_dir("check-resolve-other");
        let (vi, search) = (dir.join("V.vi"), search_in(&dir));
        let resource = link(
            Kind::Symbolic(linker::Root::Other("<resource>".into())),
            &["x.vi"],
        );
        assert_eq!(
            resolve(&resource, &vi, &search),
            Resolved::NotChecked(Unchecked::Root("<resource>".into()))
        );
        let nothing = LinkPath {
            kind: Kind::NotAPath,
            components: Vec::new(),
        };
        assert_eq!(resolve(&nothing, &vi, &search), Resolved::Skipped);

        let mut unchecked = BTreeMap::new();
        let links = [resource, nothing];
        assert_eq!(
            link_problems(&links, &vi, &search, &nobody(), &dir, &mut unchecked),
            []
        );
        assert_eq!(unchecked.values().sum::<usize>(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_link_stored_in_two_tables_is_one_finding() {
        let dir = testing::scratch_dir("check-resolve-twice");
        // As stored: relative to the VI file, so it starts with an `Up`.
        let record = b"PTH0\0\0\0\x11\0\x01\0\x03\0\x03Sub\x07Gone.vi";
        let path = decode_pth0(record).unwrap().0;
        assert_eq!(path, link(Kind::Relative, &["..", "Sub", "Gone.vi"]));
        let table = [b"\0\x01LVIN\0\0\0\x01".as_slice(), record].concat();
        let empty: &[u8] = b"\0\x01FPHP\0\0\0\0\0\x03";
        let blocks = [
            (b"LIvi", &table[..]),
            (b"LIfp", &table[..]),
            (b"LIbd", empty),
        ]
        .map(|(kind, data)| Block {
            kind: *kind,
            id: 0,
            offset: 0,
            data: Some(data),
        })
        .to_vec();
        let links = Rsrc { blocks }.links().unwrap();
        let (vi, search) = (dir.join("V.vi"), search_in(&dir));
        let found = link_problems(&links, &vi, &search, &nobody(), &dir, &mut BTreeMap::new());
        assert_eq!(
            found,
            [Problem::NotFound {
                link: "Sub/Gone.vi".into()
            }]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn findings_are_sorted_by_vi_then_rule_then_link_and_each_comes_once() {
        let vi = |vi: &str, problem| Finding {
            vi: vi.into(),
            problem,
        };
        let gone = |link: &str| Problem::NotFound { link: link.into() };
        let mut found = vec![
            vi("b.vi", gone("z")),
            vi("a.vi", gone("z")),
            vi("a.vi", gone("a")),
            vi("a.vi", gone("a")),
            vi(
                "a.vi",
                Problem::Unreadable {
                    block: "LIvi",
                    offset: 9,
                },
            ),
            vi("a.vi", Problem::NotResource),
        ];
        in_order(&mut found);
        assert_eq!(
            found,
            [
                vi("a.vi", Problem::NotResource),
                vi(
                    "a.vi",
                    Problem::Unreadable {
                        block: "LIvi",
                        offset: 9
                    }
                ),
                vi("a.vi", gone("a")),
                vi("a.vi", gone("z")),
                vi("b.vi", gone("z")),
            ]
        );
    }

    fn too_new(vi: &str, saved: u32, minimum: u32) -> Finding {
        Finding {
            vi: vi.into(),
            problem: Problem::TooNew { saved, minimum },
        }
    }

    fn not_separated(vi: &str) -> Finding {
        Finding {
            vi: vi.into(),
            problem: Problem::NotSeparated,
        }
    }

    /// The pilot VI saved in LabVIEW 2013 (see `a_pilot_control_is_read...`).
    fn pilot_vi_2013() -> Vec<u8> {
        use std::io::Read;
        let vip = Path::new(FIXTURES)
            .join("../pilots/labview_open_source_lib_data_manipulation-1.6.0.16.vip");
        let mut zip = zip::ZipArchive::new(std::fs::File::open(vip).unwrap()).unwrap();
        let name =
            "File Group 0/vi.lib/LabVIEW Open Source Project/Data Manipulation/KVP Parser.vi";
        let mut bytes = Vec::new();
        zip.by_name(name).unwrap().read_to_end(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn a_vi_saved_after_the_minimum_is_too_new_and_each_names_both_years() {
        let sandbox = Sandbox::new("too-new", Some("2025"));
        sandbox.library();
        let checked = sandbox.check();
        assert!(checked.saves_checked);
        assert_eq!(
            checked.findings,
            [
                too_new("Good.vi", 2026, 2025),
                too_new("Helper.vi", 2026, 2025)
            ]
        );
    }

    #[test]
    fn a_vi_saved_in_the_minimum_year_or_before_it_passes_and_one_year_after_does_not() {
        // The pilot VI links to files this project has not got: only its age is looked at.
        let too_new_under = |minimum: &str| {
            let sandbox = Sandbox::new(&format!("saved-2013-under-{minimum}"), Some(minimum));
            sandbox.write("KVP Parser.vi", &pilot_vi_2013());
            let found = sandbox.check().findings;
            found
                .into_iter()
                .filter(|f| matches!(f.problem, Problem::TooNew { .. }))
                .collect::<Vec<_>>()
        };
        assert_eq!(too_new_under("2026"), []);
        assert_eq!(too_new_under("2013"), []);
        assert_eq!(
            too_new_under("2012"),
            [too_new("KVP Parser.vi", 2013, 2012)]
        );
    }

    #[test]
    fn without_a_minimum_no_vi_is_too_new() {
        let sandbox = Sandbox::new("no-minimum", None);
        sandbox.library();
        let checked = sandbox.check();
        assert!(!checked.saves_checked);
        assert_eq!(checked.findings, []);
    }

    #[test]
    fn a_save_older_than_labview_9_has_no_year_and_is_never_too_new() {
        // The first byte of `vers` 4 is the major, in BCD: 0x08 is LabVIEW 8.
        let mut bytes = std::fs::read(Path::new(FIXTURES).join("project/Good.vi")).unwrap();
        let at = Rsrc::read(&bytes)
            .unwrap()
            .blocks
            .iter()
            .find(|b| &b.kind == b"vers" && b.id == 4)
            .unwrap()
            .offset;
        bytes[at + 4] = 0x08;
        assert_eq!(Rsrc::read(&bytes).unwrap().save_year(), None);
        let sandbox = Sandbox::new("vers-8", Some("2025"));
        sandbox.library().write("Good.vi", &bytes);
        assert_eq!(sandbox.check().findings, [too_new("Helper.vi", 2026, 2025)]);
    }

    #[test]
    fn a_vi_that_keeps_its_compiled_code_is_named_and_a_separated_one_is_not() {
        let sandbox = Sandbox::new("not-separated", None);
        sandbox
            .put("Lib.lvlib", "Lib.lvlib")
            .put("Helper.vi", "Helper.vi")
            .put("NotSeparated.vi", "NotSeparated.vi");
        assert_eq!(sandbox.check().findings, [not_separated("NotSeparated.vi")]);
    }

    #[test]
    fn a_vi_with_no_readable_compiled_code_flag_is_unreadable_never_a_pass() {
        // The `LVSR` section's size set to 4: too short to hold the flags at byte 8.
        let mut bytes = std::fs::read(Path::new(FIXTURES).join("project/Good.vi")).unwrap();
        let at = Rsrc::read(&bytes)
            .unwrap()
            .blocks
            .iter()
            .find(|b| &b.kind == b"LVSR")
            .unwrap()
            .offset;
        bytes[at..at + 4].copy_from_slice(&4u32.to_be_bytes());
        let sandbox = Sandbox::new("no-flag", Some("2025"));
        sandbox.write("Good.vi", &bytes);
        let want = Finding {
            vi: "Good.vi".into(),
            problem: Problem::Unreadable {
                block: "LVSR",
                offset: at,
            },
        };
        assert_eq!(sandbox.check().findings, [want]);
    }

    #[test]
    fn one_vi_breaking_several_rules_gives_one_finding_per_rule_in_rule_order() {
        let sandbox = Sandbox::new("several", Some("2025"));
        sandbox
            .put("NotSeparated.vi", "NotSeparated.vi")
            .put("Broken.vi", "Broken.vi")
            .mkdir("Sub");
        // NotSeparated.vi links Helper.vi and Lib.lvlib, neither of which is here.
        assert_eq!(
            sandbox.check().findings,
            [
                too_new("Broken.vi", 2026, 2025),
                not_found("Broken.vi", "Sub/Gone.vi"),
                too_new("NotSeparated.vi", 2026, 2025),
                not_separated("NotSeparated.vi"),
                not_found("NotSeparated.vi", "Helper.vi"),
                not_found("NotSeparated.vi", "Lib.lvlib"),
            ]
        );
    }

    fn undeclared(
        vi: &str,
        link: &str,
        package: &str,
        only_in: Option<DependencyTable>,
    ) -> Finding {
        Finding {
            vi: vi.into(),
            problem: Problem::Undeclared {
                link: link.into(),
                package: package.into(),
                only_in,
            },
        }
    }

    #[test]
    fn a_link_into_a_package_the_manifest_does_not_name_is_undeclared_until_it_is() {
        // The installation is not made, so the package is the only place the file is.
        let sandbox = Sandbox::with_packages("undeclared", "");
        sandbox
            .library_of(&["UsesDep.vi"])
            .put("UsesDep.vi", "UsesDep.vi");
        assert_eq!(
            sandbox.check().findings,
            [undeclared(
                "UsesDep.vi",
                DEP_LINK,
                "check_fixture_dep",
                None
            )]
        );
        sandbox.declare("[dependencies]\ncheck_fixture_dep = \"*\"");
        assert_eq!(sandbox.check().findings, []);
    }

    #[test]
    fn a_package_is_named_in_the_manifest_whatever_the_case() {
        let sandbox = Sandbox::with_packages("case", "[dependencies]\nCheck_Fixture_DEP = \"*\"");
        sandbox
            .library_of(&["UsesDep.vi"])
            .put("UsesDep.vi", "UsesDep.vi");
        assert_eq!(sandbox.check().findings, []);
    }

    #[test]
    fn a_package_that_is_installed_but_not_declared_is_not_enough() {
        let sandbox =
            Sandbox::with_packages("other-only", "[dependencies]\ncheck_fixture_other = \"*\"");
        sandbox
            .library_of(&["UsesDep.vi"])
            .put("UsesDep.vi", "UsesDep.vi");
        assert_eq!(
            sandbox.check().findings,
            [undeclared(
                "UsesDep.vi",
                DEP_LINK,
                "check_fixture_dep",
                None
            )]
        );
    }

    #[test]
    fn a_file_the_installation_holds_is_labviews_own_even_when_a_package_holds_it_too() {
        let sandbox = Sandbox::with_packages("precedence", "");
        sandbox
            .library_of(&["UsesDep.vi"])
            .put("UsesDep.vi", "UsesDep.vi")
            .installed(&format!("vi.lib/{DEP}"));
        assert_eq!(sandbox.check().findings, []);
    }

    #[test]
    fn a_vilib_file_that_no_record_lists_is_labviews_own() {
        let sandbox = Sandbox::with_packages("no-record", "");
        sandbox
            .library_of(&["UsesNi.vi"])
            .put("UsesNi.vi", "UsesNi.vi");
        sandbox.write(&format!(".lv-venv/pkg/1/{NI_FILE}"), b"");
        assert_eq!(sandbox.check().findings, []);
    }

    #[test]
    fn a_member_may_use_only_dependencies_and_any_other_vi_any_table_but_none_is_no_table() {
        let group = DependencyTable::Group("ci".into());
        for (declared, only_in) in [
            (
                "[dev-dependencies]\ncheck_fixture_other = \"*\"",
                Some(DependencyTable::DevDependencies),
            ),
            (
                "[dependency-groups.ci]\ncheck_fixture_other = \"*\"",
                Some(group),
            ),
            ("", None),
        ] {
            let sandbox = Sandbox::with_packages("tables", declared);
            sandbox
                .library_of(&["UsesDevOnly.vi"])
                .put("UsesDevOnly.vi", "UsesDevOnly.vi")
                .put("Tests/TestDevOnly.vi", "Tests/TestDevOnly.vi");
            let member = undeclared("UsesDevOnly.vi", DEV_LINK, "check_fixture_other", only_in);
            let test = undeclared(
                "Tests/TestDevOnly.vi",
                DEV_LINK,
                "check_fixture_other",
                None,
            );
            let want = match declared.is_empty() {
                true => vec![test, member],
                false => vec![member],
            };
            assert_eq!(sandbox.check().findings, want, "{declared:?}");
        }
    }

    #[test]
    fn without_a_distribution_every_vi_may_use_every_declared_table() {
        let sandbox = Sandbox::new("no-distribution", None);
        sandbox
            .declare("[dev-dependencies]\ncheck_fixture_other = \"*\"")
            .package("check_fixture_other", DEV)
            .put("UsesDevOnly.vi", "UsesDevOnly.vi")
            .put("Lib.lvlib", "Lib.lvlib");
        assert_eq!(sandbox.check().findings, []);
    }

    #[test]
    fn a_top_level_library_that_is_not_there_fails_naming_it() {
        let sandbox = Sandbox::new("no-library", None);
        sandbox
            .declare("[distribution]\ntop-level-library = \"Nope.lvlib\"")
            .put("UsesDep.vi", "UsesDep.vi");
        let why = check(&sandbox.env(), None).unwrap_err();
        assert!(why.to_string().contains("Nope.lvlib"), "{why}");
    }

    #[test]
    fn a_library_that_lists_a_vi_that_is_not_there_fails_naming_it() {
        let sandbox = Sandbox::new("lists-a-gap", None);
        sandbox
            .declare("[distribution]\ntop-level-library = \"Lib.lvlib\"")
            .library_of(&["Gone.vi"]);
        let why = format!("{:#}", check(&sandbox.env(), None).unwrap_err());
        assert!(why.contains("Gone.vi"), "{why}");
    }

    #[test]
    fn a_member_of_an_llb_belongs_to_the_package_that_owns_the_llb() {
        let mut search = search_in(Path::new("/nowhere"));
        search
            .owners
            .insert(key(Path::new("/p/x.llb")), "pkg".to_string());
        assert_eq!(search.owner(Path::new("/p/x.llb")), Some("pkg"));
        assert_eq!(search.owner(Path::new("/p/x.llb/m.vi")), Some("pkg"));
        assert_eq!(search.owner(Path::new("/p/y.vi")), None);
    }

    #[test]
    fn the_check_cannot_start_a_labview() {
        let source = include_str!("check.rs");
        for needle in [["vi", "server"].concat(), ["detect", "_labview"].concat()] {
            assert!(!source.contains(&needle), "check.rs names {needle}");
        }
    }
}
