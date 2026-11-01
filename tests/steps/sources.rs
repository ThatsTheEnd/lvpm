//! Steps of the first-source-wins scenarios: two `[sources]` folders on the
//! loopback feed, `first` and `second`, each listing what the scenario says.
//! The `.lvpkg` scenarios (S7a) are at the end: a package `lvpm build` wrote,
//! installed from a package folder, the cache or the loopback feed.

use super::lock::sha256_hex;
use super::staleness::{Snapshot, snapshot};
use crate::CliWorld;
use crate::support::feed::{PROJECT_FEED_DIR, archive_path, listing, md5_hex};
use crate::support::fixtures::{
    ALPHA_3_NEEDING_LABVIEW_2026, FixturePackage, LOCK_PACKAGES, Payload,
};
use crate::support::project::MANIFEST_FILE;
use crate::support::sandbox::{DEST_DIR, LOCAL_PACKAGES_DIR};
use cucumber::{given, then, when};
use lvpm_core::lockfile::LOCAL_SOURCE;
use lvpm_core::manifest_edit::DependencyTable;
use lvpm_core::released;
use regex::{Captures, Regex};
use std::io::{Cursor, Read, Write};
use std::path::PathBuf;
use std::sync::LazyLock;
use zip::write::SimpleFileOptions;

const FIRST: &str = "/first/";
const SECOND: &str = "/second/";

fn dir(source: &str) -> &'static str {
    match source {
        "first" => FIRST,
        "second" => SECOND,
        other => panic!("no source {other:?}"),
    }
}

fn fixture(name: &str, version: &str) -> &'static FixturePackage {
    LOCK_PACKAGES
        .iter()
        .chain([&ALPHA_3_NEEDING_LABVIEW_2026])
        .find(|package| package.name == name && package.version == version)
        .unwrap_or_else(|| panic!("no fixture {name} {version}"))
}

impl CliWorld {
    /// The `[sources]` URL of `source`, as `lvpm.toml` holds it.
    fn source_url(&self, source: &str) -> String {
        self.sandbox.feed().source_url(dir(source))
    }

    fn rewrite_manifest(&self, edit: impl FnOnce(String) -> String) {
        let path = self.sandbox.work_dir().join(MANIFEST_FILE);
        std::fs::write(&path, edit(self.manifest_text())).expect("write lvpm.toml");
    }
}

#[given(
    regex = r#"^a project with labview "(\d{4})" whose \[sources\] are "first" then "second"$"#
)]
async fn two_sources(world: &mut CliWorld, year: String) {
    world.sandbox.clear_local_packages();
    world.sandbox.use_project_sources(&year, &[FIRST, SECOND]);
    // Each lists nothing until a step says otherwise.
    for dir in [FIRST, SECOND] {
        world.sandbox.feed().publish(dir, &listing(&[], true));
    }
}

// ---- what a feed lists, and when each version was released ----

const SECS_PER_DAY: i64 = 86_400;
/// The oldest LabVIEW the fixtures run on, as the feeds write it.
const ANY_LABVIEW: &str = "20.0";
const SIGNED_OFFSET: &str = "a signed offset";
const SIGN_LESS_OFFSET: &str = "a sign-less offset";
/// Where a feed's clock is, east of UTC, when it writes `0530` without a sign.
const SIGN_LESS_EAST_SECS: i64 = 5 * 3600 + 30 * 60;
/// The zone of the signed shape, west of UTC.
const SIGNED_WEST_SECS: i64 = 5 * 3600;
const WEEKDAYS_FROM_THURSDAY: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// One version a feed lists, with the `Package.Release Date` it writes.
#[derive(Debug, Clone)]
struct Listed {
    package: &'static FixturePackage,
    /// The date as the feed spells it; `None` for an entry with no date line.
    date: Option<String>,
    /// The instant that date is, as the fixture wrote it; `None` when the
    /// date is not one lvpm can read.
    released: Option<i64>,
}

/// What the feeds of a scenario list.
#[derive(Debug, Default)]
pub struct ListingState {
    by_source: std::collections::HashMap<String, Vec<Listed>>,
}

impl ListingState {
    /// When the scenario released `name` `version`, if it dated it. Of a
    /// version both sources list it is the first source's: lvpm never asks
    /// the second one (ADR 4).
    fn released(&self, name: &str, version: &str) -> i64 {
        ["first", "second"]
            .iter()
            .filter_map(|source| self.by_source.get(*source))
            .flatten()
            .find(|listed| listed.package.name == name && listed.package.version == version)
            .and_then(|listed| listed.released)
            .unwrap_or_else(|| panic!("the scenario gave {name} {version} no release date"))
    }
}

