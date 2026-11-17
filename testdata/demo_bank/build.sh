#!/usr/bin/env bash
# demo_bank: ONE command that builds the public test database on both mocks.
#
# Usage:
#   testdata/demo_bank/build.sh                 # old 9.6 (:5433) -> new 17 (:5434)
#   testdata/demo_bank/build.sh 5435 5437       # old 9.4 -> new 18, other local ports
#   OLD_URL=... NEW_URL=... testdata/demo_bank/build.sh
#
# What it does, in order:
#   1. drops and recreates database demo_bank on the OLD server (LATIN1, C sort
#      order on the default mocks) and loads demo_bank/schema.sql + data.sql;
#   2. pg_dump -Fc the old database;
#   3. drops and recreates database demo_bank on the NEW server (UTF-8) and
#      pg_restore's the dump into it, so both sides hold exactly the same data;
#   4. ANALYZE on both sides and prints a table/row summary.
#
# This is the "identical data" baseline. testdata/demo_bank/verify.sh proves the
# copy is complete and that two builds produce the same digests; T016 then plants
# the PLAN.md 8.2 differences in the new database only.
#
# WARNING: this script WRITES and DROPS a database. It touches only the database
# named demo_bank (plus the "postgres" maintenance database used to create it),
# and only on local mock servers. It NEVER touches the bank database. Dmux itself
# never writes; this is a test harness.
set -euo pipefail
# T010 hardening, same as prove.sh: libpq prefers hostaddr (PGHOSTADDR or a
# PGSERVICE file) over the URI host, so PG* environment could route these
# write-capable connections outside the URL guard. The strict URL supplies every
# connection parameter, so no PG* env is needed.
unset PGHOSTADDR PGSERVICE PGSERVICEFILE PGHOST PGPORT PGDATABASE PGUSER PGPASSWORD
export PGOPTIONS="${PGOPTIONS:-} -c client_min_messages=warning"
export PGCONNECT_TIMEOUT="${PGCONNECT_TIMEOUT:-10}"

HERE="$(cd "$(dirname "$0")" && pwd)"
# Fixed literal, never taken from input: this script may only ever drop and
# create the database with this exact name.
DB=demo_bank

# With no settings, use the mock databases from testdata/compose.yml. Two
# optional arguments pick other local ports, for the version matrix:
#   testdata/demo_bank/build.sh 5435 5437     # old = 9.4, new = 18
OLD_URL="${OLD_URL:-postgres://dmux_dev:dmux_dev_only@127.0.0.1:${1:-5433}/$DB}"
NEW_URL="${NEW_URL:-postgres://dmux_dev:dmux_dev_only@127.0.0.1:${2:-5434}/$DB}"
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

echo "Servers:"
printf '  old  %s\n' "$(server_line "$OLD_ADMIN")"
printf '  new  %s\n' "$(server_line "$NEW_ADMIN")"

echo "Building demo_bank on the old server:"
recreate "$OLD_ADMIN" old
psql -X -q -v ON_ERROR_STOP=1 "$OLD_URL" \
  -f "$HERE/schema.sql" -f "$HERE/data.sql" >/dev/null
echo "  schema.sql + data.sql loaded"

echo "Copying old -> new with pg_dump/pg_restore:"
pg_dump -Fc -f "$TMP/$DB.dump" "$OLD_URL"
echo "  dumped ($(du -h "$TMP/$DB.dump" | cut -f1))"
recreate "$NEW_ADMIN" new
pg_restore -d "$NEW_URL" --no-owner --exit-on-error "$TMP/$DB.dump"
echo "  restored"

q "$OLD_URL" "ANALYZE"
q "$NEW_URL" "ANALYZE"
echo "  analysed both sides"

echo "Summary:"
printf '  old  %s\n' "$(table_counts "$OLD_URL")"
printf '  new  %s\n' "$(table_counts "$NEW_URL")"
echo "Next: testdata/demo_bank/verify.sh proves the two sides are identical."
