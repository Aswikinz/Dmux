-- demo_bank data set: deterministic, invented, safe for a LATIN1 server.
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
-- ---------------------------------------------------------------------------
-- SIZES (PLAN.md 8.1). Two integer knobs, both 1 for the small size, so the
-- default output is exactly the historic small data set:
--
--   size    sf_acct  sf_txn   accounts     transactions
--   small        1       1        5,000           20,000
--   large      200    1000    1,000,000       20,000,000
--
-- Every table either grows with one of those two knobs (accounts, customers,
-- transactions, ledger, history) or is fixed reference data (lookup lists, the
-- chart of accounts, rate and fee tables) that a bank does not scale with its
-- customer count. build.sh passes -v size=large, or -v scale=N for a custom
-- factor (N applies to both knobs), or nothing at all for the small size.
--
-- ---------------------------------------------------------------------------
-- LOADING IN PARTS. Each table (or small group of tables that share a range)
-- is one "part". With -v run=<part> only that part loads, and -v lo= / -v hi=
-- restrict the row window, so build.sh can load the large size in batches of at
-- most 1,000,000 rows, one INSERT per transaction, which keeps memory and
-- rollback cost bounded. With no -v run every part loads once in file order,
-- which is the small size in one psql run (the historic behaviour).
--
-- -v list_parts=1 prints "ord|part|rows|batched" and quits: build.sh reads the
-- plan from here, so the row counts, the dependency order and which parts can be
-- split into windows live in ONE place.
--
-- psql 10 or newer (the \if meta-command) and \gset are required.

-- Size knobs.
\if :{?size}
\else
\set size small
\endif
\if :{?scale}
\else
\set scale 0
\endif
-- psql's \if takes a boolean value, not an expression, so each test is computed
-- with \gset first; "t"/"f" are valid boolean values for \if.
SELECT (:'size' = 'large') AS is_large,
       (:scale > 0)        AS has_scale
\gset
\if :is_large
\set sf_acct 200
\set sf_txn 1000
\else
\set sf_acct 1
\set sf_txn 1
\endif
-- An explicit -v scale=N overrides the size's factors (build.sh --scale N).
\if :has_scale
\set sf_acct :scale
\set sf_txn :scale
\endif

-- Row count of every generated table for this size. Fixed reference tables are
-- literals; the rest are the small-size count multiplied by its knob.
SELECT (5000 * :sf_acct)::bigint  AS n_accounts,
       (20000 * :sf_txn)::bigint  AS n_transactions,
       (2000 * :sf_acct)::bigint  AS n_customers,
       (2500 * :sf_acct)::bigint  AS n_addresses,
       (2000 * :sf_acct)::bigint  AS n_documents,
       (500 * :sf_acct)::bigint   AS n_user_roles,
       (15000 * :sf_acct)::bigint AS n_balances,
       (1500 * :sf_acct)::bigint  AS n_beneficiaries,
       (800 * :sf_acct)::bigint   AS n_standing_orders,
       (300 * :sf_acct)::bigint   AS n_payment_batches,
       (3000 * :sf_acct)::bigint  AS n_payment_items,
       (1200 * :sf_acct)::bigint  AS n_loans,
       (7200 * :sf_acct)::bigint  AS n_loan_schedules,
       (900 * :sf_acct)::bigint   AS n_loan_collateral,
       (2500 * :sf_acct)::bigint  AS n_cards,
       (6000 * :sf_acct)::bigint  AS n_card_txns,
       (8000 * :sf_txn)::bigint   AS n_gl_entries,
       (3000 * :sf_acct)::bigint  AS n_charges,
       (3000 * :sf_acct)::bigint  AS n_exchange,
       (5000 * :sf_acct)::bigint  AS n_statements,
       (2500 * :sf_acct)::bigint  AS n_kyc,
       (1500 * :sf_acct)::bigint  AS n_alerts,
       (10000 * :sf_acct)::bigint AS n_audit,
       (2000 * :sf_acct)::bigint  AS n_cheques,
       (400 * :sf_acct)::bigint   AS n_disputes,
       (2000 * :sf_acct)::bigint  AS n_notes
\gset

-- Which part(s) to load, and the row window inside the part.
\if :{?run}
\else
\set run all
\endif
\if :{?lo}
\else
\set lo 1
\endif
\if :{?hi}
\else
\set hi ''
\endif

