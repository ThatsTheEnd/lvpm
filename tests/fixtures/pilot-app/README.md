# The pilot application

A small LabVIEW application for testing `lvpm build --app`. It is National
Instruments' "Continuous Measurement and Logging" project template, from
LabVIEW 2026, with lvpm's files beside it.

## Where `app/` comes from

`app/` is taken from the examples that come with LabVIEW. It is a copy of the
folder

    <LabVIEW 2026>\ProjectTemplates\Source\Core\Continuous Measurement

of a LabVIEW 2026 installation (NI's template for "Continuous Measurement and
Logging"). It is by National Instruments. lvpm did not write it, except for
the changes listed under "What lvpm changed". `.gitattributes` marks `app/` as
binary so git keeps the bytes as they are.

## License

National Instruments confirmed to the maintainer that the template may be used
in a public repository if a text says it is taken from the examples that come
with LabVIEW. This README is that text.

The copy carries no license file of its own.
`app/documentation/Continuous Measurement and Logging Documentation.html`
says "© 2014-2020 National Instruments. All rights reserved." That notice
stays in place.

## What lvpm changed

In `app/`:

- `lvpm Pilots.vi`, new: logs a message through HSE Logger and serializes
  the settings through OpenSerializer, so the application calls both pilots.
- `Main.vi`: calls `lvpm Pilots.vi` once at start, with the message
  "Application Start", on the error wire into the UI Message Loop.
- `Continuous Measurement.lvproj`: lists `lvpm Pilots.vi`.

Beside `app/`:

- `lvpm.toml`: the project, `[application]` naming the `.lvproj` and its
  build spec, and both pilot libraries in `[dependencies]`.
- `lvpm.lock`: the result of `lvpm lock` over `../pilots`.
- this README.

## Where a build goes

The build spec's destination is
`../builds/NI_AB_PROJECTNAME/Continuous Measurement and Logging Application/Main.exe`,
relative to the `.lvproj`; LabVIEW fills in the project's name. A build
therefore writes below `builds/` in this folder, which `.gitignore` leaves out.
