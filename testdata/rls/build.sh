#!/usr/bin/env bash
# rls: build the RLS fixture database (rls_bank) on both mock servers (T052).
#
# Usage:
#   testdata/rls/build.sh                 # old 9.6 (:5433) vs new 17 (:5434)
#   testdata/rls/build.sh 5435 5437       # other local ports (old, new)
#   DMUX_RLS_PASSWORD=... testdata/rls/build.sh
#
# This is the fixture for the H1 finding (SECURITY-AUDIT.md): row-level security
# can silently hide rows from both scans and give a false PASS. It provides a
# table the engine test can point at to prove Dmux fails closed instead.
#
# Table: public.branch_notes (id integer PK, branch_id integer, note text), 20
# rows. RLS and FORCE ROW LEVEL SECURITY are enabled, with one SELECT policy:
#
#     CREATE POLICY branch_notes_visible ... USING (branch_id = 1)
#
# so a role that is neither the owner nor BYPASSRLS sees only branch_id = 1
# (ids 1..10). The other half (branch_id = 2, ids 11..20) is hidden. The NEW
# side plants one difference in a HIDDEN row (id 20's note), so a reader that
# trusts the policy sees identical halves on both sides and would falsely PASS,
# while the hidden halves differ.
#
# Roles:
#   rls_owner         NOLOGIN, non-superuser, owns public.branch_notes. Being a
#                     different role from dmux_dev means the reader is a
#                     non-owner, so RLS applies.
#   dmux_rls_reader   LOGIN, non-superuser, NOBYPASSRLS, SELECT only. The role
#                     the engine test connects as. Its password comes from
#                     DMUX_RLS_PASSWORD (default below).
#   dmux_dev          the mock superuser (compose.yml). Superusers bypass RLS,
#                     so this role is used here only for DDL and to prove the
#                     hidden rows differ; the engine test must NOT use it.
#
# RLS requires PostgreSQL 9.5+, so a server older than 9.5 (e.g. old94 on
# :5435) is skipped with a message and no database is built on it.
#
# Idempotent: drops and recreates rls_bank each run; roles are created if
# absent and the reader's password is refreshed. It only ever touches the local
# mocks, and only the databases rls_bank and postgres.
set -euo pipefail
# Same hardening as build.sh / check_key.sh: no PG* environment may override the
# URLs below, and the URLs carry every connection parameter the write-capable
# connection needs.
unset PGHOSTADDR PGSERVICE PGSERVICEFILE PGHOST PGPORT PGDATABASE PGUSER PGPASSWORD
export PGOPTIONS="${PGOPTIONS:-} -c client_min_messages=warning"
export PGCONNECT_TIMEOUT="${PGCONNECT_TIMEOUT:-10}"

HERE="$(cd "$(dirname "$0")" && pwd)"

OLD_PORT="${1:-5433}"
NEW_PORT="${2:-5434}"

# The reader's password. Throwaway, local-only (the mocks are bound to
# 127.0.0.1). The default lives only here; set DMUX_RLS_PASSWORD to override.
PW="${DMUX_RLS_PASSWORD:-dmux_rls_only}"
case "$PW" in
  ''|*[!A-Za-z0-9._~%-]*)
    echo "build.sh: DMUX_RLS_PASSWORD must be non-empty and URL-safe ([A-Za-z0-9._~%-])" >&2
    exit 2 ;;
esac

# The mock superuser's throwaway password, from testdata/compose.yml.
ADMIN_PW="dmux_dev_only"

ADMIN_OLD="postgres://dmux_dev:$ADMIN_PW@127.0.0.1:$OLD_PORT/postgres"
ADMIN_NEW="postgres://dmux_dev:$ADMIN_PW@127.0.0.1:$NEW_PORT/postgres"
OLD_DB="postgres://dmux_dev:$ADMIN_PW@127.0.0.1:$OLD_PORT/rls_bank"
NEW_DB="postgres://dmux_dev:$ADMIN_PW@127.0.0.1:$NEW_PORT/rls_bank"
READER_OLD="postgres://dmux_rls_reader:$PW@127.0.0.1:$OLD_PORT/rls_bank"
READER_NEW="postgres://dmux_rls_reader:$PW@127.0.0.1:$NEW_PORT/rls_bank"

