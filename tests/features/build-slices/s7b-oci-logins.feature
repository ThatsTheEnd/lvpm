Feature: OCI registry credentials
  lvpm logs in to a registry with what `docker login` left in the Docker config:
  the credHelpers entry for the host, else credsStore, else the auths entry.
  With none, the call is anonymous. A credential helper is run once per host per
  run, and what a helper prints or a config holds as a secret is never shown.
  The registry is a loopback fake: `<registry>` stands for its host:port, and a
  secret is a long unique string so that "does not contain" cannot match by
  chance.

  Background:
    Given a loopback OCI registry
    And the registry holds "alpha" 1.0.0 under "team"
    And a project whose [sources] has team = "oci://<registry>/team"
    And the project depends on alpha = { version = "*", index = "team" }

  Scenario: A public package pulls anonymously
    Given the registry lets anyone read "team"
    And no Docker config
    When I run "lvpm sync --prefix dest"
    Then "alpha" 1.0.0 is installed
    And the registry received a token request with no Authorization header

  Scenario: A credential helper named for the host is used
    Given the registry requires a login for "team"
    And a Docker config whose credHelpers names "lvpmtest" for "<registry>"
    And a docker-credential-lvpmtest that returns user "u1" and secret "s1-7f3a9c-secret"
    When I run "lvpm lock"
    Then it succeeds
    And the registry's token endpoint received the login "u1"
    And the output does not contain "s1-7f3a9c-secret"

  Scenario: The credsStore is used when no helper names the host
    Given the registry requires a login for "team"
    And a Docker config whose credsStore is "lvpmtest"
    And a docker-credential-lvpmtest that returns user "u2" and secret "s2-4be1d8-secret"
    When I run "lvpm lock"
    Then the registry's token endpoint received the login "u2"
    And the output does not contain "s2-4be1d8-secret"

  Scenario: credsStore wins over auths, and a helper for another host is ignored
    Given the registry requires a login for "team"
    And a Docker config whose credHelpers names "first" for "other.example", whose credsStore is "second", and whose auths holds "u3:s3-91c0e5-secret" for "<registry>"
    And a docker-credential-second that returns user "u2" and secret "s2-4be1d8-secret"
    When I run "lvpm lock"
    Then the registry's token endpoint received the login "u2"

  Scenario: auths is used when no helper is configured
    Given the registry requires a login for "team"
    And a Docker config whose auths holds "u3:s3-91c0e5-secret" for "<registry>"
    When I run "lvpm lock"
    Then the registry's token endpoint received the login "u3"
    And the output does not contain "s3-91c0e5-secret"

  Scenario: credHelpers wins over credsStore, and credsStore over auths
    Given the registry requires a login for "team"
    And a Docker config whose credHelpers names "first" for "<registry>", whose credsStore is "second", and whose auths holds "u3:s3-91c0e5-secret" for "<registry>"
    And a docker-credential-first that returns user "u1" and secret "s1-7f3a9c-secret"
    And a docker-credential-second that returns user "u2" and secret "s2-4be1d8-secret"
    When I run "lvpm lock"
    Then the registry's token endpoint received the login "u1"

  Scenario: An identity token is exchanged as a refresh token
    Given the registry requires a login for "team"
    And a Docker config whose credHelpers names "lvpmtest" for "<registry>"
    And a docker-credential-lvpmtest that returns the identity-token user name and secret "r1-3c8f2b-token"
    When I run "lvpm lock"
    Then it succeeds
    And the token endpoint received a refresh_token grant with "r1-3c8f2b-token"
    And the output does not contain "r1-3c8f2b-token"

  Scenario: A helper that holds no credentials for the host means anonymous
    Given the registry requires a login for "team"
    And a credential helper that answers "credentials not found in native keychain"
    When I run "lvpm lock"
    Then it fails saying 'alpha: registry <registry> refused the lookup for source "team"; the package is missing or your login cannot read it; run docker login <registry>'
    And the helper ran once

  Scenario Outline: A wrong login is reported with where it came from
    Given the registry requires a login for "team" and refuses "u9"
    And a Docker config whose <config> gives user "u9" and secret "wrong-5d2e7a-secret" for "<registry>"
    When I run "lvpm lock"
    Then it fails saying "registry <registry> refused the login from <origin>; run docker login <registry>"
    And the output does not contain "wrong-5d2e7a-secret"

    # `registry` is a column only because an outline must resolve every <...>;
    # it stands for the registry's host:port, as everywhere in this file.
    Examples:
      | config                       | origin                                 | registry   |
      | credHelpers entry "lvpmtest" | credHelpers docker-credential-lvpmtest | <registry> |
      | credsStore "lvpmtest"        | credsStore docker-credential-lvpmtest  | <registry> |
      | auths entry                  | auths                                  | <registry> |

  Scenario: A helper that exits with an error is reported without its output
    Given a Docker config whose credHelpers names "lvpmtest" for "<registry>"
    And a docker-credential-lvpmtest that exits 3 printing "token s7-a41e9d-locked" on stderr
    When I run "lvpm lock"
    Then it fails saying "credential helper docker-credential-lvpmtest for <registry> failed with exit code 3; run it by hand to see why"
    And the output does not contain "s7-a41e9d-locked"

  Scenario: A helper that is not installed is reported
    Given a Docker config whose credHelpers names "lvpmtest" for "<registry>"
    And no docker-credential-lvpmtest on the PATH
    When I run "lvpm lock"
    Then it fails saying "credential helper docker-credential-lvpmtest for <registry> was not found; check that it is installed and on PATH"

  Scenario: A helper that does not answer in time is reported
    Given a Docker config whose credHelpers names "lvpmtest" for "<registry>"
    And a docker-credential-lvpmtest that never answers
    And LVPM_OCI_HELPER_TIMEOUT_SECS is 1
    When I run "lvpm lock"
    # The sandbox writes any "<n>s" as <DUR>; the unit test of the store checks the number.
    Then it fails saying "credential helper docker-credential-lvpmtest for <registry> did not answer within <DUR>"

  Scenario: The helper runs once per host per run
    Given the registry requires a login for "team"
    And the registry holds "beta" 1.0.0 under "team"
    And the project depends on beta = { version = "*", index = "team" }
    And a credential helper that counts its runs
    When I run "lvpm lock"
    Then it succeeds
    And the helper ran once

  Scenario: publish logs in with the same credentials
    Given the registry requires a login to write "team"
    And a credential helper that returns user "u1" and secret "s1-7f3a9c-secret"
    And a publisher project "alpha" 2.0.0 that lvpm build has packaged
    When I run "lvpm publish --to oci://<registry>/team"
    Then it succeeds
    And the registry's token endpoint received the login "u1" with push scope
    And the output does not contain "s1-7f3a9c-secret"

  Scenario: A refused upload says how to log in and that nothing was published
    Given the registry refuses writes to "team"
    And a publisher project "alpha" 2.0.0 that lvpm build has packaged
    When I run "lvpm publish --to oci://<registry>/team"
    Then it fails saying "registry <registry> refused the upload to team/alpha; run docker login <registry> with a token that can write packages; nothing was published, it is safe to rerun"
    And the registry holds no tag "2.0.0" in "team/alpha"