/// `"alpha" 1.1 for LabVIEW 2026 released 3 days ago written as GMT requiring
/// "beta>=2.0"`: a name and version, then what else the step says of it.
static LISTED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r#""(?P<name>[^"]+)" (?P<version>\d[\w.\-]*)"#,
        r#"(?: for LabVIEW (?P<year>\d{4}))?"#,
        r#"(?: released (?P<days>\d+) days ago(?: written as (?P<shape>a signed offset|GMT|a sign-less offset))?"#,
        r#"| (?P<undated>with no release date)"#,
        r#"| with release date "(?P<raw>[^"]*)")?"#,
        r#"(?: requiring "(?P<requires>[^"]+)")?"#,
    ))
    .expect("the pattern")
});

fn leaked(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

/// `time` as a feed writes a date in `shape` (RFC 2822), e.g.
/// `Thu, 19 May 2016 15:46:24 -0500`, meaning the same instant.
fn feed_date(time: i64, shape: &str) -> String {
    let (shift, zone) = match shape {
        "GMT" => (0, "GMT"),
        SIGN_LESS_OFFSET => (SIGN_LESS_EAST_SECS, "0530"),
        SIGNED_OFFSET => (-SIGNED_WEST_SECS, "-0500"),
        other => panic!("no date shape {other:?}"),
    };
    let local = time + shift;
    let (days, seconds) = (
        local.div_euclid(SECS_PER_DAY),
        local.rem_euclid(SECS_PER_DAY),
    );
    let day = released::day(local);
    let mut parts = day
        .split('-')
        .map(|part| part.parse::<usize>().expect("a number"));
    let (year, month, date) = (
        parts.next().expect("a year"),
        parts.next().expect("a month"),
        parts.next().expect("a day"),
    );
    format!(
        "{}, {date:02} {} {year:04} {:02}:{:02}:{:02} {zone}",
        WEEKDAYS_FROM_THURSDAY[days.rem_euclid(7) as usize],
        MONTHS[month - 1],
        seconds / 3600,
        seconds % 3600 / 60,
        seconds % 60
    )
}

/// What a listed version requires: its `requiring "name>=floor"`, if any.
fn requires_of(item: &Captures) -> &'static [(&'static str, &'static str)] {
    let Some(clause) = item.name("requires") else {
        return &[];
    };
    let (needs, floor) = clause.as_str().split_once(">=").expect("`name>=floor`");
    Box::leak(Box::new([(leaked(needs.into()), leaked(floor.into()))]))
}

/// The LabVIEW a listed version needs, as the feed writes it.
fn lv_min_of(item: &Captures) -> &'static str {
    item.name("year").map_or(ANY_LABVIEW, |year| {
        leaked(format!(
            "{}.0",
            year.as_str().parse::<u32>().expect("a year") - 2000
        ))
    })
}

/// The fixture of a version the step says no more of, else a package of
/// what it does say.
fn package_of(item: &Captures) -> &'static FixturePackage {
    let (name, version) = (&item["name"], &item["version"]);
    if item.iter().skip(3).all(|group| group.is_none()) {
        return fixture(name, version);
    }
    Box::leak(Box::new(FixturePackage {
        name: leaked(name.to_string()),
        version: leaked(version.to_string()),
        display_name: leaked(name.to_string()),
        requires: requires_of(item),
        lv_min: lv_min_of(item),
        payload: Payload::Text,
        post_install_hook: None,
    }))
}

/// The versions `text` lists, as `LISTED` reads them. Whatever it does not
/// read is an error in the scenario, not something to skip.
fn listed_in(text: &str) -> Vec<Listed> {
    let leftover = LISTED.replace_all(text, "");
    assert!(
        leftover
            .split([',', ' '])
            .all(|word| word.is_empty() || word == "and"),
        "{text:?} says more than a list of versions: {leftover:?} is left"
    );
    let now = released::now();
    LISTED
        .captures_iter(text)
        .map(|item| {
            let package = package_of(&item);
            let released = item.name("days").map(|days| {
                now - days.as_str().parse::<i64>().expect("a number of days") * SECS_PER_DAY
            });
            let shape = item
                .name("shape")
                .map_or(SIGNED_OFFSET, |shape| shape.as_str());
            let date = match (released, item.name("raw")) {
                (Some(time), _) => Some(feed_date(time, shape)),
                (None, Some(raw)) => Some(raw.as_str().to_string()),
                (None, None) => None,
            };
            Listed {
                package,
                date,
                released,
            }
        })
        .collect()
}

