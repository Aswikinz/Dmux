#!/usr/bin/env bash
# Merge a finished task into main, then remove its working copy and branch.
#   scripts/wt-merge.sh T012
# One merge at a time. If the merge conflicts, it is undone and nothing changes.
set -euo pipefail
. "$(dirname "$0")/lib.sh"

ID="${1:-}"; valid_id "$ID"
ROOT="$(main_root)"; DIR="$(wt_dir "$ID")"
[ -d "$DIR" ] || die "no working copy for $ID"
BRANCH="$(wt_branch "$ID")"

[ "$(git -C "$ROOT" rev-parse --abbrev-ref HEAD)" = main ] || die "the main checkout must be on branch 'main'"
[ -z "$(git -C "$DIR" status --porcelain)" ] || die ".worktrees/$ID has uncommitted work: run scripts/wt-commit.sh $ID \"message\" first"

# The task board and decisions log are edited in the main checkout. Commit them
# first so the merge starts from a clean main.
"$(dirname "$0")/board-commit.sh" "before merging $ID" >/dev/null
[ -z "$(git -C "$ROOT" status --porcelain)" ] || die "the main checkout has uncommitted changes other than the task board:
$(git -C "$ROOT" status --short)"

if [ "$(git -C "$ROOT" rev-list --count "main..$BRANCH")" -eq 0 ]; then
  die "$BRANCH has no commits to merge"
fi

if ! clean_git -C "$ROOT" merge -q --no-ff -m "Merge $BRANCH" "$BRANCH"; then
  clean_git -C "$ROOT" merge --abort || true
  die "merge of $BRANCH conflicts with main. Nothing was changed.
Have the task's owner bring the branch up to date, then try again."
fi

git -C "$ROOT" worktree remove "$DIR"
git -C "$ROOT" branch -q -d "$BRANCH"
echo "merged $BRANCH into main and removed .worktrees/$ID"
git -C "$ROOT" log -1 --format='  %h %s (%an)'
