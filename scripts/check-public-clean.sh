#!/usr/bin/env bash
# CI check (decision 11): private vendor names and derived details must not
# appear in tracked file NAMES or CONTENTS. The patterns themselves never live
# in this public repo. They come from:
#   1. DMUX_PRIVATE_PATTERNS — comma-separated, case-insensitive extended
#      regexes; in CI this env is fed from the secret PRIVATE_NAME_PATTERNS;
#   2. otherwise `git config dmux.privateNamePattern` (local convenience).
# Fails closed when neither source is configured.
#   scripts/in-wt.sh T006 bash scripts/check-public-clean.sh
set -euo pipefail

PATTERNS="${DMUX_PRIVATE_PATTERNS:-}"
if [[ -z "$PATTERNS" ]]; then
  PATTERNS="$(git config --get dmux.privateNamePattern || true)"
fi
if [[ -z "$PATTERNS" ]]; then
  echo "public-clean: FAIL CLOSED - no private patterns configured." >&2
  echo "CI: set the PRIVATE_NAME_PATTERNS secret (wired to DMUX_PRIVATE_PATTERNS)." >&2
  echo "Local: git config dmux.privateNamePattern '<comma-separated patterns>'" >&2
  exit 1
fi

# Comma-separated list -> one case-insensitive ERE alternation.
ALT=""
IFS=',' read -ra PARTS <<< "$PATTERNS"
for p in "${PARTS[@]}"; do
  p="$(printf '%s' "$p" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')"
  [[ -n "$p" ]] || continue
  if [[ -n "$ALT" ]]; then ALT="$ALT|$p"; else ALT="$p"; fi
done
if [[ -z "$ALT" ]]; then
  echo "public-clean: pattern list is empty after trimming." >&2
  exit 1
fi

BAD=0

# 1) Tracked file names.
while IFS= read -r f; do
  if printf '%s\n' "$f" | grep -qiE -e "$ALT"; then
    echo "public-clean: private pattern in tracked file name: $f"
    BAD=1
  fi
done < <(git -c core.quotepath=off ls-files)

# 2) Tracked file contents in the working tree (-I skips binary files).
set +e
MATCHES="$(git grep -I -i -l -E -e "$ALT")"
rc=$?
set -e
if [[ "$rc" -gt 1 ]]; then
  echo "public-clean: git grep failed (exit $rc)." >&2
  exit 1
fi
if [[ -n "$MATCHES" ]]; then
  while IFS= read -r f; do
    echo "public-clean: private pattern in content: $f"
    BAD=1
  done <<< "$MATCHES"
fi

# Decision 12: AGENTS.md is local-only and must never be tracked.
if git ls-files --error-unmatch AGENTS.md >/dev/null 2>&1; then
  echo "public-clean: AGENTS.md is tracked; it is local-only (decision 12)."
  BAD=1
fi

if [[ "$BAD" -eq 0 ]]; then
  echo "public-clean: ok (tracked names and contents scanned)"
fi
exit "$BAD"
