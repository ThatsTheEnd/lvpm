//! The GitHub client of `lvpm-core` (listing, cache, error mapping, token
//! safety) against `GitHubFake`. Rust tests, not harness scenarios. They live
//! here because `lvpm-testkit` depends on `lvpm-core`: a dev-dependency back
//! would be a cycle.

use lvpm_core::cache::Cache;
use lvpm_core::cancel::CancelToken;
use lvpm_core::errors::{GitHubError, GitHubFault};
use lvpm_core::fetch;
use lvpm_core::github::{GitHubAuth, Listing, download_error, releases, releases_cached};
use lvpm_core::index::{self, Entry};
use lvpm_testkit::github::{DOWNLOAD_HOST, FakeRelease, GitHubFake, PRIVATE_TOKEN};
use reqwest::blocking::Client;

const REPO: &str = "o/r";

fn list(fake: &GitHubFake, name: &str) -> Result<Listing, GitHubError> {
    releases(
        &Client::new(),
        &fake.api_url(),
        REPO,
        name,
        &GitHubAuth::anonymous(),
    )
}

fn found(listing: &Listing) -> Vec<(&str, &str, &str)> {
    listing
        .files
        .iter()
        .map(|file| {
            (
                file.version.raw.as_str(),
                file.file.as_str(),
                file.tag.as_str(),
            )
        })
        .collect()
}

#[test]
fn the_release_file_is_the_lvpkg_else_the_vip_named_for_the_package() {
    let fake = GitHubFake::start(&[(
        REPO,
        vec![
            FakeRelease::new("v1.0")
                .asset("pkg-1.0.0.4.vip", b"vip")
                .asset("pkg-1.0.0.4.lvpkg", b"lvpkg")
                .asset("notes.txt", b"notes"),
            FakeRelease::new("v0.9").asset("pkg-0.9.0.1.vip", b"vip"),
            FakeRelease::new("v0.8").asset("other-0.8.lvpkg", b"other"),
        ],
    )]);

    let listing = list(&fake, "pkg").unwrap();

    assert_eq!(
        found(&listing),
        [
            ("1.0.0.4", "pkg-1.0.0.4.lvpkg", "v1.0"),
            ("0.9.0.1", "pkg-0.9.0.1.vip", "v0.9")
        ]
    );
    assert_eq!(
        listing.files[0].url,
        format!(
            "{}/repos/o/r/releases/assets/{}",
            fake.api_url(),
            listing.files[0].url.rsplit('/').next().unwrap()
        ),
        "the asset's API URL, which redirects to its bytes"
    );
}

#[test]
fn drafts_and_pre_releases_are_not_versions_and_are_counted() {
    let fake = GitHubFake::start(&[(
        REPO,
        vec![
            FakeRelease::new("v2.0-draft")
                .asset("pkg-2.0.lvpkg", b"x")
                .draft(),
            FakeRelease::new("v2.0-rc")
                .asset("pkg-2.0.lvpkg", b"x")
                .prerelease(),
            FakeRelease::new("v1.0").asset("pkg-1.0.lvpkg", b"x"),
        ],
    )]);

    let listing = list(&fake, "pkg").unwrap();

    assert_eq!(found(&listing), [("1.0", "pkg-1.0.lvpkg", "v1.0")]);
    assert_eq!(listing.skipped, 2);
}

#[test]
fn every_page_is_read() {
    let all = (0..150)
        .map(|n| FakeRelease::new(&format!("v{n}")).asset(&format!("pkg-{n}.0.lvpkg"), b"x"))
        .collect();
    let fake = GitHubFake::start(&[(REPO, all)]);

    let listing = list(&fake, "pkg").unwrap();

    assert_eq!(listing.files.len(), 150);
    assert_eq!(
        fake.requests().len(),
        2,
        "100 on the first page, 50 on the second"
    );
}

#[test]
fn the_same_version_in_two_releases_fails_naming_both_tags() {
    let fake = GitHubFake::start(&[(
        REPO,
        vec![
            FakeRelease::new("a").asset("pkg-1.0.lvpkg", b"x"),
            FakeRelease::new("b").asset("pkg-1.0.lvpkg", b"y"),
        ],
    )]);

    let error = list(&fake, "pkg").unwrap_err();

    assert_eq!(
        error.to_string(),
        "pkg (github.com/o/r): version 1.0 is in two releases, a and b; delete one of the files"
    );
    assert!(matches!(error.fault, GitHubFault::TwoReleases { .. }));
}

