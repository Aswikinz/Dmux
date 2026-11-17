#!/usr/bin/env bash
# demo_bank: proof that the copy is complete and the data is reproducible.
#
# Usage:
#   testdata/demo_bank/verify.sh                       # old 9.6 (:5433) vs new 17 (:5434)
#   testdata/demo_bank/verify.sh 5435 5437             # old 9.4 vs new 18
#   testdata/demo_bank/verify.sh --baseline /tmp/demo_bank.digest
#
# For every table in schema demo_bank it computes one digest:
#
#   count(*) | md5(string_agg(row_hash, '' ORDER BY row_hash COLLATE "C"))
#
# where row_hash = md5(convert_to(ROW(<normalised columns>)::text, 'UTF8')).
# Ordering is by the hash with an explicit C collation, so neither the physical
# row order nor the two servers' text sort orders can change the digest
# (PLAN.md section 3, decision 2). Columns are normalised the way PLAN.md 3.2
# requires: floats and bytea are hashed from their bytes (decision 4), money is
# cast to numeric, everything else uses its printed form under the canonical
# settings block from the T003 review.
#
# Checks:
#   (a) every table exists on both sides with the same row count and digest,
#       i.e. the pg_dump/pg_restore copy lost nothing;
#   (b) with --baseline FILE the digests equal an earlier run's, i.e. build.sh
#       produces the same data on every run. The file is written on first use.
#
# Exit status: 0 only when every check passes.
set -euo pipefail
# Same hardening as build.sh / prove.sh: no PG* environment may override the URLs.
unset PGHOSTADDR PGSERVICE PGSERVICEFILE PGHOST PGPORT PGDATABASE PGUSER PGPASSWORD
export PGOPTIONS="${PGOPTIONS:-} -c client_min_messages=warning -c default_transaction_read_only=on"
export PGCONNECT_TIMEOUT="${PGCONNECT_TIMEOUT:-10}"

BASELINE=""
ARGS=()
for arg in "$@"; do
  case "$arg" in
    --baseline=*) BASELINE="${arg#--baseline=}" ;;
    --baseline)
      # The value follows in the next argument; handled below.
      BASELINE="__next__" ;;
    *) if [[ "$BASELINE" = "__next__" ]]; then BASELINE="$arg"; else ARGS+=("$arg"); fi ;;
  esac
done
if [[ "$BASELINE" = "__next__" ]]; then
  echo "usage: verify.sh [old port] [new port] [--baseline FILE]" >&2
  exit 2
fi

OLD_URL="${OLD_URL:-postgres://dmux_dev:dmux_dev_only@127.0.0.1:${ARGS[0]:-5433}/demo_bank}"
NEW_URL="${NEW_URL:-postgres://dmux_dev:dmux_dev_only@127.0.0.1:${ARGS[1]:-5434}/demo_bank}"

HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck disable=SC1091  # guard.sh is linted separately; $HERE is runtime-relative
. "$HERE/../phase0/guard.sh"
for u in "$OLD_URL" "$NEW_URL"; do
  if ! dmux_guard_url "$u"; then
    echo "refusing: verify.sh only runs against local mock databases" >&2
    exit 1
  fi
done

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

PASS=0
FAIL=0

# Table names, from the catalog, never by text sort order of the data.
tables_of() {
  psql -X -q -A -t -v ON_ERROR_STOP=1 "$1" -c "
    SELECT c.relname
    FROM pg_class c
    JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = 'demo_bank' AND c.relkind = 'r'
    ORDER BY c.relname::text COLLATE \"C\""
}

# One SQL expression per column, already normalised (PLAN.md 3.2).
# The table name is passed as a psql variable, so it is never pasted into SQL by
# the shell.
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

