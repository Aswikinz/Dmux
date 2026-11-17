-- demo_bank: plant every PLAN.md 8.2 difference in the NEW database only.
--
-- Applied by build.sh AFTER the pg_dump/pg_restore copy, to the new database,
-- never to the old one. The old database stays the untouched "before" picture;
-- the new one is what a broken migration would have produced. Nothing here goes
-- to a user database: this is test harness code (Dmux itself only reads).
--
-- Every statement is invented demo data. No real vendor name, dictionary name
-- or real row may be added (decision 11).
--
-- The final DO block asserts every planted change, so a data.sql edit that moves
-- the rows this file relies on fails loudly instead of silently weakening the
-- answer key. testdata/demo_bank/check_key.sh then proves the key against the
-- planted databases; testdata/expected_diffs.json carries the exact answers.
--
-- Runs after build.sh's copy, so it needs a server with declarative
-- partitioning (Postgres 10+): the new side of every mock pair is 12, 15, 17
-- or 18. The old 9.4/9.6 side never sees this file.

SET client_min_messages = warning;

DO $$
BEGIN
  IF current_setting('server_version_num')::int < 100000 THEN
    RAISE EXCEPTION 'plant.sql needs the NEW database (Postgres 10+); this server is %',
      current_setting('server_version');
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = 'demo_bank') THEN
    RAISE EXCEPTION 'plant.sql expects schema demo_bank; run build.sh first';
  END IF;
END;
$$;

-- ---------------------------------------------------------------------------
-- Guard: the copy must still be the deterministic T015 data set. If data.sql
-- ever changes, these numbers change too and the answer key must be rewritten
-- deliberately, not silently.
-- ---------------------------------------------------------------------------
DO $$
DECLARE
  n_txn   integer;
  n_audit integer;
  n_seq   bigint;
BEGIN
  SELECT count(*) INTO n_txn FROM demo_bank.transactions;
  SELECT count(*) INTO n_audit FROM demo_bank.audit_log;
  SELECT last_value INTO n_seq FROM demo_bank.seq_gl_entry;
  IF n_txn <> 20000 THEN
    RAISE EXCEPTION 'transactions: expected 20000 rows before planting, found %', n_txn;
  END IF;
  IF n_audit <> 10495 THEN
    RAISE EXCEPTION 'audit_log: expected 10495 rows before planting, found %', n_audit;
  END IF;
  IF n_seq <> 8000 THEN
    RAISE EXCEPTION 'seq_gl_entry: expected last_value 8000 before planting, found %', n_seq;
  END IF;
END;
$$;

-- ---------------------------------------------------------------------------
-- 8.2 row 1: 12 rows deleted from the transaction history.
-- Keys are spread across the table and avoid the rows changed below (7, 19997,
-- 20000) and the row whose balance changes (account 4999 is unrelated).
-- ---------------------------------------------------------------------------
DO $$
DECLARE
  found integer;
BEGIN
  SELECT count(*) INTO found
  FROM demo_bank.transactions
  WHERE transaction_id IN (137, 1902, 2988, 4221, 5450, 6777,
                           8012, 9334, 10601, 12345, 15118, 19888);
  IF found <> 12 THEN
    RAISE EXCEPTION 'expected the 12 delete candidates to exist, found %', found;
  END IF;
END;
$$;

DELETE FROM demo_bank.transactions
WHERE transaction_id IN (137, 1902, 2988, 4221, 5450, 6777,
                         8012, 9334, 10601, 12345, 15118, 19888);

-- ---------------------------------------------------------------------------
-- 8.2 row 2: 3 duplicate rows added to a table with no key (audit_log).
-- The three source rows are identified by their unique detail text, so the
-- duplicate is a byte-for-byte copy of a real row: the counted set must show
-- each of them twice on the new side.
-- ---------------------------------------------------------------------------
DO $$
DECLARE
  found integer;