# shellcheck disable=SC1091  # guard.sh is linted separately; $HERE is runtime-relative
. "$HERE/../phase0/guard.sh"
for u in "$ADMIN_OLD" "$ADMIN_NEW" "$OLD_DB" "$NEW_DB" "$READER_OLD" "$READER_NEW"; do
  if ! dmux_guard_url "$u"; then
    echo "refusing: build.sh only runs against local mock databases" >&2
    exit 1
  fi
done
for u in "$OLD_DB" "$NEW_DB" "$READER_OLD" "$READER_NEW"; do
  if [[ "${u##*/}" != "rls_bank" ]]; then
    echo "refusing: build.sh only writes to database rls_bank, not '${u##*/}'" >&2
    exit 1
  fi
done
for u in "$ADMIN_OLD" "$ADMIN_NEW"; do
  if [[ "${u##*/}" != "postgres" ]]; then
    echo "refusing: admin URLs must end in /postgres, not '${u##*/}'" >&2
    exit 1
  fi
done

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

q() { psql -X -q -v ON_ERROR_STOP=1 "$1" -c "$2" >/dev/null; }

server_version() { # admin url -> server_version_num
  psql -X -q -A -t -v ON_ERROR_STOP=1 "$1" -c "SHOW server_version_num"
}

recreate() { # admin url, label
  local admin="$1" label="$2"
  # Nothing may hold the database open while it is dropped.
  psql -X -q -v ON_ERROR_STOP=1 "$admin" -c "SELECT pg_terminate_backend(pid)
      FROM pg_stat_activity WHERE datname = 'rls_bank' AND pid <> pg_backend_pid()" >/dev/null
  q "$admin" "DROP DATABASE IF EXISTS rls_bank"
  q "$admin" "CREATE DATABASE rls_bank"
  echo "  $label: database rls_bank recreated"
}

roles_sql() { # db url - create/refresh the two roles (cluster-wide, idempotent)
  psql -X -q -v ON_ERROR_STOP=1 -v pw="$PW" "$1" <<'SQL'
DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = 'rls_owner') THEN
    CREATE ROLE rls_owner NOLOGIN;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = 'dmux_rls_reader') THEN
    CREATE ROLE dmux_rls_reader LOGIN;
  END IF;
END
$$;
ALTER ROLE rls_owner NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
ALTER ROLE dmux_rls_reader LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD :'pw';
SQL
}

load_side() { # db url, plant (0|1) - only called on servers 9.5 and up
  local db="$1" plant="$2"
  psql -X -q -v ON_ERROR_STOP=1 "$db" <<'SQL'
CREATE TABLE public.branch_notes (
    id        integer PRIMARY KEY,
    branch_id integer NOT NULL,
    note      text NOT NULL
);
INSERT INTO public.branch_notes (id, branch_id, note)
SELECT g,
       CASE WHEN g <= 10 THEN 1 ELSE 2 END,
       'branch ' || CASE WHEN g <= 10 THEN 1 ELSE 2 END || ' note ' || g
FROM generate_series(1, 20) AS g;
ALTER TABLE public.branch_notes OWNER TO rls_owner;
GRANT USAGE ON SCHEMA public TO dmux_rls_reader;
GRANT SELECT ON public.branch_notes TO dmux_rls_reader;
SQL
  psql -X -q -v ON_ERROR_STOP=1 "$db" <<'SQL'
ALTER TABLE public.branch_notes ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.branch_notes FORCE ROW LEVEL SECURITY;
CREATE POLICY branch_notes_visible ON public.branch_notes
  FOR SELECT USING (branch_id = 1);
SQL
  echo "    RLS + FORCE RLS + policy enabled (branch_id = 1 visible)"
  if [[ "$plant" = 1 ]]; then
    q "$db" "UPDATE public.branch_notes SET note = note || ' (changed in migration)' WHERE id = 20"
    echo "    planted: id 20 (hidden, branch 2) note changed on the new side only"
  fi
}

build_side() { # admin url, db url, label, server_version_num, plant (0|1)
  local admin="$1" db="$2" label="$3" ver="$4" plant="$5"
  if [[ "$ver" -lt 90500 ]]; then
    echo "  $label: SKIP - server $ver is below 9.5, row-level security is not available"
    return 0
  fi
  recreate "$admin" "$label"
  roles_sql "$db"
  load_side "$db" "$plant"
}

