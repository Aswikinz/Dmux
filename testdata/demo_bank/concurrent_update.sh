#!/usr/bin/env bash
# demo_bank: the PLAN.md 8.2 "row updated while the check runs" case (T016).
#
# Usage:
#   testdata/demo_bank/concurrent_update.sh              # 60 seconds, new :5434
#   testdata/demo_bank/concurrent_update.sh 30           # 30 seconds
#   testdata/demo_bank/concurrent_update.sh 30 5437      # 30 seconds, new on :5437
#   NEW_URL=... testdata/demo_bank/concurrent_update.sh
#
# This one cannot be planted statically: the engine has to see the change happen
# DURING its run. The answer-key harness starts this script just after the engine
# has read the first table's counts; the run must end as "changed during check"
# (amber), never PASS.
#
# Each update is its own committed transaction, so the change counters and the
# row value really move while the engine scans. It touches exactly one invented
# row (demo_bank.notes.note_id 1500) in the NEW mock database and nothing else.
#
# WARNING: unlike the engine, this script writes. It is test harness code and
# only ever runs against a local mock database, guarded below.
set -euo pipefail
# Same hardening as build.sh / verify.sh: no PG* environment may override the URL.
unset PGHOSTADDR PGSERVICE PGSERVICEFILE PGHOST PGPORT PGDATABASE PGUSER PGPASSWORD
export PGOPTIONS="${PGOPTIONS:-} -c client_min_messages=warning"
export PGCONNECT_TIMEOUT="${PGCONNECT_TIMEOUT:-10}"

HERE="$(cd "$(dirname "$0")" && pwd)"
DURATION="${1:-60}"
PORT="${2:-5434}"

case "$DURATION" in
  ''|*[!0-9]*) echo "usage: concurrent_update.sh [seconds] [new port]" >&2; exit 2 ;;
esac
[ "$DURATION" -gt 0 ] || { echo "seconds must be positive" >&2; exit 2; }
case "$PORT" in
  *[!0-9]*) echo "usage: concurrent_update.sh [seconds] [new port]" >&2; exit 2 ;;
esac

NEW_URL="${NEW_URL:-postgres://dmux_dev:dmux_dev_only@127.0.0.1:${PORT}/demo_bank}"

# shellcheck disable=SC1091  # guard.sh is linted separately; $HERE is runtime-relative
. "$HERE/../phase0/guard.sh"
if ! dmux_guard_url "$NEW_URL"; then
  echo "refusing: concurrent_update.sh only runs against local mock databases" >&2
  exit 1
fi

NOTE_ID=1500

rows="$(psql -X -q -A -t -v ON_ERROR_STOP=1 "$NEW_URL" \
  -c "SELECT count(*) FROM demo_bank.notes WHERE note_id = $NOTE_ID")"
if [ "$rows" != "1" ]; then
  echo "refusing: demo_bank.notes note_id $NOTE_ID not found (build the answer key first)" >&2
  exit 1
fi

echo "Updating demo_bank.notes note_id $NOTE_ID in the new database for ${DURATION}s..."
count=0
end=$((SECONDS + DURATION))
while [ "$SECONDS" -lt "$end" ]; do
  if [ $((count % 2)) -eq 0 ]; then token='runtime probe A'; else token='runtime probe B'; fi
  psql -X -q -v ON_ERROR_STOP=1 "$NEW_URL" \
    -c "UPDATE demo_bank.notes SET note_text = '$token' WHERE note_id = $NOTE_ID" >/dev/null
  count=$((count + 1))
  sleep 0.05
done
printf '%s updates applied to one row of demo_bank.notes in the new database\n' "$count"
