Feature: lvpm check finds broken links without LabVIEW
  `lvpm check [DIR]` reads every VI under DIR (the project by default) as bytes
  and names each VI and link that breaks a rule. It never starts or looks for
  LabVIEW. The fixture VIs are in tests/fixtures/check, where README.md lists
  each one's links.

  Scenario: a clean project passes
    Given a check project with labview ">=2026" holding "Lib.lvlib", "Good.vi" and "Helper.vi"
    When lvpm check runs
    Then it exits 0
    And stdout ends with "VIs checked: 2, problems: 0, links not checked: 0"

  Scenario: a missing subVI is named with its VI
    Given a check project holding "Broken.vi" and an empty folder "Sub"
    When lvpm check runs
    Then it exits 1
    And stdout has the line "Broken.vi: link not found: Sub/Gone.vi"
    And stdout has the line "next: restore the file, or open the VI in LabVIEW and relink it"

  Scenario: a link through an .llb needs the member
    Given a check project holding "UsesLlb.vi" and "Archive.llb"
    When lvpm check runs
    Then it exits 1
    And stdout has the line "UsesLlb.vi: link not found: Archive.llb/Removed.vi"
    And stdout has no line containing "Archive.llb/InLlb.vi"

  Scenario: a missing .llb fails every link through it
    Given a check project holding "UsesLlb.vi" and no "Archive.llb"
    When lvpm check runs
    Then stdout has the line "UsesLlb.vi: link not found: Archive.llb/InLlb.vi"
    And stdout has the line "UsesLlb.vi: link not found: Archive.llb/Removed.vi"

  Scenario: two VIs with the same missing link give two findings
    Given a check project holding "Broken.vi", and a copy of it named "Broken2.vi"
    When lvpm check runs
    Then stdout has the line "Broken.vi: link not found: Sub/Gone.vi"
    And stdout has the line "Broken2.vi: link not found: Sub/Gone.vi"
    And stdout ends with "VIs checked: 2, problems: 2, links not checked: 0"

  Scenario: an unreadable linker table fails
    Given a check project holding "Good.vi" with its LIvi section size set past the end of the file
    When lvpm check runs
    Then it exits 1
    And stdout has a line starting "Good.vi: linker table not readable (LIvi at byte "

  Scenario: a file named .vi that is not a VI
    Given a check project holding "x.vi" containing "not a VI"
    When lvpm check runs
    Then it exits 1
    And stdout has the line "x.vi: not a LabVIEW resource file"

  Scenario: a vi.lib link is checked in the bound installation, with no LabVIEW detected
    Given a check project holding "UsesNi.vi"
    And the bound installation folder holds the vi.lib file it links to
    When lvpm check runs
    Then it exits 0
    And stdout ends with "VIs checked: 1, problems: 0, links not checked: 0"

  Scenario: a vi.lib file missing from the installation is a broken link
    Given a check project holding "UsesNi.vi"
    And the bound installation folder does not hold the vi.lib file it links to
    When lvpm check runs
    Then it exits 1
    And stdout has a line starting "UsesNi.vi: link not found: <vilib>/"

  Scenario: with the bound installation absent, vi.lib links are not checked
    Given a check project holding "UsesNi.vi"
    And the bound installation folder does not exist
    When lvpm check runs
    Then it exits 0
    And stdout ends with "VIs checked: 1, problems: 0, links not checked: 1"
    And stderr has a line starting "warning: links not checked: 1 into vi.lib (the bound installation "

  Scenario: links not checked do not hide real findings
    Given a check project holding "UsesNi.vi" and "Broken.vi" and an empty folder "Sub"
    And the bound installation folder does not exist
    When lvpm check runs
    Then it exits 1
    And stdout has the line "Broken.vi: link not found: Sub/Gone.vi"
    And stdout ends with "VIs checked: 2, problems: 1, links not checked: 1"

  Scenario: without a venv check says how to make one
    Given a sandbox project with an lvpm.toml and no venv, holding "Broken.vi"
    When lvpm check runs
    Then it exits 1
    And stderr says "`lvpm venv create` makes one"
    And stdout has no line containing "link not found"
    And stdout has no line containing "is not in [dependencies]"

  Scenario: DIR limits the walk
    Given a check project holding "Broken.vi" in folder "Old" and "Good.vi", "Helper.vi", "Lib.lvlib" in folder "New"
    When lvpm check runs on "New"
    Then it exits 0
    And stdout ends with "VIs checked: 2, problems: 0, links not checked: 0"

  Scenario: DIR is the folder with the problem
    Given a check project holding "Broken.vi" in folder "Old" and "Good.vi", "Helper.vi", "Lib.lvlib" in folder "New"
    When lvpm check runs on "Old"
    Then it exits 1
    And stdout has the line "Old/Broken.vi: link not found: Old/Sub/Gone.vi"

  Scenario: DIR that does not exist
    Given a check project holding "Good.vi"
    When lvpm check runs on "Nope"
    Then it exits 1
    And stderr says "folder Nope does not exist"

  Scenario: no VIs under DIR
    Given a check project holding no VIs
    When lvpm check runs
    Then it exits 0
    And stdout ends with "VIs checked: 0, problems: 0, links not checked: 0"
    And stderr has a line starting "warning: no VIs under "

  Scenario: dot-folders, the venv among them, are not walked
    Given a check project holding "Good.vi", "Helper.vi" and "Lib.lvlib"
    And the venv holds an installed package containing "Broken.vi"
    And the project holds "Broken.vi" in folder ".hidden"
    When lvpm check runs
    Then it exits 0
    And stdout ends with "VIs checked: 2, problems: 0, links not checked: 0"

  Scenario: findings in JSON
    Given a check project holding "Broken.vi" and an empty folder "Sub"
    When lvpm check runs with --json
    Then it exits 1
    And the JSON has "ok" false, "vis" 1 and one finding with "vi" "Broken.vi", "kind" "not_found" and "link" "Sub/Gone.vi"

  Scenario: a clean project in JSON
    Given a check project with labview ">=2026" holding "Lib.lvlib", "Good.vi" and "Helper.vi"
    When lvpm check runs with --json
    Then it exits 0
    And the JSON has "ok" true, "vis" 2 and no findings

  Scenario: a VI too new for the project's LabVIEW
    Given a check project with labview ">=2025" holding "Lib.lvlib", "Good.vi" and "Helper.vi"
    When lvpm check runs
    Then it exits 1
    And stdout has the line "Good.vi: saved in LabVIEW 2026, newer than the minimum 2025"
    And stdout has the line "next: save the VI for LabVIEW 2025, or raise labview in lvpm.toml"

  Scenario: a VI saved before the minimum passes
    Given a check project with labview ">=2026" holding the pilot VI saved in LabVIEW 2013
    When lvpm check runs
    Then stdout has no line containing "newer than the minimum"

  Scenario: no labview key means no save-version check
    Given a check project with no labview key holding "Lib.lvlib", "Good.vi" and "Helper.vi"
    When lvpm check runs
    Then it exits 0
    And stderr has the line "warning: lvpm.toml has no labview, so save versions were not checked"

  Scenario: compiled code must be separated
    Given a check project holding "NotSeparated.vi" and "Helper.vi"
    When lvpm check runs
    Then it exits 1
    And stdout has the line "NotSeparated.vi: compiled code is not separated from the source"

  Scenario: one VI breaking several rules gives one finding per rule, in order
    Given a check project with labview ">=2025" holding "NotSeparated.vi", "Broken.vi" and an empty folder "Sub"
    When lvpm check runs
    Then the finding lines are, in order:
      | Broken.vi: saved in LabVIEW 2026, newer than the minimum 2025           |
      | Broken.vi: link not found: Sub/Gone.vi                                  |
      | NotSeparated.vi: saved in LabVIEW 2026, newer than the minimum 2025     |
      | NotSeparated.vi: compiled code is not separated from the source         |
      | NotSeparated.vi: link not found: Helper.vi                              |
      | NotSeparated.vi: link not found: Lib.lvlib                              |

  # A link into a package the venv holds must be to one the manifest declares.
  # Each of these projects has [distribution] top-level-library "Lib.lvlib"
  # unless it says otherwise, and the venv has check_fixture_dep and
  # check_fixture_other installed with install records. Lib.lvlib is always
  # there (the library VIs link to it) and lists only the members held.

  Scenario: a declared dependency passes
    Given a check project with [dependencies] holding "check_fixture_dep", holding "Lib.lvlib" and "UsesDep.vi"
    When lvpm check runs
    Then it exits 0

  Scenario: an undeclared dependency is named with the VI, link and package
    Given a check project with [dependencies] empty, holding "Lib.lvlib" and "UsesDep.vi"
    When lvpm check runs
    Then it exits 1
    And stdout has the line "UsesDep.vi: <vilib>/Check Fixture/Dep.vi is in check_fixture_dep, which is not in [dependencies]"
    And stdout has the line "next: add check_fixture_dep to [dependencies] in lvpm.toml"

  Scenario: a package that is only installed, not declared, is not enough
    Given a check project with [dependencies] holding "check_fixture_other" only, holding "Lib.lvlib" and "UsesDep.vi"
    And the lock lists "check_fixture_dep" as a dependency of "check_fixture_other"
    When lvpm check runs
    Then it exits 1
    And stdout has the line "UsesDep.vi: <vilib>/Check Fixture/Dep.vi is in check_fixture_dep, which is not in [dependencies]"

  Scenario: a library member may not use a dev-dependency
    Given a check project with [dev-dependencies] holding "check_fixture_other", holding "Lib.lvlib" and "UsesDevOnly.vi"
    When lvpm check runs
    Then it exits 1
    And stdout has the line "UsesDevOnly.vi: <vilib>/Check Fixture Dev/DevDep.vi is in check_fixture_other, which is only in [dev-dependencies]"

  Scenario: a library member may not use a dependency group
    Given a check project with [dependency-groups] "ci" holding "check_fixture_other", holding "Lib.lvlib" and "UsesDevOnly.vi"
    When lvpm check runs
    Then it exits 1
    And stdout has the line "UsesDevOnly.vi: <vilib>/Check Fixture Dev/DevDep.vi is in check_fixture_other, which is only in [dependency-groups] ci"

  Scenario: a test VI may use a dev-dependency
    Given a check project with [dev-dependencies] holding "check_fixture_other", holding "Lib.lvlib" and "Tests/TestDevOnly.vi"
    When lvpm check runs
    Then it exits 0

  Scenario: a test VI may use a dependency group
    Given a check project with [dependency-groups] "ci" holding "check_fixture_other", holding "Lib.lvlib" and "Tests/TestDevOnly.vi"
    When lvpm check runs
    Then it exits 0

  Scenario: a test VI may not use a package in no table
    Given a check project with every dependency table empty, holding "Lib.lvlib" and "Tests/TestDevOnly.vi"
    When lvpm check runs
    Then it exits 1
    And stdout has the line "Tests/TestDevOnly.vi: <vilib>/Check Fixture Dev/DevDep.vi is in check_fixture_other, which is not in [dependencies]"

  Scenario: an undeclared finding in JSON
    Given a check project with [dependencies] empty, holding "Lib.lvlib" and "UsesDep.vi"
    When lvpm check runs with --json
    Then the JSON has one finding with "kind" "undeclared", "package" "check_fixture_dep" and "table" null

  Scenario: a vi.lib file no package owns is LabVIEW's own
    Given a check project with [dependencies] empty, holding "UsesNi.vi"
    And the bound installation folder holds the vi.lib file it links to
    When lvpm check runs
    Then it exits 0

  Scenario: a file in the installation wins over a package
    Given a check project with [dependencies] empty, holding "Lib.lvlib" and "UsesDep.vi"
    And the bound installation folder also holds "vi.lib/Check Fixture/Dep.vi"
    When lvpm check runs
    Then it exits 0

  Scenario: package links are checked with the installation absent
    Given a check project with [dependencies] empty, holding "Lib.lvlib" and "UsesDep.vi"
    And the bound installation folder does not exist
    When lvpm check runs
    Then it exits 1
    And stdout has the line "UsesDep.vi: <vilib>/Check Fixture/Dep.vi is in check_fixture_dep, which is not in [dependencies]"

  Scenario: without [distribution], every VI may use any declared table
    Given a check project with no [distribution] table and [dev-dependencies] holding "check_fixture_other", holding "UsesDevOnly.vi"
    When lvpm check runs
    Then it exits 0

  Scenario: the top-level library cannot be read
    Given a check project with [distribution] top-level-library "Nope.lvlib", holding "UsesDep.vi"
    When lvpm check runs
    Then it exits 1
    And stderr has a line containing "Nope.lvlib"

  # The pilot sources are not committed; README.md beside this file says how to
  # check them out.
  @pilot-source
  Scenario: pilot A passes
    Given pilot A's pinned source checkout with labview ">=2016"
    And the bound installation folder does not exist
    When lvpm check runs
    Then it exits 0
    And stdout reports at least 80 VIs checked and 0 problems

  @pilot-source
  Scenario: the demo: a broken link in pilot A is named
    Given pilot A's pinned source checkout with labview ">=2016" and no [distribution] table
    And the bound installation folder does not exist
    And "Source/HSE-Logger/LogRecord/Generate Callchain.vi" is deleted from the project
    When lvpm check runs
    Then it exits 1
    And stdout has the line "Source/HSE-Logger/LogRecord/Init.vi: link not found: Source/HSE-Logger/LogRecord/Generate Callchain.vi"

  @pilot-b-source
  Scenario: pilot B needs its dependency declared
    Given pilot B's pinned source checkout with labview ">=2013" and [dependencies] empty
    And the bound installation folder does not exist
    And "labview_open_source_lib_data_manipulation" installed in the venv from the committed pilot archive
    When lvpm check runs
    Then it exits 1
    And stdout has the line "source/Serializer.INI/INI.Attributes to Variant.vi: <vilib>/LabVIEW Open Source Project/Data Manipulation/KVP Parser.vi is in labview_open_source_lib_data_manipulation, which is not in [dependencies]"

  @pilot-b-source
  Scenario: pilot B passes with its dependency declared
    Given pilot B's pinned source checkout with labview ">=2013" and "labview_open_source_lib_data_manipulation" in [dependencies]
    And the bound installation folder does not exist
    And "labview_open_source_lib_data_manipulation" installed in the venv from the committed pilot archive
    When lvpm check runs
    Then it exits 0
    And stdout reports at least 25 VIs checked and 0 problems

  Scenario: PROVENANCE records the linker format
    Then PROVENANCE.md has a section titled "Linker format" that names LabVIEW 2026 and the fixture set

  Scenario: usage lists check
    Then docs/usage.md has a line starting "lvpm check"
