-- edge_bank: the OLD database (loaded on Postgres 9.4/9.6, LATIN1).
--
-- Every statement here must run on Postgres 9.4: no declarative partitioning,
-- no GENERATED columns, no gen_random_uuid(). Everything is deterministic
-- (no random(), no now()), so a rebuild gives byte-identical data.
-- All values are invented. build.sh copies this database to the new server and
-- then applies new.sql there, which plants the differences.
SET client_encoding = 'UTF8';
SET client_min_messages = warning;

CREATE EXTENSION ltree;

CREATE SCHEMA same;
CREATE SCHEMA differs;
CREATE SCHEMA structure;
CREATE SCHEMA tricky;
CREATE SCHEMA big;

-- ===========================================================================
-- same: must stay quiet
-- ===========================================================================

-- Accented names: LATIN1 bytes on the old side, UTF-8 on the new side.
CREATE TABLE same.accented_names (
  id integer PRIMARY KEY, name text NOT NULL, city varchar(40), note text);
INSERT INTO same.accented_names
SELECT i,
  (ARRAY['José','Zoë','Søren','François','Åsa','Müller','Núñez','Çelik','Ángel','Bjørk','Renée','Ñandú'])[1 + i % 12]
    || ' ' || (ARRAY['Østergård','Lázaro','Dvorák','Skoda','Håkansson','Pérez','Böhm','Ångström'])[1 + i % 8],
  (ARRAY['Zürich','São Paulo','Málaga','Tårnby','Köln','Besançon'])[1 + i % 6],
  CASE WHEN i % 5 = 0 THEN 'caf' || chr(233) || ' £' || i ELSE NULL END
FROM generate_series(1, 60) i;

-- Floats: 9.6 prints them differently from 12+, the values are the same.
CREATE TABLE same.float_values (
  id integer PRIMARY KEY, f4 real, f8 double precision,
  f8_list double precision[], label text);
INSERT INTO same.float_values
SELECT i, (i / 7.0)::real, i / 3.0,
  ARRAY[i / 3.0, i * 0.1, 1e-7 * i, i / 9.0]::double precision[], 'row ' || i
FROM generate_series(1, 100) i;
INSERT INTO same.float_values VALUES
  (1001, 0.1, 0.1, ARRAY[0.1, 0.2, 0.3]::double precision[], 'ordinary tenths'),
  (1002, 'NaN', 'Infinity', ARRAY['-Infinity', 'NaN']::double precision[], 'special values'),
  (1003, 3.4028235e38, 1.7976931348623157e308, NULL, 'largest values'),
  (1004, 1.17549435e-38, 4.9e-324, ARRAY[]::double precision[], 'smallest values');

-- NULL and empty text are different things; both sides keep them apart.
CREATE TABLE same.null_vs_empty (
  id integer PRIMARY KEY, a text, b text, c integer);
INSERT INTO same.null_vs_empty
SELECT i,
  CASE i % 3 WHEN 0 THEN NULL WHEN 1 THEN '' ELSE 'text ' || i END,
  CASE i % 4 WHEN 0 THEN '' WHEN 1 THEN NULL ELSE ' ' END,
  CASE WHEN i % 2 = 0 THEN NULL ELSE i END
FROM generate_series(1, 90) i;

-- char(n) pads with spaces; padding is not a difference.
CREATE TABLE same.padded_chars (
  id integer PRIMARY KEY, code char(8), label char(3));
INSERT INTO same.padded_chars
SELECT i, 'C' || i, (ARRAY['A','AB','ABC'])[1 + i % 3]
FROM generate_series(1, 40) i;

-- Same rows, but the new side stores them in a different physical order.
CREATE TABLE same.shuffled_rows (
  id integer PRIMARY KEY, name text, amount numeric(10,2));
INSERT INTO same.shuffled_rows
SELECT i, 'customer ' || i, (i * 37 % 1000) / 4.0 FROM generate_series(1, 500) i;

-- Same columns, but the new side lists them in a different order.
CREATE TABLE same.reordered_columns (
  id integer PRIMARY KEY, first_name text, last_name text,
  balance numeric(12,2), opened date);
INSERT INTO same.reordered_columns
SELECT i, 'First' || i, 'Last' || (i % 17), i * 12.5,
  DATE '2015-01-01' + (i * 13 % 3000)
