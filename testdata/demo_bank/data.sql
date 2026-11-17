-- demo_bank "small" data set: deterministic, invented, safe for a LATIN1 server.
--
-- Rules this file follows:
--   * No random(), no now(), no current_date. Every value comes from the row
--     number g and md5(g), so two builds of the same schema produce byte-identical
--     data (verify.sh proves it). build.sh drops and recreates the database, so the
--     sequences and the large-object OID start from the same place every run.
--   * Accented characters are written as chr(<code>) with codes 160-255 only, so
--     the same file loads unchanged into the old LATIN1 database and the new UTF-8
--     one. The files themselves stay pure ASCII.
--   * Money amounts are numeric(20,4); the single money column is loans.penalty_charge.
--   * Pseudo-random values: (('x' || substr(md5(<seed> || g), 1, 7))::bit(28)::int)
--     gives 28 bits, always non-negative, identical on every Postgres version.
--
-- Loaded by build.sh, straight after schema.sql. Sizes: thousands of rows per
-- table, 20,000 transactions.

-- ---------------------------------------------------------------------------
-- The one large object (PLAN.md 8.1 wants one). It is created with a FIXED oid
-- so customer_documents.content_oid is the same on every run; a generated oid
-- comes from the cluster-wide counter and would change from build to build.
-- pg_dump/pg_restore keep the oid, so both sides match.
-- ---------------------------------------------------------------------------
DO $$
BEGIN
  PERFORM lo_from_bytea(7654321,
                        decode(md5('blob-one') || md5('blob-two') || md5('blob-three'), 'hex'));
END;
$$;

-- ---------------------------------------------------------------------------
-- Reference data
-- ---------------------------------------------------------------------------
INSERT INTO demo_bank.countries (country_code, country_name, dial_code, region_code, is_active)
SELECT chr(65 + ((g - 1) / 26)) || chr(65 + ((g - 1) % 26)),
       'Country ' || g,
       '+' || (100 + g)::text,
       'R' || lpad((1 + (g % 4))::text, 2, '0'),
       CASE WHEN g % 11 = 0 THEN 'N' ELSE 'Y' END
FROM generate_series(1, 30) g;

INSERT INTO demo_bank.currencies (currency_code, currency_name, minor_units, symbol_text, is_active)
SELECT 'X' || lpad(g::text, 2, '0'),
       'Unit ' || g,
       (g % 3)::smallint,
       chr(192 + (g % 60)),
       CASE WHEN g = 12 THEN 'N' ELSE 'Y' END
FROM generate_series(1, 12) g;

INSERT INTO demo_bank.branches (branch_code, branch_name, country_code, city,
                                opened_on, closed_on, is_head_office, is_active,
                                created_at, updated_at)
SELECT 'B' || lpad(g::text, 5, '0'),
       'Branch ' || g || CASE WHEN g % 5 = 0 THEN ' ' || chr(233) || 'st' ELSE '' END,
       chr(65 + ((g % 30) / 26)) || chr(65 + ((g % 30) % 26)),
       CASE WHEN g % 9 = 0 THEN 'Ciudad ' || chr(225) || 'vila' ELSE 'City ' || (g % 40) END,
       date '1995-01-01' + (g * 37),
       CASE WHEN g % 17 = 0 THEN date '2021-06-30' ELSE NULL END,
       (g = 1),
       CASE WHEN g % 17 = 0 THEN 'N' ELSE 'Y' END,
       timestamp '1995-01-01 09:00:00' + (g * interval '1 hour'),
       timestamptz '1995-01-01 09:00:00+00' + (g * interval '1 hour')
FROM generate_series(1, 60) g;

INSERT INTO demo_bank.branch_hours (branch_id, weekday, open_time, close_time, open_duration)
SELECT b.branch_id,
       d.w,
       time '09:00' + (b.branch_id % 2) * interval '1 hour',
       time '17:00' + (b.branch_id % 3) * interval '30 minutes',
       ((8 + (b.branch_id % 3)) || ' hours')::interval
FROM demo_bank.branches b
CROSS JOIN generate_series(1, 5) AS d(w)
ORDER BY b.branch_id, d.w;

INSERT INTO demo_bank.holidays (holiday_date, country_code, holiday_name, is_half_day)
SELECT date '2024-01-01' + (g * 3),
       chr(65 + (((g - 1) % 30) / 26)) || chr(65 + (((g - 1) % 30) % 26)),
       'Holiday ' || g || CASE WHEN g % 8 = 0 THEN ' (' || chr(241) || ')' ELSE '' END,
       CASE WHEN g % 9 = 0 THEN 'Y' ELSE 'N' END
FROM generate_series(1, 100) g;

