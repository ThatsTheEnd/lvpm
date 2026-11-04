Feature: lvpm build writes an SBOM, and builds applications
  `lvpm build` puts a CycloneDX SBOM of the locked [dependencies] into the
  .lvpkg. A library with no usable lock is still built, without one.

  Scenario: the package carries an SBOM of the locked [dependencies]
    Given a library project depending on "alpha" 1.0, which requires "beta" 2.0
    And "gamma" 1.0 only in [dev-dependencies]
    And lvpm.lock is current
    When lvpm build runs
    Then stdout has a line ending "files, with SBOM)"
    And the package's RECORD lists "bom.cdx.json" at the root
    And its package.toml says format = 2
    And its bom.cdx.json has specVersion "1.5" and bomFormat "CycloneDX"
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
    And the package has no bom.cdx.json and says format = 1
    And stderr has the line "no lvpm.lock, so the package has no SBOM; run lvpm lock to add one"

  Scenario: a stale lock builds the package without an SBOM
    Given a library project depending on "alpha" 1.0 and a lock written before "beta" was added to [dependencies]
    When lvpm build runs
    Then it succeeds
    And the package has no bom.cdx.json
    And stderr has a line starting "lvpm.lock is out of date (" and ending "so the package has no SBOM; run lvpm lock to add one"

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