-- The load plan: order (foreign keys need parents first), part name, rows and
-- whether the part can be split into row windows. Fixed reference data cannot be
-- windowed (it is built from cross joins or from other tables), so it always
-- loads in one call; every generated table can. build.sh reads this with
-- -v list_parts=1 and loads the parts in this order.
\if :{?list_parts}
SELECT ord, part, rows, batched FROM (VALUES
  (1,  'head',             0::bigint,  0),
  (2,  'countries',        30::bigint, 0),
  (3,  'currencies',       12::bigint, 0),
  (4,  'branches',         60::bigint, 0),
  (5,  'branch_hours',     300::bigint, 0),
  (6,  'holidays',         100::bigint, 0),
  (7,  'products',         40::bigint, 0),
  (8,  'account_types',    10::bigint, 0),
  (9,  'customers',        :n_customers,        1),
  (10, 'customer_addresses', :n_addresses,      1),
  (11, 'customer_documents', :n_documents,      1),
  (12, 'user_roles',       :n_user_roles,       1),
  (13, 'accounts',         :n_accounts,         1),
  (14, 'account_holders',  :n_accounts + :n_accounts / 10, 1),
  (15, 'balances_daily',   :n_balances,         1),
  (16, 'transaction_types', 20::bigint, 0),
  (17, 'transactions',     :n_transactions,     1),
  (18, 'beneficiaries',    :n_beneficiaries,    1),
  (19, 'standing_orders',  :n_standing_orders,  1),
  (20, 'payment_batches',  :n_payment_batches,  1),
  (21, 'payment_items',    :n_payment_items,    1),
  (22, 'loans',            :n_loans,            1),
  (23, 'loan_schedules',   :n_loan_schedules,   1),
  (24, 'loan_collateral',  :n_loan_collateral,  1),
  (25, 'cards',            :n_cards,            1),
  (26, 'card_transactions', :n_card_txns,       1),
  (27, 'gl_accounts',      60::bigint, 0),
  (28, 'gl_entries',       :n_gl_entries,       1),
  (29, 'fx_rates',         2640::bigint, 0),
  (30, 'interest_rates',   400::bigint, 0),
  (31, 'fees',             25::bigint, 0),
  (32, 'charges',          :n_charges,          1),
  (33, 'exchange_audit',   :n_exchange,         1),
  (34, 'statements',       :n_statements,       1),
  (35, 'kyc_checks',       :n_kyc,              1),
  (36, 'alerts',           :n_alerts,           1),
  (37, 'rate_bands',       120::bigint, 0),
  (38, 'audit_log',        :n_audit,            1),
  (39, 'cheques',          :n_cheques,          1),
  (40, 'disputes',         :n_disputes,         1),
  (41, 'notes',            :n_notes,            1),
  (42, 'limits',           120::bigint, 0),
  (43, 'tail',             0::bigint,  0)
) AS p(ord, part, rows, batched)
ORDER BY ord;
\quit
\endif

-- ---------------------------------------------------------------------------
-- The one large object (PLAN.md 8.1 wants one). It is created with a FIXED oid
-- so customer_documents.content_oid is the same on every run; a generated oid
-- comes from the cluster-wide counter and would change from build to build.
-- pg_dump/pg_restore keep the oid, so both sides match.
-- ---------------------------------------------------------------------------
SELECT (:'run' IN ('all', 'head')) AS run_head \gset
\if :run_head
DO $$
BEGIN
  PERFORM lo_from_bytea(7654321,
                        decode(md5('blob-one') || md5('blob-two') || md5('blob-three'), 'hex'));
END;
$$;
\endif

-- ---------------------------------------------------------------------------
-- Reference data (fixed size: lookup lists, not customer data)
-- ---------------------------------------------------------------------------
SELECT (:'run' IN ('all', 'countries')) AS run_countries \gset
\if :run_countries
INSERT INTO demo_bank.countries (country_code, country_name, dial_code, region_code, is_active)
SELECT chr(65 + ((g - 1) / 26)) || chr(65 + ((g - 1) % 26)),
       'Country ' || g,
       '+' || (100 + g)::text,
       'R' || lpad((1 + (g % 4))::text, 2, '0'),
       CASE WHEN g % 11 = 0 THEN 'N' ELSE 'Y' END
FROM generate_series(1, 30) g;
\endif

SELECT (:'run' IN ('all', 'currencies')) AS run_currencies \gset
\if :run_currencies
INSERT INTO demo_bank.currencies (currency_code, currency_name, minor_units, symbol_text, is_active)
SELECT 'X' || lpad(g::text, 2, '0'),
       'Unit ' || g,
       (g % 3)::smallint,
       chr(192 + (g % 60)),
       CASE WHEN g = 12 THEN 'N' ELSE 'Y' END
FROM generate_series(1, 12) g;
\endif

SELECT (:'run' IN ('all', 'branches')) AS run_branches \gset
\if :run_branches
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
\endif

SELECT (:'run' IN ('all', 'branch_hours')) AS run_branch_hours \gset
\if :run_branch_hours
INSERT INTO demo_bank.branch_hours (branch_id, weekday, open_time, close_time, open_duration)
SELECT b.branch_id,
       d.w,
       time '09:00' + (b.branch_id % 2) * interval '1 hour',
       time '17:00' + (b.branch_id % 3) * interval '30 minutes',
       ((8 + (b.branch_id % 3)) || ' hours')::interval