impl CliWorld {
    /// `source` lists what `more` says besides what it listed before; a
    /// version listed again takes its new date. The listing is revised, as a
    /// feed that changed is, so a client that cached the old one fetches it.
    fn list(&mut self, source: &str, more: Vec<Listed>) {
        let listed = self
            .listings
            .by_source
            .entry(source.to_string())
            .or_default();
        for item in more {
            listed.retain(|known| {
                (known.package.name, known.package.version)
                    != (item.package.name, item.package.version)
            });
            listed.push(item);
        }
        let body: String = listed
            .iter()
            .map(|item| {
                let mut entry = listing(std::slice::from_ref(item.package), true);
                if let Some(date) = &item.date {
                    entry.push_str(&format!("Package.Release Date={date}\n"));
                }
                entry
            })
            .collect();
        let feed = self.sandbox.feed();
        feed.revise(dir(source), &body);
        for item in &*listed {
            feed.serve(
                &archive_path(dir(source), &item.package.file_name()),
                item.package.build_vip(),
            );
        }
    }
}

#[given(regex = r#"^"(first|second)" (?:also )?lists (.+)$"#)]
#[when(regex = r#"^"(first|second)" (?:also )?lists (.+)$"#)]
async fn source_lists(world: &mut CliWorld, source: String, packages: String) {
    world.list(&source, listed_in(&packages));
}

#[when(regex = r#"^"(first|second)" now lists "([^"]+)" (\S+) as released (\d+) days ago$"#)]
async fn source_lists_again(
    world: &mut CliWorld,
    source: String,
    name: String,
    version: String,
    days: u32,
) {
    let again = listed_in(&format!("\"{name}\" {version} released {days} days ago"));
    // Only the date changes: what the package requires stays as it was.
    let before = world.listings.by_source[&source]
        .iter()
        .find(|listed| listed.package.name == name && listed.package.version == version)
        .unwrap_or_else(|| panic!("{source} did not list {name} {version}"));
    let item = Listed {
        package: before.package,
        ..again.into_iter().next().expect("one version")
    };
    world.list(&source, vec![item]);
}

#[given(regex = r#"^the project dev-depends on "([^"]+)" "([^"]*)"$"#)]
async fn dev_depends_on(world: &mut CliWorld, name: String, constraint: String) {
    world.depend(DependencyTable::DevDependencies, &name, &constraint);
}

#[given(regex = r#"^the project depends on "([^"]+)" "([^"]*)" and "([^"]+)" "([^"]*)"$"#)]
async fn depends_on_two(
    world: &mut CliWorld,
    name: String,
    constraint: String,
    other: String,
    other_constraint: String,
) {
    world.depend(DependencyTable::Dependencies, &name, &constraint);
    world.depend(DependencyTable::Dependencies, &other, &other_constraint);
}

#[given(regex = r#"^I have run "(lvpm [^"]+)"$"#)]
async fn have_run(world: &mut CliWorld, command_line: String) {
    let args: Vec<&str> = command_line.split_whitespace().skip(1).collect();
    world.sandbox.setup_lvpm(&args);
}

#[given(regex = r#"^"(first|second)" cannot be reached$"#)]
async fn cannot_be_reached(world: &mut CliWorld, source: String) {
    // Every request goes through the loopback feed as a proxy, so no host or
    // port is dead; a folder the feed never serves fails the same way.
    let url = world.source_url(&source);
    let unreachable = format!("{url}/gone");
    world.rewrite_manifest(|text| text.replace(&url, &unreachable));
}

#[given(regex = r#"^the local folder holds "([^"]+)" (\S+)$"#)]
async fn local_folder_holds(world: &mut CliWorld, name: String, version: String) {
    world.sandbox.put_local_package(fixture(&name, &version));
}

#[when(r#"I swap "first" and "second" in [sources]"#)]
async fn swap_sources(world: &mut CliWorld) {
    let (first, second) = (world.source_url("first"), world.source_url("second"));
    world.rewrite_manifest(|text| {
        text.replace(&first, "\u{0}")
            .replace(&second, &first)
            .replace('\u{0}', &second)
    });
}

