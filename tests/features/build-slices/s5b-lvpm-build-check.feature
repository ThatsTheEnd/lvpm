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

  Scenario: a member with a missing subVI refuses the build
    Given "Broken.vi" is a member of "Lib.lvlib" and the folder "Sub" is empty
    And [dependencies] names "spare"
    When lvpm build runs on it
    Then it exits 1
    And stdout has the line "Broken.vi: link not found: Sub/Gone.vi"
    And stderr has the line "build refused: 1 problem(s) in library members, nothing written; fix them or pass --no-check"
    And stderr has no line containing "no member links to"
    And dist holds no .lvpkg

  Scenario: a refused build leaves the previous package
    Given dist holds a built package for this version
    And "Broken.vi" is a member of "Lib.lvlib" and the folder "Sub" is empty
    When lvpm build runs on it
    Then it exits 1 and that package is byte-identical to before

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

  Scenario: a declared dependency a member uses gives no warning
    Given member "UsesDep.vi" links into package "check_fixture_dep" installed in the venv, and "check_fixture_dep" in [dependencies]
    When lvpm build runs on it
    Then it exits 0 and stderr has no line containing "no member links to"

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

  Scenario: --no-check builds without a venv and past a broken member
    Given the project has no venv
    And "Broken.vi" is a member of "Lib.lvlib" and the folder "Sub" is empty
    And [dependencies] names "spare"
    When lvpm build --no-check runs on it
    Then it exits 0 and dist holds the package
    And stderr is exactly "check skipped (--no-check)"

  Scenario: --json reports a refusal as check does
    Given "Broken.vi" is a member of "Lib.lvlib" and the folder "Sub" is empty
    When lvpm build runs on it with --json
    Then it exits 1, dist holds no .lvpkg, and the JSON has "command": "build" and one finding for "Broken.vi"
