#!/usr/bin/env bash
# Fails unless every job in the needs JSON on stdin succeeded. With
# docs-only=true a skipped job also passes: pr.yml skips the build, package,
# e2e and lint jobs on a PR that changes only docs (see docs-only.sh).
# Usage: ci-ok.sh <true|false> <<<'<toJSON(needs)>'
set -euo pipefail
case "${1:-}" in true | false) ;; *) echo "::error::ci-ok.sh: docs-only must be true or false, got '${1:-}'"; exit 1 ;; esac
# Root causes (failure, cancelled) first, then jobs skipped because of them.
bad=$(jq -r --argjson docs_only "$1" '
  def ok: .result == "success" or ($docs_only and .result == "skipped");
  to_entries | map(select(.value | ok | not)) | sort_by(.value.result == "skipped") | .[]
  | "::error::\(.key): \(if .value.result == "skipped" then "skipped (downstream)" else .value.result end)"')
[ -z "$bad" ] || { echo "$bad"; exit 1; }
if [ "$1" = true ]; then echo "docs-only PR: every job that ran passed"; else echo "every PR job passed"; fi
