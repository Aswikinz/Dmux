#!/usr/bin/env bash
# demo_bank: proof that the copy is complete and the data is reproducible.
#
# Usage:
#   testdata/demo_bank/verify.sh                       # small, old 9.6 (:5433) vs new 17 (:5434)
#   testdata/demo_bank/verify.sh 5435 5437             # small, old 9.4 vs new 18
#   testdata/demo_bank/verify.sh --db demo_bank_large  # the large size (see build.sh)
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
# Very large tables (more than CHUNK_ABOVE bytes on disk, default 256 MiB) use
# the same normalisation and the same order-independent idea, but md5 the sorted
# hash list in 256 chunks selected by the first two hex digits of each row hash,
# then md5 the sorted chunk digests. That keeps every intermediate string small:
# a single string_agg over 20,000,000 rows would be past Postgres's 1 GiB value
# limit. The chunk of a row is derived from the value, so both sides agree, and
# a changed, missing or extra row changes exactly one chunk's digest. Tables
# under the threshold (every table of the small size) keep the exact classic
# formula, so baselines stay comparable.
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
DB="demo_bank"
ARGS=()
NEXT=""
for arg in "$@"; do
  if [[ -n "$NEXT" ]]; then
    case "$NEXT" in
      baseline) BASELINE="$arg" ;;
      db)       DB="$arg" ;;
    esac
    NEXT=""
    continue
  fi
  case "$arg" in
    --baseline=*) BASELINE="${arg#--baseline=}" ;;
    --db=*)       DB="${arg#--db=}" ;;
    --baseline)   NEXT=baseline ;;
    --db)         NEXT=db ;;
    -h|--help)
      sed -n '3,/^set -euo pipefail/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'
      exit 0 ;;
    --*) echo "verify.sh: unknown option: $arg" >&2; exit 2 ;;
    *) ARGS+=("$arg") ;;
  esac
done
if [[ -n "$NEXT" ]]; then
  echo "usage: verify.sh [old port] [new port] [--db NAME] [--baseline FILE]" >&2
  exit 2
fi
case "$DB" in
  demo_bank|demo_bank_large) ;;
  *) echo "refusing: verify.sh only reads demo_bank or demo_bank_large, not '$DB'" >&2; exit 1 ;;
esac

# A single string_agg result must stay well under 1 GiB; above this size on disk
# the digest is computed in 256 value-chosen chunks instead.
CHUNK_ABOVE="${VERIFY_CHUNK_ABOVE:-268435456}"
TIMEOUT="${VERIFY_TIMEOUT:-1800s}"

OLD_URL="${OLD_URL:-postgres://dmux_dev:dmux_dev_only@127.0.0.1:${ARGS[0]:-5433}/$DB}"
NEW_URL="${NEW_URL:-postgres://dmux_dev:dmux_dev_only@127.0.0.1:${ARGS[1]:-5434}/$DB}"

HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck disable=SC1091  # guard.sh is linted separately; $HERE is runtime-relative
. "$HERE/../phase0/guard.sh"
for u in "$OLD_URL" "$NEW_URL"; do
  if ! dmux_guard_url "$u"; then
    echo "refusing: verify.sh only runs against local mock databases" >&2
    exit 1
  fi
  if [[ "${u##*/}" != "$DB" ]]; then
    echo "refusing: verify.sh reads database $DB, not '${u##*/}'" >&2
    exit 1
  fi
done

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

PASS=0
FAIL=0
CHUNKED=0

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

table_bytes() { # url, table - pg_total_relation_size, the estimate's own basis
  psql -X -q -A -t -v ON_ERROR_STOP=1 -v tbl="$2" "$1" <<'SQL'
SELECT pg_total_relation_size(format('demo_bank.%I', :'tbl')::regclass);
SQL
}

