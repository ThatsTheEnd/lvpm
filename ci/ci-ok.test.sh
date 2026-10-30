#!/usr/bin/env bash
# Self-test for ci-ok.sh: each needs result passes (0) or fails (1).
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
failures=0
expect() { # expect <0|1> <name> <docs-only> <needs JSON>
  bash "$here/ci-ok.sh" "$3" <<<"$4" >/dev/null 2>&1; local got=$?
  if [ "$got" = "$1" ]; then echo "ok   $1 $2"; else echo "FAIL want $1 got $got: $2"; failures=$((failures+1)); fi
}
pass='{"changes":{"result":"success"},"test":{"result":"success"},"build":{"result":"success"}}'
docs='{"changes":{"result":"success"},"test":{"result":"success"},"build":{"result":"skipped"},"e2e":{"result":"skipped"}}'
expect 0 "every job passed" false "$pass"
expect 0 "every job passed on a docs-only PR" true "$pass"
expect 0 "a docs-only PR with skipped jobs" true "$docs"
expect 1 "skipped jobs on a code PR" false "$docs"
expect 1 "a skipped test job on a docs-only PR" true '{"changes":{"result":"success"},"test":{"result":"skipped"},"build":{"result":"skipped"}}'
expect 1 "a failed job on a docs-only PR" true '{"changes":{"result":"success"},"test":{"result":"failure"},"build":{"result":"skipped"}}'
expect 1 "a cancelled job" false '{"changes":{"result":"success"},"test":{"result":"cancelled"}}'
expect 1 "a failed changes job" false '{"changes":{"result":"failure"},"test":{"result":"success"},"build":{"result":"skipped"}}'
expect 1 "no docs-only flag" "" "$pass"
[ $failures = 0 ] && echo "all ci-ok scenarios passed" || { echo "$failures scenario(s) failed"; exit 1; }
