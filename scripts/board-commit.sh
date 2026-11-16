#!/usr/bin/env bash
# Commit the task board and decisions log on main (and nothing else).
#   scripts/board-commit.sh "T012 done"
set -euo pipefail
. "$(dirname "$0")/lib.sh"

ROOT="$(main_root)"; MSG="${1:-update}"
[ "$(git -C "$ROOT" rev-parse --abbrev-ref HEAD)" = main ] || die "the main checkout must be on branch 'main'"
FILES=()
for f in docs/TASKS.md docs/DECISIONS.md; do
  [ -n "$(git -C "$ROOT" status --porcelain -- "$f")" ] && FILES+=("$f")
done
[ ${#FILES[@]} -gt 0 ] || { echo "task board unchanged"; exit 0; }
clean_git -C "$ROOT" add -- "${FILES[@]}"
clean_git -C "$ROOT" commit -q -m "board: $MSG" -- "${FILES[@]}"
echo "committed ${FILES[*]}"
