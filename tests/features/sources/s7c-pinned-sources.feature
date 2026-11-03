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

  Scenario: a pinned package its source does not list fails naming both
    Given "first" lists "alpha" 1.0
    And "second" lists "gamma" 1.0
    And the project depends on "alpha" "*" pinned to "second"
    When I run "lvpm lock"
    Then the command fails naming "alpha" and "second"
    And the error says "other sources are ignored for a pinned package"
    And no lvpm.lock is written

  Scenario: a pinned source with no fitting version fails naming it, not falling back
    Given "first" lists "alpha" 2.0
    And "second" lists "alpha" 1.0
    And the project depends on "alpha" ">=2.0" pinned to "second"
    When I run "lvpm lock"
    Then the command fails naming "alpha" and "second"
    And the error says "its pinned source"
    And no lvpm.lock is written

  Scenario: an unreachable pinned source fails naming it
    Given "first" lists "alpha" 1.0
    And "second" cannot be reached
    And the project depends on "alpha" "*" pinned to "second"
    When I run "lvpm lock"
    Then the command fails naming "second"
    And no lvpm.lock is written

  Scenario: an unreachable pinned source fails lvpm info naming it
    Given "first" lists "alpha" 1.0
    And "second" cannot be reached
    And the project depends on "alpha" "*" pinned to "second"
    When I run "lvpm info alpha"
    Then the command fails naming "second"
    And the error says "could not read"

  Scenario: adding a source pin makes the lock stale
    Given "first" lists "alpha" 1.0
    And "second" lists "alpha" 1.0
    And the project depends on "alpha" "*" and lvpm.lock is current
    When I pin "alpha" to "second" in lvpm.toml
    And I run "lvpm lock --check"
    Then the exit code is non-zero
    And stderr is one line naming sources changed

  Scenario: changing a source pin makes the lock stale
    Given "first" lists "alpha" 1.0
    And "second" lists "alpha" 1.0
    And the project depends on "alpha" "*" pinned to "first" and lvpm.lock is current
    When I pin "alpha" to "second" in lvpm.toml
    And I run "lvpm lock --check"
    Then the exit code is non-zero
    And stderr is one line naming sources changed

  Scenario: removing a source pin makes the lock stale
    Given "first" lists "alpha" 1.0
    And "second" lists "alpha" 1.0
    And the project depends on "alpha" "*" pinned to "second" and lvpm.lock is current
    When I remove the source pin of "alpha"
    And I run "lvpm lock --check"
    Then the exit code is non-zero
    And stderr is one line naming sources changed

  Scenario: a pinned project with a current lock is not stale
    Given "first" lists "alpha" 1.0
    And "second" lists "alpha" 1.0
    And the project depends on "alpha" "*" pinned to "second" and lvpm.lock is current
    When I run "lvpm lock --check"
    Then the command succeeds

  Scenario: a lock from before pinned sources is not stale and keeps its bytes
    Given "first" lists "alpha" 1.0
    And the project depends on "alpha" "*" and a lvpm.lock written before pinned sources
    When I run "lvpm lock --check"
    Then the command succeeds
    When I run "lvpm lock"
    Then the command succeeds
    And lvpm.lock is byte-identical to before

  Scenario: lvpm add --index writes the table form with the pinned source's version
    Given "first" lists "alpha" 2.0
    And "second" lists "alpha" 1.0
    When I run "lvpm add alpha --index second --prefix dest"
    Then the command succeeds
    And lvpm.toml pins "alpha" to "second" with ">=1.0"
    And lvpm.lock takes "alpha" 1.0 from "second"
    And the output says "(from second)"

  Scenario: lvpm add --dev --index writes the pin into [dev-dependencies]
    Given "second" lists "alpha" 1.0
    When I run "lvpm add alpha --dev --index second --prefix dest"
    Then the command succeeds
    And lvpm.toml pins "alpha" to "second" with ">=1.0" in [dev-dependencies]

  Scenario: lvpm add --index into a group
    Given "second" lists "alpha" 1.0
    When I run "lvpm add alpha --index second --group ci --prefix dest"
    Then the command succeeds
    And lvpm.toml pins "alpha" to "second" with ">=1.0" in [dependency-groups.ci]

  Scenario: lvpm add --index moves an existing pin
    Given "first" lists "alpha" 1.0
    And "second" lists "alpha" 2.0
    And the project depends on "alpha" "*" pinned to "first"
    When I run "lvpm add alpha --index second --prefix dest"
    Then the command succeeds
    And lvpm.toml pins "alpha" to "second" with ">=2.0"

  Scenario: lvpm add without --index keeps an existing pin
    Given "second" lists "alpha" 1.0, "alpha" 2.0
    And the project depends on "alpha" "1.0" pinned to "second"
    When I run "lvpm add alpha@2.0 --prefix dest"
    Then the command succeeds
    And lvpm.toml pins "alpha" to "second" with "2.0"

  Scenario: lvpm add --index naming an unknown source changes nothing
    Given "first" lists "alpha" 1.0
    And the project depends on "alpha" "*"
    And a current lock
    When I run "lvpm add alpha --index nope --prefix dest"
    Then the exit code is non-zero
    And the error says "index = \"nope\" is not a [sources] name"
    And lvpm.toml and lvpm.lock are byte-identical to before

  Scenario: lvpm add --index to a source that does not list the package changes nothing
    Given "first" lists "alpha" 1.0
    And "second" lists "gamma" 1.0
    And the project depends on "gamma" "*"
    And a current lock
    When I run "lvpm add alpha --index second --prefix dest"
    Then the command fails naming "alpha" and "second"
    And lvpm.toml and lvpm.lock are byte-identical to before

  Scenario: lvpm add --index on a github line fails
    Given the repo "owner/repo" has release "v1.0" with "alpha-1.0.vip"
    And the project depends on "alpha" via github "owner/repo"
    And a current lock
    When I run "lvpm add alpha --index second --prefix dest"
    Then the exit code is non-zero
    And the error says "github and index cannot be used together"
    And lvpm.toml and lvpm.lock are byte-identical to before

  Scenario: the docs describe source pins
    Then docs/usage.md mentions "pinned to one source"
    And docs/usage.md mentions "never falls back"
    And docs/usage.md mentions "beats a sources.local folder and a cache import"
    And docs/usage.md mentions "--index <NAME>"
    And docs/usage.md mentions "a dependency's source pin (`index`) was added, removed or changed"
    And docs/manifest.md mentions "the index key"
    And docs/manifest.md does not mention "index is not supported yet"
