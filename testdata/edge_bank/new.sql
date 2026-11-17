-- edge_bank: plant the differences in the NEW database only (Postgres 17).
--
-- Applied by build.sh after the old database was copied here, so both sides
-- start identical. The old database is never changed by this file.
SET client_min_messages = warning;
SET client_encoding = 'UTF8';

DO $$
BEGIN
  IF current_setting('server_version_num')::int < 100000 THEN
    RAISE EXCEPTION 'new.sql needs the NEW database (Postgres 10 or newer)';
  END IF;
END $$;

-- ---- same: differences that must NOT matter --------------------------------

-- Different physical row order: empty the table and refill it backwards.
CREATE TEMP TABLE shuffle_copy AS SELECT * FROM same.shuffled_rows;
TRUNCATE same.shuffled_rows;
INSERT INTO same.shuffled_rows SELECT * FROM shuffle_copy ORDER BY id DESC;
-- The keyless table is refilled in another order too.
CREATE TEMP TABLE keyless_copy AS SELECT * FROM same.keyless_with_duplicates;
TRUNCATE same.keyless_with_duplicates;
INSERT INTO same.keyless_with_duplicates SELECT * FROM keyless_copy ORDER BY note DESC NULLS FIRST, code DESC, qty;

-- Columns in another order.
CREATE TABLE same.reordered_columns_new (
  opened date, balance numeric(12,2), last_name text, id integer PRIMARY KEY, first_name text);
INSERT INTO same.reordered_columns_new (opened, balance, last_name, id, first_name)
SELECT opened, balance, last_name, id, first_name FROM same.reordered_columns ORDER BY id DESC;
DROP TABLE same.reordered_columns;
ALTER TABLE same.reordered_columns_new RENAME TO reordered_columns;
ALTER INDEX same.reordered_columns_new_pkey RENAME TO reordered_columns_pkey;

-- A plain table became a partitioned table.
CREATE TEMP TABLE ledger_copy AS SELECT * FROM same.ledger_partitioned;
DROP TABLE same.ledger_partitioned;
CREATE TABLE same.ledger_partitioned (
  id integer NOT NULL, posted_on date NOT NULL, amount numeric(12,2),
  memo text, PRIMARY KEY (id, posted_on)) PARTITION BY RANGE (posted_on);
CREATE TABLE same.ledger_partitioned_2024_h1 PARTITION OF same.ledger_partitioned
  FOR VALUES FROM ('2024-01-01') TO ('2024-07-01');
CREATE TABLE same.ledger_partitioned_2024_h2 PARTITION OF same.ledger_partitioned
  FOR VALUES FROM ('2024-07-01') TO ('2025-01-01');
INSERT INTO same.ledger_partitioned SELECT * FROM ledger_copy;

ALTER TABLE same.flag_char_to_varchar ALTER COLUMN flag TYPE varchar(1);
ALTER TABLE same.numeric_scale_raised ALTER COLUMN rate TYPE numeric(12,4);

-- ---- differs: data differences ---------------------------------------------
DELETE FROM differs.missing_rows WHERE id IN (17, 118, 250);
INSERT INTO differs.extra_rows VALUES (9001, 'account 9001', 1.00), (9002, 'account 9002', 2.00);
UPDATE differs.wide_one_value SET c17 = 'v17-changed' WHERE id = 77;
UPDATE differs.balances SET balance = balance + 0.01 WHERE account_no = 123;
UPDATE differs.empty_to_null SET middle_name = NULL WHERE id IN (8, 40, 76) AND middle_name = '';
UPDATE differs.time_dropped SET opened_at = date_trunc('day', opened_at) WHERE id IN (5, 6, 7);
DELETE FROM differs.keyless_duplicate_removed
 WHERE ctid = (SELECT ctid FROM differs.keyless_duplicate_removed WHERE sku = 'SKU-3' LIMIT 1);
UPDATE differs.trailing_spaces SET name = name || '  ' WHERE id IN (1, 2, 3, 4, 5);
ALTER TABLE differs.json_to_jsonb ALTER COLUMN payload TYPE jsonb USING payload::jsonb;
ALTER TABLE differs.varchar_widened ALTER COLUMN short_name TYPE varchar(40);
UPDATE differs.widely_different SET amount = amount + 1.00 WHERE id % 5 IN (0, 1);

-- ---- structure -------------------------------------------------------------
ALTER TABLE structure.dropped_column DROP COLUMN legacy_flag;
ALTER TABLE structure.added_column ADD COLUMN risk_rating text;
ALTER TABLE structure.renamed_column RENAME COLUMN phone TO phone_number;
ALTER TABLE structure.client_notes RENAME TO customer_notes;
SELECT setval('structure.invoice_seq_behind', 4200);
DROP SEQUENCE structure.receipt_seq_dropped;
DROP INDEX structure.missing_index_email_idx;
ALTER TABLE structure.not_valid_constraint DROP CONSTRAINT amount_not_negative;
ALTER TABLE structure.not_valid_constraint
  ADD CONSTRAINT amount_not_negative CHECK (amount >= 0) NOT VALID;
SELECT lo_unlink(7770002);

-- ---- tricky ----------------------------------------------------------------
-- A hidden row (branch 2) differs, so only a login that bypasses row-level
-- security can see this.
UPDATE tricky.rls_branch_notes SET note = 'branch 2 note 20 (changed)' WHERE id = 20;
SELECT setval('tricky.big_ids_seq_off_by_one', 9223372036854775806);