BEGIN
  SELECT count(*) INTO found
  FROM demo_bank.audit_log
  WHERE detail IN ('detail 1', 'detail 2', 'detail 3');
  IF found <> 3 THEN
    RAISE EXCEPTION 'expected the 3 duplicate source rows, found %', found;
  END IF;
END;
$$;

INSERT INTO demo_bank.audit_log (event_at, actor, action, object_name, detail)
SELECT event_at, actor, action, object_name, detail
FROM demo_bank.audit_log
WHERE detail IN ('detail 1', 'detail 2', 'detail 3');

-- ---------------------------------------------------------------------------
-- 8.2 row 3: one balance changed by 0.01 (account 4999).
-- The accounts trigger would log the change into audit_log, which would break
-- the exact "+3 duplicate rows" case above, so it is disabled for this one
-- update and enabled again straight after.
-- ---------------------------------------------------------------------------
ALTER TABLE demo_bank.accounts DISABLE TRIGGER trg_accounts_audit;
UPDATE demo_bank.accounts SET balance = balance + 0.01 WHERE account_id = 4999;
ALTER TABLE demo_bank.accounts ENABLE TRIGGER trg_accounts_audit;

-- ---------------------------------------------------------------------------
-- 8.2 row 4: one character changed in one transactions row (19997).
-- ---------------------------------------------------------------------------
UPDATE demo_bank.transactions
SET narration = 'Paxment 19997'
WHERE transaction_id = 19997;

-- ---------------------------------------------------------------------------
-- 8.2 row 5: empty text turned into NULL (transaction 7, narration '').
-- ---------------------------------------------------------------------------
UPDATE demo_bank.transactions
SET narration = NULL
WHERE transaction_id = 7 AND narration = '';

-- ---------------------------------------------------------------------------
-- 8.2 row 6: time of day dropped from one timestamp (transaction 20000).
-- value_timestamp 2024-04-07 05:20:00 becomes 2024-04-07 00:00:00.
-- ---------------------------------------------------------------------------
UPDATE demo_bank.transactions
SET value_timestamp = date_trunc('day', value_timestamp)
WHERE transaction_id = 20000;

-- ---------------------------------------------------------------------------
-- 8.2 row 7: one column dropped (user_roles.granted_by) and one column added
-- (loans.settlement_ref), in two different tables.
-- ---------------------------------------------------------------------------
ALTER TABLE demo_bank.user_roles DROP COLUMN granted_by;
ALTER TABLE demo_bank.loans ADD COLUMN settlement_ref varchar(24);

-- ---------------------------------------------------------------------------
-- 8.2 row 8: one table renamed (limits -> product_limits) and one column
-- renamed (fees.fee_name -> fee_label). The engine must offer a suggested
-- match, not a silent red/green pair.
-- ---------------------------------------------------------------------------
ALTER TABLE demo_bank.limits RENAME TO product_limits;
ALTER TABLE demo_bank.fees RENAME COLUMN fee_name TO fee_label;

-- ---------------------------------------------------------------------------
-- 8.2 row 9: varchar widened on the new side (branches.city 60 -> 120).
-- Same verdict class as the plan's illustrative varchar(50) -> varchar(100):
-- a schema change, amber, values unchanged.
-- ---------------------------------------------------------------------------
ALTER TABLE demo_bank.branches ALTER COLUMN city TYPE varchar(120);

-- ---------------------------------------------------------------------------
-- 8.2 row 10: a sequence left behind its old value. data.sql leaves
-- seq_gl_entry at 8000; the next id it hands out must be 7976, behind the
-- old side's 8001.
-- ---------------------------------------------------------------------------
SELECT setval('demo_bank.seq_gl_entry', 7975, true);

-- ---------------------------------------------------------------------------
-- 8.2 row 11: one index missing (transactions.ix_transactions_posted).
-- ---------------------------------------------------------------------------
DROP INDEX demo_bank.ix_transactions_posted;

