#!/usr/bin/env bash
# demo_bank size and scan timing: the nightly estimate-accuracy gate's input.
# (PLAN.md 4.5 "estimates use size on disk", 8.1 "large", 9 nightly workflow.)
#
# Usage:
#   testdata/demo_bank/estimate_check.sh                    # demo_bank_large, accounts + transactions
#   testdata/demo_bank/estimate_check.sh --db demo_bank     # the small size
#   testdata/demo_bank/estimate_check.sh --tables accounts,transactions,gl_entries
#   testdata/demo_bank/estimate_check.sh --out /tmp/estimate.json
#   testdata/demo_bank/estimate_check.sh 5435 5437 --db demo_bank_large
#
# For every table on each side it records three numbers:
#   rows     exact, summed from the fingerprint's own bucket counts (no extra scan)
#   bytes    pg_total_relation_size, the basis PLAN.md 4.5 says estimates use
#   seconds  wall clock of one full fingerprint scan
#
# The scan is the phase0 fingerprint shape (testdata/phase0/fingerprint.sql):
# order-independent bucket sums over md5 row hashes, key hash decides the bucket,
# canonical settings block, 4,096 buckets. It is generated from the catalog, the
# way the engine generates it, so any demo_bank table can be measured. Both
# servers' fingerprints must be byte-identical: if they are not, the timing basis
# is invalid and this script exits non-zero.
#
# Output: one small JSON document on stdout, or in --out FILE:
#
#   {"version":1,
#    "database":"demo_bank_large",
#    "generated_at":"2026-11-17T10:00:00Z",
#    "tables":[{"table":"accounts",
#               "old":{"rows":1000000,"bytes":234567890,"seconds":2.345},
#               "new":{"rows":1000000,"bytes":234567890,"seconds":1.987}},
#              ...],
#    "totals":{"old":{"rows":...,"bytes":...,"seconds":...},
#              "new":{"rows":...,"bytes":...,"seconds":...}}}
#
# A progress table goes to stderr, so `... > estimate.json` stays clean.
#
# Read-only: every statement runs in a READ ONLY transaction, the session is
# read-only too, and every URL passes the strict local-mock guard
# (testdata/phase0/guard.sh). Nothing here writes.
set -euo pipefail
# Same hardening as the other harnesses: no PG* environment may override the URLs.
unset PGHOSTADDR PGSERVICE PGSERVICEFILE PGHOST PGPORT PGDATABASE PGUSER PGPASSWORD
export PGOPTIONS="${PGOPTIONS:-} -c client_min_messages=warning -c default_transaction_read_only=on"
export PGCONNECT_TIMEOUT="${PGCONNECT_TIMEOUT:-10}"

DB="demo_bank_large"
TABLES="accounts,transactions"
OUT=""
ARGS=()
NEXT=""
for arg in "$@"; do
  if [[ -n "$NEXT" ]]; then
    case "$NEXT" in
      db)     DB="$arg" ;;
      tables) TABLES="$arg" ;;
      out)    OUT="$arg" ;;
    esac
    NEXT=""
    continue
  fi
  case "$arg" in
    --db=*)     DB="${arg#--db=}" ;;
    --tables=*) TABLES="${arg#--tables=}" ;;
    --out=*)    OUT="${arg#--out=}" ;;
    --db)       NEXT=db ;;
    --tables)   NEXT=tables ;;
    --out)      NEXT=out ;;
    -h|--help)
      sed -n '3,/^set -euo pipefail/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'
      exit 0 ;;
    --*) echo "estimate_check.sh: unknown option: $arg" >&2; exit 2 ;;
    *) ARGS+=("$arg") ;;
  esac
done
if [[ -n "$NEXT" ]]; then
  echo "usage: estimate_check.sh [old port] [new port] [--db NAME] [--tables a,b] [--out FILE]" >&2
  exit 2
fi
case "$DB" in
  demo_bank|demo_bank_large) ;;
  *) echo "refusing: estimate_check.sh only reads demo_bank or demo_bank_large, not '$DB'" >&2
     exit 1 ;;
esac
if [[ -z "$TABLES" ]]; then
  echo "estimate_check.sh: --tables needs at least one table" >&2
  exit 2
fi

OLD_URL="${OLD_URL:-postgres://dmux_dev:dmux_dev_only@127.0.0.1:${ARGS[0]:-5433}/$DB}"
NEW_URL="${NEW_URL:-postgres://dmux_dev:dmux_dev_only@127.0.0.1:${ARGS[1]:-5434}/$DB}"

HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck disable=SC1091  # guard.sh is linted separately; $HERE is runtime-relative
. "$HERE/../phase0/guard.sh"
for u in "$OLD_URL" "$NEW_URL"; do
  if ! dmux_guard_url "$u"; then
    echo "refusing: estimate_check.sh only runs against local mock databases" >&2
    exit 1
  fi
  if [[ "${u##*/}" != "$DB" ]]; then
    echo "refusing: estimate_check.sh reads database $DB, not '${u##*/}'" >&2
    exit 1
  fi