# count|digest for one table on one side.
digest() { # url, table, column expressions
  psql -X -q -A -t -v ON_ERROR_STOP=1 -v tbl="$2" -v exprs="$3" "$1" <<SQL
BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL search_path        = pg_catalog;
SET LOCAL timezone           = 'UTC';
SET LOCAL datestyle          = 'ISO, YMD';
SET LOCAL intervalstyle      = 'iso_8601';
SET LOCAL bytea_output       = 'hex';
SET LOCAL extra_float_digits = 0;
SET LOCAL xmloption          = 'document';
SET LOCAL statement_timeout  = '$TIMEOUT';
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

# The same digest for a table too big for one string_agg: the sorted hash list is
# split into 256 chunks by the first two hex digits of each row hash.
digest_chunked() { # url, table, column expressions
  psql -X -q -A -t -v ON_ERROR_STOP=1 -v tbl="$2" -v exprs="$3" "$1" <<SQL
BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL search_path        = pg_catalog;
SET LOCAL timezone           = 'UTC';
SET LOCAL datestyle          = 'ISO, YMD';
SET LOCAL intervalstyle      = 'iso_8601';
SET LOCAL bytea_output       = 'hex';
SET LOCAL extra_float_digits = 0;
SET LOCAL xmloption          = 'document';
SET LOCAL statement_timeout  = '$TIMEOUT';
SET LOCAL lock_timeout       = '5s';
SET LOCAL application_name   = 'dmux-verify';
SELECT coalesce(sum(n), 0) || '|'
       || coalesce(md5(string_agg(chunk_digest, '' ORDER BY chunk COLLATE "C")), 'empty')
FROM (
  SELECT substr(h, 1, 2) AS chunk,
         count(*) AS n,
         md5(string_agg(h, '' ORDER BY h COLLATE "C")) AS chunk_digest
  FROM (
    SELECT md5(convert_to(ROW(:exprs)::text, 'UTF8')) AS h
    FROM demo_bank.:"tbl"
  ) r
  GROUP BY 1
) g;
COMMIT;
SQL
}

ok()   { PASS=$((PASS + 1)); printf '  ok    %s\n' "$1"; }
bad()  { FAIL=$((FAIL + 1)); printf '  FAIL  %s\n' "$1"; }

server_line() {
  psql -X -q -A -t "$1" -c "SELECT current_setting('server_version')
    || ', ' || pg_encoding_to_char(encoding) || ', collate ' || datcollate
  FROM pg_database WHERE datname = current_database()"
}

echo "Database: $DB"
echo "Servers:"
printf '  old  %s\n' "$(server_line "$OLD_URL")"
printf '  new  %s\n' "$(server_line "$NEW_URL")"

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
  mode=classic
  suffix=""
  bytes="$(table_bytes "$OLD_URL" "$tbl")"
  if [[ "${bytes:-0}" -gt "$CHUNK_ABOVE" ]]; then
    mode=chunked
    suffix=" (chunked digest)"
    CHUNKED=$((CHUNKED + 1))
  fi
  if [[ "$mode" = chunked ]]; then
    d_old="$(digest_chunked "$OLD_URL" "$tbl" "$exprs_old")"
    d_new="$(digest_chunked "$NEW_URL" "$tbl" "$exprs_new")"
  else
    d_old="$(digest "$OLD_URL" "$tbl" "$exprs_old")"
    d_new="$(digest "$NEW_URL" "$tbl" "$exprs_new")"
  fi
  printf '%s|%s\n' "$tbl" "$d_old" >> "$TMP/old.digest"
  printf '%s|%s\n' "$tbl" "$d_new" >> "$TMP/new.digest"
  if [[ "$d_old" = "$d_new" ]]; then
    ok "$(printf '%-22s %s rows%s' "$tbl" "${d_old%%|*}" "$suffix")"
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

printf '\n%s passed, %s failed (chunked digests: %s)\n' "$PASS" "$FAIL" "$CHUNKED"
[[ "$FAIL" -eq 0 ]]
