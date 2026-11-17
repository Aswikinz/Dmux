#!/usr/bin/env bash
# demo_bank: prove the answer key against the planted databases (T016).
#
# Usage:
#   testdata/demo_bank/check_key.sh                 # old 9.6 (:5433) vs new 17 (:5434)
#   testdata/demo_bank/check_key.sh 5435 5437       # old 9.4 vs new 18
#   OLD_URL=... NEW_URL=... testdata/demo_bank/check_key.sh
#
# The database half of PLAN.md section 8.2. It reads testdata/expected_diffs.json
# and asks the two databases, with read-only SELECTs, whether every claim in it is
# true:
#
#   * every must_report entry: exact counts, keys and values as planted;
#   * every must_stay_quiet entry: the situation really is present (LATIN1 vs
#     UTF-8, C vs en_US.utf8, differing float print, partitioned table,
#     reordered columns, char(1) vs varchar(1)) and the data still agrees;
#   * every other table: same row count and same order-independent digest on both
#     sides, using verify.sh's normalisation (floats and bytea from bytes, money
#     as numeric, text under the canonical settings block), with renamed and
#     partitioned tables mapped by hand.
#
# This proves the key itself is right before the engine is graded against it. It
# exits non-zero on any mismatch. Run build.sh first (the default build plants).
#
# Read-only: the session runs with default_transaction_read_only=on and every
# digest runs inside BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY. A failed
# query is never silent: the query helpers record a FAIL and report no value, and
# every comparison refuses an empty side, so an error can never look like a pass.
# (The runtime case r01 needs a writer and is exercised by concurrent_update.sh,
# not by this static check.)
set -euo pipefail
unset PGHOSTADDR PGSERVICE PGSERVICEFILE PGHOST PGPORT PGDATABASE PGUSER PGPASSWORD
export PGOPTIONS="${PGOPTIONS:-} -c client_min_messages=warning -c default_transaction_read_only=on"
export PGCONNECT_TIMEOUT="${PGCONNECT_TIMEOUT:-10}"

HERE="$(cd "$(dirname "$0")" && pwd)"
KEY="$HERE/../expected_diffs.json"

OLD_URL="${OLD_URL:-postgres://dmux_dev:dmux_dev_only@127.0.0.1:${1:-5433}/demo_bank}"
NEW_URL="${NEW_URL:-postgres://dmux_dev:dmux_dev_only@127.0.0.1:${2:-5434}/demo_bank}"

# shellcheck disable=SC1091  # guard.sh is linted separately; $HERE is runtime-relative
. "$HERE/../phase0/guard.sh"
for u in "$OLD_URL" "$NEW_URL"; do
  if ! dmux_guard_url "$u"; then
    echo "refusing: check_key.sh only runs against local mock databases" >&2
    exit 1
  fi
done

[ -f "$KEY" ] || { echo "missing answer key: $KEY" >&2; exit 1; }
command -v python3 >/dev/null 2>&1 || { echo "python3 is required to read the key" >&2; exit 1; }

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

PASS=0
FAIL=0
HANDLED=""
GOT=""

ok()  { PASS=$((PASS + 1)); printf '  ok    %s\n' "$1"; }
bad() { FAIL=$((FAIL + 1)); printf '  FAIL  %s\n' "$1"; }

# A comparison cannot pass on nothing. An empty actual means the query failed or
# returned no row; an empty expected means the key (or python) failed. Both are
# failures, never a silent "" = "".
assert_eq() { # label, actual, expected
  if [ -z "$2" ] || [ -z "$3" ]; then
    bad "$1 (empty value: actual=[$2] expected=[$3]; a query or the key failed)"
  elif [ "$2" = "$3" ]; then
    ok "$1"
  else
    bad "$1 (expected [$3], got [$2])"
  fi
}

# ---------------------------------------------------------------------------
# Reading the key. keyval takes an entry id and a Python expression over v (the
# entry's "expected" object) and prints one line.
# ---------------------------------------------------------------------------
keyval() { # entry id, expression
  python3 - "$KEY" "$1" "$2" <<'PY'
import json, sys
key, eid, expr = sys.argv[1], sys.argv[2], sys.argv[3]
with open(key, encoding="utf-8") as fh:
    data = json.load(fh)
entry = None
for section in ("must_report", "must_stay_quiet", "runtime"):
    for candidate in data.get(section, []):
        if candidate.get("id") == eid:
            entry = candidate
if entry is None:
    raise SystemExit("no entry " + eid)
v = entry["expected"]
print(eval(expr, {"json": json, "v": v, "e": entry}))
PY
}

key_ids() { # section
  python3 - "$KEY" "$1" <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as fh:
    data = json.load(fh)
print(" ".join(entry["id"] for entry in data[sys.argv[2]]))
PY
}

# ---------------------------------------------------------------------------
# Query helpers. Every read asks for UTF-8 text so the LATIN1 old server prints
# the same characters as the UTF-8 new one (decision 4), under the canonical
# settings block from the T003 review.
#
# scalar/raw_scalar are the only things that talk to psql. Each returns non-zero
# on any error (ON_ERROR_STOP) and prints nothing. The get_* wrappers below are
# called directly (never inside $( )) so a failure is recorded in the real
# process, and they leave GOT empty so no comparison can pass on it.
# ---------------------------------------------------------------------------
scalar() { # url, sql
  psql -X -q -A -t -v ON_ERROR_STOP=1 "$1" -c "
    SET client_encoding      = 'UTF8';
    SET timezone             = 'UTC';
    SET datestyle            = 'ISO, YMD';
    SET extra_float_digits   = 0;
    $2"
}

