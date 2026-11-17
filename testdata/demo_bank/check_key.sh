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
# Read-only: every query runs inside BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY.
# (The runtime case r01 needs a writer and is exercised by concurrent_update.sh,
# not by this static check.)
set -euo pipefail
unset PGHOSTADDR PGSERVICE PGSERVICEFILE PGHOST PGPORT PGDATABASE PGUSER PGPASSWORD
export PGOPTIONS="${PGOPTIONS:-} -c client_min_messages=warning"
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

ok()  { PASS=$((PASS + 1)); printf '  ok    %s\n' "$1"; }
bad() { FAIL=$((FAIL + 1)); printf '  FAIL  %s\n' "$1"; }

assert_eq() { # label, actual, expected
  if [ "$2" = "$3" ]; then
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

table_exists() { # url, table -> 1 or 0 (ordinary or partitioned table)
  scalar "$1" "SELECT count(*) FROM pg_class c
    JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = 'demo_bank' AND c.relname = '$2'
      AND c.relkind IN ('r', 'p')"
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

hash_of() { digest "$@" | cut -d'|' -f2; }

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
printf '  old %s\n' "$(raw_scalar "$OLD_URL" "SELECT current_setting('server_version')
    || ', ' || pg_encoding_to_char(encoding) || ', collate ' || datcollate
  FROM pg_database WHERE datname = current_database()")"
printf '  new %s\n' "$(raw_scalar "$NEW_URL" "SELECT current_setting('server_version')
    || ', ' || pg_encoding_to_char(encoding) || ', collate ' || datcollate
  FROM pg_database WHERE datname = current_database()")"

# ---------------------------------------------------------------------------
# Must-report entries. Every expected value comes from the key, so a key edit
# cannot drift away from this check.
# ---------------------------------------------------------------------------
echo "Differences Dmux must report:"

# m01: 12 deleted transaction rows.
mark_handled m01
old_count="$(scalar "$OLD_URL" "SELECT count(*) FROM demo_bank.transactions")"
new_count="$(scalar "$NEW_URL" "SELECT count(*) FROM demo_bank.transactions")"
assert_eq "m01 old transactions count" "$old_count" "$(keyval m01 'v["old_count"]')"
assert_eq "m01 new transactions count" "$new_count" "$(keyval m01 'v["new_count"]')"
assert_eq "m01 count difference" "$((old_count - new_count))" "$(keyval m01 'v["missing_count"]')"
m01_ids="$(keyval m01 '",".join(str(k) for k in v["missing_keys"])')"
old_present="$(scalar "$OLD_URL" "SELECT count(*) FROM demo_bank.transactions WHERE transaction_id IN ($m01_ids)")"
new_present="$(scalar "$NEW_URL" "SELECT count(*) FROM demo_bank.transactions WHERE transaction_id IN ($m01_ids)")"
assert_eq "m01 all 12 keys exist on old" "$old_present" "$(keyval m01 'len(v["missing_keys"])')"
assert_eq "m01 all 12 keys are absent on new" "$new_present" "0"

# m02: 3 duplicate rows in the keyless audit_log.
mark_handled m02
old_count="$(scalar "$OLD_URL" "SELECT count(*) FROM demo_bank.audit_log")"
new_count="$(scalar "$NEW_URL" "SELECT count(*) FROM demo_bank.audit_log")"
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
  old_n="$(scalar "$OLD_URL" "SELECT count(*) FROM demo_bank.audit_log WHERE $pred")"
  new_n="$(scalar "$NEW_URL" "SELECT count(*) FROM demo_bank.audit_log WHERE $pred")"
  assert_eq "m02 duplicate row $m02_rows ($detail): once on old" "$old_n" "1"
  assert_eq "m02 duplicate row $m02_rows ($detail): twice on new" "$new_n" "2"
done < "$TMP/m02.tsv"
assert_eq "m02 number of extra rows" "$m02_rows" "$(keyval m02 'len(v["extra_rows"])')"

# m03: balance changed by exactly 0.01, and the control total follows.
mark_handled m03
m03_key="$(keyval m03 'v["key"]["account_id"]')"
old_val="$(scalar "$OLD_URL" "SELECT balance::text FROM demo_bank.accounts WHERE account_id = $m03_key")"
new_val="$(scalar "$NEW_URL" "SELECT balance::text FROM demo_bank.accounts WHERE account_id = $m03_key")"
assert_eq "m03 account $m03_key old balance" "$old_val" "$(keyval m03 'v["old"]')"
assert_eq "m03 account $m03_key new balance" "$new_val" "$(keyval m03 'v["new"]')"
assert_eq "m03 balance difference" \
  "$(python3 -c 'import sys; print("%.4f" % (float(sys.argv[1]) - float(sys.argv[2])))' "$new_val" "$old_val")" \
  "$(keyval m03 'v["difference"]')"
old_sum="$(scalar "$OLD_URL" "SELECT sum(balance)::text FROM demo_bank.accounts")"
new_sum="$(scalar "$NEW_URL" "SELECT sum(balance)::text FROM demo_bank.accounts")"
assert_eq "m03 old control total" "$old_sum" "$(keyval m03 'v["control_total"]["old"]')"
assert_eq "m03 new control total" "$new_sum" "$(keyval m03 'v["control_total"]["new"]')"

# m04: one character changed.
mark_handled m04
m04_key="$(keyval m04 'v["key"]["transaction_id"]')"
old_repr="$(scalar "$OLD_URL" "SELECT CASE WHEN narration IS NULL THEN '<NULL>' ELSE '[' || narration || ']' END FROM demo_bank.transactions WHERE transaction_id = $m04_key")"
new_repr="$(scalar "$NEW_URL" "SELECT CASE WHEN narration IS NULL THEN '<NULL>' ELSE '[' || narration || ']' END FROM demo_bank.transactions WHERE transaction_id = $m04_key")"
assert_eq "m04 transaction $m04_key old value" "$old_repr" "$(keyval m04 '"[%s]" % v["old"] if v["old"] is not None else "<NULL>"')"
assert_eq "m04 transaction $m04_key new value" "$new_repr" "$(keyval m04 '"[%s]" % v["new"] if v["new"] is not None else "<NULL>"')"

# m05: empty text became NULL.
mark_handled m05
m05_key="$(keyval m05 'v["key"]["transaction_id"]')"
old_repr="$(scalar "$OLD_URL" "SELECT CASE WHEN narration IS NULL THEN '<NULL>' ELSE '[' || narration || ']' END FROM demo_bank.transactions WHERE transaction_id = $m05_key")"
new_repr="$(scalar "$NEW_URL" "SELECT CASE WHEN narration IS NULL THEN '<NULL>' ELSE '[' || narration || ']' END FROM demo_bank.transactions WHERE transaction_id = $m05_key")"
assert_eq "m05 transaction $m05_key old value" "$old_repr" "$(keyval m05 '"[%s]" % v["old"] if v["old"] is not None else "<NULL>"')"
assert_eq "m05 transaction $m05_key new value" "$new_repr" "$(keyval m05 '"[%s]" % v["new"] if v["new"] is not None else "<NULL>"')"

# m06: time of day dropped from a timestamp.
mark_handled m06
m06_key="$(keyval m06 'v["key"]["transaction_id"]')"
old_ts="$(scalar "$OLD_URL" "SELECT value_timestamp::text FROM demo_bank.transactions WHERE transaction_id = $m06_key")"
new_ts="$(scalar "$NEW_URL" "SELECT value_timestamp::text FROM demo_bank.transactions WHERE transaction_id = $m06_key")"
assert_eq "m06 transaction $m06_key old timestamp" "$old_ts" "$(keyval m06 'v["old"]')"
assert_eq "m06 transaction $m06_key new timestamp" "$new_ts" "$(keyval m06 'v["new"]')"

# m07/m08: one column dropped, one column added.
mark_handled m07
mark_handled m08
m07_type="$(scalar "$OLD_URL" "SELECT format_type(atttypid, atttypmod) FROM pg_attribute
  WHERE attrelid = 'demo_bank.user_roles'::regclass AND attname = 'granted_by' AND NOT attisdropped")"
assert_eq "m07 user_roles.granted_by exists on old with the keyed type" "$m07_type" "$(keyval m07 'v["old_type"]')"
assert_eq "m07 user_roles.granted_by column count on new" \
  "$(scalar "$NEW_URL" "SELECT count(*) FROM pg_attribute WHERE attrelid = 'demo_bank.user_roles'::regclass AND attname = 'granted_by' AND NOT attisdropped")" "0"
assert_eq "m08 loans.settlement_ref column count on old" \
  "$(scalar "$OLD_URL" "SELECT count(*) FROM pg_attribute WHERE attrelid = 'demo_bank.loans'::regclass AND attname = 'settlement_ref' AND NOT attisdropped")" "0"
m08_type="$(scalar "$NEW_URL" "SELECT format_type(atttypid, atttypmod) FROM pg_attribute
  WHERE attrelid = 'demo_bank.loans'::regclass AND attname = 'settlement_ref' AND NOT attisdropped")"
assert_eq "m08 loans.settlement_ref exists on new with the keyed type" "$m08_type" "$(keyval m08 'v["new_type"]')"

# m09: one table renamed. The rows must be the same data under the new name.
mark_handled m09
m09_old="$(keyval m09 'v["old_name"]')"
m09_new="$(keyval m09 'v["new_name"]')"
assert_eq "m09 $m09_old exists on old" "$(table_exists "$OLD_URL" "$m09_old")" "1"
assert_eq "m09 $m09_old is gone on new" "$(table_exists "$NEW_URL" "$m09_old")" "0"
assert_eq "m09 $m09_new exists on new" "$(table_exists "$NEW_URL" "$m09_new")" "1"
assert_eq "m09 $m09_new row count on new" "$(scalar "$NEW_URL" "SELECT count(*) FROM demo_bank.$m09_new")" "$(keyval m09 'v["rows"]')"
m09_cols="$(columns_of "$OLD_URL" "$m09_old")"
assert_eq "m09 $m09_old -> $m09_new data digest" \
  "$(digest "$NEW_URL" "$m09_new" "$(col_exprs "$NEW_URL" "$m09_new" "$m09_cols")")" \
  "$(digest "$OLD_URL" "$m09_old" "$(col_exprs "$OLD_URL" "$m09_old" "$m09_cols")")"

# m10: one column renamed. The values must be the same data under the new name.
mark_handled m10
m10_old="$(keyval m10 'v["old_name"]')"
m10_new="$(keyval m10 'v["new_name"]')"
assert_eq "m10 fees.$m10_old is gone on new" \
  "$(scalar "$NEW_URL" "SELECT count(*) FROM pg_attribute WHERE attrelid = 'demo_bank.fees'::regclass AND attname = '$m10_old' AND NOT attisdropped")" "0"
assert_eq "m10 fees.$m10_new exists on new" \
  "$(scalar "$NEW_URL" "SELECT count(*) FROM pg_attribute WHERE attrelid = 'demo_bank.fees'::regclass AND attname = '$m10_new' AND NOT attisdropped")" "1"
assert_eq "m10 fees row count on new" "$(scalar "$NEW_URL" "SELECT count(*) FROM demo_bank.fees")" "$(keyval m10 'v["rows"]')"
assert_eq "m10 fees.$m10_old -> $m10_new value digest" \
  "$(scalar "$OLD_URL" "SELECT md5(string_agg(fee_id || '|' || fee_name, ',' ORDER BY fee_id)) FROM demo_bank.fees")" \
  "$(scalar "$NEW_URL" "SELECT md5(string_agg(fee_id || '|' || fee_label, ',' ORDER BY fee_id)) FROM demo_bank.fees")"

# m11: varchar widened (amber).
mark_handled m11
m11_old="$(scalar "$OLD_URL" "SELECT format_type(atttypid, atttypmod) FROM pg_attribute
  WHERE attrelid = 'demo_bank.branches'::regclass AND attname = 'city'")"
m11_new="$(scalar "$NEW_URL" "SELECT format_type(atttypid, atttypmod) FROM pg_attribute
  WHERE attrelid = 'demo_bank.branches'::regclass AND attname = 'city'")"
assert_eq "m11 branches.city old type" "$m11_old" "$(keyval m11 'v["old_type"]')"
assert_eq "m11 branches.city new type" "$m11_new" "$(keyval m11 'v["new_type"]')"

# m12: sequence left behind its old value.
mark_handled m12
m12_old="$(scalar "$OLD_URL" "SELECT last_value FROM demo_bank.seq_gl_entry")"
m12_new="$(scalar "$NEW_URL" "SELECT last_value FROM demo_bank.seq_gl_entry")"
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
assert_eq "m13 $m13_index exists on old" \
  "$(scalar "$OLD_URL" "SELECT count(*) FROM pg_indexes WHERE schemaname = 'demo_bank' AND indexname = '$m13_index'")" "1"
assert_eq "m13 $m13_index is gone on new" \
  "$(scalar "$NEW_URL" "SELECT count(*) FROM pg_indexes WHERE schemaname = 'demo_bank' AND indexname = '$m13_index'")" "0"

# m14: one constraint not validated.
mark_handled m14
m14_con="$(keyval m14 'v["constraint"]')"
m14_old="$(scalar "$OLD_URL" "SELECT convalidated FROM pg_constraint WHERE conname = '$m14_con' AND conrelid = 'demo_bank.transactions'::regclass")"
m14_new="$(scalar "$NEW_URL" "SELECT convalidated FROM pg_constraint WHERE conname = '$m14_con' AND conrelid = 'demo_bank.transactions'::regclass")"
assert_eq "m14 $m14_con validated on old" "$m14_old" "t"
assert_eq "m14 $m14_con not validated on new" "$m14_new" "f"
assert_eq "m14 $m14_con exists on both sides" \
  "$(scalar "$NEW_URL" "SELECT contype::text FROM pg_constraint WHERE conname = '$m14_con' AND conrelid = 'demo_bank.transactions'::regclass")" \
  "$(keyval m14 'v["constraint_type"]')"

# m15: the large object missing.
mark_handled m15
m15_oid="$(keyval m15 'v["large_object_oid"]')"
assert_eq "m15 large object $m15_oid exists on old" \
  "$(scalar "$OLD_URL" "SELECT count(*) FROM pg_largeobject_metadata WHERE oid = $m15_oid")" "1"
assert_eq "m15 large object $m15_oid is gone on new" \
  "$(scalar "$NEW_URL" "SELECT count(*) FROM pg_largeobject_metadata WHERE oid = $m15_oid")" "0"
assert_eq "m15 large object metadata count on new" \
  "$(scalar "$NEW_URL" "SELECT count(*) FROM pg_largeobject_metadata")" "$(keyval m15 'v["new_count"]')"

# ---------------------------------------------------------------------------
# Stay-quiet entries: the situation must really be present, and the data agree.
# ---------------------------------------------------------------------------
echo "Cases where Dmux must stay quiet:"

# q01: LATIN1 old, UTF-8 new, accented names.
mark_handled q01
assert_eq "q01 old database encoding" \
  "$(scalar "$OLD_URL" "SELECT pg_encoding_to_char(encoding) FROM pg_database WHERE datname = current_database()")" \
  "$(keyval q01 'v["old_encoding"]')"
assert_eq "q01 new database encoding" \
  "$(scalar "$NEW_URL" "SELECT pg_encoding_to_char(encoding) FROM pg_database WHERE datname = current_database()")" \
  "$(keyval q01 'v["new_encoding"]')"
q01_key="$(keyval q01 'v["sample_key"]["branch_id"]')"
assert_eq "q01 accented value: identical UTF-8 bytes on the old side" \
  "$(scalar "$OLD_URL" "SELECT encode(convert_to(branch_name, 'UTF8'), 'hex') FROM demo_bank.branches WHERE branch_id = $q01_key")" \
  "$(keyval q01 'v["utf8_hex"]')"
assert_eq "q01 accented value: identical UTF-8 bytes on the new side" \
  "$(scalar "$NEW_URL" "SELECT encode(convert_to(branch_name, 'UTF8'), 'hex') FROM demo_bank.branches WHERE branch_id = $q01_key")" \
  "$(keyval q01 'v["utf8_hex"]')"
assert_eq "q01 decoded accented value" \
  "$(scalar "$NEW_URL" "SELECT branch_name FROM demo_bank.branches WHERE branch_id = $q01_key")" \
  "$(keyval q01 'v["sample_value"]')"

# q02: the two servers sort text differently.
mark_handled q02
assert_eq "q02 old collation" \
  "$(scalar "$OLD_URL" "SELECT datcollate FROM pg_database WHERE datname = current_database()")" \
  "$(keyval q02 'v["old_collate"]')"
assert_eq "q02 new collation" \
  "$(scalar "$NEW_URL" "SELECT datcollate FROM pg_database WHERE datname = current_database()")" \
  "$(keyval q02 'v["new_collate"]')"
q02_sql="SELECT string_agg(x, ',' ORDER BY x) FROM (VALUES ('z'), (chr(233))) AS v(x)"
assert_eq "q02 old sort order" "$(scalar "$OLD_URL" "$q02_sql")" "$(keyval q02 '",".join(v["old_order"])')"
assert_eq "q02 new sort order" "$(scalar "$NEW_URL" "$q02_sql")" "$(keyval q02 '",".join(v["new_order"])')"

# q03: floats printed differently by the two servers, identical from bytes.
mark_handled q03
q03_key="$(keyval q03 'v["key"]["product_id"]')"
assert_eq "q03 old default extra_float_digits" \
  "$(raw_scalar "$OLD_URL" "SHOW extra_float_digits")" "$(keyval q03 'v["old_default_extra_float_digits"]')"
assert_eq "q03 new default extra_float_digits" \
  "$(raw_scalar "$NEW_URL" "SHOW extra_float_digits")" "$(keyval q03 'v["new_default_extra_float_digits"]')"
assert_eq "q03 canonical printed value on old" \
  "$(scalar "$OLD_URL" "SELECT nominal_rate::text FROM demo_bank.products WHERE product_id = $q03_key")" \
  "$(keyval q03 'v["canonical_value"]')"
assert_eq "q03 canonical printed value on new" \
  "$(scalar "$NEW_URL" "SELECT nominal_rate::text FROM demo_bank.products WHERE product_id = $q03_key")" \
  "$(keyval q03 'v["canonical_value"]')"
assert_eq "q03 9.6 high-precision printed form" \
  "$(raw_scalar "$OLD_URL" "SET extra_float_digits = 2; SELECT nominal_rate::text FROM demo_bank.products WHERE product_id = $q03_key")" \
  "$(keyval q03 'v["old_high_precision"]')"
assert_eq "q03 17 high-precision printed form" \
  "$(raw_scalar "$NEW_URL" "SET extra_float_digits = 2; SELECT nominal_rate::text FROM demo_bank.products WHERE product_id = $q03_key")" \
  "$(keyval q03 'v["new_high_precision"]')"
assert_eq "q03 float bytes are equal" \
  "$(scalar "$OLD_URL" "SELECT encode(float4send(nominal_rate), 'hex') FROM demo_bank.products WHERE product_id = $q03_key")" \
  "$(scalar "$NEW_URL" "SELECT encode(float4send(nominal_rate), 'hex') FROM demo_bank.products WHERE product_id = $q03_key")"

# q04: a table converted to a partitioned table.
mark_handled q04
assert_eq "q04 balances_daily is an ordinary table on old" \
  "$(scalar "$OLD_URL" "SELECT c.relkind::text FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = 'demo_bank' AND c.relname = 'balances_daily'")" \
  "$(keyval q04 'v["old_relkind"]')"
assert_eq "q04 balances_daily is a partitioned table on new" \
  "$(scalar "$NEW_URL" "SELECT c.relkind::text FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = 'demo_bank' AND c.relname = 'balances_daily'")" \
  "$(keyval q04 'v["new_relkind"]')"
q04_parts="$(scalar "$NEW_URL" "SELECT string_agg(c.relname, ',' ORDER BY c.relname)
  FROM pg_inherits i JOIN pg_class c ON c.oid = i.inhrelid
  WHERE i.inhparent = 'demo_bank.balances_daily'::regclass")"
assert_eq "q04 new partition set" "$q04_parts" "$(keyval q04 '",".join(v["new_partitions"])')"
assert_eq "q04 old row count" "$(scalar "$OLD_URL" "SELECT count(*) FROM demo_bank.balances_daily")" "$(keyval q04 'v["rows"]')"
assert_eq "q04 new row count through the parent" "$(scalar "$NEW_URL" "SELECT count(*) FROM demo_bank.balances_daily")" "$(keyval q04 'v["rows"]')"

# q05: columns in a different order.
mark_handled q05
assert_eq "q05 old column order" "$(columns_of "$OLD_URL" holidays)" "$(keyval q05 '",".join(v["old_column_order"])')"
assert_eq "q05 new column order" "$(columns_of "$NEW_URL" holidays)" "$(keyval q05 '",".join(v["new_column_order"])')"
assert_eq "q05 row count" "$(scalar "$NEW_URL" "SELECT count(*) FROM demo_bank.holidays")" "$(keyval q05 'v["rows"]')"

# q06: char(1) moved to varchar(1) - amber type change, values pass.
mark_handled q06
q06_old="$(scalar "$OLD_URL" "SELECT format_type(atttypid, atttypmod) FROM pg_attribute
  WHERE attrelid = 'demo_bank.kyc_checks'::regclass AND attname = 'result'")"
q06_new="$(scalar "$NEW_URL" "SELECT format_type(atttypid, atttypmod) FROM pg_attribute
  WHERE attrelid = 'demo_bank.kyc_checks'::regclass AND attname = 'result'")"
assert_eq "q06 kyc_checks.result old type" "$q06_old" "$(keyval q06 'v["old_type"]')"
assert_eq "q06 kyc_checks.result new type" "$q06_new" "$(keyval q06 'v["new_type"]')"
assert_eq "q06 row count" "$(scalar "$NEW_URL" "SELECT count(*) FROM demo_bank.kyc_checks")" "$(keyval q06 'v["rows"]')"

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

tables_of "$OLD_URL" > "$TMP/old.tables"
tables_of "$NEW_URL" > "$TMP/new.tables"
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
  if [ "$(table_exists "$NEW_URL" "$new_tbl")" != "1" ]; then
    bad "$tbl: no counterpart $new_tbl on new"
    continue
  fi
  old_cols="$(columns_of "$OLD_URL" "$tbl")"
  new_cols="$(columns_of "$NEW_URL" "$new_tbl")"
  common="$(common_columns "$old_cols" "$new_cols")"
  if [ -z "$common" ]; then
    bad "$tbl: no common columns with $new_tbl"
    continue
  fi
  d_old="$(digest "$OLD_URL" "$tbl" "$(col_exprs "$OLD_URL" "$tbl" "$common")")"
  d_new="$(digest "$NEW_URL" "$new_tbl" "$(col_exprs "$NEW_URL" "$new_tbl" "$common")")"
  case "$IDENTICAL_SKIP" in
    *" $tbl "*)
      # The planted changes are checked above; prove nothing else moved here.
      case "$tbl" in
        accounts)
          acc_cols="$(without_column "$old_cols" balance)"
          assert_eq "$tbl: same row count" "${d_new%%|*}" "${d_old%%|*}"
          assert_eq "$tbl: identical outside balance" \
            "$(digest "$NEW_URL" accounts "$(col_exprs "$NEW_URL" accounts "$acc_cols")")" \
            "$(digest "$OLD_URL" accounts "$(col_exprs "$OLD_URL" accounts "$acc_cols")")"
          ;;
        transactions)
          assert_eq "$tbl: identical outside the changed and deleted rows" \
            "$(hash_of "$NEW_URL" transactions "$(col_exprs "$NEW_URL" transactions "$old_cols")" "transaction_id NOT IN ($m04_key, $m05_key, $m06_key) AND transaction_id NOT IN ($m01_ids)")" \
            "$(hash_of "$OLD_URL" transactions "$(col_exprs "$OLD_URL" transactions "$old_cols")" "transaction_id NOT IN ($m04_key, $m05_key, $m06_key) AND transaction_id NOT IN ($m01_ids)")"
          ;;
        audit_log)
          assert_eq "$tbl: identical outside the 3 duplicated rows" \
            "$(hash_of "$NEW_URL" audit_log "$(col_exprs "$NEW_URL" audit_log "$old_cols")" "COALESCE(detail, '<null>') NOT IN ('detail 1', 'detail 2', 'detail 3')")" \
            "$(hash_of "$OLD_URL" audit_log "$(col_exprs "$OLD_URL" audit_log "$old_cols")" "COALESCE(detail, '<null>') NOT IN ('detail 1', 'detail 2', 'detail 3')")"
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
assert_eq "r01 target row exists on new" \
  "$(scalar "$NEW_URL" "SELECT count(*) FROM demo_bank.notes WHERE note_id = $r01_key")" "1"
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
