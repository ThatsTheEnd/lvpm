//! A community index repo read one name at a time: the transport over the
//! contents API, its cache, and which names `index::load` asks for. Rust tests,
//! not harness scenarios: they live here because `lvpm-testkit` depends on
//! `lvpm-core`.

use lvpm_core::cache::Cache;
use lvpm_core::cancel::CancelToken;
use lvpm_core::community;
use lvpm_core::config::Config;
use lvpm_core::github::{GitHub, GitHubAuth, contents_cached};
use lvpm_core::index::{self, Entry, IndexFeed, OciFeed, Remote, SourceSet};
use lvpm_core::manifest_edit::DependencyTable;
use lvpm_core::oci::Oci;
use lvpm_core::ops::index_files::IndexFiles;
use lvpm_core::ops::install::InstallOptions;
use lvpm_core::ops::{OpEnv, add_remove, install, lock, query};
use lvpm_testkit::events::RecordingSink;
use lvpm_testkit::feed::Feed;
use lvpm_testkit::github::GitHubFake;
use lvpm_testkit::oci::Registry;
use reqwest::blocking::Client;
use std::path::{Path, PathBuf};

const REPO: &str = "o/r";
const LIMIT: u64 = 1024 * 1024;

fn fake_with(files: &[(&str, &str)]) -> GitHubFake {
    let fake = GitHubFake::start(&[(REPO, vec![])]);
    for (path, body) in files {
        fake.add_file(REPO, path, body);
    }
    fake
}

fn get(
    fake: &GitHubFake,
    cache: &Cache,
    path: &str,
    refresh: bool,
    limit: u64,
) -> anyhow::Result<Option<String>> {
    let auth = GitHubAuth::anonymous();
    let client = Client::new();
    contents_cached(
        cache,
        &client,
        &fake.api_url(),
        REPO,
        path,
        &auth,
        refresh,
        limit,
    )
}

fn cache() -> (tempfile::TempDir, Cache) {
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(dir.path()).unwrap();
    (dir, cache)
}

#[test]
fn a_file_is_its_body_and_a_missing_one_is_not_listed() {
    let fake = fake_with(&[("packages/foo.toml", "name = \"foo\"\n")]);
    let (_dir, cache) = cache();
    assert_eq!(
        get(&fake, &cache, "packages/foo.toml", false, LIMIT).unwrap(),
        Some("name = \"foo\"\n".to_string())
    );
    assert_eq!(
        get(&fake, &cache, "packages/bar.toml", false, LIMIT).unwrap(),
        None
    );
    let raw = fake
        .requests()
        .into_iter()
        .map(|r| r.accept)
        .collect::<Vec<_>>();
    let want = Some("application/vnd.github.raw".to_string());
    assert_eq!(raw, [want.clone(), want]);
}

#[test]
fn any_other_status_is_an_error_that_says_the_status() {
    let fake = fake_with(&[("packages/foo.toml", "x")]);
    fake.fail_with(500);
    let (_dir, cache) = cache();
    let error = get(&fake, &cache, "packages/foo.toml", false, LIMIT).unwrap_err();
    assert_eq!(format!("{error:#}"), "HTTP 500");
}

#[test]
fn a_body_over_the_limit_is_an_error_naming_the_limit_and_is_not_cached() {
    let fake = fake_with(&[("packages/foo.toml", &"x".repeat(33))]);
    let (_dir, cache) = cache();
    let error = get(&fake, &cache, "packages/foo.toml", false, 32).unwrap_err();
    assert_eq!(format!("{error:#}"), "a response of more than 32 bytes");
    // Asked again with room for it: the first answer left nothing behind.
    assert!(
        get(&fake, &cache, "packages/foo.toml", false, 33)
            .unwrap()
            .is_some()
    );
    assert_eq!(fake.requests().len(), 2);
}

#[test]
fn a_cached_answer_is_used_without_a_request_unless_refreshing() {
    let fake = fake_with(&[("packages/foo.toml", "one")]);
    let (_dir, cache) = cache();
    get(&fake, &cache, "packages/foo.toml", false, LIMIT).unwrap();
    assert_eq!(fake.requests().len(), 1);

    fake.add_file(REPO, "packages/foo.toml", "two");
    let again = get(&fake, &cache, "packages/foo.toml", false, LIMIT).unwrap();
    assert_eq!((again.as_deref(), fake.requests().len()), (Some("one"), 1));

    let refreshed = get(&fake, &cache, "packages/foo.toml", true, LIMIT).unwrap();
    assert_eq!(
        (refreshed.as_deref(), fake.requests().len()),
        (Some("two"), 2)
    );
}

