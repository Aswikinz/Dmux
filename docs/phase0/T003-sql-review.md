# T003 — Phase 0 SQL review (Postgres 9.4 → latest)

Reviewer: Priya · Date: 2026-11-16 · Branch: `task/T003-sql-review`
Note (T009 scrub): test column names quoted here were renamed to invented names before publication; tests and findings are unchanged.
Scope: `testdata/phase0/fingerprint.sql`, `fingerprint_nokey.sql`, `prove.sh`, against
PLAN.md §§2.1, 3, 3.2, 8.2 and DECISIONS.md. **Review only — no existing file was changed.**

Every empirical claim below was run read-only (`SELECT`/`SHOW`, plus transaction-local
`set_config(...,true)` inside a single read-only statement batch) against the five local
mock servers, using psql 18.7:

| Port | Server | Encoding | Collation | lc_monetary | extra_float_digits (default) |
|---|---|---|---|---|---|
| 5435 | 9.4.26 | LATIN1 | C | C | 0 |
| 5433 | 9.6.24 | LATIN1 | C | C | 0 |
| 5436 | 12.22 | UTF8 | en_US.utf8 | en_US.utf8 | 1 |
| 5434 | 17.12 | UTF8 | en_US.utf8 | en_US.utf8 | 1 |
| 5437 | 18.7 | UTF8 | en_US.utf8 | en_US.utf8 | 1 |

"Identical on all five" means the query output was byte-for-byte equal on 9.4.26, 9.6.24,
12.22, 17.12 and 18.7 unless a specific version is named. No secrets or row data appear
here; only expression-level output from throwaway containers.

---

## 1. Summary verdict

**The reference fingerprint is sound for the data it exercises, on 9.4 → 18, as-is.**
T001/T002 already proved 15/15 on all six pairs. This review confirms the *mechanisms*
behind that result are version-stable (ROW rendering, `convert_to` UTF-8, `float8send`
bytes, `md5(bytea)`, bit casts, integer bucket ordering) and finds **no false-PASS in the
exercised path**. The row hash includes the key columns, NULL/empty are distinct, and the
counted-set logic catches duplicate changes.

**But it is not yet a safe template for the engine.** Before the engine is built, the
following must change (details and evidence in §3):

- **Pin `search_path`** (P1) — unqualified `md5`/`convert_to`/`sum`/`count` can be shadowed
  by a user schema → silent false PASS. This is the most serious finding.
- **Never cast numeric to a smaller scale** (P4) — rounding collapses distinct values → false PASS.
- **Enforce schema-diff + column coverage** (P3) and **treat query error / missing table as
  FAIL, never empty-PASS** (P5) — added/dropped columns and failed scans are invisible to the
  value hash.
- **Complete the settings block** (P17, P18): add `statement_timeout`; give the no-key file
  `intervalstyle` and `bytea_output`; pin `extra_float_digits=0` for any printed-form
  float/array/range; pin `xmloption` for xml.
- **Normalise the unexercised types** (§5) and **flag SQL_ASCII** (P19) — `convert_to` passes
  bytes through unconverted there.
- **Decide the md5 adversarial question** (P2) — md5 chosen-prefix collisions are practical;
  fine for accidental migration errors, not for a malicious operator. Owner decision via Maya.

None of these reopen a DECISIONS.md entry. Order-independent bucket sums, four-slice sums
(not XOR), UTF-8 before hashing, and floats/binary by bytes are all confirmed correct and
left settled.

---

## 2. What T001/T002 already proved (T004 must NOT duplicate these)

From `RESULTS.md`, 15/15 on all six pairs (9.4/9.6 × 12/17/18):

Stay-quiet: LATIN1→UTF-8 accented names; opposite physical row order; different column
order; `char(8)`→`varchar(8)`; no-key table with duplicates on both sides.
Must-catch (each altered exactly one bucket, then undone): balance +0.0001; one character in
a name; empty text→NULL; time-of-day dropped from a timestamp; float changed in last digit;
one byte in bytea; two values swapped between columns; one row deleted; no-key duplicate
deleted; no-key duplicate pair added (the XOR-cancellation case).

This review re-verified the *primitives* those results depend on (ROW injectivity, encoding
conversion, float bytes, bit/sum behaviour) at expression level on all five versions, so
T004 can trust them and focus on the **new types and scenarios in §6**.

---

## 3. Findings

Severity key: **FP** = false-PASS risk · **FF** = false-FAIL risk · **GAP** = missing
coverage/setting · **NOTE** = confirmation or minor caveat.

