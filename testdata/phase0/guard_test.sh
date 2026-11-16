#!/usr/bin/env bash
# T010: regression battery for the shared local-only URL guard (guard.sh).
# Runs WITHOUT any database. Two parts:
#   1. the guard predicate against accept/reject cases, including every R1
#      bypass form (multi-host comma, ?host=/?port=/?service=/?options=, extra @);
#   2. harness wiring: both harnesses refuse a bypass URL BEFORE any connection
#      attempt, and (Priya N3) strip hostile PG* env before any psql call while
#      a valid URL still passes the guard. psql is shadowed by a stub that exits
#      97 and must never run on a refusal, so even a hypothetical guard failure
#      cannot contact a database.
# Prints a pass/fail summary and exits non-zero if any expectation fails.
#   scripts/in-wt.sh T010 bash testdata/phase0/guard_test.sh
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck disable=SC1091  # guard.sh is linted separately; $HERE is runtime-relative
. "$HERE/guard.sh"
# N4: the uppercase-scheme reject case assumes default bash options; an
# inherited (exported) BASHOPTS=nocasematch would widen the guard (Priya N2).
shopt -u nocasematch

PASS=0; FAIL=0
ok()  { PASS=$((PASS+1)); printf '  ok    %s\n' "$1"; }
bad() { FAIL=$((FAIL+1)); printf '  FAIL  %s\n' "$1"; }

expect() { # want (accept|reject), url, label
  local want="$1" u="$2" label="$3" got
  if dmux_guard_url "$u"; then got=accept; else got=reject; fi
  if [ "$got" = "$want" ]; then ok "$label ($got)"
  else bad "$label (wanted $want, got $got)"; fi
}

echo "Guard predicate — accept (strict single-host local URL):"
expect accept 'postgres://dmux_dev:dmux_dev_only@127.0.0.1:5433/bank' 'mock old URL'
expect accept 'postgres://dmux_dev:dmux_dev_only@127.0.0.1:5434/bank' 'mock new URL'
expect accept 'postgres://dmux_dev:dmux_dev_only@localhost:5433/bank'  'localhost host'
expect accept 'postgres://other_user:pw123@localhost:5435/bank_c'      'strict-form OLD_URL/NEW_URL override'
# N4: percent-encoding is allowed in the value classes and routing-safe — libpq
# splits the raw URI on structural chars first, then decodes each component
# (Priya's libpq-source analysis, T010 re-review §2).
expect accept 'postgres://user%40x:pass%2Cword@127.0.0.1:5433/bank' 'percent-encoded credentials'
expect accept 'postgres://dmux_dev:dmux_dev_only@127.0.0.1:5433/bank%3Fhost%3Devil' 'percent-encoded dbname (literal ?host= text, not a query)'

