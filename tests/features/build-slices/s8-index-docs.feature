Feature: S8: publishing to the community index
  The release template registers a package in the community index only when
  the repository has the secret LVPM_INDEX_TOKEN, and says in the job summary
  what happened. docs/usage.md tells a publisher the four steps and a consumer
  the two lines.

  Scenario: the template registers only with a token
    Then the release template has a job "register" that needs "publish"
    And "register" sets LVPM_INDEX_TOKEN from the secret, and every step but the last two has "if: env.LVPM_INDEX_TOKEN != ''"
    And the step writing "register failed:" has "if: failure() && env.LVPM_INDEX_TOKEN != ''"
    And the last step of "register" has "if: env.LVPM_INDEX_TOKEN == ''" and writes "not registered in the community index: LVPM_INDEX_TOKEN is not set" to GITHUB_STEP_SUMMARY
    And "register" runs "lvpm index add" and "gh pr create" against LVPM_INDEX_REPO, defaulting to labview-package-manager/package-index
    And "register" writes to GITHUB_STEP_SUMMARY the pinned success line and each pinned "register failed:" line
    And the jobs "publish" and "publish-oci" do not mention LVPM_INDEX_TOKEN
    And the template's header comment documents "register"

  Scenario: usage.md tells a publisher and a consumer what to do
    Then docs/usage.md has a section "Publish to the community index" with four numbered steps naming lvpm.toml, lvpm-release.yml, LVPM_INDEX_TOKEN and a tag
    And the section names the token type "classic" with "public_repo", and says who closes a refused PR
    And it shows community = "github:labview-package-manager/package-index" and says source order matters
    And the command table lists "lvpm index add" and "lvpm index check"
    And the section "GitHub dependencies" links to "Publish to the community index"