#[test]
fn versions_that_compare_equal_collide_and_ones_that_differ_do_not() {
    let two = |first: &str, second: &str| {
        let fake = GitHubFake::start(&[(
            REPO,
            vec![
                FakeRelease::new("a").asset(first, b"x"),
                FakeRelease::new("b").asset(second, b"y"),
            ],
        )]);
        list(&fake, "pkg")
    };

    let error = two("pkg-1.0.lvpkg", "pkg-1.0.0.lvpkg").unwrap_err();
    assert_eq!(
        error.to_string(),
        "pkg (github.com/o/r): version 1.0.0 is in two releases, a and b; delete one of the files"
    );
    assert_eq!(
        two("pkg-1.0.lvpkg", "pkg-1.0.1.lvpkg").unwrap().files.len(),
        2
    );
}

#[test]
fn a_cached_listing_is_read_without_a_request_until_refreshed() {
    let fake = GitHubFake::start(&[(
        REPO,
        vec![FakeRelease::new("v1.0").asset("pkg-1.0.lvpkg", b"x")],
    )]);
    let cache_dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(cache_dir.path()).unwrap();
    let client = Client::new();
    let auth = GitHubAuth::anonymous();
    let api = fake.api_url();
    let cached = |name: &str, refresh: bool| {
        releases_cached(&cache, &client, &api, REPO, name, &auth, refresh).unwrap()
    };

    let first = cached("pkg", false);
    assert!(!first.from_cache);
    assert_eq!(fake.requests().len(), 1);

    fake.add_release(REPO, FakeRelease::new("v1.1").asset("pkg-1.1.lvpkg", b"y"));
    let second = cached("pkg", false);
    assert!(second.from_cache);
    assert_eq!(found(&second), found(&first), "the new release is not seen");
    assert_eq!(fake.requests().len(), 1, "no request");

    let other_package = cached("other", false);
    assert!(
        other_package.from_cache,
        "the listing is the repo's, not the package's"
    );

    let refreshed = cached("pkg", true);
    assert!(!refreshed.from_cache);
    assert_eq!(refreshed.files.len(), 2);
    assert_eq!(fake.requests().len(), 2);
    assert_eq!(
        cached("pkg", false).files.len(),
        2,
        "the refresh is what is cached now"
    );
}

#[test]
fn a_cached_listing_that_fails_is_asked_for_again_so_a_fix_is_seen() {
    let fake = GitHubFake::start(&[(
        REPO,
        vec![
            FakeRelease::new("a").asset("pkg-1.0.lvpkg", b"x"),
            FakeRelease::new("b").asset("pkg-1.0.lvpkg", b"y"),
        ],
    )]);
    let cache_dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(cache_dir.path()).unwrap();
    let (client, auth, api) = (Client::new(), GitHubAuth::anonymous(), fake.api_url());
    let cached = || releases_cached(&cache, &client, &api, REPO, "pkg", &auth, false);

    assert!(cached().is_err());
    fake.delete_release(REPO, "b");

    assert_eq!(cached().unwrap().files.len(), 1);
}

#[test]
fn a_failed_listing_is_not_cached_and_names_the_github_error() {
    let fake = GitHubFake::start(&[(REPO, vec![])]);
    fake.fail_with(502);
    let cache_dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(cache_dir.path()).unwrap();
    let client = Client::new();
    let auth = GitHubAuth::anonymous();
    let api = fake.api_url();

    let error = releases_cached(&cache, &client, &api, REPO, "pkg", &auth, false).unwrap_err();

    assert!(error.downcast_ref::<GitHubError>().is_some(), "{error:#}");
    fake.clear_requests();
    let again = releases_cached(&cache, &client, &api, REPO, "pkg", &auth, false);
    assert!(again.is_err());
    assert_eq!(fake.requests().len(), 1, "asked again: nothing was cached");
}

/// A token no text may ever hold.
const SECRET: &str = "ghp_do-not-print-this-token";
const RESET_2099: u64 = 4_070_908_800;