#[test]
fn a_404_is_cached_too_until_refreshing_finds_the_file() {
    let fake = fake_with(&[]);
    let (_dir, cache) = cache();
    assert_eq!(
        get(&fake, &cache, "packages/foo.toml", false, LIMIT).unwrap(),
        None
    );
    fake.add_file(REPO, "packages/foo.toml", "now");
    assert_eq!(
        get(&fake, &cache, "packages/foo.toml", false, LIMIT).unwrap(),
        None
    );
    assert_eq!(fake.requests().len(), 1);

    let found = get(&fake, &cache, "packages/foo.toml", true, LIMIT).unwrap();
    assert_eq!(found.as_deref(), Some("now"));
    // The body replaced the marker.
    assert_eq!(
        get(&fake, &cache, "packages/foo.toml", false, LIMIT)
            .unwrap()
            .as_deref(),
        Some("now")
    );
    assert_eq!(fake.requests().len(), 2);
}

/// `packages/<name>.toml` for `name`, owned by a/x, with one version that
/// requires `requires`.
fn entry_file(name: &str, requires: &[&str]) -> String {
    let requires = requires
        .iter()
        .map(|r| format!("\"{r}\""))
        .collect::<Vec<_>>();
    format!(
        "name = \"{name}\"
github = \"a/x\"

[[version]]
version = \"1.0.0\"
         url = \"https://github.com/a/x/releases/download/v1/{name}-1.0.0.lvpkg\"
         sha256 = \"{}\"
requires = [{}]
",
        "a".repeat(64),
        requires.join(", ")
    )
}

fn index_with(files: &[(&str, &[&str])]) -> GitHubFake {
    let fake = GitHubFake::start(&[(REPO, vec![])]);
    for (name, requires) in files {
        fake.add_file(
            REPO,
            &format!("packages/{name}.toml"),
            &entry_file(name, requires),
        );
    }
    fake
}

/// What `index::load_each` gave: the entries, and each source that failed with
/// why.
#[derive(Default)]
struct Read {
    entries: Vec<Entry>,
    failed: Vec<(String, String)>,
}

impl Read {
    fn names(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.name.clone()).collect()
    }
}

/// How a read is set up: the `[sources]` lines, which are the community index
/// at `o/r` alone unless a test lists others, and the rest of a project that
/// has one.
struct Setup<'a> {
    fake: &'a GitHubFake,
    cache: &'a Path,
    remotes: Vec<Remote>,
    local: Vec<PathBuf>,
    oci: Vec<OciFeed>,
    refresh: bool,
    limit: u64,
}

/// The community index repo of the fake.
fn index_feed() -> IndexFeed {
    IndexFeed {
        owner: "o".into(),
        repo: "r".into(),
    }
}

impl<'a> Setup<'a> {
    fn new(fake: &'a GitHubFake, cache: &'a Path) -> Setup<'a> {
        Setup {
            fake,
            cache,
            remotes: vec![Remote::Index(index_feed())],
            local: Vec::new(),
            oci: Vec::new(),
            refresh: false,
            limit: LIMIT,
        }
    }

    fn read(&self, wanted: &[&str]) -> Read {
        let cache = Cache::open(self.cache).unwrap();
        let client = Client::new();
        let auth = GitHubAuth::anonymous();
        let api = self.fake.api_url();
        let fetch = |feed: &IndexFeed, key: &str| {
            let repo = format!("{}/{}", feed.owner, feed.repo);
            let path = format!("packages/{key}.toml");
            contents_cached(
                &cache,
                &client,
                &api,
                &repo,
                &path,
                &auth,
                self.refresh,
                self.limit,
            )
        };
        self.read_with(wanted, &fetch)
    }

    /// [`Self::read`], with `fetch` answering for the index.
    fn read_with(
        &self,
        wanted: &[&str],
        fetch: &dyn Fn(&IndexFeed, &str) -> anyhow::Result<Option<String>>,
    ) -> Read {
        let wanted: Vec<String> = wanted.iter().map(|name| name.to_string()).collect();
        let mut read = Read::default();
        let loaded = index::load_each(
            self.cache,
            self.refresh,
            &SourceSet {
                extra: &self.remotes,
                local: &self.local,
                oci: &self.oci,
                wanted: &wanted,
                index_fetch: fetch,
                index_parse: community::read_entries,
                ..SourceSet::default()
            },
            &Default::default(),
            &Oci::default(),
            &RecordingSink::default(),
            &CancelToken::new(),
            |source, error| {
                read.failed.push((source.to_string(), format!("{error:#}")));
                Ok(())
            },
        )
        .unwrap();
        read.entries = loaded.index.entries;
        read
    }
}

