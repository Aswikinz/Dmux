#!/usr/bin/env bash
# CI check: every commit in the range is free of co-author lines, and every
# non-merge commit carries the owner's author email.
#   scripts/check-commits.sh origin/main..HEAD
#
# The identity check runs only when OWNER_EMAIL is set (the workflow exports it).
# GitHub-authored PR merge commits are skipped with --no-merges: their author is
# "GitHub <noreply@github.com>", not the owner, and they are expected.
set -euo pipefail
RANGE="${1:?usage: check-commits.sh <base>..<head>}"
OWNER_EMAIL="${OWNER_EMAIL:-}"
BAD=0

for sha in $(git rev-list "$RANGE"); do
  if git log -1 --format=%B "$sha" | grep -qiE '^[[:space:]]*co-authored-by[[:space:]]*:'; then
    echo "commit $(git log -1 --format='%h %s' "$sha") has a co-author line"; BAD=1
  fi
done

# Every non-merge commit must carry the owner's author email. Merge commits are
# skipped: GitHub's own PR merges are authored by "GitHub", not the owner.
if [ -n "$OWNER_EMAIL" ]; then
  while IFS= read -r sha; do
    [ -n "$sha" ] || continue
    email="$(git log -1 --format=%ae "$sha")"
    if [ "$email" != "$OWNER_EMAIL" ]; then
      echo "commit $(git log -1 --format='%h %s' "$sha") has author email '$email', not the owner's '$OWNER_EMAIL'"; BAD=1
    fi
  done < <(git rev-list --no-merges "$RANGE")
fi

[ "$BAD" -eq 0 ] && echo "commit rules: ok ($(git rev-list --count "$RANGE") commit(s) checked)"
exit "$BAD"
