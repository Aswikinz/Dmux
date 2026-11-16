# Test data

Everything here is for the local mock databases. Nothing in this folder may be pointed
at a real system.

## Databases

| Name | Postgres | Port | Encoding | Started by |
|---|---|---|---|---|
| old | 9.6 | 5433 | LATIN1, C sort order | default |
| new | 17 | 5434 | UTF-8 | default |
| old94 | 9.4 | 5435 | LATIN1, C sort order | `--profile matrix` |
| new12 | 12 | 5436 | UTF-8 | `--profile matrix` |
| new15 | 15 | 5438 | UTF-8 | `--profile matrix` |
| new18 | 18 | 5437 | UTF-8 | `--profile matrix` |

Address pattern: `postgres://dmux_dev:dmux_dev_only@127.0.0.1:<port>/bank`

```
podman compose -f testdata/compose.yml up -d
podman compose -f testdata/compose.yml --profile matrix up -d
podman compose -f testdata/compose.yml down -v      # stop and delete the data
```

### Extra databases created by the proof (T004)

`phase0/prove.sh` also creates two throwaway databases **on the old server of the
pair** (default :5433, or whichever old port is passed), for the collation and
SQL_ASCII cases. They need no compose services, no new ports and no new images:

| Name | Encoding | Collation | Purpose |
|---|---|---|---|
| bank_c | UTF8 | C (LC_COLLATE and LC_CTYPE) | same encoding as new `bank`, different collation (case 6.12) |
| bank_ascii | SQL_ASCII | C | encoding passthrough/validation case (6.13, P19) |

Both are created with `CREATE DATABASE ... TEMPLATE template0`, dropped and
recreated on every run, and dropped again at the end of a successful run. The new
side of both comparisons is the existing `bank` (UTF8, en_US.utf8). The existing
`bank` databases and the container configuration are never modified.

## Phase 0 proof

`phase0/prove.sh` loads the same 20,000 accounts into the old and the new database,
with accented names, NULLs, empty strings, floats, timestamps and binary data. The two
sides differ on purpose in encoding, row order, column order and one column type. It
then checks that:

- identical data gives an identical fingerprint, and
- each single planted change (a balance off by 0.0001, one character, NULL for empty,
  a dropped time of day, the last digit of a float, one byte of binary, two swapped
  values, a deleted row, a missing or added duplicate) changes exactly one bucket.

**Extended in T004** with the 13 cases from the T003 SQL review (§6) plus two
overflow-bound probes (§7): timestamptz (same instant, different offsets; session-TZ
normalisation; timestamp→timestamptz type change), money (`::numeric`, never `::text`;
lc_monetary recorded), json/jsonb (exact text vs canonical; json→jsonb reorder and
duplicate-key dedup must NOT stay quiet), xml (pinned xmloption), arrays (float8[] +
int[] with NULL elements — the P7 `extra_float_digits=0` regression), ranges
(canonical int4range, numrange), interval (iso_8601, the '24 hours' vs '1 day' trap,
also through the no-key template — the P10 regression), numeric scale change (both
sides up-cast to the larger scale; the P4 never-cast-down guard), NULL bytea (both
directions), empty table (explicit count=0; a missing table must error, never
empty-PASS), added/dropped column (value hash stays same but catalog/schema diff must
report it), different collations with the same encoding (`bank_c`), and SQL_ASCII
(`bank_ascii`: pure ASCII stays quiet; non-ASCII makes the fingerprint error loudly;
the engine must FLAG the encoding, never silent PASS).

```
testdata/phase0/prove.sh              # old 9.6 against new 17
testdata/phase0/prove.sh 5435 5437    # old 9.4 against new 18
```

The script creates and drops a schema called `phase0` (and the throwaway `bank_c` /
`bank_ascii` databases on the old server). Since T010 a shared guard (`phase0/guard.sh`)
refuses any URL that is not a strict single-host local address
`postgres://<user>:<password>@<host>:<port>/<dbname>` with host `127.0.0.1` or `localhost`
and a numeric port — including libpq multi-host comma lists and `?host=`/`?port=`/`?service=`
query overrides that could route elsewhere. `phase0/guard_test.sh` is a database-free
regression battery for the guard. `phase0/exit_test.sh` (T011) is an on-demand/nightly
regression test (~90 s, needs the mocks): it runs a sabotaged copy of the harness and
asserts it exits non-zero when checks FAIL.

`phase0/fingerprint.sql` and `phase0/fingerprint_nokey.sql` are the reference queries;
the 17 `fingerprint_*.sql` templates added in T004 cover the new test tables. All of
them carry the canonical settings block from the T003 review §4 (pinned search_path,
UTC, ISO dates, iso_8601 intervals, hex bytea, extra_float_digits=0, xmloption,
statement/lock timeouts, application_name) and produce the same order-independent
`bucket|n|s1|s2|s3|s4` output. The engine must generate SQL of the same shape.

**Checked so far:** T001/T002 — the original 15 checks pass on all six pairs of
old {9.4, 9.6} × new {12, 17, 18}. T004 — the extended proof (83 checks: 15 original
+ 68 new) passes on 9.6→17 and on the 9.4→18 extremes smoke. T008 — Postgres 15
joins the matrix (`new15`, :5438) and the extended proof passes on all eight pairs
of old {9.4, 9.6} × new {12, 15, 17, 18}, so every version pair now covers all
normalisation cases. See `phase0/RESULTS.md`.

## Performance timing (T005)

`phase0/perf.sh` times the reference fingerprint on large tables, for the Phase 3
estimate calibration (PLAN.md §4.5: estimates use size on disk) and nightly
large-dataset checks (§8.1 "large"). For each size (default 1,000,000 and
20,000,000 rows) on each server it loads `phase0.accounts_perf` — the
`phase0.accounts` column shape, UNLOGGED, no PK/indexes (the key columns are still
hashed) — in 1M-row `INSERT ... SELECT generate_series` batches, then times the
wall clock of `psql -f fingerprint_perf.sql` twice (first scan after load, then a
repeat), strictly sequentially — autovacuum is disabled on the perf table and the
harness waits for other active backends to finish before timing. It records
`count(*)`, `pg_total_relation_size`,
rows/s and MiB/s, checks the two servers' fingerprints are byte-identical, prints
a results table, and drops the `phase0` schema again (verified). Like `prove.sh`
it WRITES and uses the same strict local-only URL guard (`phase0/guard.sh`, T010).

```
testdata/phase0/perf.sh                          # 1M + 20M rows, old 9.6 vs new 17
testdata/phase0/perf.sh 5435 5437 1000000        # other local ports, custom sizes
PERF_SIZES="5000000" PERF_BATCH=500000 testdata/phase0/perf.sh
```

`fingerprint_perf.sql` is `fingerprint.sql`'s exact shape and canonical settings
block (T003 §4) against `accounts_perf`; the only deviation is
`statement_timeout='1800s'` instead of `'300s'`, which is too tight for a 20M-row
scan (the engine uses the run window). Results: `phase0/RESULTS.md`, T005 section.

## Still to build (Nina, Phase 1)

- `demo_bank/`: the invented public schema and its generator
- `build.py`: one command to build both databases at a chosen size
- `expected_diffs.json`: the answer key (PLAN.md section 8.2)