/// One row of the outline "an API failure maps to its text and is not
/// retried": how the fake is set up, the token sent, the text that follows
/// `pkg (github.com/o/r): `. `<in>` stands for the relative time.
struct Failure {
    case: &'static str,
    answer: fn(&GitHubFake),
    token: Option<&'static str>,
    text: &'static str,
}

fn private_repo(fake: &GitHubFake) {
    fake.make_private(REPO);
}

fn no_repo(_: &GitHubFake) {}

const FAILURES: &[Failure] = &[
    Failure {
        case: "403 with no requests remaining, no token",
        answer: |fake| fake.limit_rate(403, RESET_2099),
        token: None,
        text: "GitHub's rate limit is used up until 2099-01-01T00:00:00Z (in <in>); run gh auth login or set GH_TOKEN",
    },
    Failure {
        case: "429 with no requests remaining, no token",
        answer: |fake| fake.limit_rate(429, RESET_2099),
        token: None,
        text: "GitHub's rate limit is used up until 2099-01-01T00:00:00Z (in <in>); run gh auth login or set GH_TOKEN",
    },
    Failure {
        case: "403 with no requests remaining, token sent",
        answer: |fake| fake.limit_rate(403, RESET_2099),
        token: Some(SECRET),
        text: "GitHub's rate limit for the token from GH_TOKEN is used up until 2099-01-01T00:00:00Z (in <in>)",
    },
    Failure {
        case: "403 with requests remaining",
        answer: |fake| fake.forbid("Resource not accessible by integration"),
        token: None,
        text: "GitHub refused access (Resource not accessible by integration)",
    },
    Failure {
        case: "404, no token",
        answer: private_repo,
        token: None,
        text: "not found. If it is private, run gh auth login or set GH_TOKEN",
    },
    Failure {
        case: "404, token sent",
        answer: no_repo,
        token: Some(SECRET),
        text: "not found, or the token from GH_TOKEN cannot see it",
    },
    Failure {
        case: "401, token sent",
        answer: private_repo,
        token: Some(SECRET),
        text: "GitHub rejected the token from GH_TOKEN; unset it or run gh auth login",
    },
    Failure {
        case: "502",
        answer: |fake| fake.fail_with(502),
        token: None,
        text: "cannot read the releases; check your network and try again: HTTP 502",
    },
    Failure {
        case: "200 with a body that is not JSON",
        answer: |fake| fake.answer_not_json(),
        token: None,
        text: "cannot read the releases; check your network and try again: expected value at line 1 column 1",
    },
];

fn auth_with(token: Option<&'static str>) -> GitHubAuth {
    GitHubAuth::new(
        move |name| {
            (name == "GH_TOKEN")
                .then(|| token.map(str::to_string))
                .flatten()
        },
        || None,
    )
}

fn assert_text(got: &str, want: &str, case: &str) {
    match want.split_once("<in>") {
        Some((before, after)) => {
            assert!(
                got.starts_with(before) && got.ends_with(after),
                "{case}: {got}"
            )
        }
        None => assert_eq!(got, want, "{case}"),
    }
}

#[test]
fn an_api_failure_maps_to_its_text_is_not_retried_and_never_holds_the_token() {
    for row in FAILURES {
        let fake = GitHubFake::start(&[]);
        (row.answer)(&fake);

        let error = releases(
            &Client::new(),
            &fake.api_url(),
            REPO,
            "pkg",
            &auth_with(row.token),
        )
        .unwrap_err();

        let want = format!("pkg (github.com/o/r): {}", row.text);
        assert_text(&error.to_string(), &want, row.case);
        assert_eq!(
            fake.requests().len(),
            1,
            "{}: one request, no retry",
            row.case
        );
        for shown in [
            error.to_string(),
            format!("{error:?}"),
            format!("{error:#?}"),
        ] {
            assert!(!shown.contains(SECRET), "{}: {shown}", row.case);
        }
        let sent = fake.requests()[0].authorization.clone();
        assert_eq!(
            sent,
            row.token.map(|token| format!("Bearer {token}")),
            "{}",
            row.case
        );
    }
}

/// A server that answers its first request with what `reply` makes of its own
/// API URL, and says what it was asked. A client that hangs up early is no
/// failure of the server.
fn answer_once(
    reply: impl FnOnce(&str) -> String + Send + 'static,
) -> (String, std::thread::JoinHandle<String>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let api = format!("http://{}", listener.local_addr().unwrap());
    let own_api = api.clone();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
            head.push(byte[0]);
        }
        stream.write_all(reply(&own_api).as_bytes()).ok();
        String::from_utf8_lossy(&head).into_owned()
    });
    (api, server)
}

