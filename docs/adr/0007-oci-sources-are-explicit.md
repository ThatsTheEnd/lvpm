# 7. OCI sources are explicit

Date: 2026-11-06

## Status

Accepted 2026-11-06. It adds to [ADR 4](0004-sources-first-wins-and-pinning.md)
and changes nothing in it.

## Context

S7b lets a team publish a `.lvpkg` to a container registry (ghcr.io, GitLab,
Azure Container Registry) with `lvpm publish --to oci://host/prefix`, and lets
another project take it from an `oci://host/prefix` entry in `[sources]`.

ADR 4 searches every `[sources]` entry, in order, for every name, and the first
that lists a name wins. That works for an index folder, which is one file that
lists everything it has. A registry has no catalogue. To ask whether it has a
name, lvpm sends one request for that name, so searching a registry first would
send one request per public package. On ghcr.io each of those fails, because
it answers `403` for a repository that does not exist (observed behaviour, not
documented by ghcr.io), the same as for one the caller may not read. ADR 4 forbids reading that as "not listed" and trying the
next source, since that reopens dependency confusion, so a project with an OCI
source ahead of the public indexes could not resolve at all.

uv has the same split: an index marked `explicit = true` is used only for the
packages that name it in `[tool.uv.sources]`.

## Decision

1. **An `oci://` source is explicit.** It is asked only for the names pinned to
   it with `index = "<name>"` ([ADR 4](0004-sources-first-wins-and-pinning.md),
   decision 2), never for any other name, and it takes no part in the
   first-source-wins order. A package from a registry needs a pin. Its own
   dependencies are pinned the way ADR 4 says: as direct dependencies with
   `version = "*"` and an `index`.
2. **A pinned name the source cannot serve fails and never falls through.**
   This is ADR 4's rule. A `404` means the source does not list the name. A
   `401` or `403` cannot be told from a missing repository, so it fails too and
   says the registry refused the lookup. An unreachable registry fails. Every
   failing pin is reported together.
3. **A URL is a complete address.** `oci://host[:port]/prefix` needs no
   subtable (ADR 4, decision 3). The lock records the source and the OCI
   manifest digest of each package, and `sync --locked` asks by that digest
   only. The lock format version does not change.
4. **Credentials are not in `lvpm.toml`.** A `user:password@` part is refused
   when the manifest is read. They come from the Docker credential store, which
   `docker login` fills.
5. **A signature from an OCI source is an identity, not an endorsement.** The
   package's Sigstore bundle is read from the registry, and its repository is
   read from the bundle's own certificate, because an OCI source declares none.
   `require-signature = true` without `trusted-publishers` accepts a bundle
   built by any GitHub repository. Only `trusted-publishers` gives assurance.
   This is weaker than for a GitHub release, where lvpm passes the repository
   the dependency names.

## Consequences

- A project that lists an OCI source and pins nothing to it gets a warning that
  the source is never asked. A name that no source lists gets a note that an OCI
  source exists and is explicit.
- A publish can still race between its second tag check and the manifest write,
  because the OCI API has no way to create a tag only when it is new. Teams
  turn on tag immutability in the registry where it has it (docs/usage.md).
- A bundle is checked with `gh attestation verify` against the OCI manifest, so
  signature checks need `gh` and a token, as for GitHub releases.
- **Downgrade.** An older lvpm refuses a manifest with an `oci://` source, and
  says
  `[sources] team: "oci://ghcr.io/acme/lv" is not an http:// or https:// index folder`.
  It reads a lock that has a `digest`, because an unknown key is ignored, but it
  cannot fetch the package. An older lvpm must be upgraded to use a project
  with an OCI source.