echo "RLS fixture: rls_bank"
echo "Reader:  dmux_rls_reader (password from DMUX_RLS_PASSWORD, default in this script)"
echo "Servers:"
old_ver="$(server_version "$ADMIN_OLD")"
new_ver="$(server_version "$ADMIN_NEW")"
printf '  old  %s (%s)\n' "$old_ver" "$ADMIN_OLD"
printf '  new  %s (%s)\n' "$new_ver" "$ADMIN_NEW"

echo "Building old side:"
build_side "$ADMIN_OLD" "$OLD_DB" old "$old_ver" 0

echo "Building new side:"
build_side "$ADMIN_NEW" "$NEW_DB" new "$new_ver" 1

if [[ "$old_ver" -lt 90500 || "$new_ver" -lt 90500 ]]; then
  echo
  echo "RLS needs 9.5+ on both sides; no self-check run (a side was skipped)."
  exit 0
fi

# ---------------------------------------------------------------------------
# Self-check: prove the fixture does what the engine test needs, read-only.
# A comparison never passes on nothing: an empty value counts as a failure.
# ---------------------------------------------------------------------------
PASS=0
FAIL=0
ok()  { PASS=$((PASS + 1)); printf '  ok    %s\n' "$1"; }
bad() { FAIL=$((FAIL + 1)); printf '  FAIL  %s\n' "$1"; }

reader_count() { psql -X -q -A -t -v ON_ERROR_STOP=1 "$1" -c "SELECT count(*) FROM public.branch_notes"; }
admin_count()  { psql -X -q -A -t -v ON_ERROR_STOP=1 "$1" -c "SELECT count(*) FROM public.branch_notes"; }
hidden_digest(){ psql -X -q -A -t -v ON_ERROR_STOP=1 "$1" -c "SELECT md5(string_agg(note, '' ORDER BY id)) FROM public.branch_notes WHERE branch_id = 2"; }

echo "Self-check (read-only):"
vis_old="$(reader_count "$READER_OLD")"
vis_new="$(reader_count "$READER_NEW")"
if [[ "$vis_old" = "10" && "$vis_new" = "10" ]]; then
  ok "reader sees only the policy rows on both sides (10 of 20)"
else
  bad "reader should see 10 rows on each side, got old=$vis_old new=$vis_new"
fi

all_old="$(admin_count "$OLD_DB")"
all_new="$(admin_count "$NEW_DB")"
if [[ "$all_old" = "20" && "$all_new" = "20" ]]; then
  ok "dmux_dev (BYPASSRLS) sees all 20 rows on both sides"
else
  bad "dmux_dev should see 20 rows, got old=$all_old new=$all_new"
fi

hid_old="$(hidden_digest "$OLD_DB")"
hid_new="$(hidden_digest "$NEW_DB")"
if [[ -n "$hid_old" && -n "$hid_new" && "$hid_old" != "$hid_new" ]]; then
  ok "the hidden rows (branch 2) differ between old and new"
else
  bad "the hidden rows should differ, got old=$hid_old new=$hid_new"
fi

for pair in "old:$READER_OLD" "new:$READER_NEW"; do
  side="${pair%%:*}"
  url="${pair#*:}"
  if err="$(psql -X -q -v ON_ERROR_STOP=1 "$url" \
        -c "SET row_security = off; SELECT count(*) FROM public.branch_notes" 2>&1)"; then
    bad "$side: row_security=off SELECT succeeded, it must fail closed (42501)"
  elif [[ "$err" == *"row-level security"* ]]; then
    ok "$side: row_security=off is refused (fail closed)"
  else
    bad "$side: row_security=off failed for the wrong reason: $err"
  fi
done

echo
echo "Engine test env (see testdata/rls/README.md):"
printf '  DMUX_TEST_RLS_OLD_URL=%s\n' "$READER_OLD"
printf '  DMUX_TEST_RLS_NEW_URL=%s\n' "$READER_NEW"

printf '\n%s passed, %s failed\n' "$PASS" "$FAIL"
[[ "$FAIL" -eq 0 ]]