/// `body` as a 200 that then ends the connection, with `extra` header lines.
fn ok_reply(extra: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}",
        body.len()
    )
}

#[test]
fn a_next_page_off_the_api_is_refused_so_the_token_stays_on_the_api() {
    for link in [
        "http://other.example/steal",
        "http://127.0.0.1:1/same-host-another-port",
        "https://127.0.0.1/same-host-another-scheme",
        "/repos/o/r/releases?page=2",
    ] {
        let (api, server) = answer_once(move |_| {
            ok_reply(
                &format!(
                    "Link: <{link}>; rel=\"next\"
"
                ),
                "[]",
            )
        });

        let error =
            releases(&Client::new(), &api, REPO, "pkg", &auth_with(Some(SECRET))).unwrap_err();

        assert_eq!(
            error.to_string(),
            format!(
                "pkg (github.com/o/r): cannot read the releases; check your network and try again: a next page outside {api}: {link}"
            )
        );
        let asked = server.join().unwrap();
        assert!(asked.contains(&format!("Bearer {SECRET}")), "{asked}");
    }
}

#[test]
fn a_file_whose_url_is_off_the_api_is_refused_so_the_token_stays_on_the_api() {
    let body = r#"[{"tag_name":"v1","assets":[{"name":"pkg-1.0.lvpkg","url":"http://other.example/steal"}]}]"#;
    let (api, server) = answer_once(move |_| ok_reply("", body));

    let error = releases(&Client::new(), &api, REPO, "pkg", &auth_with(Some(SECRET))).unwrap_err();

    assert_eq!(
        error.to_string(),
        format!(
            "pkg (github.com/o/r): cannot read the releases; check your network and try again: an asset URL outside {api}: http://other.example/steal"
        )
    );
    server.join().unwrap();
}

#[test]
fn a_page_larger_than_32_mib_is_refused() {
    let (api, server) = answer_once(|_| ok_reply("", &" ".repeat(32 * 1024 * 1024 + 1)));

    let error = releases(&Client::new(), &api, REPO, "pkg", &auth_with(None)).unwrap_err();

    assert_eq!(
        error.to_string(),
        "pkg (github.com/o/r): cannot read the releases; check your network and try again: a response of more than 33554432 bytes"
    );
    server.join().unwrap();
}

#[test]
fn at_most_50_pages_are_read() {
    let releases_of = |n: usize| {
        (0..n)
            .map(|n| FakeRelease::new(&format!("v{n}")))
            .collect::<Vec<_>>()
    };
    let fake = GitHubFake::start(&[(REPO, releases_of(5_000))]);
    list(&fake, "pkg").unwrap();
    assert_eq!(fake.requests().len(), 50);

    let fake = GitHubFake::start(&[(REPO, releases_of(5_001))]);
    let error = list(&fake, "pkg").unwrap_err();
    assert_eq!(
        error.to_string(),
        "pkg (github.com/o/r): cannot read the releases; check your network and try again: more than 50 pages of releases"
    );
    assert_eq!(fake.requests().len(), 50);
}

#[test]
fn an_unreachable_api_fails_naming_the_package_and_its_cause_and_never_the_token() {
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let api = format!("http://{}", closed.local_addr().unwrap());
    drop(closed);

    let error = releases(&Client::new(), &api, REPO, "pkg", &auth_with(Some(SECRET))).unwrap_err();

    let text = error.to_string();
    let head = "pkg (github.com/o/r): cannot read the releases; check your network and try again: ";
    assert!(text.starts_with(head) && text.len() > head.len(), "{text}");
    assert!(
        !text.contains(SECRET) && !format!("{error:?}").contains(SECRET),
        "{text}"
    );
}

const OCTET_STREAM: &str = "application/octet-stream";

/// The entry the lock would build for `pkg`'s one file in `REPO`: its URL is the
/// asset's API URL, and its source says it is GitHub's.
fn github_entry(fake: &GitHubFake) -> Entry {
    let file = list(fake, "pkg").unwrap().files.remove(0);
    fake.clear_requests();
    Entry {
        name: "pkg".into(),
        version: file.version,
        url: file.url,
        md5: None,
        display_name: None,
        requires: Vec::new(),
        lv_min: None,
        source: index::github_source(REPO),
    }
}