INSERT INTO demo_bank.products (product_id, product_code, product_name, product_type,
                                currency_code, nominal_rate, is_active)
SELECT g,
       'PRD' || lpad(g::text, 4, '0'),
       'Product ' || g || ' '
         || (ARRAY['Classic', 'Plus', 'Prime', 'Basic'])[1 + (g % 4)],
       (ARRAY['ACCOUNT', 'LOAN', 'CARD', 'PAYMENT'])[1 + (g % 4)],
       'X' || lpad((1 + (g % 12))::text, 2, '0'),
       ((('x' || substr(md5('prod' || g::text), 1, 7))::bit(28)::int % 2000)::float8
          / 10000.0)::real,
       CASE WHEN g % 13 = 0 THEN 'N' ELSE 'Y' END
FROM generate_series(1, 40) g;

INSERT INTO demo_bank.account_types (account_type_id, type_code, type_name,
                                     allows_overdraft, min_balance)
SELECT g,
       'AT' || lpad(g::text, 2, '0'),
       (ARRAY['Everyday', 'Savings', 'Premium', 'Business', 'Student',
              'Pension', 'Joint', 'Trust', 'Escrow', 'Settlement'])[g],
       CASE WHEN g % 3 = 0 THEN 'Y' ELSE 'N' END,
       ((g % 5) * 1000)::numeric(20,4)
FROM generate_series(1, 10) g;

-- ---------------------------------------------------------------------------
-- Customers
-- ---------------------------------------------------------------------------
INSERT INTO demo_bank.customers (customer_id, customer_no, first_name, last_name,
                                 date_of_birth, national_id, country_code, branch_id,
                                 risk_grade, is_active, created_at, updated_at)
SELECT g,
       'CUS' || lpad(g::text, 9, '0'),
       (ARRAY['Ana', 'Bo', 'Chen', 'Dara', 'Emil',
              'Fatima', 'Gita', 'Hugo', 'In' || chr(232) || 's', 'Jonas'])[1 + (g % 10)],
       (ARRAY['Okafor', 'M' || chr(252) || 'ller', 'Garc' || chr(237) || 'a',
              'Nguy' || chr(234) || 'n', 'Kowalski', 'Silva',
              'Ahm' || chr(233) || 'd', 'Novak', 'Petrov', 'Jans' || chr(246) || 'n'])[1 + ((g / 10) % 10)],
       date '1950-01-01' + (('x' || substr(md5('dob' || g::text), 1, 7))::bit(28)::int % 25000),
       'NID' || lpad(g::text, 9, '0'),
       chr(65 + ((g % 30) / 26)) || chr(65 + ((g % 30) % 26)),
       1 + (g % 60),
       (ARRAY['A', 'B', 'C', 'D'])[1 + (g % 4)],
       CASE WHEN g % 13 = 0 THEN 'N' ELSE 'Y' END,
       timestamp '2010-01-01 08:00:00' + (g * interval '3 hours'),
       timestamptz '2010-01-01 08:00:00+00' + (g * interval '3 hours')
FROM generate_series(1, 2000) g;

INSERT INTO demo_bank.customer_addresses (address_id, customer_id, address_type,
                                          line1, line2, city, post_code, country_code,
                                          is_primary)
SELECT g,
       CASE WHEN g <= 2000 THEN g ELSE g - 2000 END,
       CASE WHEN g <= 2000 THEN 'HOME' ELSE 'WORK' END,
       (g % 50) || ' Main Street'
         || CASE WHEN g % 6 = 0 THEN ', ' || chr(193) || 'vila' ELSE '' END,
       CASE WHEN g % 3 = 0 THEN '' WHEN g % 7 = 0 THEN NULL ELSE 'Unit ' || (g % 50) END,
       'Town ' || (g % 80),
       CASE WHEN g % 19 = 0 THEN NULL ELSE lpad(g::text, 6, '0') END,
       chr(65 + ((g % 30) / 26)) || chr(65 + ((g % 30) % 26)),
       CASE WHEN g % 5 = 0 THEN 'Y' ELSE 'N' END
FROM generate_series(1, 2500) g;

-- One document points at the large object above; the rest carry an inline blob
-- or nothing at all (the CHECK allows only one source at a time).
INSERT INTO demo_bank.customer_documents (document_id, customer_id, doc_type,
                                          doc_blob, content_oid, uploaded_at, verified)