-- ---------------------------------------------------------------------------
-- 8.2 row 11: one constraint not validated. Dropped and re-added NOT VALID:
-- the definition is identical, only convalidated is false.
-- ---------------------------------------------------------------------------
ALTER TABLE demo_bank.transactions DROP CONSTRAINT ck_transactions_fee;
ALTER TABLE demo_bank.transactions
  ADD CONSTRAINT ck_transactions_fee CHECK (fee_amount >= 0) NOT VALID;

-- ---------------------------------------------------------------------------
-- 8.2 row 12: the large object missing. customer_documents row 1 still points
-- at oid 7654321, exactly the dangling reference the inventory must catch.
-- ---------------------------------------------------------------------------
DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_largeobject_metadata WHERE oid = 7654321) THEN
    RAISE EXCEPTION 'large object 7654321 is already gone; build.sh must start from a fresh copy';
  END IF;
END;
$$;
SELECT lo_unlink(7654321);

-- ===========================================================================
-- STAY-QUIET CASES: identical data that a naive tool would report.
-- ===========================================================================

-- ---------------------------------------------------------------------------
-- 8.2 quiet row 4: a table converted to a declarative partitioned table on the
-- new side (balances_daily, range on balance_date). Same name, same rows, same
-- constraint and index names; the engine must show one table, not four.
-- ---------------------------------------------------------------------------
CREATE TABLE demo_bank.balances_daily_parted (
  account_id      bigint        NOT NULL,
  balance_date    date          NOT NULL,
  opening_balance numeric(20,4) NOT NULL,
  closing_balance numeric(20,4) NOT NULL,
  debit_total     numeric(20,4) NOT NULL,
  credit_total    numeric(20,4) NOT NULL,
  txn_count       integer       NOT NULL
) PARTITION BY RANGE (balance_date);

CREATE TABLE demo_bank.balances_daily_p1 PARTITION OF demo_bank.balances_daily_parted
  FOR VALUES FROM ('2024-01-01') TO ('2024-01-02');
CREATE TABLE demo_bank.balances_daily_p2 PARTITION OF demo_bank.balances_daily_parted
  FOR VALUES FROM ('2024-01-02') TO ('2024-01-03');
CREATE TABLE demo_bank.balances_daily_p3 PARTITION OF demo_bank.balances_daily_parted
  FOR VALUES FROM ('2024-01-03') TO ('2024-01-04');

INSERT INTO demo_bank.balances_daily_parted
  (account_id, balance_date, opening_balance, closing_balance,
   debit_total, credit_total, txn_count)
SELECT account_id, balance_date, opening_balance, closing_balance,
       debit_total, credit_total, txn_count
FROM demo_bank.balances_daily;

DROP TABLE demo_bank.balances_daily;
ALTER TABLE demo_bank.balances_daily_parted RENAME TO balances_daily;

ALTER TABLE demo_bank.balances_daily
  ADD CONSTRAINT pk_balances_daily PRIMARY KEY (account_id, balance_date);
ALTER TABLE demo_bank.balances_daily
  ADD CONSTRAINT fk_balances_daily_account FOREIGN KEY (account_id)
    REFERENCES demo_bank.accounts (account_id);
ALTER TABLE demo_bank.balances_daily
  ADD CONSTRAINT ck_balances_daily_counts CHECK (txn_count >= 0);
ALTER TABLE demo_bank.balances_daily
  ADD CONSTRAINT ck_balances_daily_totals CHECK (debit_total >= 0 AND credit_total >= 0);
CREATE INDEX ix_balances_daily_date ON demo_bank.balances_daily (balance_date);

-- ---------------------------------------------------------------------------
-- 8.2 quiet row 5: the columns of one table in a different order on the new
-- side (holidays: is_half_day moved before holiday_name). Same names, types,
-- constraints and rows; only attnum order changes.
-- ---------------------------------------------------------------------------
CREATE TABLE demo_bank.holidays_reordered (
  holiday_date date        NOT NULL,
  country_code char(2)     NOT NULL,
  is_half_day  char(1)     NOT NULL DEFAULT 'N',
  holiday_name varchar(80) NOT NULL
);

