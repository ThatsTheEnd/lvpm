Feature: Dependency groups
  A project names dependency groups as [dependency-groups.<name>] beside
  [dev-dependencies]. Every command that reads the manifest refuses a
  malformed [dependency-groups] and names the header and the rule.

  Background:
    Given fixture packages "alpha" 1.0, "beta" 1.0, "caraya" 1.0 requiring "delta" >= 1.0, "delta" 1.0, "ci_tools" 1.0 and 2.0 requiring "alpha" >= 1.0
    And a project with labview "2024" depending on "alpha" "*", with a scratch destination "dest"

  # AC4
  Scenario Outline: a malformed [dependency-groups] is refused when read
    Given lvpm.toml has <manifest>
    When I run "lvpm lock"
    Then the command fails naming <named>
    And lvpm.lock does not exist

    Examples:
      | manifest                                 | named                                              |
      | [dependency-groups.dev] "caraya" "*"     | "[dependency-groups.dev]" and "[dev-dependencies]" |
      | [dependency-groups.Dev] "caraya" "*"     | "[dependency-groups.Dev]" and "[dev-dependencies]" |
      | [dependency-groups] ci = "ci_tools"      | "[dependency-groups] ci" and "table"               |
      | [dependency-groups."a.b"] "beta" "*"     | "a.b" and "letters, digits, - and _"               |
      | [dependency-groups.ci] "ci_tools" "~1.0" | "[dependency-groups.ci] ci_tools"                  |

  # AC7
  Scenario: the lock marks what only groups need
    Given lvpm.toml has [dev-dependencies] "caraya" "*"; [dependency-groups.ci] "ci_tools" "*", "caraya" "*"
    When I run "lvpm lock"
    Then lvpm.lock is format 2 and has no "dev =" line
    And lvpm.lock gives "caraya" and "delta" groups ["ci", "dev"], "ci_tools" groups ["ci"], and "alpha" no groups

  # AC3
  Scenario Outline: a group change makes the lock stale
    Given lvpm.toml has [dependency-groups.ci] "ci_tools" "*" and a current lvpm.lock
    And lvpm.toml <change>
    When I run "lvpm lock --check"
    Then the command fails saying lvpm.lock is out of date: <reason>

    Examples:
      | change                                           | reason                                                      |
      | now also lists [dependency-groups.ci] "beta" "*" | [dependency-groups.ci] dependency "beta" was added          |
      | no longer lists [dependency-groups.ci] "ci_tools" | [dependency-groups.ci] dependency "ci_tools" was removed    |
      | has no [dependency-groups.ci] table              | [dependency-groups.ci] dependency "ci_tools" was removed    |
      | gives [dependency-groups.ci] "ci_tools" "1.0"    | [dependency-groups.ci] "ci_tools" "1.0" excludes locked 2.0 |

  # AC3, AC6
  Scenario: lock --upgrade-package moves a group's pin and rewrites it
    Given lvpm.toml has [dependency-groups.ci] "ci_tools" "1.0" and lvpm.lock holds "ci_tools" 1.0
    When I run "lvpm lock --upgrade-package ci_tools@2.0"
    Then lvpm.lock gives "ci_tools" 2.0 groups ["ci"]
    And [dependency-groups.ci] says "ci_tools" = "2.0"

  # AC9
  Scenario: a format 1 lock is stale
    Given an lvpm.lock in format 1 with "caraya" marked dev = true
    When I run "lvpm lock --check"
    Then the command fails saying "lvpm.lock is out of date: the lock format is outdated (this lvpm writes format 2); run `lvpm lock` to update it"

  # AC9
  Scenario: lvpm lock rewrites a format 1 lock as format 2
    Given an lvpm.lock in format 1 with "caraya" marked dev = true
    When I run "lvpm lock"
    Then lvpm.lock is format 2

  # AC9
  Scenario: --locked refuses a format 1 lock without changing anything
    Given an lvpm.lock in format 1 with "caraya" marked dev = true
    When I run "lvpm sync --locked --prefix dest"
    Then the command fails saying the lock format is outdated
    And dest and lvpm.lock are byte-identical to before