done

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

val() { psql -X -q -A -t -v ON_ERROR_STOP=1 "$1" -c "$2"; }

# One normalised SQL expression per column (PLAN.md 3.2), exactly as verify.sh
# builds it: floats and bytea from their bytes, money cast to numeric.
col_exprs() { # url, table
  psql -X -q -A -t -v ON_ERROR_STOP=1 -v tbl="$2" "$1" <<'SQL'
SELECT string_agg(
         CASE
           WHEN a.atttypid = 'float8'::regtype
             THEN 'encode(float8send(' || quote_ident(a.attname) || '), ''hex'')'
           WHEN a.atttypid = 'float4'::regtype
             THEN 'encode(float4send(' || quote_ident(a.attname) || '), ''hex'')'
           WHEN a.atttypid = 'bytea'::regtype
             THEN 'encode(' || quote_ident(a.attname) || ', ''hex'')'
           WHEN a.atttypid = 'money'::regtype
             THEN '(' || quote_ident(a.attname) || '::numeric)::text'
           ELSE quote_ident(a.attname) || '::text'
         END, ', ' ORDER BY a.attnum)
FROM pg_attribute a
WHERE a.attrelid = format('demo_bank.%I', :'tbl')::regclass
  AND a.attnum > 0
  AND NOT a.attisdropped;
SQL
}

# The key hash is over the primary-key columns, in catalog attribute order. A
# table without a key is bucketed by its row hash (the counted-set template).
key_exprs() { # url, table
  psql -X -q -A -t -v ON_ERROR_STOP=1 -v tbl="$2" "$1" <<'SQL'
SELECT coalesce(string_agg(quote_ident(a.attname) || '::text', ', ' ORDER BY a.attnum), '')
FROM pg_attribute a
WHERE a.attrelid = format('demo_bank.%I', :'tbl')::regclass
  AND a.attnum > 0
  AND NOT a.attisdropped
  AND EXISTS (
    SELECT 1
    FROM pg_index i
    WHERE i.indrelid = a.attrelid
      AND i.indisprimary
      AND a.attnum = ANY (i.indkey)
  );
SQL
}

# Per-table scan SQL: the fingerprint shape of phase0/fingerprint.sql, generated
# from the catalog. $TIMEOUT is the harness window, as in the perf harness.
scan_sql() { # file, exprs, keys
  local f="$1" exprs="$2" keys="$3"
  {
    echo 'BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;'
    echo 'SET LOCAL search_path        = pg_catalog;'
    echo "SET LOCAL timezone           = 'UTC';"
    echo "SET LOCAL datestyle          = 'ISO, YMD';"
    echo "SET LOCAL intervalstyle      = 'iso_8601';"
    echo "SET LOCAL bytea_output       = 'hex';"
    echo 'SET LOCAL extra_float_digits = 0;'
    echo "SET LOCAL xmloption          = 'document';"
    printf "SET LOCAL statement_timeout  = '%s';\n" "$TIMEOUT"
    echo "SET LOCAL lock_timeout       = '5s';"
    echo "SET LOCAL application_name   = 'dmux-estimate';"
    echo "SELECT ('x' || substr(kh, 1, 3))::bit(12)::int              AS bucket,"
    echo "       count(*)                                             AS n,"
    echo "       sum(('x' || substr(rh,  1, 8))::bit(32)::bigint)     AS s1,"
    echo "       sum(('x' || substr(rh,  9, 8))::bit(32)::bigint)     AS s2,"
    echo "       sum(('x' || substr(rh, 17, 8))::bit(32)::bigint)     AS s3,"
    echo "       sum(('x' || substr(rh, 25, 8))::bit(32)::bigint)     AS s4"
    echo 'FROM ('
    echo '  SELECT'
    printf "    md5(convert_to(ROW(%s)::text, 'UTF8')) AS kh,\n" "$keys"
    printf "    md5(convert_to(ROW(%s)::text, 'UTF8')) AS rh\n" "$exprs"
    printf '  FROM demo_bank.%s\n' "$TBL_QUOTED"
    echo ') q'
    echo 'GROUP BY 1'
    echo 'ORDER BY 1;'
    echo 'COMMIT;'
  } > "$f"
}

now_ns() { date +%s%N; }
elapsed() { awk -v a="$1" -v b="$2" 'BEGIN { printf "%.3f", (b - a) / 1000000000 }'; }

# Read-only session with the canonical block already set; the scan file is the
# fingerprint query plus its own BEGIN/COMMIT, so the timed part is one query.
scan() { # url, sqlfile, outfile - prints elapsed seconds
  local u="$1" sqlf="$2" out="$3" t0 t1
  t0="$(now_ns)"
  psql -X -q -A -t -v ON_ERROR_STOP=1 "$u" -f "$sqlf" > "$out.raw"
  t1="$(now_ns)"
  grep -E '^[0-9]+\|' "$out.raw" > "$out" || true
  elapsed "$t0" "$t1"
}