raw_scalar() { # url, sql - no settings (for SHOW and version checks)
  psql -X -q -A -t -v ON_ERROR_STOP=1 "$1" -c "$2"
}

get_scalar() { # label, url, sql -> GOT
  local out
  if out="$(scalar "$2" "$3")"; then
    GOT="$out"
  else
    bad "$1 (query failed)"
    GOT=""
  fi
}

get_raw() { # label, url, sql -> GOT
  local out
  if out="$(raw_scalar "$2" "$3")"; then
    GOT="$out"
  else
    bad "$1 (query failed)"
    GOT=""
  fi
}

check() { # label, url, sql, expected
  get_scalar "$1" "$2" "$3"
  assert_eq "$1" "$GOT" "$4"
}

check_raw() { # label, url, sql, expected
  get_raw "$1" "$2" "$3"
  assert_eq "$1" "$GOT" "$4"
}

# A value used in shell arithmetic must really be a number; otherwise the
# arithmetic would silently treat it as 0. No PASS here: this only raises FAIL.
as_int() { # label, value -> GOT
  case "$2" in
    ''|*[!0-9]*) bad "$1 (not an integer: [$2])"; GOT=0 ;;
    *) GOT="$2" ;;
  esac
}

columns_of() { # url, table -> comma-separated names in attnum order
  psql -X -q -A -t -v ON_ERROR_STOP=1 -v tbl="$2" "$1" <<'SQL'
SELECT string_agg(a.attname, ',' ORDER BY a.attnum)
FROM pg_attribute a
WHERE a.attrelid = format('demo_bank.%I', :'tbl')::regclass
  AND a.attnum > 0 AND NOT a.attisdropped
SQL
}

col_exprs() { # url, table, comma-separated columns -> normalised SQL expressions
  psql -X -q -A -t -v ON_ERROR_STOP=1 -v tbl="$2" -v cols="$3" "$1" <<'SQL'
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
         END, ', ' ORDER BY u.ord)
FROM unnest(string_to_array(:'cols', ',')) WITH ORDINALITY AS u(name, ord)
JOIN pg_attribute a
  ON a.attrelid = format('demo_bank.%I', :'tbl')::regclass
 AND a.attname = u.name
SQL
}

digest() { # url, table, exprs, [filter] -> count|md5
  psql -X -q -A -t -v ON_ERROR_STOP=1 -v tbl="$2" -v exprs="$3" \
    -v filt="${4:-true}" "$1" <<'SQL'
BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL search_path        = pg_catalog;
SET LOCAL client_encoding    = 'UTF8';
SET LOCAL timezone           = 'UTC';
SET LOCAL datestyle          = 'ISO, YMD';
SET LOCAL intervalstyle      = 'iso_8601';
SET LOCAL bytea_output       = 'hex';
SET LOCAL extra_float_digits = 0;
SET LOCAL xmloption          = 'document';
SET LOCAL statement_timeout  = '300s';
SET LOCAL lock_timeout       = '5s';
SET LOCAL application_name   = 'dmux-check-key';
SELECT count(*) || '|' || coalesce(md5(string_agg(h, '' ORDER BY h COLLATE "C")), 'empty')
FROM (
  SELECT md5(convert_to(ROW(:exprs)::text, 'UTF8')) AS h
  FROM demo_bank.:"tbl"
  WHERE :filt
) r;
COMMIT;
SQL
}

get_digest() { # label, url, table, exprs, [filter] -> GOT as count|md5
  local out
  if out="$(digest "$2" "$3" "$4" "${5:-true}")"; then
    GOT="$out"
  else
    bad "$1 (digest query failed)"
    GOT=""
  fi
}

get_hash() { # label, url, table, exprs, [filter] -> GOT as md5 only
  get_digest "$1" "$2" "$3" "$4" "${5:-true}"
  GOT="${GOT#*|}"
}

common_columns() { # old list, new list -> intersection in old order
  local out="" c
  local IFS=','
  for c in $1; do
    case ",$2," in
      *",$c,"*) out="${out:+$out,}$c" ;;
    esac
  done
  printf '%s' "$out"
}

without_column() { # column list, name to drop
  local out="" c
  local IFS=','
  for c in $1; do
    [ "$c" = "$2" ] || out="${out:+$out,}$c"
  done
  printf '%s' "$out"
}

new_table_for() { # old table name -> new table name (mapping rules)
  case "$1" in
    limits) echo "product_limits" ;;
    *) echo "$1" ;;
  esac
}

mark_handled() { HANDLED="$HANDLED $1"; }

# ---------------------------------------------------------------------------
# Shape of the key itself.
# ---------------------------------------------------------------------------
echo "Answer key: $KEY"
if python3 - "$KEY" <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as fh:
    data = json.load(fh)
expected_top = {"version", "dataset", "must_report", "must_stay_quiet", "runtime"}
if set(data) != expected_top:
    raise SystemExit("top-level keys are " + repr(sorted(data)))
if data["version"] != 1:
    raise SystemExit("version is " + repr(data["version"]))
if data["dataset"] != "demo_bank-small":
    raise SystemExit("dataset is " + repr(data["dataset"]))
