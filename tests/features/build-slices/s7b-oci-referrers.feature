Feature: OCI referrers
  A package's SBOM is attached to its OCI manifest as a referrer: an artifact
  whose subject is the package, found through the registry's referrers API or,
  on a registry without it, through the index kept under the tag
  sha256-<hex>. The package is published first, and an SBOM that cannot be
  attached afterwards fails the command without hiding the digest of what is
  live. The registry is a loopback fake: `<registry>` stands for its host:port.

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
