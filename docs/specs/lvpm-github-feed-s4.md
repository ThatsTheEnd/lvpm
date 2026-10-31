<!-- spec-version: 14.0.1 -->
# Spec: GitHub Releases as a feed (slice S4)

**Format:** dev-team specs 14.0.1 (validate mode, from the slice issue) · **Status:** gate passed; the four stakeholder answers were given by the user on 2026-10-31 · **Date:** 2026-10-31 ·
**Slice:** [#62](https://github.com/ThatsTheEnd/lvpm/issues/62), [S4 in build-slices.md](../vision/build-slices.md#slice-s4-github-releases-as-a-feed) ·
**Rules:** [ADR 4](../adr/0004-sources-first-wins-and-pinning.md) decision 6, [ADR 6](../adr/0006-lvpkg-only-vip-read-only.md) decision 2

## Intent Description

An open-source LabVIEW library on GitHub needs no registry. Its maintainer
tags a release, CI runs `lvpm build` and uploads the `.lvpkg`, and any project
can depend on it with one line:

```toml
labview_open_source_lib_serializer = { github = "LabVIEW-Open-Source/Serializer", version = ">=1.2" }
```

That package then comes from that repo's releases and nowhere else. It is
never looked up in `[sources]`, so first-wins (ADR 4, decision 1) does not
apply to it. Releases VIPM built (a `.vip`) install too, so existing GitHub
releases work today. The file's SHA-256 is pinned in the lock the first time,
and a file re-uploaded with other bytes fails `sync`. Private repos work with
the user's GitHub token, which lvpm reads from the environment or `gh` and
never writes anywhere.

## Architecture Specification

**The manifest line.** S4 adds the per-dependency table that ADR 4 shares
between slices (S12 `hooks`, S7c `index`). S4 is the first to land, so it adds
the parser and accepts `version` and `github`:

- `version` is a constraint as in the short form. It is required in the table form.
- `github = "owner/repo"`: exactly one `/`, both parts non-empty.
- `github` together with `index` fails when the manifest is read, naming the
  package (ADR 4, decision 6). `index` alone fails as not supported yet (S7c adds it).
- Any other key fails, naming the package and the key.
- `[dev-dependencies]` takes the same table form.

**Versions from file names, not tags** (user, 2026-10-31). lvpm lists the
repo's releases. In each release, the package file is the asset named
`<name>-<version>.lvpkg`, where `<name>` is the dependency's key compared with
`project::name_key`. If the release has no such `.lvpkg`, the asset named
`<name>-<version>.vip` is used. `<version>` is read from the file name and is
the version lvpm resolves and locks. The tag only groups the files. This is
the name `lvpm build` writes (`dist/<name>-<version>.lvpkg`) and the name VIPM
writes. Both existing releases fit: pilot B's tag `1.2.1` holds
`labview_open_source_lib_serializer-1.2.1.16.vip`, and ZAlign's `v1.0.0` holds
`zuehlke_engineering_lib_zalign-1.0.0.4.vip`. A release without a matching
asset is skipped. When two releases hold the same version, resolution fails
and names both tags.

- Drafts and pre-releases are skipped (user, 2026-10-31).
- After download, the name and version inside the package must equal the file
  name's. If they differ, lvpm fails and installs nothing.
- The package's own `requires` resolve through the normal source order. A
  transitive dependency that should also come from GitHub is listed as a
  direct dependency (ADR 4, consequences).

**The GitHub API.** lvpm calls `GET /repos/{owner}/{repo}/releases` and
follows the `Link` pages. It downloads the asset through its API URL with
`Accept: application/octet-stream`, which works for public and private repos.
The base URL is `https://api.github.com`, or `GITHUB_API_URL` when that is set.
GitHub Actions sets that variable, and the loopback fake in the scenarios uses
it. The release list is cached like a feed index. `lock`, `add` and `remove`
read the cache and list again only when it lacks what the run needs;
`--refresh` and an upgrade always list again, and `sync` downloads only from a
fresh listing.

**Tokens and rate limits** (user, 2026-10-31). The token is the first of
`GH_TOKEN`, `GITHUB_TOKEN`, then the output of `gh auth token`. If none exists,
lvpm calls the API anonymously (60 requests an hour). lvpm sends the token
only to the API host and never writes it to `lvpm.toml`, the lock, the cache,
logs or error text. On a rate-limit answer (a 403 with
`x-ratelimit-remaining: 0` or a `retry-after`, or any 429), lvpm fails at once. The message names the repo
and the reset time, and says to run `gh auth login` or to set `GH_TOKEN`.
lvpm never waits and retries.

**The lock.**
- The package's `source` is `https://github.com/owner/repo`.
- `sha256` is set the first time the package is locked. As today, `sync`
  finds the archive by that hash: the cache first, then the package's GitHub
  repo. Other bytes fail the whole sync (`fetch_locked`).
- A new optional `[inputs.github]` table maps a name to `owner/repo`, so a
  changed or removed `github` value makes the lock stale. No format bump:
  locks without GitHub dependencies are unchanged.

**The CI template.** `.github/workflow-templates/lvpm-release.yml` runs on a
pushed tag on `ubuntu-latest` (`lvpm build` needs no LabVIEW). It downloads a
released lvpm, runs `lvpm build`, and uploads `dist/*.lvpkg` to the tag's
release with `gh release upload`, using the workflow's `GITHUB_TOKEN`. It is a
file to copy, documented in docs/usage.md. It is not an org-level template.

| Part | Where |
|---|---|
| Table form, `github`, refusals | `crates/lvpm-core/src/project.rs` (`parse_dependencies`) |
| Release listing, asset choice, token, rate-limit error | new `crates/lvpm-core/src/github.rs` (the slice lists `index.rs`, and it is the closest existing home); entries enter resolution as an `Index` limited to that one name |
| Download with token, name and version check | `crates/lvpm-core/src/fetch.rs` |
| `source`, `[inputs.github]`, staleness | `crates/lvpm-core/src/lockfile.rs` |
| Sync looks in the package's repo | `crates/lvpm-core/src/ops/sync.rs` (`Source::all`) |
| Fake releases API on `127.0.0.1` | `crates/lvpm-testkit/src/` next to `feed.rs` |
| Scenarios | `tests/features/build-slices/s4-github-{manifest,lock,sync,docs}.feature` (new), listed in `tests/acceptance.rs` |
| CI template | `.github/workflow-templates/lvpm-release.yml` |
| Docs | docs/manifest.md (the table form), docs/usage.md (GitHub dependencies, tokens, the template) |

**Unchanged.** `[sources]`, first-wins, `lvpm add` (it does not write
`github`; edit the manifest), folder and index feeds.

**Out of scope.** Attestations (S6), the community index (S8), GitHub
Enterprise Server outside Actions, `lvpm add --github`.

## Acceptance Criteria

Each criterion cites the issue text ([#62](https://github.com/ThatsTheEnd/lvpm/issues/62)) or names the decision it comes from.

1. **Install from a release.** With the fake releases API serving pilot B's
   release, a project with the `github` line for pilot B runs `lvpm lock` and
   `lvpm sync`. Pilot B is installed, and its dependency
   `labview_open_source_lib_data_manipulation` comes from `[sources]`. The lock
   has `source = "https://github.com/LabVIEW-Open-Source/Serializer"` and the
   file's SHA-256.
   *Source: "A project installs pilot B from a GitHub release (scenario against a loopback fake of the releases API)", "The lock's `source` field names the GitHub feed; the file's SHA-256 is pinned on first lock".*
2. **`.lvpkg` first, `.vip` second.** A release with both
   `<name>-<version>.lvpkg` and `<name>-<version>.vip` installs the `.lvpkg`.
   A release with only the `.vip` installs the `.vip`. A release with neither
   is not a version, and other assets are ignored.
   *Source: "the release file is `*.lvpkg` … with `*.vip` as a fallback"; naming per the user's answer.*
3. **Version from the file name.** Tag `1.2.1` with asset
   `…-1.2.1.16.vip` locks version `1.2.1.16`. `version = ">=1.2.2"` does not
   admit it, and resolution fails naming the package and the repo. Drafts and
   pre-releases are never picked.
   *Inference from the user's answers (naming, pre-releases).*
4. **Re-upload is caught.** After a lock, the fake serves other bytes under
   the same asset name. `lvpm sync` and `lvpm sync --locked` both fail, name
   the package and both hashes, and install nothing. The cache is empty first,
   so the check is against the download.
   *Source: "A re-uploaded release file with a different hash fails `sync` (with or without `--locked`), and nothing is installed."*
5. **Contents must match the name.** A file named for version `1.2.1.16`
   that holds another name or version fails, and nothing is installed.
   *Inference (security): otherwise one repo could ship any package under any name.*
6. **`github` with `index` fails.** The line
   `x = { github = "o/r", index = "internal", version = "*" }` fails when the
   manifest is read. The message names `x`. A table with an unknown key, a
   missing `version`, or a `github` value that is not `owner/repo` fails the
   same way, naming the package.
   *Source: "A line with both `github` and `index` fails when the manifest is read, naming the package." The other refusals are inference.*
7. **Never looked up in `[sources]`.** A `[sources]` feed that lists the same
   name in a higher version is not used. If the repo has no fitting version,
   resolution fails and does not fall back.
   *Source: "a direct source for that one package, never looked up in `[sources]` (ADR 4, decision 6)".*
8. **Private repos with a token, never stored.** The fake demands
   `Authorization` for a private repo. With `GH_TOKEN` set, lock and sync
   succeed. After the run, the token appears in none of `lvpm.toml`,
   `lvpm.lock`, the cache directory or the command's output. The precedence is
   `GH_TOKEN`, `GITHUB_TOKEN`, `gh auth token` (unit test).
   *Source: "Private repos work with the token from `gh auth token`; no token is ever written to `lvpm.toml` or the lock." The env vars come from the user's answer.*
9. **Token stays on the API host.** When the asset download redirects to
   another host, that request carries no `Authorization` header.
   *Inference (security): GitHub redirects asset downloads to a storage host.*
10. **Rate limit fails with a hint.** Without a token, a fake that answers 403
    with `x-ratelimit-remaining: 0` and `x-ratelimit-reset` makes `lock` fail
    at once. The message names the repo and the reset time, and mentions
    `gh auth login` and `GH_TOKEN`. There is no retry.
    *User's answer (rate limits).*
11. **The lock follows the manifest.** Changing or removing a `github` value
    makes `lvpm lock --check` report the lock as stale. A lock with no GitHub
    dependency is byte-identical to today's.
    *Inference: a lock that does not notice a changed source would install from the old repo.*
12. *(manual)* **The template publishes.** In
    [Zuehlke/ZAlign](https://github.com/Zuehlke/ZAlign), with an `lvpm.toml`
    and the copied template, pushing a tag makes CI attach
    `zuehlke_engineering_lib_zalign-<version>.lvpkg` to the release. A second
    project then installs it from the `github` line. Its existing `v1.0.0`
    release (VIPM-built `.vip` only) also installs, which shows the fallback
    against the real API.
    *Source: "*(manual)* The CI template publishes a release from a tag in a pilot B fork". The repo was changed to ZAlign by the user.*
13. **Docs.** docs/manifest.md describes the table form and its refusals.
    docs/usage.md covers `github` dependencies, the token chain, rate limits
    and how to copy the template.
    *Inference: build-slices.md requires docs for new behaviour.*

## Glossary

| Term | Definition | Status | Source |
|------|------------|--------|--------|
| GitHub dependency | A dependency line with `github = "owner/repo"`. Its package comes only from that repo's releases. | `verified` | ADR 4 decision 6; user answers 2026-10-31 |
| release file | The asset of a release named `<name>-<version>.lvpkg`, or `.vip` when there is no `.lvpkg`. | `verified` | user, 2026-10-31 |
| version (of a GitHub dependency) | The `<version>` in the release file's name, not the tag. | `verified` | user, 2026-10-31 |
| table form | A dependency value that is a TOML table (`{ version = …, … }`) rather than a string. | `unverified` | ADR 4 consequences ("the per-dependency table") |

## Ambiguity Log

| Decision | Classification | Resolved By | Rationale / Answer |
|----------|---------------|-------------|-------------------|
| How the release file is named (the slice asks /specs to decide) | `requires-stakeholder-input` | user, 2026-10-31 | `<name>-<version>.lvpkg`, else `.vip`; the version is read from the file name. The alternatives (version from the tag with any one file; tag that must equal the package version) were declined. The first locks pilot B as `1.2.1` while the package says `1.2.1.16`. The second rejects both existing repos. |
| Rate limits without a token (the slice asks /specs to decide) | `requires-stakeholder-input` | user, 2026-10-31 | Token chain `GH_TOKEN` → `GITHUB_TOKEN` → `gh auth token`, else anonymous. A rate limit fails at once with the reset time and a hint, and the release list is cached. Declined: wait and retry; always require a token. |
| Drafts and pre-releases | `requires-stakeholder-input` | user, 2026-10-31 | Both skipped. Declined: pre-releases count. |
| Repo for the manual demo | `requires-stakeholder-input` | user, 2026-10-31 | ZAlign (owned by the user's team, public, no fork needed, already has a VIPM `.vip`). Pilot B stays in the automated scenario, where its dependency is checked. Declined: a pilot B fork; ZAlign everywhere. |
| Tag vs. package version | `inferable` | follows from the naming answer | The file name decides. A package whose contents differ from its file name fails (AC5). Check passed: the alternative "trust the contents and ignore the file name" is ruled out, because resolution must list versions without downloading. |
| Lock `source` value | `inferable` | inference (user's standing rule: small decisions alone) | `https://github.com/owner/repo`: a URL like every other feed source, and the page a reader opens. Check: alternative `github:owner/repo` is plausible. It is recorded and can be changed in review at no cost, because no lock holds either form yet. |
| How the lock notices a changed `github` value | `inferable` | inference | A new optional `[inputs.github]` table. Without it, a changed repo would not make the lock stale (AC11). Optional, so existing locks keep their bytes and the format number. |
| `index` key before S7c | `inferable` | inference | Recognised only to refuse it: together with `github` (AC6, required by the slice), and alone as "not supported yet". Check passed: silently ignoring `index` would drop a pin the user wrote. |
| Unknown keys in the table form | `inferable` | inference | Refused, naming the key. This is how `[sources]` and `project.labview` already refuse values they cannot read. Ignoring a key such as `hooks` before S12 would silently drop what the user asked for. |
| Transitive dependencies of a GitHub package | `inferable` | inference | They resolve through the normal source order. ADR 4's consequences say a pin applies only to a direct line. |
| Same version in two releases | `inferable` | inference | Fails, naming both tags. Picking one would make the lock depend on API order. |
| API base URL and the test seam | `inferable` | inference | `GITHUB_API_URL`, which Actions already sets (so Enterprise Server works inside Actions for free), else `https://api.github.com`. The scenarios need a seam, and this one adds no lvpm-only variable. |
| Asset download path | `inferable` | inference | The asset's API URL with `Accept: application/octet-stream`. `browser_download_url` does not work for private repos with a token. |
| Token on redirects | `inferable` | inference | Sent only to the API host (AC9). GitHub answers asset downloads with a redirect to a storage host, which must not see the token. |
| CI template runner | `inferable` | inference | `ubuntu-latest`: `lvpm build` involves no LabVIEW (`ops/build.rs`), and Linux runners are the cheapest. |
| `.github/workflow-templates/` meaning | `inferable` | inference | A file users copy, documented in usage.md. GitHub only offers org-level templates from an org's `.github` repo, which this repo is not. |
| `lvpm add` writing `github` | `inferable` | inference | Out of scope. The slice does not ask for it. Users edit the line by hand, and `add` on a table-form line keeps the other keys (ADR 4 consequences). |
| CRUD: GitHub dependency | `inferable` | inference | Created and changed by editing `lvpm.toml`. Read by lock, sync and info. Deleted by removing the line, which `lvpm remove` already does by name. Changing it makes the lock stale (AC11). |
| CRUD: release file | `inferable` | inference | Read-only for lvpm. It is created by the maintainer's CI (AC12). A replaced file is caught by the hash (AC4). A deleted release fails sync unless the cache holds the hash, as with any feed today. |
| Authentication | `inferable` | inference | The user's GitHub token from the platform chain (distribution-design.md: "Credentials come from the platform's own credential chain"). |
| Authorization | `inferable` | inference | GitHub's own. A repo the token cannot see fails like a missing repo, naming it. |
| Audit / logging | `inferable` | inference | The lock's `source` and `sha256` are the record. The token is never logged (AC8). |
| Error handling | `inferable` | inference | Unreachable API, 404 repo, rate limit, no fitting version: each fails naming the package and the repo, and nothing is installed. The existing lock/sync failure paths are reused. |
| Glossary: "table form" | `inferable` | inference | ADR 4 already uses the phrase ("the table form", "table-form line"). Left `unverified` because no human has confirmed it in this loop. It does not block. |

## Consistency Gate
- [x] Intent is unambiguous
- [x] Every behavior/goal maps to an acceptance criterion: one-line dependency → AC1, AC6; releases only, no `[sources]` → AC7; `.vip` releases → AC2, AC12; pinned hash → AC1, AC4; private repos and tokens → AC8–10; CI template → AC12; lock follows manifest → AC11
- [x] Architecture constrains without over-engineering (one parser, one new module, one optional lock table, one template file)
- [x] Terminology consistent across artifacts (GitHub dependency, release file, version)
- [x] No contradictions between artifacts
- [x] Every gap/ambiguity finding is logged. The four `requires-stakeholder-input` items were answered by the user.
