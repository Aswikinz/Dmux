#!/usr/bin/env bash
# Commit everything in a task's working copy onto its task branch.
#   scripts/wt-commit.sh T012 "schema diff folds partitions under their parent"
# The commit carries the repo owner's Git identity. The commit hook strips
# co-author lines and refuses any other author.
set -euo pipefail
. "$(dirname "$0")/lib.sh"

ID="${1:-}"; MSG="${2:-}"
valid_id "$ID"; [ -n "$MSG" ] || die "give a commit message as the second argument"
DIR="$(wt_dir "$ID")"; [ -d "$DIR" ] || die "no working copy for $ID"

clean_git -C "$DIR" add -A
if clean_git -C "$DIR" diff --cached --quiet; then
  echo "nothing to commit in .worktrees/$ID"; exit 0
fi
clean_git -C "$DIR" commit -q -m "$ID: $MSG"
clean_git -C "$DIR" log -1 --format='committed %h on %D%n  author: %an <%ae>%n  %s'
