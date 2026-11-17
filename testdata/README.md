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

## Public schema: `demo_bank` (T015)

`demo_bank/` is the invented banking layout of PLAN.md 8.1: the only schema public
tests use. Nothing in it comes from a real system or a licensed dictionary
(decision 11); the table, column and code names are made up, and the data is
synthetic.

**Shape.** 41 tables, about 118,000 rows in the `small` size (20,000 transactions).
Everything is Postgres 9.4-compatible — no `IDENTITY`, no generated columns, no
declarative partitioning — because the old database of the pair is 9.6 and the
schema is loaded there first.

| Feature | Where |
|---|---|
| primary keys, composite keys, a composite foreign key | all tables; `account_holders`, `balances_daily`, `payment_items`; `transactions (account_id, currency_code)` → `accounts` |
| foreign keys, including a self-reference | `gl_accounts.parent_id` → `gl_accounts` |
| UNIQUE, CHECK, partial and functional indexes | throughout; `ux_customer_addresses_home`, `ix_customers_name` |
| a table with **no key** (duplicates legal) | `audit_log` (10,495 rows) |
| sequences (4) | `seq_branch`, `seq_gl_entry`, `seq_note`, `seq_dispute` |
| 2 views, 1 materialised view | `v_account_balances`, `v_open_disputes`, `mv_branch_totals` |
| plpgsql function + trigger | `fn_log_account_change` on `accounts` (writes `audit_log`) |
| one large object | oid 7654321, referenced by `customer_documents.content_oid` |
| types | `char(1)` flags, `char(n)`, `varchar(n)`, `numeric(20,4)`, one `money` column (`loans.penalty_charge`), `float8`/`real`, `timestamp(0)`, `timestamptz`, `date`, `bytea`, `json`, `jsonb`, `text[]`, `interval`, `numrange` |

**Determinism.** `data.sql` never calls `random()`, `now()` or `current_date`. Every
value derives from the row number `g` and `md5(g)`; the large object is created with
a fixed oid (a generated one would change on every build, because the cluster-wide
counter is not reset by `DROP DATABASE`); and the trigger takes its timestamp from
the row's own `updated_at`, never from `now()`. Accented characters are written as
`chr(<code>)` with codes 160–255 only, so the same pure-ASCII file loads unchanged
into the LATIN1 old database and the UTF-8 new one. The data set mixes NULLs, empty
strings and trailing spaces on purpose.

```
testdata/demo_bank/build.sh                 # old 9.6 (:5433) -> new 17 (:5434), planted
testdata/demo_bank/build.sh --no-plant      # identical data on both sides (verify.sh's build)
testdata/demo_bank/build.sh 5435 5437       # old 9.4 -> new 18, other local ports
testdata/demo_bank/verify.sh                # --no-plant build: old vs new, per-table digests
testdata/demo_bank/verify.sh --baseline /tmp/demo_bank.digest   # + same data every run
testdata/demo_bank/check_key.sh             # planted build: prove expected_diffs.json
```

`build.sh` drops and recreates the database **`demo_bank`** on the old server
(LATIN1, C sort order), loads `schema.sql` then `data.sql`, dumps it with
`pg_dump -Fc`, drops and recreates `demo_bank` on the new server (UTF-8), restores
with `pg_restore`, and `ANALYZE`s both sides. It then applies `plant.sql` to the
**new** database only (the PLAN.md 8.2 differences); `--no-plant` skips that step
and keeps the two sides identical. It prints each server's version, encoding and
collation, and a table/row summary. It only ever touches the database named
`demo_bank` (plus the `postgres` maintenance database used to create it) and
guards all four URLs with `phase0/guard.sh` — it can never reach the `bank`
databases or anything that is not a local mock.

`verify.sh` computes one digest per table on each side:
`count(*) | md5(string_agg(row_hash, <empty> ORDER BY row_hash COLLATE "C"))`, where
`row_hash` is `md5(convert_to(ROW(<normalised columns>)::text, 'UTF8'))`. Ordering is
by the hash with an explicit `C` collation, so neither the physical row order nor the
servers' text sort order can change the result; floats and `bytea` are hashed from
their bytes and `money` is cast to `numeric` (PLAN.md 3.2, decision 4). It asserts
that old and new agree on every table and that both sides have the same table set.
With `--baseline FILE` it also compares against an earlier run's digests, which is
how "two builds produce the same data" is proved; the file is written on first use.
It exits non-zero on any mismatch. The same digests come out of the 9.4 → 18 pair,
so the data set is identical across versions too. Run it on a `--no-plant` build:
the planted build is meant to differ.

