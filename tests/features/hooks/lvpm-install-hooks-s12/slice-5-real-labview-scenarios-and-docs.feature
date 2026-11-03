  @labview @unsafe
  Scenario: a global install of the hook fixture skips it and says how to allow it
    Given the hook fixture package is not installed
    When I run "lvpm install lvpm_test_fixture --global"
    Then the command succeeds
    And the PostInstall hook did not run
    And stderr says "lvpm_test_fixture: skipped install hook PostInstall"
    And stderr says "lvpm run-hooks lvpm_test_fixture"

  @labview @unsafe
  Scenario: following the run-it-now line runs the hook and re-enables uninstall hooks
    Given the hook fixture was installed with its PostInstall skipped
    When I run "lvpm run-hooks lvpm_test_fixture"
    Then the PostInstall hook ran for "lvpm_test_fixture"
    And "lvpm list" no longer shows "PostInstall" as skipped for "lvpm_test_fixture"

  @labview @unsafe
  Scenario: following the always-run-it line runs the hook on the next install
    Given the hook fixture package is not installed
    And the user config has hooks.run "lvpm_test_fixture"
    When I run "lvpm install lvpm_test_fixture --global"
    Then the PostInstall hook ran for "lvpm_test_fixture"
    And stderr does not say "skipped install hook"

  Scenario: the docs state the new default
    Given the repository docs
    When they are read
    Then README.md mentions "hooks are off by default"
    And docs/manifest.md mentions "hooks = \"run\""
    And docs/usage.md mentions "hooks.run" and "skipped install hook"
    And docs/labview-integration.md mentions "lvpm run-hooks" and "hooks.run"
    And docs/cli.md mentions "hooks.run"
