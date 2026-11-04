Feature: lvpm build checks the library's members first
  Before writing, `lvpm build` checks the VIs the library lists, as
  `lvpm check` does, and writes nothing when one breaks a rule. The fixture VIs
  are in tests/fixtures/check, where README.md lists each one's links. The
  package the fixture VIs link into is `check_fixture_dep`, and its file shows
  in a finding as `<vilib>/Check Fixture/Dep.vi`.

  Background:
    Given a check project with labview ">=2026" and [distribution] naming "Lib.lvlib", holding "Lib.lvlib", "Good.vi" and "Helper.vi"

  Scenario: a clean library builds
    When lvpm build runs on it
    Then it exits 0, dist holds the package, and stderr is empty
    And stdout has a line starting "built "

  Scenario: a member with a missing subVI refuses the build
    Given "Broken.vi" is a member of "Lib.lvlib" and the folder "Sub" is empty
    And [dependencies] names "spare"
    When lvpm build runs on it
    Then it exits 1
    And stdout has the line "Broken.vi: link not found: Sub/Gone.vi"
    And stdout has the line "VIs checked: 3, problems: 1, links not checked: 0"
    And stderr has the line "build refused: 1 problem(s) in library members, nothing written; fix them or pass --no-check"
    And stderr has no line containing "no member links to"
    And dist holds no .lvpkg

  Scenario: a refused build leaves the previous package
    Given dist holds a built package for this version
    And "Broken.vi" is a member of "Lib.lvlib" and the folder "Sub" is empty
    When lvpm build runs on it
    Then it exits 1 and that package is byte-identical to before
    And stderr has the line "build refused: 1 problem(s) in library members, nothing written; fix them or pass --no-check"
    And dist holds only that package

  Scenario: every broken member is counted
    Given "Broken.vi" is a member of "Lib.lvlib" and the folder "Sub" is empty
    And "Broken2.vi", a copy of "Broken.vi", is also a member
    When lvpm build runs on it
    Then it exits 1
    And stdout has the line "Broken.vi: link not found: Sub/Gone.vi"
    And stdout has the line "Broken2.vi: link not found: Sub/Gone.vi"
    And stderr has the line "build refused: 2 problem(s) in library members, nothing written; fix them or pass --no-check"
    And dist holds no .lvpkg

  Scenario: a member the library lists but the disk lacks stops the build
    Given the library lists "Gone.vi", which is not on disk
    When lvpm build runs on it
    Then it exits 1
    And stderr has a line containing "Gone.vi, which is not on disk"
    And dist holds no .lvpkg

  Scenario: a member in a dot-folder is checked
    Given "Broken.vi" is a member of "Lib.lvlib" in the dot-folder ".hidden"
    When lvpm build runs on it
    Then it exits 1
    And stdout has the line ".hidden/Broken.vi: link not found: .hidden/Sub/Gone.vi"
    And dist holds no .lvpkg

  Scenario: a broken VI outside the library does not stop the build
    Given the project also holds "Broken.vi", not a member
    When lvpm build runs on it
    Then it exits 0 and dist holds the package

  Scenario: a member linking into an undeclared package refuses the build
    Given member "UsesDep.vi" links into package "check_fixture_dep" installed in the venv, and [dependencies] is empty
    When lvpm build runs on it
    Then it exits 1
    And stdout has the line "UsesDep.vi: <vilib>/Check Fixture/Dep.vi is in check_fixture_dep, which is not in [dependencies]"
    And dist holds no .lvpkg

  Scenario: a member using a dev-dependency refuses the build
    Given member "UsesDep.vi" links into package "check_fixture_dep" installed in the venv, and "check_fixture_dep" only in [dev-dependencies]
    When lvpm build runs on it
    Then it exits 1
    And stdout has the line "UsesDep.vi: <vilib>/Check Fixture/Dep.vi is in check_fixture_dep, which is only in [dev-dependencies]"
    And dist holds no .lvpkg

  Scenario: a declared dependency no member uses is a warning
    Given [dependencies] names "spare" and no member links into it
    When lvpm build runs on it
    Then it exits 0 and dist holds the package
    And stderr has the line "[dependencies] names spare, which no member links to (ignore this if it is loaded by path at run time)"

  Scenario: a declared dependency a member uses gives no unused-dependency warning
    Given member "UsesDep.vi" links into package "check_fixture_dep" installed in the venv, and "check_fixture_dep" in [dependencies]
    When lvpm build runs on it
    Then it exits 0 and dist holds the package
    And stderr is exactly "no lvpm.lock, so the package has no SBOM; run lvpm lock to add one"

  Scenario: links lvpm cannot look for are warned of and no dependency is called unused
    Given the bound installation folder does not exist
    And "UsesNi.vi" is a member of "Lib.lvlib" and the folder "Sub" is empty
    And [dependencies] names "spare"
    When lvpm build runs on it
    Then it exits 0 and dist holds the package
    And stderr has a line starting "warning: links not checked: 1 into vi.lib"
    And stderr has no line containing "no member links to"

  Scenario: a broken link among links not checked still ends stderr on the refusal
    Given the bound installation folder does not exist
    And "UsesNi.vi" is a member of "Lib.lvlib" and the folder "Sub" is empty
    And "Broken.vi" is a member of "Lib.lvlib" and the folder "Sub" is empty
    And [dependencies] names "spare"
    When lvpm build runs on it
    Then it exits 1
    And stderr has a line starting "warning: links not checked: 1 into vi.lib"
    And stderr's last line is "build refused: 1 problem(s) in library members, nothing written; fix them or pass --no-check"

  Scenario Outline: every check rule on a member refuses the build
    Given the member "Good.vi" <is broken>
    When lvpm build runs on it
    Then it exits 1, stdout has a line starting "Good.vi: <message>", and dist holds no .lvpkg

    Examples:
      | is broken                                    | message                                        |
      | is saved in a LabVIEW newer than the minimum | saved in LabVIEW                               |
      | keeps its compiled code                      | compiled code is not separated from the source |
      | has its LIvi section size set past the end   | linker table not readable (LIvi at byte        |
      | contains "not a VI"                          | not a LabVIEW resource file                    |

  Scenario: no venv refuses and says how to go on
    Given the project has no venv
    When lvpm build runs on it
    Then it exits 1, stderr names "lvpm sync" and "--no-check", and dist holds no .lvpkg
    And stderr has a line containing "run lvpm venv create and lvpm sync, or pass --no-check"

  Scenario: --no-check builds without a venv and past a broken member
    Given the project has no venv
    And "Broken.vi" is a member of "Lib.lvlib" and the folder "Sub" is empty
    And [dependencies] names "spare"
    When lvpm build --no-check runs on it
    Then it exits 0 and dist holds the package
    And stderr is exactly the lines "check skipped (--no-check)" and "no lvpm.lock, so the package has no SBOM; run lvpm lock to add one"

  Scenario: --json reports a refusal as check does
    Given "Broken.vi" is a member of "Lib.lvlib" and the folder "Sub" is empty
    When lvpm build runs on it with --json
    Then it exits 1, dist holds no .lvpkg, and the JSON has "ok": false, "command": "build" and one finding for "Broken.vi"

  # The pilot sources are not committed; README.md beside this file says how to
  # check them out.
  @pilot-source
  Scenario: pilot A builds with the check on
    Given pilot A's pinned source checkout with labview ">=2016"
    And the bound installation folder does not exist
    When lvpm build runs on it
    Then it exits 0 and dist holds the package

  @pilot-source
  Scenario: the demo: a broken link in pilot A refuses the build
    Given pilot A's pinned source checkout with labview ">=2016"
    And the bound installation folder does not exist
    And "Source/HSE-Logger/LogRecord/Generate Callchain.vi" is deleted from the project
    And "Generate Callchain.vi" is no longer listed in "Source/HSE-Logger/LogRecord/LogRecord.lvclass"
    When lvpm build runs on it
    Then it exits 1 and dist holds no .lvpkg
    And stdout has the line "Source/HSE-Logger/LogRecord/Init.vi: link not found: Source/HSE-Logger/LogRecord/Generate Callchain.vi"

  Scenario: the docs describe the check in build
    Then docs/usage.md contains "checks the library's members first", "needs a venv", "`VIs checked` counts the library's members", and "--no-check"

  Scenario: the release template builds unchecked and says so
    Then the release template runs "lvpm build --no-check" after printing "::warning::unchecked build"
    And no line of the release template is "lvpm build"
