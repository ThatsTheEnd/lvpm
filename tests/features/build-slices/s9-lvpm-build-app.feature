Feature: lvpm build writes an SBOM, and builds applications
  `lvpm build` puts a CycloneDX SBOM of the locked [dependencies] into the
  .lvpkg. A library with no [dependencies] gets an empty SBOM whatever the lock.
  One with [dependencies] and no usable lock (missing, out of date or
  unreadable) is still built, without one.

  `lvpm build --app` builds the application `[application]` names, and
  `--package` the .lvpkg. Every refusal below happens before LabVIEW starts, so
  none of them prints the "building application" line that comes just before it.

  Scenario: the package carries an SBOM of the locked [dependencies]
    Given a library project depending on "alpha" 1.0, which requires "beta" 2.0
    And "gamma" 1.0 only in [dev-dependencies]
    And lvpm.lock is current
    When lvpm build runs
    Then stdout has a line ending "files, with SBOM)"
    And lvpm.lock lists "gamma" in the group "dev"
    And the package's RECORD lists "bom.cdx.json" at the root
    And its package.toml says format = 2
    And its bom.cdx.json has specVersion "1.5" and bomFormat "CycloneDX"
    And its bom.cdx.json has metadata.component "demo_lib" 1.2.3
    And its bom.cdx.json lists exactly alpha 1.0 and beta 2.0, each with the lock's SHA-256
    And alpha's dependencies name "beta@2.0"

  Scenario: two builds over the same lock give the same SBOM
    Given a library project depending on "alpha" 1.0 and lvpm.lock is current
    When lvpm build runs twice, keeping the first package
    Then the bom.cdx.json inside both packages is byte-identical

  Scenario: a library with no dependencies and no lock gets an empty SBOM
    Given a library project with no [dependencies] and no lvpm.lock
    When lvpm build runs
    Then it succeeds
    And its bom.cdx.json has metadata.component type "library" and no components

  Scenario: a library with dependencies and no lock is built without an SBOM
    Given a library project depending on "alpha" 1.0 and no lvpm.lock
    When lvpm build runs
    Then it succeeds
    And the package has no bom.cdx.json
    And its package.toml says format = 1
    And stderr has the line "no lvpm.lock, so the package has no SBOM; run lvpm lock to add one"

  Scenario: a stale lock builds the package without an SBOM
    Given a library project depending on "alpha" 1.0 and a lock written before "beta" was added to [dependencies]
    When lvpm build runs
    Then it succeeds
    And the package has no bom.cdx.json
    And stderr has a line starting "lvpm.lock is out of date (" and ending "so the package has no SBOM; run lvpm lock to add one"

  Scenario: a lock that cannot be read builds the package without an SBOM
    Given a library project depending on "alpha" 1.0 and an lvpm.lock that cannot be read
    When lvpm build runs
    Then it succeeds
    And the package has no bom.cdx.json
    And stderr has a line starting "lvpm.lock cannot be read (" and ending "so the package has no SBOM; run lvpm lock to add one"

  Scenario: --no-check keeps the SBOM
    Given a library project depending on "alpha" 1.0 and lvpm.lock is current
    When lvpm build --no-check runs
    Then the package's RECORD lists "bom.cdx.json" at the root

  Scenario: a package with an SBOM installs, and its SBOM does not
    Given a built "demo_lib-1.2.3.lvpkg" with bom.cdx.json
    When it is installed into a venv
    Then the install succeeds
    And no bom.cdx.json is installed

  Scenario: a package without an SBOM still installs
    Given a built "demo_lib-1.2.3.lvpkg" written without bom.cdx.json
    When it is installed into a venv
    Then the install succeeds

  # `<sandbox parent>` is the sandbox's own folder: the work directory is in it,
  # so a project there exists on every machine and lies outside the project.
  Scenario Outline: a bad [application] table is refused, naming the problem
    Given an application project whose [application] <problem>
    When I run "lvpm build --app"
    Then the exit code is non-zero
    And the error says "<message>"
    And stderr has no line starting "building application"

    Examples:
      | problem                                        | message                                                                          |
      | has no project                                 | [application] needs project                                                      |
      | has no build-spec                              | [application] needs build-spec                                                   |
      | has build-spec = ""                            | [application] needs build-spec                                                   |
      | has project = 5                                | [application] project must be a string                                           |
      | has project = ""                               | [application] needs project                                                      |
      | has build-spec = 5                             | [application] build-spec must be a string                                        |
      | has the key "target" = "RT"                    | [application] has unknown key "target"; expected project or build-spec           |
      | names "app/Missing.lvproj", which is not there | [application] project "app/Missing.lvproj" is not a file                         |
      | names "app/Main.vi"                            | [application] project "app/Main.vi" must end in .lvproj                          |
      | names "../other/X.lvproj"                      | [application] project "../other/X.lvproj" lies outside the folder with lvpm.toml |
      | names "<sandbox parent>/X.lvproj", absolute    | lies outside the folder with lvpm.toml                                           |

  Scenario: an unknown key in [application] is refused by any command reading the manifest
    Given an application project whose [application] has the key "target" = "RT"
    When I run "lvpm lock"
    Then the exit code is non-zero
    And the error says "[application] has unknown key \"target\""

  Scenario: lvpm lock works while the .lvproj is missing
    Given an application project whose [application] names "app/Missing.lvproj", which is not there
    When I run "lvpm lock"
    Then it succeeds

  Scenario: an application without a version is refused
    Given an application project with no version in lvpm.toml
    When I run "lvpm build --app"
    Then the error says "an application build needs project.name and project.version in lvpm.toml, for the SBOM"

  Scenario Outline: a flag for a table the manifest lacks fails, naming it
    Given a project with only <table>
    When I run "lvpm build <flags>"
    Then the exit code is non-zero
    And the error says "<missing> in lvpm.toml"

    Examples:
      | table          | flags           | missing                        |
      | [distribution] | --app           | --app needs [application]      |
      | [application]  | --package       | --package needs [distribution] |
      | [distribution] | --app --package | --app needs [application]      |

  Scenario: a manifest with neither table has nothing to build
    Given a project with no [distribution] and no [application]
    When I run "lvpm build"
    Then the exit code is non-zero
    And the error says "nothing to build: lvpm.toml has no [distribution] or [application]"

  Scenario: --no-check on a run that builds no package is refused
    Given an application project with no [distribution]
    When I run "lvpm build --app --no-check"
    Then the error says "--no-check applies only to the package build"

  Scenario: --package with both tables builds only the package
    Given a project with [distribution] and [application], lvpm.lock current, and no venv
    When I run "lvpm build --package --no-check"
    Then it exits 0 and dist holds the package
    And stderr has no line starting "building application"

  Scenario: --app with both tables builds no package
    Given a project with [distribution] and [application], lvpm.lock current, and no venv
    When I run "lvpm build --app"
    Then the exit code is non-zero
    And the error says "lvpm venv create"
    And dist holds no package

  Scenario Outline: the app build is refused before LabVIEW starts
    Given an application project <state>
    When I run "lvpm build --app"
    Then the exit code is non-zero
    And the error says "<reason>"
    And the error says "<fix>"
    And stderr has no line starting "building application"

    Examples:
      | state                                             | reason                          | fix                                |
      | with no lvpm.lock                                 | no lvpm.lock                    | run lvpm lock                      |
      | whose lock predates a new [dependencies] entry    | lvpm.lock is out of date        | run lvpm lock                      |
      | with a current lock and no venv                   | no venv in                      | run lvpm venv create and lvpm sync |
      | whose venv lacks a locked package                 | the venv differs from lvpm.lock | run lvpm sync                      |
      | whose venv has alpha 0.9 where the lock says 1.0  | the venv differs from lvpm.lock | run lvpm sync                      |
      | whose venv holds a package the lock does not list | the venv differs from lvpm.lock | run lvpm sync                      |

  Scenario: with both tables, a refused app writes no package
    Given a project with [distribution] and [application] and no lvpm.lock
    When I run "lvpm build"
    Then the exit code is non-zero
    And the error says "no lvpm.lock"
    And dist holds no package

  Scenario: with both tables, an app whose LabVIEW is not installed writes no package
    Given a project with [distribution] and [application], lvpm.lock current and a venv bound to a LabVIEW that is not installed here
    When I run "lvpm build"
    Then the exit code is non-zero
    And the error says "not installed here"
    And dist holds no package

  Scenario: --app --package is the same as no flag
    Given a project with [distribution] and [application] and no lvpm.lock
    When I run "lvpm build --app --package"
    Then the exit code is non-zero
    And the error says "no lvpm.lock"
    And dist holds no package

  Scenario: the help names the flags
    When I run "lvpm build --help"
    Then stdout mentions "--app" and "--package"

  Scenario: the pilot application is in the repository
    Given the repository
    Then tests/fixtures/pilot-app/lvpm.toml has [application] build-spec "Continuous Measurement and Logging Application" and both pilots in [dependencies]
    And tests/fixtures/pilot-app/README.md names "Continuous Measurement and Logging", "LabVIEW 2026" and "National Instruments"
    And the README has a section "What lvpm changed"
    And the .lvproj's build spec writes below the fixture's "builds" folder
    And the fixture's lvpm.lock is current