for section in ("must_report", "must_stay_quiet", "runtime"):
    for entry in data[section]:
        for field in ("id", "plan_ref", "object", "kind", "expected", "level"):
            if field not in entry:
                raise SystemExit("entry " + str(entry.get("id")) + " has no " + field)
        if not entry["plan_ref"].startswith("8.2 "):
            raise SystemExit("entry " + entry["id"] + " has odd plan_ref")
PY
then
  ok "version 1, dataset demo_bank-small, every entry has the required fields"
else
  bad "the answer key does not have the expected shape"
fi

echo "Servers:"
get_raw "old server" "$OLD_URL" "SELECT current_setting('server_version')
    || ', ' || pg_encoding_to_char(encoding) || ', collate ' || datcollate
  FROM pg_database WHERE datname = current_database()"
printf '  old %s\n' "$GOT"
[ -n "$GOT" ] || bad "old server did not answer"
get_raw "new server" "$NEW_URL" "SELECT current_setting('server_version')
    || ', ' || pg_encoding_to_char(encoding) || ', collate ' || datcollate
  FROM pg_database WHERE datname = current_database()"
printf '  new %s\n' "$GOT"
[ -n "$GOT" ] || bad "new server did not answer"

# ---------------------------------------------------------------------------
# Must-report entries. Every expected value comes from the key, so a key edit
# cannot drift away from this check.
# ---------------------------------------------------------------------------
echo "Differences Dmux must report:"

# m01: 12 deleted transaction rows.
mark_handled m01
get_scalar "m01 old transactions count" "$OLD_URL" "SELECT count(*) FROM demo_bank.transactions"
as_int "m01 old transactions count" "$GOT"
old_count="$GOT"
get_scalar "m01 new transactions count" "$NEW_URL" "SELECT count(*) FROM demo_bank.transactions"
as_int "m01 new transactions count" "$GOT"
new_count="$GOT"
assert_eq "m01 old transactions count" "$old_count" "$(keyval m01 'v["old_count"]')"
assert_eq "m01 new transactions count" "$new_count" "$(keyval m01 'v["new_count"]')"
assert_eq "m01 count difference" "$((old_count - new_count))" "$(keyval m01 'v["missing_count"]')"
m01_ids="$(keyval m01 '",".join(str(k) for k in v["missing_keys"])')"
get_scalar "m01 keys present on old" "$OLD_URL" "SELECT count(*) FROM demo_bank.transactions WHERE transaction_id IN ($m01_ids)"
old_present="$GOT"
get_scalar "m01 keys present on new" "$NEW_URL" "SELECT count(*) FROM demo_bank.transactions WHERE transaction_id IN ($m01_ids)"
new_present="$GOT"
assert_eq "m01 all 12 keys exist on old" "$old_present" "$(keyval m01 'len(v["missing_keys"])')"
assert_eq "m01 all 12 keys are absent on new" "$new_present" "0"

# m02: 3 duplicate rows in the keyless audit_log.
mark_handled m02
get_scalar "m02 old audit_log count" "$OLD_URL" "SELECT count(*) FROM demo_bank.audit_log"
as_int "m02 old audit_log count" "$GOT"
old_count="$GOT"
get_scalar "m02 new audit_log count" "$NEW_URL" "SELECT count(*) FROM demo_bank.audit_log"
as_int "m02 new audit_log count" "$GOT"
new_count="$GOT"
assert_eq "m02 old audit_log count" "$old_count" "$(keyval m02 'v["old_count"]')"
assert_eq "m02 new audit_log count" "$new_count" "$(keyval m02 'v["new_count"]')"
assert_eq "m02 count difference" "$((new_count - old_count))" "$(keyval m02 'v["extra_count"]')"
keyval m02 '"\n".join("\t".join([r["event_at"], r["actor"], r["action"], r["object_name"], r["detail"]]) for r in v["extra_rows"])' > "$TMP/m02.tsv"
m02_rows=0
while IFS=$'\t' read -r ts actor action object_name detail; do
  [ -n "${ts:-}" ] || continue
  m02_rows=$((m02_rows + 1))
  pred="event_at = timestamptz '$ts' AND actor = '$actor' AND action = '$action'
        AND object_name = '$object_name' AND detail = '$detail'"
  get_scalar "m02 duplicate row $m02_rows on old" "$OLD_URL" "SELECT count(*) FROM demo_bank.audit_log WHERE $pred"
  old_n="$GOT"
  get_scalar "m02 duplicate row $m02_rows on new" "$NEW_URL" "SELECT count(*) FROM demo_bank.audit_log WHERE $pred"
  new_n="$GOT"
  assert_eq "m02 duplicate row $m02_rows ($detail): once on old" "$old_n" "1"
  assert_eq "m02 duplicate row $m02_rows ($detail): twice on new" "$new_n" "2"
done < "$TMP/m02.tsv"
assert_eq "m02 number of extra rows" "$m02_rows" "$(keyval m02 'len(v["extra_rows"])')"

# m03: balance changed by exactly 0.01, and the control total follows.
mark_handled m03
m03_key="$(keyval m03 'v["key"]["account_id"]')"
get_scalar "m03 old balance" "$OLD_URL" "SELECT balance::text FROM demo_bank.accounts WHERE account_id = $m03_key"
old_val="$GOT"
get_scalar "m03 new balance" "$NEW_URL" "SELECT balance::text FROM demo_bank.accounts WHERE account_id = $m03_key"
new_val="$GOT"
assert_eq "m03 account $m03_key old balance" "$old_val" "$(keyval m03 'v["old"]')"
assert_eq "m03 account $m03_key new balance" "$new_val" "$(keyval m03 'v["new"]')"
m03_difference=""
if [ -n "$old_val" ] && [ -n "$new_val" ]; then
  m03_difference="$(python3 -c 'import sys; print("%.4f" % (float(sys.argv[1]) - float(sys.argv[2])))' "$new_val" "$old_val")"