/// The files of the index asked for; its one request about the repo itself, after
/// a 404, is not one of them.
fn asked(fake: &GitHubFake) -> Vec<String> {
    let all = fake.requests().into_iter().map(|r| r.path);
    all.filter_map(|p| Some(p.strip_prefix("/repos/o/r/contents/")?.to_string()))
        .collect()
}

#[test]
fn an_entry_file_is_read_as_entries_of_the_index_source() {
    let fake = index_with(&[("foo", &["bar >=2"])]);
    let dir = tempfile::tempdir().unwrap();
    let read = Setup::new(&fake, dir.path()).read(&["foo"]);
    assert_eq!(read.failed, []);
    let [entry] = read.entries.as_slice() else {
        panic!("{:?}", read.names())
    };
    assert_eq!(
        (entry.name.as_str(), entry.source.as_str()),
        ("foo", "github-index:o/r")
    );
    assert_eq!(entry.index_pin.as_ref().unwrap().publisher, "a/x");
}

#[test]
fn the_index_is_asked_for_each_wanted_name_and_the_names_they_require() {
    let fake = index_with(&[("foo", &["bar >=2"]), ("bar", &[])]);
    let dir = tempfile::tempdir().unwrap();
    Setup::new(&fake, dir.path()).read(&["foo"]);
    assert_eq!(asked(&fake), ["packages/foo.toml", "packages/bar.toml"]);
}

/// A hosted folder at `/team/` listing `name` 1.0 with `requires` (`bar>=1.0`).
fn hosted_listing(name: &str, requires: &str) -> (Feed, String) {
    let feed = Feed::start(&[]);
    let body = format!(
        "[Package {name}-1.0]\nPackage.URL={name}-1.0.vip\nDependencies.Requires={requires}\n"
    );
    feed.publish("/team/", &body);
    let url = feed.source_url("/team/");
    (feed, url)
}

#[test]
fn requires_of_a_name_nobody_wanted_are_not_followed() {
    let fake = index_with(&[("foo", &[]), ("zap", &[])]);
    let (_feed, url) = hosted_listing("qux", "zap>=1.0");
    let dir = tempfile::tempdir().unwrap();
    let mut setup = Setup::new(&fake, dir.path());
    setup.remotes.insert(0, Remote::Hosted(url));
    setup.read(&["foo"]);
    assert_eq!(asked(&fake), ["packages/foo.toml"]);
}

#[test]
fn requires_of_a_hosted_entry_of_a_wanted_name_are_followed() {
    let fake = index_with(&[("bar", &[])]);
    let (_feed, url) = hosted_listing("foo", "bar>=1.0");
    let dir = tempfile::tempdir().unwrap();
    let mut setup = Setup::new(&fake, dir.path());
    setup.remotes.insert(0, Remote::Hosted(url));
    let read = setup.read(&["foo"]);
    assert_eq!(asked(&fake), ["packages/foo.toml", "packages/bar.toml"]);
    assert_eq!(read.names(), ["foo", "bar"]);
}

#[test]
fn a_requires_cycle_ends_with_each_name_asked_once() {
    let fake = index_with(&[("foo", &["bar"]), ("bar", &["foo"])]);
    let dir = tempfile::tempdir().unwrap();
    Setup::new(&fake, dir.path()).read(&["foo"]);
    assert_eq!(asked(&fake), ["packages/foo.toml", "packages/bar.toml"]);
}

#[test]
fn a_name_is_asked_by_its_lower_case_key_and_a_name_that_cannot_be_a_file_name_is_not_asked() {
    let fake = index_with(&[("foo", &[])]);
    let dir = tempfile::tempdir().unwrap();
    Setup::new(&fake, dir.path()).read(&["../x", "a/b", "", "a b", "Foo", "FOO"]);
    assert_eq!(asked(&fake), ["packages/foo.toml"]);
}

