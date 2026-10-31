Feature: lvpm locks a package from its GitHub repo

  Scenario: pilot B locks from its GitHub release
    Given the pilot B project
    When I run "lvpm lock"
    Then the command succeeds
    And lvpm.lock takes "labview_open_source_lib_serializer" at version "1.2.1.16" from "https://github.com/LabVIEW-Open-Source/Serializer" with the .vip's SHA-256
    And lvpm.lock takes "labview_open_source_lib_data_manipulation" from the [sources] feed
    And lvpm.lock has github "LabVIEW-Open-Source/Serializer" for "labview_open_source_lib_serializer" in [inputs.github]
    And stderr names "listing releases of github.com/LabVIEW-Open-Source/Serializer"

  Scenario: pilot B is locked at the file's version, not the tag
    Given the pilot B project
    When I run "lvpm lock"
    Then lvpm.lock's version for "labview_open_source_lib_serializer" is exactly "1.2.1.16", not "1.2.1"

  Scenario: a second lock downloads nothing
    Given the pilot B project with a current lvpm.lock
    And the GitHub fake's request log is cleared
    When I change labview to "2025"
    And I run "lvpm lock"
    Then the command succeeds
    And the GitHub fake received no asset request

  Scenario: a re-lock keeps the locked version when a newer release appears
    Given the pilot B project with a current lvpm.lock
    And the repo "LabVIEW-Open-Source/Serializer" has release "1.3.0" with "labview_open_source_lib_serializer-1.3.0.1.vip"
    And the GitHub fake's request log is cleared
    When I change labview to "2025"
    And I run "lvpm lock --refresh"
    Then the command succeeds
    And the GitHub fake received a listing request
    And lvpm.lock takes "labview_open_source_lib_serializer" at version "1.2.1.16"

  Scenario: lvpm lock --upgrade-package moves to the newest release and keeps the github line
    Given the pilot B project with a current lvpm.lock
    And the repo "LabVIEW-Open-Source/Serializer" has release "1.3.0" with "labview_open_source_lib_serializer-1.3.0.1.vip"
    When I run "lvpm lock --upgrade-package labview_open_source_lib_serializer"
    Then lvpm.lock takes "labview_open_source_lib_serializer" at version "1.3.0.1"

  Scenario: a constraint above every file fails naming the newest
    Given the pilot B project, asking for version ">=1.2.2"
    When I run "lvpm lock"
    Then the command fails naming "labview_open_source_lib_serializer (github.com/LabVIEW-Open-Source/Serializer)" and "the newest is 1.2.1.16"
    And no lvpm.lock is written

  Scenario: a repo with no fitting version does not fall back to [sources]
    Given the pilot B project, asking for version ">=10"
    And the [sources] feed also lists "labview_open_source_lib_serializer" 10.0
    When I run "lvpm lock"
    Then the command fails naming "labview_open_source_lib_serializer (github.com/LabVIEW-Open-Source/Serializer)" and "no release has a version >=10"
    And no lvpm.lock is written

  Scenario: a higher version on a [sources] feed is not used
    Given the pilot B project
    And the [sources] feed also lists "labview_open_source_lib_serializer" 9.0
    When I run "lvpm lock"
    Then lvpm.lock takes "labview_open_source_lib_serializer" at version "1.2.1.16" from "https://github.com/LabVIEW-Open-Source/Serializer"

  Scenario: a release whose file is not named for the package fails naming the pattern
    Given the repo "o/r" has release "v1.0" with "Pkg_Release.lvpkg"
    And a project depending on "pkg" with github "o/r" and version "*"
    When I run "lvpm lock"
    Then the command fails naming "pkg (github.com/o/r)" and "pkg-<version>.lvpkg"

  Scenario: a pre-release is never picked
    Given the repo "o/r" has release "v1.0" with "pkg-1.0.lvpkg"
    And the repo "o/r" has release "v2.0-rc" with "pkg-2.0.lvpkg" as a pre-release
    And a project depending on "pkg" with github "o/r" and version "*"
    When I run "lvpm lock"
    Then lvpm.lock takes "pkg" at version "1.0"

  Scenario: a locked release that no longer runs on the project's LabVIEW gives way to one that does
    Given the repo "o/r" has release "v1.0" with "pkg-1.0.lvpkg"
    And the repo "o/r" has release "v2.0" with "pkg-2.0.lvpkg" that needs LabVIEW 2024
    And a project depending on "pkg" with github "o/r" and version "*"
    When I run "lvpm lock"
    Then lvpm.lock takes "pkg" at version "2.0"
    When I change labview to "2023"
    And I run "lvpm lock"
    Then the command succeeds
    And lvpm.lock takes "pkg" at version "1.0"

  Scenario: a release with both files installs the .lvpkg
    Given the repo "o/r" has release "v1.0" with "pkg-1.0.lvpkg" and "pkg-1.0.vip" holding different bytes
    And a project depending on "pkg" with github "o/r" and version "*"
    When I run "lvpm lock"
    Then lvpm.lock has the SHA-256 of "pkg-1.0.lvpkg"

  Scenario Outline: a file whose contents do not match its name fails
    Given the repo "o/r" has release "v1.0" with "pkg-1.0.lvpkg" that holds <inside>
    And a project depending on "pkg" with github "o/r" and version "*"
    When I run "lvpm lock"
    Then the command fails naming "pkg-1.0.lvpkg" and <said>
    And no lvpm.lock is written

    Examples:
      | inside      | said        |
      | "other" 1.0 | "other 1.0" |
      | "pkg" 1.1   | "pkg 1.1"   |

  Scenario: a private repo locks with GH_TOKEN, and the redirect drops it
    Given the repo "o/private" is private and has release "v1.0" with "pkg-1.0.lvpkg"
    And a project depending on "pkg" with github "o/private" and version "*"
    And GH_TOKEN is "s3cret-token"
    When I run "lvpm lock"
    Then the command succeeds
    And the GitHub fake saw the token on the listing and the asset request
    And it saw no Authorization header on the request to the redirected host "localhost"
    And "s3cret-token" appears in none of lvpm.toml, lvpm.lock, the cache directory, stdout and stderr

  Scenario: a private repo without a token fails with a hint
    Given the repo "o/private" is private
    And a project depending on "pkg" with github "o/private" and version "*"
    When I run "lvpm lock"
    Then the command fails naming "pkg (github.com/o/private)", "not found" and "gh auth login"
    And no lvpm.lock is written

  Scenario: a rejected token fails without printing it
    Given the repo "o/private" is private
    And a project depending on "pkg" with github "o/private" and version "*"
    And GH_TOKEN is "wrong-token"
    When I run "lvpm lock"
    Then the command fails naming "pkg (github.com/o/private)" and "rejected the token from GH_TOKEN"
    And "wrong-token" appears in neither stdout nor stderr
    And no lvpm.lock is written

  Scenario: an unreachable API fails naming the package and the repo
    Given the pilot B project with GITHUB_API_URL pointing at a closed port
    When I run "lvpm lock"
    Then the command fails naming "labview_open_source_lib_serializer (github.com/LabVIEW-Open-Source/Serializer)" and "cannot read the releases"
    And no lvpm.lock is written

  Scenario: an exhausted rate limit fails at once with a hint
    Given the pilot B project
    And the GitHub fake has no requests left for anonymous callers until "2099-01-01T00:00:00Z"
    When I run "lvpm lock"
    Then the command fails naming "labview_open_source_lib_serializer (github.com/LabVIEW-Open-Source/Serializer)", "rate limit", "2099-01-01T00:00:00Z", "gh auth login" and "GH_TOKEN"
    And the GitHub fake received one request
    And no lvpm.lock is written

  Scenario: re-locking after a re-upload fails with an empty cache
    Given the pilot B project with a current lvpm.lock and an empty cache
    And the GitHub fake now serves other bytes for pilot B's .vip
    When I change labview to "2025"
    And I run "lvpm lock"
    Then the command fails naming "labview_open_source_lib_serializer" and both SHA-256 values

  Scenario: a cached listing whose asset was uploaded again under a new id is read again
    Given the pilot B project with a current lvpm.lock
    And the cache no longer holds pilot B's archive
    And the GitHub fake now serves pilot B's .vip again under a new asset id
    When I change labview to "2025"
    And I run "lvpm lock"
    Then the command succeeds
    And stderr names "listing releases of github.com/LabVIEW-Open-Source/Serializer (cached)"
    And lvpm.lock takes "labview_open_source_lib_serializer" at version "1.2.1.16" from "https://github.com/LabVIEW-Open-Source/Serializer" with the .vip's SHA-256

  Scenario Outline: a change to github in the manifest makes the lock stale
    Given the pilot B project with a current lvpm.lock
    When I <change>
    And I run "lvpm lock --check"
    Then the exit code is non-zero
    And stderr is one line naming sources changed

    Examples:
      | change                                                     |
      | change pilot B's github to "someone/Serializer"            |
      | replace pilot B's line with the short form ">=1.2"         |
      | add github "o/r" to the "labview_open_source_lib_data_manipulation" line |

  Scenario: lvpm add keeps the github line
    Given the pilot B project with a current lvpm.lock
    When I run "lvpm add labview_open_source_lib_serializer@>=1.2.1 --prefix dest"
    Then the command succeeds
    And lvpm.toml's pilot B line has github "LabVIEW-Open-Source/Serializer" and version ">=1.2.1"

  Scenario: a cached listing without the release a new constraint asks for is read again
    Given the pilot B project with a current lvpm.lock
    And the repo "LabVIEW-Open-Source/Serializer" has release "1.3.0" with "labview_open_source_lib_serializer-1.3.0.1.vip"
    When I run "lvpm add labview_open_source_lib_serializer@>=1.3.0 --prefix dest"
    Then lvpm.lock takes "labview_open_source_lib_serializer" at version "1.3.0.1"

  Scenario: lvpm remove drops the github line and its lock input
    Given the pilot B project with a current lvpm.lock
    When I run "lvpm remove labview_open_source_lib_serializer --prefix dest"
    Then the command succeeds
    And lvpm.toml has no pilot B line
    And lvpm.lock has no [inputs.github] table

  Scenario: a lock without GitHub dependencies is unchanged
    Given a project whose lvpm.toml depends on "pkg" ">=1.2" with a feed that lists pkg 1.2
    When I run "lvpm lock"
    Then lvpm.lock equals tests/golden/s4-no-github.lock, captured from main before step 3.2