Checked: build + verify green on 9.6 → 17 and on 9.4 → 18; two consecutive
builds gave identical digests; a planted 0.01 `money` change, a NULL → empty-string
change and a trailing-space change were each reported, while a reversed physical row
order stayed quiet.

### Answer key (T016)

`expected_diffs.json` is the answer key of PLAN.md section 8.2: one versioned file
(`version: 1`, `dataset: "demo_bank-small"`) with three lists.

| List | Entries | Meaning |
|---|---|---|
| `must_report` | 15 (`m01`–`m15`) | every planted difference, with exact counts, keys, values and types |
| `must_stay_quiet` | 6 (`q01`–`q06`) | identical data a naive tool would report; `q06` also expects an amber type change |
| `runtime` | 1 (`r01`) | the one case that cannot be static: a row updated while the check runs |

Every entry has `id`, `plan_ref` (`"8.2 row 1"`…`"8.2 row 13"`; the quiet rows are
`"8.2 quiet row 1"`…), `object` (`schema.table[.column]`), `kind`, `expected` and
`level` (`structure`, `counts` or `full_values`). The `expected` values are read
straight from the planted databases and are exact — they use only invented
`demo_bank` data, never a real name or a real row (decision 11).

`plant.sql` plants them, in the new database only, after `build.sh`'s copy:

| PLAN.md 8.2 | Planted change |
|---|---|
| row 1 | 12 rows deleted from `transactions` (ids listed in `m01`) |
| row 2 | 3 duplicate rows added to the keyless `audit_log` |
| row 3 | `accounts.balance` +0.01 on account 4999 (the audit trigger disabled for the one update) |
| row 4 | one character changed in `transactions.narration` (19997) |
| row 5 | an empty `transactions.narration` (7) turned into NULL |
| row 6 | time of day dropped from `transactions.value_timestamp` (20000) |
| row 7 | `user_roles.granted_by` dropped; `loans.settlement_ref` added |
| row 8 | table `limits` → `product_limits`; column `fees.fee_name` → `fee_label` |
| row 9 | `branches.city` widened from `varchar(60)` to `varchar(120)` (same amber class as the plan's 50 → 100) |
| row 10 | `seq_gl_entry` set back to 7975 (old next value 8001, new 7976) |
| row 11 | `ix_transactions_posted` dropped; `ck_transactions_fee` dropped and re-added `NOT VALID` |
| row 12 | large object 7654321 removed (row 1 of `customer_documents` still points at it) |
| row 13 | runtime only: see below |

The stay-quiet cases: `balances_daily` becomes a declaratively partitioned table on
the new side (three partitions, same name, same constraint and index names), the
columns of `holidays` are reordered, and `kyc_checks.result` moves from `char(1)` to
`varchar(1)`. The LATIN1 → UTF-8 accents, the C vs `en_US.utf8` sort order and the
9.6 vs 17 float print need no planting: they are the old/new server difference
itself, and `check_key.sh` proves they are present.

`check_key.sh` is the database half of the answer-key test. It reads
`expected_diffs.json`, repeats every claim as a read-only query against the two
databases (guarded URLs, canonical settings block), and also re-runs `verify.sh`'s
per-table digest for every table, mapping `limits` → `product_limits` and reading the
partitioned `balances_daily` through its parent. It proves the key is true before the
engine is graded against it, checks that every key entry is covered by a check, and
exits non-zero on any mismatch. For `transactions`, `audit_log` and `accounts` it
also proves that only the planted rows moved.

The runtime case cannot be a static plant. `concurrent_update.sh 60` updates one
invented `notes` row (note 1500) in the new database in a loop for 60 seconds, one
committed transaction per update. The engine test harness starts it just after the
first table's counts are read; the run must end as **changed during check** (amber),
never PASS. It writes on purpose; it is harness code, guarded to local mocks.

The whole answer-key build is one command:

```
testdata/demo_bank/build.sh        # build, copy, plant
testdata/demo_bank/check_key.sh    # 129 checks, exit 0 only when the key is true
```

## Still to build (Nina, Phase 1)

- `demo_bank/`: the `full` and `large` sizes (PLAN.md 8.1); `small` exists (T015)
