Feature: a dependency can name a GitHub repo

  Scenario Outline: a dependency table lvpm cannot use fails naming the package
    Given a project whose lvpm.toml has the dependency line <line>
    When I run "lvpm lock"
    Then the command fails naming "[dependencies] pkg" and <said>
    And no lvpm.lock is written

    Examples:
      | line                                                        | said                           |
      | pkg = { github = "o/r", index = "internal", version = "*" } | "github and index"             |
      | pkg = { index = "internal", version = "*" }                 | "index is not supported yet"   |
      | pkg = { github = "o/r", version = "*", hooks = "run" }      | "unknown key \\"hooks\\""        |
      | pkg = { github = "o/r" }                                    | "a table needs version"        |
      | pkg = { github = "o", version = "*" }                       | "is not \\"owner/repo\\""        |
      | pkg = { github = "o/r/x", version = "*" }                   | "is not \\"owner/repo\\""        |
      | pkg = { github = "/r", version = "*" }                      | "is not \\"owner/repo\\""        |
      | pkg = { github = "o/", version = "*" }                      | "is not \\"owner/repo\\""        |
      | pkg = { github = "", version = "*" }                        | "is not \\"owner/repo\\""        |
      | pkg = { github = 5, version = "*" }                         | "is not \\"owner/repo\\""        |
      | pkg = { github = "https://github.com/o/r", version = "*" }  | "drop \\"https://github.com/\\"" |
      | pkg = { github = "o/r", version = "~1.2" }                  | "unsupported version constraint" |

  Scenario: a dev dependency is refused with its own table name
    Given a project whose lvpm.toml has the dev dependency line pkg = { github = "o/r", index = "x", version = "*" }
    When I run "lvpm lock"
    Then the command fails naming "[dev-dependencies] pkg"

  Scenario: the refusal happens when the manifest is read, not only at lock
    Given a project whose lvpm.toml has the dependency line pkg = { github = "o/r", index = "internal", version = "*" }
    When I run "lvpm sync --prefix dest"
    Then the command fails naming "[dependencies] pkg" and "github and index"

  Scenario: the short form is unchanged
    Given a project whose lvpm.toml depends on "pkg" ">=1.2" with a feed that lists pkg 1.2
    When I run "lvpm lock"
    Then the command succeeds
    And lvpm.lock has no [inputs.github] table
