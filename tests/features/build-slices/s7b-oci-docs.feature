Feature: OCI template and docs
  The release template shows how a package is published to an OCI registry and
  attested there, and the docs say what OCI sources do and what they do not
  promise.

  Scenario: The release template attests the OCI manifest
    When I read ".github/workflow-templates/lvpm-release.yml"
    Then its attest step uses "actions/attest-build-provenance"
    And that step's "with" has "subject-name", "subject-digest" and "push-to-registry: true"
    And a job runs "lvpm publish --to oci://"

  Scenario Outline: The OCI section of a doc describes OCI sources
    When I read "<doc>"
    Then its "<section>" section contains "oci://" and "<phrase>"

    Examples:
      | doc              | section                     | phrase                       |
      | docs/manifest.md | An OCI registry as a source | index =                      |
      | docs/manifest.md | An OCI registry as a source | docker login                 |
      | docs/usage.md    | OCI registries              | lvpm publish --to oci://     |
      | docs/usage.md    | OCI registries              | docker login                 |
      | docs/usage.md    | OCI registries              | tag immutability             |
      | docs/usage.md    | OCI registries              | identity, not an endorsement |

  Scenario: ADR 7 says an OCI source is no index folder
    When I read "docs/adr/0007-oci-sources-are-explicit.md"
    Then it contains "oci://" and "is not an http:// or https:// index folder"
