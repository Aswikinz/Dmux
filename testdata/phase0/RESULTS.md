# Phase 0 results

Task: T001 · Date: 2026-11-16 · Run by: Nina

## T001: old 9.6 vs new 17

Servers, as printed by `prove.sh`:

- old: 9.6.24, LATIN1, collate C — `127.0.0.1:5433`, container `dmux-old`
- new: 17.12 (Debian 17.12-1.pgdg13+1), UTF8, collate en_US.utf8 — `127.0.0.1:5434`, container `dmux-new`

Both containers were started with `podman compose -f testdata/compose.yml up -d`
and reported healthy before the run. Full output of `bash testdata/phase0/prove.sh`
(exit code 0):

```text
Servers:
  old: 9.6.24, LATIN1, collate C
  new: 17.12 (Debian 17.12-1.pgdg13+1), UTF8, collate en_US.utf8
Must stay quiet (identical data):
  ok    same data, other row order, column order, encoding   (same)
  ok    char(8) on old side, varchar(8) on new side          (same)
  ok    table with no key, duplicates on both sides          (same)
Must be reported (one change each, then undone):
  ok    one balance changed by 0.0001                        (differ, 1 bucket(s))
  ok    one character changed in a name                      (differ, 1 bucket(s))
  ok    empty text turned into NULL                          (differ, 1 bucket(s))
  ok    time of day dropped from a timestamp                 (differ, 1 bucket(s))
  ok    float changed in the last digit                      (differ, 1 bucket(s))
  ok    one byte changed in binary data                      (differ, 1 bucket(s))
  ok    two values swapped between columns of one row        (differ, 1 bucket(s))
  ok    one row deleted                                      (differ, 1 bucket(s))
  ok    no-key table: one of several duplicate rows deleted  (differ, 1 bucket(s))
  ok    no-key table: one duplicate pair added (XOR would miss it) (differ, 1 bucket(s))
After undoing every change:
  ok    accounts match again                                 (same)
  ok    no-key table matches again                           (same)

Result: 15 passed, 0 failed
```

**Result: 15 passed, 0 failed.** The three "must stay quiet" checks gave identical
fingerprints across the encoding, row-order, column-order and char/varchar
differences; each of the ten planted changes altered exactly one bucket; both
tables matched again after every change was undone.

Note: `prove.sh` is a test harness and creates then drops a `phase0` schema in both
mock databases. The Dmux engine itself remains read-only.

## T002: version matrix

Date: 2026-11-16 · Run by: Nina

Every pair of old in {9.4, 9.6} and new in {12, 17, 18}, run sequentially (the pairs
share servers, and `prove.sh` recreates the `phase0` schema on each run). The matrix
profile was started with `podman compose -f testdata/compose.yml --profile matrix up -d`,
which pulled `postgres:9.4`, `postgres:12` and `postgres:18`; all five containers
reported healthy and all five ports accepted connections (`pg_isready`) before the
runs. The local client was psql 18.7, used against every server including 9.4, with
no warnings or errors.

| Pair (old → new) | Old version / encoding / collation | New version / encoding / collation | Passed / failed |
|---|---|---|---|
| 9.4 → 12 (5435 → 5436) | 9.4.26, LATIN1, C | 12.22 (Debian 12.22-1.pgdg120+1), UTF8, en_US.utf8 | 15 / 0 |
| 9.4 → 17 (5435 → 5434) | 9.4.26, LATIN1, C | 17.12 (Debian 17.12-1.pgdg13+1), UTF8, en_US.utf8 | 15 / 0 |
| 9.4 → 18 (5435 → 5437) | 9.4.26, LATIN1, C | 18.7 (Debian 18.7-1.pgdg13+2), UTF8, en_US.utf8 | 15 / 0 |
| 9.6 → 12 (5433 → 5436) | 9.6.24, LATIN1, C | 12.22 (Debian 12.22-1.pgdg120+1), UTF8, en_US.utf8 | 15 / 0 |
| 9.6 → 17 (5433 → 5434) | 9.6.24, LATIN1, C | 17.12 (Debian 17.12-1.pgdg13+1), UTF8, en_US.utf8 | 15 / 0 |
| 9.6 → 18 (5433 → 5437) | 9.6.24, LATIN1, C | 18.7 (Debian 18.7-1.pgdg13+2), UTF8, en_US.utf8 | 15 / 0 |

**All six pairs: 15 passed, 0 failed, exit code 0.** On every pair the three "must
stay quiet" checks gave identical fingerprints across the encoding, row-order,
column-order and char/varchar differences; each of the ten planted changes altered
exactly one bucket; both tables matched again after every change was undone. The
9.6 → 17 re-run reproduces T001 exactly.

### 9.4 vs 12 (`prove.sh 5435 5436`)

Servers, as printed by `prove.sh`:

- old: 9.4.26, LATIN1, collate C — `127.0.0.1:5435`, container `dmux-old94`
- new: 12.22 (Debian 12.22-1.pgdg120+1), UTF8, collate en_US.utf8 — `127.0.0.1:5436`, container `dmux-new12`

Full output (exit code 0):

```text
Servers:
  old: 9.4.26, LATIN1, collate C
  new: 12.22 (Debian 12.22-1.pgdg120+1), UTF8, collate en_US.utf8
Must stay quiet (identical data):
  ok    same data, other row order, column order, encoding   (same)
  ok    char(8) on old side, varchar(8) on new side          (same)
  ok    table with no key, duplicates on both sides          (same)
Must be reported (one change each, then undone):
  ok    one balance changed by 0.0001                        (differ, 1 bucket(s))
  ok    one character changed in a name                      (differ, 1 bucket(s))
  ok    empty text turned into NULL                          (differ, 1 bucket(s))
  ok    time of day dropped from a timestamp                 (differ, 1 bucket(s))
  ok    float changed in the last digit                      (differ, 1 bucket(s))
  ok    one byte changed in binary data                      (differ, 1 bucket(s))
  ok    two values swapped between columns of one row        (differ, 1 bucket(s))
  ok    one row deleted                                      (differ, 1 bucket(s))
  ok    no-key table: one of several duplicate rows deleted  (differ, 1 bucket(s))
  ok    no-key table: one duplicate pair added (XOR would miss it) (differ, 1 bucket(s))
After undoing every change:
  ok    accounts match again                                 (same)
  ok    no-key table matches again                           (same)

Result: 15 passed, 0 failed
```

### 9.4 vs 17 (`prove.sh 5435 5434`)

Servers, as printed by `prove.sh`:

- old: 9.4.26, LATIN1, collate C — `127.0.0.1:5435`, container `dmux-old94`
- new: 17.12 (Debian 17.12-1.pgdg13+1), UTF8, collate en_US.utf8 — `127.0.0.1:5434`, container `dmux-new`

Full output (exit code 0):

```text
Servers:
  old: 9.4.26, LATIN1, collate C
  new: 17.12 (Debian 17.12-1.pgdg13+1), UTF8, collate en_US.utf8
Must stay quiet (identical data):
  ok    same data, other row order, column order, encoding   (same)
  ok    char(8) on old side, varchar(8) on new side          (same)
  ok    table with no key, duplicates on both sides          (same)
Must be reported (one change each, then undone):
  ok    one balance changed by 0.0001                        (differ, 1 bucket(s))
  ok    one character changed in a name                      (differ, 1 bucket(s))
  ok    empty text turned into NULL                          (differ, 1 bucket(s))
  ok    time of day dropped from a timestamp                 (differ, 1 bucket(s))
  ok    float changed in the last digit                      (differ, 1 bucket(s))
  ok    one byte changed in binary data                      (differ, 1 bucket(s))
  ok    two values swapped between columns of one row        (differ, 1 bucket(s))
  ok    one row deleted                                      (differ, 1 bucket(s))
  ok    no-key table: one of several duplicate rows deleted  (differ, 1 bucket(s))
  ok    no-key table: one duplicate pair added (XOR would miss it) (differ, 1 bucket(s))
After undoing every change:
  ok    accounts match again                                 (same)
  ok    no-key table matches again                           (same)

Result: 15 passed, 0 failed
```

### 9.4 vs 18 (`prove.sh 5435 5437`)

Servers, as printed by `prove.sh`:

- old: 9.4.26, LATIN1, collate C — `127.0.0.1:5435`, container `dmux-old94`
- new: 18.7 (Debian 18.7-1.pgdg13+2), UTF8, collate en_US.utf8 — `127.0.0.1:5437`, container `dmux-new18`

Full output (exit code 0):

```text
Servers:
  old: 9.4.26, LATIN1, collate C
  new: 18.7 (Debian 18.7-1.pgdg13+2), UTF8, collate en_US.utf8
Must stay quiet (identical data):
  ok    same data, other row order, column order, encoding   (same)
  ok    char(8) on old side, varchar(8) on new side          (same)
  ok    table with no key, duplicates on both sides          (same)
Must be reported (one change each, then undone):
  ok    one balance changed by 0.0001                        (differ, 1 bucket(s))
  ok    one character changed in a name                      (differ, 1 bucket(s))
  ok    empty text turned into NULL                          (differ, 1 bucket(s))
  ok    time of day dropped from a timestamp                 (differ, 1 bucket(s))
  ok    float changed in the last digit                      (differ, 1 bucket(s))
  ok    one byte changed in binary data                      (differ, 1 bucket(s))
  ok    two values swapped between columns of one row        (differ, 1 bucket(s))
  ok    one row deleted                                      (differ, 1 bucket(s))
  ok    no-key table: one of several duplicate rows deleted  (differ, 1 bucket(s))
  ok    no-key table: one duplicate pair added (XOR would miss it) (differ, 1 bucket(s))
After undoing every change:
  ok    accounts match again                                 (same)
  ok    no-key table matches again                           (same)

Result: 15 passed, 0 failed
```

### 9.6 vs 12 (`prove.sh 5433 5436`)

Servers, as printed by `prove.sh`:

- old: 9.6.24, LATIN1, collate C — `127.0.0.1:5433`, container `dmux-old`
- new: 12.22 (Debian 12.22-1.pgdg120+1), UTF8, collate en_US.utf8 — `127.0.0.1:5436`, container `dmux-new12`

Full output (exit code 0):

```text
Servers:
  old: 9.6.24, LATIN1, collate C
  new: 12.22 (Debian 12.22-1.pgdg120+1), UTF8, collate en_US.utf8
Must stay quiet (identical data):
  ok    same data, other row order, column order, encoding   (same)
  ok    char(8) on old side, varchar(8) on new side          (same)
  ok    table with no key, duplicates on both sides          (same)
Must be reported (one change each, then undone):
  ok    one balance changed by 0.0001                        (differ, 1 bucket(s))
  ok    one character changed in a name                      (differ, 1 bucket(s))
  ok    empty text turned into NULL                          (differ, 1 bucket(s))
  ok    time of day dropped from a timestamp                 (differ, 1 bucket(s))
  ok    float changed in the last digit                      (differ, 1 bucket(s))
  ok    one byte changed in binary data                      (differ, 1 bucket(s))
  ok    two values swapped between columns of one row        (differ, 1 bucket(s))
  ok    one row deleted                                      (differ, 1 bucket(s))
  ok    no-key table: one of several duplicate rows deleted  (differ, 1 bucket(s))
  ok    no-key table: one duplicate pair added (XOR would miss it) (differ, 1 bucket(s))
After undoing every change:
  ok    accounts match again                                 (same)
  ok    no-key table matches again                           (same)

Result: 15 passed, 0 failed
```

### 9.6 vs 17 (`prove.sh 5433 5434`, re-run of T001)

Servers, as printed by `prove.sh`:

- old: 9.6.24, LATIN1, collate C — `127.0.0.1:5433`, container `dmux-old`
- new: 17.12 (Debian 17.12-1.pgdg13+1), UTF8, collate en_US.utf8 — `127.0.0.1:5434`, container `dmux-new`

Full output (exit code 0):

```text
Servers:
  old: 9.6.24, LATIN1, collate C
  new: 17.12 (Debian 17.12-1.pgdg13+1), UTF8, collate en_US.utf8
Must stay quiet (identical data):
  ok    same data, other row order, column order, encoding   (same)
  ok    char(8) on old side, varchar(8) on new side          (same)
  ok    table with no key, duplicates on both sides          (same)
Must be reported (one change each, then undone):
  ok    one balance changed by 0.0001                        (differ, 1 bucket(s))
  ok    one character changed in a name                      (differ, 1 bucket(s))
  ok    empty text turned into NULL                          (differ, 1 bucket(s))
  ok    time of day dropped from a timestamp                 (differ, 1 bucket(s))
  ok    float changed in the last digit                      (differ, 1 bucket(s))
  ok    one byte changed in binary data                      (differ, 1 bucket(s))
  ok    two values swapped between columns of one row        (differ, 1 bucket(s))
  ok    one row deleted                                      (differ, 1 bucket(s))
  ok    no-key table: one of several duplicate rows deleted  (differ, 1 bucket(s))
  ok    no-key table: one duplicate pair added (XOR would miss it) (differ, 1 bucket(s))
After undoing every change:
  ok    accounts match again                                 (same)
  ok    no-key table matches again                           (same)

Result: 15 passed, 0 failed
```

### 9.6 vs 18 (`prove.sh 5433 5437`)

Servers, as printed by `prove.sh`:

- old: 9.6.24, LATIN1, collate C — `127.0.0.1:5433`, container `dmux-old`
- new: 18.7 (Debian 18.7-1.pgdg13+2), UTF8, collate en_US.utf8 — `127.0.0.1:5437`, container `dmux-new18`

Full output (exit code 0):

```text
Servers:
  old: 9.6.24, LATIN1, collate C
  new: 18.7 (Debian 18.7-1.pgdg13+2), UTF8, collate en_US.utf8
Must stay quiet (identical data):
  ok    same data, other row order, column order, encoding   (same)
  ok    char(8) on old side, varchar(8) on new side          (same)
  ok    table with no key, duplicates on both sides          (same)
Must be reported (one change each, then undone):
  ok    one balance changed by 0.0001                        (differ, 1 bucket(s))
  ok    one character changed in a name                      (differ, 1 bucket(s))
  ok    empty text turned into NULL                          (differ, 1 bucket(s))
  ok    time of day dropped from a timestamp                 (differ, 1 bucket(s))
  ok    float changed in the last digit                      (differ, 1 bucket(s))
  ok    one byte changed in binary data                      (differ, 1 bucket(s))
  ok    two values swapped between columns of one row        (differ, 1 bucket(s))
  ok    one row deleted                                      (differ, 1 bucket(s))
  ok    no-key table: one of several duplicate rows deleted  (differ, 1 bucket(s))
  ok    no-key table: one duplicate pair added (XOR would miss it) (differ, 1 bucket(s))
After undoing every change:
  ok    accounts match again                                 (same)
  ok    no-key table matches again                           (same)

Result: 15 passed, 0 failed
```

Note: as in T001, `prove.sh` is a test harness and creates then drops a `phase0`
schema in the mock databases it runs against. The Dmux engine itself remains
read-only. No secrets or row data are recorded here.

## How to reproduce

From the main checkout, with rootless Podman (ports bind to 127.0.0.1 only):

```
scripts/in-wt.sh T001 podman compose -f testdata/compose.yml up -d
# wait until both containers are healthy:
scripts/in-wt.sh T001 podman ps --format '{{.Names}} {{.Status}}'
scripts/in-wt.sh T001 pg_isready -h 127.0.0.1 -p 5433 -U dmux_dev -d bank
scripts/in-wt.sh T001 pg_isready -h 127.0.0.1 -p 5434 -U dmux_dev -d bank
scripts/in-wt.sh T001 bash testdata/phase0/prove.sh
```

Expected output ends with `Result: 15 passed, 0 failed` and exit code 0.

For the full version matrix (T002), start the `matrix` profile and run all six pairs
sequentially — never in parallel, because the pairs share servers and `prove.sh`
recreates the `phase0` schema on each run:

```
scripts/in-wt.sh T002 podman compose -f testdata/compose.yml --profile matrix up -d
# wait until all five containers are healthy:
scripts/in-wt.sh T002 podman ps --format '{{.Names}} {{.Status}}'
scripts/in-wt.sh T002 pg_isready -h 127.0.0.1 -p 5433 -U dmux_dev -d bank
scripts/in-wt.sh T002 pg_isready -h 127.0.0.1 -p 5434 -U dmux_dev -d bank
scripts/in-wt.sh T002 pg_isready -h 127.0.0.1 -p 5435 -U dmux_dev -d bank
scripts/in-wt.sh T002 pg_isready -h 127.0.0.1 -p 5436 -U dmux_dev -d bank
scripts/in-wt.sh T002 pg_isready -h 127.0.0.1 -p 5437 -U dmux_dev -d bank
scripts/in-wt.sh T002 bash testdata/phase0/prove.sh 5435 5436   # 9.4 vs 12
scripts/in-wt.sh T002 bash testdata/phase0/prove.sh 5435 5434   # 9.4 vs 17
scripts/in-wt.sh T002 bash testdata/phase0/prove.sh 5435 5437   # 9.4 vs 18
scripts/in-wt.sh T002 bash testdata/phase0/prove.sh 5433 5436   # 9.6 vs 12
scripts/in-wt.sh T002 bash testdata/phase0/prove.sh 5433 5434   # 9.6 vs 17
scripts/in-wt.sh T002 bash testdata/phase0/prove.sh 5433 5437   # 9.6 vs 18
```

The first `up -d` with `--profile matrix` pulls `postgres:9.4`, `postgres:12` and
`postgres:18`, which can take several minutes. Each of the six runs is expected to
end with `Result: 15 passed, 0 failed` and exit code 0.

## T004: extended proof

Date: 2026-11-16 · Run by: Nina

What was added (spec: `docs/phase0/T003-sql-review.md` §§4–7, PLAN.md §3.2):

