#!/usr/bin/env bash
# Prints true when every path on stdin (one per line) is under docs/ or is a
# top-level Markdown file, else false. An empty list prints false, so nothing
# is skipped by mistake.
# Usage: git diff --no-renames --name-only <base>...HEAD | docs-only.sh
set -euo pipefail
files=$(cat)
if [ -n "$files" ] && ! grep -qvE '^docs/|^[^/]+\.md$' <<<"$files"; then echo true; else echo false; fi