fi
assert_eq "m03 balance difference" "$m03_difference" "$(keyval m03 'v["difference"]')"
get_scalar "m03 old control total" "$OLD_URL" "SELECT sum(balance)::text FROM demo_bank.accounts"
old_sum="$GOT"
get_scalar "m03 new control total" "$NEW_URL" "SELECT sum(balance)::text FROM demo_bank.accounts"
new_sum="$GOT"
assert_eq "m03 old control total" "$old_sum" "$(keyval m03 'v["control_total"]["old"]')"
assert_eq "m03 new control total" "$new_sum" "$(keyval m03 'v["control_total"]["new"]')"

# m04: one character changed.
mark_handled m04
m04_key="$(keyval m04 'v["key"]["transaction_id"]')"
repr_sql="SELECT CASE WHEN narration IS NULL THEN '<NULL>' ELSE '[' || narration || ']' END
  FROM demo_bank.transactions WHERE transaction_id = $m04_key"
get_scalar "m04 old value" "$OLD_URL" "$repr_sql"
old_repr="$GOT"
get_scalar "m04 new value" "$NEW_URL" "$repr_sql"
new_repr="$GOT"
assert_eq "m04 transaction $m04_key old value" "$old_repr" "$(keyval m04 '"[%s]" % v["old"] if v["old"] is not None else "<NULL>"')"
assert_eq "m04 transaction $m04_key new value" "$new_repr" "$(keyval m04 '"[%s]" % v["new"] if v["new"] is not None else "<NULL>"')"

# m05: empty text became NULL.
mark_handled m05
m05_key="$(keyval m05 'v["key"]["transaction_id"]')"
repr_sql="SELECT CASE WHEN narration IS NULL THEN '<NULL>' ELSE '[' || narration || ']' END
  FROM demo_bank.transactions WHERE transaction_id = $m05_key"
get_scalar "m05 old value" "$OLD_URL" "$repr_sql"
old_repr="$GOT"
get_scalar "m05 new value" "$NEW_URL" "$repr_sql"
new_repr="$GOT"
assert_eq "m05 transaction $m05_key old value" "$old_repr" "$(keyval m05 '"[%s]" % v["old"] if v["old"] is not None else "<NULL>"')"
assert_eq "m05 transaction $m05_key new value" "$new_repr" "$(keyval m05 '"[%s]" % v["new"] if v["new"] is not None else "<NULL>"')"

# m06: time of day dropped from a timestamp.
mark_handled m06
m06_key="$(keyval m06 'v["key"]["transaction_id"]')"
get_scalar "m06 old timestamp" "$OLD_URL" "SELECT value_timestamp::text FROM demo_bank.transactions WHERE transaction_id = $m06_key"
old_ts="$GOT"
get_scalar "m06 new timestamp" "$NEW_URL" "SELECT value_timestamp::text FROM demo_bank.transactions WHERE transaction_id = $m06_key"
new_ts="$GOT"
assert_eq "m06 transaction $m06_key old timestamp" "$old_ts" "$(keyval m06 'v["old"]')"
assert_eq "m06 transaction $m06_key new timestamp" "$new_ts" "$(keyval m06 'v["new"]')"

# m07/m08: one column dropped, one column added.
mark_handled m07
mark_handled m08
get_scalar "m07 granted_by type on old" "$OLD_URL" "SELECT format_type(atttypid, atttypmod) FROM pg_attribute
  WHERE attrelid = 'demo_bank.user_roles'::regclass AND attname = 'granted_by' AND NOT attisdropped"
m07_type="$GOT"
assert_eq "m07 user_roles.granted_by exists on old with the keyed type" "$m07_type" "$(keyval m07 'v["old_type"]')"
check "m07 user_roles.granted_by column count on new" "$NEW_URL" \
  "SELECT count(*) FROM pg_attribute WHERE attrelid = 'demo_bank.user_roles'::regclass AND attname = 'granted_by' AND NOT attisdropped" "0"
check "m08 loans.settlement_ref column count on old" "$OLD_URL" \
  "SELECT count(*) FROM pg_attribute WHERE attrelid = 'demo_bank.loans'::regclass AND attname = 'settlement_ref' AND NOT attisdropped" "0"
get_scalar "m08 settlement_ref type on new" "$NEW_URL" "SELECT format_type(atttypid, atttypmod) FROM pg_attribute
  WHERE attrelid = 'demo_bank.loans'::regclass AND attname = 'settlement_ref' AND NOT attisdropped"
m08_type="$GOT"
assert_eq "m08 loans.settlement_ref exists on new with the keyed type" "$m08_type" "$(keyval m08 'v["new_type"]')"

# m09: one table renamed. The rows must be the same data under the new name.
mark_handled m09
m09_old="$(keyval m09 'v["old_name"]')"
m09_new="$(keyval m09 'v["new_name"]')"
get_scalar "m09 old table exists" "$OLD_URL" "SELECT count(*) FROM pg_class c
  JOIN pg_namespace n ON n.oid = c.relnamespace
  WHERE n.nspname = 'demo_bank' AND c.relname = '$m09_old' AND c.relkind IN ('r', 'p')"
