#!/usr/bin/env bash
# Show every open task working copy, its branch, and whether it has unsaved work.
set -euo pipefail
. "$(dirname "$0")/lib.sh"

ROOT="$(main_root)"
shopt -s nullglob
FOUND=0
for d in "$ROOT"/.worktrees/*/; do
  FOUND=1; id="$(basename "$d")"
  branch="$(git -C "$d" rev-parse --abbrev-ref HEAD)"
  ahead="$(git -C "$ROOT" rev-list --count "main..$branch")"
  dirty="clean"; [ -n "$(git -C "$d" status --porcelain)" ] && dirty="uncommitted changes"
  printf '%-6s %-40s %s commit(s) ahead of main, %s\n' "$id" "$branch" "$ahead" "$dirty"
done
[ "$FOUND" -eq 1 ] || echo "no open working copies"
