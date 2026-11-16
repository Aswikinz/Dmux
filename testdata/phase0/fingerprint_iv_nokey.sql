-- Dmux phase0 fingerprint for phase0.iv_nk, a table with NO primary key
-- (T004; T003 review §6.7, regression for P10). The bucket comes from the row
-- hash itself, so rows are compared as a counted set. This proves the no-key
-- template sets intervalstyle='iso_8601' too: before T004 the no-key settings
-- block lacked it, and interval columns would print in the session's style.
-- Normalisation (§5): iv interval -> ::text under iso_8601; '24 hours' (PT24H)
-- and '1 day' (P1D) stay distinct.
-- Output: bucket | n | s1 | s2 | s3 | s4. Order-independent; Postgres 9.4-safe.

-- Canonical settings block (T003 review section 4), identical to fingerprint.sql.
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

SELECT ('x' || substr(rh, 1, 3))::bit(12)::int              AS bucket,
       count(*)                                             AS n,
       sum(('x' || substr(rh,  1, 8))::bit(32)::bigint)     AS s1,
       sum(('x' || substr(rh,  9, 8))::bit(32)::bigint)     AS s2,
       sum(('x' || substr(rh, 17, 8))::bit(32)::bigint)     AS s3,
       sum(('x' || substr(rh, 25, 8))::bit(32)::bigint)     AS s4
FROM (
    SELECT md5(convert_to(ROW(iv::text)::text, 'UTF8')) AS rh  -- interval: iso_8601 (P10)
    FROM phase0.iv_nk
) q
GROUP BY 1
ORDER BY 1;

COMMIT;
