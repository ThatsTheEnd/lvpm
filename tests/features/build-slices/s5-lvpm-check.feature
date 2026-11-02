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

  Scenario: the venv's files and dot-folders are not walked
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
