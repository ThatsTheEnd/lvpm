Feature: a dependency can be pinned to one source

  Background:
    Given a project with labview "2024" whose [sources] are "first" then "second"

  Scenario Outline: a pin naming a source [sources] does not define fails for <command>
    Given the project depends on "alpha" "*" pinned to "nope"
    And the project has a venv bound to a LabVIEW installation
    When I run "<command>"
    Then the exit code is non-zero
    And the error says "[dependencies] alpha: index = \"nope\" is not a [sources] name"
    And no lvpm.lock is written

    Examples:
      | command                 |
      | lvpm lock               |
      | lvpm sync --prefix dest |
      | lvpm info alpha         |
      | lvpm check              |
      | lvpm search alpha       |

  Scenario Outline: a pin naming the <key> setting is not a source
    Given the project depends on "alpha" "*" pinned to "<key>"
    When I run "lvpm lock"
    Then the exit code is non-zero
    And the error says "index = \"<key>\" is not a [sources] name"
    And the error says "settings, not sources"

    Examples:
      | key          |
      | defaults     |
      | min-age-days |

  Scenario: one name pinned to two sources in two tables fails
    Given the project depends on "alpha" "*" pinned to "first"
    And the project dev-depends on "alpha" "*" pinned to "second"
    When I run "lvpm lock"
    Then the exit code is non-zero
    And the error says "differs from \"first\" in [dependencies]"

  Scenario: one name pinned to the same source in two tables is fine
    Given "first" lists "alpha" 1.0
    And the project depends on "alpha" "*" pinned to "first"
    And the project dev-depends on "alpha" "*" pinned to "first"
    When I run "lvpm lock"
    Then the command succeeds

  Scenario: a pinned package is locked from its source although an earlier one lists a higher version
    Given "first" lists "alpha" 2.0
    And "second" lists "alpha" 1.0
    And the project depends on "alpha" "*" pinned to "second"
    When I run "lvpm lock"
    Then the command succeeds
    And lvpm.lock takes "alpha" 1.0 from "second"

  Scenario: a pin beats a package in a local folder
    Given the local folder holds "alpha" 2.0
    And "second" lists "alpha" 1.0
    And the project depends on "alpha" "*" pinned to "second"
    When I run "lvpm lock"
    Then the command succeeds
    And lvpm.lock takes "alpha" 1.0 from "second"

  Scenario: a pin beats a package imported with lvpm cache add
    Given "alpha" 2.0 is in the cache
    And "second" lists "alpha" 1.0
    And the project depends on "alpha" "*" pinned to "second"
    When I run "lvpm lock"
    Then the command succeeds
    And lvpm.lock takes "alpha" 1.0 from "second"

  Scenario: a transitive dependency listed with a pin resolves from that source
    Given "first" lists "beta" 1.0, "alpha" 2.0
    And "second" lists "alpha" 1.0
    And the project depends on "beta" "*"
    And the project depends on "alpha" "*" pinned to "second"
    When I run "lvpm lock"
    Then lvpm.lock takes "beta" 1.0 from "first"
    And lvpm.lock takes "alpha" 1.0 from "second"

  Scenario: pinned and unpinned lines in one manifest each follow their own rule
    Given "first" lists "alpha" 2.0, "gamma" 1.0
    And "second" lists "alpha" 1.0, "gamma" 1.0
    And the project depends on "gamma" ">=1.0"
    And the project depends on "alpha" "*" pinned to "second"
    When I run "lvpm lock"
    Then lvpm.lock takes "gamma" 1.0 from "first"
    And lvpm.lock takes "alpha" 1.0 from "second"

  Scenario Outline: lvpm <command> shows only the pinned source's versions
    Given "first" lists "alpha" 2.0
    And "second" lists "alpha" 1.0
    And the project depends on "alpha" "*" pinned to "second"
    When I run "lvpm <command> alpha"
    Then the command succeeds
    And the output shows version 1.0 and not 2.0

    Examples:
      | command |
      | info    |
      | search  |
