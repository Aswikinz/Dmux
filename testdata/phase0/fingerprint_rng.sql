-- Dmux phase0 fingerprint: phase0.rng_test (T004; T003 review §6.6).
-- Normalisation (§5): ranges -> printed form under the fixed settings.
-- Discrete ranges canonicalise: int4range(1,5) and '[1,4]'::int4range both
-- print '[1,5)'. Continuous numrange keeps its bounds' numeric printed form
-- ('[1.1,2.2)'), stable 9.4 -> 18. Float ranges would need the P7 pin (set above).
-- Output: bucket | n | s1 | s2 | s3 | s4. Order-independent; Postgres 9.4-safe
-- (ranges exist since 9.2).

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
               ir::text,                     -- int4range: discrete, canonical '[1,5)'
               nr::text                      -- numrange: printed bounds, continuous
           )::text, 'UTF8')) AS rh
    FROM phase0.rng_test
) q
GROUP BY 1
ORDER BY 1;

COMMIT;