SELECT g,
       1 + (g % 2000),
       (ARRAY['ID', 'PASSPORT', 'UTILITY', 'CONTRACT'])[1 + (g % 4)],
       CASE WHEN g = 1 THEN NULL
            WHEN g % 5 = 0 THEN NULL
            ELSE decode(md5('doc' || g::text) || md5('docb' || g::text), 'hex') END,
       CASE WHEN g = 1 THEN 7654321::oid ELSE NULL END,
       timestamptz '2015-01-01 00:00:00+00' + (g * interval '9 hours'),
       CASE WHEN g % 4 = 0 THEN 'Y' ELSE 'N' END
FROM generate_series(1, 2000) g;

INSERT INTO demo_bank.user_roles (user_id, role_name, granted_on, permissions, granted_by)
SELECT 1 + ((g - 1) / 4),
       (ARRAY['TELLER', 'SUPERVISOR', 'AUDITOR', 'ADMIN'])[1 + ((g - 1) % 4)],
       date '2020-01-01' + (g % 900),
       CASE g % 5
         WHEN 0 THEN ARRAY[]::text[]
         WHEN 1 THEN ARRAY['VIEW']::text[]
         WHEN 2 THEN ARRAY['VIEW', 'TXN']::text[]
         WHEN 3 THEN ARRAY['VIEW', 'TXN', 'EDIT']::text[]
         ELSE ARRAY['VIEW', NULL]::text[]
       END,
       CASE WHEN g % 6 = 0 THEN NULL WHEN g % 7 = 0 THEN '' ELSE 'admin' || (g % 9) END
FROM generate_series(1, 500) g;

-- ---------------------------------------------------------------------------
-- Accounts and their history
-- ---------------------------------------------------------------------------
INSERT INTO demo_bank.accounts (account_id, account_no, customer_id, branch_id,
                                account_type_id, currency_code, balance,
                                available_balance, overdraft_limit, interest_rate,
                                opened_on, closed_on, status, statement_cycle,
                                created_at, updated_at)
SELECT g,
       'AC' || lpad(g::text, 14, '0'),
       1 + (g % 2000),
       1 + (g % 60),
       1 + (g % 10),
       'X' || lpad((1 + (g % 12))::text, 2, '0'),
       bal.amount,
       bal.amount - (g % 500)::numeric(20,4),
       CASE WHEN g % 3 = 0 THEN 0 ELSE (g % 5) * 1000 END::numeric(20,4),
       (g % 2000)::float8 / 10000.0,
       date '2005-01-01' + (g % 6000),
       CASE WHEN g % 23 = 0 THEN date '2023-01-01' + (g % 500) ELSE NULL END,
       CASE WHEN g % 23 = 0 THEN 'C'
            WHEN g % 17 = 0 THEN 'D'
            WHEN g % 97 = 0 THEN 'F'
            ELSE 'A' END,
       lpad((1 + (g % 28))::text, 2, '0'),
       timestamp '2005-01-01 08:00:00' + (g * interval '2 hours'),
       timestamptz '2005-01-01 08:00:00+00' + (g * interval '2 hours')
FROM generate_series(1, 5000) g
CROSS JOIN LATERAL (
  SELECT ((('x' || substr(md5('bal' || g::text), 1, 7))::bit(28)::int % 100000000)::numeric
            / 10000)::numeric(20,4) AS amount
) AS bal;

INSERT INTO demo_bank.account_holders (account_id, customer_id, holder_role,
                                       share_pct, added_on)
SELECT a.account_id, a.customer_id, 'PRIMARY', 100.00, a.opened_on
FROM demo_bank.accounts a
UNION ALL
SELECT a.account_id, 1 + ((a.account_id * 7) % 2000), 'JOINT', 50.00, a.opened_on
FROM demo_bank.accounts a
WHERE a.account_id % 10 = 0;

INSERT INTO demo_bank.balances_daily (account_id, balance_date, opening_balance,
                                      closing_balance, debit_total, credit_total,
                                      txn_count)
SELECT 1 + ((g - 1) / 3),
       date '2024-01-01' + ((g - 1) % 3),
       ((g % 100000)::numeric / 100)::numeric(20,4),
       (((g + 17) % 100000)::numeric / 100)::numeric(20,4),
       ((g % 5000)::numeric / 10)::numeric(20,4),
       ((g % 7000)::numeric / 10)::numeric(20,4),
       g % 40
FROM generate_series(1, 15000) g;

INSERT INTO demo_bank.transaction_types (transaction_type_id, type_code,
                                         type_description, dr_cr, affects_balance)
SELECT g,
       'TT' || lpad(g::text, 2, '0'),
       (ARRAY['Deposit', 'Withdrawal', 'Transfer out', 'Transfer in', 'Fee',
              'Interest credit', 'Card payment', 'Refund', 'Chargeback', 'Standing order',
              'Salary', 'Utility bill', 'Cash deposit', 'Cash withdrawal', 'Cheque',
              'Loan drawdown', 'Loan repayment', 'Tax', 'Reversal', 'Adjustment'])[g],
       CASE WHEN g % 2 = 0 THEN 'C' ELSE 'D' END,
       CASE WHEN g % 7 = 0 THEN 'N' ELSE 'Y' END
