-- Dmux reference fingerprint query (PLAN.md section 3.1).
-- Written for the phase0.accounts test table; the engine generates the same
-- shape for any table from its mapping. Uses nothing newer than Postgres 9.4.
--
-- Output: one line per bucket: bucket | row count | four hash sums.
-- The result does not depend on row order, sort order or database encoding.

-- Canonical settings block (T003 review section 4). The engine must apply the
-- same block to every table; statement_timeout is the harness window here.
BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL search_path        = pg_catalog;   -- P1: stop function shadowing (tables schema-qualified)
SET LOCAL timezone           = 'UTC';        -- P9: timestamptz canonical
SET LOCAL datestyle          = 'ISO, YMD';
SET LOCAL intervalstyle      = 'iso_8601';   -- P10: interval printed form
SET LOCAL bytea_output       = 'hex';        -- only matters if bytea::text is ever used
SET LOCAL extra_float_digits = 0;            -- P7: printed-form floats/arrays/ranges
SET LOCAL xmloption          = 'document';   -- P16: pin for xml columns
SET LOCAL statement_timeout  = '300s';       -- P17: harness window; engine uses the run window
SET LOCAL lock_timeout       = '5s';
SET LOCAL application_name   = 'dmux';

SELECT ('x' || substr(kh, 1, 3))::bit(12)::int              AS bucket,
       count(*)                                             AS n,
       sum(('x' || substr(rh,  1, 8))::bit(32)::bigint)     AS s1,
       sum(('x' || substr(rh,  9, 8))::bit(32)::bigint)     AS s2,
       sum(('x' || substr(rh, 17, 8))::bit(32)::bigint)     AS s3,
       sum(('x' || substr(rh, 25, 8))::bit(32)::bigint)     AS s4
FROM (
    SELECT
        -- key hash: decides the bucket, so the same row lands in the same
        -- bucket on both sides
        md5(convert_to(ROW(acct_id::text, bank_code::text)::text, 'UTF8')) AS kh,
        -- row hash: every mapped column, normalised (PLAN.md section 3.2).
        -- ROW(...)::text keeps NULL and '' apart and escapes separators.
        md5(convert_to(ROW(
            acct_id::text,                   -- varchar: exact
            bank_code::text,
            holder_name::text,
            branch_code::text,               -- char(n): cast drops pad spaces
            balance::text,                   -- numeric: printed form
            encode(float8send(rate), 'hex'), -- float: bytes, not printed form
            opened_at::text,                 -- timestamp: ISO under SET LOCAL
            md5(doc_blob),                   -- bytea: hash of raw bytes
            remarks::text
        )::text, 'UTF8')) AS rh
    FROM phase0.accounts
) q
GROUP BY 1
ORDER BY 1;

COMMIT;
