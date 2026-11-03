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
