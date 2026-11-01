Feature: lvpm syncs a package from its GitHub repo

  Scenario: pilot B syncs from its GitHub release
    Given the pilot B project with a current lvpm.lock and an empty cache
    When I run "lvpm sync --prefix dest"
    Then pilot B 1.2.1.16 and its dependencies are installed in dest
    And each install manifest's archive SHA-256 equals the locked one

  Scenario: a release with both files installs the .lvpkg
    Given the repo "o/r" has release "v1.0" with "pkg-1.0.lvpkg" and "pkg-1.0.vip" holding different bytes
    And a project depending on "pkg" with github "o/r" and version "*" and a current lvpm.lock
    When I run "lvpm sync --prefix dest"
    Then "pkg" 1.0 is installed in dest from the .lvpkg's bytes

  Scenario Outline: a re-uploaded file fails sync
    Given the pilot B project with a current lvpm.lock and an empty cache
    And the GitHub fake now serves other bytes for pilot B's .vip <how>
    When I run "<command>"
    Then the command fails naming "labview_open_source_lib_serializer" and both SHA-256 values
    And nothing is installed in dest

    Examples:
      | how                     | command                          |
      | under the same asset id | lvpm sync --prefix dest          |
      | under the same asset id | lvpm sync --locked --prefix dest |
      | under a new asset id    | lvpm sync --prefix dest          |

  Scenario Outline: a deleted release fails sync unless the cache holds the file
    Given the pilot B project with a current lvpm.lock and <cache>
    And the GitHub fake no longer has release "1.2.1"
    When I run "lvpm sync --prefix dest"
    Then <outcome>

    Examples:
      | cache          | outcome                                                                                         |
      | an empty cache | the command fails naming "labview_open_source_lib_serializer" and nothing is installed in dest |
      | a warm cache   | pilot B 1.2.1.16 is installed in dest                                                           |

  Scenario: a private repo syncs and the token is written nowhere
    Given the repo "o/private" is private and has release "v1.0" with "pkg-1.0.lvpkg"
    And GH_TOKEN is "s3cret-token"
    And a project depending on "pkg" with github "o/private" and version "*" and a current lvpm.lock
    And an empty cache
    When I run "lvpm sync --prefix dest"
    Then "pkg" 1.0 is installed in dest
    And the GitHub fake saw no Authorization header on the request to the redirected host "localhost"
    And "s3cret-token" appears in none of lvpm.toml, lvpm.lock, the cache directory, dest, stdout and stderr

  Scenario: a rate limit during sync gives the hint
    Given the pilot B project with a current lvpm.lock and an empty cache
    And the GitHub fake has no requests left for anonymous callers until "2099-01-01T00:00:00Z"
    When I run "lvpm sync --prefix dest"
    Then the command fails naming "labview_open_source_lib_serializer (github.com/LabVIEW-Open-Source/Serializer)", "rate limit", "2099-01-01T00:00:00Z", "gh auth login" and "GH_TOKEN"
