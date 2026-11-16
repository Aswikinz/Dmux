# T005 — SQL re-review (fingerprint perf template + timing harness)

Reviewer: Priya · Date: 2026-11-16 · Branch: `task/T005-timing`, commit `2f00aa1`
Note (T009 scrub): test column names quoted here were renamed to invented names before publication; tests and findings are unchanged.
Scope (AGENTS.md rule 3 re-review, narrower than T003/T004): NEW
`testdata/phase0/fingerprint_perf.sql`, NEW `testdata/phase0/perf.sh`, the appended
T005 section of `testdata/phase0/RESULTS.md`, and the perf note in `testdata/README.md`.
Reference documents: T003 review §4 (canonical settings block), `fingerprint.sql`
(reference shape), DECISIONS #2/#3/#4/#10, PLAN §§3.1, 4.5, 8.1.

Empirical checks below ran against the local mock servers only (9.4.26 :5435,
9.6.24 :5433, 12.22 :5436, 17.12 :5434, 18.7 :5437), psql 18.7. Writes happened only
through the sanctioned harness (`phase0` schema on the mocks, dropped and verified
afterwards). Everything else was read-only. No secrets or row data appear here.

---

## 1. Verdict

**APPROVE.** The template is the reference fingerprint with the full canonical
settings block and exactly one documented deviation (`statement_timeout='1800s'`,
harness window). The harness confines every write to the mock `phase0` schema,
never runs ANALYZE or relies on statistics, leaks no query text or row data,
asserts cross-server identity on the FULL fingerprint output byte-for-byte, and
was verified end-to-end by me at small sizes on 9.6×17 **and** 9.4×18 (the 9.4
extreme proves rule-3 safety empirically, not just by inspection).

Three findings, none blocking (§5): one inherited guard gap shared with the
already-merged `prove.sh` (follow-up task), one RESULTS wording inaccuracy, one
cosmetic newline. No false-PASS or false-FAIL path found.

---

## 2. Template review — `fingerprint_perf.sql`

A line-by-line `diff -u` against `testdata/phase0/fingerprint.sql` shows exactly
four differences: the header comment, the `statement_timeout` line (+ its comment),
the table name (`phase0.accounts` → `phase0.accounts_perf`, the point of the
variant), and a missing trailing newline (R3). Everything else is byte-identical.

**Settings block (L11–22) vs T003 §4 — complete and correct:**

| Setting | Value | Matches §4 |
|---|---|---|
| transaction | `BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY` | yes |
| `search_path` | `pg_catalog`, **first SET LOCAL** (P1) | yes |
| `timezone` | `'UTC'` (P9) | yes |
| `datestyle` | `'ISO, YMD'` | yes |
| `intervalstyle` | `'iso_8601'` (P10) | yes |
| `bytea_output` | `'hex'` | yes |
| `extra_float_digits` | `0` (P7) | yes |
| `xmloption` | `'document'` (P16) | yes |
| `statement_timeout` | `'1800s'` — **the only deviation** (reference `'300s'`) | documented |
| `lock_timeout` | `'5s'` | yes |
| `application_name` | `'dmux'` | yes |