#[then(regex = r#"^lvpm\.lock takes "([^"]+)" (\S+) from "(first|second|local)"$"#)]
async fn lock_holds_from(world: &mut CliWorld, name: String, version: String, source: String) {
    let package = world.locked_package(&name);
    assert_eq!(package["version"].as_str(), Some(version.as_str()));
    let expected = match source.as_str() {
        "local" => LOCAL_SOURCE.to_string(),
        _ => format!("{}/index.vipr", world.source_url(&source)),
    };
    assert_eq!(package["source"].as_str(), Some(expected.as_str()));
}

#[then(regex = r#"^the command fails naming "(first|second)"$"#)]
async fn fails_naming(world: &mut CliWorld, source: String) {
    world.assert_failed();
    let printed = world.printed();
    // Outside the progress text, which names every source read; an error can
    // end a progress line.
    let named = printed
        .lines()
        .map(|line| match line.find("Error:") {
            Some(at) => &line[at..],
            None if ["downloading feed", "checking feed"]
                .iter()
                .any(|progress| line.trim_start().starts_with(progress)) =>
            {
                ""
            }
            None => line,
        })
        .any(|line| line.contains(dir(&source)));
    assert!(named, "{source:?} is not named in:\n{printed}");
}

/// The error of a package none of whose wanted versions is old enough:
/// `listing` names the versions held back, newest first, and the dates in it
/// come from the instants the fixture wrote, shown the way lvpm shows them.
#[then(
    regex = r#"^the command fails with the min-age-days error for "([^"]+)" "([^"]*)" listing (.+)$"#
)]
async fn fails_with_the_min_age_days_error(
    world: &mut CliWorld,
    name: String,
    constraint: String,
    listing: String,
) {
    world.assert_failed();
    let days = world.min_age_days();
    let mut expected =
        format!("{name} {constraint}: every version is held back by min-age-days = {days}");
    let mut eligible = Vec::new();
    for version in VERSION.find_iter(&listing) {
        let at = world.listings.released(&name, version.as_str());
        let due = released::eligible(at, days);
        eligible.push(due);
        expected.push_str(&format!(
            "\n  {} released {}, eligible {}",
            version.as_str(),
            released::day(at),
            released::day(due)
        ));
    }
    let earliest = eligible.into_iter().min().expect("a version is listed");
    expected.push_str(&format!(
        "\nnext: wait until {}, lower min-age-days, or pin an exact version of {name} in lvpm.toml",
        released::day(earliest)
    ));
    let printed = world.printed();
    assert!(
        printed.contains(&expected),
        "{expected}\nis not in:\n{printed}"
    );
}

static VERSION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d[\w.\-]*").expect("the pattern"));

#[then(regex = r"^the error does not mention version (\S+)$")]
async fn error_does_not_mention_version(world: &mut CliWorld, version: String) {
    // A version ends at a character that is no part of one, or at a full stop
    // that ends the sentence: `1.0.` mentions 1.0, `1.0.1` does not.
    let mention = format!(r"(^|[^\d.]){}($|[^\d.]|\.(\s|$))", regex::escape(&version));
    let printed = world.printed();
    assert!(
        !Regex::new(&mention)
            .expect("the pattern")
            .is_match(&printed),
        "version {version} is in:\n{printed}"
    );
}

#[then(regex = r#"^the error says "([^"]+)"$"#)]
async fn error_says(world: &mut CliWorld, text: String) {
    let printed = world.printed();
    assert!(printed.contains(&text), "{text:?} is not in:\n{printed}");
}

impl CliWorld {
    /// The `[sources] min-age-days` of the project's manifest.
    fn min_age_days(&self) -> u32 {
        let text = self.manifest_text();
        let manifest: toml::Table = text.parse().expect("lvpm.toml is TOML");
        let days = manifest["sources"]["min-age-days"]
            .as_integer()
            .unwrap_or_else(|| panic!("no min-age-days in:\n{text}"));
        u32::try_from(days).expect("a whole number of days")
    }
}

#[then(regex = r#"^lvpm\.toml depends on "([^"]+)" "([^"]*)"$"#)]
async fn manifest_depends_on(world: &mut CliWorld, name: String, constraint: String) {
    let text = world.manifest_text();
    let manifest: toml::Table = text.parse().expect("lvpm.toml is TOML");
    let wanted = manifest["dependencies"][name.as_str()].as_str();
    assert_eq!(wanted, Some(constraint.as_str()), "{text}");
}

