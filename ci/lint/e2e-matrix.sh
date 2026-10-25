#!/usr/bin/env bash
# Checks the e2e matrix and the PR workflows are wired safely: every row's image
# is an NI LabVIEW tag whose -linux/-windows suffix matches the row's os, no
# step condition names an image, the workflows a PR runs only read the repo,
# and no secret but GITHUB_TOKEN is used.
# Usage: e2e-matrix.sh [<workflow dir>]  (default .github/workflows)
set -euo pipefail
dir="${1:-.github/workflows}"
pr="$dir/pr.yml"
[ -f "$pr" ] || { echo "e2e lint: $pr not found" >&2; exit 1; }
errors=0
report_violation() { echo "e2e lint: $1" >&2; errors=$((errors+1)); }

rows=$(tr -d '\r' < "$pr" | grep -E '^ *- \{.*\bimage:' || true)
[ -n "$rows" ] || report_violation "$pr: no e2e matrix rows with an image found."
while IFS= read -r row; do
  [ -n "$row" ] || continue
  os=$(sed -nE 's/.*\bos: *"?([a-z]+)"?.*/\1/p' <<<"$row")
  image=$(sed -nE 's/.*\bimage: *"?([^", }]+)"?.*/\1/p' <<<"$row")
  [[ "$image" =~ ^nationalinstruments/labview:[0-9]{4}q[0-9][a-z0-9]*-(windows|linux)$ ]] \
    || { report_violation "$pr: image '$image' is not a nationalinstruments/labview:<year>q<n>...-<windows|linux> tag."; continue; }
  [ "${BASH_REMATCH[1]}" = "$os" ] || report_violation "$pr: image '$image' does not match its row's os '$os'."
done <<<"$rows"

named=$(tr -d '\r' < "$pr" | grep -nE '^ *if:.*(image|labview:|-linux|-windows)' || true)
[ -z "$named" ] || report_violation "$pr: a condition names an image; branch on the row's os instead: $named"

for f in pr.yml build.yml package.yml; do
  [ -f "$dir/$f" ] || { report_violation "$dir/$f not found."; continue; }
  tr -d '\r' < "$dir/$f" | grep -qE '^permissions:$' && tr -d '\r' < "$dir/$f" | grep -qE '^  contents: read$' \
    || report_violation "$dir/$f must set top-level 'permissions: contents: read'."
  ! tr -d '\r' < "$dir/$f" | grep -qE '\b(contents|packages|actions|id-token|pull-requests|issues|statuses|deployments|checks|security-events|attestations): *write|permissions: *write-all' \
    || report_violation "$dir/$f must not grant a write permission."
  secrets=$(tr -d '\r' < "$dir/$f" | grep -noE 'secrets\.[A-Za-z_]+' | grep -v 'secrets\.GITHUB_TOKEN$' || true)
  [ -z "$secrets" ] || report_violation "$dir/$f uses a secret other than GITHUB_TOKEN: $secrets"
done

[ $errors = 0 ] && echo "e2e lint passed: $dir" || exit 1
