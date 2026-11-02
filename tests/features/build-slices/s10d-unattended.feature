Feature: s10d: run unattended
  `--yes` (or LVPM_YES) answers every prompt. Of lvpm's commands only
  `venv remove` asks, so it is the one that shows the difference.

  # --yes: `venv create` starts no LabVIEW, so these are not interactive.
  @labview
  Scenario Outline: a yes answers the venv remove prompt
    Given the project has a venv on the detected LabVIEW
    And LVPM_YES is "<env>"
    When "<command>" runs in the project
    Then it did not ask "[y/N]"
    And the venv no longer exists

    Examples:
      | command                 | env |
      | lvpm --yes venv remove  |     |
      | lvpm venv remove -y     |     |
      | lvpm venv remove        | 1   |
      | lvpm --json venv remove | 1   |

  @labview
  Scenario Outline: a falsey LVPM_YES still asks
    Given the project has a venv on the detected LabVIEW
    And LVPM_YES is "<env>"
    When "lvpm venv remove" runs in the project and "n" is typed
    Then it asked "[y/N]"
    And the venv still exists

    Examples:
      | env   |
      | 0     |
      | false |
      | no    |
      | off   |
      |       |

  @labview
  Scenario: typing y without --yes still deletes
    Given the project has a venv on the detected LabVIEW
    When "lvpm venv remove" runs in the project and "y" is typed
    Then the venv no longer exists

  # update: there is none; upgrading is two commands, as in uv.
  Scenario: the README says what to run instead of an update
    Then README.md has a "## Upgrading" section containing "has no `lvpm update` command"
    And README.md has a "## Upgrading" section where "lvpm lock --upgrade" comes before "lvpm sync"
