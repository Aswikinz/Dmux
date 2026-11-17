#!/usr/bin/env bash
# T005: fingerprint timing harness (PLAN.md sections 3.1, 4.5, 8.1 "large").
#
# Usage:  testdata/phase0/perf.sh [old port] [new port] [size ...]
#   Defaults: old 5433, new 5434; sizes 1000000 and 20000000.
#   Sizes also overridable via PERF_SIZES="1000000 20000000"; load batch via
#   PERF_BATCH (default 1000000 rows per INSERT ... SELECT generate_series).
#   OLD_URL / NEW_URL may override the URLs (local addresses only).
#
# For each size, on each server, strictly sequentially:
#   1. drop/recreate schema phase0 with UNLOGGED table accounts_perf (same
#      column shape as phase0.accounts, no PK/indexes — keys are still hashed;
#      UNLOGGED speeds the load only, a read-only scan never writes WAL;
#      autovacuum is disabled on this throwaway table so no background worker
#      scans it during a timed run — the harness never relies on statistics);
#   2. load in PERF_BATCH-row batches, one transaction per batch;
#   3. record count(*) and pg_total_relation_size (PLAN 4.5: size on disk);
#   4. wait for any other active backend on the database to finish (max 30s),
#      then time the wall clock of `psql -f fingerprint_perf.sql` (the
#      fingerprint query ONLY, load excluded), twice: first scan after load,
#      then a repeat.
# No load or other scan runs during a timed scan. Fingerprints from the two
# servers must be byte-identical (same data, same template) — checked.
#
# Basis for Phase 3 estimate calibration and nightly large-dataset checks.
#
# WARNING: this script WRITES. It creates and drops a schema called "phase0".
# Run it only against the mock databases from testdata/compose.yml, never
# against a real system. (Dmux itself never writes; this is a test harness.)
set -euo pipefail
# T010 hardening (Priya N3): libpq prefers hostaddr (PGHOSTADDR or a PGSERVICE
# file entry) over the URI host, so env could redirect these WRITE-capable
# harnesses outside the URL guard. The strict URL supplies every connection
# parameter, so no PG* env is needed. (PGOPTIONS below is set by the harness
# itself, after this unset.)
unset PGHOSTADDR PGSERVICE PGSERVICEFILE PGHOST PGPORT PGDATABASE PGUSER PGPASSWORD
export PGOPTIONS="${PGOPTIONS:-} -c client_min_messages=warning"

OLD_URL="${OLD_URL:-postgres://dmux_dev:dmux_dev_only@127.0.0.1:${1:-5433}/bank}"
NEW_URL="${NEW_URL:-postgres://dmux_dev:dmux_dev_only@127.0.0.1:${2:-5434}/bank}"
HERE="$(cd "$(dirname "$0")" && pwd)"
# T010: strict local-only URL guard, shared with prove.sh (threat model in guard.sh).
# shellcheck disable=SC1091  # guard.sh is linted separately; $HERE is runtime-relative
. "$HERE/guard.sh"
for u in "$OLD_URL" "$NEW_URL"; do
  if ! dmux_guard_url "$u"; then
    echo "refusing: perf.sh only runs against local mock databases" >&2; exit 1
  fi
done

if [[ "$#" -gt 2 ]]; then SIZES="${*:3}"; else SIZES="${PERF_SIZES:-1000000 20000000}"; fi
BATCH="${PERF_BATCH:-1000000}"
read -r -a SIZE_LIST <<< "$SIZES"
[[ "${#SIZE_LIST[@]}" -gt 0 ]] || { echo "no sizes given" >&2; exit 1; }
for n in "${SIZE_LIST[@]}" "$BATCH"; do
  case "$n" in ''|*[!0-9]*) echo "sizes and PERF_BATCH must be positive integers: $n" >&2; exit 1 ;; esac
  if [[ "$n" -eq 0 ]]; then echo "sizes and PERF_BATCH must be positive integers" >&2; exit 1; fi
done

TMP="$(mktemp -d)"
STARTED=0
cleanup() {
  # Best effort on any exit path; only once this harness has created schemas.
  if [[ "$STARTED" = 1 ]]; then
    for u in "$OLD_URL" "$NEW_URL"; do
      psql -X -q -v ON_ERROR_STOP=1 "$u" -c 'DROP SCHEMA IF EXISTS phase0 CASCADE' >/dev/null 2>&1 || true
    done
  fi
}
trap 'cleanup; rm -rf "$TMP"' EXIT

q()   { psql -X -q -v ON_ERROR_STOP=1 "$1" -c "$2" >/dev/null; }
val() { psql -X -q -A -t -v ON_ERROR_STOP=1 "$1" -c "$2"; }
now_ns() { date +%s%N; }
elapsed() { awk -v a="$1" -v b="$2" 'BEGIN { printf "%.3f", (b - a) / 1000000000 }'; }