TIMEOUT="${ESTIMATE_TIMEOUT:-1800s}"

echo "Database: $DB; tables: $TABLES" >&2
for pair in "old|$OLD_URL" "new|$NEW_URL"; do
  u="${pair#*|}"
  printf '  %s: %s\n' "${pair%%|*}" "$(val "$u" "SELECT current_setting('server_version')
      || ', ' || pg_encoding_to_char(encoding) || ', collate ' || datcollate
      || ', ' || current_setting('shared_buffers') || ' shared_buffers'
    FROM pg_database WHERE datname = current_database()")" >&2
done

RESULTS="$TMP/results"
: > "$RESULTS"
TABLES_DONE=()
IFS=',' read -r -a TABLE_LIST <<< "$TABLES"
for tbl in "${TABLE_LIST[@]}"; do
  [[ -n "$tbl" ]] || continue
  case "$tbl" in
    *[!A-Za-z0-9_]*) echo "estimate_check.sh: bad table name '$tbl'" >&2; exit 2 ;;
  esac
  TABLES_DONE+=("$tbl")
  for pair in "old|$OLD_URL" "new|$NEW_URL"; do
    side="${pair%%|*}"; u="${pair#*|}"
    # SQL identifier quoting: the name is already limited to [A-Za-z0-9_].
    TBL_QUOTED="\"${tbl//\"/\"\"}\""
    exprs="$(col_exprs "$u" "$tbl")"
    keys="$(key_exprs "$u" "$tbl")"
    if [[ -z "$exprs" ]]; then
      echo "estimate_check.sh: table demo_bank.$tbl has no columns (missing?)" >&2
      exit 1
    fi
    if [[ -z "$keys" ]]; then keys="$exprs"; fi
    bytes="$(val "$u" "SELECT pg_total_relation_size(format('demo_bank.%I', '$tbl')::regclass)")"
    scan_sql "$TMP/scan_${side}_${tbl}.sql" "$exprs" "$keys"
    secs="$(scan "$u" "$TMP/scan_${side}_${tbl}.sql" "$TMP/fp_${side}_${tbl}")"
    rows="$(awk -F'|' '{ s += $2 } END { print s + 0 }' "$TMP/fp_${side}_${tbl}")"
    buckets="$(wc -l < "$TMP/fp_${side}_${tbl}" | tr -d ' ')"
    if [[ "$buckets" -eq 0 ]] || [[ "$buckets" -gt 4096 ]]; then
      echo "estimate_check.sh: $side $tbl: fingerprint returned $buckets buckets" >&2
      exit 1
    fi
    printf '%s|%s|%s|%s|%s\n' "$tbl" "$side" "$rows" "$bytes" "$secs" >> "$RESULTS"
    printf '  %-22s %-3s %12s rows %14s bytes %8ss\n' \
      "$tbl" "$side" "$rows" "$bytes" "$secs" >&2
  done
  # Same data on both sides means the same fingerprint: otherwise the timing
  # basis is not comparable and the run is not a valid measurement.
  if cmp -s "$TMP/fp_old_${tbl}" "$TMP/fp_new_${tbl}"; then
    echo "  $tbl: old and new fingerprints are byte-identical" >&2
  else
    echo "estimate_check.sh: $tbl: old and new fingerprints DIFFER" >&2
    exit 1
  fi
done

# --- JSON -------------------------------------------------------------------
json_side() { # side, table - one object
  awk -F'|' -v want_side="$1" -v want_tbl="$2" '
    $1 == want_tbl && $2 == want_side {
      printf "{\"rows\": %s, \"bytes\": %s, \"seconds\": %s}", $3, $4, $5
    }' "$RESULTS"
}

emit_json() {
  local tbl first=1
  printf '{\n'
  printf '  "version": 1,\n'
  printf '  "database": "%s",\n' "$DB"
  printf '  "generated_at": "%s",\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
  printf '  "tables": [\n'
  for tbl in "${TABLES_DONE[@]}"; do
    if [[ "$first" -eq 1 ]]; then first=0; else printf ',\n'; fi
    printf '    {"table": "%s", "old": %s, "new": %s}' \
      "$tbl" "$(json_side old "$tbl")" "$(json_side new "$tbl")"
  done
  printf '\n  ],\n'
  printf '  "totals": {'
  local side
  first=1
  for side in old new; do
    if [[ "$first" -eq 1 ]]; then first=0; else printf ', '; fi
    awk -F'|' -v s="$side" '
      $2 == s { r += $3; b += $4; t += $5; n++ }
      END { printf "\"%s\": {\"rows\": %d, \"bytes\": %d, \"seconds\": %.3f}", s, r, b, t }' \
      "$RESULTS"
  done
  printf '}\n'
  printf '}\n'
}

if [[ -n "$OUT" ]]; then
  emit_json > "$OUT"
  echo "JSON written to $OUT" >&2
else
  emit_json
fi
