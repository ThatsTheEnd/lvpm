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
