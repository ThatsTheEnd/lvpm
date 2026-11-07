# 8. Community index sources take part in first-source-wins

Date: 2026-11-07

## Status

Accepted 2026-11-07. It amends [ADR 4](0004-sources-first-wins-and-pinning.md)
(how a source says "not listed") and contrasts
[ADR 7](0007-oci-sources-are-explicit.md), which keeps OCI sources out of the
order.

## Context

S8 adds a `github:owner/repo` entry to `[sources]`: a community index, a GitHub
repository with one `packages/<key>.toml` per package name. It has no
catalogue file. To learn whether it lists a name, lvpm asks for that name's
file.

That is ADR 7's situation: a source answers one question per name. ADR 7 made
the OCI source explicit, because ghcr.io answers `403` for a name it does not
have, which cannot be told from a registry that refuses the caller, so
searching it for every name would fail every resolve. A community index
differs on the point that decided ADR 7. The contents API answers `404` for a
file that is not there, and a `404` for a name is an answer, not a refusal. An
explicit-only index would also defeat its purpose: a consumer writes
`labview_open_source_lib_serializer = ">=1.2"` with no pin and no repo.

## Decision

1. **A `github:` source is searched like an `http(s)://` source.** It is
   asked for every wanted name, in `[sources]` order, ahead of the public
   defaults. The first source that lists a name wins (ADR 4, decision 1). A
   dependency may also pin it with `index = "<name>"` (ADR 4, decision 2).
2. **A `404` for a name's file reads as "not listed", and the next source is
   asked.** Any other answer (`401`, `403`, rate limit, `5xx`, unreachable, a
   file that does not parse) fails the source and the resolve. It never falls
   through. That is ADR 4's rule for a source that cannot be reached.
3. **A missing or forbidden index repository fails the source.** GitHub also
   answers `404` for a repository that does not exist (`github:o/typo`) and for
   a private one the caller cannot see. Read as "not listed" for every name,
   that would make a misspelled source resolve nothing, silently, and the
   misses are cached. So the first `404` for a name of an index in a run makes
   one more request, for the repository itself. If that is `404` or `403`, the
   source fails and names the repository. It is made once per index per run,
   and only after a `404`, so a run in which the index lists every name asked
   makes no extra request.
4. **One request per name.** lvpm asks for the files of the names the project
   or the command names, then for the `requires` of each name already asked,
   until no new name appears. Each answer, a `404` included, is cached in the
   feed cache, so a warm run makes none, and `--refresh` asks again. Without a
   token GitHub allows 60 API requests an hour; a token (the S4 chain) raises
   that. A cold resolve of a deep tree can reach the limit, and S4's rate-limit
   error says so.
5. **The closure of `requires` is not capped.** A package's `requires` names
   come from index files and package metadata its owners control, so one
   package can make lvpm ask for many names. We accept it: the requests are
   `GET`s of small files, the cache bounds repeats, and the rate limit bounds
   the rest. A cap would fail a legitimate deep tree at an arbitrary depth.
6. **`github-index:owner/repo` is a lock format.** `lvpm.lock` records the
   source of a package from an index as `source = "github-index:<owner>/<repo>"`
   with the entry's `sha256`. `sync --locked` reads the index for the locked
   names and checks that hash. The string never starts with `github:`, which
   marks a package from a GitHub release (S4). The lock's format version stays
   2. Changing the string later changes what locks mean, so it needs a new
   format version.

## Consequences

- Source order matters for a project that lists the index: a name an earlier
  source lists is never taken from the index, and a name the index lists is
  taken from it before vipm.io and NI.
- A consumer with a private index repository or a rate-limited network sees a
  failed source, not a missing package. This is deliberate.
- A first-come owner can publish a name that vipm.io also has. A project that
  lists the index ahead of the defaults gets the index's package. A project
  that lists it after, or not at all, does not.
- **Downgrade.** lvpm 0.24.0 (the release before S8) refuses a manifest with a
  `github:` source in `[sources]`, in every command that resolves sources, with
  `[sources] community: "github:labview-package-manager/package-index" is not an http:// or https:// index folder`
  (`Project::resolved_sources` in `project.rs`; its callers are `env.rs`,
  `lock.rs` and `sync.rs`). A project with an index source always has that
  line, so an older lvpm stops there. What an older lvpm does with a
  `github-index:` lock entry on its own, in a command that does not resolve
  sources, is unverified.