FROM demo_bank.branches b
CROSS JOIN generate_series(1, 5) AS d(w)
ORDER BY b.branch_id, d.w;
\endif

SELECT (:'run' IN ('all', 'holidays')) AS run_holidays \gset
\if :run_holidays
INSERT INTO demo_bank.holidays (holiday_date, country_code, holiday_name, is_half_day)
SELECT date '2024-01-01' + (g * 3),
       chr(65 + (((g - 1) % 30) / 26)) || chr(65 + (((g - 1) % 30) % 26)),
       'Holiday ' || g || CASE WHEN g % 8 = 0 THEN ' (' || chr(241) || ')' ELSE '' END,
       CASE WHEN g % 9 = 0 THEN 'Y' ELSE 'N' END
FROM generate_series(1, 100) g;
\endif

SELECT (:'run' IN ('all', 'products')) AS run_products \gset
\if :run_products
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
\endif

SELECT (:'run' IN ('all', 'account_types')) AS run_account_types \gset
\if :run_account_types
INSERT INTO demo_bank.account_types (account_type_id, type_code, type_name,
                                     allows_overdraft, min_balance)
SELECT g,
       'AT' || lpad(g::text, 2, '0'),
       (ARRAY['Everyday', 'Savings', 'Premium', 'Business', 'Student',
              'Pension', 'Joint', 'Trust', 'Escrow', 'Settlement'])[g],
       CASE WHEN g % 3 = 0 THEN 'Y' ELSE 'N' END,
       ((g % 5) * 1000)::numeric(20,4)
FROM generate_series(1, 10) g;
\endif

-- ---------------------------------------------------------------------------
-- Customers (n_customers = 2000 * sf_acct)
-- ---------------------------------------------------------------------------
SELECT (:'run' IN ('all', 'customers')) AS run_customers \gset
\if :run_customers
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
FROM generate_series(:lo, coalesce(nullif(:'hi', '')::bigint, :n_customers)::int) g;
\endif

SELECT (:'run' IN ('all', 'customer_addresses')) AS run_customer_addresses \gset
\if :run_customer_addresses
INSERT INTO demo_bank.customer_addresses (address_id, customer_id, address_type,
                                          line1, line2, city, post_code, country_code,
                                          is_primary)
SELECT g,
       CASE WHEN g <= :n_customers THEN g ELSE g - :n_customers END,
       CASE WHEN g <= :n_customers THEN 'HOME' ELSE 'WORK' END,
       (g % 50) || ' Main Street'
         || CASE WHEN g % 6 = 0 THEN ', ' || chr(193) || 'vila' ELSE '' END,
       CASE WHEN g % 3 = 0 THEN '' WHEN g % 7 = 0 THEN NULL ELSE 'Unit ' || (g % 50) END,
       'Town ' || (g % 80),
       CASE WHEN g % 19 = 0 THEN NULL ELSE lpad(g::text, 6, '0') END,
       chr(65 + ((g % 30) / 26)) || chr(65 + ((g % 30) % 26)),
       CASE WHEN g % 5 = 0 THEN 'Y' ELSE 'N' END
FROM generate_series(:lo, coalesce(nullif(:'hi', '')::bigint, :n_addresses)::int) g;
\endif

-- One document points at the large object above; the rest carry an inline blob
-- or nothing at all (the CHECK allows only one source at a time).
SELECT (:'run' IN ('all', 'customer_documents')) AS run_customer_documents \gset
\if :run_customer_documents
INSERT INTO demo_bank.customer_documents (document_id, customer_id, doc_type,
                                          doc_blob, content_oid, uploaded_at, verified)
SELECT g,
       1 + (g % :n_customers),
       (ARRAY['ID', 'PASSPORT', 'UTILITY', 'CONTRACT'])[1 + (g % 4)],
       CASE WHEN g = 1 THEN NULL
            WHEN g % 5 = 0 THEN NULL
            ELSE decode(md5('doc' || g::text) || md5('docb' || g::text), 'hex') END,
       CASE WHEN g = 1 THEN 7654321::oid ELSE NULL END,
       timestamptz '2015-01-01 00:00:00+00' + (g * interval '9 hours'),
       CASE WHEN g % 4 = 0 THEN 'Y' ELSE 'N' END
FROM generate_series(:lo, coalesce(nullif(:'hi', '')::bigint, :n_documents)::int) g;
\endif

SELECT (:'run' IN ('all', 'user_roles')) AS run_user_roles \gset
\if :run_user_roles
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
FROM generate_series(:lo, coalesce(nullif(:'hi', '')::bigint, :n_user_roles)::int) g;
\endif

