Feature: the .lvpkg format (S7a)
  `lvpm build` writes a .lvpkg. The keys ADR 6 removed fail with a message
  that says where each one went.

  Background:
    Given a project "demo_lib" version "1.2.3" with labview ">=2016"
    And [distribution] top-level-library "Lib.lvlib", vendor "Vendor", display-name "Demo Lib"
    And library "Lib.lvlib" holding "A.vi"

  Scenario Outline: a key ADR 6 removed fails, says what to do, and writes nothing
    Given the manifest <change>
    When lvpm build runs
    Then it fails with the "<case>" message, and dist is empty

    Examples:
      | change                                          | case               |
      | sets formats = ["vip"]                          | formats            |
      | sets formats = ["lvpkg"]                        | formats            |
      | has [distribution.vip] vendor = "Vendor"        | [distribution.vip] |
      | has an empty [distribution.vip] table           | [distribution.vip] |
      | has [distribution.vip.dependency-names] a = "b" | [distribution.vip] |

  Scenario: every key in [distribution.vip] is reported at once
    Given the manifest has [distribution.vip] with vendor, display-name, name and icon
    When lvpm build runs
    Then the message's "Found:" lists vendor, display-name, name and icon