FROM generate_series(1, 200) i;

-- A plain table here; a partitioned table on the new side.
CREATE TABLE same.ledger_partitioned (
  id integer NOT NULL, posted_on date NOT NULL, amount numeric(12,2),
  memo text, PRIMARY KEY (id, posted_on));
INSERT INTO same.ledger_partitioned
SELECT i, DATE '2024-01-01' + (i * 7 % 360), (i * 53 % 10000) / 100.0, 'entry ' || i
FROM generate_series(1, 1200) i;

-- char(1) becomes varchar(1): an amber type change, the values still match.
CREATE TABLE same.flag_char_to_varchar (id integer PRIMARY KEY, flag char(1), note text);
INSERT INTO same.flag_char_to_varchar
SELECT i, (ARRAY['Y','N','U'])[1 + i % 3], 'row ' || i FROM generate_series(1, 60) i;

-- numeric(12,2) becomes numeric(12,4): the scale is raised, the numbers match.
CREATE TABLE same.numeric_scale_raised (id integer PRIMARY KEY, rate numeric(12,2));
INSERT INTO same.numeric_scale_raised
SELECT i, (i * 7 % 5000) / 100.0 FROM generate_series(1, 80) i;

-- json stays json (the text, spacing and key order are kept exactly).
CREATE TABLE same.json_kept (id integer PRIMARY KEY, payload json);
INSERT INTO same.json_kept
SELECT i, ('{"id": ' || i || ', "tags": ["a", "b"],  "name": "item ' || i
  || '", "z": null, "a": {"k": ' || (i % 5) || '}}')::json
FROM generate_series(1, 40) i;

CREATE TYPE same.mood AS ENUM ('happy', 'neutral', 'sad');
CREATE DOMAIN same.short_code AS varchar(6) CHECK (VALUE = upper(VALUE));
-- Many less common types in one table, all identical on both sides.
CREATE TABLE same.rich_types (
  id integer PRIMARY KEY,
  tags text[], nums integer[], span int4range, days daterange,
  feeling same.mood, code same.short_code, blob bytea,
  seen_at timestamptz, wall_clock timestamp, took interval, ref uuid,
  host inet, net cidr, flag boolean, cost money);
INSERT INTO same.rich_types
SELECT i,
  ARRAY['t' || i, NULL, 'x y'], ARRAY[i, NULL, i * 2],
  int4range(i, i + 10), daterange(DATE '2020-01-01' + i, DATE '2020-03-01' + i),
  (ARRAY['happy','neutral','sad'])[1 + i % 3]::same.mood,
  ('AB' || i)::same.short_code,
  decode(lpad(to_hex(i * 40503 % 65536), 4, '0') || '00ff', 'hex'),
  TIMESTAMPTZ '2021-06-15 12:00:00+02' + (i || ' hours')::interval,
  TIMESTAMP '2021-06-15 12:00:00.123456' + (i || ' minutes')::interval,
  (i || ' days ' || i % 24 || ' hours')::interval,
  md5(i::text)::uuid,
  ('10.0.' || i % 250 || '.' || i % 7 + 1)::inet,
  ('192.168.' || i % 250 || '.0/24')::cidr,
  i % 2 = 0, i * 1.25
FROM generate_series(1, 30) i;

-- No primary key, and duplicate rows: the copy keeps every duplicate.
CREATE TABLE same.keyless_with_duplicates (code text, qty integer, note text);
INSERT INTO same.keyless_with_duplicates
SELECT 'SKU-' || (i % 10), (i % 10) * 5, 'bin ' || (i % 3)
FROM generate_series(1, 60) i;
INSERT INTO same.keyless_with_duplicates
SELECT 'SKU-X', 1, NULL FROM generate_series(1, 4);

-- ===========================================================================
-- differs: data differences Dmux must find
-- ===========================================================================

-- 3 rows missing on the new side.
CREATE TABLE differs.missing_rows (id integer PRIMARY KEY, name text, amount numeric(10,2));
INSERT INTO differs.missing_rows
SELECT i, 'account ' || i, i * 3.5 FROM generate_series(1, 300) i;