#[test]
fn with_no_wanted_names_the_index_gets_no_request() {
    let fake = index_with(&[("foo", &[])]);
    let dir = tempfile::tempdir().unwrap();
    let read = Setup::new(&fake, dir.path()).read(&[]);
    assert_eq!((fake.requests().len(), read.entries.len()), (0, 0));
}

#[test]
fn a_name_the_index_does_not_list_is_a_404_and_nothing_else() {
    let fake = index_with(&[]);
    let dir = tempfile::tempdir().unwrap();
    let read = Setup::new(&fake, dir.path()).read(&["foo"]);
    assert_eq!(read.names(), Vec::<String>::new());
    assert_eq!(read.failed, []);
    assert_eq!(asked(&fake), ["packages/foo.toml"]);
}

#[test]
fn an_index_that_fails_is_a_listing_failure_of_that_source() {
    let fake = index_with(&[("foo", &[])]);
    fake.fail_with(500);
    let dir = tempfile::tempdir().unwrap();
    let read = Setup::new(&fake, dir.path()).read(&["foo", "bar"]);
    assert_eq!(
        read.failed,
        [("github-index:o/r".to_string(), "HTTP 500".to_string())]
    );
    // The index is not asked again once it has failed.
    assert_eq!(asked(&fake), ["packages/foo.toml"]);
}

#[test]
fn an_entry_that_does_not_parse_is_a_listing_failure_naming_its_file_and_the_key() {
    let fake = index_with(&[]);
    fake.add_file(
        REPO,
        "packages/foo.toml",
        "name = \"foo\"
own = 1
",
    );
    let dir = tempfile::tempdir().unwrap();
    let read = Setup::new(&fake, dir.path()).read(&["foo"]);
    assert_eq!(
        read.failed,
        [(
            "github-index:o/r".to_string(),
            "packages/foo.toml in github:o/r does not parse: own: unknown key".to_string()
        )]
    );
    assert_eq!(read.names(), Vec::<String>::new());
}

#[test]
fn an_entry_body_over_the_limit_is_a_listing_failure_naming_the_limit() {
    let fake = index_with(&[("foo", &[])]);
    let dir = tempfile::tempdir().unwrap();
    let mut setup = Setup::new(&fake, dir.path());
    setup.limit = 16;
    let read = setup.read(&["foo"]);
    assert_eq!(
        read.failed,
        [(
            "github-index:o/r".to_string(),
            "a response of more than 16 bytes".to_string()
        )]
    );
}

#[test]
fn a_cached_entry_file_is_used_without_a_request_unless_refreshing() {
    let fake = index_with(&[("foo", &[])]);
    let dir = tempfile::tempdir().unwrap();
    let mut setup = Setup::new(&fake, dir.path());
    assert_eq!(setup.read(&["foo"]).names(), ["foo"]);
    assert_eq!(fake.requests().len(), 1);
    assert_eq!(setup.read(&["foo"]).names(), ["foo"]);
    assert_eq!(fake.requests().len(), 1, "no new request without --refresh");
    setup.refresh = true;
    assert_eq!(setup.read(&["foo"]).names(), ["foo"]);
    assert_eq!(fake.requests().len(), 2);
}

#[test]
fn a_404_is_cached_too() {
    let fake = index_with(&[]);
    let dir = tempfile::tempdir().unwrap();
    let setup = Setup::new(&fake, dir.path());
    setup.read(&["foo"]);
    setup.read(&["foo"]);
    assert_eq!(fake.requests().len(), 1);
}

/// `wanted` read through [`IndexFiles`], as a command reads them.
fn read_through_index_files(setup: &Setup, wanted: &[&str]) -> Read {
    let github = GitHub {
        api: setup.fake.api_url(),
        ..GitHub::default()
    };
    let files = IndexFiles::open(setup.cache, &github, false).unwrap();
    setup.read_with(wanted, &|feed, key| files.read(feed, key))
}

#[test]
fn an_index_repo_that_is_not_there_fails_its_source_instead_of_listing_nothing() {
    let fake = index_with(&[]);
    let dir = tempfile::tempdir().unwrap();
    let mut setup = Setup::new(&fake, dir.path());
    setup.remotes = vec![Remote::Index(IndexFeed {
        owner: "o".into(),
        repo: "typo".into(),
    })];
    let read = read_through_index_files(&setup, &["foo"]);
    assert_eq!(
        read.failed,
        [(
            "github-index:o/typo".to_string(),
            "the community index github-index:o/typo cannot be read: not found. If it is private, run gh auth login or set GH_TOKEN. fix: spell it as owner/repo as on GitHub in [sources], or remove it".to_string()
        )]
    );
}

