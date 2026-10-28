Feature: VIPM installs lvpm's builds of both pilots
  Run by hand: both scenarios are tagged @unsafe and the harness never runs
  them, because they install into the user's own LabVIEW through VIPM. The run
  line under each scenario is the record: the operator fills in the date, the
  versions and pass or fail. README.md next to this file says how to run them.

  "Accepted" is the spec's glossary term: `vipm install -y` exits 0 and
  `vipm info <name>` shows the version as its Installed Version.

  Background:
    Given VIPM 26.3.0 logged in and able to install "labview_open_source_lib_data_manipulation" by name
    And `vipm info` shows none of pilot A, pilot B, Data Manipulation or the palette installed
    And VIPM's confirmation for a package that reuses a published name and version is answered when it shows

  @unsafe
  Scenario: pilot A built by lvpm installs in VIPM and mass compiles
    Given pilot A built with lvpm build from its pinned checkout
    When it is installed with `vipm install -y`
    Then the install is accepted
    And VIPM's mass compile of the installed files lists zero broken VIs
    And the installed files are those of "hse_lib_hse_logger-2.2.2.114.vip" minus the 6 files S1 leaves out
    # Run <date>, VIPM 26.3.0, LabVIEW <version>: not run

  @unsafe
  Scenario: pilot B built by lvpm installs in VIPM with its dependencies and mass compiles
    Given pilot B built with lvpm build from its pinned checkout
    When it is installed with `vipm install -y`
    Then the install is accepted
    And `vipm info` shows Data Manipulation and the palette installed
    And VIPM's mass compile of the installed files lists zero broken VIs
    # Run <date>, VIPM 26.3.0, LabVIEW <version>: not run

  # After both: uninstall pilot B, pilot A, the palette and Data Manipulation,
  # and check `vipm info` shows the four not installed. A fail stops PR 2.
