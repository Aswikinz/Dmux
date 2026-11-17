#!/usr/bin/env bash
# Run one allowed command inside a task's working copy.
#   scripts/in-wt.sh T012 cargo test --workspace
#   scripts/in-wt.sh T012 git diff main...HEAD
# Only build, test and read-only Git commands are allowed. Committing, merging
# and pushing go through wt-commit.sh and wt-merge.sh, which only Maya may run.
set -euo pipefail
. "$(dirname "$0")/lib.sh"

ID="${1:-}"; valid_id "$ID"; shift
[[ $# -gt 0 ]] || die "usage: scripts/in-wt.sh <task ID> <command> [args...]"
DIR="$(wt_dir "$ID")"; [[ -d "$DIR" ]] || die "no working copy for $ID (scripts/wt-new.sh creates one)"

case "$1" in
  cargo|rustfmt|pnpm|npx|node|python3|uv|podman|psql|pg_dump|pg_restore|pg_isready|actionlint|gitleaks|shellcheck|ls|cat|bash)
    if [[ "$1" = bash ]]; then
      case "${2:-}" in testdata/*.sh|scripts/check-*.sh) ;;
        *) die "bash may only run scripts under testdata/ or scripts/check-*.sh" ;; esac
    fi ;;
  git)
    case "${2:-}" in status|diff|log|show|ls-files|blame) ;;
      *) die "only read-only git commands are allowed here (status, diff, log, show, ls-files, blame)" ;; esac ;;
  *) die "'$1' is not on the allowed list for working copies" ;;
esac

ROOT="$(main_root)"
export DMUX_ROOT="$ROOT" DMUX_PRIVATE="$ROOT/private" DMUX_TASK="$ID"
cd "$DIR"
exec "$@"