#[test]
fn a_private_index_repo_read_without_a_token_fails_its_source() {
    let fake = index_with(&[("foo", &[])]);
    fake.make_private(REPO);
    let dir = tempfile::tempdir().unwrap();
    let read = read_through_index_files(&Setup::new(&fake, dir.path()), &["foo"]);
    assert_eq!(read.names(), Vec::<String>::new());
    let [(source, why)] = read.failed.as_slice() else {
        panic!("{:?}", read.failed);
    };
    assert_eq!(source, "github-index:o/r");
    assert!(
        why.starts_with("the community index github-index:o/r cannot be read: not found"),
        "{why}"
    );
}

#[test]
fn an_index_repo_that_lacks_two_names_is_asked_about_itself_once() {
    let fake = index_with(&[]);
    let dir = tempfile::tempdir().unwrap();
    let read = read_through_index_files(&Setup::new(&fake, dir.path()), &["foo", "bar"]);
    assert_eq!(read.failed, []);
    let paths: Vec<String> = fake.requests().into_iter().map(|r| r.path).collect();
    assert_eq!(
        paths,
        [
            "/repos/o/r/contents/packages/foo.toml",
            "/repos/o/r",
            "/repos/o/r/contents/packages/bar.toml"
        ]
    );
}

/// A project that takes `dependencies` (lines of `[dependencies]`) and has the
/// fake at `o/r` as its community index, and an environment for it that
/// installs into a scratch folder.
fn project(dependencies: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let manifest = format!(
        "[project]
labview = \">=2024\"

[sources]
defaults = false
         community = \"github:o/r\"

[dependencies]
{dependencies}"
    );
    std::fs::write(dir.path().join("lvpm.toml"), manifest).unwrap();
    dir
}

fn env<'a>(dir: &Path, sink: &'a RecordingSink, fake: &GitHubFake) -> OpEnv<'a> {
    let config = Config {
        cache: dir.join("cache"),
        hooks_run: Vec::new(),
        local_sources: Vec::new(),
        venv_dir: ".lv-venv".into(),
    };
    let mut env = OpEnv::new(sink, config, Vec::new(), dir.to_path_buf());
    env.github = GitHub {
        api: fake.api_url(),
        ..GitHub::default()
    };
    env.flags.prefix = Some(dir.join("prefix"));
    env
}

#[test]
fn search_in_a_project_with_a_community_index_makes_no_index_request() {
    let fake = index_with(&[("foo", &[])]);
    let dir = project(
        "foo = \"*\"
",
    );
    let sink = RecordingSink::default();
    query::search(&env(dir.path(), &sink, &fake), "foo").unwrap();
    query::catalog(&env(dir.path(), &sink, &fake), None).unwrap();
    assert_eq!(asked(&fake), Vec::<String>::new());
}

#[test]
fn the_catalog_still_fails_when_its_only_read_source_fails_beside_an_index() {
    let fake = index_with(&[]);
    let dir = project("");
    let sink = RecordingSink::default();
    let mut env = env(dir.path(), &sink, &fake);
    env.config.local_sources = vec![dir.path().join("missing")];
    let why = query::catalog(&env, None).unwrap_err().to_string();
    assert!(why.starts_with("no package source could be read"), "{why}");
}

#[test]
fn lock_asks_for_the_manifests_dependencies() {
    let fake = index_with(&[]);
    let dir = project(
        "foo = \"*\"
Bar = \">=1.0\"
",
    );
    let sink = RecordingSink::default();
    let _ = lock::lock(&env(dir.path(), &sink, &fake));
    assert_eq!(asked(&fake), ["packages/foo.toml", "packages/bar.toml"]);
}

/// The package file of `name` at 1.0 requiring each of `requires`.
fn package(name: &str, requires: &[&str]) -> Vec<u8> {
    let meta = lvpm_core::lvpkg::Meta {
        name: name.into(),
        version: "1.0".into(),
        labview: ">=2024".into(),
        vendor: "acme".into(),
        display_name: name.into(),
        install_dir: format!("vi.lib/acme/{name}"),
        dependencies: requires
            .iter()
            .map(|r| (r.to_string(), "*".into()))
            .collect(),
    };
    lvpm_core::lvpkg::write(&meta, &[("a.vi".to_string(), b"a".to_vec())], None).unwrap()
}