-- ---------------------------------------------------------------------------
-- Accounts and their history (n_accounts = 5000 * sf_acct)
-- ---------------------------------------------------------------------------
SELECT (:'run' IN ('all', 'accounts')) AS run_accounts \gset
\if :run_accounts
INSERT INTO demo_bank.accounts (account_id, account_no, customer_id, branch_id,
                                account_type_id, currency_code, balance,
                                available_balance, overdraft_limit, interest_rate,
                                opened_on, closed_on, status, statement_cycle,
                                created_at, updated_at)
SELECT g,
       'AC' || lpad(g::text, 14, '0'),
       1 + (g % :n_customers),
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
FROM generate_series(:lo, coalesce(nullif(:'hi', '')::bigint, :n_accounts)::int) g
CROSS JOIN LATERAL (
  SELECT ((('x' || substr(md5('bal' || g::text), 1, 7))::bit(28)::int % 100000000)::numeric
            / 10000)::numeric(20,4) AS amount
) AS bal;
\endif

-- Windowed by account_id, so a batch is a range of accounts (10% also get a
-- JOINT holder row, so the row count is n_accounts + n_accounts/10).
SELECT (:'run' IN ('all', 'account_holders')) AS run_account_holders \gset
\if :run_account_holders
INSERT INTO demo_bank.account_holders (account_id, customer_id, holder_role,
                                       share_pct, added_on)
SELECT a.account_id, a.customer_id, 'PRIMARY', 100.00, a.opened_on
FROM demo_bank.accounts a
WHERE a.account_id BETWEEN :lo AND coalesce(nullif(:'hi', '')::bigint, :n_accounts)
UNION ALL
SELECT a.account_id, 1 + ((a.account_id * 7) % :n_customers), 'JOINT', 50.00, a.opened_on
FROM demo_bank.accounts a
WHERE a.account_id % 10 = 0
  AND a.account_id BETWEEN :lo AND coalesce(nullif(:'hi', '')::bigint, :n_accounts);
\endif

SELECT (:'run' IN ('all', 'balances_daily')) AS run_balances_daily \gset
\if :run_balances_daily
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
FROM generate_series(:lo, coalesce(nullif(:'hi', '')::bigint, :n_balances)::int) g;
\endif

SELECT (:'run' IN ('all', 'transaction_types')) AS run_transaction_types \gset
\if :run_transaction_types
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
\endif

-- The large history table: 20,000 rows in the small size, 20,000,000 in the
-- large one. The currency comes from the account so the composite foreign key
-- always holds.
SELECT (:'run' IN ('all', 'transactions')) AS run_transactions \gset
\if :run_transactions
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
FROM generate_series(:lo, coalesce(nullif(:'hi', '')::bigint, :n_transactions)::int) g
CROSS JOIN LATERAL (
  SELECT g,
         CASE WHEN raw.v = 0 THEN 1::numeric ELSE raw.v END AS amount
  FROM (
    SELECT ((('x' || substr(md5('txn' || g::text), 1, 7))::bit(28)::int % 50000000)::numeric
              / 10000)::numeric(20,4) AS v
  ) raw
) AS t
JOIN demo_bank.accounts a ON a.account_id = 1 + (t.g % :n_accounts);
\endif

-- ---------------------------------------------------------------------------
-- Payments out
-- ---------------------------------------------------------------------------
SELECT (:'run' IN ('all', 'beneficiaries')) AS run_beneficiaries \gset
\if :run_beneficiaries
INSERT INTO demo_bank.beneficiaries (beneficiary_id, customer_id, beneficiary_name,
                                     account_no, bank_code, currency_code, added_on,
                                     is_active)
SELECT g,
       1 + (g % :n_customers),
       (ARRAY['Electric Co', 'Water Board', 'Landlord', 'School',
              'S' || chr(227) || 'o Jo' || chr(227) || 'o', 'Mobile Top-up'])[1 + (g % 6)],
       'EXT' || lpad(g::text, 12, '0'),
       'BK' || lpad((g % 50)::text, 6, '0'),
       'X' || lpad((1 + (g % 12))::text, 2, '0'),
       date '2018-01-01' + (g % 2000),
       CASE WHEN g % 15 = 0 THEN 'N' ELSE 'Y' END
FROM generate_series(:lo, coalesce(nullif(:'hi', '')::bigint, :n_beneficiaries)::int) g;
\endif

SELECT (:'run' IN ('all', 'standing_orders')) AS run_standing_orders \gset
\if :run_standing_orders
INSERT INTO demo_bank.standing_orders (standing_order_id, account_id, beneficiary_id,
                                       amount, frequency, next_run_date, end_date,
                                       is_active)