# count|digest for one table on one side.
digest() { # url, table, column expressions
  psql -X -q -A -t -v ON_ERROR_STOP=1 -v tbl="$2" -v exprs="$3" "$1" <<'SQL'
BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL search_path        = pg_catalog;
SET LOCAL timezone           = 'UTC';
SET LOCAL datestyle          = 'ISO, YMD';
SET LOCAL intervalstyle      = 'iso_8601';
SET LOCAL bytea_output       = 'hex';
SET LOCAL extra_float_digits = 0;
SET LOCAL xmloption          = 'document';
SET LOCAL statement_timeout  = '300s';
SET LOCAL lock_timeout       = '5s';
SET LOCAL application_name   = 'dmux-verify';
SELECT count(*) || '|' || coalesce(md5(string_agg(h, '' ORDER BY h COLLATE "C")), 'empty')
FROM (
  SELECT md5(convert_to(ROW(:exprs)::text, 'UTF8')) AS h
  FROM demo_bank.:"tbl"
) r;
COMMIT;
SQL
}

ok()   { PASS=$((PASS + 1)); printf '  ok    %s\n' "$1"; }
bad()  { FAIL=$((FAIL + 1)); printf '  FAIL  %s\n' "$1"; }

echo "Servers:"
printf '  old  %s\n' "$(psql -X -q -A -t "$OLD_URL" -c "SELECT current_setting('server_version')
    || ', ' || pg_encoding_to_char(encoding) || ', collate ' || datcollate
  FROM pg_database WHERE datname = current_database()")"
printf '  new  %s\n' "$(psql -X -q -A -t "$NEW_URL" -c "SELECT current_setting('server_version')
    || ', ' || pg_encoding_to_char(encoding) || ', collate ' || datcollate
  FROM pg_database WHERE datname = current_database()")"

tables_of "$OLD_URL" > "$TMP/old.tables"
tables_of "$NEW_URL" > "$TMP/new.tables"
if diff -u "$TMP/old.tables" "$TMP/new.tables" > "$TMP/tables.diff"; then
  ok "both sides have the same $(wc -l < "$TMP/old.tables" | tr -d ' ') tables"
else
  bad "the two sides do not have the same table set"
  sed 's/^/        /' "$TMP/tables.diff"
fi

: > "$TMP/old.digest"
: > "$TMP/new.digest"
echo "Per-table digests:"
while IFS= read -r tbl; do
  [[ -n "$tbl" ]] || continue
  exprs_old="$(col_exprs "$OLD_URL" "$tbl")"
  exprs_new="$(col_exprs "$NEW_URL" "$tbl")"
  if [[ -z "$exprs_old" ]] || [[ -z "$exprs_new" ]]; then
    bad "$tbl: no columns found (table missing on one side?)"
    continue
  fi
  d_old="$(digest "$OLD_URL" "$tbl" "$exprs_old")"
  d_new="$(digest "$NEW_URL" "$tbl" "$exprs_new")"
  printf '%s|%s\n' "$tbl" "$d_old" >> "$TMP/old.digest"
  printf '%s|%s\n' "$tbl" "$d_new" >> "$TMP/new.digest"
  if [[ "$d_old" = "$d_new" ]]; then
    ok "$(printf '%-22s %s rows' "$tbl" "${d_old%%|*}")"
  else
    bad "$(printf '%-22s old=%s new=%s' "$tbl" "$d_old" "$d_new")"
  fi
done < "$TMP/old.tables"

echo "Copy check (old == new):"
if diff -u "$TMP/old.digest" "$TMP/new.digest" > "$TMP/copy.diff"; then
  ok "every table digest matches between the old and the new database"
else
  bad "the two sides differ"
  sed 's/^/        /' "$TMP/copy.diff"
fi

if [[ -n "$BASELINE" ]]; then
  echo "Reproducibility check (baseline: $BASELINE):"
  if [[ -f "$BASELINE" ]]; then
    if diff -u "$BASELINE" "$TMP/old.digest" > "$TMP/base.diff"; then
      ok "identical to the baseline: two builds produced the same data"
    else
      bad "the data changed since the baseline was written"
      sed 's/^/        /' "$TMP/base.diff"
    fi
  else
    cp "$TMP/old.digest" "$BASELINE"
    ok "no baseline yet: saved this run's digests to $BASELINE"
  fi
fi

printf '\n%s passed, %s failed\n' "$PASS" "$FAIL"
[[ "$FAIL" -eq 0 ]]
