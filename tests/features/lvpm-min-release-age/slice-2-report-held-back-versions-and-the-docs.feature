Feature: lvpm reports what min-age-days held back

  Background:
    Given a project with labview "2024" whose [sources] are "first" then "second"
    And the project sets min-age-days to 14

  Scenario: a held-back newer version is reported
    Given "first" lists "alpha" 1.0 released 30 days ago and "alpha" 1.1 released 3 days ago
    And the project depends on "alpha" ">=1.0"
    When I run "lvpm lock"
    Then stderr has the held-back line for "alpha" 1.1 using 1.0

  Scenario: only the newest held-back version is reported
    Given "first" lists "alpha" 1.0 released 30 days ago, "alpha" 1.1 released 5 days ago and "alpha" 1.2 released 2 days ago
    And the project depends on "alpha" ">=1.0"
    When I run "lvpm lock"
    Then stderr has the held-back line for "alpha" 1.2 using 1.0
    And stderr has no held-back line for "alpha" 1.1

  Scenario: a held-back transitive version is reported
    Given "first" lists "alpha" 1.0 released 30 days ago requiring "beta>=2.0"
    And "first" lists "beta" 2.0 released 30 days ago and "beta" 2.1 released 3 days ago
    And the project depends on "alpha" "*"
    When I run "lvpm lock"
    Then stderr has the held-back line for "beta" 2.1 using 2.0

  Scenario: the held-back event is in the JSON output
    Given "first" lists "alpha" 1.0 released 30 days ago and "alpha" 1.1 released 3 days ago
    And the project depends on "alpha" ">=1.0"
    When I run "lvpm lock --json"
    Then the JSON events include one "held_back" event for "alpha" 1.1 with chosen 1.0, min_age_days 14 and its release and eligible dates

  Scenario: nothing is printed when nothing was held back
    Given "first" lists "alpha" 1.0 released 30 days ago
    And the project depends on "alpha" "*"
    When I run "lvpm lock --json"
    Then stderr has no held-back line
    And the JSON events include no "held_back" event

  Scenario: an exact pin is not reported as held back
    Given "first" lists "alpha" 1.0 released 30 days ago and "alpha" 1.1 released 3 days ago
    And the project depends on "alpha" "1.0"
    When I run "lvpm lock"
    Then stderr has no held-back line

  Scenario: a newer version gated by LabVIEW is not reported as held back by age
    Given "first" lists "alpha" 1.0 released 30 days ago and "alpha" 3.0 for LabVIEW 2026 released 3 days ago
    And the project depends on "alpha" "*"
    When I run "lvpm lock"
    Then lvpm.lock takes "alpha" 1.0 from "first"
    And stderr has no held-back line

  Scenario: lvpm lock --check prints no held-back line
    Given "first" lists "alpha" 1.0 released 30 days ago and "alpha" 1.1 released 3 days ago
    And the project depends on "alpha" ">=1.0"
    And I have run "lvpm lock"
    When I run "lvpm lock --check"
    Then the command succeeds
    And stderr has no held-back line

  Scenario: the manifest guide documents min-age-days
    Then docs/manifest.md mentions "min-age-days"
    And docs/manifest.md mentions "an exact pin is exempt from min-age-days"
    And docs/manifest.md mentions "no release date"
    And docs/manifest.md mentions "the release date the feed lists"
    And docs/manifest.md mentions "lvpm install --global"
    And docs/reference/json-output.md mentions "held_back"
