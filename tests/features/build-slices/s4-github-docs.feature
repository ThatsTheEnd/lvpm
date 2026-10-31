Feature: the GitHub release template and its docs

  Scenario: the release template is in place
    Then .github/workflow-templates/lvpm-release.yml runs on pushed tags with "contents: write"
    And it pins LVPM_VERSION to a version matching \d+\.\d+\.\d+
    And it uploads with "gh release upload" and "--clobber"

  Scenario: the docs describe github dependencies
    Then docs/manifest.md shows a dependency line with "github" and "version"
    And docs/manifest.md names "index" and "unknown key" as refusals
    And docs/manifest.md says the version comes from the file name, not the tag
    And docs/usage.md names "GH_TOKEN", "GITHUB_TOKEN", "gh auth token", "rate limit" and "workflow-templates/lvpm-release.yml"

  # manual, AC12, not run by the harness
  # In Zuehlke/ZAlign with lvpm.toml and the copied template, a pushed tag
  # attaches zuehlke_engineering_lib_zalign-<version>.lvpkg to its release;
  # a second project installs it from the github line, and v1.0.0 (.vip only)
  # installs too.
