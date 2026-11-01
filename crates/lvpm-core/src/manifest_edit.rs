//! Edits to one entry of `lvpm.toml`, leaving every other byte as it was.
//!
//! The manifest is user-owned (comments, ordering, blank lines), so an edit
//! goes through `toml_edit`, which keeps the formatting of what it does not
//! touch. [`set`] and [`remove`] are pure: text in, text out. [`set_file`] and
//! [`remove_file`] apply one to a file, and write only when the text changed.

use anyhow::{Context, Result};
use std::path::Path;
use toml_edit::{DocumentMut, InlineTable, Item, Table, Value};

/// Which dependency table an edit targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyTable {
    Dependencies,
    DevDependencies,
    /// `[dependency-groups.<name>]`.
    Group(String),
}

impl DependencyTable {
    /// The table's name in the manifest, as it appears between brackets.
    pub fn key(&self) -> String {
        self.path().join(".")
    }

    /// The keys that lead from the top of the manifest to the table.
    fn path(&self) -> Vec<&str> {
        match self {
            DependencyTable::Dependencies => vec!["dependencies"],
            DependencyTable::DevDependencies => vec!["dev-dependencies"],
            DependencyTable::Group(name) => vec!["dependency-groups", name],
        }
    }

    /// What a lock calls the table in a package's `groups`: nothing for
    /// `[dependencies]`, `dev` for `[dev-dependencies]`, else the group's name.
    pub fn group(&self) -> Option<&str> {
        match self {
            DependencyTable::Dependencies => None,
            DependencyTable::DevDependencies => Some("dev"),
            DependencyTable::Group(name) => Some(name),
        }
    }

    /// The table `--dev` and `--group <name>` ask for; `dev` wins over `group`.
    pub fn from_flags(dev: bool, group: Option<&str>) -> DependencyTable {
        match (dev, group) {
            (true, _) => DependencyTable::DevDependencies,
            (false, Some(name)) => DependencyTable::Group(name.to_string()),
            (false, None) => DependencyTable::Dependencies,
        }
    }

    /// The table as `--dev` and `--group` spell it: whether it is
    /// `[dev-dependencies]`, and the name of the `[dependency-groups]` table.
    pub fn flags(&self) -> (bool, Option<String>) {
        match self {
            DependencyTable::Dependencies => (false, None),
            DependencyTable::DevDependencies => (true, None),
            DependencyTable::Group(name) => (false, Some(name.clone())),
        }
    }
}

/// The item `table` is at in the manifest, when the manifest has it.
fn lookup<'a>(root: &'a Table, table: &DependencyTable) -> Option<&'a Item> {
    let path = table.path();
    let (first, rest) = path.split_first()?;
    rest.iter().try_fold(root.get(first)?, |item, step| {
        item.as_table_like()?.get(step)
    })
}

fn lookup_mut<'a>(root: &'a mut Table, table: &DependencyTable) -> Option<&'a mut Item> {
    let path = table.path();
    let (first, rest) = path.split_first()?;
    rest.iter().try_fold(root.get_mut(first)?, |item, step| {
        item.as_table_like_mut()?.get_mut(step)
    })
}

/// The manifest of a new project: `[project]` with its `name`, then `version`
/// and `labview = ">=<year>"` when given, and an empty `[dependencies]`. Built
/// through `toml_edit`, so a name with a quote or a backslash is still TOML.
pub fn skeleton(name: &str, version: Option<&str>, labview_year: Option<u32>) -> String {
    let mut project = Table::new();
    project.insert("name", Item::Value(name.into()));
    if let Some(version) = version {
        project.insert("version", Item::Value(version.into()));
    }
    if let Some(year) = labview_year {
        project.insert("labview", Item::Value(format!(">={year}").into()));
    }
    let mut doc = DocumentMut::new();
    doc.insert("project", Item::Table(project));
    doc.insert("dependencies", Item::Table(Table::new()));
    doc.to_string()
}

