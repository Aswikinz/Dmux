#!/usr/bin/env bash
# demo_bank: ONE command that builds the public test database on both mocks.
#
# Usage:
#   testdata/demo_bank/build.sh                     # small, demo_bank, planted
#   testdata/demo_bank/build.sh --no-plant          # small, identical data
#   testdata/demo_bank/build.sh --size large        # large, demo_bank_large, no plant
#   testdata/demo_bank/build.sh --size large --plant   # large, planted (ids as small)
#   testdata/demo_bank/build.sh --scale 4           # custom factor, demo_bank_large
#   testdata/demo_bank/build.sh 5435 5437           # other local ports (old 9.4, new 18)
#   OLD_URL=... NEW_URL=... testdata/demo_bank/build.sh
#
# Options:
#   --size small|large   small (default) is the answer-key data set; large is the
#                        PLAN.md 8.1 speed and estimate size (1,000,000 accounts,
#                        20,000,000 transactions, everything else scaled with it)
#   --scale N            custom factor for a size: small counts x N
#   --db NAME            database name; demo_bank (default for small) or
#                        demo_bank_large (default for large). Nothing else is
#                        accepted, and demo_bank is refused for large data.
#   --batch N            max rows per INSERT when loading in parts (default 1,000,000)
#   --plant / --no-plant plant the PLAN.md 8.2 differences in the new database
#                        (default: planted for small, identical for large, so the
#                        estimate check measures the same data on both sides)
#
# What it does, in order:
#   1. drops and recreates $DB on the OLD server (LATIN1, C sort order on the
#      default mocks) and loads demo_bank/schema.sql + data.sql, in batches of at
#      most --batch rows (one transaction per INSERT) for anything above the
#      small size;
#   2. pg_dump -Fc the old database;
#   3. drops and recreates $DB on the NEW server (UTF-8) and pg_restore -j 4's the
#      dump into it, so both sides hold exactly the same data;
#   4. ANALYZE on both sides and prints a table/row summary;
#   5. plants the PLAN.md 8.2 differences with demo_bank/plant.sql in the NEW
#      database only (--no-plant skips this and keeps the two sides identical).
#
# With planting on this is the answer-key build: testdata/demo_bank/check_key.sh
# then proves every planted difference against testdata/expected_diffs.json. With
# --no-plant this is the identical-data baseline: verify.sh proves the copy is
# complete, and estimate_check.sh records the size and timing of the large one.
#
# WARNING: this script WRITES and DROPS a database. It touches only a database
# named demo_bank or demo_bank_large (plus the "postgres" maintenance database
# used to create it), and only on local mock servers. It NEVER touches the bank
# databases. Dmux itself never writes; this is a test harness.
set -euo pipefail
# T010 hardening, same as prove.sh: libpq prefers hostaddr (PGHOSTADDR or a
# PGSERVICE file) over the URI host, so PG* environment could route these
# write-capable connections outside the URL guard. The strict URL supplies every
# connection parameter, so no PG* env is needed.
unset PGHOSTADDR PGSERVICE PGSERVICEFILE PGHOST PGPORT PGDATABASE PGUSER PGPASSWORD
export PGOPTIONS="${PGOPTIONS:-} -c client_min_messages=warning"
export PGCONNECT_TIMEOUT="${PGCONNECT_TIMEOUT:-10}"

HERE="$(cd "$(dirname "$0")" && pwd)"

