#!/usr/bin/env bash
# CI check: every commit in the range is free of co-author lines.
#   scripts/check-commits.sh origin/main..HEAD
set -euo pipefail
RANGE="${1:?usage: check-commits.sh <base>..<head>}"
BAD=0
for sha in $(git rev-list "$RANGE"); do
  if git log -1 --format=%B "$sha" | grep -qiE '^[[:space:]]*co-authored-by[[:space:]]*:'; then
    echo "commit $(git log -1 --format='%h %s' "$sha") has a co-author line"; BAD=1
  fi
done
[ "$BAD" -eq 0 ] && echo "commit rules: ok ($(git rev-list --count "$RANGE") commit(s) checked)"
exit "$BAD"
