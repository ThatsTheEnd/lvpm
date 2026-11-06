Feature: OCI template and docs
  The release template shows how a package is published to an OCI registry and
  attested there, and the docs say what OCI sources do and what they do not
  promise.

  Scenario: The release template attests the OCI manifest
    When I read ".github/workflow-templates/lvpm-release.yml"
    Then its attest step uses "actions/attest-build-provenance"
    And that step's "with" has "subject-name", "subject-digest" and "push-to-registry: true"
    And a job runs "lvpm publish --to oci://"

  Scenario Outline: The docs describe OCI sources
    When I read "<doc>"
    Then it contains "oci://" and "<phrase>"

    Examples:
      | doc                                       | phrase                                     |
      | docs/manifest.md                          | index =                                    |
      | docs/manifest.md                          | docker login                               |
      | docs/usage.md                             | lvpm publish --to oci://                   |
      | docs/usage.md                             | docker login                               |
      | docs/usage.md                             | tag immutability                           |
      | docs/usage.md                             | identity, not an endorsement               |
      | docs/adr/0007-oci-sources-are-explicit.md | is not an http:// or https:// index folder |