SELECT g,
       1 + (g % :n_accounts),
       1 + (g % :n_beneficiaries),
       ((g % 20000)::numeric / 100 + 1)::numeric(20,4),
       (ARRAY['DAILY', 'WEEKLY', 'MONTHLY'])[1 + (g % 3)],
       date '2025-01-01' + (g % 300),
       CASE WHEN g % 6 = 0 THEN date '2026-12-31' ELSE NULL END,
       CASE WHEN g % 12 = 0 THEN 'N' ELSE 'Y' END
FROM generate_series(:lo, coalesce(nullif(:'hi', '')::bigint, :n_standing_orders)::int) g;
\endif

SELECT (:'run' IN ('all', 'payment_batches')) AS run_payment_batches \gset
\if :run_payment_batches
INSERT INTO demo_bank.payment_batches (batch_id, batch_ref, created_by, created_at,
                                       item_count, total_amount, status)
SELECT g,
       'BATCH' || lpad(g::text, 6, '0'),
       'operator' || (g % 7),
       timestamptz '2024-06-01 00:00:00+00' + (g * interval '3 hours'),
       10,
       ((g % 100000)::numeric / 10)::numeric(20,4),
       (ARRAY['N', 'S', 'C', 'C', 'F'])[1 + (g % 5)]
FROM generate_series(:lo, coalesce(nullif(:'hi', '')::bigint, :n_payment_batches)::int) g;
\endif

SELECT (:'run' IN ('all', 'payment_items')) AS run_payment_items \gset
\if :run_payment_items
INSERT INTO demo_bank.payment_items (batch_id, item_no, beneficiary_id, amount,
                                     currency_code, status)
SELECT 1 + ((g - 1) / 10),
       1 + ((g - 1) % 10),
       1 + (g % :n_beneficiaries),
       ((g % 20000)::numeric / 100 + 1)::numeric(20,4),
       'X' || lpad((1 + (g % 12))::text, 2, '0'),
       (ARRAY['N', 'S', 'C', 'C', 'F'])[1 + (g % 5)]
FROM generate_series(:lo, coalesce(nullif(:'hi', '')::bigint, :n_payment_items)::int) g;
\endif

-- ---------------------------------------------------------------------------
-- Lending
-- ---------------------------------------------------------------------------
SELECT (:'run' IN ('all', 'loans')) AS run_loans \gset
\if :run_loans
INSERT INTO demo_bank.loans (loan_id, loan_no, customer_id, branch_id, product_id,
                             principal, outstanding, interest_rate, disbursed_on,
                             maturity_date, status, penalty_charge, created_at)
SELECT g,
       'LN' || lpad(g::text, 14, '0'),
       1 + (g % :n_customers),
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
FROM generate_series(:lo, coalesce(nullif(:'hi', '')::bigint, :n_loans)::int) g
CROSS JOIN LATERAL (
  SELECT ((('x' || substr(md5('loan' || g::text), 1, 7))::bit(28)::int % 100000000)::numeric
            / 100)::numeric(20,4) AS amount
) AS principal;
\endif

SELECT (:'run' IN ('all', 'loan_schedules')) AS run_loan_schedules \gset
\if :run_loan_schedules
INSERT INTO demo_bank.loan_schedules (loan_id, installment_no, due_date,
                                      principal_due, interest_due, paid_amount)
SELECT 1 + ((g - 1) / 6),
       1 + ((g - 1) % 6),
       (l.disbursed_on + ((1 + ((g - 1) % 6)) || ' months')::interval)::date,
       ((g % 5000)::numeric / 10)::numeric(20,4),
       ((g % 700)::numeric / 10)::numeric(20,4),
       CASE WHEN g % 4 = 0 THEN ((g % 5000)::numeric / 10)::numeric(20,4)
            ELSE 0::numeric(20,4) END
FROM generate_series(:lo, coalesce(nullif(:'hi', '')::bigint, :n_loan_schedules)::int) g
JOIN demo_bank.loans l ON l.loan_id = 1 + ((g - 1) / 6);
\endif

SELECT (:'run' IN ('all', 'loan_collateral')) AS run_loan_collateral \gset
\if :run_loan_collateral
INSERT INTO demo_bank.loan_collateral (collateral_id, loan_id, collateral_type,
                                       description, market_value, valued_on,
                                       currency_code)
SELECT g,
       1 + (g % :n_loans),
       (ARRAY['PROPERTY', 'VEHICLE', 'DEPOSIT', 'GUARANTEE'])[1 + (g % 4)],
       CASE WHEN g % 8 = 0 THEN NULL
            WHEN g % 5 = 0 THEN ''
            ELSE 'Asset ' || g || ' in ' || chr(193) || 'vila' END,
       ((g % 500000)::numeric / 10 + 1)::numeric(20,4),
       date '2020-01-01' + (g % 1500),
       'X' || lpad((1 + (g % 12))::text, 2, '0')