INSERT INTO demo_bank.holidays_reordered
  (holiday_date, country_code, is_half_day, holiday_name)
SELECT holiday_date, country_code, is_half_day, holiday_name
FROM demo_bank.holidays;

DROP TABLE demo_bank.holidays;
ALTER TABLE demo_bank.holidays_reordered RENAME TO holidays;

ALTER TABLE demo_bank.holidays
  ADD CONSTRAINT pk_holidays PRIMARY KEY (holiday_date, country_code);
ALTER TABLE demo_bank.holidays
  ADD CONSTRAINT fk_holidays_country FOREIGN KEY (country_code)
    REFERENCES demo_bank.countries (country_code);
ALTER TABLE demo_bank.holidays
  ADD CONSTRAINT ck_holidays_half CHECK (is_half_day IN ('Y', 'N'));

-- ---------------------------------------------------------------------------
-- 8.2 quiet row 6: a char(1) column moved to varchar(1) on the new side
-- (kyc_checks.result). The definition change is amber; the values are equal
-- because a one-character char is not padded.
-- ---------------------------------------------------------------------------
ALTER TABLE demo_bank.kyc_checks ALTER COLUMN result TYPE varchar(1);

-- ---------------------------------------------------------------------------
-- 8.2 quiet rows 1, 2 and 3 (LATIN1 vs UTF-8 accents, different text sort
-- order, floats printed differently by 9.6 and 17) need no statement: the old
-- database is LATIN1 with collation C and runs 9.6, the new one is UTF-8 with
-- en_US.utf8 and runs 17. The canonical settings block and byte hashing keep
-- them quiet; check_key.sh proves the situations really are present.
-- ---------------------------------------------------------------------------

-- ===========================================================================
-- Assert the planted state. If any line here fails, the answer key is stale.
-- ===========================================================================
DO $$
DECLARE
  n          integer;
  bal        numeric(20,4);
  ts         timestamp(0);
  seq        bigint;
  validated  boolean;
