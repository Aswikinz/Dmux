#!/usr/bin/env bash
# edge_bank: an owner playground with lots of variability (T077).
#
# Usage:
#   testdata/edge_bank/build.sh                # old 9.6 (:5433), new 17 (:5434)
#   testdata/edge_bank/build.sh 5435 5437      # other local ports (old, new)
#   EDGE_ADMIN_PASSWORD=... testdata/edge_bank/build.sh
#   EDGE_READER_PASSWORD=... testdata/edge_bank/build.sh
#
# What it does, in order:
#   1. drops and recreates ONLY the database edge_bank on the old server and
#      loads old.sql (schemas same, differs, structure, tricky, big);
#   2. pg_dump -Fc the old database, drops and recreates edge_bank on the new
#      server and pg_restores it, so both sides start identical;
#   3. plants the differences with new.sql on the NEW side only;
#   4. creates the read-only logins edge_reader (BYPASSRLS) and edge_reader_rls
#      (no BYPASSRLS) on both servers and grants them SELECT;
#   5. ANALYZE on both sides and prints a short summary.
#
# It is idempotent: run it again and edge_bank is rebuilt from scratch. It never
# touches bank, demo_bank, demo_bank_large or rls_bank, and only runs against
# local mock servers (the same URL guard as the other testdata scripts).
# WARNING: the old server is Postgres 9.6 (LATIN1, C sort order), the new one 17.
# The logins' password is a throwaway local one for these 127.0.0.1-only mocks.
set -euo pipefail
unset PGHOSTADDR PGSERVICE PGSERVICEFILE PGHOST PGPORT PGDATABASE PGUSER PGPASSWORD
export PGOPTIONS="${PGOPTIONS:-} -c client_min_messages=warning"
export PGCONNECT_TIMEOUT="${PGCONNECT_TIMEOUT:-10}"

HERE="$(cd "$(dirname "$0")" && pwd)"
DB=edge_bank
OLD_PORT="${1:-5433}"
NEW_PORT="${2:-5434}"
case "$OLD_PORT$NEW_PORT" in ''|*[!0-9]*) echo "build.sh: ports must be numbers" >&2; exit 2 ;; esac

# The mock superuser (set-up only). Its password is the one in compose.yml,
# unless EDGE_ADMIN_PASSWORD or the DMUX_TEST_OLD_URL variable says otherwise.
ADMIN_USER=dmux_dev
ADMIN_PW="${EDGE_ADMIN_PASSWORD:-}"
if [[ -z "$ADMIN_PW" && -n "${DMUX_TEST_OLD_URL:-}" ]]; then
  ADMIN_PW="$(printf '%s' "$DMUX_TEST_OLD_URL" | sed -n 's#^postgres://[^:]*:\([^@]*\)@.*#\1#p')"
fi
if [[ -z "$ADMIN_PW" ]]; then
  ADMIN_PW="$(sed -n 's/^ *POSTGRES_PASSWORD: *//p' "$HERE/../compose.yml" | head -n 1)"
fi
[[ -n "$ADMIN_PW" ]] || { echo "build.sh: no admin password (set EDGE_ADMIN_PASSWORD)" >&2; exit 2; }

READER_PW="${EDGE_READER_PASSWORD:-edge_reader_only}"
case "$READER_PW" in
  ''|*[!A-Za-z0-9._~%-]*) echo "build.sh: EDGE_READER_PASSWORD must be non-empty and URL-safe" >&2; exit 2 ;;
esac

ADMIN_OLD="postgres://$ADMIN_USER:$ADMIN_PW@127.0.0.1:$OLD_PORT/postgres"
ADMIN_NEW="postgres://$ADMIN_USER:$ADMIN_PW@127.0.0.1:$NEW_PORT/postgres"
OLD_URL="postgres://$ADMIN_USER:$ADMIN_PW@127.0.0.1:$OLD_PORT/$DB"
NEW_URL="postgres://$ADMIN_USER:$ADMIN_PW@127.0.0.1:$NEW_PORT/$DB"

# shellcheck disable=SC1091  # guard.sh is linted separately
. "$HERE/../phase0/guard.sh"
for u in "$ADMIN_OLD" "$ADMIN_NEW" "$OLD_URL" "$NEW_URL"; do
  dmux_guard_url "$u" || { echo "refusing: edge_bank only builds on local mock servers" >&2; exit 1; }
done

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
q() { psql -X -q -v ON_ERROR_STOP=1 "$1" -c "$2" >/dev/null; }
val() { psql -X -q -A -t -v ON_ERROR_STOP=1 "$1" -c "$2"; }

recreate() { # admin url, label
  q "$1" "SELECT pg_terminate_backend(pid) FROM pg_stat_activity
          WHERE datname = '$DB' AND pid <> pg_backend_pid()"
  q "$1" "DROP DATABASE IF EXISTS $DB"
  q "$1" "CREATE DATABASE $DB"
  echo "  $2: database $DB recreated"
}

ensure_logins() { # admin url
  local u="$1" bypass_edge="" bypass_rls=""
  if [[ "$(val "$u" "SHOW server_version_num")" -ge 90500 ]]; then
    bypass_edge="BYPASSRLS"; bypass_rls="NOBYPASSRLS"
  fi
  local r
  for r in edge_reader edge_reader_rls; do
    psql -X -q -v ON_ERROR_STOP=1 "$u" >/dev/null <<SQL
DO \$\$ BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '$r') THEN
    CREATE ROLE $r LOGIN;
  END IF;
END \$\$;
SQL
  done
  q "$u" "ALTER ROLE edge_reader LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE $bypass_edge PASSWORD '$READER_PW'"
  q "$u" "ALTER ROLE edge_reader_rls LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE $bypass_rls PASSWORD '$READER_PW'"
}

echo "Old server: $(val "$ADMIN_OLD" "SELECT split_part(version(), ' on ', 1)")"
echo "New server: $(val "$ADMIN_NEW" "SELECT split_part(version(), ' on ', 1)")"

echo "Building $DB on the old server:"
recreate "$ADMIN_OLD" old
psql -X -q -v ON_ERROR_STOP=1 "$OLD_URL" -f "$HERE/old.sql" >/dev/null
echo "  old.sql loaded"

echo "Copying old -> new with pg_dump/pg_restore:"
pg_dump -Fc -f "$TMP/$DB.dump" "$OLD_URL"
recreate "$ADMIN_NEW" new
pg_restore -d "$NEW_URL" --no-owner --exit-on-error -j 4 "$TMP/$DB.dump"
echo "  restored"

echo "Planting the differences on the new side only:"
psql -X -q -v ON_ERROR_STOP=1 "$NEW_URL" -f "$HERE/new.sql" >/dev/null
echo "  new.sql applied"

echo "Logins and grants:"
for a in "$ADMIN_OLD" "$ADMIN_NEW"; do ensure_logins "$a"; done
for u in "$OLD_URL" "$NEW_URL"; do psql -X -q -v ON_ERROR_STOP=1 "$u" -f "$HERE/grants.sql" >/dev/null; done
echo "  edge_reader (BYPASSRLS where the server has it) and edge_reader_rls (no BYPASSRLS)"

q "$OLD_URL" "ANALYZE"
q "$NEW_URL" "ANALYZE"
for pair in "old:$OLD_URL" "new:$NEW_URL"; do
  echo "  ${pair%%:*}: $(val "${pair#*:}" "SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
      WHERE c.relkind IN ('r', 'p') AND n.nspname IN ('same', 'differs', 'structure', 'tricky', 'big')") tables"
done
echo "Done. Connection details and the list of cases: testdata/edge_bank/README.md"