FROM generate_series(1, 20) g;

-- The large history table: 20,000 rows. The currency comes from the account so
-- the composite foreign key always holds.
INSERT INTO demo_bank.transactions (transaction_id, account_id, currency_code,
                                    transaction_type_id, txn_date, posted_at,
                                    value_timestamp, amount, fee_amount,
                                    running_balance, narration, reference, channel,
                                    status, is_reversal, created_at)
SELECT t.g,
       a.account_id,
       a.currency_code,
       1 + (t.g % 20),
       date '2024-01-01' + (t.g % 365),
       timestamptz '2024-01-01 00:00:00+00' + (t.g * interval '7 minutes'),
       timestamp '2024-01-01 00:00:00' + (t.g * interval '7 minutes'),
       CASE WHEN t.g % 9 = 0 THEN -t.amount ELSE t.amount END,
       CASE WHEN t.g % 4 = 0 THEN ((t.g % 500)::numeric / 100)::numeric(20,4)
            ELSE 0::numeric(20,4) END,
       a.balance + CASE WHEN t.g % 9 = 0 THEN -t.amount ELSE t.amount END,
       CASE WHEN t.g % 5 = 0 THEN NULL
            WHEN t.g % 7 = 0 THEN ''
            WHEN t.g % 11 = 0 THEN 'Narration ' || t.g || '   '
            WHEN t.g % 3 = 0 THEN 'Paie ' || chr(233) || 'lectrique ' || t.g
            ELSE 'Payment ' || t.g END,
       'REF' || lpad(t.g::text, 12, '0'),
       (ARRAY['B', 'A', 'M', 'W', 'C'])[1 + (t.g % 5)],
       (ARRAY['P', 'P', 'P', 'H', 'R', 'X'])[1 + (t.g % 6)],
       CASE WHEN t.g % 31 = 0 THEN 'Y' ELSE 'N' END,
       timestamp '2024-01-01 00:00:00' + (t.g * interval '7 minutes')
FROM generate_series(1, 20000) g
CROSS JOIN LATERAL (
  SELECT g,
         CASE WHEN raw.v = 0 THEN 1::numeric ELSE raw.v END AS amount
  FROM (
    SELECT ((('x' || substr(md5('txn' || g::text), 1, 7))::bit(28)::int % 50000000)::numeric
              / 10000)::numeric(20,4) AS v
  ) raw
) AS t
JOIN demo_bank.accounts a ON a.account_id = 1 + (t.g % 5000);

-- ---------------------------------------------------------------------------
-- Payments out
-- ---------------------------------------------------------------------------
INSERT INTO demo_bank.beneficiaries (beneficiary_id, customer_id, beneficiary_name,
                                     account_no, bank_code, currency_code, added_on,
                                     is_active)
SELECT g,
       1 + (g % 2000),
       (ARRAY['Electric Co', 'Water Board', 'Landlord', 'School',
              'S' || chr(227) || 'o Jo' || chr(227) || 'o', 'Mobile Top-up'])[1 + (g % 6)],
       'EXT' || lpad(g::text, 12, '0'),
       'BK' || lpad((g % 50)::text, 6, '0'),
       'X' || lpad((1 + (g % 12))::text, 2, '0'),
       date '2018-01-01' + (g % 2000),
       CASE WHEN g % 15 = 0 THEN 'N' ELSE 'Y' END
FROM generate_series(1, 1500) g;

INSERT INTO demo_bank.standing_orders (standing_order_id, account_id, beneficiary_id,
                                       amount, frequency, next_run_date, end_date,
                                       is_active)
SELECT g,
       1 + (g % 5000),
       1 + (g % 1500),
       ((g % 20000)::numeric / 100 + 1)::numeric(20,4),
       (ARRAY['DAILY', 'WEEKLY', 'MONTHLY'])[1 + (g % 3)],
       date '2025-01-01' + (g % 300),
       CASE WHEN g % 6 = 0 THEN date '2026-12-31' ELSE NULL END,
       CASE WHEN g % 12 = 0 THEN 'N' ELSE 'Y' END
FROM generate_series(1, 800) g;

INSERT INTO demo_bank.payment_batches (batch_id, batch_ref, created_by, created_at,
                                       item_count, total_amount, status)
SELECT g,
       'BATCH' || lpad(g::text, 6, '0'),
       'operator' || (g % 7),
       timestamptz '2024-06-01 00:00:00+00' + (g * interval '3 hours'),
       10,
       ((g % 100000)::numeric / 10)::numeric(20,4),
       (ARRAY['N', 'S', 'C', 'C', 'F'])[1 + (g % 5)]
