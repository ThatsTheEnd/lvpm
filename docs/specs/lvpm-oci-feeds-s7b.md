<!-- spec-version: 14.0.1 -->
# Spec: OCI feeds (slice S7b)

**Format:** dev-team specs 14.0.1 · **Status:** approved by the user on 2026-11-05; gate passed; seven stakeholder answers given by the user on 2026-11-05; amended by the user on 2026-11-06 (see [Amendments](#amendments-2026-11-06)) · **Date:** 2026-11-05 ·
**Slice:** [#68](https://github.com/ThatsTheEnd/lvpm/issues/68), [S7b in build-slices.md](../vision/build-slices.md) ·
**Rules:** [ADR 4](../adr/0004-sources-first-wins-and-pinning.md) (first source wins, pins), [ADR 6](../adr/0006-lvpkg-only-vip-read-only.md) (`.lvpkg`)

## Intent Description

Today a team shares packages through a folder share, a hosted index folder or
GitHub Releases. Most organisations already run a container registry (ghcr,
GitLab, ACR) with access control, retention and audit. After this slice a team
publishes a `.lvpkg` to such a registry with `lvpm publish --to oci://…`, and
another project installs it by listing the registry in `[sources]`,
`org = "oci://ghcr.io/org/lv"`, and pinning the package to it with
`index = "org"`. An OCI source serves only the packages pinned to it.

The lock records the OCI feed and the manifest digest, so `sync --locked`
pulls exactly those bytes even if someone re-points the tag. A published
version is never replaced: publishing a version whose tag exists fails and
writes nothing. The package's SBOM, and the signature its CI attaches, are
OCI referrers of the package; lock finds the signature and checks it under
the existing `[policy]`, and install does not read the SBOM. Registry credentials come from the Docker
credential store (which `docker login` and `az acr login` fill), never from
`lvpm.toml`.

## Architecture Specification

**Client.** A new `crates/lvpm-core/src/oci.rs`: a small OCI Distribution
client over the blocking `reqwest` lvpm already uses (user decision; no
tokio, no `oras` binary). It covers only what the slice needs: token auth,
tags list, manifest GET/HEAD/PUT, blob GET/HEAD/upload (monolithic POST+PUT),
referrers (API with the spec's tag-schema fallback).

**Feed shape (user decision).** A `[sources]` value starting with `oci://`
is an OCI feed: `org = "oci://ghcr.io/org/lv"`. Package `X` lives in
repository `<prefix>/<name_key(X)>` (lowercase), one tag per version. An OCI source is
**explicit** (user decision, as uv's `explicit = true` index): it is asked
only for the names pinned to it with `index = "org"`, never for any other
name, and it takes no part in first-source-wins. Registries have no package
catalogue to read, and ADR 4 searches `[sources]` before the public indexes,
so probing every name would send one request per public name and, on ghcr,
fail on each. An internal package's internal dependencies are pinned too (as
S7c already allows for transitive ones). A new ADR 7 records this. A pinned
name is "listed" when the tags list returns ≥ 1 tag;
`404 NAME_UNKNOWN` means not listed; any other failure means "cannot be
reached" (fails, never falls through). A `401`/`403` (ghcr answers
`403 DENIED` for a repository that does not exist, probed 2026-11-05, and the
same for one the caller may not read) cannot tell the two apart, so it fails
too (user decision): the error names the source and the package and says
that the registry refused the lookup, so either the package does not exist
there or lvpm's Docker credentials for that host may not read it. `Entry.source` and the lock's `source`
hold the `oci://` URL as written.

**Artifact (media types).** One OCI 1.1 image manifest per version:

| Part | Media type |
|---|---|
| manifest `artifactType` | `application/vnd.lvpm.package.v1` |
| config | `application/vnd.oci.empty.v1+json` (the 2-byte `{}` blob) |
| single layer, the `.lvpkg` bytes | `application/vnd.lvpm.lvpkg.v1+zip` |

Manifest annotations carry what the index needs without downloading the
layer: `org.opencontainers.image.title` (package name),
`org.opencontainers.image.version`, `org.opencontainers.image.created`
(feeds `min-age-days`), `dev.lvpm.requires` and `dev.lvpm.labview`, in the
same text form `index.vipr` lists them. The layer digest is the archive
SHA-256 the lock already records.

**Referrers.** SBOM: `lvpm publish` pushes the `bom.cdx.json` inside the
`.lvpkg` as a referrer (artifactType `application/vnd.cyclonedx+json`,
`subject` = the package manifest). Signature: the release workflow template
runs `actions/attest-build-provenance` with `subject-digest` = the package
manifest digest and `push-to-registry: true`, which attaches a Sigstore bundle
referrer (`application/vnd.dev.sigstore.bundle.v0.3+json`). Its subject is the
package manifest digest, not the `.lvpkg` sha256. On lock, lvpm lists the
package's referrers and verifies a bundle with
`gh attestation verify <manifest file> --bundle <file> --repo <repo>`, where
the manifest file holds the digest-checked manifest bytes (the S6 route;
`<repo>` is the bundle certificate's own source-repository claim, which gh
then verifies). The result goes to `verify::decide` and the existing
`[policy]`. No bundle → `Unattested`. `sync --locked` trusts the locked signer,
as in S6. Credentials and the token cache are injected by the front end; core
reads no environment. Every failing OCI pin is reported together.

**Publish.** `lvpm publish --to <folder | oci://…>`. For OCI, in order:
1. HEAD the version tag. Exists → fail, nothing written. Any failure other than 404 → fail, nothing written.
2. Upload the config and layer blobs (HEAD first; skip a blob the registry has).
3. HEAD the tag again; exists → fail (orphan blobs are not visible to anyone).
4. PUT the manifest under the tag.
5. Push the SBOM referrer by digest.

A race between steps 3 and 4 remains (OCI has no create-only tag; user
decision). `docs/usage.md` tells teams to turn on tag immutability (ACR lock,
GitLab protected tags).

**Credentials.** From the Docker config (`$DOCKER_CONFIG/config.json`, else
`~/.docker/config.json`): `credHelpers[host]`, else `credsStore`, via
`docker-credential-<name> get`, else `auths[host].auth`. A username of
`<token>` is an identity token (refresh-token grant); otherwise basic auth on
the token realm. No credentials → anonymous token. An `oci://` URL carrying
`user:pass@` is refused when sources are resolved, naming `[sources]`.

**Lock.** `LockedPackage` gains an optional `digest` (manifest digest,
`sha256:…`), present only for OCI packages; `source` is the `oci://` URL.
`sync --locked` GETs the manifest by that digest, never by tag, then the
layer, and checks the layer bytes against `sha256`. No `FORMAT_VERSION` bump
(S4/S7c precedent: absent field, older lvpm ignores it). A tag that moved
(2026-11-06): a plain `lvpm lock` that finds a locked version at a different
manifest digest fails, names the package, the version, the locked digest and
the registry's, and writes nothing; `lvpm lock --upgrade`, or
`--upgrade-package <name>@<version>`, accepts the new digest.

**Tests.** A loopback fake registry in `lvpm-testkit` (`oci.rs`): in-memory
repos, tags, blobs, manifests, bearer-token challenge, referrers API that a
test can switch off (to exercise the tag-schema fallback), a ghcr mode that
answers `403 DENIED` for a missing repository, and a write log so "changes
nothing" is asserted. Scenarios against real ghcr.io are tagged `@ghcr` and
run only with `LVPM_TEST_GHCR=1`, as `@labview` does; they use the user's test
repository `labview-package-manager/lvpm-oci-test` (a public and a private package), and
its workflow publishes a fixture `.lvpkg` with this branch's lvpm and attests
it with `push-to-registry: true`. Scenarios live in
`tests/features/build-slices/s7b-*.feature` (not `tests/features/sources/`) and
are added to `FEATURE_FILES`.

**Constraints.** No new async runtime. No registry hosting. Azure Artifacts
out of scope. Existing folder publish unchanged.

## Acceptance Criteria

1. **Install by digest.** A package published to the fake registry with `lvpm publish --to oci://127.0.0.1:<port>/team` installs in a second project whose `[sources]` lists that URL as `team` and whose dependency line pins it with `index = "team"`.
2. **Lock.** That project's lock records `source = "oci://…"` and `digest = "sha256:…"`. After the tag is re-pointed to other bytes, `sync --locked` still installs the locked bytes and requests the manifest by digest, not by tag.
3. **Existing tag refused.** Publishing a version whose tag exists fails naming the tag, and the fake's write log shows no blob or manifest write.
4. **SBOM referrer.** After publish, the package manifest has a referrer of artifactType `application/vnd.cyclonedx+json` whose bytes equal the package's `bom.cdx.json`; `publish` attaches it as a referrer with the referrers API on and with it off (tag-schema fallback); install does not read it. *(Amended 2026-11-06; was: install finds it.)*
5. **Signature referrer.** A package with a Sigstore bundle referrer locks with the signer the (faked) gh run reports; with `require-signature = true`, one without a bundle is refused; one whose subject is not in `trusted-publishers` is refused.
6. **Explicit source.** An `oci://` source is asked only for names pinned to it: a project with an OCI source and an unpinned public dependency sends the registry no request for that name. A pinned name the registry does not list (404) fails naming the package and the source; an unreachable OCI source fails naming it; a `403` (the fake's ghcr mode) fails naming both and saying the registry refused the lookup (missing package or no access). None falls through to another source.
7. **Credentials.** With a Docker config whose `credHelpers` names a fake helper on the PATH, lvpm sends that helper's credentials to the token realm; with `auths` only, it uses those; with neither, it pulls anonymously. An `oci://user:pass@host/…` source is refused when sources are resolved, naming `[sources]`. *(Amended 2026-11-06; was: when the manifest is read.)*
8. **ghcr.io** (`@ghcr`, opt-in): against `labview-package-manager/lvpm-oci-test`, the public package installs anonymously by digest with its SBOM and signature referrers found; republishing its existing tag is refused; the private package fails anonymously saying the registry refused the lookup and installs with Docker credentials.
9. **GitLab and ACR** *(manual)*: publish and install (ACR after `az acr login`); evidence in the PR.
10. **Template.** `.github/workflow-templates/lvpm-release.yml` shows the OCI publish and `attest-build-provenance` with `push-to-registry: true`; a test checks the template's keys line by line. *(Amended 2026-11-06.)*
11. `docs/manifest.md` and `docs/usage.md` document `oci://` sources, `publish --to oci://`, credentials and tag immutability.

## Amendments (2026-11-06)

The user approved these on 2026-11-06, while reviewing the finished slice.

- **AC4:** `publish` attaches the SBOM as a referrer, with the referrers API on and off; install does not read it.
- **AC10:** a test checks the template's keys line by line.
- **AC7:** an `oci://user:pass@host/…` source is refused when sources are resolved.
- **Wording corrections that follow ADR 7 (every OCI package is pinned):** AC8 says the private package "fails anonymously saying the registry refused the lookup", not "with the pin hint"; the Ambiguity Log answer for a `401`/`403` suggests `docker login`, not an `index =` pin.
- **Listing text:** `lvpm catalog`, `lvpm info` and the TUI show core's plain listing text, without the publish hint that `lvpm lock` and `lvpm install` add.
- **Moved tag:** a plain `lvpm lock` that finds a locked OCI version at another manifest digest fails and writes nothing; `lvpm lock --upgrade` accepts the new digest (see Lock).
- **Feature files:** the scenarios live in `tests/features/build-slices/s7b-*.feature`.
- **Deviation, unpinned transitive dependencies:** an OCI package's dependency that is pinned to no source is not an error with a pin hint. As for a package from a hosted source, lock warns `beta, required by alpha, is in no source` and records it as unresolved. Only a direct dependency that no source lists fails.

## Glossary

| Term | Definition | Status | Source |
|------|------------|--------|--------|
| OCI source | A `[sources]` entry whose value starts with `oci://`: a registry host plus a repository prefix | `verified` | user answer 2026-11-05 |
| Package manifest | The OCI image manifest of one package version, tagged with the version | `unverified` | this spec |
| Referrer | An OCI manifest whose `subject` is the package manifest (SBOM, signature bundle) | `unverified` | OCI distribution spec 1.1 |
| Digest | `sha256:<hex>` of the package manifest bytes | `unverified` | OCI spec |

## Ambiguity Log

| Decision | Classification | Resolved By | Rationale / Answer |
|----------|---------------|-------------|-------------------|
| Split the slice | `requires-stakeholder-input` | human | One slice, one PR. |
| Rust OCI client or `oras` | `requires-stakeholder-input` | human | Own client over blocking reqwest. |
| How an OCI feed is declared | `requires-stakeholder-input` | human | `oci://` in `[sources]`, repo per package, tag = version. |
| Atomic refusal of an existing tag | `requires-stakeholder-input` | human | Check, upload, recheck, PUT; remaining race documented, immutability recommended. |
| How a `401`/`403` on the tags list counts | `requires-stakeholder-input` | human | Fail safe, naming source and package, and suggest `docker login`. ghcr answers `403 DENIED` for a missing repository, so "missing" and "no access" look alike; falling through would reopen dependency confusion (ADR 4). |
| Which names an OCI source is asked for | `requires-stakeholder-input` | human | Only names pinned to it (explicit source, as uv's `explicit = true`). Found while planning: `[sources]` comes before the public indexes, so probing all names would hit ghcr's 403 for every public package. |
| Test against a real registry | `requires-stakeholder-input` | human | Yes: opt-in `@ghcr` scenarios against a test repository the user sets up. |
| OCI media types | `inferable` | inference | OCI 1.1 artifact guidance: custom `artifactType`, empty config, typed layer. Alternative (Helm-style JSON config blob with metadata) ruled out: annotations give the same data in the manifest GET already needed for the digest. |
| Where version metadata lives | `inferable` | inference | Manifest annotations, so listing needs no layer download (unlike S4's walk). Cost: one manifest GET per tag; shortcut noted if a feed grows large. |
| Who signs | `inferable` | inference | lvpm has no signer (S6 verifies only); `attest-build-provenance push-to-registry` already produces OCI referrers. |
| How the signature is verified | `inferable` | inference + plan review | Reuse S6's gh route with `--bundle`, verified against the manifest digest (push-to-registry's subject), so `verify::decide` and `[policy]` are unchanged. A real-gh spike confirms it before the build relies on it. |
| Lock format version | `inferable` | inference | No bump; S4/S7c precedent for optional fields. |
| Registries without referrers API (ghcr) | `inferable` | inference | OCI spec requires clients to fall back to the `sha256-<hex>` tag schema. |
| Version not valid as an OCI tag | `inferable` | inference | lvpm versions are dotted numbers, valid tags; a version that is not fails publish naming it. |
| Repository name case | `inferable` | inference | OCI requires lowercase; `name_key` is already lowercase. |
| "Found on install" for SBOM | `inferable` | inference | Observable as AC 4 (referrer present and equal to the package's SBOM). No lock field: nothing downstream reads it. |
| Moved tag on a plain lock | `requires-stakeholder-input` | human (2026-11-06) | Fail and write nothing; `--upgrade` accepts the new digest. A silent re-hash would take bytes someone re-pointed the tag to. |
| Delete / unpublish | `inferable` | inference | Out of scope: a published version is never replaced (S2 rule); registry UI deletes. |
| Proxy, TLS settings | `inferable` | inference | Same reqwest client and settings as hosted feeds. |
| Error logging of credentials | `inferable` | inference | Tokens never appear in errors or events (as in github/auth.rs). |

## Consistency Gate
- [x] Intent is unambiguous
- [x] Every behavior/goal maps to an acceptance criterion
- [x] Architecture constrains without over-engineering
- [x] Terminology consistent across artifacts
- [x] No contradictions between artifacts
- [x] Every gap/ambiguity finding is logged — inferable with rationale or resolved by human
