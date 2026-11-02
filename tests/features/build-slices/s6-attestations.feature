Feature: S6: attestations
  The release template signs a provenance attestation for the package it
  builds, so a consumer's `[policy]` can check who published it, and `lvpm lock`
  refuses a GitHub release that the `[policy]` does not accept.

  Scenario: the release template attests what it uploads
    Then the release template's job grants "id-token: write" and "attestations: write"
    And the release template runs "actions/attest-build-provenance@v2" with subject-path "dist/*.lvpkg"
    And that step comes before the step that runs "gh release upload"

  Scenario: an unattested release is refused when the policy requires a signature
    Given the pilot B project
    And the manifest has [policy] require-signature = true
    When I run "lvpm lock"
    Then the command fails with exit code 1 naming "has no attestation"
    And no lvpm.lock is written

  Scenario: without a policy the same release locks with no signer
    Given the pilot B project
    When I run "lvpm lock"
    Then the command succeeds
    And lvpm.lock has no signer for "labview_open_source_lib_serializer"

  Scenario: a policy change makes the lock stale
    Given the pilot B project, locked
    And the manifest gains [policy] require-signature = true
    When I run "lvpm lock --check"
    Then the command fails naming "[policy] changed"