FROM generate_series(1, 300) g;

INSERT INTO demo_bank.payment_items (batch_id, item_no, beneficiary_id, amount,
                                     currency_code, status)
SELECT 1 + ((g - 1) / 10),
       1 + ((g - 1) % 10),
       1 + (g % 1500),
       ((g % 20000)::numeric / 100 + 1)::numeric(20,4),
       'X' || lpad((1 + (g % 12))::text, 2, '0'),
       (ARRAY['N', 'S', 'C', 'C', 'F'])[1 + (g % 5)]
FROM generate_series(1, 3000) g;

-- ---------------------------------------------------------------------------
-- Lending
-- ---------------------------------------------------------------------------
INSERT INTO demo_bank.loans (loan_id, loan_no, customer_id, branch_id, product_id,
                             principal, outstanding, interest_rate, disbursed_on,
                             maturity_date, status, penalty_charge, created_at)
SELECT g,
       'LN' || lpad(g::text, 14, '0'),
       1 + (g % 2000),
       1 + (g % 60),
       1 + (g % 40),
       principal.amount,
       (principal.amount * ((g % 100)::numeric / 100))::numeric(20,4),
       (g % 2000)::float8 / 10000.0 + 0.01,
       date '2019-01-01' + (g % 1800),
       date '2019-01-01' + (g % 1800) + 1800,
       (ARRAY['A', 'A', 'A', 'C', 'W', 'D'])[1 + (g % 6)],
       CASE WHEN g % 7 = 0 THEN 250.00::money
            WHEN g % 11 = 0 THEN 1250.50::money
            ELSE 0::money END,
       timestamp '2019-01-01 10:00:00' + (g * interval '5 hours')
FROM generate_series(1, 1200) g
CROSS JOIN LATERAL (
  SELECT ((('x' || substr(md5('loan' || g::text), 1, 7))::bit(28)::int % 100000000)::numeric
            / 100)::numeric(20,4) AS amount
) AS principal;

INSERT INTO demo_bank.loan_schedules (loan_id, installment_no, due_date,
                                      principal_due, interest_due, paid_amount)
SELECT 1 + ((g - 1) / 6),
       1 + ((g - 1) % 6),
       (l.disbursed_on + ((1 + ((g - 1) % 6)) || ' months')::interval)::date,
       ((g % 5000)::numeric / 10)::numeric(20,4),
       ((g % 700)::numeric / 10)::numeric(20,4),
       CASE WHEN g % 4 = 0 THEN ((g % 5000)::numeric / 10)::numeric(20,4)
            ELSE 0::numeric(20,4) END
FROM generate_series(1, 7200) g
JOIN demo_bank.loans l ON l.loan_id = 1 + ((g - 1) / 6);

INSERT INTO demo_bank.loan_collateral (collateral_id, loan_id, collateral_type,
                                       description, market_value, valued_on,
                                       currency_code)
SELECT g,
       1 + (g % 1200),
       (ARRAY['PROPERTY', 'VEHICLE', 'DEPOSIT', 'GUARANTEE'])[1 + (g % 4)],
       CASE WHEN g % 8 = 0 THEN NULL
            WHEN g % 5 = 0 THEN ''
            ELSE 'Asset ' || g || ' in ' || chr(193) || 'vila' END,
       ((g % 500000)::numeric / 10 + 1)::numeric(20,4),
       date '2020-01-01' + (g % 1500),
       'X' || lpad((1 + (g % 12))::text, 2, '0')
FROM generate_series(1, 900) g;

-- ---------------------------------------------------------------------------
-- Cards
-- ---------------------------------------------------------------------------
INSERT INTO demo_bank.cards (card_id, card_no, account_id, card_type, expiry_date,
                             credit_limit, status, issued_at, is_contactless)
SELECT g,
       lpad(g::text, 16, '0'),
       1 + (g % 5000),
       (ARRAY['DEBIT', 'CREDIT', 'PREPAID'])[1 + (g % 3)],
       date '2027-01-01' + (g % 700),
       CASE WHEN g % 3 = 0 THEN ((g % 50) * 100)::numeric(20,4) ELSE 0::numeric(20,4) END,
       (ARRAY['A', 'A', 'A', 'B', 'E', 'X'])[1 + (g % 6)],
       timestamp '2016-01-01 12:00:00' + (g * interval '4 hours'),
       CASE WHEN g % 4 = 0 THEN 'N' ELSE 'Y' END
FROM generate_series(1, 2500) g;

