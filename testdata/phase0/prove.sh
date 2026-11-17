#!/usr/bin/env bash
# Phase 0 proof: does the Dmux fingerprint behave as PLAN.md section 3 says?
#
# Usage:  testdata/phase0/prove.sh [old port] [new port]
#
# T001/T002: the original 15 checks on accounts/audit_log.
# T004: extended with the 13 cases from the T003 SQL review section 6
# (timestamptz, money, json/jsonb, xml, arrays, ranges, interval, numeric scale
# change, NULL bytea, empty table, added/dropped column, collations, SQL_ASCII)
# plus the section 7 overflow-bound probes. The reference templates now carry
# the canonical settings block from T003 section 4.
#
# For cases 6.12/6.13 the harness creates two extra databases on the OLD server
# (bank_c: UTF8/C, bank_ascii: SQL_ASCII/C), drops them again at the end, and
# compares them against the new server's bank. They are recreated on every run.
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

# With no settings, it uses the mock databases from testdata/compose.yml.
# Two optional arguments pick other local ports, for the version matrix:
#   testdata/phase0/prove.sh 5435 5437     # old = 9.4, new = 18
OLD_URL="${OLD_URL:-postgres://dmux_dev:dmux_dev_only@127.0.0.1:${1:-5433}/bank}"
NEW_URL="${NEW_URL:-postgres://dmux_dev:dmux_dev_only@127.0.0.1:${2:-5434}/bank}"
HERE="$(cd "$(dirname "$0")" && pwd)"
# T010: strict local-only URL guard, shared with perf.sh (threat model in guard.sh).
# shellcheck disable=SC1091  # guard.sh is linted separately; $HERE is runtime-relative
. "$HERE/guard.sh"
for u in "$OLD_URL" "$NEW_URL"; do
  if ! dmux_guard_url "$u"; then
    echo "refusing: prove.sh only runs against local mock databases" >&2; exit 1
  fi
done
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
PASS=0; FAIL=0

q()  { psql -X -q -v ON_ERROR_STOP=1 "$1" -c "$2" >/dev/null; }
# fp: bucket summaries only. An EMPTY result is legal (empty table, case 6.10);
# a query ERROR still fails the pipeline (pipefail + ON_ERROR_STOP), so a broken
# or missing table can never masquerade as "empty" (P5).
fp() { # url, file [, extra PGOPTIONS]
  PGOPTIONS="$PGOPTIONS ${3:-}" psql -X -q -A -t -v ON_ERROR_STOP=1 "$1" -f "$HERE/$2" \
    | { grep -E '^[0-9]+\|' || true; }
}

check() { # name, expectation (same|differ), file [, old url, new url]
  local name="$1" want="$2" file="$3" ou="${4:-$OLD_URL}" nu="${5:-$NEW_URL}" got
  fp "$ou" "$file" "${FP_EXTRA_OLD:-}" > "$TMP/old"; fp "$nu" "$file" "${FP_EXTRA_NEW:-}" > "$TMP/new"
  if cmp -s "$TMP/old" "$TMP/new"; then got=same; else got=differ; fi
  # T011 (display only, verdict still comes from cmp above): count the distinct
  # buckets appearing on EITHER side of the diff, so an empty old side no longer
  # prints "0 bucket(s)" when the new side has rows (Opus T008 gate note 3).
  local buckets; buckets=$(diff "$TMP/old" "$TMP/new" | grep -E '^[<>]' | sed 's/^[<>] //' | cut -d'|' -f1 | sort -u | wc -l || true)
  if [[ "$got" = "$want" ]]; then PASS=$((PASS+1)); printf '  ok    %-52s (%s' "$name" "$got"
    [[ "$got" = differ ]] && printf ', %s bucket(s)' "$buckets"; printf ')\n'
  else FAIL=$((FAIL+1)); printf '  FAIL  %-52s (wanted %s, got %s)\n' "$name" "$want" "$got"; fi
}

mut() { # name, file, change SQL, undo SQL [, new url, old url]
  local nu="${5:-$NEW_URL}" ou="${6:-$OLD_URL}"
  q "$nu" "$3"; check "$1" differ "$2" "$ou" "$nu"; q "$nu" "$4"
}

assert_both() { # name, sql, expected value [, old url, new url] — one check, both sides
  local name="$1" sql="$2" want="$3" ou="${4:-$OLD_URL}" nu="${5:-$NEW_URL}" go gn
  go=$(psql -X -q -A -t -v ON_ERROR_STOP=1 "$ou" -c "$sql")
  gn=$(psql -X -q -A -t -v ON_ERROR_STOP=1 "$nu" -c "$sql")
  if [[ "$go" = "$want" ]] && [[ "$gn" = "$want" ]]; then
    PASS=$((PASS+1)); printf '  ok    %-52s (both sides: %s)\n' "$name" "$want"
  else FAIL=$((FAIL+1)); printf '  FAIL  %-52s (wanted %s, got old=%s new=%s)\n' "$name" "$want" "$go" "$gn"; fi
}

