# JSON output and exit codes

{{#include ../../README.md:json-output}}

## The `held_back` event

When `[sources] min-age-days` kept out a version newer than the one a
resolving command chose (`lvpm lock`, `lvpm lock --upgrade`, `lvpm add`,
`lvpm sync`), it writes one `held_back` event for each package, naming the
newest such version. `released` and `eligible` are RFC 3339 UTC instants;
`eligible` is `released` plus `min_age_days` days, the first moment the
version may be chosen.

```json
{"event": "held_back", "data": {"name": "alpha", "version": "1.1", "chosen": "1.0", "released": "2026-10-28T11:46:49Z", "eligible": "2026-11-11T11:46:49Z", "min_age_days": 14}}
```
