Feature: S8: installing from the community index
  A project that lists a community index in [sources] installs a package by
  name. lvpm checks the file against the index's sha256 before it uses it, and
  the package's attestation against the repo the index entry names, as it does
  for a GitHub dependency.

  Background:
    Given a fake GitHub whose index labview-package-manager/package-index lists pilot B, published by "someone/Serializer"
    And someone/Serializer's release serves pilot B's file and its dependency data_manipulation is in the project's hosted folder

  Scenario: a project installs pilot B by name through the index
    Given pilot B's release is attested by someone/Serializer
    And the pilot B project with community = "github:labview-package-manager/package-index" in [sources] and pilot B as a plain dependency
    When I run "lvpm sync --prefix dest"
    Then the command succeeds
    And lvpm.lock has pilot B from "github-index:labview-package-manager/package-index" with the index's sha256
    And lvpm.lock has pilot B's signer and subject "https://github.com/someone/Serializer"
    And pilot B and data_manipulation are installed in dest

  Scenario: a locked index package installs again from the lock
    Given pilot B's release is attested by someone/Serializer
    And the pilot B project with community = "github:labview-package-manager/package-index" in [sources] and pilot B as a plain dependency
    And that project was synced once and its installed packages are removed
    And an empty cache
    When I run "lvpm sync --locked --prefix dest"
    Then the command succeeds and pilot B is installed with the locked sha256

  Scenario: a file whose hash differs from the index is not installed
    Given the release serves other bytes for pilot B's file
    And the pilot B project with community = "github:labview-package-manager/package-index" in [sources] and pilot B as a plain dependency
    When I run "lvpm sync --prefix dest"
    Then the command fails printing the line "the file of labview_open_source_lib_serializer 1.2.1.16 from github-index:labview-package-manager/package-index has sha256 <actual>, but the index lists <entry>"
    And the hint says "report this to the package's owner, someone/Serializer; lvpm installed nothing"
    And no lvpm.lock is written and pilot B is not installed

  Scenario Outline: the policy applies to index packages
    Given pilot B's release is <attestation>
    And the pilot B project with community = "github:labview-package-manager/package-index" in [sources] and pilot B as a plain dependency
    And the manifest has [policy] require-signature = true
    When I run "lvpm lock"
    Then the command fails with exit code 1 and stderr has these lines in a row: "<lines>"

    Examples:
      | attestation            | lines |
      | not attested           | lvpm refused 1 package: // labview_open_source_lib_serializer 1.2.1.16 from github.com/someone/Serializer has no attestation // hint: ask the publisher to build with the lvpm release template |
      | attested by other/Fork | lvpm refused 1 package: // the attestation of labview_open_source_lib_serializer 1.2.1.16 from github.com/someone/Serializer does not verify: // verifying with issuer "sigstore.dev" // hint: if gh's reason is a network error, try again; otherwise report this to the publisher, the archive may have been tampered with. lvpm refuses it whatever [policy] says |

  Scenario: without a policy an unattested index package installs unsigned
    Given pilot B's release is not attested
    And the pilot B project with community = "github:labview-package-manager/package-index" in [sources] and pilot B as a plain dependency
    When I run "lvpm lock"
    Then the command succeeds and lvpm.lock has pilot B with no signer

  Scenario: a name the index does not list comes from a later source
    Given the index lists nothing
    And the project with community = "github:labview-package-manager/package-index" in [sources] and data_manipulation as a plain dependency
    When I run "lvpm lock"
    Then data_manipulation is locked from the hosted folder

  Scenario: a dependency pinned to the index that it does not list fails
    Given data_manipulation is pinned with index = "community" and the index does not list it
    When I run "lvpm lock"
    Then the command fails naming data_manipulation and "searched github:labview-package-manager/package-index"
    And data_manipulation is not taken from the hosted folder

  Scenario Outline: a github: source that is not owner/repo fails when the manifest is read
    Given the project with community = "<value>" in [sources] and data_manipulation as a plain dependency
    When I run "lvpm lock"
    Then the command fails with exit code 1 printing the manifest error "lvpm.toml: [sources] community: "<value>" is not github:owner/repo"

    Examples:
      | value        |
      | github:x     |
      | github:/r    |
      | github:o/    |
      | github:o/r/x |
      | github:../r  |
      | github:o/..  |
      | github:o/r?x |