assert_eq "m09 $m09_old exists on old" "$GOT" "1"
get_scalar "m09 old name is gone on new" "$NEW_URL" "SELECT count(*) FROM pg_class c
  JOIN pg_namespace n ON n.oid = c.relnamespace
  WHERE n.nspname = 'demo_bank' AND c.relname = '$m09_old' AND c.relkind IN ('r', 'p')"
assert_eq "m09 $m09_old is gone on new" "$GOT" "0"
get_scalar "m09 new table exists" "$NEW_URL" "SELECT count(*) FROM pg_class c
  JOIN pg_namespace n ON n.oid = c.relnamespace
  WHERE n.nspname = 'demo_bank' AND c.relname = '$m09_new' AND c.relkind IN ('r', 'p')"
assert_eq "m09 $m09_new exists on new" "$GOT" "1"
check "m09 $m09_new row count on new" "$NEW_URL" "SELECT count(*) FROM demo_bank.$m09_new" "$(keyval m09 'v["rows"]')"
if ! m09_cols="$(columns_of "$OLD_URL" "$m09_old")"; then
  bad "m09 columns query failed for $m09_old"
  m09_cols=""
fi
if [ -z "$m09_cols" ]; then
  bad "m09 cannot compare $m09_old with $m09_new (no columns)"
else
  get_digest "m09 new data digest" "$NEW_URL" "$m09_new" "$(col_exprs "$NEW_URL" "$m09_new" "$m09_cols")"
  m09_new_digest="$GOT"
  get_digest "m09 old data digest" "$OLD_URL" "$m09_old" "$(col_exprs "$OLD_URL" "$m09_old" "$m09_cols")"
  assert_eq "m09 $m09_old -> $m09_new data digest" "$m09_new_digest" "$GOT"
fi

# m10: one column renamed. The values must be the same data under the new name.
mark_handled m10
m10_old="$(keyval m10 'v["old_name"]')"
m10_new="$(keyval m10 'v["new_name"]')"
check "m10 fees.$m10_old is gone on new" "$NEW_URL" \
  "SELECT count(*) FROM pg_attribute WHERE attrelid = 'demo_bank.fees'::regclass AND attname = '$m10_old' AND NOT attisdropped" "0"
check "m10 fees.$m10_new exists on new" "$NEW_URL" \
  "SELECT count(*) FROM pg_attribute WHERE attrelid = 'demo_bank.fees'::regclass AND attname = '$m10_new' AND NOT attisdropped" "1"
check "m10 fees row count on new" "$NEW_URL" "SELECT count(*) FROM demo_bank.fees" "$(keyval m10 'v["rows"]')"
get_scalar "m10 old fee values" "$OLD_URL" "SELECT md5(string_agg(fee_id || '|' || fee_name, ',' ORDER BY fee_id)) FROM demo_bank.fees"
m10_old_digest="$GOT"
get_scalar "m10 new fee values" "$NEW_URL" "SELECT md5(string_agg(fee_id || '|' || fee_label, ',' ORDER BY fee_id)) FROM demo_bank.fees"
assert_eq "m10 fees.$m10_old -> $m10_new value digest" "$m10_old_digest" "$GOT"

# m11: varchar widened (amber).
mark_handled m11
get_scalar "m11 old city type" "$OLD_URL" "SELECT format_type(atttypid, atttypmod) FROM pg_attribute
  WHERE attrelid = 'demo_bank.branches'::regclass AND attname = 'city'"
m11_old="$GOT"
get_scalar "m11 new city type" "$NEW_URL" "SELECT format_type(atttypid, atttypmod) FROM pg_attribute
  WHERE attrelid = 'demo_bank.branches'::regclass AND attname = 'city'"
m11_new="$GOT"
assert_eq "m11 branches.city old type" "$m11_old" "$(keyval m11 'v["old_type"]')"
assert_eq "m11 branches.city new type" "$m11_new" "$(keyval m11 'v["new_type"]')"

# m12: sequence left behind its old value.
mark_handled m12
get_scalar "m12 old last_value" "$OLD_URL" "SELECT last_value FROM demo_bank.seq_gl_entry"
as_int "m12 old last_value" "$GOT"
m12_old="$GOT"
get_scalar "m12 new last_value" "$NEW_URL" "SELECT last_value FROM demo_bank.seq_gl_entry"
as_int "m12 new last_value" "$GOT"
m12_new="$GOT"
assert_eq "m12 seq_gl_entry old last_value" "$m12_old" "$(keyval m12 'v["old_last_value"]')"
assert_eq "m12 seq_gl_entry new last_value" "$m12_new" "$(keyval m12 'v["new_last_value"]')"
assert_eq "m12 old next value" "$((m12_old + 1))" "$(keyval m12 'v["old_next_value"]')"
assert_eq "m12 new next value" "$((m12_new + 1))" "$(keyval m12 'v["new_next_value"]')"
if [ "$m12_new" -lt "$m12_old" ]; then
  ok "m12 the new sequence is behind the old one"
else
  bad "m12 the new sequence is not behind the old one"
fi

# m13: one index missing.
mark_handled m13
m13_index="$(keyval m13 'v["index"]')"
check "m13 $m13_index exists on old" "$OLD_URL" \
  "SELECT count(*) FROM pg_indexes WHERE schemaname = 'demo_bank' AND indexname = '$m13_index'" "1"
