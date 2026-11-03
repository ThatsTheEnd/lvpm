  Scenario: a package whose install hook was skipped runs no uninstall hook
    Given "hooked" was installed into the installation with its PostInstall skipped
    And "hooked" declares a PreUninstall hook
    When "hooked" is uninstalled
    Then no hook is started
    And the uninstall says its uninstall hooks did not run because its install hook did not run
    And it says to run lvpm run-hooks first

  Scenario: a package with both install hooks gets its uninstall hooks back after run-hooks
    Given "hooked" declares PreInstall, PostInstall and PreUninstall hooks
    And "hooked" was installed with both install hooks skipped
    And its skipped hooks were cleared as a successful run-hooks clears them
    When "hooked" is uninstalled
    Then the PreUninstall hook of "hooked" is started

  Scenario: a failed PostInstall keeps the uninstall hooks off
    Given "hooked" was installed and its PostInstall hook failed
    When "hooked" is uninstalled
    Then no hook is started
    And the uninstall says its install hook did not run

  Scenario: a headless-installed record from before S12 no longer runs its uninstall hooks
    Given an install record for "hooked" written by a headless install, listing every hook as skipped
    When "hooked" is uninstalled
    Then no hook is started

  Scenario: a package with only uninstall hooks runs none unless it was allowed
    Given "unhooked" declares only a PreUninstall hook
    And "unhooked" was installed into the installation without being allowed
    When "unhooked" is uninstalled
    Then no hook is started

  Scenario: a global record from before S12 keeps running its uninstall hooks
    Given an install record for "hooked" with no skipped hooks and a PreUninstall hook
    When "hooked" is uninstalled
    Then the PreUninstall hook of "hooked" is started

  Scenario: a failed run-hooks leaves the record unchanged
    Given "hooked" was installed with its PostInstall skipped
    When lvpm run-hooks "hooked" fails because the hook VI is gone
    Then the install record is unchanged
