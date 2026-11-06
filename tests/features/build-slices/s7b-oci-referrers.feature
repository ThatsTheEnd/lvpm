Feature: OCI referrers
  A package's SBOM is attached to its OCI manifest as a referrer: an artifact
  whose subject is the package, found through the registry's referrers API or,
  on a registry without it, through the index kept under the tag
  sha256-<hex>. The package is published first, and an SBOM that cannot be
  attached afterwards fails the command without hiding the digest of what is
  live. The signature of a package is a Sigstore bundle attached the same way:
  `lvpm lock` hands gh the package's OCI manifest bytes and the bundle, and
  `[policy]` decides on what gh reports. A registry that cannot say whether a
  bundle exists is an error, never "unsigned". The registry is a loopback fake
  and gh a fake program that records what it was given: `<registry>` stands for
  the registry's host:port.

  Background:
    Given a loopback OCI registry

  Scenario Outline: publish attaches the SBOM as a referrer
    Given the registry <api> the referrers API
    And a project "alpha" 1.0.0 that lvpm build has packaged with its SBOM
    When I run "lvpm publish --to oci://<registry>/team"
    Then the output says "SBOM attached"
    And "team/alpha:1.0.0" has a referrer of artifactType "application/vnd.cyclonedx+json"
    And its bytes equal the package's bom.cdx.json

    Examples: the registry is the fake's host:port, as everywhere else
      | api            | registry   |
      | supports       | <registry> |
      | does not offer | <registry> |

  Scenario: An SBOM push that fails after the package is live says so
    Given a project "alpha" 1.0.0 that lvpm build has packaged with its SBOM
    And the registry refuses referrer writes to "team/alpha"
    When I run "lvpm publish --to oci://<registry>/team"
    Then it fails saying "SBOM could not be attached"
    And the output gives the OCI manifest digest of "team/alpha:1.0.0"
    And the output says "the package is published and a rerun will be refused"

  Scenario: A package without an SBOM publishes without a referrer
    Given a project "alpha" 1.0.0 that lvpm build has packaged without an SBOM
    When I run "lvpm publish --to oci://<registry>/team"
    Then it succeeds
    And the output says "no SBOM in the package"
    And "team/alpha:1.0.0" has no referrer

  Scenario: The tag-schema fallback keeps an existing referrer
    Given the registry does not offer the referrers API
    And "team/alpha:1.0.0" already has a Sigstore bundle referrer in its fallback index
    When lvpm attaches the SBOM of alpha 1.0.0
    Then the fallback index of "team/alpha:1.0.0" lists both the bundle and the SBOM

  Scenario Outline: A Sigstore bundle on the OCI manifest locks with its signer
    Given the registry <api> the referrers API
    And "team/alpha:1.0.0" has a Sigstore bundle referrer that gh verifies as built by "https://github.com/o/r"
    And a project pinned to alpha from team
    When I run "lvpm lock"
    Then the lock records alpha's signer and subject "https://github.com/o/r"
    And gh was asked to verify a file whose sha256 is the OCI manifest digest of "team/alpha:1.0.0"
    And gh was given the bundle of "team/alpha:1.0.0" byte for byte

    Examples: the registry is the fake's host:port, as everywhere else
      | api            | registry   |
      | supports       | <registry> |
      | does not offer | <registry> |

  Scenario: require-signature refuses a package without a bundle
    Given "team/alpha:1.0.0" has no Sigstore bundle referrer
    And a project pinned to alpha from team
    And the project's [policy] has require-signature = true
    When I run "lvpm lock"
    Then it fails saying alpha has no attestation
    And gh was not run

  Scenario: trusted-publishers accepts a matching subject
    Given "team/alpha:1.0.0" has a Sigstore bundle referrer that gh verifies as built by "https://github.com/o/r"
    And a project pinned to alpha from team
    And the project's [policy] trusts only "https://github.com/o/*"
    When I run "lvpm lock"
    Then the lock records alpha's subject "https://github.com/o/r"

  Scenario: trusted-publishers refuses a foreign subject
    Given "team/alpha:1.0.0" has a Sigstore bundle referrer that gh verifies as built by "https://github.com/evil/r"
    And a project pinned to alpha from team
    And the project's [policy] trusts only "https://github.com/o/*"
    When I run "lvpm lock"
    Then it fails naming the subject "https://github.com/evil/r"
    And no version of alpha is locked

  Scenario: A bundle gh rejects is refused under require-signature
    Given "team/alpha:1.0.0" has a Sigstore bundle referrer that gh rejects
    And a project pinned to alpha from team
    And the project's [policy] has require-signature = true
    When I run "lvpm lock"
    Then it fails saying alpha's attestation does not verify

  Scenario: A bundle gh rejects is refused whatever the policy says
    Given "team/alpha:1.0.0" has a Sigstore bundle referrer that gh rejects
    And a project pinned to alpha from team
    When I run "lvpm lock"
    Then it fails saying alpha's attestation does not verify
    And no version of alpha is locked

  Scenario: A bundle with no source repository in its certificate does not verify
    Given "team/alpha:1.0.0" has a Sigstore bundle referrer whose certificate names no source repository
    And a project pinned to alpha from team
    And the project's [policy] has require-signature = true
    When I run "lvpm lock"
    Then it fails saying alpha's attestation does not verify
    And gh was not run

  Scenario: gh missing is reported under require-signature
    Given "team/alpha:1.0.0" has a Sigstore bundle referrer
    And gh is not installed
    And a project pinned to alpha from team
    And the project's [policy] has require-signature = true
    When I run "lvpm lock"
    Then it fails saying "cannot check the attestation of alpha 1.0.0: gh is not installed"

  Scenario Outline: A referrers lookup that fails is an error, not "unsigned"
    Given "team/alpha:1.0.0" is in the registry
    And the registry answers <status> to the referrers lookup of "team/alpha:1.0.0"
    And a project pinned to alpha from team
    And the project's [policy] <policy>
    When I run "lvpm lock"
    Then it fails saying "alpha 1.0.0: the signature lookup on registry <registry> failed"
    And it fails saying "the package was not locked"
    And no version of alpha is locked
    And gh was not run

    Examples: the registry is the fake's host:port, as everywhere else
      | status | policy                       | registry   |
      | 500    | has require-signature = true | <registry> |
      | 403    | has require-signature = true | <registry> |
      | 500    | is empty                     | <registry> |

  Scenario: With no policy, an unsigned package locks with no signer
    Given "team/alpha:1.0.0" has no Sigstore bundle referrer
    And a project pinned to alpha from team
    When I run "lvpm lock"
    Then the lock records alpha with no signer

  Scenario: sync --locked installs a signed lock without running gh
    Given "team/alpha:1.0.0" has a Sigstore bundle referrer that gh verifies as built by "https://github.com/o/r"
    And a project pinned to alpha from team, locked under require-signature with alpha's signer
    When I run "lvpm sync --locked --prefix dest"
    Then alpha is installed
    And gh was not run
