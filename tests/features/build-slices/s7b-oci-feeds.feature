Feature: lvpm publish to an OCI registry
  `lvpm publish --to oci://host/prefix` pushes the package lvpm build wrote as
  one tagged OCI manifest, `<prefix>/<name>:<version>`, and never replaces a
  tag. The registry is a loopback fake: `<registry>` stands for its host:port.

  Background:
    Given a loopback OCI registry
    And a project "alpha" 1.0.0 that lvpm build has packaged, requiring "beta >=1.0" and LabVIEW 21.0

  Scenario: A package is published as one tagged OCI manifest
    When I run "lvpm publish --to oci://<registry>/team"
    Then it succeeds
    And the output says "published alpha 1.0.0 to oci://<registry>/team/alpha:1.0.0"
    And the output says "uploading team/alpha:1.0.0 to <registry>"
    And the output gives the OCI manifest digest of "team/alpha:1.0.0"
    And the registry holds tag "1.0.0" in repository "team/alpha"
    And that OCI manifest has artifactType "application/vnd.lvpm.package.v1"
    And its config has media type "application/vnd.oci.empty.v1+json"
    And its one layer has media type "application/vnd.lvpm.lvpkg.v1+zip" and the package's bytes
    And its annotations are:
      | key                              | value      |
      | org.opencontainers.image.title   | alpha      |
      | org.opencontainers.image.version | 1.0.0      |
      | dev.lvpm.requires                | beta >=1.0 |
      | dev.lvpm.labview                 | 21.0       |
    And annotation "org.opencontainers.image.created" is an RFC 3339 time

  Scenario: An existing tag is refused and nothing is written
    Given the registry already holds tag "1.0.0" in repository "team/alpha"
    When I run "lvpm publish --to oci://<registry>/team"
    Then it fails saying "team/alpha:1.0.0 is already published at oci://<registry>/team; a published version is never replaced; bump the version to publish again"
    And the registry received no blob or manifest write

  Scenario: A tag that appears during the upload is refused before the OCI manifest is written
    Given the registry will create tag "1.0.0" in "team/alpha" once the first blob is uploaded
    When I run "lvpm publish --to oci://<registry>/team"
    Then it fails saying "team/alpha:1.0.0 is already published"
    And the registry received no manifest write

  Scenario: A registry that cannot be reached fails before writing
    Given the registry is down
    When I run "lvpm publish --to oci://<registry>/team"
    Then it fails naming "<registry>"
    And the output says "nothing was published, it is safe to rerun"

  Scenario: A rerun after an aborted publish succeeds
    Given the registry went down after the blobs of "team/alpha:1.0.0" were uploaded
    And I ran "lvpm publish --to oci://<registry>/team" and it failed
    When the registry is back and I run "lvpm publish --to oci://<registry>/team"
    Then it succeeds
    And the registry received no second upload of a blob it already held

  Scenario: A tag check that fails with an error other than 404 stops the publish
    Given the registry answers 500 to a HEAD of "team/alpha:1.0.0"
    When I run "lvpm publish --to oci://<registry>/team"
    Then it fails naming "<registry>"
    And the registry received no blob or manifest write

  Scenario: A version that is not a valid OCI tag is refused before any request
    Given a package "alpha" whose metadata gives version 1.0.0+build.7
    When I run "lvpm publish --to oci://<registry>/team"
    Then it fails saying "1.0.0+build.7" is not a valid OCI tag
    And the registry received no request

  Scenario: Credentials in the publish target are refused
    When I run "lvpm publish --to oci://u-4c1d2e:pw-7f3a9c-secret@<registry>/team"
    Then it fails saying "--to: credentials do not belong in the target; remove the user:password@ part and run docker login <registry>"
    And the output does not contain "pw-7f3a9c-secret"
    And the output does not contain "u-4c1d2e"
    And the registry received no request

  Scenario: Folder publish is unchanged
    When I run "lvpm publish --to <folder>"
    Then it succeeds
    And the package file is in the folder
    And the registry received no request
