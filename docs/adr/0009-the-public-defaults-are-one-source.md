# 9. The public defaults are one source

Date: 2026-11-11

## Status

Accepted 2026-11-11. Amends [ADR 4](0004-sources-first-wins-and-pinning.md).

## Context

ADR 4 ranks NI's index ahead of JKI's and never merges versions of a name
across sources. NI's index stopped taking some packages that JKI's still
carries: it lists `delacor_lib_dqmh_toolkit` up to 6.0.0.83, JKI's up to
7.1.2.1547. So lvpm offered DQMH 6 while VIPM, which reads the JKI index as
"VIPM Community", offered DQMH 7.

Dependency confusion, the reason for ADR 4, is a public index shadowing a
private name. Both defaults are public and curated by the same ecosystem, so
merging the two of them adds no stranger who could shadow a name.

## Decision

The NI and JKI indexes count as one source in first-source-wins. A name both
list keeps the versions of both; a version both list is kept once, NI's entry
(it carries an MD5). Every other source still ranks on its own, ahead of the
defaults, and never merges with them.

## Consequences

- A name a `[sources]` entry, a `sources.local` folder or the cache lists still
  hides both defaults.
- A lock written before this can now see newer versions from JKI;
  `lvpm lock --upgrade` moves to them, `lvpm sync` does not.
- uv has no such group: its `first-index` treats every index alone. This is a
  narrow `unsafe-best-match`, limited to the two built-in indexes.