-- 2 extra rows on the new side.
CREATE TABLE differs.extra_rows (id integer PRIMARY KEY, name text, amount numeric(10,2));
INSERT INTO differs.extra_rows
SELECT i, 'account ' || i, i * 3.5 FROM generate_series(1, 300) i;

-- A wide table: one cell in one row changed.
DO $$
DECLARE cols text := ''; vals text := '';
BEGIN
  FOR n IN 1..30 LOOP
    cols := cols || format(', c%s text', lpad(n::text, 2, '0'));
    vals := vals || format(', ''v%s-'' || i', n);
  END LOOP;
  EXECUTE 'CREATE TABLE differs.wide_one_value (id integer PRIMARY KEY' || cols || ')';
  EXECUTE 'INSERT INTO differs.wide_one_value SELECT i' || vals || ' FROM generate_series(1, 200) i';
END $$;

-- A balance off by 0.01 (the control total of the balance column shows it).
CREATE TABLE differs.balances (
  account_no integer PRIMARY KEY, owner text, balance numeric(14,2));
INSERT INTO differs.balances
SELECT i, 'owner ' || i, (i * 9973 % 1000000) / 100.0 FROM generate_series(1, 400) i;

-- Empty text becomes NULL on the new side.
CREATE TABLE differs.empty_to_null (id integer PRIMARY KEY, middle_name text);
INSERT INTO differs.empty_to_null
SELECT i, CASE WHEN i % 4 = 0 THEN '' ELSE 'M' || i END FROM generate_series(1, 100) i;

-- The time of day is dropped on the new side.
CREATE TABLE differs.time_dropped (id integer PRIMARY KEY, opened_at timestamp);
INSERT INTO differs.time_dropped
SELECT i, TIMESTAMP '2022-03-01 00:00:00' + (i || ' days')::interval + ((i * 7919) % 86400 || ' seconds')::interval
FROM generate_series(1, 100) i;

-- A keyless table where one copy of a duplicated row is gone on the new side.
CREATE TABLE differs.keyless_duplicate_removed (sku text, qty integer);
INSERT INTO differs.keyless_duplicate_removed
SELECT 'SKU-' || (i % 20), 1 + i % 4 FROM generate_series(1, 100) i;

-- Trailing spaces added on the new side (the ignore-trailing-spaces option
-- turns this into PASS (with options); see run-options.toml).
CREATE TABLE differs.trailing_spaces (id integer PRIMARY KEY, name text, city text);
INSERT INTO differs.trailing_spaces
SELECT i, 'name ' || i, 'city ' || (i % 9) FROM generate_series(1, 120) i;

-- json becomes jsonb on the new side (reformatted, so Dmux flags it).
CREATE TABLE differs.json_to_jsonb (id integer PRIMARY KEY, payload json);
INSERT INTO differs.json_to_jsonb
SELECT i, ('{"b": ' || i || ',  "a": [1,2,  3], "dup": 1, "dup": 2}')::json
FROM generate_series(1, 30) i;

-- A varchar column widened on the new side.
CREATE TABLE differs.varchar_widened (id integer PRIMARY KEY, short_name varchar(20));
INSERT INTO differs.varchar_widened
SELECT i, 'name ' || i FROM generate_series(1, 50) i;

-- Widely different: 40 percent of the rows have another amount on the new side.
CREATE TABLE differs.widely_different (
  id integer PRIMARY KEY, code text, amount numeric(12,2));
INSERT INTO differs.widely_different
SELECT i, 'CODE-' || i, (i * 31 % 100000) / 100.0 FROM generate_series(1, 20000) i;

-- ===========================================================================
-- structure: the shape of the database changed
-- ===========================================================================
CREATE TABLE structure.dropped_column (id integer PRIMARY KEY, name text, legacy_flag char(1));
INSERT INTO structure.dropped_column SELECT i, 'row ' || i, 'Y' FROM generate_series(1, 40) i;

CREATE TABLE structure.added_column (id integer PRIMARY KEY, name text);
INSERT INTO structure.added_column SELECT i, 'row ' || i FROM generate_series(1, 40) i;

CREATE TABLE structure.renamed_column (id integer PRIMARY KEY, name text, phone text);
INSERT INTO structure.renamed_column
SELECT i, 'row ' || i, '555-01' || lpad(i::text, 2, '0') FROM generate_series(1, 40) i;