#[then(regex = r"^the output shows version (\S+) and not (\S+)$")]
async fn output_shows_version(world: &mut CliWorld, shown: String, hidden: String) {
    let version = |v: &str| Regex::new(&format!(r"(^|[^\d.]){}($|[^\d.])", regex::escape(v)));
    let printed = world.output().stdout.clone();
    assert!(version(&shown).unwrap().is_match(&printed), "{printed}");
    assert!(!version(&hidden).unwrap().is_match(&printed), "{printed}");
}

// ---- .lvpkg from every feed (S7a) ----

/// Where a `.lvpkg` of the Background's library puts its members.
const INSTALL_DIR: &str = "vi.lib/Vendor/Demo Lib";
/// The consumer takes any version of a package.
const ANY_VERSION: &str = ">=1.0";
/// The consumer project's oldest LabVIEW, newer than the package's `>=2016`.
const CONSUMER_LABVIEW: &str = "2024";
const DEMO_LIB: &str = "demo_lib-1.2.3.lvpkg";
/// A scratch prefix keeps a package's files below this folder.
const PREFIX_APPLICATION: &str = "LabVIEW";

/// What a `.lvpkg` scenario has set up and seen.
#[derive(Debug, Default)]
pub struct LvpkgState {
    /// The package whose entry was changed after build, and the entry.
    tampered: Option<(PathBuf, String)>,
    /// The cache as it was before `lvpm cache add` ran.
    cache: Option<Snapshot>,
}

/// A `.vip` of the fixture builder that no `.lvpkg` here shares a name with.
const OTHER_LIB_VIP: FixturePackage = FixturePackage {
    name: "other_lib",
    version: "1.0.0.1",
    display_name: "Other Lib",
    requires: &[],
    lv_min: "20.0",
    payload: Payload::Text,
    post_install_hook: None,
};

/// `archive` with the bytes of `entry` changed, as a download that was altered
/// on the way is: every other entry, and `RECORD` with them, are as they were.
fn with_entry_changed(archive: &[u8], entry: &str) -> Vec<u8> {
    let mut zip = zip::ZipArchive::new(Cursor::new(archive)).expect("a zip");
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut changed = false;
    for i in 0..zip.len() {
        let mut member = zip.by_index(i).expect("an entry");
        let mut bytes = Vec::new();
        member.read_to_end(&mut bytes).expect("read an entry");
        let name = member.name().to_string();
        if name == entry {
            bytes = b"changed after build".to_vec();
            changed = true;
        }
        let options = SimpleFileOptions::default()
            .compression_method(member.compression())
            .last_modified_time(zip::DateTime::default());
        out.start_file(name, options).expect("start an entry");
        out.write_all(&bytes).expect("write an entry");
    }
    assert!(changed, "the package has no entry {entry:?}");
    let tampered = out.finish().expect("finish the zip").into_inner();
    assert_ne!(tampered, archive);
    tampered
}

/// What the cache holds, but for its database, which `cache add` creates empty
/// before it reads the first archive: the stored archives are what a refused
/// package must not add.
fn cache_files(cache: &std::path::Path) -> Snapshot {
    let mut files = snapshot(cache);
    files.retain(|path, _| path.file_name() != Some("lvpm-cache.db".as_ref()));
    files
}

/// What lvpm says of a package whose `entry` is not what `RECORD` lists, after
/// the file's name and a colon.
fn entry_changed(entry: &str) -> String {
    format!(
        "entry \"{entry}\" differs from RECORD (the package was altered or is corrupt); rebuild or download it again"
    )
}

impl CliWorld {
    /// The package `lvpm build` writes for the Background's library, under
    /// this name, version and display name, as setup that must succeed.
    fn built_as(&mut self, name: &str, version: &str, display_name: &str) -> Vec<u8> {
        self.set(&["project"], "name", name);
        self.set(&["project"], "version", version);
        self.set(&["distribution"], "display-name", display_name);
        self.lvpm_build();
        self.succeeded();
        let file = format!("dist/{name}-{version}.lvpkg");
        std::fs::read(self.work(&file)).expect("the built package")
    }

    fn built_demo_lib(&mut self) -> Vec<u8> {
        self.built_as("demo_lib", "1.2.3", "Demo Lib")
    }