# Hardware context (the script runs unrestricted bash; servers are containers
# on this same host, so host CPU/RAM/disk is the relevant context).
cpu="$(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2- | sed 's/^ *//')"
cores="$(nproc)"
ram_gb="$(awk '/MemTotal/ { printf "%.1f", $2 / 1048576 }' /proc/meminfo)"
disk_gb="$(df -BG --output=avail "$HERE" | tail -1 | tr -dc '0-9')"
echo "Hardware: $cpu, $cores CPUs, ${ram_gb} GB RAM, ${disk_gb} GB free disk"
echo "Client:   $(psql --version)"
echo "Sizes:    ${SIZE_LIST[*]} rows; load batch $BATCH rows; template fingerprint_perf.sql"
echo "Servers:"
for pair in "old|$OLD_URL" "new|$NEW_URL"; do
  u="${pair#*|}"
  printf '  %s: ' "${pair%%|*}"
  val "$u" "SELECT current_setting('server_version') || ', ' || pg_encoding_to_char(encoding) || ', collate ' || datcollate || ', shared_buffers=' || current_setting('shared_buffers') || CASE WHEN current_setting('server_version') ~ '^9\.[0-5]\.' THEN '' ELSE ', max_parallel_workers_per_gather=' || current_setting('max_parallel_workers_per_gather') END FROM pg_database WHERE datname = current_database()"
done

# EXPLAIN variant of the template (plan shape only, never executed): same
# SELECT, wrapped in the same read-only transaction with pinned search_path.
select_sql="$(sed -n '/^SELECT/,/^ORDER BY 1;/p' "$HERE/fingerprint_perf.sql")"
{
  echo 'BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;'
  echo 'SET LOCAL search_path = pg_catalog;'
  printf 'EXPLAIN %s\n' "$select_sql"
  echo 'ROLLBACK;'
} > "$TMP/explain.sql"

load_perf() { # url, rows — batched INSERT ... SELECT, one transaction per batch
  local u="$1" rows="$2" lo hi t0 t1
  t0="$(now_ns)"
  lo=1
  while [[ "$lo" -le "$rows" ]]; do
    hi=$((lo + BATCH - 1))
    if [[ "$hi" -gt "$rows" ]]; then hi="$rows"; fi
    # Same value expressions as prove.sh's load_accounts (accents via chr(),
    # NULL/empty/remark mix), so the perf data matches the proven shape.
    q "$u" "INSERT INTO phase0.accounts_perf (acct_id, bank_code, holder_name, branch_code, balance, rate, opened_at, doc_blob, remarks)
    SELECT lpad(g::text, 11, '0'), 'BANK01',
           CASE g % 3 WHEN 0 THEN 'Jos' || chr(233) || ' Mu' || chr(241) || 'oz ' WHEN 1 THEN 'Zo' || chr(235) || ' ' || chr(197) || 'str' || chr(246) || 'm ' ELSE 'Plain Name ' END || g,
           'S' || (g % 500), (g * 1234.5678)::numeric(20,4), g / 7.0,
           timestamp '2001-01-01 00:00:00' + (g || ' minutes')::interval,
           decode(md5(g::text), 'hex'),
           CASE g % 4 WHEN 0 THEN NULL WHEN 1 THEN '' ELSE 'remark ' || g END
    FROM generate_series($lo, $hi) g;"
    lo=$((hi + 1))
  done
  t1="$(now_ns)"
  elapsed "$t0" "$t1"
}

scan() { # url, outfile — wall clock of the fingerprint query ONLY; prints seconds
  local u="$1" out="$2" t0 t1
  t0="$(now_ns)"
  psql -X -q -A -t -v ON_ERROR_STOP=1 "$u" -f "$HERE/fingerprint_perf.sql" > "$out.raw"
  t1="$(now_ns)"
  grep -E '^[0-9]+\|' "$out.raw" > "$out" || true
  elapsed "$t0" "$t1"
}

check_fp() { # outfile, rows — sanity: non-empty, <=4096 buckets, counts sum to rows
  local out="$1" rows="$2" buckets sumn
  buckets="$(wc -l < "$out")"
  sumn="$(awk -F'|' '{ s += $2 } END { print s + 0 }' "$out")"
  if [[ "$buckets" -eq 0 ]] || [[ "$buckets" -gt 4096 ]] || [[ "$sumn" != "$rows" ]]; then
    echo "fingerprint sanity failed: $buckets buckets, sum(n)=$sumn, wanted $rows rows" >&2
    exit 1
  fi
}