/// The name of the project in `root`: its folder's name, or `project` for a
/// path that ends in none (a drive root, or `..`).
pub fn folder_name(root: &Path) -> String {
    root.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".to_string())
}

/// `text` as the new file `path`. A file already there is left as it is and
/// the error is [`ErrorKind::AlreadyExists`](std::io::ErrorKind::AlreadyExists).
pub fn create_file(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(text.as_bytes()).inspect_err(|_| {
        // A partial file would make the next `lvpm init` refuse the folder.
        let _ = std::fs::remove_file(path);
    })
}

/// `text` with `name = "<constraint>"` in `table`: a new entry is added at the
/// end of the table (the table is appended to the file when it is missing), an
/// existing one changes its value only, keeping its same-line comment. An
/// entry that is a table, `{ github = "o/r", version = "1.0" }`, changes only
/// its `version`.
pub fn set(text: &str, table: &DependencyTable, name: &str, constraint: &str) -> Result<String> {
    let mut doc = parse(text)?;
    if lookup(&doc, table).is_none() {
        return match with_group_in_inline_groups(&mut doc, table, name, constraint)? {
            true => Ok(render(text, &doc)),
            false => Ok(with_appended_table(text, table, name, constraint)),
        };
    }
    let entries = lookup_mut(&mut doc, table)
        .and_then(Item::as_table_like_mut)
        .with_context(|| format!("[{}] is not a table", table.key()))?;
    let changed = match entries.get_mut(name) {
        Some(entry) => set_entry(entry, constraint),
        None => {
            entries.insert(name, Item::Value(constraint.into()));
            true
        }
    };
    if !changed {
        return Ok(text.to_string());
    }
    Ok(render(text, &doc))
}

/// Gives `entry` the constraint, whether it is a string or a table. False
/// when it already had it, and nothing was touched.
fn set_entry(entry: &mut Item, constraint: &str) -> bool {
    let Some(fields) = entry.as_table_like_mut() else {
        return set_string(entry, constraint);
    };
    match fields.get_mut("version") {
        Some(version) => set_string(version, constraint),
        None => {
            fields.insert("version", Item::Value(constraint.into()));
            true
        }
    }
}

/// Makes `item` the string `constraint`, keeping its decor (spaces and
/// comment). False when it already was that string, however quoted.
fn set_string(item: &mut Item, constraint: &str) -> bool {
    match item {
        Item::Value(old) if old.as_str() == Some(constraint) => false,
        Item::Value(old) => {
            let mut new = Value::from(constraint);
            *new.decor_mut() = old.decor().clone();
            *old = new;
            true
        }
        other => {
            *other = Item::Value(constraint.into());
            true
        }
    }
}

/// `text` without the `name` entry of `table`: its own line, with any
/// same-line comment, and nothing else. Comment lines and blank lines above it
/// stay. The table header stays too, even when that was its last entry. Text
/// without such an entry comes back as it was.
pub fn remove(text: &str, table: &DependencyTable, name: &str) -> Result<String> {
    let doc = toml_edit::Document::parse(text).map_err(not_valid)?;
    let Some(item) = lookup(&doc, table) else {
        return Ok(text.to_string());
    };
    let entries = item
        .as_table_like()
        .with_context(|| format!("[{}] is not a table", table.key()))?;
    let Some((entry_key, entry)) = entries.get_key_value(name) else {
        return Ok(text.to_string());
    };
    let line = entry_key
        .span()
        .zip(entry.span())
        .and_then(|(key_span, value_span)| own_line(text, key_span.start, value_span.end));
    match line {
        Some(line) => Ok(format!("{}{}", &text[..line.start], &text[line.end..])),
        None => remove_through_document(text, table, name),
    }
}