# Defaults. $DB is one of two fixed literals and is validated below before it is
# ever pasted into SQL, so no other database can be dropped or created.
SIZE=small
SCALE=""
DB=""
BATCH="${DEMO_BATCH:-1000000}"
PLANT_ARG="auto"
PORTS=()
PENDING=""
for arg in "$@"; do
  case "$PENDING" in
    size)  SIZE="$arg";  PENDING=""; continue ;;
    db)    DB="$arg";    PENDING=""; continue ;;
    scale) SCALE="$arg"; PENDING=""; continue ;;
    batch) BATCH="$arg"; PENDING=""; continue ;;
  esac
  case "$arg" in
    --plant)      PLANT_ARG=1 ;;
    --no-plant)   PLANT_ARG=0 ;;
    --size=*)     SIZE="${arg#--size=}" ;;
    --db=*)       DB="${arg#--db=}" ;;
    --scale=*)    SCALE="${arg#--scale=}" ;;
    --batch=*)    BATCH="${arg#--batch=}" ;;
    --size)       PENDING="size" ;;
    --db)         PENDING="db" ;;
    --scale)      PENDING="scale" ;;
    --batch)      PENDING="batch" ;;
    -h|--help)
      sed -n '3,/^set -euo pipefail/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'
      exit 0 ;;
    --*) echo "build.sh: unknown option: $arg" >&2; exit 2 ;;
    *) PORTS+=("$arg") ;;
  esac
done
if [[ -n "$PENDING" ]]; then
  echo "build.sh: --$PENDING needs a value" >&2
  exit 2
fi
if [[ "${#PORTS[@]}" -gt 2 ]]; then
  echo "build.sh: at most two ports (old, new)" >&2
  exit 2
fi

case "$SIZE" in
  small|large) ;;
  *) echo "build.sh: --size must be small or large, not '$SIZE'" >&2; exit 2 ;;
esac
if [[ -n "$SCALE" ]]; then
  case "$SCALE" in
    ''|*[!0-9]*) echo "build.sh: --scale must be a positive integer, not '$SCALE'" >&2; exit 2 ;;
  esac
  if [[ "$SCALE" -lt 1 ]]; then
    echo "build.sh: --scale must be a positive integer" >&2; exit 2
  fi
fi
case "$BATCH" in
  ''|*[!0-9]*) echo "build.sh: --batch must be a positive integer, not '$BATCH'" >&2; exit 2 ;;
esac
if [[ "$BATCH" -lt 1 ]]; then
  echo "build.sh: --batch must be a positive integer" >&2; exit 2
fi

# The large size always lives in demo_bank_large: engine tests read demo_bank on
# :5433/:5434 while other tasks build (decision 19), so demo_bank is never
# rebuilt with a million accounts.
if [[ -z "$DB" ]]; then
  if [[ "$SIZE" = large ]] || [[ -n "$SCALE" ]]; then DB=demo_bank_large; else DB=demo_bank; fi
fi
case "$DB" in
  demo_bank|demo_bank_large) ;;
  *) echo "refusing: build.sh only writes to demo_bank or demo_bank_large, not '$DB'" >&2; exit 1 ;;
esac
if [[ "$DB" = demo_bank ]] && { [[ "$SIZE" = large ]] || [[ -n "$SCALE" ]]; }; then
  echo "refusing: the large size lives in demo_bank_large; demo_bank is shared with running tests" >&2
  exit 1
fi

# Planting: the answer-key build by default for small, off for large and for a
# custom scale (both sides must hold the same data for the estimate check).
PLANT="$PLANT_ARG"
if [[ "$PLANT" = auto ]]; then
  if [[ "$SIZE" = small ]] && [[ -z "$SCALE" ]]; then PLANT=1; else PLANT=0; fi
fi

OLD_URL="${OLD_URL:-postgres://dmux_dev:dmux_dev_only@127.0.0.1:${PORTS[0]:-5433}/$DB}"
NEW_URL="${NEW_URL:-postgres://dmux_dev:dmux_dev_only@127.0.0.1:${PORTS[1]:-5434}/$DB}"
# Maintenance connections: same server and credentials, the "postgres" database.
# The guard forbids query strings, so the database name is the last path segment.
OLD_ADMIN="${OLD_URL%/*}/postgres"
NEW_ADMIN="${NEW_URL%/*}/postgres"

# Strict local-only URL guard, shared with the phase0 harnesses (threat model in
# testdata/phase0/guard.sh). All four URLs are checked before any connection.
# shellcheck disable=SC1091  # guard.sh is linted separately; $HERE is runtime-relative
. "$HERE/../phase0/guard.sh"
for u in "$OLD_URL" "$NEW_URL" "$OLD_ADMIN" "$NEW_ADMIN"; do
  if ! dmux_guard_url "$u"; then
    echo "refusing: build.sh only runs against local mock databases" >&2
    exit 1
  fi