FROM generate_series(:lo, coalesce(nullif(:'hi', '')::bigint, :n_loan_collateral)::int) g;
\endif

-- ---------------------------------------------------------------------------
-- Cards
-- ---------------------------------------------------------------------------
SELECT (:'run' IN ('all', 'cards')) AS run_cards \gset
\if :run_cards
INSERT INTO demo_bank.cards (card_id, card_no, account_id, card_type, expiry_date,
                             credit_limit, status, issued_at, is_contactless)
SELECT g,
       lpad(g::text, 16, '0'),
       1 + (g % :n_accounts),
       (ARRAY['DEBIT', 'CREDIT', 'PREPAID'])[1 + (g % 3)],
       date '2027-01-01' + (g % 700),
       CASE WHEN g % 3 = 0 THEN ((g % 50) * 100)::numeric(20,4) ELSE 0::numeric(20,4) END,
       (ARRAY['A', 'A', 'A', 'B', 'E', 'X'])[1 + (g % 6)],
       timestamp '2016-01-01 12:00:00' + (g * interval '4 hours'),
       CASE WHEN g % 4 = 0 THEN 'N' ELSE 'Y' END
FROM generate_series(:lo, coalesce(nullif(:'hi', '')::bigint, :n_cards)::int) g;
\endif

SELECT (:'run' IN ('all', 'card_transactions')) AS run_card_transactions \gset
\if :run_card_transactions
INSERT INTO demo_bank.card_transactions (card_txn_id, card_id, posted_at, merchant,
                                         amount, currency_code, is_declined, auth_code)
SELECT g,
       1 + (g % :n_cards),
       timestamptz '2024-03-01 00:00:00+00' + (g * interval '13 minutes'),
       (ARRAY['Grocer', 'Fuel Stop', 'Caf' || chr(233) || ' Central', 'Book Shop',
              'Rail Co', 'Pharmacy'])[1 + (g % 6)],
       CASE WHEN raw.v = 0 THEN 5::numeric ELSE raw.v END,
       'X' || lpad((1 + (g % 12))::text, 2, '0'),
       CASE WHEN g % 9 = 0 THEN 'Y' ELSE 'N' END,
       CASE WHEN g % 9 = 0 THEN NULL ELSE 'A' || lpad((g % 100000)::text, 6, '0') END
FROM generate_series(:lo, coalesce(nullif(:'hi', '')::bigint, :n_card_txns)::int) g
CROSS JOIN LATERAL (
  SELECT ((('x' || substr(md5('ct' || g::text), 1, 7))::bit(28)::int % 5000000)::numeric
            / 100)::numeric(20,4) AS v
) AS raw;
\endif

-- ---------------------------------------------------------------------------
-- General ledger, rates and fees
-- ---------------------------------------------------------------------------
-- Parents first: the self-referencing foreign key is checked at the end of the
-- statement, but loading the roots first keeps the intent obvious. Both
-- statements belong to one part; the table is fixed reference data (60 rows).
SELECT (:'run' IN ('all', 'gl_accounts')) AS run_gl_accounts \gset
\if :run_gl_accounts
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
\endif

SELECT (:'run' IN ('all', 'gl_entries')) AS run_gl_entries \gset
\if :run_gl_entries
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
  FROM generate_series(:lo, coalesce(nullif(:'hi', '')::bigint, :n_gl_entries)::int) g
  ORDER BY g
) AS e;
\endif

SELECT (:'run' IN ('all', 'fx_rates')) AS run_fx_rates \gset
\if :run_fx_rates
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
\endif

SELECT (:'run' IN ('all', 'interest_rates')) AS run_interest_rates \gset
\if :run_interest_rates
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
\endif

SELECT (:'run' IN ('all', 'fees')) AS run_fees \gset
\if :run_fees
INSERT INTO demo_bank.fees (fee_id, fee_code, fee_name, fee_type, flat_amount, pct_rate)
SELECT g,
       'FEE' || lpad(g::text, 3, '0'),
       (ARRAY['Monthly maintenance', 'Cash handling', 'Overdraft', 'Late payment',
              'Card replacement'])[1 + (g % 5)] || ' ' || g,
       (ARRAY['FLAT', 'PERCENT', 'TIERED'])[1 + (g % 3)],
       CASE WHEN g % 3 = 0 THEN ((g % 50)::numeric)::numeric(20,4) ELSE 0::numeric(20,4) END,
       CASE WHEN g % 3 = 1 THEN (g % 300)::float8 / 10000.0 ELSE 0::float8 END
FROM generate_series(1, 25) g;
\endif

SELECT (:'run' IN ('all', 'charges')) AS run_charges \gset
\if :run_charges
INSERT INTO demo_bank.charges (charge_id, account_id, fee_id, charged_on, amount,
                               waive_flag, narration)
