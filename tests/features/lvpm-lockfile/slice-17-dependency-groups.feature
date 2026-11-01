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

  # AC1, AC2
  Scenario Outline: the selection decides what is installed
    Given lvpm.toml has [dev-dependencies] "caraya" "*"; [dependency-groups.ci] "caraya" "*", "ci_tools" "*", "delta" "*" and a current lvpm.lock
    When I run "lvpm sync <flags> --prefix dest"
    Then dest's lvpm-installed packages are exactly <installed>

    Examples:
      | flags                 | installed                      |
      |                       | alpha, caraya, delta           |
      | --no-dev              | alpha                          |
      | --group ci            | alpha, caraya, ci_tools, delta |
      | --no-dev --group ci   | alpha, caraya, ci_tools, delta |
      | --group ci --group ci | alpha, caraya, ci_tools, delta |
      | --group dev           | alpha, caraya, delta           |
      | --group Dev           | alpha, caraya, delta           |

  # AC1
  Scenario: an unselected group's package is announced, then removed
    Given lvpm.toml has [dependency-groups.ci] "ci_tools" "*", a current lvpm.lock, and dest synced with "--group ci"
    When I run "lvpm sync --prefix dest"
    Then the line "ci_tools: only the group ci needs it, and it is not selected" comes before the removal output
    And dest's lvpm-installed packages are exactly alpha

  # AC2
  Scenario Outline: a package both [dependencies] and a group need survives every selection
    Given lvpm.toml has [dependency-groups.ci] "ci_tools" "*", "alpha" "*" and a current lvpm.lock
    When I run "lvpm sync <flags> --prefix dest"
    Then "alpha" is installed in dest

    Examples:
      | flags      |
      |            |
      | --no-dev   |
      | --group ci |

  # AC8
  Scenario Outline: an unknown group fails before anything changes
    Given lvpm.toml has [dependency-groups.ci] "ci_tools" "*", <lock>, and "beta" 1.0 installed in dest
    When I run "lvpm sync --group nope --prefix dest"
    Then the command fails saying "unknown group 'nope'; defined groups: ci, dev"
    And dest, lvpm.lock and lvpm.toml are byte-identical to before

    Examples:
      | lock                |
      | a current lvpm.lock |
      | a stale lvpm.lock   |
      | no lvpm.lock        |

  # AC1
  Scenario: an empty group is a known group
    Given lvpm.toml has [dependency-groups.ci] empty and a current lvpm.lock
    When I run "lvpm sync --group ci --prefix dest"
    Then the command succeeds and dest's lvpm-installed packages are exactly alpha

  # AC8
  Scenario: --group dev is known without a dev table
    Given a current lvpm.lock and no [dev-dependencies]
    When I run "lvpm sync --group dev --prefix dest"
    Then the command succeeds

  Scenario: --group dev with --no-dev is refused
    When I run "lvpm sync --group dev --no-dev --prefix dest"
    Then the command fails saying "--group dev" and "--no-dev" cannot be combined

  # AC1
  Scenario: bare install takes --group like sync
    Given lvpm.toml has [dependency-groups.ci] "ci_tools" "*" and a current lvpm.lock
    When I run "lvpm install --group ci --prefix dest"
    Then dest's lvpm-installed packages are exactly alpha and ci_tools
    And lvpm.lock is byte-identical to before

  Scenario: --group is refused with a package argument
    When I run "lvpm install beta --group ci --prefix dest"
    Then the command fails as a usage error naming "--group"