echo "Guard predicate — reject (everything else, incl. the R1 bypasses):"
expect reject 'postgres://dmux_dev:dmux_dev_only@127.0.0.1:59999,127.0.0.1:5433/bank' 'R1a multi-host comma (both local)'
expect reject 'postgres://dmux_dev:dmux_dev_only@127.0.0.1:5433,db.example.com:5433/bank' 'R1a multi-host comma (remote 2nd host)'
expect reject 'postgres://dmux_dev:dmux_dev_only@127.0.0.1:59999/bank?host=127.0.0.1&port=5433' 'R1b ?host=&port= override'
expect reject 'postgres://dmux_dev:dmux_dev_only@127.0.0.1:5433/bank?host=db.example.com' '?host= remote override'
expect reject 'postgres://dmux_dev:dmux_dev_only@127.0.0.1:5433/bank?port=5432' '?port= override'
expect reject 'postgres://dmux_dev:dmux_dev_only@127.0.0.1:5433/bank?service=evil' '?service= override'
expect reject 'postgres://dmux_dev:dmux_dev_only@127.0.0.1:5433/bank?options=-c%20search_path%3Devil' '?options= override'
expect reject 'postgres://dmux_dev:dmux_dev_only@db.example.com:5433/bank' 'remote host'
expect reject 'postgres://u:p@ss@127.0.0.1:5433/bank' "two '@' (embedded in password)"
expect reject 'postgres://u@extra:p@127.0.0.1:5433/bank' "two '@' (embedded in user)"
expect reject 'mysql://dmux_dev:dmux_dev_only@127.0.0.1:5433/bank' 'non-postgres scheme'
expect reject 'postgresql://dmux_dev:dmux_dev_only@127.0.0.1:5433/bank' 'postgresql:// scheme (strict form is postgres://)'
expect reject '' 'empty string'
expect reject 'postgres://dmux_dev:dmux_dev_only@127.0.0.1:5433/' 'missing dbname (trailing slash)'
expect reject 'postgres://dmux_dev:dmux_dev_only@127.0.0.1:5433' 'missing dbname (no slash)'
expect reject 'postgres://u/p:p@127.0.0.1:5433/bank' "'/' inside credentials"
expect reject 'postgres://u:p,x@127.0.0.1:5433/bank' "',' inside credentials"
expect reject 'postgres://dmux_dev:dmux_dev_only@127.0.0.1:abc/bank' 'non-numeric port'
expect reject 'postgres://dmux_dev:dmux_dev_only@127.0.0.1/bank' 'missing port'
expect reject 'postgres://dmux_dev:dmux_dev_only@127.0.0.1:5433/bank/extra' 'path after dbname'
expect reject 'postgres://dmux_dev:dmux_dev_only@127.0.0.2:5433/bank' 'other loopback address'
expect reject 'postgres://dmux_dev:dmux_dev_only@[::1]:5433/bank' 'IPv6 loopback'
expect reject 'postgres://dmux_dev@127.0.0.1:5433/bank' 'missing password'
expect reject 'postgres://dmux_dev:dmux_dev_only@LOCALHOST:5433/bank' 'uppercase host (fail closed)'
expect reject $'postgres://dmux_dev:dmux_dev_only@127.0.0.1:5433/bank\n' 'trailing newline in dbname'
# N4: forms Priya probed in the T010 re-review (§2/§4), folded into the battery.
expect reject $'postgres://dmux_dev:dmux_dev_only@127.0.0.1:5433/ba\tnk' 'TAB embedded in dbname'
expect reject $'postgres://dmux_dev:dmux_dev_only@127.0.0.1:5433/ba\nnk' 'LF embedded in dbname'
expect reject $'postgres://dmux_dev:dmux_dev_only@127.0.0.1:5433/ba\rnk' 'CR embedded in dbname'
expect reject 'postgres://dmux_dev:dmux_dev_only@127.0.0.1.evil.com:5433/bank' 'host-suffix lookalike'
expect reject 'postgres://dmux_dev:dmux_dev_only@0177.0.0.1:5433/bank' 'octal loopback'
expect reject 'postgres://dmux_dev:dmux_dev_only@2130706433:5433/bank' 'decimal loopback'
expect reject 'postgres://dmux_dev:dmux_dev_only@127.1:5433/bank' 'short-form loopback'
expect reject 'postgres://dmux_dev:dmux_dev_only@[::ffff:127.0.0.1]:5433/bank' 'IPv4-mapped IPv6 loopback ([::1] already covered above)'
expect reject 'postgres://dmux_dev:dmux_dev_only@127.0.0.1:5433/bank#x' 'fragment'
expect reject 'postgres://dmux_dev:dmux_dev_only@127.0.0.1:+5433/bank' 'signed port (+)'
expect reject 'postgres://dmux_dev:dmux_dev_only@127.0.0.1:-5433/bank' 'signed port (-)'
expect reject 'POSTGRES://dmux_dev:dmux_dev_only@127.0.0.1:5433/bank' 'uppercase scheme (default bash options)'

echo
echo "Harness wiring — must refuse BEFORE any connection (psql stubbed to exit 97):"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
printf '#!/bin/sh\nexit 97\n' > "$TMP/psql"; chmod +x "$TMP/psql"
# A ?host/?port override aimed at an unused local port. The guard must reject it;
# the stub guarantees no database is contacted even if the guard were broken.
BYPASS='postgres://dmux_dev:dmux_dev_only@127.0.0.1:5433/bank?host=127.0.0.1&port=59999'
wiring() { # script, label
  local script="$1" label="$2" rc=0
  PATH="$TMP:$PATH" OLD_URL="$BYPASS" bash "$HERE/$script" >"$TMP/out" 2>"$TMP/err" || rc=$?
  if [ "$rc" -eq 1 ] && grep -q 'refusing:' "$TMP/err"; then
    ok "$label refused before connecting (exit 1)"
  else
    bad "$label (exit $rc, wanted 1 + 'refusing:'); first stderr line: $(head -1 "$TMP/err")"
  fi
}
wiring prove.sh 'prove.sh'
wiring perf.sh  'perf.sh'

