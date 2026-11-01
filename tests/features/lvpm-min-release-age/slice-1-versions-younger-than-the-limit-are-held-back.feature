Feature: lvpm does not choose a package version younger than min-age-days

  Background:
    Given a project with labview "2024" whose [sources] are "first" then "second"

  Scenario: without min-age-days the newest version is locked
    Given "first" lists "alpha" 1.0 released 30 days ago and "alpha" 1.1 released 3 days ago
    And the project depends on "alpha" ">=1.0"
    When I run "lvpm lock"
    Then the command succeeds
    And lvpm.lock takes "alpha" 1.1 from "first"

  Scenario: min-age-days = 0 is off
    Given the project sets min-age-days to 0
    And "first" lists "alpha" 1.0 released 30 days ago and "alpha" 1.1 released 3 days ago
    And the project depends on "alpha" ">=1.0"
    When I run "lvpm lock"
    Then lvpm.lock takes "alpha" 1.1 from "first"

  Scenario: a version younger than the limit is held back for the newest old enough
    Given the project sets min-age-days to 14
    And "first" lists "alpha" 1.0 released 30 days ago and "alpha" 1.1 released 3 days ago
    And the project depends on "alpha" ">=1.0"
    When I run "lvpm lock"
    Then the command succeeds
    And lvpm.lock takes "alpha" 1.0 from "first"

  Scenario: a held-back version never falls through to a later source
    Given the project sets min-age-days to 14
    And "first" lists "alpha" 1.1 released 3 days ago
    And "second" lists "alpha" 1.1 released 30 days ago and "alpha" 2.0 released 30 days ago
    And the project depends on "alpha" ">=1.0"
    When I run "lvpm lock"
    Then the command fails with the min-age-days error for "alpha" ">=1.0" listing 1.1
    And the error does not mention version 2.0
    And no lvpm.lock is written

  Scenario: a transitive requirement is held to the limit too
    Given the project sets min-age-days to 14
    And "first" lists "alpha" 1.0 released 30 days ago requiring "beta>=2.0"
    And "first" lists "beta" 2.0 released 30 days ago and "beta" 2.1 released 3 days ago
    And the project depends on "alpha" "*"
    When I run "lvpm lock"
    Then lvpm.lock takes "beta" 2.0 from "first"

  Scenario: lvpm sync without a lock applies the limit
    Given the project sets min-age-days to 14
    And "first" lists "alpha" 1.0 released 30 days ago and "alpha" 1.1 released 3 days ago
    And the project depends on "alpha" ">=1.0"
    When I run "lvpm sync --prefix dest"
    Then lvpm.lock takes "alpha" 1.0 from "first"

  Scenario: lvpm add without a version writes the newest eligible version
    Given the project sets min-age-days to 14
    And "first" lists "alpha" 1.0 released 30 days ago and "alpha" 1.1 released 3 days ago
    When I run "lvpm add alpha --prefix dest"
    Then lvpm.lock takes "alpha" 1.0 from "first"

  Scenario: lvpm lock --upgrade does not move to a version younger than the limit
    Given the project sets min-age-days to 14
    And "first" lists "alpha" 1.0 released 30 days ago
    And the project depends on "alpha" ">=1.0"
    And I have run "lvpm lock"
    When "first" also lists "alpha" 1.1 released 3 days ago
    And I run "lvpm lock --upgrade"
    Then the command succeeds
    And lvpm.lock takes "alpha" 1.0 from "first"

  # Exact pins
  Scenario: an exact pin younger than the limit is locked
    Given the project sets min-age-days to 14
    And "first" lists "alpha" 1.0 released 30 days ago and "alpha" 1.1 released 3 days ago
    And the project depends on "alpha" "1.1"
    When I run "lvpm lock"
    Then lvpm.lock takes "alpha" 1.1 from "first"

  Scenario: an exact dev-dependency pin is exempt too
    Given the project sets min-age-days to 14
    And "first" lists "alpha" 1.0 released 30 days ago and "alpha" 1.1 released 3 days ago
    And the project dev-depends on "alpha" "1.1"
    When I run "lvpm lock"
    Then lvpm.lock takes "alpha" 1.1 from "first"

  Scenario: lvpm add with an explicit version is exempt
    Given the project sets min-age-days to 14
    And "first" lists "alpha" 1.0 released 30 days ago and "alpha" 1.1 released 3 days ago
    When I run "lvpm add alpha@1.1 --prefix dest"
    Then lvpm.lock takes "alpha" 1.1 from "first"

  Scenario: an exact pin stays exempt when another package also requires the package
    Given the project sets min-age-days to 14
    And "first" lists "alpha" 1.0 released 30 days ago requiring "beta>=2.0"
    And "first" lists "beta" 2.0 released 30 days ago and "beta" 2.1 released 3 days ago
    And the project depends on "alpha" "*" and "beta" "2.1"
    When I run "lvpm lock"
    Then lvpm.lock takes "beta" 2.1 from "first"

  Scenario: an exact pin exempts only the pinned package, not its requirements
    Given the project sets min-age-days to 14
    And "first" lists "alpha" 1.1 released 3 days ago requiring "beta>=2.0"
    And "first" lists "beta" 2.0 released 30 days ago and "beta" 2.1 released 3 days ago
    And the project depends on "alpha" "1.1"
    When I run "lvpm lock"
    Then lvpm.lock takes "alpha" 1.1 from "first"
    And lvpm.lock takes "beta" 2.0 from "first"

  # Undated versions
  Scenario: a feed entry without a release date is eligible
    Given the project sets min-age-days to 14
    And "first" lists "alpha" 1.0 released 30 days ago and "alpha" 1.1 with no release date
    And the project depends on "alpha" ">=1.0"
    When I run "lvpm lock"
    Then lvpm.lock takes "alpha" 1.1 from "first"

  Scenario: a feed entry with an unreadable release date is eligible
    Given the project sets min-age-days to 14
    And "first" lists "alpha" 1.0 released 30 days ago and "alpha" 1.1 with release date "???, 19 ??? 2016 15:46:24 0530"
    And the project depends on "alpha" ">=1.0"
    When I run "lvpm lock"
    Then lvpm.lock takes "alpha" 1.1 from "first"

  Scenario: a package in a local folder is eligible whatever the limit
    Given the project sets min-age-days to 14
    And the local folder holds "alpha" 1.0
    And the project depends on "alpha" "*"
    When I run "lvpm lock"
    Then lvpm.lock takes "alpha" 1.0 from "local"

  Scenario Outline: a release date in each feed shape is read, not treated as undated
    Given the project sets min-age-days to 14
    And "first" lists "alpha" 1.0 released 30 days ago and "alpha" 1.1 released 3 days ago written as <shape>
    And the project depends on "alpha" ">=1.0"
    When I run "lvpm lock"
    Then lvpm.lock takes "alpha" 1.0 from "first"

    Examples:
      | shape                 |
      | a signed offset       |
      | GMT                   |
      | a sign-less offset    |

  # Failures
  Scenario: no version old enough fails listing what was held back
    Given the project sets min-age-days to 14
    And "first" lists "alpha" 1.0 released 30 days ago, "alpha" 1.1 released 3 days ago and "alpha" 1.2 released 2 days ago
    And the project depends on "alpha" ">=1.1"
    When I run "lvpm lock"
    Then the command fails with the min-age-days error for "alpha" ">=1.1" listing 1.2 and 1.1
    And the error does not mention version 1.0
    And no lvpm.lock is written

  Scenario: a LabVIEW-gated version is not listed in the min-age-days error
    Given the project sets min-age-days to 14
    And "first" lists "alpha" 1.1 released 3 days ago and "alpha" 3.0 for LabVIEW 2026 released 30 days ago
    And the project depends on "alpha" ">=1.1"
    When I run "lvpm lock"
    Then the command fails with the min-age-days error for "alpha" ">=1.1" listing 1.1
    And the error does not mention version 3.0

  Scenario: a transitive requirement with no old enough version fails naming who needs it
    Given the project sets min-age-days to 14
    And "first" lists "alpha" 1.0 released 30 days ago requiring "beta>=2.0"
    And "first" lists "beta" 2.0 released 3 days ago
    And the project depends on "alpha" "*"
    When I run "lvpm lock"
    Then the command fails with the min-age-days error for "beta" ">=2.0" listing 2.0
    And the error says "it is required by alpha"
    And no lvpm.lock is written

  Scenario Outline: a min-age-days that is not a whole number of days is refused
    Given the project's [sources] has the line 'min-age-days = <value>'
    When I run "lvpm lock"
    Then the command fails saying "[sources] min-age-days must be a whole number of days, 0 or more (got <value>)"
    And no lvpm.lock is written

    Examples:
      | value      |
      | -1         |
      | "14"       |
      | 14.5       |
      | true       |
      | 4294967296 |

  # The lock
  Scenario Outline: the lock records a non-zero min-age-days only
    Given the project sets min-age-days to <value>
    And "first" lists "alpha" 1.0 released 30 days ago
    And the project depends on "alpha" "*"
    When I run "lvpm lock"
    Then lvpm.lock <records>

    Examples:
      | value  | records                           |
      | absent | has no min-age-days entry         |
      | 0      | has no min-age-days entry         |
      | 14     | records min-age-days 14           |

  Scenario Outline: changing min-age-days makes the lock stale
    Given the project sets min-age-days to <before>
    And "first" lists "alpha" 1.0 released 30 days ago
    And the project depends on "alpha" "*"
    And I have run "lvpm lock"
    When the project sets min-age-days to <after>
    And I run "lvpm lock --check"
    Then the command fails saying "min-age-days changed"

    Examples:
      | before | after  |
      | 14     | 7      |
      | absent | 14     |
      | 14     | absent |

  Scenario Outline: 0 and absent are the same for the lock
    Given the project sets min-age-days to <before>
    And "first" lists "alpha" 1.0 released 30 days ago
    And the project depends on "alpha" "*"
    And I have run "lvpm lock"
    When the project sets min-age-days to <after>
    And I run "lvpm lock --check"
    Then the command succeeds

    Examples:
      | before | after  |
      | absent | 0      |
      | 0      | absent |

  Scenario: days passing do not make the lock stale
    Given the project sets min-age-days to 14
    And "first" lists "alpha" 1.0 released 30 days ago and "alpha" 1.1 released 3 days ago
    And the project depends on "alpha" ">=1.0"
    And I have run "lvpm lock"
    When "first" now lists "alpha" 1.1 as released 30 days ago
    And I run "lvpm lock --check"
    Then the command succeeds
    And lvpm.lock takes "alpha" 1.0 from "first"

  Scenario: raising min-age-days re-locks a version that is now too new
    Given the project sets min-age-days to 7
    And "first" lists "alpha" 1.0 released 30 days ago and "alpha" 1.1 released 10 days ago
    And the project depends on "alpha" ">=1.0"
    And I have run "lvpm lock"
    When the project sets min-age-days to 14
    And I run "lvpm lock"
    Then lvpm.lock takes "alpha" 1.0 from "first"