INSERT INTO demo_bank.card_transactions (card_txn_id, card_id, posted_at, merchant,
                                         amount, currency_code, is_declined, auth_code)
SELECT g,
       1 + (g % 2500),
       timestamptz '2024-03-01 00:00:00+00' + (g * interval '13 minutes'),
       (ARRAY['Grocer', 'Fuel Stop', 'Caf' || chr(233) || ' Central', 'Book Shop',
              'Rail Co', 'Pharmacy'])[1 + (g % 6)],
       CASE WHEN raw.v = 0 THEN 5::numeric ELSE raw.v END,
       'X' || lpad((1 + (g % 12))::text, 2, '0'),
       CASE WHEN g % 9 = 0 THEN 'Y' ELSE 'N' END,
       CASE WHEN g % 9 = 0 THEN NULL ELSE 'A' || lpad((g % 100000)::text, 6, '0') END
FROM generate_series(1, 6000) g
CROSS JOIN LATERAL (
  SELECT ((('x' || substr(md5('ct' || g::text), 1, 7))::bit(28)::int % 5000000)::numeric
            / 100)::numeric(20,4) AS v
) AS raw;

-- ---------------------------------------------------------------------------
-- General ledger, rates and fees
-- ---------------------------------------------------------------------------
-- Parents first: the self-referencing foreign key is checked at the end of the
-- statement, but loading the roots first keeps the intent obvious.
INSERT INTO demo_bank.gl_accounts (gl_account_id, gl_code, gl_name, account_class,
                                   parent_id, is_active)
SELECT g, 'GL' || lpad(g::text, 6, '0'), 'Ledger root ' || g,
       (ARRAY['A', 'L', 'E', 'R', 'X'])[1 + (g % 5)], NULL, 'Y'
FROM generate_series(1, 5) g;

INSERT INTO demo_bank.gl_accounts (gl_account_id, gl_code, gl_name, account_class,
                                   parent_id, is_active)
SELECT g, 'GL' || lpad(g::text, 6, '0'), 'Ledger ' || g,
       (ARRAY['A', 'L', 'E', 'R', 'X'])[1 + (g % 5)],
       1 + (g % 5),
       CASE WHEN g % 17 = 0 THEN 'N' ELSE 'Y' END
FROM generate_series(6, 60) g;

INSERT INTO demo_bank.gl_entries (entry_date, gl_account_id, branch_id, debit,
                                  credit, currency_code, reference, created_at)
SELECT date '2024-01-01' + (e.g % 365),
       1 + (e.g % 60),
       1 + (e.g % 60),
       CASE WHEN e.g % 2 = 0 THEN e.amount ELSE 0::numeric(20,4) END,
       CASE WHEN e.g % 2 = 1 THEN e.amount ELSE 0::numeric(20,4) END,
       'X' || lpad((1 + (e.g % 12))::text, 2, '0'),
       'GL' || lpad(e.g::text, 10, '0'),
       timestamptz '2024-01-01 00:00:00+00' + (e.g * interval '11 minutes')
FROM (
  SELECT g, (((g % 99999) + 1)::numeric / 100)::numeric(20,4) AS amount
  FROM generate_series(1, 8000) g
  ORDER BY g
) AS e;

INSERT INTO demo_bank.fx_rates (from_currency, to_currency, rate_date, rate, spread,
                                source)
SELECT 'X' || lpad(f::text, 2, '0'),
       'X' || lpad(t::text, 2, '0'),
       date '2024-01-01' + d,
       (1 + ((('x' || substr(md5('fx' || f || '-' || t || '-' || d), 1, 7))::bit(28)::int
               % 200000)::numeric / 100000))::numeric(18,8),
       ((('x' || substr(md5('sp' || f || '-' || t), 1, 7))::bit(28)::int % 100)::float8
          / 10000.0)::real,
       'BOOK'
FROM generate_series(1, 12) f
CROSS JOIN generate_series(1, 12) t
CROSS JOIN generate_series(0, 19) d
WHERE f <> t;

INSERT INTO demo_bank.interest_rates (rate_id, product_id, currency_code, band_from,
                                      band_to, annual_rate, effective_from,
                                      effective_to, tier_code)
SELECT g,
       1 + (g % 40),
       'X' || lpad((1 + (g % 12))::text, 2, '0'),
       ((g % 10) * 1000)::numeric(20,4),
       (((g % 10) + 1) * 1000)::numeric(20,4),
       (g % 1500)::float8 / 10000.0,
       date '2023-01-01' + (g % 700),
       CASE WHEN g % 5 = 0 THEN date '2025-12-31' ELSE NULL END,
       'T' || (1 + (g % 4))::text
FROM generate_series(1, 400) g;

