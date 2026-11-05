Feature: Installing from an explicit OCI source
  A `[sources]` entry `oci://<registry>/<prefix>` is asked only for the names a
  dependency pins to it with `index = "<source>"`. What it lists is locked with
  the digest of the package's OCI manifest, and `lvpm sync` fetches by that
  digest, never by a tag. The registry is a loopback fake: `<registry>` stands
  for its host:port, and a secret is a long unique string so that "does not
  contain" cannot match by chance.

  Background:
    Given a loopback OCI registry
    And a project whose [sources] has team = "oci://<registry>/team"

  Scenario: A package published by lvpm installs in another project
    Given a publisher project "alpha" 1.0.0 that lvpm build has packaged
    And I ran "lvpm publish --to oci://<registry>/team" in the publisher project
    And the consumer project depends on alpha = { version = ">=1.0", index = "team" }
    When I run "lvpm sync --prefix dest" in the consumer project
    Then "alpha" 1.0.0 is installed with the published bytes
    And the consumer's lock records the OCI manifest digest that publish printed

  Scenario: A pinned package installs by digest and the lock records it
    Given the registry holds "alpha" 1.0.0 and 1.1.0 under "team"
    And the project depends on alpha = { version = ">=1.0", index = "team" }
    When I run "lvpm sync --prefix dest"
    Then "alpha" 1.1.0 is installed
    And the lock records alpha with source "oci://<registry>/team" and the OCI manifest digest of 1.1.0
    And the lock's sha256 for alpha is the sha256 of the installed archive

  Scenario: sync --locked pulls by digest even after the tag moves
    Given the registry holds "alpha" 1.1.0 under "team"
    And the project is locked with alpha 1.1.0
    And tag "1.1.0" in "team/alpha" now points at other bytes
    And the package cache is empty
    When I run "lvpm sync --locked --prefix dest"
    Then the locked bytes of alpha are installed
    And the registry was asked for the OCI manifest by digest and never by tag "1.1.0"

  Scenario: A layer whose bytes do not match its digest is refused
    Given the registry holds "alpha" 1.1.0 under "team" with a layer whose bytes do not match its digest
    And the project depends on alpha = { version = "*", index = "team" }
    When I run "lvpm sync --prefix dest"
    Then it fails saying 'alpha 1.1.0 from source "team": registry <registry> served bytes that do not match'
    And alpha is not installed

  Scenario: An unpinned name is never asked of the OCI source
    Given the project also depends on a public package "beta" listed by a hosted feed
    When I run "lvpm lock"
    Then the registry received no request for "team/beta"
    And beta comes from the hosted feed

  Scenario: An OCI source with no pins warns
    Given the project depends only on "beta" from a hosted feed
    When I run "lvpm lock"
    Then the output warns 'OCI source "team" has no package pinned to it and is never asked'

  Scenario: A name no source lists hints at the explicit OCI source
    Given the project depends on gamma = "*" without an index
    And no source lists "gamma"
    When I run "lvpm lock"
    Then it fails naming "gamma"
    And the output says 'note: OCI source "team" is explicit; pin with index = "team"'

  Scenario Outline: A pinned name the OCI source cannot serve never falls through
    Given the registry <answer> for "team/gamma"
    And a hosted feed also lists "gamma"
    And the project depends on gamma = { version = "*", index = "team" }
    When I run "lvpm lock"
    Then it fails saying '<message>'
    And no version of gamma is locked

    Examples:
      | answer                        | message                                                                                                                                  |
      | holds nothing                 | gamma: source "team" (oci://<registry>/team) does not list it; check the name, or publish it with lvpm publish --to oci://<registry>/team |
      | answers 500 for the tags list | gamma: source "team" (oci://<registry>/team) is unreachable                                                                              |
      | is down                       | gamma: source "team" (oci://<registry>/team) is unreachable                                                                              |

  Scenario Outline: A registry that refuses the lookup fails and does not fall through
    Given the registry answers <status> for the tags list of "team/gamma"
    And a hosted feed also lists "gamma"
    And the project depends on gamma = { version = "*", index = "team" }
    When I run "lvpm lock"
    Then it fails saying '<message>'
    And no version of gamma is locked

    Examples:
      | status                         | message |
      | 403 DENIED after a valid token | gamma: registry <registry> refused the lookup for source "team"; the package is missing or your login cannot read it; run docker login <registry> |
      | 401 without a challenge        | gamma: registry <registry> refused the lookup for source "team"; the package is missing or your login cannot read it; run docker login <registry> |

  Scenario: Every failing pin is reported together
    Given the project depends on gamma = { version = "*", index = "team" }
    And the project depends on delta = { version = "*", index = "team" }
    And the registry holds neither, and answers 403 DENIED after a valid token for the tags list of "team/gamma"
    When I run "lvpm lock"
    Then it fails with these lines, in this order:
      | delta: source "team" (oci://<registry>/team) does not list it; check the name, or publish it with lvpm publish --to oci://<registry>/team |
      | gamma: registry <registry> refused the lookup for source "team"; the package is missing or your login cannot read it; run docker login <registry> |

  Scenario: A locked digest the registry no longer holds says to re-lock
    Given the registry holds "alpha" 1.1.0 under "team"
    And the project is locked with alpha 1.1.0
    And the registry deleted that OCI manifest
    And the package cache is empty
    When I run "lvpm sync --locked --prefix dest"
    Then it fails saying 'cannot fetch alpha 1.1.0 by digest from source "team": registry <registry> no longer holds'
    And the output says "run lvpm lock"

  Scenario: sync --locked without a login says how to log in
    Given the registry holds "alpha" 1.1.0 under "team"
    And the project is locked with alpha 1.1.0
    And the registry now requires a login for "team"
    And no Docker config
    And the package cache is empty
    When I run "lvpm sync --locked --prefix dest"
    Then it fails saying 'cannot fetch alpha 1.1.0 by digest from source "team": registry <registry> refused it; run docker login <registry>'

  Scenario: A cached locked package installs with the registry down
    Given the registry holds "alpha" 1.1.0 under "team"
    And the project is locked with alpha 1.1.0 and sync has installed it once
    And the registry is down
    When I run "lvpm sync --locked --prefix dest" into a fresh environment
    Then "alpha" 1.1.0 is installed
    And the registry received no request

  Scenario: Locked bytes that do not match the lock are refused
    Given the registry holds "alpha" 1.1.0 under "team"
    And the project is locked with alpha 1.1.0
    And the registry now serves other bytes for that layer digest
    And the package cache is empty
    When I run "lvpm sync --locked --prefix dest"
    Then it fails saying 'alpha 1.1.0 from source "team": registry <registry> served bytes that do not match'
    And alpha is not installed

  Scenario: A re-lock after a tag moved records the new bytes
    Given the registry holds "alpha" 1.1.0 under "team"
    And the project is locked with alpha 1.1.0
    And tag "1.1.0" in "team/alpha" now points at other bytes
    When I run "lvpm lock --upgrade"
    Then the lock's digest and sha256 for alpha both describe the new bytes

  Scenario: An OCI manifest that is not an lvpm package is skipped
    Given tag "1.2.0" in "team/alpha" holds a container image, not an lvpm package
    And the registry holds "alpha" 1.1.0 under "team"
    And the project depends on alpha = { version = "*", index = "team" }
    When I run "lvpm lock"
    Then alpha 1.1.0 is locked

  Scenario: A version newer than the project's LabVIEW is not chosen
    Given the registry holds "alpha" 1.1.0 and 1.2.0 under "team"
    And "alpha" 1.2.0 needs a newer LabVIEW than the project's
    And the project depends on alpha = { version = "*", index = "team" }
    When I run "lvpm lock"
    Then alpha 1.1.0 is locked

  Scenario: min-age-days uses the OCI manifest's creation time
    Given the registry holds "alpha" 1.1.0 created 10 days ago and 1.2.0 created today under "team"
    And the project's min-age-days is 7
    And the project depends on alpha = { version = "*", index = "team" }
    When I run "lvpm lock"
    Then alpha 1.1.0 is locked

  Scenario: Credentials in an oci:// source are refused when the manifest is read
    Given [sources] has team = "oci://u1-4d2e8b-user:s1-7f3a9c-secret@<registry>/team"
    When I run "lvpm lock"
    Then it fails saying '[sources] team: credentials do not belong in lvpm.toml; remove the user:password@ part and run docker login <registry>'
    And the output does not contain "s1-7f3a9c-secret"
    And the output does not contain "u1-4d2e8b-user"