#[test]
fn what_a_github_dependency_requires_is_asked_of_the_community_index() {
    use lvpm_testkit::github::FakeRelease;
    use sha2::{Digest, Sha256};
    let bar = package("bar", &[]);
    let release = |file: &str, bytes: &[u8]| vec![FakeRelease::new("v1").asset(file, bytes)];
    let fake = GitHubFake::start(&[
        (REPO, vec![]),
        ("g/foo", release("foo-1.0.lvpkg", &package("foo", &["bar"]))),
        ("a/x", release("bar-1.0.lvpkg", &bar)),
    ]);
    fake.add_file(
        REPO,
        "packages/bar.toml",
        &format!(
            "name = \"bar\"
github = \"a/x\"

[[version]]
version = \"1.0\"
url = \"{}/a/x/releases/download/v1/bar-1.0.lvpkg\"
sha256 = \"{}\"
",
            fake.api_url(),
            Sha256::digest(&bar)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        ),
    );
    let dir = project("foo = { github = \"g/foo\", version = \"*\" }\n");
    let sink = RecordingSink::default();

    let locked = lock::lock(&env(dir.path(), &sink, &fake)).unwrap();

    let names: Vec<&str> = locked.packages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["bar", "foo"]);
    assert_eq!(asked(&fake), ["packages/foo.toml", "packages/bar.toml"]);
}

#[test]
fn a_name_passed_to_add_is_asked() {
    let fake = index_with(&[]);
    let dir = project("");
    let sink = RecordingSink::default();
    let env = env(dir.path(), &sink, &fake);
    let options = InstallOptions::default();
    let _ = add_remove::add(&env, "foo", DependencyTable::Dependencies, None, &options);
    assert_eq!(asked(&fake), ["packages/foo.toml"]);
}

#[test]
fn a_name_passed_to_install_is_asked_with_the_manifests_dependencies() {
    let fake = index_with(&[]);
    let dir = project(
        "bar = \"*\"
",
    );
    let sink = RecordingSink::default();
    let env = env(dir.path(), &sink, &fake);
    let _ = install::install(&env, Some("foo@1.0"), &InstallOptions::default());
    assert_eq!(asked(&fake), ["packages/bar.toml", "packages/foo.toml"]);
}

#[test]
fn a_name_passed_to_info_is_asked_with_the_manifests_dependencies() {
    let fake = index_with(&[]);
    let dir = project(
        "bar = \"*\"
",
    );
    let sink = RecordingSink::default();
    let _ = query::info(&env(dir.path(), &sink, &fake), "foo");
    assert_eq!(asked(&fake), ["packages/bar.toml", "packages/foo.toml"]);
}

#[test]
fn requires_of_an_oci_entry_of_a_wanted_name_are_followed() {
    let fake = index_with(&[("beta", &[])]);
    let registry = Registry::start();
    registry.seed_package(
        "team/alpha",
        "1.0.0",
        &[
            ("org.opencontainers.image.title", "alpha"),
            ("org.opencontainers.image.version", "1.0.0"),
            ("dev.lvpm.requires", "beta >=1.0"),
        ],
        b"alpha 1.0.0",
    );
    let dir = tempfile::tempdir().unwrap();
    let mut setup = Setup::new(&fake, dir.path());
    setup.oci = vec![OciFeed {
        name: "team".to_string(),
        url: lvpm_core::oci::FeedUrl::parse(&format!("oci://{}/team", registry.host())).unwrap(),
        pinned: vec!["alpha".to_string()],
    }];
    let read = setup.read(&["alpha"]);
    assert_eq!(asked(&fake), ["packages/alpha.toml", "packages/beta.toml"]);
    // The index, a remote source, ranks before OCI.
    assert_eq!(read.names(), ["beta", "alpha"]);
}

#[test]
fn an_index_that_fails_after_listing_a_name_contributes_no_entries() {
    let fake = index_with(&[]);
    let dir = tempfile::tempdir().unwrap();
    let fetch = |_: &IndexFeed, key: &str| match key {
        "foo" => Ok(Some(entry_file("foo", &[]))),
        _ => Err(anyhow::anyhow!("HTTP 500")),
    };
    let read = Setup::new(&fake, dir.path()).read_with(&["foo", "bar"], &fetch);
    assert_eq!(
        read.failed,
        [("github-index:o/r".to_string(), "HTTP 500".to_string())]
    );
    assert_eq!(read.names(), Vec::<String>::new());
}