The deviation is documented in three places: the SQL comment (L19–20, "engine uses
the run window"), `testdata/README.md`, and RESULTS.md L700–704. The headroom
arithmetic checks out: (300 − 214.721) / 300 ≈ 28.4%, matching "~28% headroom".
1800 s is a harness window only; it does not touch the engine's run-window design.

**Shape (L24–51) — identical to the reference:**
- `bucket = ('x'||substr(kh,1,3))::bit(12)::int` → 4096 key-hash buckets (P23).
- `count(*)` plus four slices `substr(rh, 1/9/17/25, 8)::bit(32)::bigint` — all 32
  md5 hex chars, unsigned 32-bit, `sum(bigint)` → exact numeric (T003 §7 bound:
  20M × 4294967295 ≈ 8.6×10¹⁶, far below i64 max).
- `GROUP BY 1` / `ORDER BY 1` reference the **integer** bucket; no text ordering
  anywhere (P24, rule 4). Output `bucket|n|s1|s2|s3|s4`.
- Key hash `md5(convert_to(ROW(acct_id::text, bank_code::text)::text,'UTF8'))` (L34);
  the key columns are also the first two fields of the row hash (L38–39) — no
  key-omission false PASS.
- Per-column normalisation (L37–47) is byte-identical to the reference for the
  same column shape: varchar exact `::text`, char(n) pad-stripping `::text`,
  numeric printed form, `encode(float8send(rate),'hex')` bytes, timestamp ISO
  under UTC, `md5(doc_blob)` raw bytea bytes, NULL/'' kept apart by ROW rendering.
- **md5 unchanged** — DECISIONS #10 respected; no reopening.
- Table schema-qualified (`phase0.accounts_perf`); functions resolve under the
  pinned `pg_catalog` path.

**9.4-safe (rule 3):** only ≤9.4 syntax in the template. Verified empirically, not
just by inspection: I ran the harness on 9.4.26 (:5435) × 18.7 (:5437) at 100k and
200k rows — the template executed cleanly on 9.4 and the fingerprints were
byte-identical to 18.7 at both sizes (§6, run 2).

---

## 3. Harness query review — `perf.sh`

**Write inventory (complete; grep-verified).** The only statements that write:
- `DROP SCHEMA IF EXISTS phase0 CASCADE` — trap (L56), run start (L160), cleanup (L202);
- `CREATE SCHEMA phase0` (L160);
- `CREATE UNLOGGED TABLE phase0.accounts_perf (...) WITH (autovacuum_enabled = off)`
  (L161–164) — UNLOGGED is 9.1+, the reloption ran fine on 9.4.26;
- batched `INSERT INTO phase0.accounts_perf ... SELECT ... generate_series($lo,$hi)`
  (L102–109), one implicit transaction per batch, bounds shell-validated as
  positive integers (L44–47).

Nothing else is written: no ANALYZE, no VACUUM, no UPDATE/DELETE/ALTER/TRUNCATE,
no indexes, no writes outside `phase0`. The load value expressions are verbatim
from `prove.sh`'s `load_accounts` (only the `generate_series` bounds differ; the
`ORDER BY g` is dropped, which is fine — the fingerprint is order-independent).
Column shape equals `phase0.accounts` (old-side shape) on BOTH servers; no PK, but
the key columns are still hashed. No statistics reliance anywhere: `count(*)` is
exact, sizes come from `pg_total_relation_size`, and EXPLAIN is used only for plan
shape — consistent with PLAN §4.5 ("a freshly loaded database has no statistics").

**URL guard (L33–38).** Runs before any connection, covers `OLD_URL`/`NEW_URL`
overrides, and accepts 127.0.0.1/localhost with any port (or none). Plain remote
URLs are refused (probe-verified). It is byte-identical to the merged `prove.sh`
guard. Two exotic-but-valid libpq URL forms slip past it — see R1; inherited, not
a T005 regression, and not blocking.

**Quiescence (L135–151) — no leaks (rule 7).** `active_info` reads
`pg_stat_activity` (`state='active'`, `pid <> pg_backend_pid()`,
`datname = current_database()`) and emits only the labels `autovacuum`/`other` via
`CASE WHEN query LIKE 'autovacuum:%'` inside `string_agg` — query text is never
selected into output, never printed. `wait_quiet` waits ≤30 s in 5 s steps, then
warns (does not fail) — matching the recorded first-run contamination history.
`dmux_dev` is superuser in the mocks, so the autovacuum classification sees the
text; under a non-superuser it would conservatively label `other` — still no leak.
`pg_stat_activity.state/query`, `string_agg`, `pg_backend_pid()` are all 9.4-safe.

**EXPLAIN (L83–91) — read-only.** `sed` extracts the exact `SELECT ... ORDER BY 1;`
block from the template and wraps it in `BEGIN ISOLATION LEVEL REPEATABLE READ
READ ONLY` + `SET LOCAL search_path = pg_catalog` + `ROLLBACK`. `EXPLAIN` without
ANALYZE never executes the query. Plan text is captured, not printed; only the
label `parallel (Gather)` / `single process` is echoed. Confirmed live: single
process on 9.4/9.6, Gather on 12/17/18 defaults.

**Identity assertions (L177–187) — full output, not a sample.** `cmp -s` compares
the complete grep-filtered fingerprint files (every bucket, all six fields):
scan2 vs scan1 within each server, and old vs new across servers, per size.
A differing or empty result exits 1. Verified live at 100k/200k on both version
pairs. `check_fp` (L125–133) additionally asserts ≥1 and ≤4096 buckets and
`sum(n) = rows` (awk integer arithmetic is exact at these sizes).