/// The whole lines holding an entry that starts at `start` and ends at `end`,
/// when it is alone on them, apart from a comment after it. `None` for an
/// entry sharing its line with something else, such as one in an inline table.
fn own_line(text: &str, start: usize, end: usize) -> Option<std::ops::Range<usize>> {
    let line_start = text[..start].rfind('\n').map_or(0, |at| at + 1);
    let line_end = text[end..].find('\n').map_or(text.len(), |at| end + at + 1);
    let before = &text[line_start..start];
    let after = text[end..line_end].trim_start_matches([' ', '\t']);
    let alone = before.chars().all(|c| c == ' ' || c == '\t')
        && (after.is_empty() || after.starts_with(['#', '\r', '\n']));
    alone.then_some(line_start..line_end)
}

/// The edit for an entry that is not on a line of its own: the document drops
/// it, comments attached to it included.
fn remove_through_document(text: &str, table: &DependencyTable, name: &str) -> Result<String> {
    let mut doc = parse(text)?;
    if let Some(entries) = lookup_mut(&mut doc, table).and_then(Item::as_table_like_mut) {
        entries.remove(name);
    }
    Ok(render(text, &doc))
}

/// [`set`] on the manifest at `path`. The file is written only if it changed.
pub fn set_file(path: &Path, table: &DependencyTable, name: &str, constraint: &str) -> Result<()> {
    edit_file(path, |text| set(text, table, name, constraint))
}

/// [`remove`] on the manifest at `path`. The file is written only if it changed.
pub fn remove_file(path: &Path, table: &DependencyTable, name: &str) -> Result<()> {
    edit_file(path, |text| remove(text, table, name))
}

fn edit_file(path: &Path, edit: impl FnOnce(&str) -> Result<String>) -> Result<()> {
    let before = std::fs::read_to_string(path)
        .with_context(|| format!("reading manifest {}", path.display()))?;
    let after = edit(&before).with_context(|| format!("editing manifest {}", path.display()))?;
    if after != before {
        std::fs::write(path, after)
            .with_context(|| format!("writing manifest {}", path.display()))?;
    }
    Ok(())
}

fn parse(text: &str) -> Result<DocumentMut> {
    text.parse::<DocumentMut>().map_err(not_valid)
}

/// The parse error names the line and column and quotes the line.
fn not_valid(why: toml_edit::TomlError) -> anyhow::Error {
    anyhow::anyhow!("lvpm.toml is not valid TOML, so it was not edited: {why}")
}

/// `text` followed by a new `[table]` holding the one entry. Built by
/// appending, not through the document, so that every byte of `text` stays
/// exactly where it was (toml_edit would move a trailing blank line below the
/// new table). The table is set off by a blank line unless one is there.
fn with_appended_table(
    text: &str,
    table: &DependencyTable,
    name: &str,
    constraint: &str,
) -> String {
    let mut entries = Table::new();
    entries.insert(name, Item::Value(constraint.into()));
    let path = table.path();
    // The tables above the entries' own are implicit: no header of theirs is written.
    let nested = path[1..]
        .iter()
        .rev()
        .fold(Item::Table(entries), |inner, step| {
            let mut outer = Table::new();
            outer.set_implicit(true);
            outer.insert(step, inner);
            Item::Table(outer)
        });
    let mut fragment = DocumentMut::new();
    fragment.insert(path[0], nested);

    let eol = line_ending(text);
    let mut out = text.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push_str(eol);
    }
    if !out.is_empty() && !out.ends_with("\n\n") && !out.ends_with("\r\n\r\n") {
        out.push_str(eol);
    }
    out.push_str(&fragment.to_string().replace('\n', eol));
    out
}