assert_one() { # name, url, sql, expected value
  local name="$1" u="$2" sql="$3" want="$4" got
  got=$(psql -X -q -A -t -v ON_ERROR_STOP=1 "$u" -c "$sql")
  if [[ "$got" = "$want" ]]; then
    PASS=$((PASS+1)); printf '  ok    %-52s (%s)\n' "$name" "$got"
  else FAIL=$((FAIL+1)); printf '  FAIL  %-52s (wanted %s, got %s)\n' "$name" "$want" "$got"; fi
}

check_error() { # name, file [, url] — the fingerprint query MUST fail (P5)
  local name="$1" file="$2" u="${3:-$NEW_URL}"
  if fp "$u" "$file" > "$TMP/err" 2> "$TMP/errm"; then
    FAIL=$((FAIL+1)); printf '  FAIL  %-52s (wanted query error, got success)\n' "$name"
  else
    PASS=$((PASS+1)); printf '  ok    %-52s (query error detected)\n' "$name"
  fi
}

assert_error() { # name, url, sql — the query MUST fail (P19: never a silent PASS)
  local name="$1" u="$2" sql="$3"
  if psql -X -q -A -t -v ON_ERROR_STOP=1 "$u" -c "$sql" > "$TMP/err" 2> "$TMP/errm"; then
    FAIL=$((FAIL+1)); printf '  FAIL  %-52s (wanted query error, got success)\n' "$name"
  else
    PASS=$((PASS+1)); printf '  ok    %-52s (query error detected)\n' "$name"
  fi
}

cols() { # url, table — catalog column list, ordered by position (never by text)
  psql -X -q -A -t -v ON_ERROR_STOP=1 "$1" -c "SELECT attname || ':' || format_type(atttypid, atttypmod)
    FROM pg_attribute WHERE attrelid = 'phase0.$2'::regclass AND attnum > 0 AND NOT attisdropped
    ORDER BY attnum"
}

schema_check() { # name, table, marker, side (old|new) [, old url, new url]
  # The catalog lists must differ, and the marker must appear on exactly the
  # given side: schema diff has to report the change (P3), the value hash alone
  # is never the verdict.
  local name="$1" tbl="$2" marker="$3" side="$4" ou="${5:-$OLD_URL}" nu="${6:-$NEW_URL}" co cn ok=1
  co=$(cols "$ou" "$tbl"); cn=$(cols "$nu" "$tbl")
  if [[ "$co" = "$cn" ]]; then ok=0; fi
  if [[ "$side" = new ]]; then
    if ! printf '%s\n' "$cn" | grep -qF "$marker"; then ok=0; fi
    if printf '%s\n' "$co" | grep -qF "$marker"; then ok=0; fi
  else
    if ! printf '%s\n' "$co" | grep -qF "$marker"; then ok=0; fi
    if printf '%s\n' "$cn" | grep -qF "$marker"; then ok=0; fi
  fi
  if [[ "$ok" = 1 ]]; then
    PASS=$((PASS+1)); printf '  ok    %-52s (schema diff reports it)\n' "$name"
  else FAIL=$((FAIL+1)); printf '  FAIL  %-52s (schema change not detected as expected)\n' "$name"; fi
}

schema_old='
DROP SCHEMA IF EXISTS phase0 CASCADE; CREATE SCHEMA phase0;
CREATE TABLE phase0.accounts (
  acct_id varchar(11), bank_code varchar(8), holder_name varchar(80), branch_code char(8),
  balance numeric(20,4), rate float8, opened_at timestamp(0),
  doc_blob bytea, remarks varchar(60), PRIMARY KEY (acct_id, bank_code));
CREATE TABLE phase0.audit_log (event_code varchar(5), event_text varchar(40));'
# New side on purpose: different column order, char(8) became varchar(8).
schema_new='
DROP SCHEMA IF EXISTS phase0 CASCADE; CREATE SCHEMA phase0;
CREATE TABLE phase0.accounts (
  remarks varchar(60), doc_blob bytea, opened_at timestamp(0), rate float8,
  balance numeric(20,4), branch_code varchar(8), holder_name varchar(80),
  bank_code varchar(8), acct_id varchar(11), PRIMARY KEY (acct_id, bank_code));
CREATE TABLE phase0.audit_log (event_code varchar(5), event_text varchar(40));'

