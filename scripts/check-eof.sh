#!/usr/bin/env bash
# CI check (the T011 lesson): every tracked text file ends with a newline.
# A harness once lacked its final newline, which hid its real exit line from
# line-based review. Binary files (icons etc.) and empty files are skipped.
#   scripts/in-wt.sh T006 bash scripts/check-eof.sh
set -euo pipefail

BAD=0
CHECKED=0
while IFS= read -r f; do
  [ -f "$f" ] || continue
  [ -s "$f" ] || continue          # empty file: nothing to check
  grep -Iq '' "$f" || continue     # binary file: skip
  CHECKED=$((CHECKED + 1))
  last="$(tail -c 1 "$f" | od -An -tx1 | tr -d '[:space:]')"
  if [ "$last" != "0a" ]; then
    echo "eof: missing final newline: $f"
    BAD=1
  fi
done < <(git -c core.quotepath=off ls-files)

if [ "$BAD" -eq 0 ]; then
  echo "eof: ok ($CHECKED text file(s) checked)"
fi
exit "$BAD"