SELECT g,
       1 + (g % :n_accounts),
       1 + (g % 25),
       date '2024-01-01' + (g % 365),
       ((g % 500)::numeric / 100)::numeric(20,4),
       CASE WHEN g % 8 = 0 THEN 'Y' ELSE 'N' END,
       CASE WHEN g % 6 = 0 THEN NULL
            WHEN g % 9 = 0 THEN ''
            WHEN g % 10 = 0 THEN 'Fee charged   '
            ELSE 'Fee ' || g || ' (' || chr(233) || ')' END
FROM generate_series(:lo, coalesce(nullif(:'hi', '')::bigint, :n_charges)::int) g;
\endif

SELECT (:'run' IN ('all', 'exchange_audit')) AS run_exchange_audit \gset
\if :run_exchange_audit
INSERT INTO demo_bank.exchange_audit (exchange_id, from_currency, to_currency, rate,
                                      converted_amount, requested_at, status)
SELECT g,
       'X' || lpad((1 + (g % 12))::text, 2, '0'),
       'X' || lpad((1 + ((g + 5) % 12))::text, 2, '0'),
       (1 + ((g % 500000)::numeric / 100000))::numeric(18,8),
       ((g % 10000000)::numeric / 100)::numeric(20,4),
       timestamptz '2024-02-01 00:00:00+00' + (g * interval '17 minutes'),
       (ARRAY['C', 'C', 'C', 'F', 'R'])[1 + (g % 5)]
FROM generate_series(:lo, coalesce(nullif(:'hi', '')::bigint, :n_exchange)::int) g;
\endif

-- ---------------------------------------------------------------------------
-- Statements, compliance, notes, limits
-- ---------------------------------------------------------------------------
SELECT (:'run' IN ('all', 'statements')) AS run_statements \gset
\if :run_statements
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
FROM generate_series(:lo, coalesce(nullif(:'hi', '')::bigint, :n_statements)::int) g;
\endif

SELECT (:'run' IN ('all', 'kyc_checks')) AS run_kyc_checks \gset
\if :run_kyc_checks
INSERT INTO demo_bank.kyc_checks (check_id, customer_id, check_type, checked_at,
                                  result, detail)
SELECT g,
       1 + (g % :n_customers),
       (ARRAY['IDENTITY', 'ADDRESS', 'SANCTIONS', 'PEP'])[1 + (g % 4)],
       timestamptz '2022-01-01 00:00:00+00' + (g * interval '19 minutes'),
       (ARRAY['P', 'P', 'P', 'F', 'R'])[1 + (g % 5)],
       ('{"score":' || (g % 100) || ',"flags":["A","B"],"note":null,"version":2}')::jsonb
FROM generate_series(:lo, coalesce(nullif(:'hi', '')::bigint, :n_kyc)::int) g;
\endif

SELECT (:'run' IN ('all', 'alerts')) AS run_alerts \gset
\if :run_alerts
INSERT INTO demo_bank.alerts (alert_id, customer_id, raised_at, rule_code, severity,
                              payload)
SELECT g,
       1 + (g % :n_customers),
       timestamptz '2024-04-01 00:00:00+00' + (g * interval '23 minutes'),
       'R' || lpad((1 + (g % 20))::text, 3, '0'),
       (1 + (g % 5))::smallint,
       ('{"rule":"R' || (g % 20) || '","amount":' || (g % 1000)
          || ',"channel":"WEB","tags":["kyc","review"]}')::json
FROM generate_series(:lo, coalesce(nullif(:'hi', '')::bigint, :n_alerts)::int) g;
\endif

SELECT (:'run' IN ('all', 'rate_bands')) AS run_rate_bands \gset
\if :run_rate_bands
INSERT INTO demo_bank.rate_bands (band_id, product_id, amount_band, rate_pct,
                                  effective_from)
SELECT g,
       1 + (g % 40),
       numrange(((g % 10) * 1000)::numeric, (((g % 10) + 1) * 1000)::numeric, '[)'),
       (g % 900)::float8 / 10000.0,
       date '2023-06-01' + (g % 400)
FROM generate_series(1, 120) g;
\endif

-- No key, duplicates on purpose: the same (event_at, actor, action, object_name)
-- combination repeats, and every fifth row has a NULL detail.
SELECT (:'run' IN ('all', 'audit_log')) AS run_audit_log \gset
\if :run_audit_log
INSERT INTO demo_bank.audit_log (event_at, actor, action, object_name, detail)
SELECT timestamptz '2024-01-01 00:00:00+00' + ((g % 400) * interval '1 hour'),
       (ARRAY['teller1', 'teller2', 'supervisor', 'batch', 'trigger'])[1 + (g % 5)],
       (ARRAY['INSERT', 'UPDATE', 'DELETE', 'LOGIN', 'LOGOUT'])[1 + (g % 5)],
       (ARRAY['accounts', 'customers', 'transactions', 'loans', 'cards'])[1 + ((g / 5) % 5)],
       CASE WHEN g % 5 = 0 THEN NULL
            WHEN g % 7 = 0 THEN ''
            ELSE 'detail ' || g END