/// Puts a new group into `dependency-groups = { .. }` when the manifest writes
/// it as an inline table, which a `[dependency-groups.<name>]` header cannot
/// extend. False, and nothing touched, for any other manifest: a header is
/// appended there.
fn with_group_in_inline_groups(
    doc: &mut DocumentMut,
    table: &DependencyTable,
    name: &str,
    constraint: &str,
) -> Result<bool> {
    let DependencyTable::Group(group) = table else {
        return Ok(false);
    };
    let Some(groups) = doc
        .get_mut("dependency-groups")
        .filter(|item| !item.is_table())
    else {
        return Ok(false);
    };
    let groups = groups
        .as_table_like_mut()
        .context("[dependency-groups] is not a table")?;
    let mut entries = InlineTable::new();
    entries.insert(name, constraint.into());
    groups.insert(group, Item::Value(entries.into()));
    Ok(true)
}

/// The document as text. New lines use the file's own line ending: a file
/// whose every line ends in CRLF stays CRLF; anything else is left as it is.
fn render(original: &str, doc: &DocumentMut) -> String {
    let text = doc.to_string();
    match uses_crlf_throughout(original) {
        true => text.replace("\r\n", "\n").replace('\n', "\r\n"),
        false => text,
    }
}

fn uses_crlf_throughout(text: &str) -> bool {
    let crlf = text.matches("\r\n").count();
    crlf > 0 && crlf == text.matches('\n').count()
}