#[test]
fn two_files_that_do_not_parse_are_one_failure_of_the_index_and_the_first_is_the_one_told() {
    let fake = index_with(&[]);
    fake.add_file(REPO, "packages/foo.toml", "name = \"foo\"\nown = 1\n");
    fake.add_file(REPO, "packages/bar.toml", "name = \"bar\"\nmine = 1\n");
    fake.add_file(REPO, "packages/baz.toml", &entry_file("baz", &[]));
    let dir = tempfile::tempdir().unwrap();
    let read = Setup::new(&fake, dir.path()).read(&["foo", "bar", "baz"]);
    assert_eq!(
        read.failed,
        [(
            "github-index:o/r".to_string(),
            "packages/foo.toml in github:o/r does not parse: own: unknown key".to_string()
        )]
    );
    // A failed index contributes nothing, so the good file is not used either.
    assert_eq!(read.names(), Vec::<String>::new());
}

/// A package folder with `name` `version` in it, as an archive.
fn local_folder(name: &str, version: &str) -> tempfile::TempDir {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("spec", options).unwrap();
    let spec = format!("[Package]\nName={name}\nVersion={version}\n");
    zip.write_all(spec.as_bytes()).unwrap();
    let bytes = zip.finish().unwrap().into_inner();
    std::fs::write(dir.path().join(format!("{name}-{version}.vip")), bytes).unwrap();
    dir
}

/// Where `read` says each name comes from.
fn sources_of(read: &Read) -> Vec<(String, String)> {
    let each = read.entries.iter();
    each.map(|e| (e.name.clone(), e.source.clone())).collect()
}

fn pair(name: &str, source: &str) -> (String, String) {
    (name.to_string(), source.to_string())
}

#[test]
fn a_local_folder_ranks_before_the_index_and_oci_after_it() {
    let fake = index_with(&[("baz", &[]), ("qux", &[])]);
    let registry = Registry::start();
    let notes = [
        ("org.opencontainers.image.title", "qux"),
        ("org.opencontainers.image.version", "2.0.0"),
    ];
    registry.seed_package("team/qux", "2.0.0", &notes, b"qux 2.0.0");
    let folder = local_folder("baz", "1.0.0");
    let dir = tempfile::tempdir().unwrap();
    let mut setup = Setup::new(&fake, dir.path());
    setup.local = vec![folder.path().to_path_buf()];
    let url = format!("oci://{}/team", registry.host());
    setup.oci = vec![OciFeed {
        name: "team".to_string(),
        url: lvpm_core::oci::FeedUrl::parse(&url).unwrap(),
        pinned: vec!["qux".to_string()],
    }];
    let read = setup.read(&["baz", "qux"]);
    assert_eq!(read.failed, []);
    assert_eq!(
        sources_of(&read),
        [
            pair("baz", &folder.path().to_string_lossy()),
            pair("qux", "github-index:o/r")
        ]
    );
}

#[test]
fn the_index_ranks_among_hosted_folders_by_its_line_in_sources() {
    let fake = index_with(&[("foo", &[])]);
    let (_first, a) = hosted_listing("foo", "");
    let (_second, c) = hosted_listing("foo", "");
    let feed_url = |folder: &str| format!("{}/index.vipr", folder.trim_end_matches('/'));
    let dir = tempfile::tempdir().unwrap();
    let mut setup = Setup::new(&fake, dir.path());
    let index = Remote::Index(index_feed());

    setup.remotes = vec![
        Remote::Hosted(a.clone()),
        index.clone(),
        Remote::Hosted(c.clone()),
    ];
    assert_eq!(
        sources_of(&setup.read(&["foo"])),
        [pair("foo", &feed_url(&a))]
    );

    setup.remotes = vec![
        Remote::Hosted(c.clone()),
        index.clone(),
        Remote::Hosted(a.clone()),
    ];
    assert_eq!(
        sources_of(&setup.read(&["foo"])),
        [pair("foo", &feed_url(&c))]
    );

    setup.remotes = vec![index, Remote::Hosted(a)];
    assert_eq!(
        sources_of(&setup.read(&["foo"])),
        [pair("foo", "github-index:o/r")]
    );
}