-- Renamed on the new side to structure.customer_notes.
CREATE TABLE structure.client_notes (id integer PRIMARY KEY, note text);
INSERT INTO structure.client_notes SELECT i, 'note ' || i FROM generate_series(1, 40) i;

-- A sequence that the new side left behind at a lower value.
CREATE SEQUENCE structure.invoice_seq_behind;
SELECT setval('structure.invoice_seq_behind', 5000);
-- A sequence that the new side never got.
CREATE SEQUENCE structure.receipt_seq_dropped;
SELECT setval('structure.receipt_seq_dropped', 777);

-- An index missing on the new side.
CREATE TABLE structure.missing_index (id integer PRIMARY KEY, email text);
INSERT INTO structure.missing_index SELECT i, 'user' || i || '@example.test' FROM generate_series(1, 40) i;
CREATE INDEX missing_index_email_idx ON structure.missing_index (email);

-- A check constraint that is NOT VALID on the new side.
CREATE TABLE structure.not_valid_constraint (
  id integer PRIMARY KEY, amount numeric(10,2),
  CONSTRAINT amount_not_negative CHECK (amount >= 0));
INSERT INTO structure.not_valid_constraint SELECT i, i FROM generate_series(1, 40) i;

-- Three large objects; one is missing on the new side.
SELECT lo_from_bytea(7770001, 'first large object'::bytea);
SELECT lo_from_bytea(7770002, 'second large object, gone on the new side'::bytea);
SELECT lo_from_bytea(7770003, 'third large object'::bytea);

-- ===========================================================================
-- tricky
-- ===========================================================================

-- A column type Dmux does not know how to compare safely (ltree extension).
CREATE TABLE tricky.unknown_type (id integer PRIMARY KEY, path ltree, name text);
INSERT INTO tricky.unknown_type
SELECT i, ('top.branch' || i % 5 || '.leaf' || i)::ltree, 'node ' || i FROM generate_series(1, 30) i;

-- Row-level security: a login without BYPASSRLS only sees branch 1.
CREATE TABLE tricky.rls_branch_notes (id integer PRIMARY KEY, branch_id integer, note text);
INSERT INTO tricky.rls_branch_notes
SELECT i, CASE WHEN i <= 10 THEN 1 ELSE 2 END,
  'branch ' || CASE WHEN i <= 10 THEN 1 ELSE 2 END || ' note ' || i
FROM generate_series(1, 20) i;
ALTER TABLE tricky.rls_branch_notes ENABLE ROW LEVEL SECURITY;
CREATE POLICY branch_notes_visible ON tricky.rls_branch_notes FOR SELECT USING (branch_id = 1);

-- A sequence at the largest bigint, identical on both sides.
CREATE SEQUENCE tricky.big_ids_seq;
SELECT setval('tricky.big_ids_seq', 9223372036854775807);
CREATE TABLE tricky.big_ids (id bigint PRIMARY KEY, name text);
INSERT INTO tricky.big_ids VALUES
  (9223372036854775805, 'two below the top'),
  (9223372036854775806, 'one below the top'),
  (9223372036854775807, 'the top');
-- A sequence at the largest bigint on the old side and one lower on the new side.
CREATE SEQUENCE tricky.big_ids_seq_off_by_one;
SELECT setval('tricky.big_ids_seq_off_by_one', 9223372036854775807);

-- ===========================================================================
-- big: a larger table, to see estimates, progress and time remaining
-- ===========================================================================
CREATE TABLE big.ledger_entries (
  id bigint PRIMARY KEY, account_id integer NOT NULL, posted_at timestamptz NOT NULL,
  amount numeric(14,2) NOT NULL, memo text);
INSERT INTO big.ledger_entries
SELECT i, (i * 7919 % 50000)::integer,
  TIMESTAMPTZ '2020-01-01 00:00:00+00' + ((i * 37 % 157680000) || ' seconds')::interval,
  ((i * 104729) % 10000000) / 100.0,
  CASE WHEN i % 11 = 0 THEN NULL ELSE 'ref-' || (i * 31 % 999983) END
FROM generate_series(1::bigint, 3000000::bigint) i;
