#!/usr/bin/env bash
# Throw away a task's working copy and branch WITHOUT merging.
#   scripts/wt-remove.sh T012
# Use when a task is abandoned. Its commits and any unsaved work are lost.
set -euo pipefail
. "$(dirname "$0")/lib.sh"

ID="${1:-}"; valid_id "$ID"
ROOT="$(main_root)"; DIR="$(wt_dir "$ID")"
[ -d "$DIR" ] || die "no working copy for $ID"
BRANCH="$(wt_branch "$ID")"
git -C "$ROOT" worktree remove --force "$DIR"
git -C "$ROOT" branch -q -D "$BRANCH"
echo "removed .worktrees/$ID and deleted branch $BRANCH (not merged)"