/// The bytes of `entry`, fetched with `token` into a cache of its own.
fn download(entry: &Entry, token: Option<&str>) -> anyhow::Result<Vec<u8>> {
    let cache_dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(cache_dir.path()).unwrap();
    fetch::package(&cache, &Client::new(), entry, || token, &CancelToken::new())
}

fn one_file() -> GitHubFake {
    GitHubFake::start(&[(
        REPO,
        vec![FakeRelease::new("v1.0").asset("pkg-1.0.lvpkg", b"the package")],
    )])
}

#[test]
fn a_github_file_is_downloaded_with_the_token_on_the_api_host_only() {
    let fake = one_file();
    let entry = github_entry(&fake);

    let bytes = download(&entry, Some(SECRET)).unwrap();

    assert_eq!(bytes, b"the package");
    let seen = fake.requests();
    assert_eq!(
        seen.len(),
        2,
        "the API, then where it redirected to: {seen:?}"
    );
    let (api, redirected) = (&seen[0], &seen[1]);
    assert!(
        api.path.starts_with("/repos/o/r/releases/assets/"),
        "{api:?}"
    );
    assert_eq!(api.accept.as_deref(), Some(OCTET_STREAM));
    assert_eq!(api.authorization, Some(format!("Bearer {SECRET}")));
    assert!(redirected.path.starts_with("/download/"), "{redirected:?}");
    assert!(
        redirected
            .host
            .as_deref()
            .unwrap()
            .starts_with(DOWNLOAD_HOST),
        "another host than the API's: {redirected:?}"
    );
    assert_eq!(
        redirected.authorization, None,
        "the token left with the redirect"
    );
}

#[test]
fn a_github_file_is_downloaded_without_a_token_asking_for_the_bytes() {
    let fake = one_file();
    let entry = github_entry(&fake);

    assert_eq!(download(&entry, None).unwrap(), b"the package");

    let seen = fake.requests();
    assert_eq!(seen[0].accept.as_deref(), Some(OCTET_STREAM));
    assert!(
        seen.iter().all(|request| request.authorization.is_none()),
        "{seen:?}"
    );
}

#[test]
fn a_private_file_needs_the_token_to_download() {
    let fake = one_file();
    let entry = github_entry(&fake);
    fake.make_private(REPO);

    assert_eq!(
        download(&entry, Some(PRIVATE_TOKEN)).unwrap(),
        b"the package"
    );
    let failed = download(&entry, None).unwrap_err();
    let shown = download_error(failed, "pkg", REPO, &GitHubAuth::anonymous());
    assert_eq!(
        shown.to_string(),
        "pkg (github.com/o/r): not found. If it is private, run gh auth login or set GH_TOKEN"
    );
}

#[test]
fn a_rate_limited_download_says_when_the_limit_resets_and_how_to_raise_it() {
    for status in [403, 429] {
        let fake = one_file();
        let entry = github_entry(&fake);
        fake.limit_rate(status, RESET_2099);

        let failed = download(&entry, None).unwrap_err();
        let shown = download_error(failed, "pkg", REPO, &GitHubAuth::anonymous());

        let want = "pkg (github.com/o/r): GitHub's rate limit is used up until 2099-01-01T00:00:00Z (in <in>); run gh auth login or set GH_TOKEN";
        assert_text(&shown.to_string(), want, &status.to_string());
    }
}

#[test]
fn a_feed_file_is_downloaded_with_no_token_and_no_octet_stream_accept() {
    let fake = one_file();
    let github = github_entry(&fake);
    let id = github.url.rsplit('/').next().unwrap();
    let feed = Entry {
        url: format!("{}/download/{id}", fake.api_url()),
        source: "http://feed.example/index.json".into(),
        ..github
    };

    // A token is at hand, but a feed entry never gets it.
    assert_eq!(download(&feed, Some("t0ken")).unwrap(), b"the package");

    let seen = fake.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].authorization, None);
    assert_ne!(seen[0].accept.as_deref(), Some(OCTET_STREAM), "{seen:?}");
}
