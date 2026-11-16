#!/usr/bin/env bash
# Create a separate working copy for one task.
#   scripts/wt-new.sh T012 schema-diff
# Makes branch task/T012-schema-diff from main, checked out in .worktrees/T012/
set -euo pipefail
. "$(dirname "$0")/lib.sh"

ID="${1:-}"; SLUG="${2:-}"
valid_id "$ID"
[[ "$SLUG" =~ ^[a-z0-9][a-z0-9-]{1,40}$ ]] || die "second argument is a short name like 'schema-diff'"

ROOT="$(main_root)"; DIR="$(wt_dir "$ID")"; BRANCH="task/$ID-$SLUG"
MAX="${DMUX_MAX_WORKTREES:-3}"

[ ! -e "$DIR" ] || die "$ID already has a working copy at .worktrees/$ID"
git -C "$ROOT" show-ref --verify --quiet refs/heads/main || die "no 'main' branch yet: make the first commit on main"
mkdir -p "$ROOT/.worktrees"
OPEN=$(find "$ROOT/.worktrees" -mindepth 1 -maxdepth 1 -type d | wc -l)
[ "$OPEN" -lt "$MAX" ] || die "$OPEN working copies are already open (limit $MAX). Merge or remove one first: scripts/wt-list.sh"

git -C "$ROOT" worktree add -q -b "$BRANCH" "$DIR" main
# AGENTS.md is local-only (never in Git); copy it in so agents in this
# working copy still load the rules. It stays ignored here too.
[ ! -f "$ROOT/AGENTS.md" ] || cp "$ROOT/AGENTS.md" "$DIR/AGENTS.md"
echo "created .worktrees/$ID on branch $BRANCH"
echo "run commands in it with: scripts/in-wt.sh $ID <command>"
