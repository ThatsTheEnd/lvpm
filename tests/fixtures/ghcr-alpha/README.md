# ghcr-alpha

The package behind the `@ghcr` scenarios (`tests/features/build-slices/s7b-ghcr.feature`):
`alpha` 1.0.0 with one text file for a VI, so `lvpm build --no-check` packages
it with no LabVIEW. The workflow in `docs/oci-test-repo-workflow.yml` publishes
it once to `ghcr.io/thatstheend/lvpm-oci-test`; the republish scenario relies
on that tag existing and never being deleted. Do not change `name` or `version`.
