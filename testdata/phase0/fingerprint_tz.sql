-- Dmux phase0 fingerprint: phase0.tz_test (T004; T003 review §6.1).
-- Normalisation (§5): ts timestamptz -> ::text under SET LOCAL timezone='UTC'
-- (P9). The same instant written with different offsets prints identically.
-- Output: bucket | n | s1 | s2 | s3 | s4. Order-independent; Postgres 9.4-safe.

-- Canonical settings block (T003 review section 4).
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
    SELECT md5(convert_to(ROW(id::text)::text, 'UTF8')) AS kh,
           md5(convert_to(ROW(
               id::text,                     -- int key: printed form
               ts::text                      -- timestamptz: ISO under UTC (P9)
           )::text, 'UTF8')) AS rh
    FROM phase0.tz_test
) q
GROUP BY 1
ORDER BY 1;

COMMIT;