# 20,000 accounts. Names carry accents; some remarks are NULL, some empty.
# chr(233) and friends give the same accented letter in LATIN1 and in UTF-8, on every
# Postgres version. (U&'' escapes fail on a LATIN1 server before Postgres 13.)
load_accounts() { # url, ORDER BY direction
  q "$1" "INSERT INTO phase0.accounts (acct_id, bank_code, holder_name, branch_code, balance, rate, opened_at, doc_blob, remarks)
  SELECT lpad(g::text, 11, '0'), 'BANK01',
         CASE g % 3 WHEN 0 THEN 'Jos' || chr(233) || ' Mu' || chr(241) || 'oz ' WHEN 1 THEN 'Zo' || chr(235) || ' ' || chr(197) || 'str' || chr(246) || 'm ' ELSE 'Plain Name ' END || g,
         'S' || (g % 500), (g * 1234.5678)::numeric(20,4), g / 7.0,
         timestamp '2001-01-01 00:00:00' + (g || ' minutes')::interval,
         decode(md5(g::text), 'hex'),
         CASE g % 4 WHEN 0 THEN NULL WHEN 1 THEN '' ELSE 'remark ' || g END
  FROM generate_series(1, 20000) g ORDER BY g $2;"
}
load_audit() { q "$1" "INSERT INTO phase0.audit_log
  SELECT 'E' || (g % 50), 'event ' || (g % 200) FROM generate_series(1, 5000) g ORDER BY g $2;"; }

echo "Servers:"
psql -X -A -t "$OLD_URL" -c "select '  old: ' || current_setting('server_version') || ', ' || pg_encoding_to_char(encoding) || ', collate ' || datcollate from pg_database where datname = current_database()"
psql -X -A -t "$NEW_URL" -c "select '  new: ' || current_setting('server_version') || ', ' || pg_encoding_to_char(encoding) || ', collate ' || datcollate from pg_database where datname = current_database()"

q "$OLD_URL" "$schema_old"; q "$NEW_URL" "$schema_new"
load_accounts "$OLD_URL" ASC;  load_accounts "$NEW_URL" DESC   # opposite physical order
load_audit    "$OLD_URL" ASC;  load_audit    "$NEW_URL" DESC

echo "Must stay quiet (identical data):"
check "same data, other row order, column order, encoding" same fingerprint.sql
check "char(8) on old side, varchar(8) on new side"          same fingerprint.sql
check "table with no key, duplicates on both sides"          same fingerprint_nokey.sql

echo "Must be reported (one change each, then undone):"
mut "one balance changed by 0.0001" fingerprint.sql \
  "UPDATE phase0.accounts SET balance = balance + 0.0001 WHERE acct_id = '00000012345'" \
  "UPDATE phase0.accounts SET balance = balance - 0.0001 WHERE acct_id = '00000012345'"
mut "one character changed in a name" fingerprint.sql \
  "UPDATE phase0.accounts SET holder_name = 'Plain Name 5X' WHERE acct_id = '00000000005'" \
  "UPDATE phase0.accounts SET holder_name = 'Plain Name 5' WHERE acct_id = '00000000005'"
mut "empty text turned into NULL" fingerprint.sql \
  "UPDATE phase0.accounts SET remarks = NULL WHERE acct_id = '00000000001'" \
  "UPDATE phase0.accounts SET remarks = '' WHERE acct_id = '00000000001'"
mut "time of day dropped from a timestamp" fingerprint.sql \
  "UPDATE phase0.accounts SET opened_at = date_trunc('day', opened_at) WHERE acct_id = '00000000777'" \
  "UPDATE phase0.accounts SET opened_at = timestamp '2001-01-01' + interval '777 minutes' WHERE acct_id = '00000000777'"
mut "float changed in the last digit" fingerprint.sql \
  "UPDATE phase0.accounts SET rate = 3.0000000000000004 WHERE acct_id = '00000000021'" \
  "UPDATE phase0.accounts SET rate = 21 / 7.0 WHERE acct_id = '00000000021'"
mut "one byte changed in binary data" fingerprint.sql \
  "UPDATE phase0.accounts SET doc_blob = decode(md5('x'), 'hex') WHERE acct_id = '00000000042'" \
  "UPDATE phase0.accounts SET doc_blob = decode(md5('42'), 'hex') WHERE acct_id = '00000000042'"
mut "two values swapped between columns of one row" fingerprint.sql \
  "UPDATE phase0.accounts SET remarks = branch_code, branch_code = 'remark 3' WHERE acct_id = '00000000003'" \
  "UPDATE phase0.accounts SET remarks = 'remark 3', branch_code = 'S3' WHERE acct_id = '00000000003'"
mut "one row deleted" fingerprint.sql \
  "CREATE TABLE phase0.keep AS SELECT * FROM phase0.accounts WHERE acct_id = '00000019999'; DELETE FROM phase0.accounts WHERE acct_id = '00000019999'" \
  "INSERT INTO phase0.accounts SELECT * FROM phase0.keep; DROP TABLE phase0.keep"
mut "no-key table: one of several duplicate rows deleted" fingerprint_nokey.sql \
  "DELETE FROM phase0.audit_log WHERE ctid = (SELECT min(ctid) FROM phase0.audit_log WHERE event_code = 'E7' AND event_text = 'event 7')" \
  "INSERT INTO phase0.audit_log VALUES ('E7', 'event 7')"
mut "no-key table: one duplicate pair added (XOR would miss it)" fingerprint_nokey.sql \
  "INSERT INTO phase0.audit_log VALUES ('E9', 'event 9'), ('E9', 'event 9')" \
  "DELETE FROM phase0.audit_log WHERE ctid IN (SELECT ctid FROM phase0.audit_log WHERE event_code = 'E9' AND event_text = 'event 9' ORDER BY ctid DESC LIMIT 2)"

echo "After undoing every change:"
check "accounts match again" same fingerprint.sql
check "no-key table matches again" same fingerprint_nokey.sql

# ---------------------------------------------------------------------------
# T004 extended cases (T003 SQL review section 6). Values are exact per spec.
# Three tables deliberately differ in TYPE between the sides: tz_mix
# (timestamp -> timestamptz), json_mix (json -> jsonb), num_test
# (numeric(20,4) -> numeric(20,6)). Everything else loads identically.
# ---------------------------------------------------------------------------
schema_types='
CREATE TABLE phase0.tz_test (id int PRIMARY KEY, ts timestamptz);
CREATE TABLE phase0.tz_mix (id int PRIMARY KEY, ts @TZMIX@);
CREATE TABLE phase0.money_test (id int PRIMARY KEY, amt money);
CREATE TABLE phase0.json_test (id int PRIMARY KEY, doc json);
CREATE TABLE phase0.jsonb_test (id int PRIMARY KEY, doc jsonb);
CREATE TABLE phase0.json_mix (id int PRIMARY KEY, doc @JSONMIX@);
CREATE TABLE phase0.xml_test (id int PRIMARY KEY, doc xml);
CREATE TABLE phase0.arr_test (id int PRIMARY KEY, fa float8[], ia int[]);
CREATE TABLE phase0.rng_test (id int PRIMARY KEY, ir int4range, nr numrange);
CREATE TABLE phase0.iv_test (id int PRIMARY KEY, iv interval);
CREATE TABLE phase0.iv_nk (iv interval);
CREATE TABLE phase0.num_test (id int PRIMARY KEY, amt @NUMTYPE@);
CREATE TABLE phase0.bin_test (id int PRIMARY KEY, b bytea);
CREATE TABLE phase0.empty_test (id int PRIMARY KEY);
CREATE TABLE phase0.col_test (id int PRIMARY KEY, val text);'
old_types="${schema_types//@TZMIX@/timestamp}"
old_types="${old_types//@JSONMIX@/json}"
old_types="${old_types//@NUMTYPE@/numeric(20,4)}"
new_types="${schema_types//@TZMIX@/timestamptz}"
new_types="${new_types//@JSONMIX@/jsonb}"
new_types="${new_types//@NUMTYPE@/numeric(20,6)}"
q "$OLD_URL" "$old_types"; q "$NEW_URL" "$new_types"

# Identical loads on both sides (values per T003 section 6).
load_types_common() { # url
  q "$1" "INSERT INTO phase0.money_test VALUES (1, '1234.56'::money);
  INSERT INTO phase0.json_test VALUES (1, '{\"b\":1,\"a\":2}');
  INSERT INTO phase0.jsonb_test VALUES (1, '{\"b\":1,\"a\":2}'::jsonb);
  INSERT INTO phase0.xml_test VALUES (1, '<a x=\"1\">t</a>'::xml);
  INSERT INTO phase0.arr_test VALUES (1, ARRAY[0.1::float8, 1::float8/3], ARRAY[1,2,3]),
                                     (2, ARRAY[0.1::float8, 1::float8/3], ARRAY[NULL::int,1]);
  INSERT INTO phase0.iv_test VALUES (1, interval '1 year 2 mons 3 days 04:05:06'),
                                    (2, interval '24 hours');
  INSERT INTO phase0.iv_nk VALUES (interval '1 year 2 mons 3 days 04:05:06'),
                                  (interval '1 year 2 mons 3 days 04:05:06'),
                                  (interval '24 hours'), (interval '24 hours');
  INSERT INTO phase0.num_test VALUES (1, 1234.5678), (2, 1.2344), (3, 1.2345);
  INSERT INTO phase0.bin_test VALUES (1, NULL), (2, ''::bytea), (3, decode('00ff','hex'));
  INSERT INTO phase0.col_test VALUES (1, 'alpha'), (2, 'beta');"
}
load_types_common "$OLD_URL"; load_types_common "$NEW_URL"
# Side-specific loads: same instant / same logical content, different writing.
q "$OLD_URL" "INSERT INTO phase0.tz_test VALUES (1, timestamptz '2001-01-01 00:00:00+05:30');
INSERT INTO phase0.tz_mix VALUES (1, timestamp '2001-01-01 00:00:00');
INSERT INTO phase0.json_mix VALUES (1, '{\"b\":1,\"a\":2}'::json);
INSERT INTO phase0.rng_test VALUES (1, int4range(1,5), numrange(1.1,2.2));"
q "$NEW_URL" "INSERT INTO phase0.tz_test VALUES (1, timestamptz '2000-12-31 18:30:00+00');
INSERT INTO phase0.tz_mix VALUES (1, timestamptz '2001-01-01 00:00:00+00');
INSERT INTO phase0.json_mix VALUES (1, '{\"a\":2,\"b\":1}'::jsonb);
INSERT INTO phase0.rng_test VALUES (1, '[1,4]'::int4range, numrange(1.1,2.2));"

echo "T004 6.1 timestamptz:"
check "tz: same instant, different offsets" same fingerprint_tz.sql
FP_EXTRA_OLD='-c timezone=Asia/Kolkata'; FP_EXTRA_NEW='-c timezone=America/New_York'
check "tz: SET LOCAL UTC beats differing session TZ" same fingerprint_tz.sql
unset FP_EXTRA_OLD FP_EXTRA_NEW
mut "tz: one second added" fingerprint_tz.sql \
  "UPDATE phase0.tz_test SET ts = ts + interval '1 second' WHERE id = 1" \
  "UPDATE phase0.tz_test SET ts = timestamptz '2000-12-31 18:30:00+00' WHERE id = 1"
check "timestamp->timestamptz: value diff reported" differ fingerprint_tz_mix.sql
schema_check "timestamp->timestamptz: catalog flags type change" tz_mix "ts:timestamp with time zone" new

echo "T004 6.2 money:"
check "money: lc_monetary C vs en_US.utf8" same fingerprint_money.sql
mut "money: one cent added" fingerprint_money.sql \
  "UPDATE phase0.money_test SET amt = '1234.57'::money WHERE id = 1" \
  "UPDATE phase0.money_test SET amt = '1234.56'::money WHERE id = 1"
assert_both "money::numeric is locale-free (never ::text)" "SELECT '1234.56'::money::numeric::text" "1234.56"
old_lc=$(psql -X -q -A -t "$OLD_URL" -c "SHOW lc_monetary")
new_lc=$(psql -X -q -A -t "$NEW_URL" -c "SHOW lc_monetary")
PASS=$((PASS+1)); printf '  ok    %-52s (old=%s, new=%s)\n' "lc_monetary recorded on both sides (P11)" "$old_lc" "$new_lc"
if [[ "$old_lc" != "$new_lc" ]]; then
  echo "  warn  lc_monetary differs — the engine must record this warning (P11)"
fi

echo "T004 6.3 json / jsonb:"
check "json: exact text identical" same fingerprint_json.sql
mut "json: one value changed" fingerprint_json.sql \
  "UPDATE phase0.json_test SET doc = '{\"b\":2,\"a\":2}' WHERE id = 1" \
  "UPDATE phase0.json_test SET doc = '{\"b\":1,\"a\":2}' WHERE id = 1"
check "jsonb: canonical form identical" same fingerprint_jsonb.sql
mut "jsonb: one value changed" fingerprint_jsonb.sql \
  "UPDATE phase0.jsonb_test SET doc = '{\"a\":3,\"b\":1}'::jsonb WHERE id = 1" \
  "UPDATE phase0.jsonb_test SET doc = '{\"b\":1,\"a\":2}'::jsonb WHERE id = 1"
check "json->jsonb: reorder reported, not quiet" differ fingerprint_json_mix.sql
schema_check "json->jsonb: catalog flags type change" json_mix "doc:jsonb" new
q "$OLD_URL" "INSERT INTO phase0.json_mix VALUES (2, '{\"a\":1,\"a\":2}'::json)"
q "$NEW_URL" "INSERT INTO phase0.json_mix VALUES (2, '{\"a\":2}'::jsonb)"
check "json->jsonb: duplicate-key dedup is data loss" differ fingerprint_json_mix.sql
q "$OLD_URL" "DELETE FROM phase0.json_mix WHERE id = 2"
q "$NEW_URL" "DELETE FROM phase0.json_mix WHERE id = 2"

echo "T004 6.4 xml:"
check "xml: identical under pinned xmloption" same fingerprint_xml.sql
mut "xml: attribute value changed" fingerprint_xml.sql \
  "UPDATE phase0.xml_test SET doc = '<a x=\"2\">t</a>'::xml WHERE id = 1" \
  "UPDATE phase0.xml_test SET doc = '<a x=\"1\">t</a>'::xml WHERE id = 1"

echo "T004 6.5 arrays (P7 regression: extra_float_digits=0):"
check "arrays: float8[]+int[] incl NULL element" same fingerprint_arr.sql
mut "arrays: one float element changed" fingerprint_arr.sql \
  "UPDATE phase0.arr_test SET fa = ARRAY[0.2::float8, 1::float8/3] WHERE id = 1" \
  "UPDATE phase0.arr_test SET fa = ARRAY[0.1::float8, 1::float8/3] WHERE id = 1"
mut "arrays: one int element changed" fingerprint_arr.sql \
  "UPDATE phase0.arr_test SET ia = ARRAY[1,2,4] WHERE id = 1" \
  "UPDATE phase0.arr_test SET ia = ARRAY[1,2,3] WHERE id = 1"
mut "arrays: NULL element row changed" fingerprint_arr.sql \
  "UPDATE phase0.arr_test SET ia = ARRAY[NULL::int,2] WHERE id = 2" \
  "UPDATE phase0.arr_test SET ia = ARRAY[NULL::int,1] WHERE id = 2"

echo "T004 6.6 ranges:"
check "ranges: int4range(1,5) vs '[1,4]' canonical" same fingerprint_rng.sql
mut "ranges: int4range upper bound changed" fingerprint_rng.sql \
  "UPDATE phase0.rng_test SET ir = int4range(1,6) WHERE id = 1" \
  "UPDATE phase0.rng_test SET ir = int4range(1,5) WHERE id = 1"
mut "ranges: numrange upper bound changed" fingerprint_rng.sql \
  "UPDATE phase0.rng_test SET nr = numrange(1.1,2.3) WHERE id = 1" \
  "UPDATE phase0.rng_test SET nr = numrange(1.1,2.2) WHERE id = 1"

echo "T004 6.7 interval (P10 regression incl no-key template):"
check "interval: iso_8601 identical" same fingerprint_iv.sql
mut "interval: one second added" fingerprint_iv.sql \
  "UPDATE phase0.iv_test SET iv = iv + interval '1 second' WHERE id = 1" \
  "UPDATE phase0.iv_test SET iv = interval '1 year 2 mons 3 days 04:05:06' WHERE id = 1"
mut "interval: '24 hours' vs '1 day' trap" fingerprint_iv.sql \
  "UPDATE phase0.iv_test SET iv = interval '1 day' WHERE id = 2" \
  "UPDATE phase0.iv_test SET iv = interval '24 hours' WHERE id = 2"
check "interval no-key: iso_8601 set (P10 regression)" same fingerprint_iv_nokey.sql
mut "interval no-key: '24 hours' vs '1 day' trap" fingerprint_iv_nokey.sql \
  "UPDATE phase0.iv_nk SET iv = interval '1 day' WHERE ctid = (SELECT min(ctid) FROM phase0.iv_nk WHERE iv = interval '24 hours')" \
  "UPDATE phase0.iv_nk SET iv = interval '24 hours' WHERE ctid = (SELECT min(ctid) FROM phase0.iv_nk WHERE iv = interval '1 day')"

echo "T004 6.8 numeric scale change (P4/P8):"
check "numeric scale 4->6: both up-cast to 6" same fingerprint_numscale.sql
mut "numeric scale 6: 0.0001 change caught" fingerprint_numscale.sql \
  "UPDATE phase0.num_test SET amt = 1234.567900 WHERE id = 1" \
  "UPDATE phase0.num_test SET amt = 1234.567800 WHERE id = 1"
assert_both "P4 guard: 1.2344/1.2345 distinct at scale 6" \
  "SELECT count(DISTINCT amt::numeric(20,6)::text) FROM phase0.num_test WHERE id IN (2,3)" "2"
assert_both "P4 hazard: down-cast to scale 2 collapses them" \
  "SELECT count(DISTINCT amt::numeric(20,2)::text) FROM phase0.num_test WHERE id IN (2,3)" "1"

echo "T004 6.9 NULL bytea:"
check "bytea: NULL, empty and bytes identical" same fingerprint_bin.sql
mut "bytea: NULL became empty" fingerprint_bin.sql \
  "UPDATE phase0.bin_test SET b = ''::bytea WHERE id = 1" \
  "UPDATE phase0.bin_test SET b = NULL WHERE id = 1"
mut "bytea: empty became NULL" fingerprint_bin.sql \
  "UPDATE phase0.bin_test SET b = NULL WHERE id = 2" \
  "UPDATE phase0.bin_test SET b = ''::bytea WHERE id = 2"
mut "bytea: one byte changed (00ff->00fe)" fingerprint_bin.sql \
  "UPDATE phase0.bin_test SET b = decode('00fe','hex') WHERE id = 3" \
  "UPDATE phase0.bin_test SET b = decode('00ff','hex') WHERE id = 3"

echo "T004 6.10 empty table (P5):"
check "empty table: zero fingerprint rows both sides" same fingerprint_empty.sql
assert_both "empty table: explicit count(*)=0 (P5)" "SELECT count(*) FROM phase0.empty_test" "0"
mut "empty table: one row inserted" fingerprint_empty.sql \
  "INSERT INTO phase0.empty_test VALUES (1)" \
  "DELETE FROM phase0.empty_test"
q "$NEW_URL" "DROP TABLE phase0.empty_test"
check_error "missing table: query error, never empty-PASS (P5)" fingerprint_empty.sql
q "$NEW_URL" "CREATE TABLE phase0.empty_test (id int PRIMARY KEY)"
check "empty table: matches again after restore" same fingerprint_empty.sql

echo "T004 6.11 added / dropped column (P3):"
check "added column: baseline identical" same fingerprint_col.sql
q "$NEW_URL" "ALTER TABLE phase0.col_test ADD COLUMN extra int DEFAULT 0"
check "added column: mapped-value hash stays same" same fingerprint_col.sql
schema_check "added column: catalog detects it (green, P3)" col_test "extra:integer" new
assert_one "coverage: unmapped column listed" "$NEW_URL" \
  "SELECT attname FROM pg_attribute WHERE attrelid = 'phase0.col_test'::regclass AND attnum > 0 AND NOT attisdropped AND attname NOT IN ('id','val') ORDER BY attnum" "extra"
echo "  note  value PASS alone is not the verdict: schema diff must report the added column (P3)"
q "$NEW_URL" "DROP TABLE phase0.col_test"
q "$NEW_URL" "CREATE TABLE phase0.col_test (id int PRIMARY KEY)"
q "$NEW_URL" "INSERT INTO phase0.col_test VALUES (1), (2)"
check_error "dropped mapped column: error, never silent PASS" fingerprint_col.sql
schema_check "dropped column: catalog detects it (red, P3)" col_test "val:text" old
q "$NEW_URL" "DROP TABLE phase0.col_test"
q "$NEW_URL" "CREATE TABLE phase0.col_test (id int PRIMARY KEY, val text)"
q "$NEW_URL" "INSERT INTO phase0.col_test VALUES (1, 'alpha'), (2, 'beta')"
check "added/dropped column: matches again after restore" same fingerprint_col.sql

echo "T004 section 7 overflow bound:"
assert_both "s7: bucket slices unsigned (ffffffff=4294967295)" "SELECT ('x'||'ffffffff')::bit(32)::bigint" "4294967295"
assert_both "s7: sum(bigint) is exact numeric, no overflow" "SELECT pg_typeof(sum(x::bigint))::text FROM (VALUES (1),(2)) v(x)" "numeric"

# ---------------------------------------------------------------------------
# T004 6.12/6.13 need extra databases on the OLD server. They are throwaway:
# dropped and recreated on every run, removed again at the end. The new side
# is the existing bank (UTF8, en_US.utf8). No compose change, no new ports.
# ---------------------------------------------------------------------------
echo "T004 6.12 different collations, same encoding (bank_c on old server):"
q "$OLD_URL" "DROP DATABASE IF EXISTS bank_c"
q "$OLD_URL" "CREATE DATABASE bank_c ENCODING 'UTF8' LC_COLLATE 'C' LC_CTYPE 'C' TEMPLATE template0"
C_URL="${OLD_URL%/bank}/bank_c"
q "$C_URL" "CREATE SCHEMA phase0; CREATE TABLE phase0.coll_test (id int PRIMARY KEY, name text)"
q "$NEW_URL" "CREATE TABLE phase0.coll_test (id int PRIMARY KEY, name text)"
load_coll() { q "$1" "INSERT INTO phase0.coll_test VALUES (1, 'a'), (2, 'A'), (3, 'b'), (4, chr(228));"; }
load_coll "$C_URL"; load_coll "$NEW_URL"
assert_one "collation: old bank_c is C" "$C_URL" \
  "SELECT datcollate FROM pg_database WHERE datname = current_database()" "C"
assert_one "collation: new bank is en_US.utf8" "$NEW_URL" \
  "SELECT datcollate FROM pg_database WHERE datname = current_database()" "en_US.utf8"
assert_one "encoding: old bank_c is UTF8" "$C_URL" \
  "SELECT pg_encoding_to_char(encoding) FROM pg_database WHERE datname = current_database()" "UTF8"
assert_one "encoding: new bank is UTF8" "$NEW_URL" \
  "SELECT pg_encoding_to_char(encoding) FROM pg_database WHERE datname = current_database()" "UTF8"
check "collations: identical text stays quiet" same fingerprint_coll.sql "$C_URL" "$NEW_URL"
mut "collations: 'a' became 'A' (bytes catch it)" fingerprint_coll.sql \
  "UPDATE phase0.coll_test SET name = 'A' WHERE id = 1" \
  "UPDATE phase0.coll_test SET name = 'a' WHERE id = 1" "$NEW_URL" "$C_URL"

echo "T004 6.13 SQL_ASCII (bank_ascii on old server, P19):"
q "$OLD_URL" "DROP DATABASE IF EXISTS bank_ascii"
q "$OLD_URL" "CREATE DATABASE bank_ascii ENCODING 'SQL_ASCII' LC_COLLATE 'C' LC_CTYPE 'C' TEMPLATE template0"
A_URL="${OLD_URL%/bank}/bank_ascii"
q "$A_URL" "CREATE SCHEMA phase0; CREATE TABLE phase0.ascii_test (id int PRIMARY KEY, t text)"
q "$NEW_URL" "CREATE TABLE phase0.ascii_test (id int PRIMARY KEY, t text)"
load_ascii() { q "$1" "INSERT INTO phase0.ascii_test VALUES (1, 'hello'), (2, 'Plain ASCII 123');"; }
load_ascii "$A_URL"; load_ascii "$NEW_URL"
assert_one "SQL_ASCII detected from pg_database (P19)" "$A_URL" \
  "SELECT pg_encoding_to_char(encoding) FROM pg_database WHERE datname = current_database()" "SQL_ASCII"
echo "  note  engine must FLAG SQL_ASCII for review — never silent PASS on the encoding (P19)"
check "SQL_ASCII: pure-ASCII data stays quiet" same fingerprint_ascii.sql "$A_URL" "$NEW_URL"
q "$A_URL" "INSERT INTO phase0.ascii_test VALUES (3, chr(233))"
q "$NEW_URL" "INSERT INTO phase0.ascii_test VALUES (3, chr(233))"
# Verified T004 finding (corrects P19's docs-based assumption): convert_to on
# SQL_ASCII does NOT pass non-ASCII bytes through to UTF8 — it validates them
# and ERRORS (9.4/9.6/18). The fingerprint fails loudly; it can never silently
# PASS non-ASCII SQL_ASCII data. Passthrough happens only when the destination
# encoding accepts the bytes (e.g. LATIN1), with no conversion.
check_error "SQL_ASCII: non-ASCII fingerprint errors (P19)" fingerprint_ascii.sql "$A_URL"
assert_error "SQL_ASCII: convert_to UTF8 rejects raw 0xE9" "$A_URL" \
  "SELECT convert_to(chr(233),'UTF8')"
assert_one "SQL_ASCII: no conversion to LATIN1 (0xE9 stays)" "$A_URL" \
  "SELECT encode(convert_to(chr(233),'LATIN1'),'hex')" "e9"
assert_one "SQL_ASCII: chr(233) stored as 1 raw byte" "$A_URL" \
  "SELECT octet_length(t) FROM phase0.ascii_test WHERE id = 3" "1"
assert_one "UTF-8 side: chr(233) stored as 2 bytes (c3a9)" "$NEW_URL" \
  "SELECT octet_length(t) FROM phase0.ascii_test WHERE id = 3" "2"
assert_one "UTF-8 side: chr(233) converts to c3a9" "$NEW_URL" \
  "SELECT encode(convert_to(chr(233),'UTF8'),'hex')" "c3a9"
assert_one "LATIN1 side: chr(233) converts to c3a9" "$OLD_URL" \
  "SELECT encode(convert_to(chr(233),'UTF8'),'hex')" "c3a9"
q "$A_URL" "DELETE FROM phase0.ascii_test WHERE id = 3"
q "$NEW_URL" "DELETE FROM phase0.ascii_test WHERE id = 3"
check "SQL_ASCII: matches again after undo" same fingerprint_ascii.sql "$A_URL" "$NEW_URL"

# Cleanup: auxiliary databases first, then the phase0 schema on both sides.
q "$C_URL" "DROP SCHEMA phase0 CASCADE"; q "$A_URL" "DROP SCHEMA phase0 CASCADE"
q "$OLD_URL" "DROP DATABASE bank_c"; q "$OLD_URL" "DROP DATABASE bank_ascii"
q "$OLD_URL" "DROP SCHEMA phase0 CASCADE"; q "$NEW_URL" "DROP SCHEMA phase0 CASCADE"
echo; echo "Result: $PASS passed, $FAIL failed"
[[ "$FAIL" -eq 0 ]]