check "m13 $m13_index is gone on new" "$NEW_URL" \
  "SELECT count(*) FROM pg_indexes WHERE schemaname = 'demo_bank' AND indexname = '$m13_index'" "0"

# m14: one constraint not validated.
mark_handled m14
m14_con="$(keyval m14 'v["constraint"]')"
get_scalar "m14 validated on old" "$OLD_URL" "SELECT convalidated FROM pg_constraint WHERE conname = '$m14_con' AND conrelid = 'demo_bank.transactions'::regclass"
m14_old="$GOT"
get_scalar "m14 validated on new" "$NEW_URL" "SELECT convalidated FROM pg_constraint WHERE conname = '$m14_con' AND conrelid = 'demo_bank.transactions'::regclass"
m14_new="$GOT"
assert_eq "m14 $m14_con validated on old" "$m14_old" "t"
assert_eq "m14 $m14_con not validated on new" "$m14_new" "f"
check "m14 $m14_con exists on both sides" "$NEW_URL" \
  "SELECT contype::text FROM pg_constraint WHERE conname = '$m14_con' AND conrelid = 'demo_bank.transactions'::regclass" \
  "$(keyval m14 'v["constraint_type"]')"

# m15: the large object missing.
mark_handled m15
m15_oid="$(keyval m15 'v["large_object_oid"]')"
check "m15 large object $m15_oid exists on old" "$OLD_URL" \
  "SELECT count(*) FROM pg_largeobject_metadata WHERE oid = $m15_oid" "1"
check "m15 large object $m15_oid is gone on new" "$NEW_URL" \
  "SELECT count(*) FROM pg_largeobject_metadata WHERE oid = $m15_oid" "0"
check "m15 large object metadata count on new" "$NEW_URL" \
  "SELECT count(*) FROM pg_largeobject_metadata" "$(keyval m15 'v["new_count"]')"

# ---------------------------------------------------------------------------
# Stay-quiet entries: the situation must really be present, and the data agree.
# ---------------------------------------------------------------------------
echo "Cases where Dmux must stay quiet:"

# q01: LATIN1 old, UTF-8 new, accented names.
mark_handled q01
check "q01 old database encoding" "$OLD_URL" \
  "SELECT pg_encoding_to_char(encoding) FROM pg_database WHERE datname = current_database()" \
  "$(keyval q01 'v["old_encoding"]')"
check "q01 new database encoding" "$NEW_URL" \
  "SELECT pg_encoding_to_char(encoding) FROM pg_database WHERE datname = current_database()" \
  "$(keyval q01 'v["new_encoding"]')"
q01_key="$(keyval q01 'v["sample_key"]["branch_id"]')"
check "q01 accented value: identical UTF-8 bytes on the old side" "$OLD_URL" \
  "SELECT encode(convert_to(branch_name, 'UTF8'), 'hex') FROM demo_bank.branches WHERE branch_id = $q01_key" \
  "$(keyval q01 'v["utf8_hex"]')"
check "q01 accented value: identical UTF-8 bytes on the new side" "$NEW_URL" \
  "SELECT encode(convert_to(branch_name, 'UTF8'), 'hex') FROM demo_bank.branches WHERE branch_id = $q01_key" \
  "$(keyval q01 'v["utf8_hex"]')"
check "q01 decoded accented value" "$NEW_URL" \
  "SELECT branch_name FROM demo_bank.branches WHERE branch_id = $q01_key" \
  "$(keyval q01 'v["sample_value"]')"

# q02: the two servers sort text differently.
mark_handled q02
check "q02 old collation" "$OLD_URL" \
  "SELECT datcollate FROM pg_database WHERE datname = current_database()" \
  "$(keyval q02 'v["old_collate"]')"
check "q02 new collation" "$NEW_URL" \
  "SELECT datcollate FROM pg_database WHERE datname = current_database()" \
  "$(keyval q02 'v["new_collate"]')"
q02_sql="SELECT string_agg(x, ',' ORDER BY x) FROM (VALUES ('z'), (chr(233))) AS v(x)"
check "q02 old sort order" "$OLD_URL" "$q02_sql" "$(keyval q02 '",".join(v["old_order"])')"
check "q02 new sort order" "$NEW_URL" "$q02_sql" "$(keyval q02 '",".join(v["new_order"])')"

# q03: floats printed differently by the two servers, identical from bytes.
mark_handled q03
q03_key="$(keyval q03 'v["key"]["product_id"]')"
check_raw "q03 old default extra_float_digits" "$OLD_URL" "SHOW extra_float_digits" \
  "$(keyval q03 'v["old_default_extra_float_digits"]')"
check_raw "q03 new default extra_float_digits" "$NEW_URL" "SHOW extra_float_digits" \
  "$(keyval q03 'v["new_default_extra_float_digits"]')"
check "q03 canonical printed value on old" "$OLD_URL" \
  "SELECT nominal_rate::text FROM demo_bank.products WHERE product_id = $q03_key" \
  "$(keyval q03 'v["canonical_value"]')"
check "q03 canonical printed value on new" "$NEW_URL" \
  "SELECT nominal_rate::text FROM demo_bank.products WHERE product_id = $q03_key" \
  "$(keyval q03 'v["canonical_value"]')"
