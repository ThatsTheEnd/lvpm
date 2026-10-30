#!/usr/bin/env bash
# Self-test for docs-only.sh: each file list prints true or false.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
failures=0
expect() { # expect <true|false> <name> <file list>
  local got; got=$(printf '%s' "$3" | bash "$here/docs-only.sh" 2>&1)
  if [ "$got" = "$1" ]; then echo "ok   $1 $2"; else echo "FAIL want $1 got '$got': $2"; failures=$((failures+1)); fi
}
expect true  "a docs page" "docs/usage.md"
expect true  "a nested docs file" "docs/adr/0001-x.md"
expect true  "a top-level README" "README.md"
expect true  "docs and top-level Markdown" $'docs/a.md\nREADME.md\nCHANGELOG.md'
expect false "a source file" "crates/lvpm/src/main.rs"
expect false "docs and a source file" $'docs/a.md\ncrates/x.rs'
expect false "a README below the top level" "crates/lvpm/README.md"
expect false "the workflow itself" ".github/workflows/pr.yml"
expect false "a docs-like folder name" "docsx/a.md"
expect false "an empty list" ""
[ $failures = 0 ] && echo "all docs-only scenarios passed" || { echo "$failures scenario(s) failed"; exit 1; }