BEGIN
  SELECT count(*) INTO n FROM demo_bank.transactions;
  IF n <> 19988 THEN
    RAISE EXCEPTION 'transactions: expected 19988 after planting, found %', n;
  END IF;

  SELECT count(*) INTO n FROM demo_bank.audit_log;
  IF n <> 10498 THEN
    RAISE EXCEPTION 'audit_log: expected 10498 after planting, found %', n;
  END IF;

  SELECT balance INTO bal FROM demo_bank.accounts WHERE account_id = 4999;
  IF bal <> 2086.0014 THEN
    RAISE EXCEPTION 'accounts.balance: expected 2086.0014, found %', bal;
  END IF;

  SELECT count(*) INTO n FROM demo_bank.transactions
  WHERE transaction_id = 19997 AND narration = 'Paxment 19997';
  IF n <> 1 THEN
    RAISE EXCEPTION 'transactions 19997: expected the one-character change';
  END IF;

  SELECT count(*) INTO n FROM demo_bank.transactions
  WHERE transaction_id = 7 AND narration IS NULL;
  IF n <> 1 THEN
    RAISE EXCEPTION 'transactions 7: expected narration NULL';
  END IF;

  SELECT value_timestamp INTO ts FROM demo_bank.transactions WHERE transaction_id = 20000;
  IF ts <> timestamp '2024-04-07 00:00:00' THEN
    RAISE EXCEPTION 'transactions 20000: expected the time of day dropped, found %', ts;
  END IF;

  SELECT count(*) INTO n FROM information_schema.columns
  WHERE table_schema = 'demo_bank' AND table_name = 'user_roles'
    AND column_name = 'granted_by';
  IF n <> 0 THEN
    RAISE EXCEPTION 'user_roles.granted_by: expected dropped';
  END IF;

  SELECT count(*) INTO n FROM information_schema.columns
  WHERE table_schema = 'demo_bank' AND table_name = 'loans'
    AND column_name = 'settlement_ref' AND data_type = 'character varying';
  IF n <> 1 THEN
    RAISE EXCEPTION 'loans.settlement_ref: expected added';
  END IF;

  SELECT count(*) INTO n FROM information_schema.tables
  WHERE table_schema = 'demo_bank' AND table_name IN ('limits', 'product_limits');
  IF n <> 1 THEN
    RAISE EXCEPTION 'expected exactly one of limits/product_limits, found %', n;
  END IF;
  SELECT count(*) INTO n FROM information_schema.columns
  WHERE table_schema = 'demo_bank' AND table_name = 'fees'
    AND column_name IN ('fee_name', 'fee_label');
  IF n <> 1 THEN
    RAISE EXCEPTION 'expected exactly one of fees.fee_name/fee_label, found %', n;
  END IF;

  SELECT count(*) INTO n
  FROM information_schema.columns
  WHERE table_schema = 'demo_bank' AND table_name = 'branches'
    AND column_name = 'city' AND character_maximum_length = 120;
  IF n <> 1 THEN
    RAISE EXCEPTION 'branches.city: expected varchar(120)';
  END IF;

  SELECT last_value INTO seq FROM demo_bank.seq_gl_entry;
  IF seq <> 7975 THEN
    RAISE EXCEPTION 'seq_gl_entry: expected 7975, found %', seq;
  END IF;

  SELECT count(*) INTO n FROM pg_indexes
  WHERE schemaname = 'demo_bank' AND indexname = 'ix_transactions_posted';
  IF n <> 0 THEN
    RAISE EXCEPTION 'ix_transactions_posted: expected dropped';
  END IF;

  SELECT convalidated INTO validated FROM pg_constraint
  WHERE conname = 'ck_transactions_fee'
    AND conrelid = 'demo_bank.transactions'::regclass;
  IF validated IS DISTINCT FROM false THEN
    RAISE EXCEPTION 'ck_transactions_fee: expected NOT VALID';
  END IF;

  SELECT count(*) INTO n FROM pg_largeobject_metadata WHERE oid = 7654321;
  IF n <> 0 THEN
    RAISE EXCEPTION 'large object 7654321: expected removed';
  END IF;

  SELECT count(*) INTO n FROM pg_class c
  JOIN pg_namespace s ON s.oid = c.relnamespace
  WHERE s.nspname = 'demo_bank' AND c.relname = 'balances_daily' AND c.relkind = 'p';
  IF n <> 1 THEN
    RAISE EXCEPTION 'balances_daily: expected a partitioned table';
  END IF;

  SELECT count(*) INTO n FROM information_schema.columns
  WHERE table_schema = 'demo_bank' AND table_name = 'holidays'
    AND column_name = 'is_half_day' AND ordinal_position = 3;
  IF n <> 1 THEN
    RAISE EXCEPTION 'holidays: expected is_half_day third (reordered columns)';
  END IF;

  SELECT count(*) INTO n
  FROM information_schema.columns
  WHERE table_schema = 'demo_bank' AND table_name = 'kyc_checks'
    AND column_name = 'result' AND data_type = 'character varying'
    AND character_maximum_length = 1;
  IF n <> 1 THEN
    RAISE EXCEPTION 'kyc_checks.result: expected varchar(1)';
  END IF;
END;
$$;

ANALYZE demo_bank.transactions;
ANALYZE demo_bank.audit_log;
ANALYZE demo_bank.accounts;
ANALYZE demo_bank.balances_daily;
ANALYZE demo_bank.holidays;
ANALYZE demo_bank.kyc_checks;
ANALYZE demo_bank.product_limits;
