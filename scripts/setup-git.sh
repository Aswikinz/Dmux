#!/usr/bin/env bash
# One-time Git setup for Dmux. Run once from the main checkout after copying
# the package in:   bash scripts/setup-git.sh
#
# It switches on the commit hooks (for the main copy and every task worktree)
# and records you as the only allowed commit author.
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/lib.sh

ROOT="$(main_root)"
[[ "$ROOT" = "$(pwd -P)" ]] || die "run this from the main checkout ($ROOT), not from a worktree"

NAME="$(git config user.name || true)"; EMAIL="$(git config user.email || true)"
[[ -n "$NAME" ]] && [[ -n "$EMAIL" ]] || die "set your Git identity first:
  git config --global user.name  \"Your Name\"
  git config --global user.email \"you@example.com\"   # must be verified on GitHub"

# Zip files and some copies drop the executable flag, so restore it.
chmod +x scripts/*.sh .githooks/* testdata/phase0/*.sh 2>/dev/null || true

# An absolute path means every worktree uses the main copy's hooks.
git config core.hooksPath "$ROOT/.githooks"
git config dmux.ownerName  "$NAME"
git config dmux.ownerEmail "$EMAIL"
# Keep the identity local to this repo as well, so nothing global can drift.
git config user.name  "$NAME"
git config user.email "$EMAIL"

for entry in "private/" ".worktrees/"; do
  grep -qxF "$entry" .gitignore 2>/dev/null || echo "$entry" >> .gitignore
done

echo "Hooks on:       $ROOT/.githooks"
echo "Commit author:  $NAME <$EMAIL>"
echo "Ignored:        private/  .worktrees/"
echo
echo "Check that $EMAIL is a verified email on your GitHub account,"
echo "or GitHub will not link commits to your username."
