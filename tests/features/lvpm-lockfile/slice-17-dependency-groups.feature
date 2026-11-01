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

  # AC6, AC7
  Scenario: add --group writes the group's table and installs it
    When I run "lvpm add ci_tools --group ci --prefix dest"
    Then [dependency-groups.ci] says "ci_tools" = ">=2.0"
    And the output says "created [dependency-groups.ci], a new group"
    And lvpm.lock gives "ci_tools" groups ["ci"]
    And "ci_tools" 2.0 is installed

  # AC11
  Scenario: add --group reports the group in JSON
    When I run "lvpm add ci_tools --group ci --json --prefix dest"
    Then the manifest-edited event has "dev": false and "group": "ci"

  # AC10
  Scenario: a later add keeps what a group installed
    Given lvpm.toml has [dependency-groups.ci] "ci_tools" "*", a current lvpm.lock, and dest synced with "--group ci"
    When I run "lvpm add beta --prefix dest"
    Then dest's lvpm-installed packages are exactly alpha, beta, ci_tools

  # AC10
  Scenario: a later remove keeps what a group installed
    Given lvpm.toml has [dependencies] "beta" "*"; [dependency-groups.ci] "ci_tools" "*", a current lvpm.lock, and dest synced with "--group ci"
    When I run "lvpm remove beta --prefix dest"
    Then dest's lvpm-installed packages are exactly alpha, ci_tools

  # AC10
  Scenario: add does not install another group
    Given lvpm.toml has [dependency-groups.ci] "ci_tools" "*" and a current lvpm.lock
    When I run "lvpm add beta --prefix dest"
    Then dest's lvpm-installed packages are exactly alpha, beta

  # AC6
  Scenario Outline: --group dev is the same as --dev
    Given <setup>
    When I run "lvpm <command> caraya <flag> --prefix dest"
    Then lvpm.toml and lvpm.lock are byte-identical to the result of "lvpm <command> caraya --dev --prefix dest" from the same start

    Examples:
      | command | flag        | setup                                                                 |
      | add     | --group dev | a current lvpm.lock                                                   |
      | add     | --group Dev | a current lvpm.lock                                                   |
      | remove  | --group dev | lvpm.toml has [dev-dependencies] "caraya" "*" and a current lvpm.lock |

  # AC6
  Scenario: remove --group removes from the group
    Given lvpm.toml has [dependency-groups.ci] "ci_tools" "*", a current lvpm.lock, and dest synced with "--group ci"
    When I run "lvpm remove ci_tools --group ci --prefix dest"
    Then "ci_tools" is gone from [dependency-groups.ci], lvpm.lock and dest

  Scenario: remove from the wrong table lists every table holding it
    Given lvpm.toml has [dependencies] "beta" "*"; [dependency-groups.ci] "beta" "*"
    When I run "lvpm remove beta --dev --prefix dest"
    Then the command fails saying "beta" is in [dependencies] and [dependency-groups.ci], with one next line each: "lvpm remove beta" and "lvpm remove beta --group ci"

  Scenario Outline: remove --group of something that is not there fails
    Given lvpm.toml has [dependency-groups.ci] "ci_tools" "*"
    When I run "lvpm remove <name> --group <group> --prefix dest"
    Then the command fails naming <named>
    And lvpm.toml and lvpm.lock are byte-identical to before

    Examples:
      | name     | group | named                                           |
      | beta     | ci    | "beta" and "[dependency-groups.ci]"             |
      | ci_tools | nope  | "unknown group 'nope'; defined groups: ci, dev" |

  Scenario Outline: a bad --group on add is refused before any change
    When I run "lvpm add beta <flags> --prefix dest"
    Then the command fails naming <named>
    And lvpm.toml is byte-identical to before

    Examples:
      | flags            | named                      |
      | --group ci --dev | "--group" and "--dev"      |
      | --group a.b      | "letters, digits, - and _" |

  Scenario: the docs describe the groups that are built
    Then docs/manifest.md mentions "[dependency-groups.<name>]"
    And docs/manifest.md mentions "--group"
    And docs/manifest.md mentions "--no-dev"
    And docs/manifest.md does not mention "not built in this fork"
    And docs/usage.md mentions "lvpm sync --group"