check_raw "q03 9.6 high-precision printed form" "$OLD_URL" \
  "SET extra_float_digits = 2; SELECT nominal_rate::text FROM demo_bank.products WHERE product_id = $q03_key" \
  "$(keyval q03 'v["old_high_precision"]')"
check_raw "q03 17 high-precision printed form" "$NEW_URL" \
  "SET extra_float_digits = 2; SELECT nominal_rate::text FROM demo_bank.products WHERE product_id = $q03_key" \
  "$(keyval q03 'v["new_high_precision"]')"
get_scalar "q03 old float bytes" "$OLD_URL" \
  "SELECT encode(float4send(nominal_rate), 'hex') FROM demo_bank.products WHERE product_id = $q03_key"
q03_old_bytes="$GOT"
get_scalar "q03 new float bytes" "$NEW_URL" \
  "SELECT encode(float4send(nominal_rate), 'hex') FROM demo_bank.products WHERE product_id = $q03_key"
assert_eq "q03 float bytes are equal" "$q03_old_bytes" "$GOT"

# q04: a table converted to a partitioned table.
mark_handled q04
check "q04 balances_daily is an ordinary table on old" "$OLD_URL" \
  "SELECT c.relkind::text FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = 'demo_bank' AND c.relname = 'balances_daily'" \
  "$(keyval q04 'v["old_relkind"]')"
check "q04 balances_daily is a partitioned table on new" "$NEW_URL" \
  "SELECT c.relkind::text FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = 'demo_bank' AND c.relname = 'balances_daily'" \
  "$(keyval q04 'v["new_relkind"]')"
get_scalar "q04 new partition set" "$NEW_URL" "SELECT string_agg(c.relname, ',' ORDER BY c.relname)
  FROM pg_inherits i JOIN pg_class c ON c.oid = i.inhrelid
  WHERE i.inhparent = 'demo_bank.balances_daily'::regclass"
assert_eq "q04 new partition set" "$GOT" "$(keyval q04 '",".join(v["new_partitions"])')"
check "q04 old row count" "$OLD_URL" "SELECT count(*) FROM demo_bank.balances_daily" "$(keyval q04 'v["rows"]')"
check "q04 new row count through the parent" "$NEW_URL" "SELECT count(*) FROM demo_bank.balances_daily" "$(keyval q04 'v["rows"]')"

# q05: columns in a different order.
mark_handled q05
if ! q05_old="$(columns_of "$OLD_URL" holidays)"; then bad "q05 old column query failed"; q05_old=""; fi
if ! q05_new="$(columns_of "$NEW_URL" holidays)"; then bad "q05 new column query failed"; q05_new=""; fi
assert_eq "q05 old column order" "$q05_old" "$(keyval q05 '",".join(v["old_column_order"])')"
assert_eq "q05 new column order" "$q05_new" "$(keyval q05 '",".join(v["new_column_order"])')"
check "q05 row count" "$NEW_URL" "SELECT count(*) FROM demo_bank.holidays" "$(keyval q05 'v["rows"]')"

# q06: char(1) moved to varchar(1) - amber type change, values pass.
mark_handled q06
get_scalar "q06 old result type" "$OLD_URL" "SELECT format_type(atttypid, atttypmod) FROM pg_attribute
  WHERE attrelid = 'demo_bank.kyc_checks'::regclass AND attname = 'result'"
q06_old="$GOT"
get_scalar "q06 new result type" "$NEW_URL" "SELECT format_type(atttypid, atttypmod) FROM pg_attribute
  WHERE attrelid = 'demo_bank.kyc_checks'::regclass AND attname = 'result'"
q06_new="$GOT"
assert_eq "q06 kyc_checks.result old type" "$q06_old" "$(keyval q06 'v["old_type"]')"
assert_eq "q06 kyc_checks.result new type" "$q06_new" "$(keyval q06 'v["new_type"]')"
check "q06 row count" "$NEW_URL" "SELECT count(*) FROM demo_bank.kyc_checks" "$(keyval q06 'v["rows"]')"

# ---------------------------------------------------------------------------
# Per-table data identity, with renamed and repartitioned tables mapped.
# transactions, audit_log and accounts carry planted changes; their other rows
# are checked just below, and the changes themselves above.
# ---------------------------------------------------------------------------
echo "Per-table data identity:"

tables_of() { # url
  local extra=""
  # relispartition is Postgres 10+; the 9.4/9.6 old side has no partitions.
  if [ "$(scalar "$1" "SELECT count(*) FROM pg_attribute
      WHERE attrelid = 'pg_class'::regclass AND attname = 'relispartition'")" = "1" ]; then
    extra="AND NOT c.relispartition"
  fi
  psql -X -q -A -t -v ON_ERROR_STOP=1 "$1" -c "
    SELECT c.relname
    FROM pg_class c
    JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = 'demo_bank' AND c.relkind IN ('r', 'p') $extra
    ORDER BY c.relname::text COLLATE \"C\""
}

if ! tables_of "$OLD_URL" > "$TMP/old.tables"; then
  bad "could not list the tables on old"
  exit 1
fi
if ! tables_of "$NEW_URL" > "$TMP/new.tables"; then
  bad "could not list the tables on new"
  exit 1
fi
grep -vx 'limits' "$TMP/old.tables" > "$TMP/old.remap" || true
grep -vx 'product_limits' "$TMP/new.tables" > "$TMP/new.remap" || true
if diff -u "$TMP/old.remap" "$TMP/new.remap" > "$TMP/tables.diff"; then
  ok "the two sides hold the same tables once the planted rename is mapped"
