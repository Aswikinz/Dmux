# T004 — Phase 0 SQL re-review (Postgres 9.4 → latest)

Reviewer: Priya · Date: 2026-11-16 · Branch reviewed: `task/T004-extend-proof` (commit `2c66b0a`)
Scope: the full T004 diff against `main` — `prove.sh` (+354), the settings-block change in
`fingerprint.sql` / `fingerprint_nokey.sql`, all 17 new `fingerprint_*.sql` templates,
`RESULTS.md` (T004 section), `testdata/README.md`. Re-review per AGENTS.md rule 3, following
my T003 review (`docs/phase0/T003-sql-review.md`, which stays unedited as the historical record).

## 1. Verdict

**APPROVE.** No false-PASS or false-FAIL route found in the exercised paths; no blocking
findings. 0 FP, 0 FF, 0 GAP requiring changes; 7 non-blocking NOTEs (§5). All T003 findings
P1, P3–P19 that T004 was asked to address are addressed and empirically proven. DECISIONS.md
entries are untouched: md5 stays (#10, not reopened), bucket sums / four slices / UTF-8 /
bytes rules (#2–#4) unchanged — the diff modifies only settings blocks and adds new files.

I ran the proof myself on both pairs: **83 passed, 0 failed, exit 0** on 9.6→17 and on
9.4→18 (§6). The 9.4 run is the empirical 9.4-safety proof for every new template and every
new harness statement (rule 11: checked, not guessed).

## 2. Settings block (T003 §4) — verified identical in all 19 files

Extracted `BEGIN ISOLATION LEVEL …` through `SET LOCAL application_name` from all 19
fingerprint files and md5-compared: **one identical block, 19/19**
(`7c87d63093574ed8c4c3a83a8e2330cf`). It matches §4 exactly:

- `BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY` — read-only, `SET LOCAL` only.
- `search_path = pg_catalog` **first, above all other settings** (P1); every table is
  schema-qualified (`phase0.*`), so function shadowing is closed for the fingerprint path.
- `timezone='UTC'`, `datestyle='ISO, YMD'`, `intervalstyle='iso_8601'`, `bytea_output='hex'`,
  `extra_float_digits=0`, `xmloption='document'`, `statement_timeout='300s'` (harness window;
  §4's `<run window>` placeholder correctly instantiated, engine uses the run window),
  `lock_timeout='5s'`, `application_name='dmux'`.
- The no-key reference file gained `intervalstyle` + `bytea_output` (P10/P18 closed); both
  references gained `search_path`, `extra_float_digits`, `xmloption`, `statement_timeout`
  (P1/P7/P16/P17 closed). Hash logic in both references is byte-identical to T001/T002.
- Every setting exists on 9.4 (proven by the 9.4.26 run; `lock_timeout` is 9.3+,
  `bytea_output` 9.0+, `intervalstyle`/`xmloption` older).

## 3. Per-template review (17 new + 2 modified references)

Shape checks applied to all 19 files and verified mechanically: identical outer aggregate
(`count(*) AS n`, four `sum(('x'||substr(rh,1/9/17/25,8))::bit(32)::bigint)` slices covering
all 32 md5 hex chars), bucket `('x'||substr(kh|rh,1,3))::bit(12)::int` (17 keyed from `kh`,
2 no-key from `rh` — matching the respective reference), `GROUP BY 1` / `ORDER BY 1` on the
**integer** bucket in all 19 (no text ordering anywhere, P24), `COMMIT` at end. Key columns
are the first ROW fields of every keyed row hash (P-note from T003 holds for all new tables).
grep for post-9.4 syntax (`ON CONFLICT`, `MERGE`, `GROUPING SETS/CUBE/ROLLUP`, `jsonb_pretty`,
numeric `Infinity`, generated columns, multirange, window functions, SQL/JSON, …): **none**.

| Template | Normalisation vs T003 §5 | Notes |
|---|---|---|
| `fingerprint.sql` (ref) | unchanged hash logic | settings block only; 15 original checks are the regression guard |
| `fingerprint_nokey.sql` (ref) | unchanged counted-set logic | settings block only; bucket from `rh` |
| `fingerprint_tz.sql` | `ts::text` under UTC (P9) | §6.1 exact values; same instant `+05:30` vs `+00` stays quiet |
| `fingerprint_tz_mix.sql` | same `::text` rule both sides | printed forms differ (`…00:00:00` vs `…+00`) → value diff reported, never silent PASS of the semantic shift; catalog flags type change. By design per §6.1 |
| `fingerprint_money.sql` | `amt::numeric::text` (P11) | **never `money::text`**; locale-free cast asserted on both sides; lc_monetary recorded + warned |
| `fingerprint_json.sql` | `doc::text` exact text (P13) | key order/whitespace significant; json is 9.2+ |
| `fingerprint_jsonb.sql` | `doc::text` canonical (P12) | jsonb is 9.4+ — safe at the floor; version-stable output |
| `fingerprint_json_mix.sql` | same `::text` rule both sides | json→jsonb reorder + duplicate-key dedup both asserted `differ`; dedup is data loss, flagged, never quiet |
| `fingerprint_xml.sql` | `doc::text`, `xmloption` pinned (P16) | engine warning requirement documented in header |
| `fingerprint_arr.sql` | `fa::text`, `ia::text` under `extra_float_digits=0` (P7) | §5 allows printed-form-with-pin or `array_send`; pin chosen and proven — this is the P7 regression (without it, 9.4/9.6 vs 12+ false-FAILs). NULL elements kept distinct |
| `fingerprint_rng.sql` | `ir::text`, `nr::text` under fixed settings | discrete canonicalisation (`int4range(1,5)` = `'[1,4]'` → `[1,5)`); ranges 9.2+ |
| `fingerprint_iv.sql` | `iv::text` under iso_8601 (P10) | `PT24H` vs `P1D` component trap asserted |
| `fingerprint_iv_nokey.sql` | `ROW(iv::text)` — covers the table's only column | no-key shape identical to reference; proves the no-key block now sets intervalstyle (P10 regression) |
| `fingerprint_numscale.sql` | `amt::numeric(20,6)::text` — **up-cast only, both sides** (P4/P8) | never casts down; harness asserts 1.2344/1.2345 distinct at scale 6 and demonstrates the scale-2 collapse |
| `fingerprint_bin.sql` | `md5(b)` raw bytes (NULL≠empty) | immune to `bytea_output`; NULL → empty ROW field, `''::bytea` → md5-of-empty; distinct both directions |
| `fingerprint_empty.sql` | `ROW(id::text)` | zero rows legal only with explicit count=0 + error-is-FAIL harness guarantees (§4 below) |
| `fingerprint_col.sql` | mapped columns `id,val` only (P3) | header states the value hash alone is never the verdict; dropped mapped column errors → FAIL |
| `fingerprint_coll.sql` | `name::text` exact UTF-8 bytes | byte hashing collation-independent (P24/P25); bank_c (UTF8/C) vs bank (UTF8/en_US.utf8) |
| `fingerprint_ascii.sql` | `t::text` via `convert_to(...,'UTF8')` | header documents the **corrected** P19 behaviour (validate, not passthrough); non-ASCII errors loudly |

NULL markers: every template relies on the proven ROW rendering (NULL = empty field,
empty string = `""`, literal `'NULL'` unquoted) — unchanged from the reference, re-confirmed
by the NULL/empty bytea and NULL-element array cases.

## 4. prove.sh — harness semantics

**Original 15 checks truly unchanged.** Extracted all top-level `check`/`mut` call lines
from `main` and HEAD: the first 15 are byte-identical (names, expectations, SQL, undos).
`mut()` moved above its first use and gained optional URL args whose defaults reproduce the
old behaviour exactly; `check()` likewise. Schema/load sections untouched.

**`fp()` empty-tolerance cannot swallow an error (P5).** Exact pipeline
(`prove.sh:40-43`): `PGOPTIONS="$PGOPTIONS ${3:-}" psql -X -q -A -t -v ON_ERROR_STOP=1 … -f … | { grep -E '^[0-9]+\|' || true; }`.
- `set -euo pipefail` is in force (line 20); `PGOPTIONS` is exported at line 21, so the
  `$PGOPTIONS` reference is never unbound under `set -u`.
- Any SQL error ⇒ `ON_ERROR_STOP=1` ⇒ psql exits non-zero. The grep group always exits 0,
  but **pipefail makes the pipeline status psql's status** — the error propagates.
- In `check()`, `fp` is a plain command: a non-zero status triggers errexit and aborts the
  whole run (loud failure, never a PASS). Partial bucket output before an error cannot
  produce a PASS for the same reason.
- Empty output is reachable **only** when psql exited 0 (genuine empty table). Case 6.10
  additionally asserts `count(*)=0` explicitly on both sides (line 364), inserts → `differ`,
  drops the table → `check_error` proves the missing-table error, restores → `same`.
- `check_error`/`assert_error` use `if fp …` / `if psql …` — the conditional context is the
  only place failure is legal, and it is counted as the *expected* error.
- `assert_both`/`assert_one`/`cols` use command substitution: a failing psql aborts via
  errexit rather than comparing an empty string.

**mut/undo correctness — every new case checked.** Each undo restores the exact value or
multiset: tz (explicit instant), money (exact cent), json/jsonb (exact literals), dedup row
(insert id=2 both sides → delete both sides), xml, arrays (rows 1 and 2 separately),
ranges, interval (exact literals), iv_nk (ctid-targeted change of exactly one of the two
`'24 hours'` rows to `'1 day'`, undo targets the single `'1 day'` row — counted-set equality
restored, order-independent), numscale (1234.567900 → 1234.567800), bytea (all three rows,
both NULL↔empty directions), empty (insert → delete; drop → recreate), col (add → drop →
recreate with original rows). 6.12's `mut` passes `"$NEW_URL" "$C_URL"` in the right slots
(change on the en_US.utf8 side, compared against the C side).

**Per-case URLs and the local-only guard.** `check`/`mut`/`assert_*`/`schema_check` accept
optional URLs defaulting to `OLD_URL`/`NEW_URL`. `C_URL`/`A_URL` (lines 403, 424) are derived
from `OLD_URL` by replacing only the trailing database name (`${OLD_URL%/bank}/bank_c`);
the host authority is untouched, so the unchanged guard (lines 28–31, `@127.0.0.1[:/]` or
`@localhost[:/]`) covers **every URL used**. See NOTE N3 for the non-`/bank` override edge.

**§6 assertions match the spec:**
- **6.8 (P4):** `count(DISTINCT amt::numeric(20,6)::text) … id IN (2,3)` = **2** on both
  sides, and the down-cast `numeric(20,2)` = **1** on both sides (lines 345–348) — the
  guard *and* the collapse demonstration, exactly as specified.
- **6.10 (P5):** explicit `count(*)=0` both sides, not just empty output (line 364).
- **6.11 (P3):** mapped fingerprint `same` after ADD COLUMN is asserted as EXPECTED
  (line 376); `schema_check` requires the catalog lists to differ AND the marker
  (`extra:integer`) on exactly the new side; coverage query lists the unmapped column
  (`extra`); dropped mapped column → `check_error` + catalog marker `val:text` on the old
  side only. `cols()` orders by `attnum`, never by text. The value hash is never the verdict.
- **6.13 (P19):** asserts the `convert_to` ERROR (`check_error` on the full fingerprint +
  `assert_error` on the expression), the LATIN1 passthrough (`e9`), raw-byte storage
  difference (`octet_length` 1 vs 2), `c3a9` conversion on the UTF8 and LATIN1 sides,
  `pg_database` encoding detection (`SQL_ASCII`), and prints the FLAG-for-review
  requirement (lines 429–452). See §5 below for the correction record.
- **6.1 session-TZ proof:** `FP_EXTRA_OLD/NEW` inject `-c timezone=Asia/Kolkata` /
  `America/New_York` via PGOPTIONS; `SET LOCAL timezone='UTC'` beats both (line 261–263).
- **§7 probes:** unsigned slice `ffffffff` = 4294967295 and `pg_typeof(sum(bigint))` =
  numeric, both sides — matches my T003 §7 bound.

Check arithmetic verified: 68 new = 5+4+7+2+4+3+5+4+4+5+7+2+6+10 (6.1…6.13 + §7); 15+68 = 83.

## 5. The P19 correction — confirmed and recorded here

T003's P19 was docs-based (no SQL_ASCII server existed in the matrix then) and claimed
`convert_to(x,'UTF8')` passes SQL_ASCII bytes through **unconverted and unvalidated**,
risking a silent false PASS. **That assumption is wrong and is hereby corrected:**

- PostgreSQL performs no *conversion* for a SQL_ASCII source but **validates the bytes
  against the destination encoding** (`mbutils.c`, `pg_do_encoding_conversion`: "No
  conversion is possible, but we must validate the result").
- Verified empirically by Nina on 9.4.26 / 9.6.24 / 18.7, directly probed by Maya on 9.6.24,
  and re-run by me in case 6.13 on both pairs (9.6→17 and 9.4→18):
  - `convert_to(chr(233),'UTF8')` on SQL_ASCII ⇒ **ERROR** `invalid byte sequence for
    encoding "UTF8": 0xe9`; the full `md5(convert_to(ROW(...)::text,'UTF8'))` fingerprint
    path errors loudly (asserted by `check_error`).
  - `convert_to(chr(233),'LATIN1')` ⇒ raw `e9` (valid in destination, no conversion).
  - Storage: SQL_ASCII side keeps 1 raw byte `0xE9`; UTF8 side stores 2 bytes `c3a9`.
- Consequences: the feared silent false PASS on non-ASCII SQL_ASCII data **cannot happen**
  through the reference fingerprint — it errors. Pure-ASCII data hashes equal (ASCII is
  valid UTF-8). This is strictly safer than assumed; **no check was weakened**.
- Residual engine requirement (unchanged, and the reason the FLAG must not depend on
  whether the query errored): SQL_ASCII data that *happens* to be valid UTF-8 would
  validate and hash equal to a genuinely UTF-8 side, and pure-ASCII data hides the true
  encoding entirely. The declared encoding is unverifiable, so the engine must read
  `pg_database.encoding` and **FLAG SQL_ASCII for review in all cases** (T003 §5/§8).
  In phase0 this is encoded as the detection assertion + the harness note; the flag itself
  is an engine-task requirement (NOTE N5).
- Case 6.13's assertions encode the verified behaviour exactly (§4 above). The T003 report
  remains unedited as the historical record; this section is the correction of record.

## 6. What I ran (all via the wrapper or read-only)

1. Read AGENTS.md, DECISIONS.md (#10 noted, not reopened), T003 review, and the full
   `git diff main...HEAD` (22 files, +1396/−25), plus every changed file in the worktree.
2. Original-15 call-site diff (main vs HEAD): identical.
3. Settings-block md5 across all 19 files: one identical block; aggregate/bucket/GROUP/ORDER
   shape uniformity verified by extraction + uniq.
4. grep for post-9.4 syntax across all templates and prove.sh: none.
5. `shellcheck testdata/phase0/prove.sh`: clean.
6. `scripts/in-wt.sh T004 bash testdata/phase0/prove.sh` (9.6.24→17.12):
   **Result: 83 passed, 0 failed, exit 0.**
7. `scripts/in-wt.sh T004 bash testdata/phase0/prove.sh 5435 5437` (9.4.26→18.7):
   **Result: 83 passed, 0 failed, exit 0.**
8. Post-run read-only catalog checks: `bank_c`/`bank_ascii` absent on both old servers
   (5433, 5435); `phase0` schema absent on both new banks (5434, 5437) — cleanup works.
9. `cmp` of RESULTS.md: the entire T001/T002 content is a byte-identical prefix of the new
   file; the T004 section is a pure append.

## 7. Harness hygiene

- **Writes confined to the mock harness:** `phase0` schema in the mock `bank` databases,
  plus `bank_c`/`bank_ascii` on the OLD mock server only. `CREATE DATABASE … TEMPLATE
  template0` with explicit ENCODING/LC_COLLATE/LC_CTYPE (9.4-safe; proven on 9.4.26).
  Idempotent: `DROP DATABASE IF EXISTS` before each create; recreated every run; dropped at
  the end (verified live after my runs). No compose/port/image changes (diff confirms);
  `expected_diffs.json` untouched; engine read-only rule unaffected (prove.sh is the
  documented WRITING test harness).
- **No secrets or row data** in RESULTS.md: only check names, counts, bucket counts, and
  expression-level output from invented data (`e9`, `c3a9`, `1234.56`, version/encoding/
  collation strings). Connection passwords appear nowhere in the report.
- **README accurate:** extra-DB table, template0, per-run recreate/drop, new-side = existing
  `bank`, 83 checks / 17 templates / settings block description, T008 owns the full matrix
  re-run — all match the script.

## 8. Findings

**Blocking: none.** Non-blocking NOTEs (no action required for T004 merge; N5/N6 carry into
engine tasks):

| ID | Sev | Location | Note |
|----|-----|----------|------|
| N1 | NOTE (cosmetic, pre-existing) | prove.sh:49 | `buckets` counts diff `^<` lines, so a change that *adds* a bucket prints "0 bucket(s)" (empty-table insert case). `cmp` still decides same/differ correctly; display only. |
| N2 | NOTE | prove.sh:276-281 | The lc_monetary step is a *recording* check (always ok, warns on mismatch) per §6.2 "record … as a warning"; the real assertion is `money::numeric` locale-freedom. Its two `SHOW` psql calls omit `ON_ERROR_STOP`, but a connection failure still aborts via errexit. |
| N3 | NOTE | prove.sh:403,424 | `C_URL`/`A_URL` derivation assumes `OLD_URL` ends in `/bank`. A non-`/bank` override yields a malformed dbname path → loud connection failure (still local-only; host untouched, guard intact). Documented usage always uses `/bank`. |
| N4 | NOTE | prove.sh:457-460 | Auxiliary-DB cleanup runs on the success path; an interrupted run leaves `bank_c`/`bank_ascii` until the next run's `DROP … IF EXISTS` (idempotent, documented in README/RESULTS). |
| N5 | NOTE (engine carry-forward) | prove.sh:429-431 | SQL_ASCII FLAG-for-review is a harness note + detection assertion here; the engine must implement the flag for **every** SQL_ASCII database, even pure-ASCII or coincidentally-valid-UTF-8 data (§5 above; T003 §5/§8). |
| N6 | NOTE (engine carry-forward) | prove.sh:95-99, 378-379 | Harness catalog/coverage probes run without pinned `search_path` — fine on mock DBs. The engine's own catalog/coverage queries should pin `search_path=pg_catalog` too (P1 extended to catalog reads). |
| N7 | NOTE | RESULTS.md | T001/T002 content verified byte-identical prefix; T004 appended. No secrets/row data. |

## 9. Conclusion

T004 implements T003 §§4–7 faithfully: canonical settings block everywhere, §5 normalisation
per column, §6 cases with exact values and real assertions (including the P4 down-cast
collapse demonstration, explicit empty counts, catalog-enforced schema diff, and the
corrected P19 error behaviour), reference checks untouched, 9.4-safe throughout (proven by
execution on 9.4.26), clean harness hygiene. **APPROVE for merge.** Maya: the only
carry-forwards are N5/N6 (engine tasks) and the full-matrix re-run already owned by T008.