    /// The built package, under `file`, in the package folder of a project that
    /// depends on `demo_lib`.
    fn project_with_demo_lib_in_folder_as(&mut self, file: &str) {
        let bytes = self.built_demo_lib();
        self.put_in_package_folder(file, &bytes);
        self.project_of_the_package_folder();
        self.depend_on(&["demo_lib"]);
    }

    fn put_in_package_folder(&self, file: &str, bytes: &[u8]) -> PathBuf {
        let path = self.sandbox.root().join(LOCAL_PACKAGES_DIR).join(file);
        std::fs::write(&path, bytes).expect("write a package into the folder");
        path
    }

    /// A project that reads the package folder and nothing else, installing
    /// into the scratch destination `dest`, which stands for the venv: a venv
    /// needs a LabVIEW, and the sandbox refuses to touch a real one.
    fn project_of_the_package_folder(&self) {
        self.sandbox.use_local_project(Some(CONSUMER_LABVIEW));
        self.sandbox.make_empty_dest();
    }

    /// The loopback feed lists `file` in its `index.vipr`, with the MD5 of
    /// `bytes`, serves `bytes` as it, and is the project's only source.
    fn host(&self, file: &str, bytes: Vec<u8>) {
        let id = file.strip_suffix(".lvpkg").expect("a .lvpkg");
        let index = format!(
            "; lvpm acceptance fixture\n\n[Package {id}]\nPackage.URL=packages/{file}\n\
             Package.MD5={}\nPackage.Display Name=Demo Lib\n",
            md5_hex(&bytes)
        );
        let feed = self.sandbox.feed();
        feed.publish(PROJECT_FEED_DIR, &index);
        feed.serve(&archive_path(PROJECT_FEED_DIR, file), bytes);
        self.sandbox
            .use_loopback_project_for_labview(CONSUMER_LABVIEW);
        self.sandbox.make_empty_dest();
    }

    fn depend_on(&self, names: &[&str]) {
        for name in names {
            self.depend(DependencyTable::Dependencies, name, ANY_VERSION);
        }
    }

    fn lvpm_install(&mut self) {
        self.note_the_project();
        let output = self
            .sandbox
            .run_lvpm(&["install", "--prefix", DEST_DIR], None);
        self.record_output(output);
    }

    /// The members of the Background's library, below `dest`.
    fn assert_members_installed(&self) {
        let dir = self.dest_dir().join(PREFIX_APPLICATION).join(INSTALL_DIR);
        let a_vi = dir.join("A.vi");
        let text = std::fs::read_to_string(&a_vi).unwrap_or_else(|e| {
            let held = self.sandbox.list_file_names(DEST_DIR);
            panic!("{}: {e}; dest holds:\n{held}", a_vi.display())
        });
        assert_eq!(text, "a vi", "{}", a_vi.display());
        assert!(dir.join("Lib.lvlib").is_file(), "{}", dir.display());
    }
}

#[given(
    regex = r#"^a project depending on demo_lib, whose package folder \(sources\.local\) holds the built "([^"]+)"$"#
)]
async fn folder_holds_the_built_package(world: &mut CliWorld, file: String) {
    assert_eq!(file, DEMO_LIB);
    world.project_with_demo_lib_in_folder_as(&file);
}

#[when("lvpm install runs")]
async fn install_runs(world: &mut CliWorld) {
    world.lvpm_install();
}

#[then(
    regex = r#"^it succeeds, the lock records demo_lib 1\.2\.3 with the archive's SHA-256, and the venv holds the members below "([^"]+)"$"#
)]
async fn lock_and_venv_of_the_install(world: &mut CliWorld, dir: String) {
    assert_eq!(dir, INSTALL_DIR);
    world.succeeded();
    let package = world.locked_package("demo_lib");
    assert_eq!(package["version"].as_str(), Some("1.2.3"));
    let archive = std::fs::read(world.work(&format!("dist/{DEMO_LIB}"))).expect("the archive");
    assert_eq!(
        package["sha256"].as_str(),
        Some(sha256_hex(&archive).as_str())
    );
    world.assert_members_installed();
}

#[given(regex = r#"^"([^"]+)" imported with lvpm cache add$"#)]
async fn package_imported(world: &mut CliWorld, file: String) {
    assert_eq!(file, DEMO_LIB);
    world.built_demo_lib();
    let archive = world.work(&format!("dist/{file}"));
    world
        .sandbox
        .setup_lvpm(&["cache", "add", archive.to_str().expect("a path")]);
    world.project_of_the_package_folder();
}