else
  bad "the two sides do not hold the same tables"
  sed 's/^/        /' "$TMP/tables.diff"
fi

IDENTICAL_SKIP=" transactions audit_log accounts "
while IFS= read -r tbl; do
  [ -n "$tbl" ] || continue
  new_tbl="$(new_table_for "$tbl")"
  get_scalar "$tbl: counterpart exists" "$NEW_URL" "SELECT count(*) FROM pg_class c
    JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = 'demo_bank' AND c.relname = '$new_tbl' AND c.relkind IN ('r', 'p')"
  if [ "$GOT" != "1" ]; then
    bad "$tbl: no counterpart $new_tbl on new"
    continue
  fi
  if ! old_cols="$(columns_of "$OLD_URL" "$tbl")"; then
    bad "$tbl: column query failed on old"
    continue
  fi
  if ! new_cols="$(columns_of "$NEW_URL" "$new_tbl")"; then
    bad "$tbl: column query failed on new"
    continue
  fi
  common="$(common_columns "$old_cols" "$new_cols")"
  if [ -z "$common" ]; then
    bad "$tbl: no common columns with $new_tbl"
    continue
  fi
  get_digest "$tbl: digest on old" "$OLD_URL" "$tbl" "$(col_exprs "$OLD_URL" "$tbl" "$common")"
  d_old="$GOT"
  get_digest "$tbl: digest on new" "$NEW_URL" "$new_tbl" "$(col_exprs "$NEW_URL" "$new_tbl" "$common")"
  d_new="$GOT"
  case "$IDENTICAL_SKIP" in
    *" $tbl "*)
      # The planted changes are checked above; prove nothing else moved here.
      case "$tbl" in
        accounts)
          acc_cols="$(without_column "$old_cols" balance)"
          assert_eq "$tbl: same row count" "${d_new%%|*}" "${d_old%%|*}"
          get_digest "$tbl: filtered digest on new" "$NEW_URL" accounts \
            "$(col_exprs "$NEW_URL" accounts "$acc_cols")"
          acc_new="$GOT"
          get_digest "$tbl: filtered digest on old" "$OLD_URL" accounts \
            "$(col_exprs "$OLD_URL" accounts "$acc_cols")"
          assert_eq "$tbl: identical outside balance" "$acc_new" "$GOT"
          ;;
        transactions)
          tx_filter="transaction_id NOT IN ($m04_key, $m05_key, $m06_key) AND transaction_id NOT IN ($m01_ids)"
          get_hash "$tbl: filtered hash on new" "$NEW_URL" transactions \
            "$(col_exprs "$NEW_URL" transactions "$old_cols")" "$tx_filter"
          tx_new="$GOT"
          get_hash "$tbl: filtered hash on old" "$OLD_URL" transactions \
            "$(col_exprs "$OLD_URL" transactions "$old_cols")" "$tx_filter"
          assert_eq "$tbl: identical outside the changed and deleted rows" "$tx_new" "$GOT"
          ;;
        audit_log)
          audit_filter="COALESCE(detail, '<null>') NOT IN ('detail 1', 'detail 2', 'detail 3')"
          get_hash "$tbl: filtered hash on new" "$NEW_URL" audit_log \
            "$(col_exprs "$NEW_URL" audit_log "$old_cols")" "$audit_filter"
          audit_new="$GOT"
          get_hash "$tbl: filtered hash on old" "$OLD_URL" audit_log \
            "$(col_exprs "$OLD_URL" audit_log "$old_cols")" "$audit_filter"
          assert_eq "$tbl: identical outside the 3 duplicated rows" "$audit_new" "$GOT"
          ;;
      esac
      ;;
    *)
      assert_eq "$(printf '%-22s %s rows' "$tbl" "${d_old%%|*}")" "$d_new" "$d_old"
      ;;
  esac
done < "$TMP/old.tables"

# ---------------------------------------------------------------------------
# Runtime case: static check only (the helper must exist and the target row must
# be there). The behaviour itself is exercised by the engine harness.
# ---------------------------------------------------------------------------
mark_handled r01
if [ -x "$HERE/concurrent_update.sh" ]; then
  ok "r01 concurrent_update.sh is present and executable"
else
  bad "r01 concurrent_update.sh is missing or not executable"
fi
r01_key="$(keyval r01 'v["row"]["note_id"]')"
check "r01 target row exists on new" "$NEW_URL" \
  "SELECT count(*) FROM demo_bank.notes WHERE note_id = $r01_key" "1"
assert_eq "r01 the key says the verdict may never be PASS" "$(keyval r01 'v["never"]')" "PASS"

# ---------------------------------------------------------------------------
# Coverage: every entry in the key was checked by this script.
# ---------------------------------------------------------------------------
echo "Key coverage:"
uncovered=0
for id in $(key_ids must_report) $(key_ids must_stay_quiet) $(key_ids runtime); do
  case " $HANDLED " in
    *" $id "*) ;;
    *) uncovered=$((uncovered + 1)); bad "entry $id is in the key but check_key.sh does not check it" ;;
  esac
done
if [ "$uncovered" -eq 0 ]; then
  ok "every entry in the key is covered by a check"
fi

printf '\n%s passed, %s failed\n' "$PASS" "$FAIL"
[ "$FAIL" -eq 0 ]
