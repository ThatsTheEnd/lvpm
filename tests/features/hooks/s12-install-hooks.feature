Feature: install hooks run only for allowed packages

  Scenario Outline: a hooks value other than "run" is refused when the manifest is read
    Given a project that <relation> "alpha" with hooks set to <value>
    When I run "lvpm lock"
    Then the exit code is non-zero
    And the error says "<table> alpha: hooks must be \"run\" (delete the key to keep hooks off)"
    And no lvpm.lock is written

    Examples:
      | relation       | value  | table              |
      | depends on     | "yes"  | [dependencies]     |
      | depends on     | "Run"  | [dependencies]     |
      | depends on     | ""     | [dependencies]     |
      | depends on     | true   | [dependencies]     |
      | dev-depends on | "skip" | [dev-dependencies] |

  Scenario: hooks = "run" is accepted on every dependency table
    Given "first" lists "alpha" 1.0, "beta" 1.0
    And a project that depends on "alpha" with hooks set to "run"
    And the project dev-depends on "beta" with hooks set to "run"
    When I run "lvpm lock"
    Then the command succeeds

  Scenario: hooks = "run" next to github keeps both
    Given a project that depends on "pkg" with github "o/r", version "*" and hooks set to "run"
    When the manifest is read
    Then it is accepted with the github source and hooks allowed for "pkg"

  Scenario: lvpm add of a newer version keeps hooks = "run"
    Given "first" lists "alpha" 1.0 and "alpha" 2.0
    And a project that depends on "alpha" "1.0" with hooks set to "run" and lvpm.lock is current
    When I run "lvpm add alpha@2.0 --prefix dest"
    Then the command succeeds
    And lvpm.toml has exactly the line: alpha = { version = "2.0", hooks = "run" }

  Scenario: lvpm config set hooks.run writes the global allowlist
    When I run "lvpm config set hooks.run pilot_a delacor_lib_qmh"
    And I run "lvpm config get hooks.run"
    Then the output lists "pilot_a" and "delacor_lib_qmh"

  Scenario: config set hooks.run replaces the list
    Given the user config has hooks.run "old_pkg"
    When I run "lvpm config set hooks.run new_pkg"
    And I run "lvpm config get hooks.run"
    Then the output lists "new_pkg" and not "old_pkg"

  Scenario: commas and spaces both separate names
    When I run "lvpm config set hooks.run a,b"
    And I run "lvpm config get hooks.run"
    Then the output lists "a" and "b"

  Scenario: LVPM_CONFIG_HOOKS_RUN overrides the file
    Given the user config has hooks.run "pilot_a"
    And LVPM_CONFIG_HOOKS_RUN is "other_pkg" for the command
    When I run "lvpm config get hooks.run"
    Then the output lists "other_pkg" and not "pilot_a"

  Scenario: an unknown config key names hooks.run among the supported ones
    When I run "lvpm config get hooks.allow"
    Then the exit code is non-zero
    And the error says "hooks.run"
