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
