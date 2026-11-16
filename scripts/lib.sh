#!/usr/bin/env bash
# Shared helpers for the Dmux worktree scripts. Sourced, not run.

die() { echo "error: $*" >&2; exit 1; }

# Absolute path of the MAIN checkout, even when called from inside a worktree.
main_root() {
  local common
  common="$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null)" \
    || die "not inside a Git repository"
  dirname "$common"
}

valid_id() { [[ "$1" =~ ^T[0-9]{3,4}$ ]] || die "task ID must look like T001, got '$1'"; }

wt_dir() { echo "$(main_root)/.worktrees/$1"; }

wt_branch() { # the task branch checked out in that worktree
  git -C "$(wt_dir "$1")" rev-parse --abbrev-ref HEAD
}

# Git with the caller's identity variables removed, so a commit can only
# carry the identity stored in the repo's own Git config.
clean_git() {
  env -u GIT_AUTHOR_NAME -u GIT_AUTHOR_EMAIL -u GIT_AUTHOR_DATE \
      -u GIT_COMMITTER_NAME -u GIT_COMMITTER_EMAIL -u GIT_COMMITTER_DATE \
      git "$@"
}
