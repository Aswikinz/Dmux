#!/usr/bin/env bash
# T011: permanent regression test for prove.sh's EXIT SEMANTICS.
#
# Background: the Opus T008 gate claimed prove.sh always exits 0 even with FAILs.
# Maya disproved it with a sabotage experiment (a version-dependent expression in
# the row hash made identical data hash differently on the two servers; the run
# ended "Result: 80 passed, 3 failed" and exited 1). The real root cause of the
# misreading was a missing EOF newline: line-based review saw the final echo as
# the last line and never saw the `[ "$FAIL" -eq 0 ]` exit guard. This test makes
# the sabotage experiment permanent, so a harness that ever exits 0 with FAILs
# fails loudly here.
#
# What it does: copies prove.sh, guard.sh and ALL *.sql templates to a temp dir,
# injects `|| current_setting('server_version')` into the row hash of the copied
# fingerprint.sql (identical data then hashes differently on the two servers),
# runs the copied harness against the local mock pair, and asserts:
#   - the exit code is NON-ZERO,
#   - the output has "Result: N passed, M failed" with M > 0,
#   - at least one "  FAIL  " line, and the FAIL count matches M,
#   - at least one FAIL is "wanted same, got differ" (the sabotage signature).
# Afterwards it verifies the mocks are clean (no phase0 schema on either server,
# no bank_c/bank_ascii on the old server); leftovers are dropped and noted.
#
# RUNTIME ~90 s (one full 83-check run) and it WRITES to the mocks via the copied
# harness (creates/drops the phase0 schema and bank_c/bank_ascii — legitimate
# harness behaviour). Run ON DEMAND or NIGHTLY, NOT on every CI build:
#   scripts/in-wt.sh T011 bash testdata/phase0/exit_test.sh
# Optional arguments pick other local ports, exactly like prove.sh.
# No row data is printed: only check names, counts and server versions.
set -euo pipefail
# Same hostile-env stripping as prove.sh (T010 N3): the strict URLs below supply
# every connection parameter for this script's own verification psql calls.
unset PGHOSTADDR PGSERVICE PGSERVICEFILE PGHOST PGPORT PGDATABASE PGUSER PGPASSWORD
export PGOPTIONS="${PGOPTIONS:-} -c client_min_messages=warning"

OLD_URL="${OLD_URL:-postgres://dmux_dev:dmux_dev_only@127.0.0.1:${1:-5433}/bank}"
NEW_URL="${NEW_URL:-postgres://dmux_dev:dmux_dev_only@127.0.0.1:${2:-5434}/bank}"
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck disable=SC1091  # guard.sh is linted separately; $HERE is runtime-relative
. "$HERE/guard.sh"
for u in "$OLD_URL" "$NEW_URL"; do
  if ! dmux_guard_url "$u"; then
    echo "refusing: exit_test.sh only runs against local mock databases" >&2; exit 1
  fi
done

PASS=0; FAIL=0
ok()  { PASS=$((PASS+1)); printf '  ok    %s\n' "$1"; }
bad() { FAIL=$((FAIL+1)); printf '  FAIL  %s\n' "$1"; }

WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
cp "$HERE/prove.sh" "$HERE/guard.sh" "$WORK/"
cp "$HERE"/*.sql "$WORK/"

# Sabotage the COPY only: make the row hash version-dependent. The pattern
# occurs exactly once (row hash; the key hash ends in `bank_code::text)`), so the
# bucket assignment stays identical and every planted-change check still works —
# only the "identical data stays quiet" checks can fail. If the template ever
# changes shape, fail loudly instead of silently running an unsabotaged harness.
hits=$(grep -c 'bank_code::text,' "$WORK/fingerprint.sql" || true)
if [ "$hits" != 1 ]; then
  echo "exit_test: sabotage pattern found $hits times (wanted 1) in fingerprint.sql — template changed, update this test" >&2
  exit 1
fi
sed -i "s/bank_code::text,/bank_code::text || current_setting('server_version'),/" "$WORK/fingerprint.sql"
hits=$(grep -c "current_setting('server_version')" "$WORK/fingerprint.sql" || true)
if [ "$hits" != 1 ]; then
  echo "exit_test: sabotage injection failed (marker found $hits times, wanted 1)" >&2
  exit 1
fi
echo "Sabotaged copy ready (row hash now includes server_version). Running it against the mocks..."

# Capture the expected NON-ZERO exit without tripping set -e.
rc=0
bash "$WORK/prove.sh" "$@" > "$WORK/out" 2>&1 || rc=$?

echo
echo "Assertions on the sabotaged run:"
if [ "$rc" -ne 0 ]; then
  ok "harness exited non-zero with FAILs (exit $rc)"
else
  bad "harness exited 0 despite FAILs — exit guard broken (this is the regression)"
fi

result=$(grep -E '^Result: [0-9]+ passed, [0-9]+ failed$' "$WORK/out" || true)
if [ -n "$result" ]; then
  passed=$(printf '%s\n' "$result" | sed -E 's/^Result: ([0-9]+) passed, [0-9]+ failed$/\1/')
  failed=$(printf '%s\n' "$result" | sed -E 's/^Result: [0-9]+ passed, ([0-9]+) failed$/\1/')
  if [ "$failed" -gt 0 ] && [ "$passed" -gt 0 ]; then
    ok "summary line present with failures: $result"
  else
    bad "summary line has no failures (or no passes): $result"
  fi
else
  bad "no 'Result: N passed, M failed' line in harness output"
  passed=0; failed=0
fi

faillines=$(grep -c '^  FAIL  ' "$WORK/out" || true)
if [ "$faillines" -ge 1 ]; then
  ok "at least one '  FAIL  ' line ($faillines printed)"
else
  bad "no '  FAIL  ' line in harness output"
fi
if [ "$faillines" = "$failed" ]; then
  ok "FAIL line count matches the summary ($faillines = $failed)"
else
  bad "FAIL line count ($faillines) does not match summary failed count ($failed)"
fi
if grep -q 'wanted same, got differ' "$WORK/out"; then
  ok "sabotage signature present: identical data reported as differ"
else
  bad "no 'wanted same, got differ' FAIL — failures are not the injected version dependency"
fi

# Mock cleanliness: the copied harness drops phase0 (and bank_c/bank_ascii) even
# when checks FAIL. Verify; drop and note any leftovers.
echo
echo "Mock cleanup verification:"
q() { psql -X -q -v ON_ERROR_STOP=1 "$1" -c "$2" >/dev/null; }
val() { psql -X -q -A -t -v ON_ERROR_STOP=1 "$1" -c "$2"; }
for pair in "old|$OLD_URL" "new|$NEW_URL"; do
  side="${pair%%|*}"; u="${pair#*|}"
  left=$(val "$u" "SELECT count(*) FROM pg_namespace WHERE nspname = 'phase0'")
  if [ "$left" = 0 ]; then
    ok "$side: phase0 schema dropped by the harness, verified gone"
  else
    echo "  warn  $side: phase0 schema left behind — dropping it now (harness cleanup regression?)"
    q "$u" "DROP SCHEMA IF EXISTS phase0 CASCADE"
  fi
done
aux=$(val "$OLD_URL" "SELECT count(*) FROM pg_database WHERE datname IN ('bank_c', 'bank_ascii')")
if [ "$aux" = 0 ]; then
  ok "old: bank_c/bank_ascii dropped by the harness, verified gone"
else
  echo "  warn  old: auxiliary databases left behind — dropping them now (harness cleanup regression?)"
  q "$OLD_URL" "DROP DATABASE IF EXISTS bank_c"
  q "$OLD_URL" "DROP DATABASE IF EXISTS bank_ascii"
fi

if [ "$FAIL" -ne 0 ]; then
  echo
  echo "Sabotaged harness output (check names only, no row data):"
  grep -E '^  FAIL  |^Result: ' "$WORK/out" || true
fi
echo
echo "Result: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