echo
echo "Harness wiring — hostile PG* env (Priya N3): stripped before any psql, guard still holds:"
VALID_OLD='postgres://dmux_dev:dmux_dev_only@127.0.0.1:5433/bank'
VALID_NEW='postgres://dmux_dev:dmux_dev_only@127.0.0.1:5434/bank'
# Service file the hostile PGSERVICE/PGSERVICEFILE would resolve to if they
# survived; it supplies a remote hostaddr, which beats the URI host in libpq.
printf '[evil]\nhostaddr=198.51.100.1\nhost=198.51.100.1\n' > "$TMP/evil.service"
HOSTILE=(PGHOSTADDR=198.51.100.1 PGHOST=198.51.100.1 PGSERVICE=evil
         PGSERVICEFILE="$TMP/evil.service" PGPORT=59999 PGDATABASE=prod
         PGUSER=evil PGPASSWORD=evil)
# Leak-detecting stub: records which hostile PG* vars are still set in psql's
# environment ("none" if the harness unset them all), then exits 97 like the
# plain stub — reaching it proves the harness proceeded past the guard.
cat > "$TMP/psql" <<'STUB'
#!/bin/sh
leaked=none
for v in PGHOSTADDR PGSERVICE PGSERVICEFILE PGHOST PGPORT PGDATABASE PGUSER PGPASSWORD; do
  eval "flag=\${$v+set}"
  if [ "$flag" = set ]; then leaked="$leaked $v"; fi
done
printf '%s\n' "$leaked" > "$STUB_MARKER"
exit 97
STUB
chmod +x "$TMP/psql"

run_hostile() { # script, old url, new url — hostile env + leak stub
  PATH="$TMP:$PATH" STUB_MARKER="$TMP/leak" env "${HOSTILE[@]}" \
    OLD_URL="$2" NEW_URL="$3" bash "$HERE/$1" >"$TMP/out" 2>"$TMP/err"
}
wiring_env_pass() { # script, label — valid URLs: guard passes, stub sees NO PG* vars
  local script="$1" label="$2" rc=0
  rm -f "$TMP/leak"
  run_hostile "$script" "$VALID_OLD" "$VALID_NEW" || rc=$?
  if [ "$rc" -eq 97 ] && [ "$(cat "$TMP/leak" 2>/dev/null)" = none ]; then
    ok "$label hostile PG* env stripped, guard passed (stub reached, no leak)"
  else
    bad "$label (exit $rc, wanted 97 + marker 'none'); marker: $(cat "$TMP/leak" 2>/dev/null || echo missing); first stderr line: $(head -1 "$TMP/err")"
  fi
}
wiring_env_pass prove.sh 'prove.sh'
wiring_env_pass perf.sh  'perf.sh'

wiring_env_refuse() { # script, bypass side (OLD|NEW), label — refusal must hold under hostile env
  local script="$1" side="$2" label="$3" rc=0 ou="$VALID_OLD" nu="$VALID_NEW"
  if [ "$side" = OLD ]; then ou="$BYPASS"; else nu="$BYPASS"; fi
  rm -f "$TMP/leak"
  run_hostile "$script" "$ou" "$nu" || rc=$?
  if [ "$rc" -eq 1 ] && grep -q 'refusing:' "$TMP/err" && [ ! -e "$TMP/leak" ]; then
    ok "$label refused with hostile env (exit 1, no psql)"
  else
    bad "$label (exit $rc, wanted 1 + 'refusing:' + no stub marker); first stderr line: $(head -1 "$TMP/err")"
  fi
}
wiring_env_refuse prove.sh OLD 'prove.sh OLD_URL bypass'
wiring_env_refuse prove.sh NEW 'prove.sh NEW_URL bypass'
wiring_env_refuse perf.sh  OLD 'perf.sh OLD_URL bypass'
wiring_env_refuse perf.sh  NEW 'perf.sh NEW_URL bypass'

echo
echo "Result: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