**Catalog queries.** `SELECT count(*) FROM phase0.accounts_perf` (L166) asserted
equal to the requested rows; `pg_total_relation_size('phase0.accounts_perf')`
(L168) — schema-qualified regclass, includes heap + init/FSM/VM forks, the right
"size on disk" measure for PLAN §4.5. I independently reproduced RESULTS' constant
**237,568-byte (29-page)** old/new size delta at 100k AND 200k rows, on both
9.6×17 and 9.4×18 — corroborating the "non-heap fork overhead, not row data"
finding at two sizes Nina did not report.

**Version-aware GUC read (L80).** `CASE WHEN current_setting('server_version')
~ '^9\.[0-5]\.' THEN '' ELSE ... current_setting('max_parallel_workers_per_gather')
END` — the regex skips exactly the pre-9.6 release versions (9.0–9.5; the GUC
exists 9.6+). Verified live on all five: 9.4.26 skips it (no "unrecognized
configuration parameter" error), 9.6.24 reads `0`, 12.22/17.12/18.7 read `2`.
The rest of the banner (encoding, datcollate, shared_buffers) is 9.4-safe.

**Timing (L116–123) excludes load.** Load is timed separately (L95–113); the scan
clock wraps only `psql -f fingerprint_perf.sql`. Connection setup + ≤4096 summary
rows are included and honestly documented (RESULTS L705–707). `date +%s%N`
nanosecond wall clock; scans strictly sequential, one table at a time.

**Error semantics (`set -euo pipefail`).** Every psql uses `-X -v ON_ERROR_STOP=1`;
failures inside `q`/`val`/`scan` propagate through command substitution and abort
the script. Exactly two intentional `|| true`: the trap's best-effort cleanup
(L56, superseded by the verified cleanup) and the output grep filter (L121,
guarded by `check_fp`, so an empty result can never pass). A query error aborts
before the filter — no empty-PASS (P5 held). Cleanup (L200–206) drops with
`IF EXISTS`, then **verifies** via `pg_namespace` count = 0 and exits 1 otherwise;
the `STARTED` gate makes the trap idempotent, and an interrupted run is recovered
by the drop/recreate at the next run's start. Both my runs ended with
"phase0 schema dropped, verified gone" on both servers.

**Metrics (L192–196).** rows/s = rows/scan1, MiB/s = (bytes/1048576)/scan1,
delta% = (scan2−scan1)·100/scan1 — spot-checked against the printed tables and
RESULTS; consistent.

---

## 4. RESULTS.md (T005 section) and README.md — claims vs code

- **Methodology matches the code**, with one wording exception (R2): the
  quiescence wait runs once per table before scan1, not before *each* timed scan.
- **Parallelism**: the template pins no parallel GUCs; server defaults (9.6 = 0,
  17 = 2) confirmed live; Gather/single-process labels confirmed on all five
  versions. The "version + parallelism, not version alone" caveat is correct.
- **Cache state**: honest — warm OS page cache, 128MB shared_buffers confirmed,
  no cold-cache numbers claimed.
- **UNLOGGED rationale**: correct — a read-only scan writes no WAL, so UNLOGGED
  affects load only; keys still hashed; autovacuum off + no ANALYZE verified in
  code.
- **Timeout deviation**: documented with correct ~28% headroom arithmetic (§2).
- **Variance**: recomputed — old +0.3%/−0.1% within run, ~6.5% between runs;
  new −0.8%/+13.7% within run, 81.6 s → 109.2 s between runs disclosed. The
  conservative planning ranges (old ~93–102k, new ~183–245k rows/s) are
  consistent with both recorded runs. First-run contamination and the hardened
  canonical re-run are transparently recorded.
- **Metrics arithmetic**: rows/s and MiB/s recomputed from the table — consistent
  (also Maya-verified). ~4,883 rows/bucket = 20M/4096 ✓; overflow bound matches
  T003 §7 ✓.
- **Hardware context present** and matches my runs exactly (Ryzen 5 7640U,
  12 CPUs, 60.6 GB RAM, 1818 GB free disk, psql 18.7; container names, encodings,
  collations, shared_buffers match compose.yml defaults).
- **No row data or secrets** — counts, sizes, timings and aggregate byte counts
  only (rule 7). The 10k-row encoding probe's heap-identity figure is Nina's
  measurement (not re-run by me); its constant total-size delta was independently
  reproduced (§3).
- **README** is accurate: "exact shape ... only deviation is statement_timeout"
  holds for the settings/shape; "refuses any non-local URL" holds for ordinary
  URLs, with the R1 caveat. Earlier RESULTS sections are untouched (append-only
  diff; Maya byte-verified).

---

## 5. Findings

Severity key as in T003: FP = false-PASS risk, FF = false-FAIL risk, GAP = missing
coverage/protection, NOTE = confirmation or minor caveat. **No FP/FF findings.**

| ID | Sev | Finding | Required fix |
|----|-----|---------|--------------|
| R1 | GAP (low, inherited) | **URL guard bypassable by valid libpq syntax** — `perf.sh:35–38` (byte-identical to merged `prove.sh:28–31`). Demonstrated with read-only `SELECT 1` probes on local targets: (a) comma multi-host `...@127.0.0.1:59999,127.0.0.1:5433/bank` — guard accepts, libpq falls through to the SECOND host, which could be any remote; (b) query-param override `...@127.0.0.1:59999/bank?host=127.0.0.1&port=5433` — guard accepts, libpq connects to the query-param host, overriding the netloc. The embedded-`@` variant errors loudly in libpq 18.7 (invalid port) — not a bypass, but older clients may parse differently. Plain remote URLs are correctly refused. | **Follow-up task covering BOTH harnesses** (out of T005's lane, rule 9): reject `,`, `?` and extra `@` in the URL, or validate the effective host libpq will actually use. Non-blocking for T005: the pattern is verbatim from the reviewed `prove.sh` baseline, the defaults and ordinary fat-fingered remote URLs are safe, and writes are confined to a `phase0` schema. Maya should schedule the hardening before the harness is pointed at anything but the mocks. |
| R2 | NOTE (doc accuracy) | **RESULTS.md:708–710** says "before each timed scan the harness waits (≤30 s)", but `perf.sh:172–176` calls `wait_quiet` once per table, before scan1 only; scan2 follows immediately with no re-check (no harness DB activity in between, but an external backend could start during scan1). | Reword to "before the first timed scan of each table" — or add a second `wait_quiet` before scan2. One-line fix; Maya may fold it in before merge. |
| R3 | NOTE (cosmetic) | **fingerprint_perf.sql:53** — no trailing newline at EOF (the reference has one). No functional impact: psql and the sed extraction handled it correctly in all four of my runs. | Optional: add the trailing newline for consistency with the reference. |

Positive confirmations (no action): full §4 block with `search_path` first;
integer-only ordering; keys inside the row hash; md5 per DECISIONS #10; 9.4
executed live; full-output byte identity asserted and observed on two version
pairs; no statistics reliance; no query-text/row-data output; idempotent verified
cleanup; load excluded from timing; no swallowed errors.

---

## 6. What I ran

All against the local mocks; the only writes were the sanctioned harness's
`phase0` schema, dropped and verified afterwards. No full 1M/20M re-run (per
brief: Nina ran it twice, Maya verified the mechanics).

1. `PERF_SIZES="100000 200000" scripts/in-wt.sh T005 bash testdata/phase0/perf.sh`
   — **exit 0**. 9.6.24 × 17.12: identity byte-identical at both sizes; plans
   single-process / parallel (Gather); repeat scans identical; counts/sizes
   asserted; cleanup verified gone on both servers.
2. `PERF_SIZES="100000 200000" scripts/in-wt.sh T005 bash testdata/phase0/perf.sh 5435 5437`
   — **exit 0**. 9.4.26 × 18.7 (the version extremes): proves the template and
   every harness query 9.4-safe, and the pre-9.6 GUC-skip branch live; identity
   byte-identical at both sizes; cleanup verified gone on both servers.
3. The banner/GUC query verbatim on :5436 (12.22) — reads
   `max_parallel_workers_per_gather=2` correctly (fifth version point).
4. Guard probes: the bash glob + `psql -c 'SELECT 1'` (read-only, local targets
   only) on multi-host, embedded-`@`, `?host=`-override and plain-remote URLs —
   results recorded under R1.
5. Read-only: `diff -u` of the two templates; `git diff main...HEAD` (scope:
   exactly the four allowed files); grep write-inventory of perf.sh; cross-reads
   of prove.sh, compose.yml, PLAN §§4.5/8.1, DECISIONS, T003 §4; arithmetic
   spot-checks of the RESULTS tables; independent reproduction of the 237,568-byte
   size delta at 100k/200k on both pairs.

No file was edited except this review record; no commits, no git config changes.