active_info() { # url — classify other active backends; never print query text (rule 7)
  val "$1" "SELECT coalesce(string_agg(CASE WHEN query LIKE 'autovacuum:%' THEN 'autovacuum' ELSE 'other' END, ','), '') FROM pg_stat_activity WHERE state = 'active' AND pid <> pg_backend_pid() AND datname = current_database()"
}

wait_quiet() { # url — let other active backends finish before a timed scan (max 30s)
  local u="$1" waited=0 info
  info="$(active_info "$u")"
  while [[ -n "$info" ]] && [[ "$waited" -lt 30 ]]; do
    echo "  waiting for quiescence (active: $info, ${waited}s elapsed)"
    sleep 5
    waited=$((waited + 5))
    info="$(active_info "$u")"
  done
  if [[ -n "$info" ]]; then
    echo "  warn: other active backend(s) during the timed scans: $info"
  fi
}

METRICS="$TMP/metrics"
: > "$METRICS"
for rows in "${SIZE_LIST[@]}"; do
  for pair in "old|$OLD_URL" "new|$NEW_URL"; do
    side="${pair%%|*}"; u="${pair#*|}"
    echo "== $side: $rows rows =="
    STARTED=1
    q "$u" "DROP SCHEMA IF EXISTS phase0 CASCADE; CREATE SCHEMA phase0;
    CREATE UNLOGGED TABLE phase0.accounts_perf (
      acct_id varchar(11), bank_code varchar(8), holder_name varchar(80), branch_code char(8),
      balance numeric(20,4), rate float8, opened_at timestamp(0),
      doc_blob bytea, remarks varchar(60)) WITH (autovacuum_enabled = off);"
    load_s="$(load_perf "$u" "$rows")"
    cnt="$(val "$u" "SELECT count(*) FROM phase0.accounts_perf")"
    if [[ "$cnt" != "$rows" ]]; then echo "load produced $cnt rows, wanted $rows" >&2; exit 1; fi
    bytes="$(val "$u" "SELECT pg_total_relation_size('phase0.accounts_perf')")"
    plan="$(psql -X -q -A -t -v ON_ERROR_STOP=1 "$u" -f "$TMP/explain.sql")"
    if grep -q 'Gather' <<< "$plan"; then plan_note='parallel (Gather)'; else plan_note='single process'; fi
    echo "  load ${load_s}s; count=$cnt; size=${bytes} bytes; plan: $plan_note"
    wait_quiet "$u"
    s1="$(scan "$u" "$TMP/fp_${side}_${rows}_1")"
    check_fp "$TMP/fp_${side}_${rows}_1" "$rows"
    s2="$(scan "$u" "$TMP/fp_${side}_${rows}_2")"
    check_fp "$TMP/fp_${side}_${rows}_2" "$rows"
    if ! cmp -s "$TMP/fp_${side}_${rows}_1" "$TMP/fp_${side}_${rows}_2"; then
      echo "repeat scan differs from first scan — nondeterministic result" >&2; exit 1
    fi
    echo "  scan1 ${s1}s (first after load); scan2 ${s2}s (repeat); fingerprint identical on repeat"
    printf '%s|%s|%s|%s|%s\n' "$side" "$rows" "$bytes" "$s1" "$s2" >> "$METRICS"
  done
  if cmp -s "$TMP/fp_old_${rows}_1" "$TMP/fp_new_${rows}_1"; then
    echo "  ok: old and new fingerprints byte-identical for $rows rows"
  else
    echo "old and new fingerprints DIFFER for $rows rows — timing invalid" >&2; exit 1
  fi
done

echo
echo "Results (scan1 = first scan after load; MB = MiB = 1048576 bytes; size = pg_total_relation_size):"
awk -F'|' '
  BEGIN { printf "%-6s | %10s | %10s | %9s | %10s | %9s | %9s | %7s\n", "server", "rows", "size MiB", "scan1 s", "rows/s", "MiB/s", "scan2 s", "delta%" }
  { mb = $3 / 1048576
    printf "%-6s | %10d | %10.1f | %9.3f | %10.0f | %9.1f | %9.3f | %+6.1f%%\n", $1, $2, mb, $4, $2 / $4, mb / $4, $5, ($5 - $4) * 100 / $4 }
' "$METRICS"

echo
echo "Cleanup:"
for pair in "old|$OLD_URL" "new|$NEW_URL"; do
  u="${pair#*|}"
  q "$u" "DROP SCHEMA IF EXISTS phase0 CASCADE"
  left="$(val "$u" "SELECT count(*) FROM pg_namespace WHERE nspname = 'phase0'")"
  if [[ "$left" != 0 ]]; then echo "cleanup failed on ${pair%%|*}: phase0 schema still present" >&2; exit 1; fi
  echo "  ${pair%%|*}: phase0 schema dropped, verified gone"
done
STARTED=0
echo "Done."
