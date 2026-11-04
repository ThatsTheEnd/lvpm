# The pilot application

A small LabVIEW application for testing `lvpm build --app`. It is National
Instruments' "Continuous Measurement and Logging" project template, from
LabVIEW 2026, with lvpm's files beside it.

## Where `app/` comes from

`app/` is a byte-for-byte copy of the folder

    <LabVIEW 2026>\ProjectTemplates\Source\Core\Continuous Measurement

of a LabVIEW 2026 installation (NI's template for "Continuous Measurement and
Logging"). It is by National Instruments. lvpm did not write it.
`.gitattributes` marks `app/` as binary so git keeps the bytes as they are.

## License

The copy carries no license or notice file of its own. What was found:

- The template folder holds none.
- `app/documentation/Continuous Measurement and Logging Documentation.html`
  says "© 2014-2020 National Instruments. All rights reserved." It says NI
  software is covered by NI's end-user license agreements and points to
  `<National Instruments>\Shared\MDF\Legal\license` and
  `<National Instruments>\_Legal Information.txt`. Neither exists on the
  machine this copy was made on.
- The `.lvproj` build spec has a copyright field of "Copyright © 2012 ", with
  no owner.

No license that allows redistribution was found, so none is claimed here. The
template ships with LabVIEW to be used as the start of a project; whether it may
also be committed to a public repository is not settled by anything in this
folder. Check it against the LabVIEW license before publishing the repository.

## What lvpm changed

Nothing in `app/`. lvpm added:

- `lvpm.toml`: the project, `[application]` naming the `.lvproj` and its
  build spec, and both pilot libraries in `[dependencies]`.
- `lvpm.lock`: the result of `lvpm lock` over `../pilots`.
- this README.

A later slice adds a call to both pilots from the application.

## Where a build goes

The build spec's destination is
`../builds/NI_AB_PROJECTNAME/Continuous Measurement and Logging Application/Main.exe`,
relative to the `.lvproj`; LabVIEW fills in the project's name. A build
therefore writes below `builds/` in this folder, which `.gitignore` leaves out.