done
# The URL guard allows any local database, but this script writes: it loads
# schema.sql + data.sql into OLD_URL, restores and plants into NEW_URL. The
# database name is fixed to the two literals validated above, so a URL ending in
# /bank (or anything else) is refused before the first connection. The admin URLs
# end in /postgres on purpose: they only run DROP/CREATE DATABASE for $DB.
for u in "$OLD_URL" "$NEW_URL"; do
  db="${u##*/}"
  if [[ "$db" != "$DB" ]]; then
    echo "refusing: build.sh only writes to database $DB, not '$db'" >&2
    exit 1
  fi
done

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

q() { psql -X -q -v ON_ERROR_STOP=1 "$1" -c "$2" >/dev/null; }

server_line() { # url - version, encoding and collation of the current database
  psql -X -q -A -t -v ON_ERROR_STOP=1 "$1" -c "SELECT current_setting('server_version')
      || ', ' || pg_encoding_to_char(encoding) || ', collate ' || datcollate
    FROM pg_database WHERE datname = current_database()"
}

recreate() { # admin url, label
  local admin="$1" label="$2"
  # Nothing may hold the database open while it is dropped.
  psql -X -q -v ON_ERROR_STOP=1 "$admin" -c "SELECT pg_terminate_backend(pid)
      FROM pg_stat_activity
      WHERE datname = '$DB' AND pid <> pg_backend_pid()" >/dev/null
  q "$admin" "DROP DATABASE IF EXISTS $DB"
  q "$admin" "CREATE DATABASE $DB"
  echo "  $label: database $DB recreated"
}

# Ordinary tables of the schema, one count each, as a single UNION ALL query.
table_counts() { # url - "tables<TAB>rows" for the demo_bank schema
  local u="$1" list
  list="$(psql -X -q -A -t -v ON_ERROR_STOP=1 "$u" -c "
    SELECT string_agg(format('SELECT count(*) AS n FROM demo_bank.%I', c.relname),
                      ' UNION ALL ' ORDER BY c.relname)
    FROM pg_class c
    JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = 'demo_bank' AND c.relkind = 'r'")"
  psql -X -q -A -t -v ON_ERROR_STOP=1 "$u" -c \
    "SELECT count(*) || ' tables, ' || coalesce(sum(n), 0) || ' rows' FROM ($list) AS s"
}

# Generator arguments shared by every loader call. -v size= is always passed;
# -v scale= only when a custom factor was asked for.
GEN_ARGS=(-v "size=$SIZE")
if [[ -n "$SCALE" ]]; then GEN_ARGS+=(-v "scale=$SCALE"); fi

# One psql process per part (or per batch), each INSERT its own transaction.
# synchronous_commit=off only shortens the WAL flush of a throwaway mock load; it
# changes nothing about the data or the durability rules of the engine.
psql_load() { # url, psql args...
  local u="$1"; shift
  PGOPTIONS="$PGOPTIONS -c synchronous_commit=off" \
    psql -X -q -v ON_ERROR_STOP=1 "$u" "$@" >/dev/null
}

