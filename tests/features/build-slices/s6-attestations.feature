Feature: S6: the release template attests
  The template signs a provenance attestation for the package it builds, so a
  consumer's `[policy]` can check who published it.

  Scenario: the release template attests what it uploads
    Then the release template's job grants "id-token: write" and "attestations: write"
    And the release template runs "actions/attest-build-provenance@v2" with subject-path "dist/*.lvpkg"
    And that step comes before the step that runs "gh release upload"