#[when("a project depending on demo_lib runs lvpm install")]
async fn project_runs_install(world: &mut CliWorld) {
    world.depend_on(&["demo_lib"]);
    world.lvpm_install();
}

#[then("it succeeds and the venv holds the members")]
async fn succeeds_and_venv_holds_members(world: &mut CliWorld) {
    world.succeeded();
    world.assert_members_installed();
}

#[given(regex = r#"^a hosted feed whose index\.vipr names "([^"]+)"$"#)]
async fn hosted_feed_names(world: &mut CliWorld, file: String) {
    assert_eq!(file, DEMO_LIB);
    let bytes = world.built_demo_lib();
    world.host(&file, bytes);
}

#[given(regex = r#"^a package folder holding the built "([^"]+)" and "([^"]+)"$"#)]
async fn folder_holds_lvpkg_and_vip(world: &mut CliWorld, lvpkg: String, vip: String) {
    assert_eq!(
        (lvpkg.as_str(), vip.as_str()),
        (DEMO_LIB, "other_lib-1.0.0.1.vip")
    );
    assert_eq!(OTHER_LIB_VIP.file_name(), vip);
    let bytes = world.built_demo_lib();
    world.put_in_package_folder(&lvpkg, &bytes);
    world.put_in_package_folder(&vip, &OTHER_LIB_VIP.build_vip());
    world.project_of_the_package_folder();
}

#[when("a project depending on both runs lvpm install")]
#[when("a project depending on demo_lib and other_lib runs lvpm install")]
async fn project_runs_install_for_both(world: &mut CliWorld) {
    world.depend_on(&["demo_lib", "other_lib"]);
    world.lvpm_install();
}

#[then("it succeeds and the venv holds both packages' files")]
async fn succeeds_and_venv_holds_both(world: &mut CliWorld) {
    world.succeeded();
    world.assert_members_installed();
    let vip_file = world
        .dest_dir()
        .join(PREFIX_APPLICATION)
        .join(OTHER_LIB_VIP.payload_path());
    let text = std::fs::read_to_string(&vip_file)
        .unwrap_or_else(|e| panic!("{}: {e}", vip_file.display()));
    assert_eq!(text, "other_lib 1.0.0.1\n");
    let locked = world.locked_versions();
    for pair in [("demo_lib", "1.2.3"), ("other_lib", "1.0.0.1")] {
        assert!(
            locked.contains(&(pair.0.into(), pair.1.into())),
            "{locked:?}"
        );
    }
}

#[given(regex = r#"^a package folder holding the built package renamed "([^"]+)"$"#)]
async fn folder_holds_renamed_package(world: &mut CliWorld, file: String) {
    assert_eq!(file, "demo_lib-1.2.3.vip");
    world.project_with_demo_lib_in_folder_as(&file);
}

#[given(regex = r#"^a hosted feed whose "([^"]+)" has "([^"]+)" changed after build$"#)]
async fn hosted_feed_tampered(world: &mut CliWorld, file: String, entry: String) {
    assert_eq!(file, DEMO_LIB);
    let bytes = world.built_demo_lib();
    world.host(&file, with_entry_changed(&bytes, &entry));
}

#[then(
    regex = r#"^it fails with the "entry changed" message naming "([^"]+)" and "([^"]+)", and the venv is unchanged$"#
)]
async fn fails_entry_changed_and_nothing_written(
    world: &mut CliWorld,
    entry: String,
    package: String,
) {
    let message = world.failed();
    let want = entry_changed(&entry);
    assert!(message.contains(&want), "{want:?} is not in: {message}");
    // A sync reads the archive from the cache by its SHA-256, so it has no file
    // name to give; it names the package it was installing.
    assert!(
        message.contains(&package),
        "{package:?} is not named in: {message}"
    );
    world.assert_dest_unchanged();
}

#[given(regex = r#"^"([^"]+)" with "([^"]+)" changed after build$"#)]
async fn package_tampered(world: &mut CliWorld, file: String, entry: String) {
    assert_eq!(file, DEMO_LIB);
    let bytes = world.built_demo_lib();
    let path = world.sandbox.root().join("incoming").join(&file);
    std::fs::create_dir_all(path.parent().expect("a folder")).expect("create the folder");
    std::fs::write(&path, with_entry_changed(&bytes, &entry)).expect("write the package");
    world.lvpkg.tampered = Some((path, entry));
}