load_parts() { # url - schema.sql, then every part of data.sql in plan order
  local u="$1" plan _ part rows batched lo hi got
  psql_load "$u" -f "$HERE/schema.sql"
  echo "  schema.sql loaded"
  # The plan (order, part name, rows, windowable) comes from data.sql itself, so
  # the counts and the foreign-key order live in one place.
  plan="$(psql -X -q -A -t -v ON_ERROR_STOP=1 "$u" "${GEN_ARGS[@]}" \
            -v list_parts=1 -f "$HERE/data.sql")"
  if [[ -z "$plan" ]]; then
    echo "build.sh: data.sql returned no load plan" >&2
    exit 1
  fi
  while IFS='|' read -r _ part rows batched; do
    [[ -n "$part" ]] || continue
    case "$part" in
      *[!a-z_]*) echo "build.sh: bad part name in the load plan: '$part'" >&2; exit 1 ;;
    esac
    case "$rows" in
      ''|*[!0-9]*) echo "build.sh: bad load plan line for $part: '$rows'" >&2; exit 1 ;;
    esac
    case "$batched" in
      0|1) ;;
      *) echo "build.sh: bad 'batched' flag for $part: '$batched'" >&2; exit 1 ;;
    esac
    if [[ "$batched" = 0 ]] || [[ "$rows" -le "$BATCH" ]]; then
      psql_load "$u" "${GEN_ARGS[@]}" -v "run=$part" -f "$HERE/data.sql"
      printf '  %-18s %12s rows\n' "$part" "$rows"
    else
      lo=1
      while [[ "$lo" -le "$rows" ]]; do
        hi=$((lo + BATCH - 1))
        if [[ "$hi" -gt "$rows" ]]; then hi="$rows"; fi
        psql_load "$u" "${GEN_ARGS[@]}" -v "run=$part" -v "lo=$lo" -v "hi=$hi" \
          -f "$HERE/data.sql"
        printf '  %-18s %12s rows %s..%s\n' "$part" "$rows" "$lo" "$hi"
        lo=$((hi + 1))
      done
    fi
    # Every data part fills the table named after it, so the plan's own row
    # count is the expected count: a window or a guard that silently loaded
    # nothing can never pass unnoticed.
    if [[ "$rows" -gt 0 ]]; then
      got="$(psql -X -q -A -t -v ON_ERROR_STOP=1 "$u" \
               -c "SELECT count(*) FROM demo_bank.\"$part\"")"
      if [[ "$got" != "$rows" ]]; then
        echo "build.sh: part $part holds $got rows, the load plan wanted $rows" >&2
        exit 1
      fi
    fi
  done <<< "$plan"
}

echo "Size:     $SIZE${SCALE:+ (scale $SCALE)} in database $DB; load batch $BATCH rows; plant $PLANT"
echo "Servers:"
printf '  old  %s\n' "$(server_line "$OLD_ADMIN")"
printf '  new  %s\n' "$(server_line "$NEW_ADMIN")"

echo "Building $DB on the old server:"
recreate "$OLD_ADMIN" old
if [[ "$SIZE" = small ]] && [[ -z "$SCALE" ]]; then
  # The small size loads in one psql run, exactly as it always has.
  psql_load "$OLD_URL" -f "$HERE/schema.sql" -f "$HERE/data.sql"
  echo "  schema.sql + data.sql loaded"
else
  load_parts "$OLD_URL"
fi

echo "Copying old -> new with pg_dump/pg_restore:"
pg_dump -Fc -f "$TMP/$DB.dump" "$OLD_URL"
echo "  dumped ($(du -h "$TMP/$DB.dump" | cut -f1))"
recreate "$NEW_ADMIN" new
pg_restore -d "$NEW_URL" --no-owner --exit-on-error -j 4 "$TMP/$DB.dump"
echo "  restored (4 jobs)"

q "$OLD_URL" "ANALYZE"
q "$NEW_URL" "ANALYZE"
echo "  analysed both sides"

echo "Summary (before planting):"
printf '  old  %s\n' "$(table_counts "$OLD_URL")"
printf '  new  %s\n' "$(table_counts "$NEW_URL")"

if [[ "$PLANT" -eq 1 ]]; then
  echo "Planting the PLAN.md 8.2 differences in the new database only:"
  if [[ ! -f "$HERE/plant.sql" ]]; then
    echo "  plant.sql is missing" >&2
    exit 1
  fi
  # NEW only. The old database stays the untouched "before" picture.
  psql -X -q -v ON_ERROR_STOP=1 "$NEW_URL" -f "$HERE/plant.sql" >/dev/null
  echo "  plant.sql applied (old side untouched)"
  echo "Next: testdata/demo_bank/check_key.sh grades expected_diffs.json against these databases."
else
  echo "No planting (--no-plant): both sides hold identical data."
  echo "Next: testdata/demo_bank/verify.sh proves the two sides are identical."
fi
