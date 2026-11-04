  # [3.1] the verdict
  Scenario: a global install skips a PostInstall hook nobody allowed
    Given a LabVIEW installation and no project
    And a package "hooked" declaring a PostInstall hook
    When "hooked" is installed into the installation
    Then the install succeeds
    And no hook is started
    And the install record lists "PostInstall" as skipped

  Scenario: a package in hooks.run has its hook started
    Given a LabVIEW installation and no project
    And the user config has hooks.run "Hooked"
    And a package "hooked" declaring a PostInstall hook
    When "hooked" is installed into the installation
    Then the PostInstall hook of "hooked" is started

  Scenario: sync --global starts the hook of a package with hooks = "run"
    Given a project that depends on "hooked" with hooks set to "run" and a current lock
    When the project is synced into the LabVIEW installation
    Then the PostInstall hook of "hooked" is started
    And no hooks-skipped warning is given

  Scenario: sync --global --hooks starts a hook nobody allowed
    Given a project that depends on "hooked" without hooks = "run" and a current lock
    When the project is synced into the LabVIEW installation with --hooks
    Then the PostInstall hook of "hooked" is started

  Scenario: only the package that is not allowed is skipped
    Given a project that depends on "hooked" with hooks set to "run" and on "other_hooked" without it, with a current lock
    When the project is synced into the LabVIEW installation
    Then the PostInstall hook of "hooked" is started
    And no hook of "other_hooked" is started

  Scenario: hooks.run does not allow a project sync
    Given the user config has hooks.run "hooked"
    And a project that depends on "hooked" without hooks = "run" and a current lock
    When the project is synced into the LabVIEW installation
    Then no hook is started

  Scenario: hooks = "run" in a project does not allow lvpm install
    Given a project that depends on "hooked" with hooks set to "run"
    And a package "hooked" declaring a PostInstall hook
    When "hooked" is installed with lvpm install
    Then no hook is started

  Scenario: add --global of a hooks = "run" line starts its hook
    Given a project that depends on "hooked" with hooks set to "run"
    When "lvpm add hooked@2.0 --global" runs
    Then the PostInstall hook of "hooked" is started

  Scenario: a transitive dependency listed with version "*" and hooks = "run" starts its hook
    Given "beta" depends on "hooked"
    And a project that depends on "beta" and on "hooked" "*" with hooks set to "run", with a current lock
    When the project is synced into the LabVIEW installation
    Then the PostInstall hook of "hooked" is started

  Scenario: hooks = "run" in a dependency's own lvpm.toml allows nothing
    Given a package "beta" from a source tree whose own lvpm.toml lists "hooked" with hooks set to "run"
    And a project that depends on "beta" and a current lock
    When the project is synced into the LabVIEW installation
    Then no hook of "hooked" is started

  Scenario: --hooks starts every hook of that one command
    Given a LabVIEW installation and no project
    And packages "hooked" and "other_hooked", each declaring PreInstall and PostInstall hooks
    When both are installed with --hooks
    Then the PreInstall and PostInstall hooks of both packages are started

  Scenario: headless with --hooks starts the hook
    Given LabVIEW runs headless
    And a package "hooked" declaring a PostInstall hook
    When "hooked" is installed into the installation with --hooks
    Then the PostInstall hook of "hooked" is started

  Scenario: a venv install runs no hook and reports as before
    Given a project with a venv that depends on "hooked" with hooks set to "run"
    When the project is synced into its venv
    Then no hook is started
    And the hooks-not-run line is given, not the hooks-skipped warning

  # [3.2] the warning
  Scenario: the skip warns with the config line
    Given a LabVIEW installation and no project
    And the user config has hooks.run "other_pkg"
    And a package "hooked" declaring a PostInstall hook
    When "hooked" is installed into the installation
    Then a hooks-skipped warning names "hooked", "PostInstall" and reason "not allowed"
    And its allow line is config set with "other_pkg", "hooked"

  Scenario: two skipped packages share one config line that keeps both
    Given a LabVIEW installation and no project
    And the user config has hooks.run "other_pkg"
    And packages "a_hooked" and "b_hooked", each declaring a PostInstall hook
    When both are installed into the installation
    Then each gets a hooks-skipped warning
    And exactly one allow line is given: config set with "other_pkg", "a_hooked", "b_hooked"

  Scenario: a listed project line is told to add the key
    Given a project that depends on "hooked" with github "o/r" and a current lock
    When the project is synced into the LabVIEW installation
    Then the allow line says to add the key to the "hooked" line in [dependencies]

  Scenario: an unlisted transitive dependency is offered a version "*" line
    Given "beta" depends on "hooked"
    And a project that depends on "beta" and a current lock
    When the project is synced into the LabVIEW installation
    Then the allow line is a new [dependencies] line for "hooked" with version "*"

  Scenario: headless skips an allowed package and says the allowlist does not apply
    Given LabVIEW runs headless
    And the user config has hooks.run "hooked"
    And a package "hooked" declaring a PostInstall hook
    When "hooked" is installed into the installation
    Then no hook is started
    And a hooks-skipped warning names "hooked" with reason "headless"
    And its allow line is "pass --hooks" and it has no run-it-now line

  Scenario: headless and not allowed gets the headless reason
    Given LabVIEW runs headless
    And a package "hooked" declaring a PostInstall hook
    When "hooked" is installed into the installation
    Then a hooks-skipped warning names "hooked" with reason "headless"

  Scenario: a PreInstall-only package gets no run-hooks line
    Given a LabVIEW installation and no project
    And a package "pre_only" declaring only a PreInstall hook
    When "pre_only" is installed into the installation
    Then no hook is started
    And the warning has no run-it-now line
    And the warning has an allow line

  Scenario: a dry run says what it would skip and writes nothing
    Given a LabVIEW installation and no project
    And a package "hooked" declaring a PostInstall hook
    When "hooked" is installed into the installation as a dry run
    Then a dry-run hooks-skipped warning names "hooked" with no run-it-now line
    And no hook is started
    And no install record is written

  Scenario: allowing an installed package with a skipped hook says to run it
    Given "hooked" was installed with its PostInstall skipped
    And the user config now has hooks.run "hooked"
    When "hooked" is installed into the installation again, same version
    Then no hook is started
    And a hooks-skipped warning names "hooked" with reason "already installed" and a run-it-now line

  Scenario: a failure after a skipped package still warns about it
    Given a LabVIEW installation and no project
    And "hooked" declaring a PostInstall hook, then "broken" that fails to unpack
    When both are installed into the installation
    Then the command fails
    And a hooks-skipped warning names "hooked"
