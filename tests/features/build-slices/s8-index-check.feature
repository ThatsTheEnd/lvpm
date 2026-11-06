Feature: S8: lvpm index check
  The community index's bot runs lvpm index check on each PR. It accepts a
  version only when the version is new, its url is a release file of the
  name's owner, the file is the one the entry describes, and the owner's own
  workflow attested it. Every other change is refused with the reason and a
  fix. Where a line below shows <file-sha256> or <entry-sha256>, it stands for
  the SHA-256 of the file the release serves and the one the head's entry
  says; <old> and <new> stand for the two SHA-256 values of the step that
  changes one.

  Background:
    Given a base index where packages/foo.toml is owned by "a/x" with version 1.0.0
    And a fake GitHub where a/x's releases hold foo 1.0.0 and foo 1.1.0, attested by a/x

  Scenario: a new version from the owner is accepted
    Given the head adds foo 1.1.0 with the release's url and sha256
    When I run "lvpm index check --base base --head head"
    Then the command succeeds printing "accepted foo 1.1.0"
    And the release file of foo 1.1.0 was downloaded without a token

  Scenario: a new name is accepted
    Given the head adds packages/bar.toml owned by "b/y", whose release is attested by b/y
    When I run "lvpm index check --base base --head head"
    Then the command succeeds printing "accepted bar 1.0.0"

  Scenario: a changed hash for an existing version is rejected
    Given the head changes foo 1.0.0's sha256 from <old> to <new>
    When I run "lvpm index check --base base --head head"
    Then the command fails with exit code 1 printing "refused foo 1.0.0: already listed with sha256 <old>, now <new>. fix: listed versions cannot change; publish a new version"

  Scenario Outline: a network rule refuses with a fix
    Given <fixture>
    And the head adds foo 1.1.0 <entry>
    When I run "lvpm index check --base base --head head"
    Then the command fails with exit code 1 printing "<line>"

    Examples:
      | fixture                                       | entry                                  | line                                                                                                                                                                             |
      | the Background                                | with a url on example.com              | refused foo 1.1.0: https://example.com/a/x/releases/download/1.1.0/foo-1.1.0.vip is not a release file of a/x. fix: upload the package to a release of a/x                        |
      | the Background                                | with another sha256                    | refused foo 1.1.0: the file has sha256 <file-sha256>, the entry says <entry-sha256>. fix: don't replace release files after tagging; push a new tag                              |
      | the release file of foo 1.1.0 answers 404     | with the release's url and sha256      | refused foo 1.1.0: the file could not be downloaded: HTTP 404. fix: check the release is published and public, then push a new tag                                                |
      | the Background                                | whose requires differ from the package | refused foo 1.1.0: the package says requires bar >=2.0, the entry says bar >=3.0. fix: don't replace release files after tagging; push a new tag                                 |
      | the Background                                | whose labview differs from the package | refused foo 1.1.0: the package says labview >=20.0, the entry says >=21.0. fix: don't replace release files after tagging; push a new tag                                         |
      | the Background                                | named foo but the package says bar     | refused foo 1.1.0: the package says name bar, the entry says foo. fix: don't replace release files after tagging; push a new tag                                                  |
      | the Background                                | whose version differs from the package | refused foo 1.1.0: the package says version 1.1.1, the entry says 1.1.0. fix: don't replace release files after tagging; push a new tag                                           |
      | foo 1.1.0's release has no attestation        | with the release's url and sha256      | refused foo 1.1.0: not attested (no attestation). fix: build with the lvpm release template, which attests the package                                                            |
      | gh cannot run                                 | with the release's url and sha256      | refused foo 1.1.0: not attested (gh is not installed). fix: build with the lvpm release template, which attests the package                                                      |
      | foo 1.1.0's attestation does not verify       | with the release's url and sha256      | refused foo 1.1.0: not attested (does not verify). fix: build with the lvpm release template, which attests the package                                                          |
      | foo 1.1.0 is attested by b/x                  | with the release's url and sha256      | refused foo 1.1.0: attested by https://github.com/b/x, not https://github.com/a/x. fix: publish from a/x                                                                          |

  Scenario: an attestation by the owner spelled in another case is accepted
    Given foo 1.1.0 is attested by A/X
    And the head adds foo 1.1.0 with the release's url and sha256
    When I run "lvpm index check --base base --head head"
    Then the command succeeds printing "accepted foo 1.1.0"

  Scenario: a file that does not parse is refused
    Given the head adds packages/bar.toml that is not TOML
    When I run "lvpm index check --base base --head head"
    Then the command fails with exit code 1 printing "refused packages/bar.toml: does not parse: <error>. fix: register with the lvpm release template instead of editing by hand"

  Scenario: an entry file over 1 MiB is refused as one that does not parse
    Given the head adds packages/big.toml of 2 MiB
    When I run "lvpm index check --base base --head head"
    Then the command fails with exit code 1 printing "refused packages/big.toml: does not parse: the file is larger than 1048576 bytes. fix: register with the lvpm release template instead of editing by hand"

  Scenario: several findings print sorted by path, then version
    Given the head changes foo 1.0.0's sha256 and adds notes.md
    When I run "lvpm index check --base base --head head"
    Then the output is the notes.md line, then the foo line, and the exit code is 1

  Scenario: no change is nothing to check
    Given the head equals the base
    When I run "lvpm index check --base base --head head"
    Then the command succeeds printing "nothing to check"