FROM generate_series(:lo, coalesce(nullif(:'hi', '')::bigint, :n_audit)::int) g;
\endif

SELECT (:'run' IN ('all', 'cheques')) AS run_cheques \gset
\if :run_cheques
INSERT INTO demo_bank.cheques (cheque_id, account_id, cheque_no, issued_on, amount,
                               status, payee, cleared_on)
SELECT g,
       1 + (g % :n_accounts),
       lpad(g::text, 10, '0'),
       date '2024-01-01' + (g % 500),
       ((g % 200000)::numeric / 100 + 1)::numeric(20,4),
       (ARRAY['I', 'I', 'C', 'S', 'B'])[1 + (g % 5)],
       CASE WHEN g % 6 = 0 THEN NULL
            WHEN g % 7 = 0 THEN ''
            ELSE 'Payee ' || g || ' ' || chr(199) || 'o' END,
       CASE WHEN g % 5 = 3 THEN date '2024-01-05' + (g % 500) ELSE NULL END
FROM generate_series(:lo, coalesce(nullif(:'hi', '')::bigint, :n_cheques)::int) g;
\endif

SELECT (:'run' IN ('all', 'disputes')) AS run_disputes \gset
\if :run_disputes
INSERT INTO demo_bank.disputes (card_txn_id, customer_id, opened_at, reason, amount,
                                status, resolved_on)
SELECT g,
       1 + (g % :n_customers),
       timestamptz '2024-05-01 00:00:00+00' + (g * interval '29 minutes'),
       (ARRAY['FRAUD', 'DUPLICATE', 'NOT_RECEIVED', 'WRONG_AMOUNT'])[1 + (g % 4)],
       ((g % 100000)::numeric / 100 + 1)::numeric(20,4),
       (ARRAY['O', 'O', 'R', 'U', 'C'])[1 + (g % 5)],
       CASE WHEN g % 5 >= 2 THEN date '2024-07-01' + (g % 200) ELSE NULL END
FROM generate_series(:lo, coalesce(nullif(:'hi', '')::bigint, :n_disputes)::int) g
ORDER BY g;
\endif

SELECT (:'run' IN ('all', 'notes')) AS run_notes \gset
\if :run_notes
INSERT INTO demo_bank.notes (customer_id, account_id, note_text, author, created_at)
SELECT CASE WHEN n.g % 4 = 0 THEN NULL ELSE 1 + (n.g % :n_customers) END,
       CASE WHEN n.g % 4 = 1 THEN NULL ELSE 1 + (n.g % :n_accounts) END,
       CASE WHEN n.g % 29 = 0 THEN ''
            WHEN n.g % 5 = 0 THEN 'Customer asked about ' || chr(233) || 'pargne   '
            WHEN n.g % 7 = 0 THEN 'Nota en espa' || chr(241) || 'ol: ' || chr(243) || 'rdenes'
            ELSE 'Note ' || n.g || ' about account ' || (1 + (n.g % :n_accounts)) END,
       (ARRAY['teller1', 'supervisor', 'auditor', 'batch'])[1 + (n.g % 4)],
       timestamp '2023-01-01 09:00:00' + (n.g * interval '3 hours')
FROM (
  SELECT g FROM generate_series(:lo, coalesce(nullif(:'hi', '')::bigint, :n_notes)::int) g ORDER BY g
) AS n;
\endif

SELECT (:'run' IN ('all', 'limits')) AS run_limits \gset
\if :run_limits
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
\endif

-- ---------------------------------------------------------------------------
-- Exercise the trigger: 500 accounts get a small credit and a later updated_at.
-- The trigger writes one audit_log row per changed account, using the row's own
-- updated_at as the event time, so the extra rows are reproducible. The fixed
-- 500 is a property of the test data, not a size: it is 500 audit_log rows at
-- every size.
-- ---------------------------------------------------------------------------
SELECT (:'run' IN ('all', 'tail')) AS run_tail \gset
\if :run_tail
UPDATE demo_bank.accounts
SET balance = balance + (account_id % 100)::numeric / 100,
    available_balance = available_balance + (account_id % 100)::numeric / 100,
    updated_at = updated_at + interval '30 days'
WHERE account_id <= 500;

-- The materialised view was created before the accounts existed.
REFRESH MATERIALIZED VIEW demo_bank.mv_branch_totals;
\endif