| ID | Sev | Finding | Evidence (query → output, versions) | Recommended action |
|----|-----|---------|--------------------------------------|--------------------|
| P1 | FP | `search_path` not pinned; every function/aggregate in the fingerprint is unqualified (`md5`, `convert_to`, `substr`, `encode`, `float8send`, `sum`, `count`, the `ROW::text`/`bit` casts). A user schema earlier in `search_path` can shadow any of them and silently change hashes. | `SHOW search_path` → `"$user", public` on 9.4.26 (5435) and 18.7 (5437). `fingerprint.sql` L26–39 uses unqualified names. | Engine: `SET LOCAL search_path = pg_catalog;` and schema-qualify every table; or `pg_catalog.`-qualify every function. Add to the settings block on both files. |
| P2 | FP | Row/key/bytea hashes use `md5`. md5 chosen-prefix collisions are practical, so a *deliberate* actor can craft two different rows with the same row hash → invisible to the sums. Negligible for accidental migration errors (random collision ≈ n²/2¹²⁹). | `fingerprint.sql` L26, L29, L37 use `md5`. `SELECT md5('dmux-probe')` → `d4924ef6…` on 18.7 (function present, non-FIPS). Collision weakness is published cryptanalysis, not reproduced here. | If the threat model includes a malicious migration operator, propose SHA-256 row hashes via Maya/owner (PLAN change). Do not reopen DECISIONS #3/#4. For non-adversarial use, document residual risk as negligible. |
| P3 | FP | The value hash covers only *mapped* columns. An added/dropped/unmapped column is invisible to the fingerprint; PASS would be false if schema diff and coverage reporting are not enforced. | `fingerprint.sql` L29–39 hashes a fixed column list. PLAN §2.1: "a column silently left out of the hash gives a false PASS". | Engine must list covered columns per table, report added (green)/dropped (red)/unmapped columns, and never PASS a table whose column set changed without a recorded reason. T004 §6.11. |
| P4 | FP | Casting numeric to a **smaller** scale rounds and collapses distinct values. | `SELECT (1.2344::numeric(10,2))::text` → `1.23`; `(1.2345::numeric(10,2))::text` → `1.23` (both!) on all five. `(1.235::numeric(10,2))::text` → `1.24`. | PLAN §3.2 rule is cast to the **larger** scale — engine must never cast down. T004 must-catch: 1.2344 vs 1.2345 stay distinct. |
| P5 | FP | An empty table yields zero fingerprint rows on both sides → `cmp` equal → PASS. A failed query or a missing table could masquerade as "empty" if errors are swallowed. | Fingerprint returns 0 rows for an empty table; `prove.sh` `fp()` greps `^[0-9]+\|` (empty output both sides compares equal). `SELECT count(*) FROM generate_series(1,0)` → `0` on all five. | Engine: query error ⇒ tool-fault/FAIL, never PASS; distinguish empty vs missing via inventory; record an explicit count=0 marker. `prove.sh` already uses `ON_ERROR_STOP=1`+`pipefail` (good). |
| P6 | FP | `char(n)::text` strips trailing pad spaces (bpchar semantics). A `varchar`→`char(n)` migration that truncates *significant* trailing spaces compares equal. | `SELECT ('S1 '::char(8))::text = ('S1'::char(8))::text` → `t`; `('S1'::char(8))::text` → `S1` (length 2); `('S1      '::varchar(8))::text` → `S1······` (spaces kept) on all five. | Bounded: schema diff must flag `char`↔`varchar` type change amber (PLAN §8.2 already does). Document bpchar trailing-space insignificance; value PASS is by design. |
| P7 | FF | Float **printed form** differs across versions (`extra_float_digits` default 0 on 9.4/9.6 vs 1 on 12+). Scalar floats are safe (reference uses `float8send`), but **float arrays/ranges hashed as printed form would false-FAIL**. | `SELECT (1::float8/3)::text` → `0.333333333333333` (9.4/9.6) vs `0.3333333333333333` (12/17/18). `SELECT ARRAY[0.1::float8,1::float8/3]::text` → `{0.1,0.333333333333333}` vs `{0.1,0.3333333333333333}`. `encode(float8send(1::float8/3),'hex')` → `3fd5555555555555` on all five. Pinning `extra_float_digits=0` makes printed form match on all five; `=3` does NOT (9.4 → `0.333333333333333315`, 18 → `0.3333333333333333`). | Keep `float8send` for scalars. For arrays/ranges of floats: `SET LOCAL extra_float_digits=0` (verified stable) **or** hash elements by bytes (`encode(array_send(...),'hex')` verified identical on all five). |
| P8 | FF | numeric typmod/scale change alters printed form (`1234.5678` vs `1234.567800`). | `SELECT (1234.5678::numeric(20,4))::text` → `1234.5678`; `(::numeric(20,6))::text` → `1234.567800`; casting up `(::numeric(20,4))::numeric(20,6)::text` → `1234.567800` on all five. | PLAN rule: cast both sides to the larger scale. Verified the up-cast normalises correctly. T004 §6.8. |
| P9 | FF | `timestamptz` printed form depends on session timezone. Mitigated by `SET LOCAL timezone='UTC'` in both files, but the engine must always set it; a `timestamp`→`timestamptz` type change also changes semantics. | Under UTC: `timestamptz '2001-01-01 00:00:00+05:30'::text` → `2000-12-31 18:30:00+00`. Under `set_config('timezone','Asia/Kolkata',true)`: same value → `2001-01-01 00:00:00+05:30`; under America/New_York (18.7) → `2000-12-31 13:30:00-05`. UTC output identical on all five. | Keep `SET LOCAL timezone='UTC'`. Flag `timestamp`↔`timestamptz` type change in schema diff (interpretation depends on session TZ at load). T004 §6.1. |
| P10 | FF/GAP | interval printed form depends on `intervalstyle`; `fingerprint_nokey.sql` does not set it. | iso_8601: `interval '1 year 2 mons 3 days 04:05:06'::text` → `P1Y2M3DT4H5M6S`; postgres style → `1 year 2 mons 3 days 04:05:06`. `'24 hours'` → `PT24H` vs `'1 day'` → `P1D` (distinct values). Identical on all five under each style. | Add `SET LOCAL intervalstyle='iso_8601'` to the no-key template; engine uses one full settings block for every table. T004 §6.7. |
| P11 | FF | `money::text` depends on `lc_monetary` (C on old vs en_US.utf8 on new). The PLAN rule `money→numeric` avoids it. | `lc_monetary` = C (5435/5433) vs en_US.utf8 (5436/5434/5437). `SELECT '1234.56'::money::numeric::text` → `1234.56` and `= 1234.56::numeric` → `t` on all five (cast is locale-free, operates on internal int64 cents). `money::text` → `$1,234.56` under both C and en_US here; de_DE.utf8 unavailable in container. | Use `money→numeric` (verified locale-independent). Record/compare `lc_monetary` in inventory and warn if it differs (affects money *input parsing* and any `::text` fallback). Never hash `money::text`. T004 §6.2. |
| P12 | FF | `json`→`jsonb` reorders keys, changes spacing, and **dedupes duplicate keys** (real data loss). Printed forms differ. | `SELECT '{"b":1,"a":2}'::json::text` → `{"b":1,"a":2}`; `::jsonb::text` → `{"a": 2, "b": 1}`; `'{"a":1,"a":2}'::jsonb::text` → `{"a": 2}` (duplicate dropped). jsonb output identical on all five. | PLAN §3.2: flag json→jsonb type change; engine must NOT treat it as silently equal. jsonb→jsonb is stable. T004 §6.3. |
| P13 | FF | `json` preserves exact text (key order + whitespace); a migration that re-serializes json false-FAILs. | `'{"b":1,"a":2}'::json::text` → `{"b":1,"a":2}` vs `'{"b":1, "a":2}'::json::text` → `{"b":1, "a":2}` (space preserved) on all five. | Document json as exact-text. Reformatting is a real change; only canonicalise via jsonb when the type change is flagged. |
| P14 | FF | Unconstrained numeric preserves per-value scale: `1.10` ≠ `1.1` as text though numerically equal. | `SELECT '1.10'::numeric::text` → `1.10`; `'1.1'::numeric::text` → `1.1` on all five. | typmod scale changes handled by P8 cast rule. Per-value dscale drift in unconstrained numeric (ETL recomputation) → user "round to N decimals" option, or owner decision on trailing-zero normalisation. Note, not a blocker. |
| P15 | FF/NOTE | float `-0.0` vs `0.0` have different bytes → FAIL though numerically equal. Consequence of the settled bytes rule. | `encode(float8send(-0.0::float8),'hex')` → `8000000000000000`; `(0.0::float8)` → `0000000000000000`; equality → `f` on all five. | Document. DECISIONS #4 (floats by bytes) is settled; a migration normalising -0→0 will FAIL. Optional recorded option if owner wants numeric-equality for floats. |
| P16 | FF/NOTE | xml printed form can depend on `xmloption`/libxml. Stable in this test. | `SELECT '<a x="1">t</a>'::xml::text` → `<a x="1">t</a>` identical on all five; `xmloption` default `content`. | Pin `SET LOCAL xmloption` for xml columns; treat xml as printed form with a report warning (PLAN §3.2). T004 §6.4. |
| P17 | GAP | **No `statement_timeout`** in either file, though PLAN §3.1 requires it. | `fingerprint.sql` L9–14 sets timezone/datestyle/intervalstyle/bytea_output/lock_timeout/application_name only; `fingerprint_nokey.sql` L6–9 likewise. | Engine adds `SET LOCAL statement_timeout` (and keeps `lock_timeout='5s'`). Safety on live banking DBs. |
| P18 | GAP | Settings block incomplete/inconsistent: no-key file lacks `intervalstyle`+`bytea_output`; both lack `extra_float_digits`, `search_path`, `xmloption`. Harmless for the current varchar/bytea-bytes columns, real for other types. | `fingerprint_nokey.sql` L6–9 vs `fingerprint.sql` L9–14. `bytea_output` is irrelevant to `md5(bytea)` (P-note below) but needed for any `bytea::text` path. | One canonical settings block for every table: timezone, datestyle, intervalstyle, bytea_output, extra_float_digits=0, xmloption, search_path=pg_catalog, statement_timeout, lock_timeout, application_name. |
| P19 | GAP/FP+FF | **SQL_ASCII**: `convert_to(x,'UTF8')` passes bytes through unconverted, so encoding cannot be verified; accented data may false-FAIL (LATIN1 bytes vs UTF-8 bytes) or false-PASS (same raw bytes, different actual characters). | PostgreSQL 18.7 docs §23.3.1: "No encoding conversion will be done when the setting is SQL_ASCII"; Table 23.4: SQL_ASCII → "any (no conversion will be performed)". No SQL_ASCII server in the local matrix, so confirmed from docs, not run. | Engine reads `pg_database.encoding`; if SQL_ASCII, **flag for review** (PLAN §2.1/§3.2), never silent PASS on non-ASCII data. T004 §6.13 (Nina creates a SQL_ASCII DB). |
| P20 | GAP | Types/scenarios not exercised at all by phase0: timestamptz, money, json/jsonb, xml, arrays, ranges, interval, numeric scale change, NULL bytea, empty table, added column, different collations, SQL_ASCII. | `prove.sh` schema L38–52 covers varchar/char/numeric/float8/timestamp(0)/bytea only. | T004 covers each — precise cases in §6, normalisation rules in §5. |
| P21 | NOTE/GAP | numeric `Infinity`/`-Infinity` are PG14+; errors on 9.4/9.6/12. `NaN` works everywhere. | `SELECT 'Infinity'::numeric::text` → ERROR "invalid input syntax" on 9.4.26/9.6.24/12.22; → `Infinity` on 17.12/18.7. `'NaN'::numeric::text` → `NaN` on all five. | Engine handles `Infinity`/`-Infinity`/`NaN` numeric output; a numeric domain widening is a schema/domain diff. Don't emit Infinity literals on <14. |
| P22 | NOTE | 9.4 syntax limits for the engine (reference query itself is 9.4-safe, verified by T001/T002). | Reference uses only `BEGIN…REPEATABLE READ READ ONLY`, `SET LOCAL`, `md5`, `convert_to`, `ROW::text`, `substr`, `bit(n)::int/bigint`, `encode`, `float8send`, `sum`, `count`, `GROUP BY/ORDER BY 1` — all run on 9.4.26. | Engine must avoid/branch: `ON CONFLICT` (9.5+), `GROUPING SETS/CUBE/ROLLUP` (9.5+), RLS (9.5+), `jsonb_pretty` (9.5+), generated columns (12+), multirange (14+), numeric Infinity (14+), SQL/JSON constructors & `MERGE` (15+), and the PG15 shared-memory `pg_stat` API change for change counters. |
| P23 | NOTE | Bucket count hardcoded to 4096 (3 hex digits of key hash). PLAN raises it for huge tables (~10k rows/bucket). | `fingerprint.sql` L16 `substr(kh,1,3)::bit(12)::int`; slices L18–21 use `substr(rh,1/9/17/25,8)` covering all 32 md5 hex chars. | Engine must use the **same** bucket count on both sides and record it in the fingerprint file; parameterise the hex-digit width. |
| P24 | NOTE | No text ordering anywhere in the fingerprint: `GROUP BY 1`/`ORDER BY 1` reference the integer bucket. Collation-independent. Drill-down (not in phase0) must keep this property. | Bucket is `::bit(12)::int` (L16); md5 hex is only used via `substr→bit→int`, never grouped/ordered as text. | Drill-down: order/compare by integer bucket or in-app, never by raw hash text under a server collation. |
| P25 | NOTE | Nondeterministic collations (PG12+ ICU) can make distinct-byte keys compare equal at SQL level (unique/key semantics). Byte hashing is immune, so the fingerprint still distinguishes them. | Current matrix already pairs C/LATIN1 vs en_US.utf8/UTF8. Byte hashing (DECISIONS #4) ignores collation. | Inventory concern, not fingerprint. T004 §6.12 adds a same-encoding/different-collation pair; must-catch case change even under a case-insensitive collation. |

### Positive confirmations (no action; these are why the reference is sound)

- **ROW()::text is injective for the tested adversarial pairs and stable 9.4→18.**
  `ROW('a',NULL,'')::text` → `(a,,"")`; `ROW('NULL','""',' ')::text` → `(NULL,""""""," ")`;
  `ROW('a,b','c')` → `("a,b",c)` vs `ROW('a','b,c')` → `(a,"b,c")`; `ROW('a"b',E'a\\b')` →
  `("a""b","a\\b")`. All collision probes (`NULL` vs `'NULL'`, `''` vs `'""'` vs `'"'`,
  comma-shift, quote/backslash doubling, 1-col vs 2-col) returned `f` on all five.
  NULL renders as an empty field, empty string as `""`, the literal `'NULL'` as unquoted
  `NULL` — three distinct renderings. The only cross-version difference was the raw
  `ROW(chr(233),'x')::text` byte (LATIN1 `0xE9` vs UTF-8 `0xC3A9`) **before** `convert_to`,
  which is exactly what `convert_to` fixes.
- **UTF-8 conversion correct and stable.** `md5(convert_to(chr(233),'UTF8'))` →
  `66ddcd97cfdeabb2f6fb8a999b4bc76f` and `encode(convert_to(chr(233),'UTF8'),'hex')` → `c3a9`
  on LATIN1 (9.4/9.6) and UTF8 (12/17/18) alike. `md5(convert_to(ROW(chr(233),'a b',NULL,'')::text,'UTF8'))`
  → `33e9fbecfed556911e0d9935a9141661` on all five.
- **bytea hashed as raw bytes, immune to `bytea_output`.** `md5(decode('00ff','hex'))` →
  `d07d34efac6328007ad67c7e0a985e00` under both `hex` and `escape` output (18.7), and equal
  on all five. `md5(''::bytea)` → `d41d8cd98f00b204e9800998ecf8427e`; `md5(NULL::bytea)` → NULL
  (distinct). `md5(bytea)` ≠ `md5(text)` for the same hex-looking string (`d07d34…` vs
  `74a760…`) — raw bytes, not printed form. `bytea::text` does change with `bytea_output`
  (`\x00ff` vs `\000\377`), so the setting matters only if the engine ever uses `::text`.
- **float send bytes stable; widening exact.** `float8send` of 0.1, 1/3, ±0, NaN, ±Infinity
  identical on all five. `float4send(0.1::float4)` → `3dcccccd`;
  `float8send(0.1::float4::float8)` → `3fb99999a0000000` (stable) — so a float4→float8 mapping
  must cast **both** sides to float8 then `float8send` (widening is exact); hashing float4 by
  `float4send` on one side and float8 by `float8send` on the other would false-FAIL.
- **bit casts unsigned; sums cannot overflow in SQL.** `('x'||'ffffffff')::bit(32)::bigint` →
  `4294967295`; `'80000000'` → `2147483648` (unsigned, all five).
  `pg_typeof(sum(...::bigint))` → **numeric**; `sum` of 3×4294967295 → `12884901885`, of
  10000× → `42949672950000` (exact integer text, no overflow, no scientific notation).
- **timestamp printed form stable under the settings.** Fractional trailing zeros trimmed
  (`.123000`→`.123`); `timestamp(6)` of a whole second → no fraction; `timestamp(3)` truncates
  `.123456`→`.123` (precision reduction is a real change → caught). `infinity`/`-infinity`/
  BC/year-10000 render identically on all five. `date`, `time` stable. `timetz` printed form
  is session-TZ-independent (outputs its stored offset: `12:34:56.789+05:30` under UTC,
  Asia/Kolkata and America/New_York on 18.7).
- **jsonb / ranges / uuid / bool / inet / bit / name printed forms stable 9.4→18.**
  jsonb key order (`{"a": 2, "b": 1}`), numeric-scale preservation (`{"a": 1.00}`),
  `int4range(1,5)` = `'[1,4]'::int4range` → `[1,5)` (canonical), `numrange` `[1.1,2.2)`,
  `tsrange` under ISO/UTC, uuid lowercased, `true`, `192.168.1.0/24`, `1010`, `hello` —
  all identical on all five. `array_send` bytes identical on all five (element-type OID
  embedded, so int[] vs bigint[] differ — type change flagged anyway).
- **Key columns are inside the row hash** (`acct_id`, `bank_code` are the first two ROW fields,
  L30–31) → no key-omission false PASS. A key change moves the row's bucket → caught.
- **Counted-set duplicate logic sound** (sums count multiplicity, unlike XOR) — proven by
  T001/T002 and re-confirmed by the unsigned-slice/numeric-sum behaviour above.

---

## 4. Settings block the engine must use (every table, both files)

```
BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL search_path        = pg_catalog;     -- P1: stop function shadowing (tables schema-qualified)
SET LOCAL timezone           = 'UTC';          -- P9: timestamptz canonical
SET LOCAL datestyle          = 'ISO, YMD';
SET LOCAL intervalstyle      = 'iso_8601';     -- P10: missing from no-key file today
SET LOCAL bytea_output       = 'hex';          -- only matters if bytea::text is ever used
SET LOCAL extra_float_digits = 0;              -- P7: printed-form floats/arrays/ranges (verified stable)
SET LOCAL xmloption          = 'document';     -- P16: pin for xml columns (choose one, both sides)
SET LOCAL statement_timeout  = '<run window>'; -- P17: required by PLAN 3.1, missing today
SET LOCAL lock_timeout       = '5s';
SET LOCAL application_name   = 'dmux';
```

`lc_monetary` cannot be safely forced per-transaction for money already stored; instead the
engine **records and compares** it (P11) and uses `money→numeric`, which is locale-free.
Scalar floats stay on `float8send`/`float4send` bytes regardless of `extra_float_digits`.

---

## 5. Normalisation rules per type (engine, PLAN §3.2 made concrete)

| Type | Rule | Verified basis |
|---|---|---|
| text, varchar | exact UTF-8 bytes (`convert_to(x,'UTF8')`) | battery1 |
| char(n) | `::text` strips trailing pad (bpchar); flag char↔varchar type change | P6 |
| numeric | printed form; on scale change cast **both** sides to the **larger** scale; never down | P4, P8, P14 |
| real (float4) | `float4send` bytes; if mapped to float8, cast both to float8 then `float8send` | battery2 |
| double precision | `float8send` bytes (never printed form) | P7, P15 |
| date | `::text` ISO | battery3 |
| timestamp | `::text` ISO under UTC; precision reduction is a real change | battery3 |
| timestamptz | `::text` under `SET LOCAL timezone='UTC'`; flag timestamp↔timestamptz | P9 |
| time | `::text`; timetz outputs stored offset (TZ-independent) | battery3 |
| interval | `::text` under `intervalstyle='iso_8601'`; days≠hours components | P10 |
| bytea | `md5(bytea)` raw bytes (NULL≠empty); not `::text` | battery1/3 |
| money | `::numeric` (locale-free); record/compare lc_monetary; never `::text` | P11 |
| json | exact printed text (whitespace/key-order sensitive) | P13 |
| jsonb | printed form (canonical, version-stable); **flag json→jsonb** (reorder/dedup) | P12 |
| xml | printed form, pin `xmloption`, report warning | P16 |
| arrays | printed form with `extra_float_digits=0`, **or** `array_send` bytes; float elements by bytes | P7 |
| ranges | printed form under fixed settings; discrete ranges canonicalise; float ranges need P7 | battery4 |
| uuid/bool/inet/cidr/bit/name | printed form (stable) | battery4 |
| extension/unknown | printed form + report warning | PLAN §3.2 |
| NULL | own marker (empty ROW field), never equal to empty value | battery1 |
| SQL_ASCII database | flag for review; convert_to does not convert | P19 |

---

## 6. Test cases for T004 (Nina extends prove.sh)

Each case follows the existing harness pattern: load identical data on both sides, run the
fingerprint, expect `same`; plant one change on the new side, expect `differ`; undo, expect
`same`. New tables live in the `phase0` schema. Engine normalisation per §5 must be applied
by the fingerprint template Nina uses (settings block §4). Values are exact.

### 6.1 timestamptz — `phase0.tz_test(id int PRIMARY KEY, ts timestamptz)`
- Stay quiet: old `ts = timestamptz '2001-01-01 00:00:00+05:30'`, new
  `ts = timestamptz '2000-12-31 18:30:00+00'` (same instant). Under UTC both print
  `2000-12-31 18:30:00+00` → `same`. (Also run with the two servers' session TZ differing to
  prove `SET LOCAL timezone='UTC'` normalises.)
- Must catch: new `ts = ts + interval '1 second'` → `differ`.
- Schema-diff case (separate): old column `timestamp`, new `timestamptz` → type change amber;
  value comparison must not silently PASS the semantic shift.

### 6.2 money — `phase0.money_test(id int PRIMARY KEY, amt money)`
- Stay quiet: old `amt = '1234.56'::money` (lc_monetary C), new `amt = '1234.56'::money`
  (lc_monetary en_US.utf8). Engine hashes `amt::numeric` → `1234.56` both → `same`.
- Must catch: new `amt = '1234.57'::money` → `differ`.
- Guard: assert the engine never hashes `money::text`; record lc_monetary mismatch as a warning.

### 6.3 json / jsonb — `phase0.json_test(id int PRIMARY KEY, doc json)` and `phase0.jsonb_test(id int PRIMARY KEY, doc jsonb)`
- json stay quiet: both `doc = '{"b":1,"a":2}'` (exact text) → `same`.
- json must catch: new `doc = '{"b":2,"a":2}'` → `differ`.
- jsonb stay quiet: both `doc = '{"b":1,"a":2}'::jsonb` (stored canonical `{"a": 2, "b": 1}`) → `same`.
- jsonb must catch: new `doc = '{"a":3,"b":1}'::jsonb` → `differ`.
- json→jsonb type change (must NOT stay quiet): old `json '{"b":1,"a":2}'`, new
  `jsonb '{"a":2,"b":1}'` → schema diff flags type change AND value diff reported.
- Dedup data-loss case: old `json '{"a":1,"a":2}'`, new `jsonb '{"a":2}'` → flagged (duplicate
  key dropped), never silent PASS.

### 6.4 xml — `phase0.xml_test(id int PRIMARY KEY, doc xml)`
- Stay quiet: both `doc = '<a x="1">t</a>'::xml` (pin xmloption) → `same`.
- Must catch: new `doc = '<a x="2">t</a>'::xml` → `differ`.

### 6.5 arrays — `phase0.arr_test(id int PRIMARY KEY, fa float8[], ia int[])`
- Stay quiet: both `fa = ARRAY[0.1::float8, 1::float8/3]`, `ia = ARRAY[1,2,3]`. Engine pins
  `extra_float_digits=0` (or `array_send`) → `same` across 9.4→18.
- Must catch (float element): new `fa = ARRAY[0.2::float8, 1::float8/3]` → `differ`.
- Must catch (int element): new `ia = ARRAY[1,2,4]` → `differ`.
- NULL element stay quiet: both `ia = ARRAY[NULL::int,1]` → `same`; must catch `ARRAY[NULL::int,2]`.
- Note: without the P7 fix this case false-FAILs on 9.4→12+ — it is the regression test for P7.

### 6.6 ranges — `phase0.rng_test(id int PRIMARY KEY, ir int4range, nr numrange)`
- Stay quiet: old `ir = int4range(1,5)`, new `ir = '[1,4]'::int4range` (both canonical `[1,5)`);
  both `nr = numrange(1.1,2.2)` → `same`.
- Must catch: new `ir = int4range(1,6)` → `differ`; new `nr = numrange(1.1,2.3)` → `differ`.

### 6.7 interval — `phase0.iv_test(id int PRIMARY KEY, iv interval)`
- Stay quiet: both `iv = interval '1 year 2 mons 3 days 04:05:06'` (iso_8601 `P1Y2M3DT4H5M6S`) → `same`.
- Must catch: new `iv = iv + interval '1 second'` → `differ`.
- Component trap (must catch): old `iv = interval '24 hours'` (`PT24H`), new
  `iv = interval '1 day'` (`P1D`) → `differ` (distinct internal values).
- Regression for P10: run via the **no-key** template too, to prove intervalstyle is now set.

### 6.8 numeric scale change — old `numeric(20,4)`, new `numeric(20,6)`
- Stay quiet: old `1234.5678`, new `1234.567800`; engine casts both to scale 6 → `1234.567800` → `same`.
- Must catch: new `1234.567900` (0.0001 change) → `differ`.
- False-PASS guard (P4): two rows `1.2344` and `1.2345` must remain distinct; assert the engine
  never casts to the smaller scale (which would print both `1.23`).

### 6.9 NULL bytea — `phase0.bin_test(id int PRIMARY KEY, b bytea)`
- Stay quiet: both `b = NULL` → `same`; both `b = ''::bytea` → `same`.
- Must catch: NULL→`''::bytea` and `''::bytea`→NULL (each) → `differ`.
- Must catch: `decode('00ff','hex')` → `decode('00fe','hex')` (one byte) → `differ`.

### 6.10 empty table — `phase0.empty_test(id int PRIMARY KEY)`
- Stay quiet: empty on both sides → fingerprint 0 rows, count 0, PASS (engine records explicit
  zero marker; P5).
- Must catch: insert one row on new side → a bucket appears → `differ`.
- Guard: a missing table (dropped on new) must be reported by inventory, not as empty-PASS.

### 6.11 added / dropped column (schema diff, not value hash)
- Add `extra int DEFAULT 0` on the new side: the mapped-column fingerprint stays `same`
  (expected), but schema diff MUST report added (green) and the overall verdict MUST NOT be a
  silent PASS (P3). Dropped column → red. Assert coverage line lists the unmapped column.

### 6.12 different collations (needs new DBs/containers — Nina's harness writes)
- Create old and new both UTF8, old `LC_COLLATE=C`, new `en_US.utf8` (or an ICU collation on
  15+). Load identical text → stay quiet (byte hashing is collation-independent).
- Must catch: change `'a'`→`'A'` in a name → `differ` even if the new collation treats them as
  equal. (Current matrix already covers C/LATIN1 vs en_US.utf8/UTF8; this adds same-encoding.)

### 6.13 SQL_ASCII (needs a SQL_ASCII DB — Nina)
- Pure-ASCII data on a SQL_ASCII side: bytes hash equal, but engine MUST flag the encoding for
  review (P19), never silent PASS on the encoding question.
- Non-ASCII: LATIN1 `chr(233)` vs SQL_ASCII raw byte `0xE9` vs UTF-8 `0xC3A9` — demonstrate
  `convert_to` passthrough on SQL_ASCII and that the engine flags rather than guesses.

---

## 7. Bucket-sum overflow bound (P-note; PLAN ~10,000 rows/bucket)

- Each 32-bit slice is **unsigned**: `('x'||'ffffffff')::bit(32)::bigint = 4294967295`
  (verified all five). Slice range `[0, 2³²−1]`.
- In SQL, `sum(bigint)` returns **numeric** (verified `pg_typeof`), so the database sum cannot
  overflow — arbitrary precision, exact integer text.
- If the engine accumulates in **signed 64-bit** (i64 max `9223372036854775807`), worst-case
  bucket sum `= N × 4294967295`. Overflow when
  `N > floor(9223372036854775807 / 4294967295) = 2147483648` (≈2³¹, 2.147 billion rows/bucket).
- At PLAN's ~10,000 rows/bucket: max sum ≈ `4.295×10¹³`, ~214,748× below i64 max. Even 20 million
  rows in a single bucket: `8.59×10¹⁶ < 9.22×10¹⁸` (safe, ~107× margin). With u64 the bound
  doubles (~4.29×10⁹ rows).
- **Engine requirements:** read sums as exact integers (i64/u64/numeric), never f64; use
  checked (non-wrapping) arithmetic; keep bucket sizing far below 2³¹ rows (trivially satisfied).
  Four slices + count make accidental cancellation infeasible (≈2⁻¹²⁸ for all four sums to
  coincide); the only practical collision route is the adversarial md5 weakness in P2.

---

## 8. Residual risks to record in the report / "How Dmux decides"

- md5 is not collision-resistant against a deliberate chosen-prefix attack (P2). For accidental
  migration errors the probability is negligible; state the threat model explicitly.
- Byte-level float comparison flags `-0.0` vs `0.0` and any bit change (P15) — by design.
- bpchar trailing-space insignificance can hide trailing-space loss on varchar→char (P6);
  the type change is flagged amber.
- Unconstrained numeric per-value scale drift can false-FAIL (P14); offer the round option.
- SQL_ASCII encoding is unverifiable (P19); always flag.
- json is exact-text; reformatting is a real change (P13).

---

## 9. Method appendix (reproducibility)

All probes were read-only. Multi-statement batches that needed a setting used a single
`psql -c "SELECT set_config(...,true); SELECT ...; ..."` so the setting stayed transaction-local
(one implicit transaction, never persisted). No `SET` outside a query, no DDL, no temp tables,
no writes; the `phase0` schema does not exist during this review (prove.sh drops it). Batteries
were run on all five ports and diffed against 9.4.26; the only cross-version differences found
were the expected ones (raw pre-`convert_to` encoding byte; float printed form under default
`extra_float_digits`). SQL_ASCII behaviour was confirmed from the PostgreSQL 18.7 documentation
(§23.3.1 and Table 23.4) because no SQL_ASCII server is in the local matrix.
