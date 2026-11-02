# Architecture map

**[Open the architecture map](map/index.html)**: an interactive map of
lvpm's code. Every part, module and file has a plain-language name and a
one-line description, connected by what uses what. Zoom in from the top-level
parts down to single files.

It has guided flows, played from **Flows**:

- **Getting started**: `lvpm init`, then adding DQMH. Shows what happens from
  an empty folder until the project's `.lv-venv` holds DQMH and its
  dependencies.
- **Per-project packages (.lv-venv)**: how a project's packages live in the
  project rather than in LabVIEW, unlike a VIPM install.
- **Installing a package globally**: the install into LabVIEW itself.
- **Pinning a project's versions**: what `lvpm lock` does.

**Start here** in the map's header runs a one-minute tour.

## Keeping it current

The map is a snapshot made with the Floorplan plugin for Claude Code, which
works in the git-ignored `floorplan/` folder. To update it, ask Claude
Code to "update the floorplan". Then use the map's **Export** button, or run
`node floorplan/serve.mjs --export docs/map/index.html`, and commit the
result.
