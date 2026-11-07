<!-- spec-version: 14.0.1 -->
# Spec: Federated community index (slice S8)

**Format:** dev-team specs 14.0.1 (validate mode, from the slice issue) · **Status:** gate passed; nine stakeholder answers given and the spec approved by the user on 2026-11-06 · **Date:** 2026-11-06 ·
**Slice:** [#69](https://github.com/ThatsTheEnd/lvpm/issues/69), [S8 in build-slices.md](../vision/build-slices.md#slice-s8-federated-community-index) ·
**Builds on:** [S4](lvpm-github-feed-s4.md) (GitHub Releases), [S6](lvpm-attestations-s6.md) (attestations), [ADR 4](../adr/0004-sources-first-wins-and-pinning.md) (first source wins)

## Intent Description

An open-source LabVIEW library on GitHub can already be a dependency through
`github = "owner/repo"` (S4), but only by someone who knows the repo. After
this slice, a library also has a **name** in a public community index, and a
project that lists the index installs it by that name alone.

The index is the repo
[`labview-package-manager/package-index`](https://github.com/labview-package-manager/package-index),
in the `labview-package-manager` organisation (user, 2026-11-06). It holds one
TOML file per package, `packages/<name>.toml`. The files themselves stay on
the owner's GitHub Releases. A maintainer joins by pushing a tag: the release
template builds and attests the package and opens a pull request on the
index. A bot in the index repo checks the name, the SHA-256 and the
attestation, then merges. No person has to approve, and nobody owns the
names: the first repo to register a name owns it, and only that repo's
attested releases can add versions.

uv's counterpart is PyPI with its Simple API: one page per project name,
read only for the names being resolved. Unlike PyPI, uploads are not
accounts and passwords: a version is accepted only with a GitHub attestation
from the repo that owns the name. Cargo's sparse crates.io index is the
closest model: one file per crate, changed by pull request.

## Architecture Specification

### Publisher journey

What a maintainer does to get a `.lvpkg` into the index, once. After that,
every tag adds a version with no further step:

1. The repo has an `lvpm.toml` with `[project]` and `[distribution]`, as
   `lvpm build` needs today.
2. Copy `lvpm-release.yml` into `.github/workflows/`.
3. Create a GitHub token that can fork and open pull requests on public repos,
   and save it as the repo secret `LVPM_INDEX_TOKEN`. A classic personal access
   token with the `public_repo` and `workflow` scopes is the tested choice. The
   `workflow` scope is needed because the PR branch starts at the index's tip:
   once the index has a workflow file the publisher's fork lacks, GitHub
   refuses the push of a token without it. A fine-grained token may not work.
   The docs also recommend a dedicated machine account for the token, and,
   optionally, a GitHub Environment with a deployment protection rule.
4. Push a tag.

What the maintainer then sees:

- The `register` job writes the index PR's link into the run's summary.
- The bot merges the PR when every rule passes. When a rule fails, the bot
  comments the refusal lines from `lvpm index check` on the PR and leaves
  it open. The comment says what to fix, for example "version 1.2.1.16
  already listed with another sha256: push a new version".

A consumer then adds two lines and nothing else:

```toml
[sources]
community = "github:labview-package-manager/package-index"

[dependencies]
labview_open_source_lib_serializer = ">=1.2"
```

uv's counterpart: `uv publish` with a PyPI trusted publisher, set up once on
PyPI's site. Here the setup is one secret and one copied workflow, and the
trust comes from the attestation, not from an account on the index.

### The source line

A project opts in by listing the index in `[sources]` (user, 2026-11-06). The
defaults (vipm.io, NI) and first-source-wins order (ADR 4) do not change:

```toml
[sources]
community = "github:labview-package-manager/package-index"
```

- `github:owner/repo` is a new source kind next to `http(s)://` (hosted
  folder) and `oci://` (registry) (user, 2026-11-06). `owner` and `repo` are
  non-empty and contain no `/`; anything else fails when the manifest is read,
  naming the source.
- A dependency can pin it with `index = "community"` (S7c), like any source.
- `lvpm search` does not list index packages in this slice (user,
  2026-11-06), because the index has no catalogue.

### Reading the index

- **One file per name** (user, 2026-11-06). For each package name it
  resolves, lvpm asks the index for `packages/<name>.toml`, where `<name>` is
  `project::name_key` of the name. A 404 means the index does not list the
  name, and the next source is asked. lvpm never lists the whole repo. A
  missing or forbidden index repo fails the source instead; the first 404 of
  an index in a run checks that the repo exists. Why a `github:` source takes
  part in first-source-wins, and what that costs, is in
  [ADR 8](../adr/0008-community-index-sources-take-part-in-first-source-wins.md).
- The file is read through lvpm's existing GitHub client: the contents API
  (`GET {api}/repos/{owner}/{repo}/contents/packages/{name}.toml`, raw media
  type), the API from `GITHUB_API_URL`, and the token chain of S4. So a private
  index works with the user's token, and tests point the client at a loopback
  fake.
- Any other failure (unreachable, 5xx, a file that does not parse) is a
  listing failure of that source, reported the way hosted folders report
  theirs today.
- The `Entry::source` of an index entry must not start with `github:`,
  because that prefix already marks a GitHub-release dependency
  (`index::github_repo`). The exact spelling is a plan decision.

### The entry file

`packages/<name>.toml` holds the name, the repo that owns it, and one table
per version. Every field of a version is what lvpm needs to resolve without
downloading the package:

```toml
name = "labview_open_source_lib_serializer"
github = "someone/Serializer"   # owns the name

[[version]]
version = "1.2.1.16"
url = "https://github.com/someone/Serializer/releases/download/1.2.1/labview_open_source_lib_serializer-1.2.1.16.lvpkg"
sha256 = "<hex>"
labview = ">=13.0"                                            # the package's LabVIEW minimum, if it has one
requires = ["labview_open_source_lib_data_manipulation >=1.0"] # the package's own requires
```

The exact key spellings of `labview` and `requires` are a plan decision. Each
field must be what lvpm itself reads out of the package (`entry_from_bytes`),
so `lvpm index add` writes it and `lvpm index check` can compare against it.

### Installing from the index

- An index package resolves like a hosted-folder entry: its `requires` go
  through the normal source order.
- After the download, lvpm checks the file's SHA-256 against the index entry
  **before** it uses the file. A mismatch fails, names the package, version
  and both hashes, and installs nothing. This is stronger than S4's trust on
  first use: the index pins the hash before the first download.
- **The consumer checks the attestation too** (user, 2026-11-06). At lock
  time an index package gets the same S6 check as a `github = "owner/repo"`
  dependency, against the entry's `github` repo. `[policy]` applies to it,
  and `signer` and `subject` are written into `lvpm.lock`.
- The lock records the index as the package's source, with its `sha256`
  (the lock has no `url` field). `sync --locked` reads the index again for the
  locked names, downloads the entry's `url` and checks the locked hash, as for
  every other source. `lockfile::FORMAT_VERSION` stays the same.

### Two new commands

ADR 5 applies to both: an operation in `crates/lvpm-core/src/ops/`, events in
`events.rs`, arguments in `src/main.rs`, output in `src/render/`. Each gets a
scenario and a line in the command table of `docs/usage.md`.

- **`lvpm index add <entries-dir> <package> --github owner/repo --url <url>`**
  reads the package, computes its SHA-256 and writes or extends
  `<entries-dir>/<name>.toml`. A new name gets a new file with `github` set. An
  existing file gets one `[[version]]` appended, if its `github` equals
  `--github`. A version that is already listed is refused. The release
  template runs it.
- **`lvpm index check --base <dir> --head <dir>`** is the bot. It compares two
  checkouts of the index repo (`main` and the PR) and needs no git. It
  downloads each new version and checks its attestation over the network, and
  it prints one line for each finding. The exit code is non-zero if any change
  is refused. It refuses:
  1. a changed file outside `packages/`, or a file whose name is not
     `<name_key(name)>.toml`;
  2. a changed `github` in an existing file;
  3. a changed or removed version, so a new SHA-256 for an existing version is
     refused, naming the package, version and both hashes;
  4. a new version whose `url` is not under
     `https://github.com/<github>/releases/download/`;
  5. a new version whose download has a different SHA-256, or a name, version,
     LabVIEW minimum or `requires` other than what the package itself says;
  6. a new version whose S6 outcome is not **attested**, or whose attestation
     subject is another repo than `github`. Matching ignores case;
  7. a new name that a public index (NI or JKI, the feeds `defaults = true`
     reads) lists, compared by name key. Only a name the base has no file for is
     asked; a new version of a listed name is not. If the public indexes cannot
     be read, or list no name at all, the check fails as an error, not as a
     verdict.

  Rule 8 is not a refusal. `--author <login>` is the PR's author. For each new
  version, and for each entry file that is in the base and the head with other
  bytes (even a reordered key or the case of `name`), the author must own the
  entry's `github = "owner/repo"` as the base has it: the login equals `owner`
  (ASCII case-insensitive), or `owner` is an organisation that lists the
  author as a public member (`GET /orgs/{owner}/public_members/{author}`: 204
  yes, 404 no, no token needed). Otherwise, and for every new version and
  rewritten file when there is no `--author`, the PR is left for a maintainer:
  not refused, not merged. A refusal wins over this.

  With `--json` the result has `command` (`index-check`), `accepted` and
  `refused` (counts), `verdict` (`merge`, `refuse` or `review`) and `comment`,
  the text the bot leaves on a PR that is refused or left for review (null for
  any other). `verdict` is `refuse` when anything is refused; else `review` when
  the author is not the owner of a version or of a rewritten file, or nothing
  was accepted; else `merge`.

  The check never runs code from the PR: it reads the PR's TOML as data.
  Attestation goes through the existing `verify` module and the `gh` runner,
  so tests stub `gh` as S6's tests do.

### Name ownership

First come, first served (user, 2026-11-06). The first merged file for a name
sets its `github` owner, and rule 2 keeps it. Proof of ownership is the
attestation: only a workflow in the owner repo can attest its releases.
vipm.io names are reserved for vipm.io (user, 2026-11-07): the index's
`[sources]` line ranks before the public defaults, so a name registered here
would be taken from vipm.io for every consumer. Rule 7 refuses a new name that
NI or JKI lists. A name vipm.io adds later is not removed from the index. Only
the owner can list: rule 8 leaves a PR from another account for a maintainer.
Two PRs that add the same new name conflict on the same file, so only the first
can merge.

### The index repo

Created during build in `labview-package-manager` (user, 2026-11-06). The
user is asked before each push to it. It holds:

- `packages/`, empty at first;
- a README for maintainers: how to register, the rules above, and how to
  set `LVPM_INDEX_TOKEN`;
- `.github/workflows/check.yml`, the bot. It runs on `pull_request_target` for
  PRs to `main`, in two jobs. `check` has a read-only `GITHUB_TOKEN`: it checks
  out `main` and the PR's merge (`refs/pull/<n>/merge`, so a branch older than
  `main` is compared only on the files it changed) as data
  (`persist-credentials: false`), installs a pinned lvpm release checked against
  `SHA256SUMS` (or a pinned SHA-256, as the release template does), runs
  `lvpm index check`, and hands over a verdict. `merge` has the write token
  (`contents: write`, `pull-requests: write`) and no checkout: on a `merge` verdict from a `check` job that
  succeeded it runs `gh pr merge --squash`, and on a refusal it comments the check's refusal
  lines on the PR with `gh pr comment` and fails the run.

The workflow can only run after an lvpm release ships `lvpm index check`.
Until then, the manual criterion is shown by running `lvpm index check` from
this PR's build against a real PR on the index (see the acceptance criteria).

### The release template

`.github/workflow-templates/lvpm-release.yml` gets a third job, `register`,
after `publish`. Its steps do nothing when the repo has no
`LVPM_INDEX_TOKEN` secret, so existing users see no change. The secret is a
classic token of the publisher with the `public_repo` and `workflow` scopes
(see the publisher journey). The job env holds only `HAS_INDEX_TOKEN`, whether
the secret is set; the token itself goes as `GH_TOKEN` to the steps that fork,
push and open the PR. The job:

1. downloads the `.lvpkg` that `publish` attached;
2. forks the index (the repository variable `LVPM_INDEX_REPO`, default
   `labview-package-manager/package-index`) and clones the index itself, not the
   fork, so the branch starts at the index's current tip;
3. runs `lvpm index add` with the asset's download URL and
   `--github ${{ github.repository }}`;
4. commits on a branch named `lvpm/<owner>/<repo>/<tag>` (the publishing repo and the
   tag, so two repos of one account never replace each other's PR), pushes it to the fork (a re-run of
   the tag replaces it), and opens the PR with `gh pr create`.

The `--clobber` upload of `publish` stays. A re-pushed tag with new bytes
then fails at the bot (rule 3), which is the intended outcome.

### Out of scope

- Moving the lvpm repo to `labview-package-manager`. The user wants this right
  after S8 merges, as its own step.
- Yanking or deleting versions; the bot refuses removals.
- `lvpm search` over the index; the index as a default feed.
- OCI files in index entries (S7b has shipped; a later slice may add them).

## Acceptance Criteria

1. *(manual)* A tag in pilot B's fork, with the release template and
   `LVPM_INDEX_TOKEN` set, opens a PR on `labview-package-manager/package-index`.
   `lvpm index check` built from this PR accepts it, and the PR is merged.
   The evidence (the PR link, the check's output) goes in this slice's PR.
   Source: issue #69, criterion 1.
2. A project with `community = "github:<owner>/<repo>"` in `[sources]` and
   pilot B as a plain `name = "version"` dependency installs pilot B from the
   index. `lvpm.lock` records the index source, `sha256`, `signer` and
   `subject`. Tested against a loopback fake API with a stubbed `gh`. Source:
   issue #69, criterion 2.
3. `lvpm index check` refuses a head whose entry changes the `sha256` of an
   existing version, names the package, version and both hashes, and exits
   non-zero. Source: issue #69, criterion 3.
4. `lvpm index check` refuses each of the other cases in rules 1–7, one
   scenario or unit test each, and accepts a new name and a new version of
   an owned name that pass all rules. With rule 8, a PR whose author is not the
   owner (or a public member of the owning organisation) of a version it adds
   or of an entry file it rewrites, or that has no `--author`, gets the verdict
   `review`, and a refusal still wins. Source: inference from "a bot checks
   name, hash and attestation".
5. `lvpm index add` creates a new entry file; appends a version to the
   owner's existing file; refuses another `--github`; and refuses a version
   that is already listed. Source: inference from "opened by a release
   Action".
6. A name the index does not list (404) resolves from the next source, and a
   name listed by an earlier source is not taken from the index (ADR 4).
   Source: inference from "lvpm reads the index as a feed".
7. A downloaded index file whose SHA-256 differs from its entry fails the
   install, names both hashes, and installs nothing. Source: inference from
   "a bot checks hash"; the consumer checks it as well.
8. With `[policy] require-signature = true`, an unattested index package is
   refused at lock time, as a GitHub dependency is (S6). Source: user,
   2026-11-06 ("lvpm checks too").
9. A `github:` source that is not `owner/repo` fails when the manifest is
   read, naming the source. Source: inference from the existing `[sources]`
   validation.
10. *(manual, with criterion 1)* `docs/usage.md` has a section "Publish to
    the community index" with the four steps of the publisher journey.
    Criterion 1 is run by following only that section in pilot B's fork.
    Every step the section is missing is fixed in the section before the PR.
    The same section shows the consumer's two lines. Source: user,
    2026-11-06 ("what do they have to do to get their lvpkg onto this
    index").
11. Every refusal line of `lvpm index check` names the package, the version
    (when there is one) and what the publisher must change. A scenario per
    rule checks the line's text. Source: user, 2026-11-06, as above.
12. Docs: `docs/usage.md` documents the source kind and both commands in the
    command table. The release template documents `register`. The README
    milestone is ticked, and build-slices.md S8 names the organisation.
    Source: the slice rules in build-slices.md.

## Glossary

| Term | Definition | Status | Source |
|------|------------|--------|--------|
| community index | The repo `labview-package-manager/package-index`, one TOML file per package name | `verified` | user, 2026-11-06 |
| index entry | `packages/<name>.toml`: the name, its owner repo, and one table per version | `verified` | user, 2026-11-06 (one file per name) |
| owner repo | The `github` value of an entry: the only repo whose attested releases may add versions | `verified` | user, 2026-11-06 (first come, first served) |
| the bot | `lvpm index check` run by the index repo's `check.yml` on each PR | `unverified` | inference from issue #69 |

## Ambiguity Log

| Decision | Classification | Resolved By | Rationale / Answer |
|----------|---------------|-------------|-------------------|
| Index organisation and repo | `requires-stakeholder-input` | human | `labview-package-manager/package-index` |
| Default feed or opt-in | `requires-stakeholder-input` | human | Opt-in via `[sources]`; defaults unchanged |
| How lvpm reads the index | `requires-stakeholder-input` | human | One file per name, 404 = not listed; no search |
| Credential for the registration PR | `requires-stakeholder-input` | human | Publisher's PAT as `LVPM_INDEX_TOKEN` |
| Spelling in `[sources]` | `requires-stakeholder-input` | human | `github:owner/repo` shorthand |
| Consumer attestation check | `requires-stakeholder-input` | human | lvpm runs the S6 check on index packages too |
| Name ownership vs vipm.io names | `requires-stakeholder-input` | human | First come, first served; no reserved names |
| Who creates the index repo | `requires-stakeholder-input` | human | Created during build; ask before each push |
| What a publisher must do, and what they see | `requires-stakeholder-input` | human | Four steps in docs/usage.md, proven by following them for criterion 1; refusals are commented on the PR with a fix (user, 2026-11-06) |
| Which token type opens the index PR | `inferable` | inference | Classic with `public_repo` and `workflow` is the tested choice (`workflow`: the branch starts at the index's tip, which may hold workflow files the fork lacks). A fine-grained token may not work. The docs name the one that worked |
| Transport for `packages/<name>.toml` | `inferable` | inference | The existing GitHub client (contents API, `GITHUB_API_URL`, S4 token chain) gives private indexes, auth and the loopback test hook for free. Alternative raw.githubusercontent.com has no test hook. Predictability: both work; the client is what S4/S6 tests already fake. Risk: anonymous API rate limit (60/h) on large resolves; the token chain lifts it |
| Bot logic in Rust as `lvpm index check` | `inferable` | inference | Criterion 3 must be a `cargo test` (slice rules), and the hash, package reading and S6 verification already live in `lvpm-core` |
| `Entry::source` spelling for index entries | `inferable` | inference | Must differ from `github:` (taken by S4 entries); exact string left to the plan |
| Removing or changing versions | `inferable` | inference | Versions are immutable, as in every other lvpm lock and source. Yanking is out of scope |
| Listing failures of the index | `inferable` | inference | Same handling as hosted folders today, so behaviour stays consistent across sources |
| Manual criterion before an lvpm release exists | `inferable` | inference | The hosted bot needs a released `lvpm index check`; the PR shows the same check run from its own build against a real index PR |
| Template change for existing users | `inferable` | inference | The `register` job skips without `LVPM_INDEX_TOKEN`, so no existing publisher changes behaviour |
| Two PRs for the same new name | `inferable` | inference | Both add the same file, so the second merge conflicts. No extra lock needed |
| `--clobber` in the release template | `inferable` | inference | Kept; a changed hash is caught by bot rule 3, which is criterion 3 |
| Completeness: index entry CRUD | `inferable` | inference | Create = registration PR; Read = lvpm; Update = append a version only; Delete = refused (out of scope) |
| Completeness: authn/authz | `inferable` | inference | Reads: S4 token chain. Registration: publisher PAT. Merge: index repo's `GITHUB_TOKEN`. Authority to add versions = attestation from owner repo |
| Completeness: audit | `inferable` | inference | The index repo's git history and merged PRs are the audit trail |
| Names that a public index lists (R7) | `requires-stakeholder-input` | human (2026-11-07) | A new name that vipm.io (NI or JKI) lists is refused. Security review: index sources rank ahead of the public indexes, so anyone could register `oglib_error` and hijack it. Public indexes unreadable or empty: the check errors, the bot posts "could not run". New versions of a listed name are not re-checked |
| Who may list (R8) | `requires-stakeholder-input` | human (2026-11-07) | Only the owner: the PR author's login equals the entry's `github` owner, or the owner is an organisation with the author as a public member. Otherwise the verdict is review (not refused, not merged). The same holds for an entry file the PR rewrites without adding a version to it (security review: a cosmetic edit to another owner's file is a change to what they publish). No `--author`: review for every new version and rewritten file |
| `lvpm search` not covering the index | `LOW_VALUE` | inference | Decided out of scope; no criterion |

## Consistency Gate

- [x] Intent is unambiguous
- [x] Every behavior/goal maps to an acceptance criterion
- [x] Architecture constrains without over-engineering
- [x] Terminology consistent across artifacts
- [x] No contradictions between artifacts
- [x] Every gap/ambiguity finding is logged — inferable with rationale or resolved by human
