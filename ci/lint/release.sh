#!/usr/bin/env bash
# Checks the release workflow is wired safely: build and publish need guard,
# the release is created as a draft and made public only as the last gh call,
# nothing else tags, no tests run, and the failure report job exists.
# Usage: release.sh [<workflow file>]  (default .github/workflows/release.yml)
set -euo pipefail
file="${1:-.github/workflows/release.yml}"
[ -f "$file" ] || { echo "release lint: $file not found" >&2; exit 1; }
errors=0
bad() { echo "release lint: $file: $1" >&2; errors=$((errors+1)); }

# Prints the value of <key> in job <job> (a two-space-indented key under jobs:).
job_key() { tr -d '\r' < "$file" | awk -v job="$1" -v key="$2" '
  /^jobs:/ {j=1; next}  /^[^ ]/ {j=0}
  j && /^  [A-Za-z0-9_-]+:/ {cur=$1; sub(":$","",cur); next}
  j && cur==job && $0 ~ "^    "key":" {sub("^    "key": *",""); print; exit}'; }

for job in build publish; do
  [[ "$(job_key "$job" needs)" =~ (^|[^a-z-])guard([^a-z-]|$) ]] || bad "job '$job' must need guard."
done
[ "$(job_key release-report if)" = "failure()" ] || bad "job 'release-report' with 'if: failure()' is missing."

gh_lines=$(tr -d '\r' < "$file" | grep -E '^[^#]*\bgh release (create|upload|edit|delete)\b' || true)
first_create=$(grep -m1 'gh release create' <<<"$gh_lines" || true)
[ -n "$first_create" ] || bad "no 'gh release create' found."
[[ "$first_create" == *--draft* ]] || bad "the first 'gh release create' must pass --draft."
last=$(tail -n1 <<<"$gh_lines")
[[ "$last" == *"gh release edit"*"--draft=false"* ]] || bad "the last gh release call must be 'gh release edit ... --draft=false'."

forbidden=$(tr -d '\r' < "$file" | grep -nE '^[^#]*(\bgit tag\b|\bgit push\b.*--tags|\bcargo test\b)' || true)
[ -z "$forbidden" ] || bad "no git tag, git push --tags or cargo test allowed: $forbidden"

[ $errors = 0 ] && echo "release lint passed: $file" || exit 1