fn line_ending(text: &str) -> &'static str {
    match uses_crlf_throughout(text) {
        true => "\r\n",
        false => "\n",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use DependencyTable::{Dependencies, DevDependencies};

    const MANIFEST: &str = "\
# a header comment

[project]
name = \"fixture\"

[sources]
mirror = \"http://host/files\"
# the last word on sources

[dependencies]
alpha = \"1.0\"
beta = \">=1.0\"
";

    #[test]
    fn the_skeleton_without_a_version_is_what_venv_create_wrote_before_it_shared_it() {
        assert_eq!(
            skeleton("demo", None, Some(2026)),
            "[project]\nname = \"demo\"\nlabview = \">=2026\"\n\n[dependencies]\n"
        );
    }

    #[test]
    fn the_skeleton_puts_the_version_between_the_name_and_labview_and_leaves_out_what_is_not_given()
    {
        assert_eq!(
            skeleton("demo", Some("0.1.0"), Some(2023)),
            "[project]\nname = \"demo\"\nversion = \"0.1.0\"\nlabview = \">=2023\"\n\n[dependencies]\n"
        );
        assert_eq!(
            skeleton("demo", Some("0.1.0"), None),
            "[project]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[dependencies]\n"
        );
    }

    #[test]
    fn a_skeleton_name_with_a_quote_or_a_backslash_is_still_toml() {
        for name in ["a \"q\" name", r"back\slash", "tab\there"] {
            let text = skeleton(name, Some("0.1.0"), None);
            let parsed: toml::Table = text.parse().unwrap_or_else(|why| panic!("{text}: {why}"));
            assert_eq!(parsed["project"]["name"].as_str(), Some(name));
        }
    }

    #[test]
    fn a_skeleton_takes_a_dependency_like_any_manifest() {
        let after = set(
            &skeleton("demo", None, Some(2026)),
            &Dependencies,
            "alpha",
            ">=1.0",
        )
        .unwrap();
        assert_eq!(
            after,
            "[project]\nname = \"demo\"\nlabview = \">=2026\"\n\n[dependencies]\nalpha = \">=1.0\"\n"
        );
    }

    #[test]
    fn a_folders_name_is_the_projects_and_a_path_with_none_is_called_project() {
        assert_eq!(folder_name(Path::new("C:/Git/Serializer")), "Serializer");
        assert_eq!(folder_name(Path::new("Serializer")), "Serializer");
        assert_eq!(folder_name(Path::new("..")), "project");
    }

    #[test]
    fn a_file_is_created_once_and_a_second_create_leaves_it_as_it_is() {
        let dir = std::env::temp_dir().join(format!("lvpm-create-file-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("lvpm.toml");

        create_file(&path, "first\n").unwrap();
        let again = create_file(&path, "second\n").unwrap_err();

        assert_eq!(again.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_new_entry_is_one_added_line_at_the_end_of_its_table() {
        let after = set(MANIFEST, &Dependencies, "gamma", ">=1.0").unwrap();
        assert_eq!(after, format!("{MANIFEST}gamma = \">=1.0\"\n"));
    }

    #[test]
    fn a_new_entry_goes_before_the_tables_that_follow() {
        let text = "[dependencies]\nalpha = \"1.0\"\n\n[nipm.dependencies]\nx = \"1\"\n";
        let after = set(text, &Dependencies, "gamma", "*").unwrap();
        assert_eq!(
            after,
            "[dependencies]\nalpha = \"1.0\"\ngamma = \"*\"\n\n[nipm.dependencies]\nx = \"1\"\n"
        );
    }

    #[test]
    fn replacing_changes_the_value_and_keeps_the_comment_and_the_key_spelling() {
        let text = "[dependencies]\n\"alpha\" = '1.0'  # pinned for DQMH\n";
        let after = set(text, &Dependencies, "alpha", ">=2.0").unwrap();
        assert_eq!(
            after,
            "[dependencies]\n\"alpha\" = \">=2.0\"  # pinned for DQMH\n"
        );
    }

    #[test]
    fn removing_takes_the_line_and_keeps_the_header_of_an_emptied_table() {
        let after = remove(MANIFEST, &Dependencies, "alpha").unwrap();
        assert_eq!(after, MANIFEST.replace("alpha = \"1.0\"\n", ""));
        let text = "[dependencies]\nalpha = \"1.0\"\n";
        assert_eq!(
            remove(text, &Dependencies, "alpha").unwrap(),
            "[dependencies]\n"
        );
    }

    const COMMENTED: &str = "[dependencies]\n# the first\nalpha = \"1.0\"  # same line\n\n# the middle\nbeta = \">=1.0\"\n# the last\ngamma = \"*\"\n";

    #[test]
    fn removing_a_first_entry_keeps_the_comment_lines_above_it() {
        assert_eq!(
            remove(COMMENTED, &Dependencies, "alpha").unwrap(),
            "[dependencies]\n# the first\n\n# the middle\nbeta = \">=1.0\"\n# the last\ngamma = \"*\"\n"
        );
    }

    #[test]
    fn removing_a_middle_entry_keeps_the_comment_lines_above_it() {
        assert_eq!(
            remove(COMMENTED, &Dependencies, "beta").unwrap(),
            "[dependencies]\n# the first\nalpha = \"1.0\"  # same line\n\n# the middle\n# the last\ngamma = \"*\"\n"
        );
    }

    #[test]
    fn removing_a_last_entry_keeps_the_comment_lines_above_it() {
        assert_eq!(
            remove(COMMENTED, &Dependencies, "gamma").unwrap(),
            "[dependencies]\n# the first\nalpha = \"1.0\"  # same line\n\n# the middle\nbeta = \">=1.0\"\n# the last\n"
        );
    }

    #[test]
    fn removing_an_entry_keeps_comment_lines_in_a_crlf_file() {
        let crlf = COMMENTED.replace('\n', "\r\n");
        assert_eq!(
            remove(&crlf, &Dependencies, "beta").unwrap(),
            "[dependencies]\n# the first\nalpha = \"1.0\"  # same line\n\n# the middle\n# the last\ngamma = \"*\"\n"
                .replace('\n', "\r\n")
        );
        assert_eq!(
            remove(&crlf, &Dependencies, "gamma").unwrap(),
            COMMENTED
                .replace("gamma = \"*\"\n", "")
                .replace('\n', "\r\n")
        );
    }

    #[test]
    fn removing_the_last_entry_without_a_final_newline_works() {
        assert_eq!(
            remove(
                "[dependencies]\n# c\nalpha = \"1.0\"",
                &Dependencies,
                "alpha"
            )
            .unwrap(),
            "[dependencies]\n# c\n"
        );
    }

    #[test]
    fn an_entry_in_an_inline_table_is_removed_by_the_document() {
        assert_eq!(
            remove(
                "dependencies = { alpha = \"1.0\", beta = \"*\" }\n",
                &Dependencies,
                "alpha"
            )
            .unwrap(),
            "dependencies = { beta = \"*\" }\n"
        );
    }

    #[test]
    fn removing_what_is_not_there_changes_nothing() {
        assert_eq!(remove(MANIFEST, &Dependencies, "nope").unwrap(), MANIFEST);
        assert_eq!(
            remove(MANIFEST, &DevDependencies, "alpha").unwrap(),
            MANIFEST
        );
    }

    #[test]
    fn a_missing_table_is_appended_after_a_blank_line() {
        let after = set(MANIFEST, &DevDependencies, "caraya", ">=1.0").unwrap();
        assert_eq!(
            after,
            format!("{MANIFEST}\n[dev-dependencies]\ncaraya = \">=1.0\"\n")
        );
        let blank = format!("{MANIFEST}\n");
        assert_eq!(
            set(&blank, &DevDependencies, "caraya", "*").unwrap(),
            format!("{blank}[dev-dependencies]\ncaraya = \"*\"\n")
        );
        assert_eq!(
            set("", &Dependencies, "a", "*").unwrap(),
            "[dependencies]\na = \"*\"\n"
        );
    }

    #[test]
    fn a_file_without_a_final_newline_gets_one_before_the_new_table() {
        let after = set("[project]\nname = \"x\"", &DevDependencies, "c", "*").unwrap();
        assert_eq!(
            after,
            "[project]\nname = \"x\"\n\n[dev-dependencies]\nc = \"*\"\n"
        );
    }

    #[test]
    fn an_equal_value_comes_back_untouched_whatever_its_quoting() {
        let text = "[dependencies]\nalpha = '1.0'   # x\n";
        assert_eq!(set(text, &Dependencies, "alpha", "1.0").unwrap(), text);
    }

    #[test]
    fn a_table_line_changes_only_its_version_and_keeps_github_and_the_comment() {
        let text = "[dependencies]\nalpha = { github = \"o/r\", version = \"1.0\" }  # pinned\nbeta = \">=1.0\"\n";
        assert_eq!(
            set(text, &Dependencies, "alpha", ">=2.0").unwrap(),
            "[dependencies]\nalpha = { github = \"o/r\", version = \">=2.0\" }  # pinned\nbeta = \">=1.0\"\n"
        );
        let reversed = "[dependencies]\nalpha = { version = '1.0',   github = \"o/r\" }\n";
        assert_eq!(
            set(reversed, &Dependencies, "alpha", "*").unwrap(),
            "[dependencies]\nalpha = { version = \"*\",   github = \"o/r\" }\n"
        );
    }

    #[test]
    fn a_table_line_with_the_same_version_comes_back_byte_identical() {
        let text = "[dev-dependencies]\nalpha = {github='o/r',  version = '1.0' }   # x\n";
        assert_eq!(set(text, &DevDependencies, "alpha", "1.0").unwrap(), text);
    }

    #[test]
    fn a_table_without_a_version_gets_one_and_keeps_its_other_keys() {
        let after = set(
            "[dependencies]
alpha = { github = \"o/r\" }
",
            &Dependencies,
            "alpha",
            "*",
        )
        .unwrap();
        let parsed: toml::Table = after.parse().unwrap();
        assert_eq!(
            parsed["dependencies"]["alpha"],
            toml::Value::Table(
                "github = \"o/r\"
version = \"*\"
"
                .parse::<toml::Table>()
                .unwrap()
            ),
            "{after}"
        );
    }

    #[test]
    fn a_table_written_as_its_own_section_changes_only_its_version() {
        let text = "[dependencies.alpha]\ngithub = \"o/r\"  # the repo\nversion = \"1.0\"\n";
        assert_eq!(
            set(text, &Dependencies, "alpha", ">=2.0").unwrap(),
            "[dependencies.alpha]\ngithub = \"o/r\"  # the repo\nversion = \">=2.0\"\n"
        );
    }

    #[test]
    fn removing_a_table_entry_drops_the_whole_entry_and_keeps_the_rest() {
        let inline = "[dependencies]
# the first
alpha = { github = \"o/r\", version = \"1.0\" }  # pinned
beta = \">=1.0\"
";
        assert_eq!(
            remove(inline, &Dependencies, "alpha").unwrap(),
            "[dependencies]
# the first
beta = \">=1.0\"
"
        );
        let section = "[dependencies]
beta = \">=1.0\"

# the first
[dependencies.alpha]
github = \"o/r\"
version = \"1.0\"
";
        let after = remove(section, &Dependencies, "alpha").unwrap();
        let parsed: toml::Table = after.parse().unwrap();
        assert_eq!(
            parsed["dependencies"].as_table().unwrap().len(),
            1,
            "{after}"
        );
        assert!(after.contains("beta = \">=1.0\""), "{after}");
    }

    #[test]
    fn crlf_files_stay_crlf_for_every_edit() {
        let crlf = MANIFEST.replace('\n', "\r\n");
        for after in [
            set(&crlf, &Dependencies, "gamma", "*").unwrap(),
            set(&crlf, &DevDependencies, "caraya", "*").unwrap(),
            remove(&crlf, &Dependencies, "alpha").unwrap(),
        ] {
            assert_eq!(after.matches('\n').count(), after.matches("\r\n").count());
        }
        assert_eq!(
            set(&crlf, &Dependencies, "gamma", "*").unwrap(),
            format!("{crlf}gamma = \"*\"\r\n")
        );
    }

    #[test]
    fn invalid_toml_is_refused_naming_the_line() {
        let text = "[project]\nname = \"x\"\n\n[dependencies]\nalpha 1.0\n";
        let why = format!("{:#}", set(text, &Dependencies, "gamma", "*").unwrap_err());
        assert!(why.contains("line 5"), "{why}");
        assert!(why.contains("alpha 1.0"), "{why}");
        assert!(remove(text, &Dependencies, "alpha").is_err());
    }

    #[test]
    fn a_dependencies_value_that_is_not_a_table_is_an_error() {
        assert!(set("dependencies = 3\n", &Dependencies, "a", "*").is_err());
        assert!(remove("dependencies = 3\n", &Dependencies, "a").is_err());
    }

    fn ci() -> DependencyTable {
        DependencyTable::Group("ci".to_string())
    }

    #[test]
    fn a_tables_key_is_its_header_and_a_groups_is_nested_under_dependency_groups() {
        assert_eq!(Dependencies.key(), "dependencies");
        assert_eq!(DevDependencies.key(), "dev-dependencies");
        assert_eq!(ci().key(), "dependency-groups.ci");
    }

    #[test]
    fn a_table_and_its_flags_are_each_others_inverse_and_dev_wins_over_a_group() {
        for table in [Dependencies, DevDependencies, ci()] {
            let (dev, group) = table.flags();
            assert_eq!(DependencyTable::from_flags(dev, group.as_deref()), table);
        }
        assert_eq!(
            DependencyTable::from_flags(true, Some("ci")),
            DevDependencies
        );
    }

    #[test]
    fn a_missing_group_table_is_appended_with_its_nested_header() {
        let after = set(MANIFEST, &ci(), "ci_tools", ">=2.0").unwrap();
        assert_eq!(
            after,
            format!(
                "{MANIFEST}
[dependency-groups.ci]
ci_tools = \">=2.0\"
"
            )
        );
    }

    #[test]
    fn a_new_group_goes_beside_the_groups_that_are_there() {
        let text = "[dependency-groups.docs]
sphinx = \"*\"
";
        assert_eq!(
            set(text, &ci(), "ci_tools", "*").unwrap(),
            "[dependency-groups.docs]
sphinx = \"*\"

[dependency-groups.ci]
ci_tools = \"*\"
"
        );
    }

    #[test]
    fn an_entry_set_in_an_existing_group_changes_that_group_and_keeps_every_other_byte() {
        let text = "[dependency-groups.docs]
sphinx = \"*\"

[dependency-groups.ci]
# tools
ci_tools = \"1.0\"  # pinned

[dependencies]
alpha = \"1.0\"
";
        assert_eq!(
            set(text, &ci(), "ci_tools", ">=2.0").unwrap(),
            text.replace("ci_tools = \"1.0\"", "ci_tools = \">=2.0\"")
        );
        assert_eq!(
            set(text, &ci(), "other", "*").unwrap(),
            text.replace(
                "# pinned
",
                "# pinned
other = \"*\"
"
            )
        );
    }

    #[test]
    fn removing_from_a_group_takes_its_line_and_leaves_the_other_tables_alone() {
        let text = "[dependencies]
ci_tools = \"*\"

[dependency-groups.ci]
ci_tools = \"*\"
beta = \"1.0\"
";
        assert_eq!(
            remove(text, &ci(), "ci_tools").unwrap(),
            "[dependencies]
ci_tools = \"*\"

[dependency-groups.ci]
beta = \"1.0\"
"
        );
    }

    #[test]
    fn removing_from_a_group_that_is_not_there_changes_nothing() {
        assert_eq!(remove(MANIFEST, &ci(), "alpha").unwrap(), MANIFEST);
        let other = "[dependency-groups.docs]
alpha = \"*\"
";
        assert_eq!(remove(other, &ci(), "alpha").unwrap(), other);
    }

    #[test]
    fn an_entry_in_an_inline_group_is_removed_by_the_document() {
        assert_eq!(
            remove(
                "[dependency-groups]
ci = { a = \"1.0\", b = \"*\" }
",
                &ci(),
                "a"
            )
            .unwrap(),
            "[dependency-groups]
ci = { b = \"*\" }
"
        );
    }

    #[test]
    fn a_new_group_goes_into_an_inline_dependency_groups_table_and_keeps_the_others() {
        let text = "dependency-groups = { docs = { sphinx = \"*\" } }

[project]
name = \"x\"
";
        let after = set(text, &ci(), "ci_tools", ">=2.0").unwrap();
        let parsed: toml::Table = after.parse().unwrap_or_else(|why| panic!("{after}: {why}"));
        assert_eq!(
            parsed["dependency-groups"]["docs"]["sphinx"].as_str(),
            Some("*")
        );
        assert_eq!(
            parsed["dependency-groups"]["ci"]["ci_tools"].as_str(),
            Some(">=2.0")
        );
    }

    #[test]
    fn a_dependency_groups_value_that_is_not_a_table_is_an_error_when_a_group_is_added() {
        assert!(set("dependency-groups = 3\n", &ci(), "a", "*").is_err());
    }

    #[test]
    fn a_group_that_is_not_a_table_is_an_error() {
        let text = "[dependency-groups]
ci = 3
";
        assert!(set(text, &ci(), "a", "*").is_err());
        assert!(remove(text, &ci(), "a").is_err());
    }

    #[test]
    fn files_are_written_only_when_the_text_changed() {
        let dir = std::env::temp_dir().join(format!("lvpm-manifest-edit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("lvpm.toml");
        std::fs::write(&path, MANIFEST).unwrap();
        let long_ago =
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(long_ago)
            .unwrap();

        set_file(&path, &Dependencies, "alpha", "1.0").unwrap();
        remove_file(&path, &Dependencies, "nope").unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            long_ago
        );

        set_file(&path, &Dependencies, "alpha", "2.0").unwrap();
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("alpha = \"2.0\"")
        );
        assert_ne!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            long_ago
        );

        remove_file(&path, &Dependencies, "alpha").unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("alpha"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