INSERT INTO demo_bank.fees (fee_id, fee_code, fee_name, fee_type, flat_amount, pct_rate)
SELECT g,
       'FEE' || lpad(g::text, 3, '0'),
       (ARRAY['Monthly maintenance', 'Cash handling', 'Overdraft', 'Late payment',
              'Card replacement'])[1 + (g % 5)] || ' ' || g,
       (ARRAY['FLAT', 'PERCENT', 'TIERED'])[1 + (g % 3)],
       CASE WHEN g % 3 = 0 THEN ((g % 50)::numeric)::numeric(20,4) ELSE 0::numeric(20,4) END,
       CASE WHEN g % 3 = 1 THEN (g % 300)::float8 / 10000.0 ELSE 0::float8 END
FROM generate_series(1, 25) g;

INSERT INTO demo_bank.charges (charge_id, account_id, fee_id, charged_on, amount,
                               waive_flag, narration)
SELECT g,
       1 + (g % 5000),
       1 + (g % 25),
       date '2024-01-01' + (g % 365),
       ((g % 500)::numeric / 100)::numeric(20,4),
       CASE WHEN g % 8 = 0 THEN 'Y' ELSE 'N' END,
       CASE WHEN g % 6 = 0 THEN NULL
            WHEN g % 9 = 0 THEN ''
            WHEN g % 10 = 0 THEN 'Fee charged   '
            ELSE 'Fee ' || g || ' (' || chr(233) || ')' END
FROM generate_series(1, 3000) g;

INSERT INTO demo_bank.exchange_audit (exchange_id, from_currency, to_currency, rate,
                                      converted_amount, requested_at, status)
SELECT g,
       'X' || lpad((1 + (g % 12))::text, 2, '0'),
       'X' || lpad((1 + ((g + 5) % 12))::text, 2, '0'),
       (1 + ((g % 500000)::numeric / 100000))::numeric(18,8),
       ((g % 10000000)::numeric / 100)::numeric(20,4),
       timestamptz '2024-02-01 00:00:00+00' + (g * interval '17 minutes'),
       (ARRAY['C', 'C', 'C', 'F', 'R'])[1 + (g % 5)]
FROM generate_series(1, 3000) g;

-- ---------------------------------------------------------------------------
-- Statements, compliance, notes, limits
-- ---------------------------------------------------------------------------
INSERT INTO demo_bank.statements (statement_id, account_id, period_start, period_end,
                                  opening_balance, closing_balance, txn_count,
                                  generated_at)
SELECT g,
       g,
       (date '2024-01-01' + ((g % 12) || ' months')::interval)::date,
       (date '2024-01-01' + ((g % 12) || ' months')::interval + interval '1 month'
          - interval '1 day')::date,
       ((g % 100000)::numeric / 100)::numeric(20,4),
       (((g + 33) % 100000)::numeric / 100)::numeric(20,4),
       g % 60,
       timestamp '2024-12-31 23:00:00' + (g * interval '1 minute')
FROM generate_series(1, 5000) g;

INSERT INTO demo_bank.kyc_checks (check_id, customer_id, check_type, checked_at,
                                  result, detail)
SELECT g,
       1 + (g % 2000),
       (ARRAY['IDENTITY', 'ADDRESS', 'SANCTIONS', 'PEP'])[1 + (g % 4)],
       timestamptz '2022-01-01 00:00:00+00' + (g * interval '19 minutes'),
       (ARRAY['P', 'P', 'P', 'F', 'R'])[1 + (g % 5)],
       ('{"score":' || (g % 100) || ',"flags":["A","B"],"note":null,"version":2}')::jsonb
FROM generate_series(1, 2500) g;

INSERT INTO demo_bank.alerts (alert_id, customer_id, raised_at, rule_code, severity,
                              payload)
SELECT g,
       1 + (g % 2000),
       timestamptz '2024-04-01 00:00:00+00' + (g * interval '23 minutes'),
       'R' || lpad((1 + (g % 20))::text, 3, '0'),
       (1 + (g % 5))::smallint,
       ('{"rule":"R' || (g % 20) || '","amount":' || (g % 1000)
          || ',"channel":"WEB","tags":["kyc","review"]}')::json
FROM generate_series(1, 1500) g;

INSERT INTO demo_bank.rate_bands (band_id, product_id, amount_band, rate_pct,
                                  effective_from)
SELECT g,
       1 + (g % 40),
       numrange(((g % 10) * 1000)::numeric, (((g % 10) + 1) * 1000)::numeric, '[)'),
       (g % 900)::float8 / 10000.0,
       date '2023-06-01' + (g % 400)
FROM generate_series(1, 120) g;