- **Canonical settings block (§4)** in `fingerprint.sql` and `fingerprint_nokey.sql`:
  `search_path=pg_catalog` (P1), `timezone='UTC'`, `datestyle='ISO, YMD'`,
  `intervalstyle='iso_8601'` and `bytea_output='hex'` (added to the no-key file,
  P10/P18), `extra_float_digits=0` (P7), `xmloption='document'` (P16),
  `statement_timeout='300s'` (P17, harness window; the engine uses the run window),
  `lock_timeout='5s'`, `application_name='dmux'`. Hash logic unchanged (md5 stays,
  DECISIONS #10). All 15 original checks still pass unchanged — the regression guard.
- **All 13 cases from §6**, values exactly as specified, plus two §7 overflow-bound
  probes: 68 new checks (83 total). 17 new per-table fingerprint templates
  (`fingerprint_tz.sql`, `fingerprint_tz_mix.sql`, `fingerprint_money.sql`,
  `fingerprint_json.sql`, `fingerprint_json_mix.sql`, `fingerprint_jsonb.sql`,
  `fingerprint_xml.sql`, `fingerprint_arr.sql`, `fingerprint_rng.sql`,
  `fingerprint_iv.sql`, `fingerprint_iv_nokey.sql`, `fingerprint_numscale.sql`,
  `fingerprint_bin.sql`, `fingerprint_empty.sql`, `fingerprint_col.sql`,
  `fingerprint_coll.sql`, `fingerprint_ascii.sql`), each with the same settings
  block, the same `bucket|n|s1|s2|s3|s4` output shape, order-independent,
  9.4-safe, and a comment naming the normalisation rule per column.
- **New databases for 6.12/6.13**: `bank_c` (UTF8, LC_COLLATE/LC_CTYPE C) and
  `bank_ascii` (SQL_ASCII, C), created with `CREATE DATABASE ... TEMPLATE template0`
  on the OLD server of the pair (default :5433; :5435 in the 9.4 smoke). No compose
  change, no new ports, no new images. `prove.sh` drops and recreates them on every
  run and drops them again at the end; the new side is the existing `bank`
  (UTF8, en_US.utf8). The existing `bank` databases and container config are untouched.
- Harness helpers extended backward-compatibly: per-case database URLs, scalar
  assertions (`assert_both`/`assert_one`), expected-error checks
  (`check_error`/`assert_error`, P5/P19), catalog schema-diff checks
  (`schema_check`, P3), and per-side session settings (`FP_EXTRA_OLD/NEW`, used to
  prove `SET LOCAL timezone` beats differing session timezones). `fp()` now tolerates
  an EMPTY fingerprint result (needed for the empty-table case); query errors still
  fail the run via `pipefail` + `ON_ERROR_STOP=1`, so a broken or missing table can
  never masquerade as empty.

**Finding (corrects P19's docs-based assumption).** On a SQL_ASCII database,
`convert_to(x,'UTF8')` does NOT pass non-ASCII bytes through unconverted: PostgreSQL
performs no conversion but VALIDATES the bytes against the destination encoding
(`mbutils.c`, `pg_do_encoding_conversion`: "No conversion is possible, but we must
validate the result"). Verified on 9.4.26, 9.6.24 and 18.7: `convert_to(chr(233),'UTF8')`
raises `invalid byte sequence for encoding "UTF8": 0xe9`, while
`convert_to(chr(233),'LATIN1')` returns the raw byte `e9` (valid in the destination,
no conversion). Consequence for the engine: the reference fingerprint query ERRORS
on any non-ASCII SQL_ASCII row — a loud failure, never a silent PASS — and pure-ASCII
data still hashes equal. The harness asserts the error, the raw-byte storage
(1 byte `0xE9` vs 2 bytes `0xC3A9`), the encoding detection from `pg_database`, and
prints that the engine must FLAG SQL_ASCII for review (P19). This is safer than the
assumed passthrough; no check was weakened. The engine should still flag SQL_ASCII
even when all data is pure ASCII, because the encoding itself is unverifiable.

### Default pair: 9.6 vs 17 (`prove.sh`)

Servers, as printed by `prove.sh`:

- old: 9.6.24, LATIN1, collate C — `127.0.0.1:5433`, container `dmux-old`
- new: 17.12 (Debian 17.12-1.pgdg13+1), UTF8, collate en_US.utf8 — `127.0.0.1:5434`, container `dmux-new`

Full output (exit code 0):

```text
Servers:
  old: 9.6.24, LATIN1, collate C
  new: 17.12 (Debian 17.12-1.pgdg13+1), UTF8, collate en_US.utf8
Must stay quiet (identical data):
  ok    same data, other row order, column order, encoding   (same)
  ok    char(8) on old side, varchar(8) on new side          (same)
  ok    table with no key, duplicates on both sides          (same)
Must be reported (one change each, then undone):
  ok    one balance changed by 0.0001                        (differ, 1 bucket(s))
  ok    one character changed in a name                      (differ, 1 bucket(s))
  ok    empty text turned into NULL                          (differ, 1 bucket(s))
  ok    time of day dropped from a timestamp                 (differ, 1 bucket(s))
  ok    float changed in the last digit                      (differ, 1 bucket(s))
  ok    one byte changed in binary data                      (differ, 1 bucket(s))
  ok    two values swapped between columns of one row        (differ, 1 bucket(s))
  ok    one row deleted                                      (differ, 1 bucket(s))
  ok    no-key table: one of several duplicate rows deleted  (differ, 1 bucket(s))
  ok    no-key table: one duplicate pair added (XOR would miss it) (differ, 1 bucket(s))
After undoing every change:
  ok    accounts match again                                 (same)
  ok    no-key table matches again                           (same)
T004 6.1 timestamptz:
  ok    tz: same instant, different offsets                  (same)
  ok    tz: SET LOCAL UTC beats differing session TZ         (same)
  ok    tz: one second added                                 (differ, 1 bucket(s))
  ok    timestamp->timestamptz: value diff reported          (differ, 1 bucket(s))
  ok    timestamp->timestamptz: catalog flags type change    (schema diff reports it)
T004 6.2 money:
  ok    money: lc_monetary C vs en_US.utf8                   (same)
  ok    money: one cent added                                (differ, 1 bucket(s))
  ok    money::numeric is locale-free (never ::text)         (both sides: 1234.56)
  ok    lc_monetary recorded on both sides (P11)             (old=C, new=en_US.utf8)
  warn  lc_monetary differs — the engine must record this warning (P11)
T004 6.3 json / jsonb:
  ok    json: exact text identical                           (same)
  ok    json: one value changed                              (differ, 1 bucket(s))
  ok    jsonb: canonical form identical                      (same)
  ok    jsonb: one value changed                             (differ, 1 bucket(s))
  ok    json->jsonb: reorder reported, not quiet             (differ, 1 bucket(s))
  ok    json->jsonb: catalog flags type change               (schema diff reports it)
  ok    json->jsonb: duplicate-key dedup is data loss        (differ, 2 bucket(s))
T004 6.4 xml:
  ok    xml: identical under pinned xmloption                (same)
  ok    xml: attribute value changed                         (differ, 1 bucket(s))
T004 6.5 arrays (P7 regression: extra_float_digits=0):
  ok    arrays: float8[]+int[] incl NULL element             (same)
  ok    arrays: one float element changed                    (differ, 1 bucket(s))
  ok    arrays: one int element changed                      (differ, 1 bucket(s))
  ok    arrays: NULL element row changed                     (differ, 1 bucket(s))
T004 6.6 ranges:
  ok    ranges: int4range(1,5) vs '[1,4]' canonical          (same)
  ok    ranges: int4range upper bound changed                (differ, 1 bucket(s))
  ok    ranges: numrange upper bound changed                 (differ, 1 bucket(s))
T004 6.7 interval (P10 regression incl no-key template):
  ok    interval: iso_8601 identical                         (same)
  ok    interval: one second added                           (differ, 1 bucket(s))
  ok    interval: '24 hours' vs '1 day' trap                 (differ, 1 bucket(s))
  ok    interval no-key: iso_8601 set (P10 regression)       (same)
  ok    interval no-key: '24 hours' vs '1 day' trap          (differ, 1 bucket(s))
T004 6.8 numeric scale change (P4/P8):
  ok    numeric scale 4->6: both up-cast to 6                (same)
  ok    numeric scale 6: 0.0001 change caught                (differ, 1 bucket(s))
  ok    P4 guard: 1.2344/1.2345 distinct at scale 6          (both sides: 2)
  ok    P4 hazard: down-cast to scale 2 collapses them       (both sides: 1)
T004 6.9 NULL bytea:
  ok    bytea: NULL, empty and bytes identical               (same)
  ok    bytea: NULL became empty                             (differ, 1 bucket(s))
  ok    bytea: empty became NULL                             (differ, 1 bucket(s))
  ok    bytea: one byte changed (00ff->00fe)                 (differ, 1 bucket(s))
T004 6.10 empty table (P5):
  ok    empty table: zero fingerprint rows both sides        (same)
  ok    empty table: explicit count(*)=0 (P5)                (both sides: 0)
  ok    empty table: one row inserted                        (differ, 0 bucket(s))
  ok    missing table: query error, never empty-PASS (P5)    (query error detected)
  ok    empty table: matches again after restore             (same)
T004 6.11 added / dropped column (P3):
  ok    added column: baseline identical                     (same)
  ok    added column: mapped-value hash stays same           (same)
  ok    added column: catalog detects it (green, P3)         (schema diff reports it)
  ok    coverage: unmapped column listed                     (extra)
  note  value PASS alone is not the verdict: schema diff must report the added column (P3)
  ok    dropped mapped column: error, never silent PASS      (query error detected)
  ok    dropped column: catalog detects it (red, P3)         (schema diff reports it)
  ok    added/dropped column: matches again after restore    (same)
T004 section 7 overflow bound:
  ok    s7: bucket slices unsigned (ffffffff=4294967295)     (both sides: 4294967295)
  ok    s7: sum(bigint) is exact numeric, no overflow        (both sides: numeric)
T004 6.12 different collations, same encoding (bank_c on old server):
  ok    collation: old bank_c is C                           (C)
  ok    collation: new bank is en_US.utf8                    (en_US.utf8)
  ok    encoding: old bank_c is UTF8                         (UTF8)
  ok    encoding: new bank is UTF8                           (UTF8)
  ok    collations: identical text stays quiet               (same)
  ok    collations: 'a' became 'A' (bytes catch it)          (differ, 1 bucket(s))
T004 6.13 SQL_ASCII (bank_ascii on old server, P19):
  ok    SQL_ASCII detected from pg_database (P19)            (SQL_ASCII)
  note  engine must FLAG SQL_ASCII for review — never silent PASS on the encoding (P19)
  ok    SQL_ASCII: pure-ASCII data stays quiet               (same)
  ok    SQL_ASCII: non-ASCII fingerprint errors (P19)        (query error detected)
  ok    SQL_ASCII: convert_to UTF8 rejects raw 0xE9          (query error detected)
  ok    SQL_ASCII: no conversion to LATIN1 (0xE9 stays)      (e9)
  ok    SQL_ASCII: chr(233) stored as 1 raw byte             (1)
  ok    UTF-8 side: chr(233) stored as 2 bytes (c3a9)        (2)
  ok    UTF-8 side: chr(233) converts to c3a9                (c3a9)
  ok    LATIN1 side: chr(233) converts to c3a9               (c3a9)
  ok    SQL_ASCII: matches again after undo                  (same)

Result: 83 passed, 0 failed
```

### Extremes smoke: 9.4 vs 18 (`prove.sh 5435 5437`)

Servers, as printed by `prove.sh`:

- old: 9.4.26, LATIN1, collate C — `127.0.0.1:5435`, container `dmux-old94`
- new: 18.7 (Debian 18.7-1.pgdg13+2), UTF8, collate en_US.utf8 — `127.0.0.1:5437`, container `dmux-new18`

Full output (exit code 0):

```text
Servers:
  old: 9.4.26, LATIN1, collate C
  new: 18.7 (Debian 18.7-1.pgdg13+2), UTF8, collate en_US.utf8
Must stay quiet (identical data):
  ok    same data, other row order, column order, encoding   (same)
  ok    char(8) on old side, varchar(8) on new side          (same)
  ok    table with no key, duplicates on both sides          (same)
Must be reported (one change each, then undone):
  ok    one balance changed by 0.0001                        (differ, 1 bucket(s))
  ok    one character changed in a name                      (differ, 1 bucket(s))
  ok    empty text turned into NULL                          (differ, 1 bucket(s))
  ok    time of day dropped from a timestamp                 (differ, 1 bucket(s))
  ok    float changed in the last digit                      (differ, 1 bucket(s))
  ok    one byte changed in binary data                      (differ, 1 bucket(s))
  ok    two values swapped between columns of one row        (differ, 1 bucket(s))
  ok    one row deleted                                      (differ, 1 bucket(s))
  ok    no-key table: one of several duplicate rows deleted  (differ, 1 bucket(s))
  ok    no-key table: one duplicate pair added (XOR would miss it) (differ, 1 bucket(s))
After undoing every change:
  ok    accounts match again                                 (same)
  ok    no-key table matches again                           (same)
T004 6.1 timestamptz:
  ok    tz: same instant, different offsets                  (same)
  ok    tz: SET LOCAL UTC beats differing session TZ         (same)
  ok    tz: one second added                                 (differ, 1 bucket(s))
  ok    timestamp->timestamptz: value diff reported          (differ, 1 bucket(s))
  ok    timestamp->timestamptz: catalog flags type change    (schema diff reports it)
T004 6.2 money:
  ok    money: lc_monetary C vs en_US.utf8                   (same)
  ok    money: one cent added                                (differ, 1 bucket(s))
  ok    money::numeric is locale-free (never ::text)         (both sides: 1234.56)
  ok    lc_monetary recorded on both sides (P11)             (old=C, new=en_US.utf8)
  warn  lc_monetary differs — the engine must record this warning (P11)
T004 6.3 json / jsonb:
  ok    json: exact text identical                           (same)
  ok    json: one value changed                              (differ, 1 bucket(s))
  ok    jsonb: canonical form identical                      (same)
  ok    jsonb: one value changed                             (differ, 1 bucket(s))
  ok    json->jsonb: reorder reported, not quiet             (differ, 1 bucket(s))
  ok    json->jsonb: catalog flags type change               (schema diff reports it)
  ok    json->jsonb: duplicate-key dedup is data loss        (differ, 2 bucket(s))
T004 6.4 xml:
  ok    xml: identical under pinned xmloption                (same)
  ok    xml: attribute value changed                         (differ, 1 bucket(s))
T004 6.5 arrays (P7 regression: extra_float_digits=0):
  ok    arrays: float8[]+int[] incl NULL element             (same)
  ok    arrays: one float element changed                    (differ, 1 bucket(s))
  ok    arrays: one int element changed                      (differ, 1 bucket(s))
  ok    arrays: NULL element row changed                     (differ, 1 bucket(s))
T004 6.6 ranges:
  ok    ranges: int4range(1,5) vs '[1,4]' canonical          (same)
  ok    ranges: int4range upper bound changed                (differ, 1 bucket(s))
  ok    ranges: numrange upper bound changed                 (differ, 1 bucket(s))
T004 6.7 interval (P10 regression incl no-key template):
  ok    interval: iso_8601 identical                         (same)
  ok    interval: one second added                           (differ, 1 bucket(s))
  ok    interval: '24 hours' vs '1 day' trap                 (differ, 1 bucket(s))
  ok    interval no-key: iso_8601 set (P10 regression)       (same)
  ok    interval no-key: '24 hours' vs '1 day' trap          (differ, 1 bucket(s))
T004 6.8 numeric scale change (P4/P8):
  ok    numeric scale 4->6: both up-cast to 6                (same)
  ok    numeric scale 6: 0.0001 change caught                (differ, 1 bucket(s))
  ok    P4 guard: 1.2344/1.2345 distinct at scale 6          (both sides: 2)
  ok    P4 hazard: down-cast to scale 2 collapses them       (both sides: 1)
T004 6.9 NULL bytea:
  ok    bytea: NULL, empty and bytes identical               (same)
  ok    bytea: NULL became empty                             (differ, 1 bucket(s))
  ok    bytea: empty became NULL                             (differ, 1 bucket(s))
  ok    bytea: one byte changed (00ff->00fe)                 (differ, 1 bucket(s))
T004 6.10 empty table (P5):
  ok    empty table: zero fingerprint rows both sides        (same)
  ok    empty table: explicit count(*)=0 (P5)                (both sides: 0)
  ok    empty table: one row inserted                        (differ, 0 bucket(s))
  ok    missing table: query error, never empty-PASS (P5)    (query error detected)
  ok    empty table: matches again after restore             (same)
T004 6.11 added / dropped column (P3):
  ok    added column: baseline identical                     (same)
  ok    added column: mapped-value hash stays same           (same)
  ok    added column: catalog detects it (green, P3)         (schema diff reports it)
  ok    coverage: unmapped column listed                     (extra)
  note  value PASS alone is not the verdict: schema diff must report the added column (P3)
  ok    dropped mapped column: error, never silent PASS      (query error detected)
  ok    dropped column: catalog detects it (red, P3)         (schema diff reports it)
  ok    added/dropped column: matches again after restore    (same)
T004 section 7 overflow bound:
  ok    s7: bucket slices unsigned (ffffffff=4294967295)     (both sides: 4294967295)
  ok    s7: sum(bigint) is exact numeric, no overflow        (both sides: numeric)
T004 6.12 different collations, same encoding (bank_c on old server):
  ok    collation: old bank_c is C                           (C)
  ok    collation: new bank is en_US.utf8                    (en_US.utf8)
  ok    encoding: old bank_c is UTF8                         (UTF8)
  ok    encoding: new bank is UTF8                           (UTF8)
  ok    collations: identical text stays quiet               (same)
  ok    collations: 'a' became 'A' (bytes catch it)          (differ, 1 bucket(s))
T004 6.13 SQL_ASCII (bank_ascii on old server, P19):
  ok    SQL_ASCII detected from pg_database (P19)            (SQL_ASCII)
  note  engine must FLAG SQL_ASCII for review — never silent PASS on the encoding (P19)
  ok    SQL_ASCII: pure-ASCII data stays quiet               (same)
  ok    SQL_ASCII: non-ASCII fingerprint errors (P19)        (query error detected)
  ok    SQL_ASCII: convert_to UTF8 rejects raw 0xE9          (query error detected)
  ok    SQL_ASCII: no conversion to LATIN1 (0xE9 stays)      (e9)
  ok    SQL_ASCII: chr(233) stored as 1 raw byte             (1)
  ok    UTF-8 side: chr(233) stored as 2 bytes (c3a9)        (2)
  ok    UTF-8 side: chr(233) converts to c3a9                (c3a9)
  ok    LATIN1 side: chr(233) converts to c3a9               (c3a9)
  ok    SQL_ASCII: matches again after undo                  (same)

Result: 83 passed, 0 failed
```

**Both runs: 83 passed, 0 failed, exit code 0** — the 15 original checks unchanged,
plus 68 new checks (6.1: 5, 6.2: 4, 6.3: 7, 6.4: 2, 6.5: 4, 6.6: 3, 6.7: 5, 6.8: 4,
6.9: 4, 6.10: 5, 6.11: 7, §7: 2, 6.12: 6, 6.13: 10). Every stay-quiet case stayed
quiet, every must-catch case was caught (each planted change altered exactly one
bucket, except the json→jsonb dedup case which alters two: the changed row plus the
duplicate-key row), every undo restored equality, and the expected-error cases
(P5 missing table / dropped column, P19 non-ASCII SQL_ASCII) errored loudly.
The P7 regression is covered: without `extra_float_digits=0` the float8[] printed
form differs between 9.4/9.6 and 12+ (verified: `{0.1,0.333333333333333}` vs
`{0.1,0.3333333333333333}`); with the pin the arrays case passes on both pairs.
The full matrix re-run on all pairs is T008, not T004.

Notes:

- `prove.sh` remains a WRITING test harness (phase0 schema, plus the throwaway
  `bank_c`/`bank_ascii` databases on the old server, all removed at the end of a
  successful run). The Dmux engine itself remains read-only.
- No secrets or row data are recorded here; only counts, hashes and expression output.
- Every new/changed SQL query needs Priya's re-review before merge (AGENTS.md rule 3).

### How to reproduce T004

From the main checkout, with the five mock servers running (ports bind to 127.0.0.1):

```
scripts/in-wt.sh T004 bash testdata/phase0/prove.sh              # 9.6 vs 17 (default)
scripts/in-wt.sh T004 bash testdata/phase0/prove.sh 5435 5437    # 9.4 vs 18 (extremes)
```

Expected output ends with `Result: 83 passed, 0 failed` and exit code 0. The script
creates/drops `bank_c` and `bank_ascii` on the old server itself; no compose change
is needed. If a run is interrupted, the next run drops and recreates them.

## T005: fingerprint timing (1M and 20M rows)

Date: 2026-11-16 · Run by: Nina

Purpose: measure the reference fingerprint's throughput on the PLAN.md §8.1
"large" sizes (1 million and 20 million rows) on both default servers, as the
basis for Phase 3 estimate calibration (§4.5: estimates use size on disk) and
nightly large-dataset checks. Harness: `phase0/perf.sh` (new, reusable);
template: `phase0/fingerprint_perf.sql` (new).

### Servers and hardware

- old: 9.6.24, LATIN1, collate C — `127.0.0.1:5433`, container `dmux-old`;
  `shared_buffers=128MB`, `max_parallel_workers_per_gather=0`.
- new: 17.12 (Debian 17.12-1.pgdg13+1), UTF8, collate en_US.utf8 —
  `127.0.0.1:5434`, container `dmux-new`; `shared_buffers=128MB`,
  `max_parallel_workers_per_gather=2`.
- Host (both containers run on it, rootless Podman): AMD Ryzen 5 7640U,
  12 CPUs, 60.6 GB RAM, 1818 GB free disk. Client: psql 18.7.

### Methodology

- **Table**: `phase0.accounts_perf`, the exact `phase0.accounts` column shape
  (accented names via `chr()`, NULL/empty/remark mix, bytea, float8,
  timestamp(0), numeric(20,4)), on BOTH servers. **UNLOGGED**, no PK and no
  indexes: UNLOGGED speeds the load only — a read-only scan never writes WAL,
  so scan timing is unaffected; the key columns (`acct_id`, `bank_code`) are still
  hashed into the key hash exactly as in the reference. Autovacuum is disabled
  on this throwaway table (`autovacuum_enabled=off`) so no background worker
  scans it during a timed run; no ANALYZE is run — the harness never relies on
  statistics (PLAN §4.5 estimates use size on disk for the same reason).
- **Load**: batched `INSERT ... SELECT generate_series`, 1,000,000 rows per
  batch, one transaction per batch (20 batches for 20M — no gigantic
  transaction). Value expressions are verbatim from `prove.sh`'s
  `load_accounts`, so both servers hold identical data. Load is excluded from
  all scan timings. Load took ~2 s per 1M rows and ~41–45 s per 20M rows per
  server.
- **Template**: `fingerprint_perf.sql` is `fingerprint.sql`'s exact shape —
  `bucket|n|s1|s2|s3|s4`, key-hash buckets (4096), four unsigned 32-bit slices,
  integer `GROUP BY 1`/`ORDER BY 1` — with the FULL canonical settings block
  (T003 review §4: `search_path=pg_catalog` first, UTC, ISO, iso_8601, hex
  bytea, `extra_float_digits=0`, xmloption, lock_timeout 5s,
  application_name dmux). md5 stays (DECISIONS #10). **One deviation:**
  `statement_timeout='1800s'` instead of the reference `'300s'`, raised
  pre-emptively for the 20M scan. In the event the slowest measured scan
  (214.7 s) would have fit inside 300 s, but with only ~28% headroom; 1800 s
  keeps the harness safe on slower hosts. Harness window only — the engine
  uses the run window.
- **Timing**: wall clock of `psql -f fingerprint_perf.sql` only (includes
  connection setup and transfer of ≤4096 summary rows, ~0.2 s — a few percent
  of the 1M new-side scan, negligible at 20M). Scans run **strictly
  sequentially**: one table at a time, no concurrent load or scan; before the
  first timed scan of each table the harness waits (≤30 s) for any other active
  backend on the database to finish. Two scans per table: scan1 = first scan after load,
  scan2 = repeat.
- **Cache state**: scan1 runs immediately after the load, so pages are warm in
  the OS page cache (60.6 GB RAM vs ≤2.7 GB table; `shared_buffers` is only
  128MB, so most reads come from the OS cache, not disk). scan2 is a fully warm
  repeat. No cold-cache (disk-bound) numbers: dropping caches needs root and
  would disturb the other containers.
- **Plan shape** (recorded via EXPLAIN, server defaults — the template does not
  pin parallelism): old 9.6 scans single-process (`max_parallel_workers_per_gather=0`);
  new 17 uses a **parallel Gather** (leader + up to 2 workers) at both sizes.
  The throughput gap below is therefore version + parallelism, not version alone.
- **Sanity checks, all passed**: `count(*)` equals the requested rows; the
  bucket counts sum to `count(*)`; ≤4096 buckets; scan2 byte-identical to
  scan1; and **old vs new fingerprints byte-identical at both sizes** — the
  cross-encoding identity proof holds at 20M rows, not just 20k. At 20M rows
  there are ~4,883 rows/bucket (within PLAN's ~10k guidance); bucket sums stay
  far below the T003 §7 overflow bound (20M × 4294967295 ≈ 8.6×10¹⁶ < i64 max).
- **Encoding/size finding** (10k-row probe, cleaned up afterwards): name bytes
  stored differ as expected — 148,894 (LATIN1) vs 165,562 (UTF8), i.e. the
  accented data is real — yet the heap-only size is byte-identical
  (1,327,104 bytes): the extra UTF-8 bytes fit in tuple alignment padding and
  page slack. At 1M and 20M, `pg_total_relation_size` differs by a constant
  237,568 bytes (29 pages) at both sizes — non-heap fork overhead (init/FSM/VM),
  not row data. Size-on-disk estimates therefore transfer across the encoding
  boundary for this row shape.

### Run history (transparency)

The first full run flagged **1 other active backend** during the old-side 20M
timed scans (most likely an autovacuum worker analyzing the freshly loaded
table). The harness was then hardened (autovacuum off on the perf table,
quiescence wait, backend classification without printing query text) and the
full suite re-run: the canonical run below had **zero warnings**. First-run
scans (s): old 1M 9.796/9.945, new 1M 4.111/4.135, old 20M 201.613/203.592,
new 20M 81.580/83.211 — same shape of results; the concurrent backend did not
materially move the old-side numbers (its 20M rate was within 3% of its 1M
rate).

### Metrics (canonical clean re-run; MB = MiB = 1048576 bytes; size = pg_total_relation_size)

| Server | Rows | Size MiB | Scan s | Rows/s | MiB/s |
|---|---|---|---|---|---|
| old 9.6.24 | 1,000,000 | 131.9 | 10.153 | 98,493 | 13.0 |
| new 17.12 | 1,000,000 | 132.1 | 4.239 | 235,905 | 31.2 |
| old 9.6.24 | 20,000,000 | 2694.8 | 214.721 | 93,144 | 12.6 |
| new 17.12 | 20,000,000 | 2695.1 | 109.178 | 183,187 | 24.7 |

Repeat scans (scan2) and within-run variance:

| Server | Rows | Scan2 s | Δ vs scan1 |
|---|---|---|---|
| old 9.6.24 | 1,000,000 | 10.188 | +0.3% |
| new 17.12 | 1,000,000 | 4.203 | −0.8% |
| old 9.6.24 | 20,000,000 | 214.404 | −0.1% |
| new 17.12 | 20,000,000 | 124.139 | +13.7% |

**Variance.** Single-process scans (old side) are very stable: ±1.5% within
runs, ~6% between the two full runs (20M: 201.6 s → 214.7 s). The new side's
parallel scans vary more: 20M scan1 was 81.6 s in the first run vs 109.2 s in
the canonical run, and the canonical repeat was +13.7% — consistent with host
CPU frequency/thermal state under sustained load (laptop-class CPU) and
parallel-worker scheduling. For Phase 3 calibration: measure a timed sample
per server (PLAN §4.5), show estimates as ranges, and expect up to ~±15% on
parallel scans. Conservative planning rates from this host: old ~93–102k
rows/s (~12.6–13.5 MiB/s) single-process; new ~183–245k rows/s (~24.7–33.0
MiB/s) with default parallelism (2 workers).

Total measured load+scan time of the canonical run: ~782 s (~13 min); catalog
queries (count/size/EXPLAIN/quiescence) add a few seconds. First run: ~687 s.

### Cleanup

`perf.sh` dropped the `phase0` schema on both servers and verified via
`pg_namespace` that it is gone (both reported 0); the 10k-row encoding probe
was dropped and verified the same way. No perf tables remain; the mock
databases are otherwise untouched. No secrets or row data are recorded here —
counts, sizes and timings only.

### How to reproduce T005

From the main checkout, with the two default mock servers running:

```
scripts/in-wt.sh T005 podman compose -f testdata/compose.yml up -d   # if not already up
scripts/in-wt.sh T005 bash testdata/phase0/perf.sh                   # 1M + 20M, ~13 min
```

Overrides: `perf.sh <old port> <new port> [size ...]`, or
`PERF_SIZES="1000000 20000000" PERF_BATCH=1000000`. Expected output: per-table
load/scan lines, `ok: old and new fingerprints byte-identical` for each size,
the results table, `phase0 schema dropped, verified gone` on both servers, and
`Done.` (exit code 0). The script refuses non-local URLs. Note:
`fingerprint_perf.sql` is new SQL (the reviewed reference shape with a new
table name and the timeout deviation) and needs Priya's re-review before
merge, like every query change (AGENTS.md rule 3).

## T010: URL guard hardening

Date: 2026-11-16 · Run by: Nina · Follow-up to Priya's T005 re-review (R1/R2/R3).

### What changed

- **Shared guard (R1).** The local-only URL check in `prove.sh` and `perf.sh` was
  a glob (`*@127.0.0.1[:/]*|*@localhost[:/]*`) that only looked for a local host
  *prefix*. libpq honours valid URL syntax that keeps that prefix but routes the
  connection elsewhere: multi-host comma lists (it falls through to the second,
  possibly remote host) and `?host=`/`?port=`/`?service=`/`?options=` query
  overrides. Both harnesses WRITE a throwaway `phase0` schema, so a bypass could
  point them at a real system. The rule now lives in one sourced helper,
  `phase0/guard.sh` (`dmux_guard_url`), and accepts ONLY the strict single-host
  form `postgres://<user>:<password>@<host>:<numeric port>/<dbname>` with host
  exactly `127.0.0.1` or `localhost`. An anchored allow-list rejects every comma,
  any `?` query string, a second `@`, `/`/`?`/`,` in credentials, a non-numeric or
  missing port, an empty dbname, anything after the dbname, and every other
  host/scheme — before any connection attempt. Each harness keeps its original
  refusal message and exit 1. The standard mock URLs and strict-form
  `OLD_URL`/`NEW_URL` overrides still work.
- **Regression battery.** New `phase0/guard_test.sh` runs WITHOUT a database:
  29 guard-predicate cases (4 accept, 25 reject, covering every R1 bypass form)
  plus 2 harness-wiring checks confirming `prove.sh` and `perf.sh` refuse a bypass
  URL with exit 1 before connecting (psql is stubbed, so no database is contacted).
  It prints a pass/fail summary and exits non-zero on any failure. Run it with
  `scripts/in-wt.sh <task> bash testdata/phase0/guard_test.sh`.
- **R2 (wording).** The T005 methodology now says the quiescence wait runs before
  the *first* timed scan of each table (it always did; scan2 follows immediately).
  No numbers or other sections changed.
- **R3 (cosmetic).** Added the missing trailing newline to `fingerprint_perf.sql`
  (now matches the reference `fingerprint.sql`); no other change to that file.

### Re-run results (no regression)

- `scripts/in-wt.sh T010 bash testdata/phase0/prove.sh` → **83 passed, 0 failed**
  (exit 0), 9.6.24 × 17.12; all 83 check names unchanged.
- `PERF_SIZES="100000 200000" scripts/in-wt.sh T010 bash testdata/phase0/perf.sh`
  → **exit 0**; old/new fingerprints byte-identical at both sizes, repeat scans
  identical, `phase0` schema dropped and verified gone on both servers.
- `scripts/in-wt.sh T010 bash testdata/phase0/guard_test.sh` → **31 passed,
  0 failed** (exit 0).
- Direct refusal probes: a comma multi-host `OLD_URL` and a `?host=` `NEW_URL`
  each made the respective harness print `refusing: … only runs against local
  mock databases` and exit 1 before any connection.
- `shellcheck` clean on `guard.sh`, `guard_test.sh`, `prove.sh` and `perf.sh`.

No row data or secrets recorded — counts, sizes, timings and pass/fail only. The
guard diff is safety-critical and goes to Priya for re-review before merge.

### Hardening round 1 (Priya N3/N4)

Date: 2026-11-16 · Run by: Nina · Closes the non-blocking notes N3 and N4 from
Priya's T010 re-review (`docs/phase0/T010-guard-rereview.md`). `guard.sh` is
untouched — the regex is approved as-is.

- **N3 (env-level redirect).** Both `prove.sh` and `perf.sh` now run, immediately
  after `set -euo pipefail` and before any psql invocation:
  `unset PGHOSTADDR PGSERVICE PGSERVICEFILE PGHOST PGPORT PGDATABASE PGUSER PGPASSWORD`.
  Why: libpq prefers hostaddr (`PGHOSTADDR`, or a `PGSERVICE`/`PGSERVICEFILE`
  service-file entry) over the URI host, so env could redirect these
  WRITE-capable harnesses outside the URL guard. The strict URL supplies every
  connection parameter, so no PG* env is needed. Each harness's own `PGOPTIONS`
  export is set after the unset and keeps working.
- **N4 (battery additions).** `guard_test.sh` grows from 31 to **51 cases**
  (6 accept + 37 reject + 8 wiring); all 31 original cases are intact.
  - 2 new accept cases: percent-encoded credentials
    (`user%40x:pass%2Cword`) and a percent-encoded dbname (`bank%3Fhost%3Devil`
    is literal dbname text, not a query) — allowed by the class and routing-safe
    per Priya's libpq split-before-decode source analysis.
  - 12 new reject cases: TAB/LF/CR embedded in an otherwise-valid URL,
    host-suffix `127.0.0.1.evil.com`, octal `0177.0.0.1`, decimal `2130706433`,
    short `127.1`, IPv4-mapped IPv6 `[::ffff:127.0.0.1]`, fragment `#x`, signed
    ports `+5433`/`-5433`, uppercase scheme `POSTGRES://` (the battery pins
    `nocasematch` off so the case runs under default bash options). The brief's
    `[::1]` case was already in the battery (kept); the mapped variant adds the
    new IPv6 coverage.
  - 6 new wiring cases, all with a hostile env (`PGHOSTADDR`/`PGHOST` =
    198.51.100.1, `PGSERVICE=evil` + a service file supplying a remote hostaddr,
    `PGSERVICEFILE`, `PGPORT=59999`, `PGDATABASE=prod`, `PGUSER`/`PGPASSWORD=evil`)
    and a leak-detecting psql stub (records any surviving PG* var, exits 97):
    valid mock URLs must pass the guard and reach the stub with **none** of the
    vars set (both harnesses); bypass `OLD_URL` **and** `NEW_URL` must still
    refuse with exit 1 and zero psql calls (both harnesses — this also closes
    the NEW_URL wiring-coverage asymmetry Priya noted). A negative control
    (harness copy without the unset, stubbed psql, /tmp only) confirmed the stub
    reports every hostile var when the unset is absent, so the check has teeth.
- **Re-run results.**
  `scripts/in-wt.sh T010 bash testdata/phase0/guard_test.sh` → **51 passed,
  0 failed** (exit 0).
  `scripts/in-wt.sh T010 bash testdata/phase0/prove.sh` → **83 passed, 0 failed**
  (exit 0), 9.6.24 × 17.12.
  `PERF_SIZES="100000 200000" scripts/in-wt.sh T010 bash testdata/phase0/perf.sh`
  → **exit 0**; old/new fingerprints byte-identical at both sizes, repeat scans
  identical, `phase0` dropped and verified gone on both servers.
  `shellcheck` clean on `prove.sh`, `perf.sh`, `guard_test.sh` (and unchanged
  `guard.sh`). No row data or secrets — counts and pass/fail only.

## T008: full version matrix (extended proof)

Date: 2026-11-16 · Run by: Nina

Postgres 15 joins the matrix (DECISIONS #9, owner approved). `testdata/compose.yml`
gains a `new15` service mirroring `new12`/`new18`: image `docker.io/library/postgres:15`,
container `dmux-new15`, port `127.0.0.1:5438`, volume `dmux-new15` mounted at the
STANDARD data directory `/var/lib/postgresql/data` (only the 18 image moved it up one
level; the `new18` mount is unchanged), `matrix` profile, and the same common anchors
(environment, healthcheck, restart, shm_size). No `POSTGRES_INITDB_ARGS` override, so
UTF8 with the image's default locale, like the other new-side containers. The
cross-version set is now old {9.4, 9.6} × new {12, 15, 17, 18} — eight pairs.

Every pair ran the EXTENDED `prove.sh` (83 checks: the 15 original plus the 68 T004
cases), so each pair now covers all normalisation cases, not just the two T004 smoke
pairs. The matrix profile was started from the T008 working copy with
`podman compose -f testdata/compose.yml --profile matrix up -d`, which pulled
`postgres:15` (15.20, Debian 15.20-1.pgdg13+1); all six containers reported healthy
and all six ports accepted connections (`pg_isready`) before the runs. Pairs ran
strictly sequentially — the pairs share servers, and `prove.sh` recreates the `phase0`
schema and the `bank_c`/`bank_ascii` databases on the OLD server of the pair per run.
Local client: psql 18.7 against every server including 9.4, with no warnings or
errors beyond the expected P11 `lc_monetary` note. **No harness or template change
was needed for PG15**: `prove.sh`, `guard.sh`, `guard_test.sh`, `perf.sh` and all
fingerprint templates are untouched.

| Pair (old → new) | Old version / encoding / collation | New version / encoding / collation | Passed / failed |
|---|---|---|---|
| 9.4 → 12 (5435 → 5436) | 9.4.26, LATIN1, C | 12.22 (Debian 12.22-1.pgdg120+1), UTF8, en_US.utf8 | 83 / 0 |
| 9.4 → 15 (5435 → 5438) | 9.4.26, LATIN1, C | 15.20 (Debian 15.20-1.pgdg13+1), UTF8, en_US.utf8 | 83 / 0 |
| 9.4 → 17 (5435 → 5434) | 9.4.26, LATIN1, C | 17.12 (Debian 17.12-1.pgdg13+1), UTF8, en_US.utf8 | 83 / 0 |
| 9.4 → 18 (5435 → 5437) | 9.4.26, LATIN1, C | 18.7 (Debian 18.7-1.pgdg13+2), UTF8, en_US.utf8 | 83 / 0 |
| 9.6 → 12 (5433 → 5436) | 9.6.24, LATIN1, C | 12.22 (Debian 12.22-1.pgdg120+1), UTF8, en_US.utf8 | 83 / 0 |
| 9.6 → 15 (5433 → 5438) | 9.6.24, LATIN1, C | 15.20 (Debian 15.20-1.pgdg13+1), UTF8, en_US.utf8 | 83 / 0 |
| 9.6 → 17 (5433 → 5434) | 9.6.24, LATIN1, C | 17.12 (Debian 17.12-1.pgdg13+1), UTF8, en_US.utf8 | 83 / 0 |
| 9.6 → 18 (5433 → 5437) | 9.6.24, LATIN1, C | 18.7 (Debian 18.7-1.pgdg13+2), UTF8, en_US.utf8 | 83 / 0 |

**All eight pairs: 83 passed, 0 failed, exit code 0.** On every pair the three "must
stay quiet" checks gave identical fingerprints across the encoding, row-order,
column-order and char/varchar differences; each planted change altered exactly the
expected bucket(s); every T004 case (timestamptz, money, json/jsonb, xml, arrays,
ranges, interval, numeric scale, NULL bytea, empty table, added/dropped column,
collations, SQL_ASCII, overflow bounds) behaved as specified. The only warning on
any pair is the expected `lc_monetary differs` note (P11: old C, new en_US.utf8),
which the engine must record. The T004 SQL_ASCII finding (convert_to validates and
errors on non-ASCII, never silent passthrough) reproduces on 15 exactly as on
9.4/9.6/18. The eight outputs are byte-identical apart from the two server lines.

Cleanup verified after the last run: no `phase0` schema and no `bank_c`/`bank_ascii`
database on either old server (5433, 5435), and no `phase0` schema on any new-side
server (5434, 5436, 5437, 5438). The existing `bank` databases and container
configuration were never modified. No row data or secrets recorded — versions,
encodings, collations and pass/fail only. Earlier sections of this file are
byte-identical.

### 9.4 vs 12 (`prove.sh 5435 5436`)

Servers, as printed by `prove.sh`:

- old: 9.4.26, LATIN1, collate C — `127.0.0.1:5435`, container `dmux-old94`
- new: 12.22 (Debian 12.22-1.pgdg120+1), UTF8, collate en_US.utf8 — `127.0.0.1:5436`, container `dmux-new12`

Full output (exit code 0):

```text
Servers:
  old: 9.4.26, LATIN1, collate C
  new: 12.22 (Debian 12.22-1.pgdg120+1), UTF8, collate en_US.utf8
Must stay quiet (identical data):
  ok    same data, other row order, column order, encoding   (same)
  ok    char(8) on old side, varchar(8) on new side          (same)
  ok    table with no key, duplicates on both sides          (same)
Must be reported (one change each, then undone):
  ok    one balance changed by 0.0001                        (differ, 1 bucket(s))
  ok    one character changed in a name                      (differ, 1 bucket(s))
  ok    empty text turned into NULL                          (differ, 1 bucket(s))
  ok    time of day dropped from a timestamp                 (differ, 1 bucket(s))
  ok    float changed in the last digit                      (differ, 1 bucket(s))
  ok    one byte changed in binary data                      (differ, 1 bucket(s))
  ok    two values swapped between columns of one row        (differ, 1 bucket(s))
  ok    one row deleted                                      (differ, 1 bucket(s))
  ok    no-key table: one of several duplicate rows deleted  (differ, 1 bucket(s))
  ok    no-key table: one duplicate pair added (XOR would miss it) (differ, 1 bucket(s))
After undoing every change:
  ok    accounts match again                                 (same)
  ok    no-key table matches again                           (same)
T004 6.1 timestamptz:
  ok    tz: same instant, different offsets                  (same)
  ok    tz: SET LOCAL UTC beats differing session TZ         (same)
  ok    tz: one second added                                 (differ, 1 bucket(s))
  ok    timestamp->timestamptz: value diff reported          (differ, 1 bucket(s))
  ok    timestamp->timestamptz: catalog flags type change    (schema diff reports it)
T004 6.2 money:
  ok    money: lc_monetary C vs en_US.utf8                   (same)
  ok    money: one cent added                                (differ, 1 bucket(s))
  ok    money::numeric is locale-free (never ::text)         (both sides: 1234.56)
  ok    lc_monetary recorded on both sides (P11)             (old=C, new=en_US.utf8)
  warn  lc_monetary differs — the engine must record this warning (P11)
T004 6.3 json / jsonb:
  ok    json: exact text identical                           (same)
  ok    json: one value changed                              (differ, 1 bucket(s))
  ok    jsonb: canonical form identical                      (same)
  ok    jsonb: one value changed                             (differ, 1 bucket(s))
  ok    json->jsonb: reorder reported, not quiet             (differ, 1 bucket(s))
  ok    json->jsonb: catalog flags type change               (schema diff reports it)
  ok    json->jsonb: duplicate-key dedup is data loss        (differ, 2 bucket(s))
T004 6.4 xml:
  ok    xml: identical under pinned xmloption                (same)
  ok    xml: attribute value changed                         (differ, 1 bucket(s))
T004 6.5 arrays (P7 regression: extra_float_digits=0):
  ok    arrays: float8[]+int[] incl NULL element             (same)
  ok    arrays: one float element changed                    (differ, 1 bucket(s))
  ok    arrays: one int element changed                      (differ, 1 bucket(s))
  ok    arrays: NULL element row changed                     (differ, 1 bucket(s))
T004 6.6 ranges:
  ok    ranges: int4range(1,5) vs '[1,4]' canonical          (same)
  ok    ranges: int4range upper bound changed                (differ, 1 bucket(s))
  ok    ranges: numrange upper bound changed                 (differ, 1 bucket(s))
T004 6.7 interval (P10 regression incl no-key template):
  ok    interval: iso_8601 identical                         (same)
  ok    interval: one second added                           (differ, 1 bucket(s))
  ok    interval: '24 hours' vs '1 day' trap                 (differ, 1 bucket(s))
  ok    interval no-key: iso_8601 set (P10 regression)       (same)
  ok    interval no-key: '24 hours' vs '1 day' trap          (differ, 1 bucket(s))
T004 6.8 numeric scale change (P4/P8):
  ok    numeric scale 4->6: both up-cast to 6                (same)
  ok    numeric scale 6: 0.0001 change caught                (differ, 1 bucket(s))
  ok    P4 guard: 1.2344/1.2345 distinct at scale 6          (both sides: 2)
  ok    P4 hazard: down-cast to scale 2 collapses them       (both sides: 1)
T004 6.9 NULL bytea:
  ok    bytea: NULL, empty and bytes identical               (same)
  ok    bytea: NULL became empty                             (differ, 1 bucket(s))
  ok    bytea: empty became NULL                             (differ, 1 bucket(s))
  ok    bytea: one byte changed (00ff->00fe)                 (differ, 1 bucket(s))
T004 6.10 empty table (P5):
  ok    empty table: zero fingerprint rows both sides        (same)
  ok    empty table: explicit count(*)=0 (P5)                (both sides: 0)
  ok    empty table: one row inserted                        (differ, 0 bucket(s))
  ok    missing table: query error, never empty-PASS (P5)    (query error detected)
  ok    empty table: matches again after restore             (same)
T004 6.11 added / dropped column (P3):
  ok    added column: baseline identical                     (same)
  ok    added column: mapped-value hash stays same           (same)
  ok    added column: catalog detects it (green, P3)         (schema diff reports it)
  ok    coverage: unmapped column listed                     (extra)
  note  value PASS alone is not the verdict: schema diff must report the added column (P3)
  ok    dropped mapped column: error, never silent PASS      (query error detected)
  ok    dropped column: catalog detects it (red, P3)         (schema diff reports it)
  ok    added/dropped column: matches again after restore    (same)
T004 section 7 overflow bound:
  ok    s7: bucket slices unsigned (ffffffff=4294967295)     (both sides: 4294967295)
  ok    s7: sum(bigint) is exact numeric, no overflow        (both sides: numeric)
T004 6.12 different collations, same encoding (bank_c on old server):
  ok    collation: old bank_c is C                           (C)
  ok    collation: new bank is en_US.utf8                    (en_US.utf8)
  ok    encoding: old bank_c is UTF8                         (UTF8)
  ok    encoding: new bank is UTF8                           (UTF8)
  ok    collations: identical text stays quiet               (same)
  ok    collations: 'a' became 'A' (bytes catch it)          (differ, 1 bucket(s))
T004 6.13 SQL_ASCII (bank_ascii on old server, P19):
  ok    SQL_ASCII detected from pg_database (P19)            (SQL_ASCII)
  note  engine must FLAG SQL_ASCII for review — never silent PASS on the encoding (P19)
  ok    SQL_ASCII: pure-ASCII data stays quiet               (same)
  ok    SQL_ASCII: non-ASCII fingerprint errors (P19)        (query error detected)
  ok    SQL_ASCII: convert_to UTF8 rejects raw 0xE9          (query error detected)
  ok    SQL_ASCII: no conversion to LATIN1 (0xE9 stays)      (e9)
  ok    SQL_ASCII: chr(233) stored as 1 raw byte             (1)
  ok    UTF-8 side: chr(233) stored as 2 bytes (c3a9)        (2)
  ok    UTF-8 side: chr(233) converts to c3a9                (c3a9)
  ok    LATIN1 side: chr(233) converts to c3a9               (c3a9)
  ok    SQL_ASCII: matches again after undo                  (same)

Result: 83 passed, 0 failed
```

### 9.4 vs 15 (`prove.sh 5435 5438`)

Servers, as printed by `prove.sh`:

- old: 9.4.26, LATIN1, collate C — `127.0.0.1:5435`, container `dmux-old94`
- new: 15.20 (Debian 15.20-1.pgdg13+1), UTF8, collate en_US.utf8 — `127.0.0.1:5438`, container `dmux-new15`

Full output (exit code 0):

```text
Servers:
  old: 9.4.26, LATIN1, collate C
  new: 15.20 (Debian 15.20-1.pgdg13+1), UTF8, collate en_US.utf8
Must stay quiet (identical data):
  ok    same data, other row order, column order, encoding   (same)
  ok    char(8) on old side, varchar(8) on new side          (same)
  ok    table with no key, duplicates on both sides          (same)
Must be reported (one change each, then undone):
  ok    one balance changed by 0.0001                        (differ, 1 bucket(s))
  ok    one character changed in a name                      (differ, 1 bucket(s))
  ok    empty text turned into NULL                          (differ, 1 bucket(s))
  ok    time of day dropped from a timestamp                 (differ, 1 bucket(s))
  ok    float changed in the last digit                      (differ, 1 bucket(s))
  ok    one byte changed in binary data                      (differ, 1 bucket(s))
  ok    two values swapped between columns of one row        (differ, 1 bucket(s))
  ok    one row deleted                                      (differ, 1 bucket(s))
  ok    no-key table: one of several duplicate rows deleted  (differ, 1 bucket(s))
  ok    no-key table: one duplicate pair added (XOR would miss it) (differ, 1 bucket(s))
After undoing every change:
  ok    accounts match again                                 (same)
  ok    no-key table matches again                           (same)
T004 6.1 timestamptz:
  ok    tz: same instant, different offsets                  (same)
  ok    tz: SET LOCAL UTC beats differing session TZ         (same)
  ok    tz: one second added                                 (differ, 1 bucket(s))
  ok    timestamp->timestamptz: value diff reported          (differ, 1 bucket(s))
  ok    timestamp->timestamptz: catalog flags type change    (schema diff reports it)
T004 6.2 money:
  ok    money: lc_monetary C vs en_US.utf8                   (same)
  ok    money: one cent added                                (differ, 1 bucket(s))
  ok    money::numeric is locale-free (never ::text)         (both sides: 1234.56)
  ok    lc_monetary recorded on both sides (P11)             (old=C, new=en_US.utf8)
  warn  lc_monetary differs — the engine must record this warning (P11)
T004 6.3 json / jsonb:
  ok    json: exact text identical                           (same)
  ok    json: one value changed                              (differ, 1 bucket(s))
  ok    jsonb: canonical form identical                      (same)
  ok    jsonb: one value changed                             (differ, 1 bucket(s))
  ok    json->jsonb: reorder reported, not quiet             (differ, 1 bucket(s))
  ok    json->jsonb: catalog flags type change               (schema diff reports it)
  ok    json->jsonb: duplicate-key dedup is data loss        (differ, 2 bucket(s))
T004 6.4 xml:
  ok    xml: identical under pinned xmloption                (same)
  ok    xml: attribute value changed                         (differ, 1 bucket(s))
T004 6.5 arrays (P7 regression: extra_float_digits=0):
  ok    arrays: float8[]+int[] incl NULL element             (same)
  ok    arrays: one float element changed                    (differ, 1 bucket(s))
  ok    arrays: one int element changed                      (differ, 1 bucket(s))
  ok    arrays: NULL element row changed                     (differ, 1 bucket(s))
T004 6.6 ranges:
  ok    ranges: int4range(1,5) vs '[1,4]' canonical          (same)
  ok    ranges: int4range upper bound changed                (differ, 1 bucket(s))
  ok    ranges: numrange upper bound changed                 (differ, 1 bucket(s))
T004 6.7 interval (P10 regression incl no-key template):
  ok    interval: iso_8601 identical                         (same)
  ok    interval: one second added                           (differ, 1 bucket(s))
  ok    interval: '24 hours' vs '1 day' trap                 (differ, 1 bucket(s))
  ok    interval no-key: iso_8601 set (P10 regression)       (same)
  ok    interval no-key: '24 hours' vs '1 day' trap          (differ, 1 bucket(s))
T004 6.8 numeric scale change (P4/P8):
  ok    numeric scale 4->6: both up-cast to 6                (same)
  ok    numeric scale 6: 0.0001 change caught                (differ, 1 bucket(s))
  ok    P4 guard: 1.2344/1.2345 distinct at scale 6          (both sides: 2)
  ok    P4 hazard: down-cast to scale 2 collapses them       (both sides: 1)
T004 6.9 NULL bytea:
  ok    bytea: NULL, empty and bytes identical               (same)
  ok    bytea: NULL became empty                             (differ, 1 bucket(s))
  ok    bytea: empty became NULL                             (differ, 1 bucket(s))
  ok    bytea: one byte changed (00ff->00fe)                 (differ, 1 bucket(s))
T004 6.10 empty table (P5):
  ok    empty table: zero fingerprint rows both sides        (same)
  ok    empty table: explicit count(*)=0 (P5)                (both sides: 0)
  ok    empty table: one row inserted                        (differ, 0 bucket(s))
  ok    missing table: query error, never empty-PASS (P5)    (query error detected)
  ok    empty table: matches again after restore             (same)
T004 6.11 added / dropped column (P3):
  ok    added column: baseline identical                     (same)
  ok    added column: mapped-value hash stays same           (same)
  ok    added column: catalog detects it (green, P3)         (schema diff reports it)
  ok    coverage: unmapped column listed                     (extra)
  note  value PASS alone is not the verdict: schema diff must report the added column (P3)
  ok    dropped mapped column: error, never silent PASS      (query error detected)
  ok    dropped column: catalog detects it (red, P3)         (schema diff reports it)
  ok    added/dropped column: matches again after restore    (same)
T004 section 7 overflow bound:
  ok    s7: bucket slices unsigned (ffffffff=4294967295)     (both sides: 4294967295)
  ok    s7: sum(bigint) is exact numeric, no overflow        (both sides: numeric)
T004 6.12 different collations, same encoding (bank_c on old server):
  ok    collation: old bank_c is C                           (C)
  ok    collation: new bank is en_US.utf8                    (en_US.utf8)
  ok    encoding: old bank_c is UTF8                         (UTF8)
  ok    encoding: new bank is UTF8                           (UTF8)
  ok    collations: identical text stays quiet               (same)
  ok    collations: 'a' became 'A' (bytes catch it)          (differ, 1 bucket(s))
T004 6.13 SQL_ASCII (bank_ascii on old server, P19):
  ok    SQL_ASCII detected from pg_database (P19)            (SQL_ASCII)
  note  engine must FLAG SQL_ASCII for review — never silent PASS on the encoding (P19)
  ok    SQL_ASCII: pure-ASCII data stays quiet               (same)
  ok    SQL_ASCII: non-ASCII fingerprint errors (P19)        (query error detected)
  ok    SQL_ASCII: convert_to UTF8 rejects raw 0xE9          (query error detected)
  ok    SQL_ASCII: no conversion to LATIN1 (0xE9 stays)      (e9)
  ok    SQL_ASCII: chr(233) stored as 1 raw byte             (1)
  ok    UTF-8 side: chr(233) stored as 2 bytes (c3a9)        (2)
  ok    UTF-8 side: chr(233) converts to c3a9                (c3a9)
  ok    LATIN1 side: chr(233) converts to c3a9               (c3a9)
  ok    SQL_ASCII: matches again after undo                  (same)

Result: 83 passed, 0 failed
```

### 9.4 vs 17 (`prove.sh 5435 5434`)

Servers, as printed by `prove.sh`:

- old: 9.4.26, LATIN1, collate C — `127.0.0.1:5435`, container `dmux-old94`
- new: 17.12 (Debian 17.12-1.pgdg13+1), UTF8, collate en_US.utf8 — `127.0.0.1:5434`, container `dmux-new`

Full output (exit code 0):

```text
Servers:
  old: 9.4.26, LATIN1, collate C
  new: 17.12 (Debian 17.12-1.pgdg13+1), UTF8, collate en_US.utf8
Must stay quiet (identical data):
  ok    same data, other row order, column order, encoding   (same)
  ok    char(8) on old side, varchar(8) on new side          (same)
  ok    table with no key, duplicates on both sides          (same)
Must be reported (one change each, then undone):
  ok    one balance changed by 0.0001                        (differ, 1 bucket(s))
  ok    one character changed in a name                      (differ, 1 bucket(s))
  ok    empty text turned into NULL                          (differ, 1 bucket(s))
  ok    time of day dropped from a timestamp                 (differ, 1 bucket(s))
  ok    float changed in the last digit                      (differ, 1 bucket(s))
  ok    one byte changed in binary data                      (differ, 1 bucket(s))
  ok    two values swapped between columns of one row        (differ, 1 bucket(s))
  ok    one row deleted                                      (differ, 1 bucket(s))
  ok    no-key table: one of several duplicate rows deleted  (differ, 1 bucket(s))
  ok    no-key table: one duplicate pair added (XOR would miss it) (differ, 1 bucket(s))
After undoing every change:
  ok    accounts match again                                 (same)
  ok    no-key table matches again                           (same)
T004 6.1 timestamptz:
  ok    tz: same instant, different offsets                  (same)
  ok    tz: SET LOCAL UTC beats differing session TZ         (same)
  ok    tz: one second added                                 (differ, 1 bucket(s))
  ok    timestamp->timestamptz: value diff reported          (differ, 1 bucket(s))
  ok    timestamp->timestamptz: catalog flags type change    (schema diff reports it)
T004 6.2 money:
  ok    money: lc_monetary C vs en_US.utf8                   (same)
  ok    money: one cent added                                (differ, 1 bucket(s))
  ok    money::numeric is locale-free (never ::text)         (both sides: 1234.56)
  ok    lc_monetary recorded on both sides (P11)             (old=C, new=en_US.utf8)
  warn  lc_monetary differs — the engine must record this warning (P11)
T004 6.3 json / jsonb:
  ok    json: exact text identical                           (same)
  ok    json: one value changed                              (differ, 1 bucket(s))
  ok    jsonb: canonical form identical                      (same)
  ok    jsonb: one value changed                             (differ, 1 bucket(s))
  ok    json->jsonb: reorder reported, not quiet             (differ, 1 bucket(s))
  ok    json->jsonb: catalog flags type change               (schema diff reports it)
  ok    json->jsonb: duplicate-key dedup is data loss        (differ, 2 bucket(s))
T004 6.4 xml:
  ok    xml: identical under pinned xmloption                (same)
  ok    xml: attribute value changed                         (differ, 1 bucket(s))
T004 6.5 arrays (P7 regression: extra_float_digits=0):
  ok    arrays: float8[]+int[] incl NULL element             (same)
  ok    arrays: one float element changed                    (differ, 1 bucket(s))
  ok    arrays: one int element changed                      (differ, 1 bucket(s))
  ok    arrays: NULL element row changed                     (differ, 1 bucket(s))
T004 6.6 ranges:
  ok    ranges: int4range(1,5) vs '[1,4]' canonical          (same)
  ok    ranges: int4range upper bound changed                (differ, 1 bucket(s))
  ok    ranges: numrange upper bound changed                 (differ, 1 bucket(s))
T004 6.7 interval (P10 regression incl no-key template):
  ok    interval: iso_8601 identical                         (same)
  ok    interval: one second added                           (differ, 1 bucket(s))
  ok    interval: '24 hours' vs '1 day' trap                 (differ, 1 bucket(s))
  ok    interval no-key: iso_8601 set (P10 regression)       (same)
  ok    interval no-key: '24 hours' vs '1 day' trap          (differ, 1 bucket(s))
T004 6.8 numeric scale change (P4/P8):
  ok    numeric scale 4->6: both up-cast to 6                (same)
  ok    numeric scale 6: 0.0001 change caught                (differ, 1 bucket(s))
  ok    P4 guard: 1.2344/1.2345 distinct at scale 6          (both sides: 2)
  ok    P4 hazard: down-cast to scale 2 collapses them       (both sides: 1)
T004 6.9 NULL bytea:
  ok    bytea: NULL, empty and bytes identical               (same)
  ok    bytea: NULL became empty                             (differ, 1 bucket(s))
  ok    bytea: empty became NULL                             (differ, 1 bucket(s))
  ok    bytea: one byte changed (00ff->00fe)                 (differ, 1 bucket(s))
T004 6.10 empty table (P5):
  ok    empty table: zero fingerprint rows both sides        (same)
  ok    empty table: explicit count(*)=0 (P5)                (both sides: 0)
  ok    empty table: one row inserted                        (differ, 0 bucket(s))
  ok    missing table: query error, never empty-PASS (P5)    (query error detected)
  ok    empty table: matches again after restore             (same)
T004 6.11 added / dropped column (P3):
  ok    added column: baseline identical                     (same)
  ok    added column: mapped-value hash stays same           (same)
  ok    added column: catalog detects it (green, P3)         (schema diff reports it)
  ok    coverage: unmapped column listed                     (extra)
  note  value PASS alone is not the verdict: schema diff must report the added column (P3)
  ok    dropped mapped column: error, never silent PASS      (query error detected)
  ok    dropped column: catalog detects it (red, P3)         (schema diff reports it)
  ok    added/dropped column: matches again after restore    (same)
T004 section 7 overflow bound:
  ok    s7: bucket slices unsigned (ffffffff=4294967295)     (both sides: 4294967295)
  ok    s7: sum(bigint) is exact numeric, no overflow        (both sides: numeric)
T004 6.12 different collations, same encoding (bank_c on old server):
  ok    collation: old bank_c is C                           (C)
  ok    collation: new bank is en_US.utf8                    (en_US.utf8)
  ok    encoding: old bank_c is UTF8                         (UTF8)
  ok    encoding: new bank is UTF8                           (UTF8)
  ok    collations: identical text stays quiet               (same)
  ok    collations: 'a' became 'A' (bytes catch it)          (differ, 1 bucket(s))
T004 6.13 SQL_ASCII (bank_ascii on old server, P19):
  ok    SQL_ASCII detected from pg_database (P19)            (SQL_ASCII)
  note  engine must FLAG SQL_ASCII for review — never silent PASS on the encoding (P19)
  ok    SQL_ASCII: pure-ASCII data stays quiet               (same)
  ok    SQL_ASCII: non-ASCII fingerprint errors (P19)        (query error detected)
  ok    SQL_ASCII: convert_to UTF8 rejects raw 0xE9          (query error detected)
  ok    SQL_ASCII: no conversion to LATIN1 (0xE9 stays)      (e9)
  ok    SQL_ASCII: chr(233) stored as 1 raw byte             (1)
  ok    UTF-8 side: chr(233) stored as 2 bytes (c3a9)        (2)
  ok    UTF-8 side: chr(233) converts to c3a9                (c3a9)
  ok    LATIN1 side: chr(233) converts to c3a9               (c3a9)
  ok    SQL_ASCII: matches again after undo                  (same)

Result: 83 passed, 0 failed
```

### 9.4 vs 18 (`prove.sh 5435 5437`)

Servers, as printed by `prove.sh`:

- old: 9.4.26, LATIN1, collate C — `127.0.0.1:5435`, container `dmux-old94`
- new: 18.7 (Debian 18.7-1.pgdg13+2), UTF8, collate en_US.utf8 — `127.0.0.1:5437`, container `dmux-new18`

Full output (exit code 0):

```text
Servers:
  old: 9.4.26, LATIN1, collate C
  new: 18.7 (Debian 18.7-1.pgdg13+2), UTF8, collate en_US.utf8
Must stay quiet (identical data):
  ok    same data, other row order, column order, encoding   (same)
  ok    char(8) on old side, varchar(8) on new side          (same)
  ok    table with no key, duplicates on both sides          (same)
Must be reported (one change each, then undone):
  ok    one balance changed by 0.0001                        (differ, 1 bucket(s))
  ok    one character changed in a name                      (differ, 1 bucket(s))
  ok    empty text turned into NULL                          (differ, 1 bucket(s))
  ok    time of day dropped from a timestamp                 (differ, 1 bucket(s))
  ok    float changed in the last digit                      (differ, 1 bucket(s))
  ok    one byte changed in binary data                      (differ, 1 bucket(s))
  ok    two values swapped between columns of one row        (differ, 1 bucket(s))
  ok    one row deleted                                      (differ, 1 bucket(s))
  ok    no-key table: one of several duplicate rows deleted  (differ, 1 bucket(s))
  ok    no-key table: one duplicate pair added (XOR would miss it) (differ, 1 bucket(s))
After undoing every change:
  ok    accounts match again                                 (same)
  ok    no-key table matches again                           (same)
T004 6.1 timestamptz:
  ok    tz: same instant, different offsets                  (same)
  ok    tz: SET LOCAL UTC beats differing session TZ         (same)
  ok    tz: one second added                                 (differ, 1 bucket(s))
  ok    timestamp->timestamptz: value diff reported          (differ, 1 bucket(s))
  ok    timestamp->timestamptz: catalog flags type change    (schema diff reports it)
T004 6.2 money:
  ok    money: lc_monetary C vs en_US.utf8                   (same)
  ok    money: one cent added                                (differ, 1 bucket(s))
  ok    money::numeric is locale-free (never ::text)         (both sides: 1234.56)
  ok    lc_monetary recorded on both sides (P11)             (old=C, new=en_US.utf8)
  warn  lc_monetary differs — the engine must record this warning (P11)
T004 6.3 json / jsonb:
  ok    json: exact text identical                           (same)
  ok    json: one value changed                              (differ, 1 bucket(s))
  ok    jsonb: canonical form identical                      (same)
  ok    jsonb: one value changed                             (differ, 1 bucket(s))
  ok    json->jsonb: reorder reported, not quiet             (differ, 1 bucket(s))
  ok    json->jsonb: catalog flags type change               (schema diff reports it)
  ok    json->jsonb: duplicate-key dedup is data loss        (differ, 2 bucket(s))
T004 6.4 xml:
  ok    xml: identical under pinned xmloption                (same)
  ok    xml: attribute value changed                         (differ, 1 bucket(s))
T004 6.5 arrays (P7 regression: extra_float_digits=0):
  ok    arrays: float8[]+int[] incl NULL element             (same)
  ok    arrays: one float element changed                    (differ, 1 bucket(s))
  ok    arrays: one int element changed                      (differ, 1 bucket(s))
  ok    arrays: NULL element row changed                     (differ, 1 bucket(s))
T004 6.6 ranges:
  ok    ranges: int4range(1,5) vs '[1,4]' canonical          (same)
  ok    ranges: int4range upper bound changed                (differ, 1 bucket(s))
  ok    ranges: numrange upper bound changed                 (differ, 1 bucket(s))
T004 6.7 interval (P10 regression incl no-key template):
  ok    interval: iso_8601 identical                         (same)
  ok    interval: one second added                           (differ, 1 bucket(s))
  ok    interval: '24 hours' vs '1 day' trap                 (differ, 1 bucket(s))
  ok    interval no-key: iso_8601 set (P10 regression)       (same)
  ok    interval no-key: '24 hours' vs '1 day' trap          (differ, 1 bucket(s))
T004 6.8 numeric scale change (P4/P8):
  ok    numeric scale 4->6: both up-cast to 6                (same)
  ok    numeric scale 6: 0.0001 change caught                (differ, 1 bucket(s))
  ok    P4 guard: 1.2344/1.2345 distinct at scale 6          (both sides: 2)
  ok    P4 hazard: down-cast to scale 2 collapses them       (both sides: 1)
T004 6.9 NULL bytea:
  ok    bytea: NULL, empty and bytes identical               (same)
  ok    bytea: NULL became empty                             (differ, 1 bucket(s))
  ok    bytea: empty became NULL                             (differ, 1 bucket(s))
  ok    bytea: one byte changed (00ff->00fe)                 (differ, 1 bucket(s))
T004 6.10 empty table (P5):
  ok    empty table: zero fingerprint rows both sides        (same)
  ok    empty table: explicit count(*)=0 (P5)                (both sides: 0)
  ok    empty table: one row inserted                        (differ, 0 bucket(s))
  ok    missing table: query error, never empty-PASS (P5)    (query error detected)
  ok    empty table: matches again after restore             (same)
T004 6.11 added / dropped column (P3):
  ok    added column: baseline identical                     (same)
  ok    added column: mapped-value hash stays same           (same)
  ok    added column: catalog detects it (green, P3)         (schema diff reports it)
  ok    coverage: unmapped column listed                     (extra)
  note  value PASS alone is not the verdict: schema diff must report the added column (P3)
  ok    dropped mapped column: error, never silent PASS      (query error detected)
  ok    dropped column: catalog detects it (red, P3)         (schema diff reports it)
  ok    added/dropped column: matches again after restore    (same)
T004 section 7 overflow bound:
  ok    s7: bucket slices unsigned (ffffffff=4294967295)     (both sides: 4294967295)
  ok    s7: sum(bigint) is exact numeric, no overflow        (both sides: numeric)
T004 6.12 different collations, same encoding (bank_c on old server):
  ok    collation: old bank_c is C                           (C)
  ok    collation: new bank is en_US.utf8                    (en_US.utf8)
  ok    encoding: old bank_c is UTF8                         (UTF8)
  ok    encoding: new bank is UTF8                           (UTF8)
  ok    collations: identical text stays quiet               (same)
  ok    collations: 'a' became 'A' (bytes catch it)          (differ, 1 bucket(s))
T004 6.13 SQL_ASCII (bank_ascii on old server, P19):
  ok    SQL_ASCII detected from pg_database (P19)            (SQL_ASCII)
  note  engine must FLAG SQL_ASCII for review — never silent PASS on the encoding (P19)
  ok    SQL_ASCII: pure-ASCII data stays quiet               (same)
  ok    SQL_ASCII: non-ASCII fingerprint errors (P19)        (query error detected)
  ok    SQL_ASCII: convert_to UTF8 rejects raw 0xE9          (query error detected)
  ok    SQL_ASCII: no conversion to LATIN1 (0xE9 stays)      (e9)
  ok    SQL_ASCII: chr(233) stored as 1 raw byte             (1)
  ok    UTF-8 side: chr(233) stored as 2 bytes (c3a9)        (2)
  ok    UTF-8 side: chr(233) converts to c3a9                (c3a9)
  ok    LATIN1 side: chr(233) converts to c3a9               (c3a9)
  ok    SQL_ASCII: matches again after undo                  (same)

Result: 83 passed, 0 failed
```

### 9.6 vs 12 (`prove.sh 5433 5436`)

Servers, as printed by `prove.sh`:

- old: 9.6.24, LATIN1, collate C — `127.0.0.1:5433`, container `dmux-old`
- new: 12.22 (Debian 12.22-1.pgdg120+1), UTF8, collate en_US.utf8 — `127.0.0.1:5436`, container `dmux-new12`

Full output (exit code 0):

```text
Servers:
  old: 9.6.24, LATIN1, collate C
  new: 12.22 (Debian 12.22-1.pgdg120+1), UTF8, collate en_US.utf8
Must stay quiet (identical data):
  ok    same data, other row order, column order, encoding   (same)
  ok    char(8) on old side, varchar(8) on new side          (same)
  ok    table with no key, duplicates on both sides          (same)
Must be reported (one change each, then undone):
  ok    one balance changed by 0.0001                        (differ, 1 bucket(s))
  ok    one character changed in a name                      (differ, 1 bucket(s))
  ok    empty text turned into NULL                          (differ, 1 bucket(s))
  ok    time of day dropped from a timestamp                 (differ, 1 bucket(s))
  ok    float changed in the last digit                      (differ, 1 bucket(s))
  ok    one byte changed in binary data                      (differ, 1 bucket(s))
  ok    two values swapped between columns of one row        (differ, 1 bucket(s))
  ok    one row deleted                                      (differ, 1 bucket(s))
  ok    no-key table: one of several duplicate rows deleted  (differ, 1 bucket(s))
  ok    no-key table: one duplicate pair added (XOR would miss it) (differ, 1 bucket(s))
After undoing every change:
  ok    accounts match again                                 (same)
  ok    no-key table matches again                           (same)
T004 6.1 timestamptz:
  ok    tz: same instant, different offsets                  (same)
  ok    tz: SET LOCAL UTC beats differing session TZ         (same)
  ok    tz: one second added                                 (differ, 1 bucket(s))
  ok    timestamp->timestamptz: value diff reported          (differ, 1 bucket(s))
  ok    timestamp->timestamptz: catalog flags type change    (schema diff reports it)
T004 6.2 money:
  ok    money: lc_monetary C vs en_US.utf8                   (same)
  ok    money: one cent added                                (differ, 1 bucket(s))
  ok    money::numeric is locale-free (never ::text)         (both sides: 1234.56)
  ok    lc_monetary recorded on both sides (P11)             (old=C, new=en_US.utf8)
  warn  lc_monetary differs — the engine must record this warning (P11)
T004 6.3 json / jsonb:
  ok    json: exact text identical                           (same)
  ok    json: one value changed                              (differ, 1 bucket(s))
  ok    jsonb: canonical form identical                      (same)
  ok    jsonb: one value changed                             (differ, 1 bucket(s))
  ok    json->jsonb: reorder reported, not quiet             (differ, 1 bucket(s))
  ok    json->jsonb: catalog flags type change               (schema diff reports it)
  ok    json->jsonb: duplicate-key dedup is data loss        (differ, 2 bucket(s))
T004 6.4 xml:
  ok    xml: identical under pinned xmloption                (same)
  ok    xml: attribute value changed                         (differ, 1 bucket(s))
T004 6.5 arrays (P7 regression: extra_float_digits=0):
  ok    arrays: float8[]+int[] incl NULL element             (same)
  ok    arrays: one float element changed                    (differ, 1 bucket(s))
  ok    arrays: one int element changed                      (differ, 1 bucket(s))
  ok    arrays: NULL element row changed                     (differ, 1 bucket(s))
T004 6.6 ranges:
  ok    ranges: int4range(1,5) vs '[1,4]' canonical          (same)
  ok    ranges: int4range upper bound changed                (differ, 1 bucket(s))
  ok    ranges: numrange upper bound changed                 (differ, 1 bucket(s))
T004 6.7 interval (P10 regression incl no-key template):
  ok    interval: iso_8601 identical                         (same)
  ok    interval: one second added                           (differ, 1 bucket(s))
  ok    interval: '24 hours' vs '1 day' trap                 (differ, 1 bucket(s))
  ok    interval no-key: iso_8601 set (P10 regression)       (same)
  ok    interval no-key: '24 hours' vs '1 day' trap          (differ, 1 bucket(s))
T004 6.8 numeric scale change (P4/P8):
  ok    numeric scale 4->6: both up-cast to 6                (same)
  ok    numeric scale 6: 0.0001 change caught                (differ, 1 bucket(s))
  ok    P4 guard: 1.2344/1.2345 distinct at scale 6          (both sides: 2)
  ok    P4 hazard: down-cast to scale 2 collapses them       (both sides: 1)
T004 6.9 NULL bytea:
  ok    bytea: NULL, empty and bytes identical               (same)
  ok    bytea: NULL became empty                             (differ, 1 bucket(s))
  ok    bytea: empty became NULL                             (differ, 1 bucket(s))
  ok    bytea: one byte changed (00ff->00fe)                 (differ, 1 bucket(s))
T004 6.10 empty table (P5):
  ok    empty table: zero fingerprint rows both sides        (same)
  ok    empty table: explicit count(*)=0 (P5)                (both sides: 0)
  ok    empty table: one row inserted                        (differ, 0 bucket(s))
  ok    missing table: query error, never empty-PASS (P5)    (query error detected)
  ok    empty table: matches again after restore             (same)
T004 6.11 added / dropped column (P3):
  ok    added column: baseline identical                     (same)
  ok    added column: mapped-value hash stays same           (same)
  ok    added column: catalog detects it (green, P3)         (schema diff reports it)
  ok    coverage: unmapped column listed                     (extra)
  note  value PASS alone is not the verdict: schema diff must report the added column (P3)
  ok    dropped mapped column: error, never silent PASS      (query error detected)
  ok    dropped column: catalog detects it (red, P3)         (schema diff reports it)
  ok    added/dropped column: matches again after restore    (same)
T004 section 7 overflow bound:
  ok    s7: bucket slices unsigned (ffffffff=4294967295)     (both sides: 4294967295)
  ok    s7: sum(bigint) is exact numeric, no overflow        (both sides: numeric)
T004 6.12 different collations, same encoding (bank_c on old server):
  ok    collation: old bank_c is C                           (C)
  ok    collation: new bank is en_US.utf8                    (en_US.utf8)
  ok    encoding: old bank_c is UTF8                         (UTF8)
  ok    encoding: new bank is UTF8                           (UTF8)
  ok    collations: identical text stays quiet               (same)
  ok    collations: 'a' became 'A' (bytes catch it)          (differ, 1 bucket(s))
T004 6.13 SQL_ASCII (bank_ascii on old server, P19):
  ok    SQL_ASCII detected from pg_database (P19)            (SQL_ASCII)
  note  engine must FLAG SQL_ASCII for review — never silent PASS on the encoding (P19)
  ok    SQL_ASCII: pure-ASCII data stays quiet               (same)
  ok    SQL_ASCII: non-ASCII fingerprint errors (P19)        (query error detected)
  ok    SQL_ASCII: convert_to UTF8 rejects raw 0xE9          (query error detected)
  ok    SQL_ASCII: no conversion to LATIN1 (0xE9 stays)      (e9)
  ok    SQL_ASCII: chr(233) stored as 1 raw byte             (1)
  ok    UTF-8 side: chr(233) stored as 2 bytes (c3a9)        (2)
  ok    UTF-8 side: chr(233) converts to c3a9                (c3a9)
  ok    LATIN1 side: chr(233) converts to c3a9               (c3a9)
  ok    SQL_ASCII: matches again after undo                  (same)

Result: 83 passed, 0 failed
```

### 9.6 vs 15 (`prove.sh 5433 5438`)

Servers, as printed by `prove.sh`:

- old: 9.6.24, LATIN1, collate C — `127.0.0.1:5433`, container `dmux-old`
- new: 15.20 (Debian 15.20-1.pgdg13+1), UTF8, collate en_US.utf8 — `127.0.0.1:5438`, container `dmux-new15`

Full output (exit code 0):

```text
Servers:
  old: 9.6.24, LATIN1, collate C
  new: 15.20 (Debian 15.20-1.pgdg13+1), UTF8, collate en_US.utf8
Must stay quiet (identical data):
  ok    same data, other row order, column order, encoding   (same)
  ok    char(8) on old side, varchar(8) on new side          (same)
  ok    table with no key, duplicates on both sides          (same)
Must be reported (one change each, then undone):
  ok    one balance changed by 0.0001                        (differ, 1 bucket(s))
  ok    one character changed in a name                      (differ, 1 bucket(s))
  ok    empty text turned into NULL                          (differ, 1 bucket(s))
  ok    time of day dropped from a timestamp                 (differ, 1 bucket(s))
  ok    float changed in the last digit                      (differ, 1 bucket(s))
  ok    one byte changed in binary data                      (differ, 1 bucket(s))
  ok    two values swapped between columns of one row        (differ, 1 bucket(s))
  ok    one row deleted                                      (differ, 1 bucket(s))
  ok    no-key table: one of several duplicate rows deleted  (differ, 1 bucket(s))
  ok    no-key table: one duplicate pair added (XOR would miss it) (differ, 1 bucket(s))
After undoing every change:
  ok    accounts match again                                 (same)
  ok    no-key table matches again                           (same)
T004 6.1 timestamptz:
  ok    tz: same instant, different offsets                  (same)
  ok    tz: SET LOCAL UTC beats differing session TZ         (same)
  ok    tz: one second added                                 (differ, 1 bucket(s))
  ok    timestamp->timestamptz: value diff reported          (differ, 1 bucket(s))
  ok    timestamp->timestamptz: catalog flags type change    (schema diff reports it)
T004 6.2 money:
  ok    money: lc_monetary C vs en_US.utf8                   (same)
  ok    money: one cent added                                (differ, 1 bucket(s))
  ok    money::numeric is locale-free (never ::text)         (both sides: 1234.56)
  ok    lc_monetary recorded on both sides (P11)             (old=C, new=en_US.utf8)
  warn  lc_monetary differs — the engine must record this warning (P11)
T004 6.3 json / jsonb:
  ok    json: exact text identical                           (same)
  ok    json: one value changed                              (differ, 1 bucket(s))
  ok    jsonb: canonical form identical                      (same)
  ok    jsonb: one value changed                             (differ, 1 bucket(s))
  ok    json->jsonb: reorder reported, not quiet             (differ, 1 bucket(s))
  ok    json->jsonb: catalog flags type change               (schema diff reports it)
  ok    json->jsonb: duplicate-key dedup is data loss        (differ, 2 bucket(s))
T004 6.4 xml:
  ok    xml: identical under pinned xmloption                (same)
  ok    xml: attribute value changed                         (differ, 1 bucket(s))
T004 6.5 arrays (P7 regression: extra_float_digits=0):
  ok    arrays: float8[]+int[] incl NULL element             (same)
  ok    arrays: one float element changed                    (differ, 1 bucket(s))
  ok    arrays: one int element changed                      (differ, 1 bucket(s))
  ok    arrays: NULL element row changed                     (differ, 1 bucket(s))
T004 6.6 ranges:
  ok    ranges: int4range(1,5) vs '[1,4]' canonical          (same)
  ok    ranges: int4range upper bound changed                (differ, 1 bucket(s))
  ok    ranges: numrange upper bound changed                 (differ, 1 bucket(s))
T004 6.7 interval (P10 regression incl no-key template):
  ok    interval: iso_8601 identical                         (same)
  ok    interval: one second added                           (differ, 1 bucket(s))
  ok    interval: '24 hours' vs '1 day' trap                 (differ, 1 bucket(s))
  ok    interval no-key: iso_8601 set (P10 regression)       (same)
  ok    interval no-key: '24 hours' vs '1 day' trap          (differ, 1 bucket(s))
T004 6.8 numeric scale change (P4/P8):
  ok    numeric scale 4->6: both up-cast to 6                (same)
  ok    numeric scale 6: 0.0001 change caught                (differ, 1 bucket(s))
  ok    P4 guard: 1.2344/1.2345 distinct at scale 6          (both sides: 2)
  ok    P4 hazard: down-cast to scale 2 collapses them       (both sides: 1)
T004 6.9 NULL bytea:
  ok    bytea: NULL, empty and bytes identical               (same)
  ok    bytea: NULL became empty                             (differ, 1 bucket(s))
  ok    bytea: empty became NULL                             (differ, 1 bucket(s))
  ok    bytea: one byte changed (00ff->00fe)                 (differ, 1 bucket(s))
T004 6.10 empty table (P5):
  ok    empty table: zero fingerprint rows both sides        (same)
  ok    empty table: explicit count(*)=0 (P5)                (both sides: 0)
  ok    empty table: one row inserted                        (differ, 0 bucket(s))
  ok    missing table: query error, never empty-PASS (P5)    (query error detected)
  ok    empty table: matches again after restore             (same)
T004 6.11 added / dropped column (P3):
  ok    added column: baseline identical                     (same)
  ok    added column: mapped-value hash stays same           (same)
  ok    added column: catalog detects it (green, P3)         (schema diff reports it)
  ok    coverage: unmapped column listed                     (extra)
  note  value PASS alone is not the verdict: schema diff must report the added column (P3)
  ok    dropped mapped column: error, never silent PASS      (query error detected)
  ok    dropped column: catalog detects it (red, P3)         (schema diff reports it)
  ok    added/dropped column: matches again after restore    (same)
T004 section 7 overflow bound:
  ok    s7: bucket slices unsigned (ffffffff=4294967295)     (both sides: 4294967295)
  ok    s7: sum(bigint) is exact numeric, no overflow        (both sides: numeric)
T004 6.12 different collations, same encoding (bank_c on old server):
  ok    collation: old bank_c is C                           (C)
  ok    collation: new bank is en_US.utf8                    (en_US.utf8)
  ok    encoding: old bank_c is UTF8                         (UTF8)
  ok    encoding: new bank is UTF8                           (UTF8)
  ok    collations: identical text stays quiet               (same)
  ok    collations: 'a' became 'A' (bytes catch it)          (differ, 1 bucket(s))
T004 6.13 SQL_ASCII (bank_ascii on old server, P19):
  ok    SQL_ASCII detected from pg_database (P19)            (SQL_ASCII)
  note  engine must FLAG SQL_ASCII for review — never silent PASS on the encoding (P19)
  ok    SQL_ASCII: pure-ASCII data stays quiet               (same)
  ok    SQL_ASCII: non-ASCII fingerprint errors (P19)        (query error detected)
  ok    SQL_ASCII: convert_to UTF8 rejects raw 0xE9          (query error detected)
  ok    SQL_ASCII: no conversion to LATIN1 (0xE9 stays)      (e9)
  ok    SQL_ASCII: chr(233) stored as 1 raw byte             (1)
  ok    UTF-8 side: chr(233) stored as 2 bytes (c3a9)        (2)
  ok    UTF-8 side: chr(233) converts to c3a9                (c3a9)
  ok    LATIN1 side: chr(233) converts to c3a9               (c3a9)
  ok    SQL_ASCII: matches again after undo                  (same)

Result: 83 passed, 0 failed
```

### 9.6 vs 17 (`prove.sh 5433 5434`)

Servers, as printed by `prove.sh`:

- old: 9.6.24, LATIN1, collate C — `127.0.0.1:5433`, container `dmux-old`
- new: 17.12 (Debian 17.12-1.pgdg13+1), UTF8, collate en_US.utf8 — `127.0.0.1:5434`, container `dmux-new`

Full output (exit code 0):

```text
Servers:
  old: 9.6.24, LATIN1, collate C
  new: 17.12 (Debian 17.12-1.pgdg13+1), UTF8, collate en_US.utf8
Must stay quiet (identical data):
  ok    same data, other row order, column order, encoding   (same)
  ok    char(8) on old side, varchar(8) on new side          (same)
  ok    table with no key, duplicates on both sides          (same)
Must be reported (one change each, then undone):
  ok    one balance changed by 0.0001                        (differ, 1 bucket(s))
  ok    one character changed in a name                      (differ, 1 bucket(s))
  ok    empty text turned into NULL                          (differ, 1 bucket(s))
  ok    time of day dropped from a timestamp                 (differ, 1 bucket(s))
  ok    float changed in the last digit                      (differ, 1 bucket(s))
  ok    one byte changed in binary data                      (differ, 1 bucket(s))
  ok    two values swapped between columns of one row        (differ, 1 bucket(s))
  ok    one row deleted                                      (differ, 1 bucket(s))
  ok    no-key table: one of several duplicate rows deleted  (differ, 1 bucket(s))
  ok    no-key table: one duplicate pair added (XOR would miss it) (differ, 1 bucket(s))
After undoing every change:
  ok    accounts match again                                 (same)
  ok    no-key table matches again                           (same)
T004 6.1 timestamptz:
  ok    tz: same instant, different offsets                  (same)
  ok    tz: SET LOCAL UTC beats differing session TZ         (same)
  ok    tz: one second added                                 (differ, 1 bucket(s))
  ok    timestamp->timestamptz: value diff reported          (differ, 1 bucket(s))
  ok    timestamp->timestamptz: catalog flags type change    (schema diff reports it)
T004 6.2 money:
  ok    money: lc_monetary C vs en_US.utf8                   (same)
  ok    money: one cent added                                (differ, 1 bucket(s))
  ok    money::numeric is locale-free (never ::text)         (both sides: 1234.56)
  ok    lc_monetary recorded on both sides (P11)             (old=C, new=en_US.utf8)
  warn  lc_monetary differs — the engine must record this warning (P11)
T004 6.3 json / jsonb:
  ok    json: exact text identical                           (same)
  ok    json: one value changed                              (differ, 1 bucket(s))
  ok    jsonb: canonical form identical                      (same)
  ok    jsonb: one value changed                             (differ, 1 bucket(s))
  ok    json->jsonb: reorder reported, not quiet             (differ, 1 bucket(s))
  ok    json->jsonb: catalog flags type change               (schema diff reports it)
  ok    json->jsonb: duplicate-key dedup is data loss        (differ, 2 bucket(s))
T004 6.4 xml:
  ok    xml: identical under pinned xmloption                (same)
  ok    xml: attribute value changed                         (differ, 1 bucket(s))
T004 6.5 arrays (P7 regression: extra_float_digits=0):
  ok    arrays: float8[]+int[] incl NULL element             (same)
  ok    arrays: one float element changed                    (differ, 1 bucket(s))
  ok    arrays: one int element changed                      (differ, 1 bucket(s))
  ok    arrays: NULL element row changed                     (differ, 1 bucket(s))
T004 6.6 ranges:
  ok    ranges: int4range(1,5) vs '[1,4]' canonical          (same)
  ok    ranges: int4range upper bound changed                (differ, 1 bucket(s))
  ok    ranges: numrange upper bound changed                 (differ, 1 bucket(s))
T004 6.7 interval (P10 regression incl no-key template):
  ok    interval: iso_8601 identical                         (same)
  ok    interval: one second added                           (differ, 1 bucket(s))
  ok    interval: '24 hours' vs '1 day' trap                 (differ, 1 bucket(s))
  ok    interval no-key: iso_8601 set (P10 regression)       (same)
  ok    interval no-key: '24 hours' vs '1 day' trap          (differ, 1 bucket(s))
T004 6.8 numeric scale change (P4/P8):
  ok    numeric scale 4->6: both up-cast to 6                (same)
  ok    numeric scale 6: 0.0001 change caught                (differ, 1 bucket(s))
  ok    P4 guard: 1.2344/1.2345 distinct at scale 6          (both sides: 2)
  ok    P4 hazard: down-cast to scale 2 collapses them       (both sides: 1)
T004 6.9 NULL bytea:
  ok    bytea: NULL, empty and bytes identical               (same)
  ok    bytea: NULL became empty                             (differ, 1 bucket(s))
  ok    bytea: empty became NULL                             (differ, 1 bucket(s))
  ok    bytea: one byte changed (00ff->00fe)                 (differ, 1 bucket(s))
T004 6.10 empty table (P5):
  ok    empty table: zero fingerprint rows both sides        (same)
  ok    empty table: explicit count(*)=0 (P5)                (both sides: 0)
  ok    empty table: one row inserted                        (differ, 0 bucket(s))
  ok    missing table: query error, never empty-PASS (P5)    (query error detected)
  ok    empty table: matches again after restore             (same)
T004 6.11 added / dropped column (P3):
  ok    added column: baseline identical                     (same)
  ok    added column: mapped-value hash stays same           (same)
  ok    added column: catalog detects it (green, P3)         (schema diff reports it)
  ok    coverage: unmapped column listed                     (extra)
  note  value PASS alone is not the verdict: schema diff must report the added column (P3)
  ok    dropped mapped column: error, never silent PASS      (query error detected)
  ok    dropped column: catalog detects it (red, P3)         (schema diff reports it)
  ok    added/dropped column: matches again after restore    (same)
T004 section 7 overflow bound:
  ok    s7: bucket slices unsigned (ffffffff=4294967295)     (both sides: 4294967295)
  ok    s7: sum(bigint) is exact numeric, no overflow        (both sides: numeric)
T004 6.12 different collations, same encoding (bank_c on old server):
  ok    collation: old bank_c is C                           (C)
  ok    collation: new bank is en_US.utf8                    (en_US.utf8)
  ok    encoding: old bank_c is UTF8                         (UTF8)
  ok    encoding: new bank is UTF8                           (UTF8)
  ok    collations: identical text stays quiet               (same)
  ok    collations: 'a' became 'A' (bytes catch it)          (differ, 1 bucket(s))
T004 6.13 SQL_ASCII (bank_ascii on old server, P19):
  ok    SQL_ASCII detected from pg_database (P19)            (SQL_ASCII)
  note  engine must FLAG SQL_ASCII for review — never silent PASS on the encoding (P19)
  ok    SQL_ASCII: pure-ASCII data stays quiet               (same)
  ok    SQL_ASCII: non-ASCII fingerprint errors (P19)        (query error detected)
  ok    SQL_ASCII: convert_to UTF8 rejects raw 0xE9          (query error detected)
  ok    SQL_ASCII: no conversion to LATIN1 (0xE9 stays)      (e9)
  ok    SQL_ASCII: chr(233) stored as 1 raw byte             (1)
  ok    UTF-8 side: chr(233) stored as 2 bytes (c3a9)        (2)
  ok    UTF-8 side: chr(233) converts to c3a9                (c3a9)
  ok    LATIN1 side: chr(233) converts to c3a9               (c3a9)
  ok    SQL_ASCII: matches again after undo                  (same)

Result: 83 passed, 0 failed
```

### 9.6 vs 18 (`prove.sh 5433 5437`)

Servers, as printed by `prove.sh`:

- old: 9.6.24, LATIN1, collate C — `127.0.0.1:5433`, container `dmux-old`
- new: 18.7 (Debian 18.7-1.pgdg13+2), UTF8, collate en_US.utf8 — `127.0.0.1:5437`, container `dmux-new18`

Full output (exit code 0):

```text
Servers:
  old: 9.6.24, LATIN1, collate C
  new: 18.7 (Debian 18.7-1.pgdg13+2), UTF8, collate en_US.utf8
Must stay quiet (identical data):
  ok    same data, other row order, column order, encoding   (same)
  ok    char(8) on old side, varchar(8) on new side          (same)
  ok    table with no key, duplicates on both sides          (same)
Must be reported (one change each, then undone):
  ok    one balance changed by 0.0001                        (differ, 1 bucket(s))
  ok    one character changed in a name                      (differ, 1 bucket(s))
  ok    empty text turned into NULL                          (differ, 1 bucket(s))
  ok    time of day dropped from a timestamp                 (differ, 1 bucket(s))
  ok    float changed in the last digit                      (differ, 1 bucket(s))
  ok    one byte changed in binary data                      (differ, 1 bucket(s))
  ok    two values swapped between columns of one row        (differ, 1 bucket(s))
  ok    one row deleted                                      (differ, 1 bucket(s))
  ok    no-key table: one of several duplicate rows deleted  (differ, 1 bucket(s))
  ok    no-key table: one duplicate pair added (XOR would miss it) (differ, 1 bucket(s))
After undoing every change:
  ok    accounts match again                                 (same)
  ok    no-key table matches again                           (same)
T004 6.1 timestamptz:
  ok    tz: same instant, different offsets                  (same)
  ok    tz: SET LOCAL UTC beats differing session TZ         (same)
  ok    tz: one second added                                 (differ, 1 bucket(s))
  ok    timestamp->timestamptz: value diff reported          (differ, 1 bucket(s))
  ok    timestamp->timestamptz: catalog flags type change    (schema diff reports it)
T004 6.2 money:
  ok    money: lc_monetary C vs en_US.utf8                   (same)
  ok    money: one cent added                                (differ, 1 bucket(s))
  ok    money::numeric is locale-free (never ::text)         (both sides: 1234.56)
  ok    lc_monetary recorded on both sides (P11)             (old=C, new=en_US.utf8)
  warn  lc_monetary differs — the engine must record this warning (P11)
T004 6.3 json / jsonb:
  ok    json: exact text identical                           (same)
  ok    json: one value changed                              (differ, 1 bucket(s))
  ok    jsonb: canonical form identical                      (same)
  ok    jsonb: one value changed                             (differ, 1 bucket(s))
  ok    json->jsonb: reorder reported, not quiet             (differ, 1 bucket(s))
  ok    json->jsonb: catalog flags type change               (schema diff reports it)
  ok    json->jsonb: duplicate-key dedup is data loss        (differ, 2 bucket(s))
T004 6.4 xml:
  ok    xml: identical under pinned xmloption                (same)
  ok    xml: attribute value changed                         (differ, 1 bucket(s))
T004 6.5 arrays (P7 regression: extra_float_digits=0):
  ok    arrays: float8[]+int[] incl NULL element             (same)
  ok    arrays: one float element changed                    (differ, 1 bucket(s))
  ok    arrays: one int element changed                      (differ, 1 bucket(s))
  ok    arrays: NULL element row changed                     (differ, 1 bucket(s))
T004 6.6 ranges:
  ok    ranges: int4range(1,5) vs '[1,4]' canonical          (same)
  ok    ranges: int4range upper bound changed                (differ, 1 bucket(s))
  ok    ranges: numrange upper bound changed                 (differ, 1 bucket(s))
T004 6.7 interval (P10 regression incl no-key template):
  ok    interval: iso_8601 identical                         (same)
  ok    interval: one second added                           (differ, 1 bucket(s))
  ok    interval: '24 hours' vs '1 day' trap                 (differ, 1 bucket(s))
  ok    interval no-key: iso_8601 set (P10 regression)       (same)
  ok    interval no-key: '24 hours' vs '1 day' trap          (differ, 1 bucket(s))
T004 6.8 numeric scale change (P4/P8):
  ok    numeric scale 4->6: both up-cast to 6                (same)
  ok    numeric scale 6: 0.0001 change caught                (differ, 1 bucket(s))
  ok    P4 guard: 1.2344/1.2345 distinct at scale 6          (both sides: 2)
  ok    P4 hazard: down-cast to scale 2 collapses them       (both sides: 1)
T004 6.9 NULL bytea:
  ok    bytea: NULL, empty and bytes identical               (same)
  ok    bytea: NULL became empty                             (differ, 1 bucket(s))
  ok    bytea: empty became NULL                             (differ, 1 bucket(s))
  ok    bytea: one byte changed (00ff->00fe)                 (differ, 1 bucket(s))
T004 6.10 empty table (P5):
  ok    empty table: zero fingerprint rows both sides        (same)
  ok    empty table: explicit count(*)=0 (P5)                (both sides: 0)
  ok    empty table: one row inserted                        (differ, 0 bucket(s))
  ok    missing table: query error, never empty-PASS (P5)    (query error detected)
  ok    empty table: matches again after restore             (same)
T004 6.11 added / dropped column (P3):
  ok    added column: baseline identical                     (same)
  ok    added column: mapped-value hash stays same           (same)
  ok    added column: catalog detects it (green, P3)         (schema diff reports it)
  ok    coverage: unmapped column listed                     (extra)
  note  value PASS alone is not the verdict: schema diff must report the added column (P3)
  ok    dropped mapped column: error, never silent PASS      (query error detected)
  ok    dropped column: catalog detects it (red, P3)         (schema diff reports it)
  ok    added/dropped column: matches again after restore    (same)
T004 section 7 overflow bound:
  ok    s7: bucket slices unsigned (ffffffff=4294967295)     (both sides: 4294967295)
  ok    s7: sum(bigint) is exact numeric, no overflow        (both sides: numeric)
T004 6.12 different collations, same encoding (bank_c on old server):
  ok    collation: old bank_c is C                           (C)
  ok    collation: new bank is en_US.utf8                    (en_US.utf8)
  ok    encoding: old bank_c is UTF8                         (UTF8)
  ok    encoding: new bank is UTF8                           (UTF8)
  ok    collations: identical text stays quiet               (same)
  ok    collations: 'a' became 'A' (bytes catch it)          (differ, 1 bucket(s))
T004 6.13 SQL_ASCII (bank_ascii on old server, P19):
  ok    SQL_ASCII detected from pg_database (P19)            (SQL_ASCII)
  note  engine must FLAG SQL_ASCII for review — never silent PASS on the encoding (P19)
  ok    SQL_ASCII: pure-ASCII data stays quiet               (same)
  ok    SQL_ASCII: non-ASCII fingerprint errors (P19)        (query error detected)
  ok    SQL_ASCII: convert_to UTF8 rejects raw 0xE9          (query error detected)
  ok    SQL_ASCII: no conversion to LATIN1 (0xE9 stays)      (e9)
  ok    SQL_ASCII: chr(233) stored as 1 raw byte             (1)
  ok    UTF-8 side: chr(233) stored as 2 bytes (c3a9)        (2)
  ok    UTF-8 side: chr(233) converts to c3a9                (c3a9)
  ok    LATIN1 side: chr(233) converts to c3a9               (c3a9)
  ok    SQL_ASCII: matches again after undo                  (same)

Result: 83 passed, 0 failed
```

### How to reproduce (T008)

From the main checkout, with rootless Podman (ports bind to 127.0.0.1 only):

```
scripts/in-wt.sh T008 podman compose -f testdata/compose.yml --profile matrix up -d
# wait until all six containers are healthy:
scripts/in-wt.sh T008 podman ps --format '{{.Names}} {{.Status}}'
scripts/in-wt.sh T008 pg_isready -h 127.0.0.1 -p 5433 -U dmux_dev -d bank
scripts/in-wt.sh T008 pg_isready -h 127.0.0.1 -p 5434 -U dmux_dev -d bank
scripts/in-wt.sh T008 pg_isready -h 127.0.0.1 -p 5435 -U dmux_dev -d bank
scripts/in-wt.sh T008 pg_isready -h 127.0.0.1 -p 5436 -U dmux_dev -d bank
scripts/in-wt.sh T008 pg_isready -h 127.0.0.1 -p 5437 -U dmux_dev -d bank
scripts/in-wt.sh T008 pg_isready -h 127.0.0.1 -p 5438 -U dmux_dev -d bank
scripts/in-wt.sh T008 bash testdata/phase0/prove.sh 5435 5436   # 9.4 vs 12
scripts/in-wt.sh T008 bash testdata/phase0/prove.sh 5435 5438   # 9.4 vs 15
scripts/in-wt.sh T008 bash testdata/phase0/prove.sh 5435 5434   # 9.4 vs 17
scripts/in-wt.sh T008 bash testdata/phase0/prove.sh 5435 5437   # 9.4 vs 18
scripts/in-wt.sh T008 bash testdata/phase0/prove.sh 5433 5436   # 9.6 vs 12
scripts/in-wt.sh T008 bash testdata/phase0/prove.sh 5433 5438   # 9.6 vs 15
scripts/in-wt.sh T008 bash testdata/phase0/prove.sh 5433 5434   # 9.6 vs 17
scripts/in-wt.sh T008 bash testdata/phase0/prove.sh 5433 5437   # 9.6 vs 18
```

The first `up -d` with `--profile matrix` on a fresh machine pulls `postgres:9.4`,
`postgres:12`, `postgres:15` and `postgres:18`, which can take several minutes. Run
the pairs sequentially — never in parallel, because they share servers and `prove.sh`
recreates the `phase0` schema (and `bank_c`/`bank_ascii` on the old server of the
pair) per run. Each of the eight runs is expected to end with
`Result: 83 passed, 0 failed` and exit code 0.

## T011: harness exit-semantics hardening

**The disproven gate finding.** The Opus T008 gate (carry-forward #1) claimed
`prove.sh` always exits 0 even with FAILs. Maya DISPROVED this empirically: a
sabotaged copy — a version-dependent expression injected into `fingerprint.sql`'s
row hash, so identical data hashes differently on the two servers — ended with
`Result: 80 passed, 3 failed` and **exit code 1**. The exit guard
`[ "$FAIL" -eq 0 ]` was present and working all along.

**Root cause.** `prove.sh` had NO trailing EOF newline (as did 18 other harness
files). Line-based review tooling read the preceding `echo "Result: ..."` as the
last line and never saw the exit guard on the unterminated final line.

**What changed.**

1. Trailing newline added to every file under `testdata/phase0/` that lacked one
   (19 files, newline-only edits, verified with `diff`):
   `prove.sh`, `perf.sh`, and the 17 templates `fingerprint_arr.sql`,
   `fingerprint_ascii.sql`, `fingerprint_bin.sql`, `fingerprint_coll.sql`,
   `fingerprint_col.sql`, `fingerprint_empty.sql`, `fingerprint_iv.sql`,
   `fingerprint_iv_nokey.sql`, `fingerprint_json.sql`, `fingerprint_json_mix.sql`,
   `fingerprint_jsonb.sql`, `fingerprint_money.sql`, `fingerprint_numscale.sql`,
   `fingerprint_rng.sql`, `fingerprint_tz.sql`, `fingerprint_tz_mix.sql`,
   `fingerprint_xml.sql`. (`fingerprint.sql`, `fingerprint_nokey.sql`,
   `fingerprint_perf.sql`, `guard.sh`, `guard_test.sh` already ended in newline.)
   `prove.sh` is byte-identical apart from the final newline and the one
   display-only change in item 3.
2. NEW committed regression test `exit_test.sh` — Maya's sabotage experiment made
   permanent. It copies `prove.sh`, `guard.sh` and ALL `*.sql` templates to a
   `mktemp -d` (trap-cleaned), injects
   `bank_code::text || current_setting('server_version')` into the copy's
   `fingerprint.sql` row hash (asserting the pattern occurs exactly once, so a
   future template change fails loudly instead of silently unsabotaging), runs
   the copy against the default local mock pair, and asserts: exit code NON-ZERO;
   a `Result: N passed, M failed` line with M > 0; at least one `  FAIL  ` line;
   FAIL-line count equals M; and at least one `wanted same, got differ` FAIL (the
   sabotage signature). It then verifies the mocks are clean (no `phase0` schema
   on either server, no `bank_c`/`bank_ascii` on the old server), dropping and
   noting any leftovers. No row data in output; shellcheck-clean. Runtime ~90 s
   (one full 83-check run): **on-demand/nightly, not every CI build.**
3. Cosmetic (Opus T008 gate note 3), display only: the differ bucket count now
   counts the distinct buckets appearing on EITHER side of the diff instead of
   only old-side lines. `empty table: one row inserted` now prints
   `(differ, 1 bucket(s))` instead of the misleading `(differ, 0 bucket(s))`
   (the old side is empty there). One more display shifts:
   `interval no-key: '24 hours' vs '1 day' trap` now prints 2 buckets — honest,
   because the no-key template buckets by row hash, so the changed row moves
   between two buckets. No check's verdict, expectation or name changed; the
   verdict still comes from `cmp` alone.

**Re-run results (default pair, old 9.6 :5433 vs new 17 :5434).**

```
scripts/in-wt.sh T011 bash testdata/phase0/prove.sh       # Result: 83 passed, 0 failed — exit 0
scripts/in-wt.sh T011 bash testdata/phase0/guard_test.sh  # Result: 51 passed, 0 failed — exit 0
scripts/in-wt.sh T011 bash testdata/phase0/exit_test.sh   # Result: 8 passed, 0 failed  — exit 0
scripts/in-wt.sh T011 shellcheck testdata/phase0/prove.sh testdata/phase0/guard.sh \
  testdata/phase0/guard_test.sh testdata/phase0/perf.sh testdata/phase0/exit_test.sh   # clean
```

`exit_test.sh` output (the sabotaged copy reproduces Maya's evidence exactly):

```
Sabotaged copy ready (row hash now includes server_version). Running it against the mocks...

Assertions on the sabotaged run:
  ok    harness exited non-zero with FAILs (exit 1)
  ok    summary line present with failures: Result: 80 passed, 3 failed
  ok    at least one '  FAIL  ' line (3 printed)
  ok    FAIL line count matches the summary (3 = 3)
  ok    sabotage signature present: identical data reported as differ

Mock cleanup verification:
  ok    old: phase0 schema dropped by the harness, verified gone
  ok    new: phase0 schema dropped by the harness, verified gone
  ok    old: bank_c/bank_ascii dropped by the harness, verified gone

Result: 8 passed, 0 failed
```

The three FAILs in the sabotaged run are the three "identical data stays quiet /
matches again" checks that use `fingerprint.sql`; every planted-change check
still passes (the injection leaves bucket assignment untouched).

**Cleanup confirmation.** After the sabotaged run, `exit_test.sh` verified no
`phase0` schema on either mock and no `bank_c`/`bank_ascii` on the old server —
the copied harness dropped all of them itself despite the FAILs (FAILs never
abort the run before cleanup). The subsequent clean `prove.sh` re-run also ended
83/83 with the mocks clean.

**How to run the exit-semantics regression (on demand or nightly):**

```
scripts/in-wt.sh T011 bash testdata/phase0/exit_test.sh            # default pair 9.6 vs 17
scripts/in-wt.sh T011 bash testdata/phase0/exit_test.sh 5435 5437  # any local pair, like prove.sh
```

## T009: dictionary-name scrub

Date: 2026-11-16 · Run by: Nina

Final-audit finding F1 (DECISIONS #11): the phase-0 `accounts`/`accounts_perf`
columns copied names from a licensed data dictionary. The nine dictionary-derived
names were renamed to invented names in `prove.sh`, `perf.sh`, `fingerprint.sql`,
`fingerprint_perf.sql`, `exit_test.sh` and the quoted prose above. The columns are
now `acct_id`, `bank_code`, `holder_name`, `branch_code`, `balance`, `rate`,
`opened_at`, `doc_blob`, `remarks` (the old names are deliberately not reproduced
here — DECISIONS #11 forbids derived detail in public files). Table names, column
widths/types, literals, check names, value generation and the `ROW(...)` column
order are unchanged, so test semantics are identical. Recorded results remain
byte-valid: `ROW(...)::text` hashes VALUES, not column names.

Verification: all eight matrix pairs re-run (83 passed, 0 failed, exit 0), and each
fresh output is byte-identical to a pre-rename baseline run of the same pair.
`exit_test.sh` 8/8 (sabotaged copy exits 1, "80 passed, 3 failed"), `guard_test.sh`
51/51, `perf.sh` at 100k/200k rows (cross-server fingerprint identity and cleanup
verified). shellcheck and check-eof clean; no old name remains under `testdata/`.
Note: the T008 fenced blocks above predate T011's display-only bucket-count change,
so fresh output differs from them on exactly two lines (`empty table: one row
inserted` 0→1 bucket, `interval no-key … trap` 1→2 buckets) — the same two lines
before and after this scrub; no verdict or hash changed.