#[when("lvpm cache add runs on it")]
async fn cache_add_runs_on_it(world: &mut CliWorld) {
    let (path, _) = world.lvpkg.tampered.clone().expect("a tampered package");
    world.lvpkg.cache = Some(cache_files(&world.sandbox.cache_dir()));
    let output = world
        .sandbox
        .run_lvpm(&["cache", "add", path.to_str().expect("a path")], None);
    world.record_output(output);
}

#[then(
    regex = r#"^it fails with the "entry changed" message naming "([^"]+)" and the entry, and the cache is unchanged$"#
)]
async fn cache_add_refused(world: &mut CliWorld, file: String) {
    let (_, entry) = world.lvpkg.tampered.clone().expect("a tampered package");
    let message = world.failed();
    // The file is named by the line that says what was being imported, the
    // entry by the cause under it.
    assert!(
        message
            .lines()
            .any(|line| line.contains("importing") && line.ends_with(&file)),
        "{file} is not named in: {message}"
    );
    let want = entry_changed(&entry);
    assert!(message.contains(&want), "{want:?} is not in: {message}");
    let before = world.lvpkg.cache.as_ref().expect("the cache was noted");
    assert!(
        cache_files(&world.sandbox.cache_dir()) == *before,
        "the cache changed"
    );
    // A recorded import would make a second run report the file as reused.
    let (path, _) = world.lvpkg.tampered.clone().expect("a tampered package");
    let again = world
        .sandbox
        .run_lvpm(&["cache", "add", path.to_str().expect("a path")], None);
    assert_ne!(
        again.exit_code, 0,
        "a second cache add succeeded: {again:?}"
    );
}

#[given(regex = r#"^a package folder holding a good "([^"]+)" and a tampered "([^"]+)"$"#)]
async fn folder_holds_good_and_tampered(world: &mut CliWorld, good: String, tampered: String) {
    assert_eq!(
        (good.as_str(), tampered.as_str()),
        ("other_lib-1.0.0.lvpkg", DEMO_LIB)
    );
    let other = world.built_as("other_lib", "1.0.0", "Other Lib");
    let demo = world.built_demo_lib();
    world.put_in_package_folder(&good, &other);
    let entry = format!("{INSTALL_DIR}/A.vi");
    let path = world.put_in_package_folder(&tampered, &with_entry_changed(&demo, &entry));
    world.lvpkg.tampered = Some((path, entry));
    world.project_of_the_package_folder();
}

#[then(
    regex = r#"^stderr warns "! ignoring" the tampered file, naming "([^"]+)", the install fails because demo_lib is not found, and the venv is unchanged$"#
)]
async fn folder_ignores_tampered(world: &mut CliWorld, entry: String) {
    let message = world.failed();
    let (path, _) = world.lvpkg.tampered.clone().expect("a tampered package");
    let file = path.file_name().expect("a file name").to_string_lossy();
    let warnings: Vec<&str> = message
        .lines()
        .filter(|l| l.contains("! ignoring"))
        .collect();
    let [warning] = warnings[..] else {
        panic!("one package is ignored, not {warnings:?}, in: {message}");
    };
    assert!(
        warning.contains(file.as_ref()),
        "{file} is not named in: {warning}"
    );
    assert!(
        warning.ends_with(&entry_changed(&entry)),
        "{entry:?} is not named in: {warning}"
    );
    assert!(
        message.contains("demo_lib is in no configured source"),
        "demo_lib is not reported missing in: {message}"
    );
    world.assert_dest_unchanged();
}

#[when(regex = r#"^lvpm cache add runs on "([^"]+)"$"#)]
async fn cache_add_runs_on_file(world: &mut CliWorld, file: String) {
    // A file with the wrong extension, not a missing one.
    world.put(&file, "PK");
    let output = world.sandbox.run_lvpm(&["cache", "add", &file], None);
    world.record_output(output);
}

#[then(regex = r"^it fails, and the message names \.vip, \.ogp and \.lvpkg$")]
async fn names_the_formats(world: &mut CliWorld) {
    let message = world.failed();
    for format in [".vip", ".ogp", ".lvpkg"] {
        assert!(
            message.contains(format),
            "{format} is not named in: {message}"
        );
    }
}