-- No key, duplicates on purpose: the same (event_at, actor, action, object_name)
-- combination repeats, and every fifth row has a NULL detail.
INSERT INTO demo_bank.audit_log (event_at, actor, action, object_name, detail)
SELECT timestamptz '2024-01-01 00:00:00+00' + ((g % 400) * interval '1 hour'),
       (ARRAY['teller1', 'teller2', 'supervisor', 'batch', 'trigger'])[1 + (g % 5)],
       (ARRAY['INSERT', 'UPDATE', 'DELETE', 'LOGIN', 'LOGOUT'])[1 + (g % 5)],
       (ARRAY['accounts', 'customers', 'transactions', 'loans', 'cards'])[1 + ((g / 5) % 5)],
       CASE WHEN g % 5 = 0 THEN NULL
            WHEN g % 7 = 0 THEN ''
            ELSE 'detail ' || g END
FROM generate_series(1, 10000) g;

INSERT INTO demo_bank.cheques (cheque_id, account_id, cheque_no, issued_on, amount,
                               status, payee, cleared_on)
SELECT g,
       1 + (g % 5000),
       lpad(g::text, 10, '0'),
       date '2024-01-01' + (g % 500),
       ((g % 200000)::numeric / 100 + 1)::numeric(20,4),
       (ARRAY['I', 'I', 'C', 'S', 'B'])[1 + (g % 5)],
       CASE WHEN g % 6 = 0 THEN NULL
            WHEN g % 7 = 0 THEN ''
            ELSE 'Payee ' || g || ' ' || chr(199) || 'o' END,
       CASE WHEN g % 5 = 3 THEN date '2024-01-05' + (g % 500) ELSE NULL END
FROM generate_series(1, 2000) g;

INSERT INTO demo_bank.disputes (card_txn_id, customer_id, opened_at, reason, amount,
                                status, resolved_on)
SELECT g,
       1 + (g % 2000),
       timestamptz '2024-05-01 00:00:00+00' + (g * interval '29 minutes'),
       (ARRAY['FRAUD', 'DUPLICATE', 'NOT_RECEIVED', 'WRONG_AMOUNT'])[1 + (g % 4)],
       ((g % 100000)::numeric / 100 + 1)::numeric(20,4),
       (ARRAY['O', 'O', 'R', 'U', 'C'])[1 + (g % 5)],
       CASE WHEN g % 5 >= 2 THEN date '2024-07-01' + (g % 200) ELSE NULL END
FROM generate_series(1, 400) g
ORDER BY g;

INSERT INTO demo_bank.notes (customer_id, account_id, note_text, author, created_at)
SELECT CASE WHEN n.g % 4 = 0 THEN NULL ELSE 1 + (n.g % 2000) END,
       CASE WHEN n.g % 4 = 1 THEN NULL ELSE 1 + (n.g % 5000) END,
       CASE WHEN n.g % 29 = 0 THEN ''
            WHEN n.g % 5 = 0 THEN 'Customer asked about ' || chr(233) || 'pargne   '
            WHEN n.g % 7 = 0 THEN 'Nota en espa' || chr(241) || 'ol: ' || chr(243) || 'rdenes'
            ELSE 'Note ' || n.g || ' about account ' || (1 + (n.g % 5000)) END,
       (ARRAY['teller1', 'supervisor', 'auditor', 'batch'])[1 + (n.g % 4)],
       timestamp '2023-01-01 09:00:00' + (n.g * interval '3 hours')
FROM (
  SELECT g FROM generate_series(1, 2000) g ORDER BY g
) AS n;

INSERT INTO demo_bank.limits (limit_id, product_id, limit_code, max_amount, max_count,
                              currency_code, effective_from)
SELECT g,
       1 + (g % 40),
       'L' || lpad(g::text, 4, '0'),
       ((g % 500000)::numeric / 10 + 100)::numeric(20,4),
       1 + (g % 50),
       'X' || lpad((1 + (g % 12))::text, 2, '0'),
       date '2023-01-01' + (g % 500)
FROM generate_series(1, 120) g;

-- ---------------------------------------------------------------------------
-- Exercise the trigger: 500 accounts get a small credit and a later updated_at.
-- The trigger writes one audit_log row per changed account, using the row's own
-- updated_at as the event time, so the extra rows are reproducible.
-- ---------------------------------------------------------------------------
UPDATE demo_bank.accounts
SET balance = balance + (account_id % 100)::numeric / 100,
    available_balance = available_balance + (account_id % 100)::numeric / 100,
    updated_at = updated_at + interval '30 days'
WHERE account_id <= 500;

-- The materialised view was created before the accounts existed.
REFRESH MATERIALIZED VIEW demo_bank.mv_branch_totals;
