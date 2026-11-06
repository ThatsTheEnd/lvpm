@ghcr
Feature: ghcr.io
  These scenarios depend on state outside this repository: the test repo
  ThatsTheEnd/lvpm-oci-test and its packages (the public `alpha` and the
  private `internal`, set up as docs/oci-test-repo-workflow.yml says), and a
  login in the Docker credential store of whoever runs them. The tag alpha:1.0.0
  must never be deleted. They run only with LVPM_TEST_GHCR=1, and the children
  reach the real network.

  Background:
    Given the children reach the real ghcr.io

  Scenario: The public test package installs anonymously by digest
    Given a project pinned to alpha from "oci://ghcr.io/thatstheend/lvpm-oci-test"
    And no Docker config
    When I run "lvpm sync --prefix dest"
    Then alpha is installed and the lock records its digest and signer
    And alpha 1.0.0 has an SBOM referrer on the registry
    And that referrer holds the bom.cdx.json of the published package

  Scenario: Republishing the fixed tag is refused
    Given the Docker config of the test runner
    And the fixture package alpha 1.0.0 from tests/fixtures
    And the digest of the tag alpha:1.0.0 is noted
    When I run "lvpm publish --to oci://ghcr.io/thatstheend/lvpm-oci-test"
    Then it fails saying "thatstheend/lvpm-oci-test/alpha:1.0.0 is already published"
    And the tag alpha:1.0.0 still has the noted digest

  Scenario: The private package is refused without credentials
    Given a project pinned to internal from "oci://ghcr.io/thatstheend/lvpm-oci-test"
    And no Docker config
    When I run "lvpm lock"
    Then it fails saying "registry ghcr.io refused the lookup"

  Scenario: The private package locks with the runner's credentials
    Given a project pinned to internal from "oci://ghcr.io/thatstheend/lvpm-oci-test"
    And the Docker config of the test runner
    When I run "lvpm sync --prefix dest"
    Then internal is installed and locked
