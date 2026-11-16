#!/usr/bin/env bash
# CI check: the answer key and product code never change in the same pull
# request. This stops a failing test being "fixed" by editing the expected result.
#   scripts/check-protected.sh origin/main
set -euo pipefail
BASE="${1:?usage: check-protected.sh <base ref>}"
CHANGED="$(git diff --name-only "$BASE"...HEAD)"

KEY=0; CODE=0
while IFS= read -r f; do
  [ -n "$f" ] || continue
  case "$f" in
    testdata/expected_diffs.json|testdata/*/expected_diffs.json) KEY=1 ;;
    crates/*|src-tauri/*|src/*) CODE=1 ;;
  esac
done <<< "$CHANGED"

if [ "$KEY" -eq 1 ] && [ "$CODE" -eq 1 ]; then
  echo "protected files: this change edits both the answer key and product code."
  echo "Split it: the answer key changes alone, in its own pull request."
  exit 1
fi
echo "protected files: ok"
