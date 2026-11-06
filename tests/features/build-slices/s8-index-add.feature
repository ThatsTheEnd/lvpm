Feature: S8: lvpm index add
  A publisher's workflow lists a package's file in the community index with
  lvpm index add. It writes the entry file of the package's name, or adds a
  version to the file its owner already has, and refuses, writing nothing,
  when the name is another repo's, the version is listed, or the url is not
  a file of the owner's releases.

  Background:
    Given pilot B's built .lvpkg as <file>, version 1.2.1.16

  Scenario: a first version creates the entry file
    Given an empty packages folder
    When I run "lvpm index add packages <file> --github someone/Serializer --url https://github.com/someone/Serializer/releases/download/1.2.1/<file>"
    Then the command succeeds printing "added labview_open_source_lib_serializer 1.2.1.16 to packages/labview_open_source_lib_serializer.toml"
    And that file lists github "someone/Serializer" and one version with the file's sha256, requires and LabVIEW minimum

  Scenario: a new version is appended to the owner's file
    Given packages/labview_open_source_lib_serializer.toml owned by someone/Serializer with version 1.2.0.1
    When I run lvpm index add on <file> with --github someone/Serializer and its release url
    Then the file lists both versions, the old one unchanged

  Scenario: the owner's name in another case is the owner
    Given packages/labview_open_source_lib_serializer.toml owned by someone/Serializer with version 1.2.0.1
    When I run lvpm index add on <file> with --github Someone/serializer and its release url
    Then the command succeeds and the file's github stays "someone/Serializer"

  Scenario Outline: add refuses and leaves the folder unchanged
    Given <state>
    When I run lvpm index add on <input>
    Then the command fails with exit code 1 printing "<line>"
    And the packages folder is unchanged

    Examples:
      | state                                         | input                                                                     | line                                                                                                            |
      | the entry file is owned by someone/Serializer | <file> with --github other/Fork                                           | lvpm index: labview_open_source_lib_serializer 1.2.1.16 belongs to someone/Serializer in packages/labview_open_source_lib_serializer.toml, not other/Fork. fix: publish from someone/Serializer, or register another name |
      | the entry file already lists 1.2.1.16         | <file> with --github someone/Serializer                                   | lvpm index: labview_open_source_lib_serializer 1.2.1.16 is already listed in packages/labview_open_source_lib_serializer.toml. fix: listed versions cannot change; publish a new version |
      | an empty packages folder                      | <file> with --github someone/Serializer --url https://example.com/x.lvpkg | lvpm index: labview_open_source_lib_serializer 1.2.1.16: --url must be https://github.com/someone/Serializer/releases/download/<tag>/<file>. fix: pass the release asset's download URL |
      | an empty packages folder                      | notes.txt with --github someone/Serializer                                | lvpm index: notes.txt is not an .lvpkg: <error>. fix: pass the package lvpm build made                           |
      | the entry file is not TOML                    | <file> with --github someone/Serializer                                   | lvpm index: packages/labview_open_source_lib_serializer.toml does not parse: <error>. fix: restore it from the index repo |
