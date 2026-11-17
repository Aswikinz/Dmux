-- Dmux public test schema: an INVENTED retail-banking layout (PLAN.md 8.1).
--
-- Everything here is made up: the table, column, code and product names are not
-- taken from any real system or licensed dictionary (decision 11). Do not add
-- real vendor names, real dictionary file names or real data to this file.
--
-- Scope: the public `demo_bank` schema for every size of PLAN.md 8.1 (T015
-- built it; T031 added the `large` size in data.sql). It exists so the engine,
-- the answer key and the UI have a realistic shape to work on:
--   * primary keys, composite keys, foreign keys (including a composite FK),
--     UNIQUE, CHECK, partial and functional indexes;
--   * a table with no key at all (audit_log, duplicates allowed);
--   * sequences, two views, one materialised view, a plpgsql function and trigger;
--   * every column type PLAN.md 3.2 lists: char(n) and char(1) flags, varchar(n),
--     numeric(20,4) money amounts, one money column, float8/real, timestamp(0),
--     timestamptz, date, bytea, json, jsonb, text[], interval, numrange.
--
-- Everything is Postgres 9.4-compatible on purpose: no IDENTITY, no generated
-- columns, no declarative partitioning, no newer catalog functions. The old
-- database of the pair is 9.6, so the schema is loaded there first and then
-- copied to the new one with pg_dump/pg_restore.
--
-- Loaded by build.sh. Deterministic data lives in data.sql; nothing here may
-- call now(), random() or any other non-repeatable function.

SET client_min_messages = warning;

DROP SCHEMA IF EXISTS demo_bank CASCADE;
CREATE SCHEMA demo_bank;

-- ---------------------------------------------------------------------------
-- Sequences (4). Three of them feed column defaults below; the fourth feeds
-- disputes. The generator always inserts in a deterministic order, so the
-- sequence values are reproducible run after run.
-- ---------------------------------------------------------------------------
CREATE SEQUENCE demo_bank.seq_branch    START WITH 1 INCREMENT BY 1 NO MINVALUE NO MAXVALUE CACHE 1;
CREATE SEQUENCE demo_bank.seq_gl_entry  START WITH 1 INCREMENT BY 1 NO MINVALUE NO MAXVALUE CACHE 1;
CREATE SEQUENCE demo_bank.seq_note      START WITH 1 INCREMENT BY 1 NO MINVALUE NO MAXVALUE CACHE 1;
CREATE SEQUENCE demo_bank.seq_dispute   START WITH 1 INCREMENT BY 1 NO MINVALUE NO MAXVALUE CACHE 1;

-- ---------------------------------------------------------------------------
-- Reference data
-- ---------------------------------------------------------------------------
CREATE TABLE demo_bank.countries (
  country_code char(2)      NOT NULL,
  country_name varchar(80)  NOT NULL,
  dial_code    varchar(8),
  region_code  char(3),
  is_active    char(1)      NOT NULL DEFAULT 'Y',
  CONSTRAINT pk_countries PRIMARY KEY (country_code),
  CONSTRAINT ck_countries_active CHECK (is_active IN ('Y', 'N'))
);

CREATE TABLE demo_bank.currencies (
  currency_code char(3)     NOT NULL,
  currency_name varchar(60) NOT NULL,
  minor_units   smallint    NOT NULL,
  symbol_text   varchar(8),
  is_active     char(1)     NOT NULL DEFAULT 'Y',
  CONSTRAINT pk_currencies PRIMARY KEY (currency_code),
  CONSTRAINT ck_currencies_minor CHECK (minor_units BETWEEN 0 AND 4),
  CONSTRAINT ck_currencies_active CHECK (is_active IN ('Y', 'N'))
);

CREATE TABLE demo_bank.branches (
  branch_id     integer      NOT NULL DEFAULT nextval('demo_bank.seq_branch'),
  branch_code   char(6)      NOT NULL,
  branch_name   varchar(80)  NOT NULL,
  country_code  char(2)      NOT NULL,
  city          varchar(60),
  opened_on     date         NOT NULL,
  closed_on     date,
  is_head_office boolean     NOT NULL DEFAULT false,
  is_active     char(1)      NOT NULL DEFAULT 'Y',
  created_at    timestamp(0) NOT NULL,
  updated_at    timestamptz  NOT NULL,
  CONSTRAINT pk_branches PRIMARY KEY (branch_id),
  CONSTRAINT uq_branches_code UNIQUE (branch_code),
  CONSTRAINT fk_branches_country FOREIGN KEY (country_code)
    REFERENCES demo_bank.countries (country_code),
  CONSTRAINT ck_branches_dates CHECK (closed_on IS NULL OR closed_on >= opened_on),
  CONSTRAINT ck_branches_active CHECK (is_active IN ('Y', 'N'))
);
CREATE INDEX ix_branches_country ON demo_bank.branches (country_code);

CREATE TABLE demo_bank.branch_hours (
  branch_id     integer  NOT NULL,
  weekday       smallint NOT NULL,
  open_time     time     NOT NULL,
  close_time    time     NOT NULL,
  open_duration interval NOT NULL,
  CONSTRAINT pk_branch_hours PRIMARY KEY (branch_id, weekday),
  CONSTRAINT fk_branch_hours_branch FOREIGN KEY (branch_id)
    REFERENCES demo_bank.branches (branch_id),
  CONSTRAINT ck_branch_hours_weekday CHECK (weekday BETWEEN 0 AND 6),
  CONSTRAINT ck_branch_hours_times CHECK (close_time > open_time)
);

CREATE TABLE demo_bank.holidays (
  holiday_date date        NOT NULL,
  country_code char(2)     NOT NULL,
  holiday_name varchar(80) NOT NULL,
  is_half_day  char(1)     NOT NULL DEFAULT 'N',
  CONSTRAINT pk_holidays PRIMARY KEY (holiday_date, country_code),
  CONSTRAINT fk_holidays_country FOREIGN KEY (country_code)
    REFERENCES demo_bank.countries (country_code),
  CONSTRAINT ck_holidays_half CHECK (is_half_day IN ('Y', 'N'))
);

CREATE TABLE demo_bank.products (
  product_id    integer     NOT NULL,
  product_code  varchar(12) NOT NULL,
  product_name  varchar(80) NOT NULL,
  product_type  varchar(20) NOT NULL,
  currency_code char(3)     NOT NULL,
  nominal_rate  real,
  is_active     char(1)     NOT NULL DEFAULT 'Y',
  CONSTRAINT pk_products PRIMARY KEY (product_id),
  CONSTRAINT uq_products_code UNIQUE (product_code),
  CONSTRAINT fk_products_currency FOREIGN KEY (currency_code)
    REFERENCES demo_bank.currencies (currency_code),
  CONSTRAINT ck_products_type CHECK (product_type IN ('ACCOUNT', 'LOAN', 'CARD', 'PAYMENT')),
  CONSTRAINT ck_products_active CHECK (is_active IN ('Y', 'N'))
);

CREATE TABLE demo_bank.account_types (
  account_type_id integer      NOT NULL,
  type_code       varchar(8)   NOT NULL,
  type_name       varchar(60)  NOT NULL,
  allows_overdraft char(1)     NOT NULL DEFAULT 'N',
  min_balance     numeric(20,4) NOT NULL DEFAULT 0,
  CONSTRAINT pk_account_types PRIMARY KEY (account_type_id),
  CONSTRAINT uq_account_types_code UNIQUE (type_code),
  CONSTRAINT ck_account_types_overdraft CHECK (allows_overdraft IN ('Y', 'N')),
  CONSTRAINT ck_account_types_min CHECK (min_balance >= 0)
);

-- ---------------------------------------------------------------------------
-- Customers
-- ---------------------------------------------------------------------------
CREATE TABLE demo_bank.customers (
  customer_id   bigint       NOT NULL,
  customer_no   varchar(12)  NOT NULL,
  first_name    varchar(40)  NOT NULL,
  last_name     varchar(40)  NOT NULL,
  date_of_birth date         NOT NULL,
  national_id   varchar(20)  NOT NULL,
  country_code  char(2)      NOT NULL,
  branch_id     integer      NOT NULL,
  risk_grade    char(1)      NOT NULL DEFAULT 'B',
  is_active     char(1)      NOT NULL DEFAULT 'Y',
  created_at    timestamp(0) NOT NULL,
  updated_at    timestamptz  NOT NULL,
  CONSTRAINT pk_customers PRIMARY KEY (customer_id),
  CONSTRAINT uq_customers_no UNIQUE (customer_no),
  CONSTRAINT uq_customers_national_id UNIQUE (national_id),
  CONSTRAINT fk_customers_country FOREIGN KEY (country_code)
    REFERENCES demo_bank.countries (country_code),
  CONSTRAINT fk_customers_branch FOREIGN KEY (branch_id)
    REFERENCES demo_bank.branches (branch_id),
  CONSTRAINT ck_customers_risk CHECK (risk_grade IN ('A', 'B', 'C', 'D')),
  CONSTRAINT ck_customers_active CHECK (is_active IN ('Y', 'N')),
  CONSTRAINT ck_customers_dob CHECK (date_of_birth > date '1900-01-01')
);
CREATE INDEX ix_customers_branch ON demo_bank.customers (branch_id);
-- Functional index: search by surname regardless of case.
CREATE INDEX ix_customers_name ON demo_bank.customers (lower(last_name), lower(first_name));

CREATE TABLE demo_bank.customer_addresses (
  address_id    bigint      NOT NULL,
  customer_id   bigint      NOT NULL,
  address_type  varchar(10) NOT NULL,
  line1         varchar(80) NOT NULL,
  line2         varchar(80),
  city          varchar(60) NOT NULL,
  post_code     varchar(12),
  country_code  char(2)     NOT NULL,
  is_primary    char(1)     NOT NULL DEFAULT 'N',
  CONSTRAINT pk_customer_addresses PRIMARY KEY (address_id),
  CONSTRAINT fk_customer_addresses_customer FOREIGN KEY (customer_id)
    REFERENCES demo_bank.customers (customer_id),
  CONSTRAINT fk_customer_addresses_country FOREIGN KEY (country_code)
    REFERENCES demo_bank.countries (country_code),
  CONSTRAINT ck_customer_addresses_type CHECK (address_type IN ('HOME', 'WORK', 'POST')),
  CONSTRAINT ck_customer_addresses_primary CHECK (is_primary IN ('Y', 'N'))
);
-- At most one HOME address per customer (partial unique index, 9.4-safe).
CREATE UNIQUE INDEX ux_customer_addresses_home
  ON demo_bank.customer_addresses (customer_id) WHERE address_type = 'HOME';

CREATE TABLE demo_bank.customer_documents (
  document_id  bigint      NOT NULL,
  customer_id  bigint      NOT NULL,
  doc_type     varchar(12) NOT NULL,
  doc_blob     bytea,
  content_oid  oid,
  uploaded_at  timestamptz NOT NULL,
  verified     char(1)     NOT NULL DEFAULT 'N',
  CONSTRAINT pk_customer_documents PRIMARY KEY (document_id),
  CONSTRAINT fk_customer_documents_customer FOREIGN KEY (customer_id)
    REFERENCES demo_bank.customers (customer_id),
  CONSTRAINT ck_customer_documents_type CHECK (doc_type IN ('ID', 'PASSPORT', 'UTILITY', 'CONTRACT')),
  CONSTRAINT ck_customer_documents_verified CHECK (verified IN ('Y', 'N')),
  -- A document carries either an inline blob or a large-object reference.
  CONSTRAINT ck_customer_documents_source CHECK (doc_blob IS NULL OR content_oid IS NULL)
);
CREATE INDEX ix_customer_documents_customer ON demo_bank.customer_documents (customer_id);

CREATE TABLE demo_bank.user_roles (
  user_id     integer     NOT NULL,
  role_name   varchar(30) NOT NULL,
  granted_on  date        NOT NULL,
  permissions text[]      NOT NULL,
  granted_by  varchar(40),
  CONSTRAINT pk_user_roles PRIMARY KEY (user_id, role_name),
  CONSTRAINT ck_user_roles_role CHECK (role_name IN ('TELLER', 'SUPERVISOR', 'AUDITOR', 'ADMIN'))
);

-- ---------------------------------------------------------------------------
-- Accounts and their history
-- ---------------------------------------------------------------------------
CREATE TABLE demo_bank.accounts (
  account_id        bigint       NOT NULL,
  account_no        varchar(16)  NOT NULL,
  customer_id       bigint       NOT NULL,
  branch_id         integer      NOT NULL,
  account_type_id   integer      NOT NULL,
  currency_code     char(3)      NOT NULL,
  balance           numeric(20,4) NOT NULL DEFAULT 0,
  available_balance numeric(20,4) NOT NULL DEFAULT 0,
  overdraft_limit   numeric(20,4) NOT NULL DEFAULT 0,
  interest_rate     float8       NOT NULL DEFAULT 0,
  opened_on         date         NOT NULL,
  closed_on         date,
  status            char(1)      NOT NULL DEFAULT 'A',
  statement_cycle   char(2)      NOT NULL DEFAULT '01',
  created_at        timestamp(0) NOT NULL,
  updated_at        timestamptz  NOT NULL,
  CONSTRAINT pk_accounts PRIMARY KEY (account_id),
  CONSTRAINT uq_accounts_no UNIQUE (account_no),
  -- Redundant with the primary key, but it lets transactions carry a composite
  -- foreign key, so a transaction can never point at another currency's account.
  CONSTRAINT uq_accounts_id_currency UNIQUE (account_id, currency_code),
  CONSTRAINT fk_accounts_customer FOREIGN KEY (customer_id)
    REFERENCES demo_bank.customers (customer_id),
  CONSTRAINT fk_accounts_branch FOREIGN KEY (branch_id)
    REFERENCES demo_bank.branches (branch_id),
  CONSTRAINT fk_accounts_type FOREIGN KEY (account_type_id)
    REFERENCES demo_bank.account_types (account_type_id),
  CONSTRAINT fk_accounts_currency FOREIGN KEY (currency_code)
    REFERENCES demo_bank.currencies (currency_code),
  CONSTRAINT ck_accounts_status CHECK (status IN ('A', 'D', 'C', 'F')),
  CONSTRAINT ck_accounts_overdraft CHECK (overdraft_limit >= 0),
  CONSTRAINT ck_accounts_closed CHECK (closed_on IS NULL OR closed_on >= opened_on),
  CONSTRAINT ck_accounts_cycle CHECK (statement_cycle ~ '^[0-3][0-9]$')
);
CREATE INDEX ix_accounts_customer ON demo_bank.accounts (customer_id);
CREATE INDEX ix_accounts_branch ON demo_bank.accounts (branch_id, status);
-- Partial index: the "open accounts of a branch" query.
CREATE INDEX ix_accounts_open ON demo_bank.accounts (branch_id, opened_on) WHERE status = 'A';

CREATE TABLE demo_bank.account_holders (
  account_id   bigint      NOT NULL,
  customer_id  bigint      NOT NULL,
  holder_role  varchar(10) NOT NULL,
  share_pct    numeric(5,2) NOT NULL,
  added_on     date        NOT NULL,
  CONSTRAINT pk_account_holders PRIMARY KEY (account_id, customer_id, holder_role),
  CONSTRAINT fk_account_holders_account FOREIGN KEY (account_id)
    REFERENCES demo_bank.accounts (account_id),
  CONSTRAINT fk_account_holders_customer FOREIGN KEY (customer_id)
    REFERENCES demo_bank.customers (customer_id),
  CONSTRAINT ck_account_holders_role CHECK (holder_role IN ('PRIMARY', 'JOINT', 'SIGNATORY')),
  CONSTRAINT ck_account_holders_share CHECK (share_pct > 0 AND share_pct <= 100)
);

CREATE TABLE demo_bank.balances_daily (
  account_id      bigint        NOT NULL,
  balance_date    date          NOT NULL,
  opening_balance numeric(20,4) NOT NULL,
  closing_balance numeric(20,4) NOT NULL,
  debit_total     numeric(20,4) NOT NULL,
  credit_total    numeric(20,4) NOT NULL,
  txn_count       integer       NOT NULL,
  CONSTRAINT pk_balances_daily PRIMARY KEY (account_id, balance_date),
  CONSTRAINT fk_balances_daily_account FOREIGN KEY (account_id)
    REFERENCES demo_bank.accounts (account_id),
  CONSTRAINT ck_balances_daily_counts CHECK (txn_count >= 0),
  CONSTRAINT ck_balances_daily_totals CHECK (debit_total >= 0 AND credit_total >= 0)
);
CREATE INDEX ix_balances_daily_date ON demo_bank.balances_daily (balance_date);

CREATE TABLE demo_bank.transaction_types (
  transaction_type_id integer     NOT NULL,
  type_code           varchar(8)  NOT NULL,
  type_description    varchar(80) NOT NULL,
  dr_cr               char(1)     NOT NULL,
  affects_balance     char(1)     NOT NULL DEFAULT 'Y',
  CONSTRAINT pk_transaction_types PRIMARY KEY (transaction_type_id),
  CONSTRAINT uq_transaction_types_code UNIQUE (type_code),
  CONSTRAINT ck_transaction_types_dr_cr CHECK (dr_cr IN ('D', 'C')),
  CONSTRAINT ck_transaction_types_affects CHECK (affects_balance IN ('Y', 'N'))
);

-- The large history table: 20,000 rows in the small size, 20,000,000 in the
-- large one (data.sql scales it with the transaction multiplier).
CREATE TABLE demo_bank.transactions (
  transaction_id     bigint        NOT NULL,
  account_id         bigint        NOT NULL,
  currency_code      char(3)       NOT NULL,
  transaction_type_id integer      NOT NULL,
  txn_date           date          NOT NULL,
  posted_at          timestamptz   NOT NULL,
  value_timestamp    timestamp(0)  NOT NULL,
  amount             numeric(20,4) NOT NULL,
  fee_amount         numeric(20,4) NOT NULL DEFAULT 0,
  running_balance    numeric(20,4) NOT NULL,
  narration          varchar(120),
  reference          varchar(24),
  channel            char(1)       NOT NULL DEFAULT 'B',
  status             char(1)       NOT NULL DEFAULT 'P',
  is_reversal        char(1)       NOT NULL DEFAULT 'N',
  created_at         timestamp(0)  NOT NULL,
  CONSTRAINT pk_transactions PRIMARY KEY (transaction_id),
  CONSTRAINT uq_transactions_reference UNIQUE (reference),
  -- Composite foreign key: the currency must match the account's own currency.
  CONSTRAINT fk_transactions_account FOREIGN KEY (account_id, currency_code)
    REFERENCES demo_bank.accounts (account_id, currency_code),
  CONSTRAINT fk_transactions_type FOREIGN KEY (transaction_type_id)
    REFERENCES demo_bank.transaction_types (transaction_type_id),
  CONSTRAINT ck_transactions_amount CHECK (amount <> 0),
  CONSTRAINT ck_transactions_fee CHECK (fee_amount >= 0),
  CONSTRAINT ck_transactions_channel CHECK (channel IN ('B', 'A', 'M', 'W', 'C')),
  CONSTRAINT ck_transactions_status CHECK (status IN ('P', 'H', 'R', 'X')),
  CONSTRAINT ck_transactions_reversal CHECK (is_reversal IN ('Y', 'N'))
);
CREATE INDEX ix_transactions_account_date ON demo_bank.transactions (account_id, txn_date);
CREATE INDEX ix_transactions_posted ON demo_bank.transactions (posted_at);
CREATE INDEX ix_transactions_type ON demo_bank.transactions (transaction_type_id, status);

-- ---------------------------------------------------------------------------
-- Payments out: beneficiaries, standing orders, batches
-- ---------------------------------------------------------------------------
CREATE TABLE demo_bank.beneficiaries (
  beneficiary_id bigint      NOT NULL,
  customer_id    bigint      NOT NULL,
  beneficiary_name varchar(80) NOT NULL,
  account_no     varchar(20) NOT NULL,
  bank_code      char(8)     NOT NULL,
  currency_code  char(3)     NOT NULL,
  added_on       date        NOT NULL,
  is_active      char(1)     NOT NULL DEFAULT 'Y',
  CONSTRAINT pk_beneficiaries PRIMARY KEY (beneficiary_id),
  CONSTRAINT fk_beneficiaries_customer FOREIGN KEY (customer_id)
    REFERENCES demo_bank.customers (customer_id),
  CONSTRAINT fk_beneficiaries_currency FOREIGN KEY (currency_code)
    REFERENCES demo_bank.currencies (currency_code),
  CONSTRAINT ck_beneficiaries_active CHECK (is_active IN ('Y', 'N'))
);

CREATE TABLE demo_bank.standing_orders (
  standing_order_id bigint        NOT NULL,
  account_id        bigint        NOT NULL,
  beneficiary_id    bigint        NOT NULL,
  amount            numeric(20,4) NOT NULL,
  frequency         varchar(10)   NOT NULL,
  next_run_date     date          NOT NULL,
  end_date          date,
  is_active         char(1)       NOT NULL DEFAULT 'Y',
  CONSTRAINT pk_standing_orders PRIMARY KEY (standing_order_id),
  CONSTRAINT fk_standing_orders_account FOREIGN KEY (account_id)
    REFERENCES demo_bank.accounts (account_id),
  CONSTRAINT fk_standing_orders_beneficiary FOREIGN KEY (beneficiary_id)
    REFERENCES demo_bank.beneficiaries (beneficiary_id),
  CONSTRAINT ck_standing_orders_frequency CHECK (frequency IN ('DAILY', 'WEEKLY', 'MONTHLY')),
  CONSTRAINT ck_standing_orders_amount CHECK (amount > 0),
  CONSTRAINT ck_standing_orders_active CHECK (is_active IN ('Y', 'N'))
);

CREATE TABLE demo_bank.payment_batches (
  batch_id      bigint        NOT NULL,
  batch_ref     varchar(20)   NOT NULL,
  created_by    varchar(30)   NOT NULL,
  created_at    timestamptz   NOT NULL,
  item_count    integer       NOT NULL,
  total_amount  numeric(20,4) NOT NULL,
  status        char(1)       NOT NULL DEFAULT 'N',
  CONSTRAINT pk_payment_batches PRIMARY KEY (batch_id),
  CONSTRAINT uq_payment_batches_ref UNIQUE (batch_ref),
  CONSTRAINT ck_payment_batches_status CHECK (status IN ('N', 'S', 'C', 'F')),
  CONSTRAINT ck_payment_batches_count CHECK (item_count >= 0)
);

CREATE TABLE demo_bank.payment_items (
  batch_id       bigint        NOT NULL,
  item_no        smallint      NOT NULL,
  beneficiary_id bigint        NOT NULL,
  amount         numeric(20,4) NOT NULL,
  currency_code  char(3)       NOT NULL,
  status         char(1)       NOT NULL DEFAULT 'N',
  CONSTRAINT pk_payment_items PRIMARY KEY (batch_id, item_no),
  CONSTRAINT fk_payment_items_batch FOREIGN KEY (batch_id)
    REFERENCES demo_bank.payment_batches (batch_id),
  CONSTRAINT fk_payment_items_beneficiary FOREIGN KEY (beneficiary_id)
    REFERENCES demo_bank.beneficiaries (beneficiary_id),
  CONSTRAINT ck_payment_items_amount CHECK (amount > 0),
  CONSTRAINT ck_payment_items_status CHECK (status IN ('N', 'S', 'C', 'F'))
);

-- ---------------------------------------------------------------------------
-- Lending
-- ---------------------------------------------------------------------------
CREATE TABLE demo_bank.loans (
  loan_id       bigint        NOT NULL,
  loan_no       varchar(16)   NOT NULL,
  customer_id   bigint        NOT NULL,
  branch_id     integer       NOT NULL,
  product_id    integer       NOT NULL,
  principal     numeric(20,4) NOT NULL,
  outstanding   numeric(20,4) NOT NULL,
  interest_rate float8        NOT NULL,
  disbursed_on  date          NOT NULL,
  maturity_date date          NOT NULL,
  status        char(1)       NOT NULL DEFAULT 'A',
  -- The one money column of the schema (PLAN.md 3.2: money is cast to numeric).
  penalty_charge money        NOT NULL DEFAULT 0,
  created_at    timestamp(0)  NOT NULL,
  CONSTRAINT pk_loans PRIMARY KEY (loan_id),
  CONSTRAINT uq_loans_no UNIQUE (loan_no),
  CONSTRAINT fk_loans_customer FOREIGN KEY (customer_id)
    REFERENCES demo_bank.customers (customer_id),
  CONSTRAINT fk_loans_branch FOREIGN KEY (branch_id)
    REFERENCES demo_bank.branches (branch_id),
  CONSTRAINT fk_loans_product FOREIGN KEY (product_id)
    REFERENCES demo_bank.products (product_id),
  CONSTRAINT ck_loans_status CHECK (status IN ('A', 'C', 'W', 'D')),
  CONSTRAINT ck_loans_amounts CHECK (principal > 0 AND outstanding >= 0),
  CONSTRAINT ck_loans_dates CHECK (maturity_date > disbursed_on),
  CONSTRAINT ck_loans_penalty CHECK (penalty_charge >= 0::money)
);
CREATE INDEX ix_loans_customer ON demo_bank.loans (customer_id, status);

CREATE TABLE demo_bank.loan_schedules (
  loan_id        bigint        NOT NULL,
  installment_no smallint      NOT NULL,
  due_date       date          NOT NULL,
  principal_due  numeric(20,4) NOT NULL,
  interest_due   numeric(20,4) NOT NULL,
  paid_amount    numeric(20,4) NOT NULL DEFAULT 0,
  CONSTRAINT pk_loan_schedules PRIMARY KEY (loan_id, installment_no),
  CONSTRAINT fk_loan_schedules_loan FOREIGN KEY (loan_id)
    REFERENCES demo_bank.loans (loan_id),
  CONSTRAINT ck_loan_schedules_no CHECK (installment_no > 0),
  CONSTRAINT ck_loan_schedules_amounts CHECK (principal_due >= 0 AND interest_due >= 0)
);

CREATE TABLE demo_bank.loan_collateral (
  collateral_id   bigint        NOT NULL,
  loan_id         bigint        NOT NULL,
  collateral_type varchar(20)   NOT NULL,
  description     varchar(120),
  market_value    numeric(20,4) NOT NULL,
  valued_on       date          NOT NULL,
  currency_code   char(3)       NOT NULL,
  CONSTRAINT pk_loan_collateral PRIMARY KEY (collateral_id),
  CONSTRAINT fk_loan_collateral_loan FOREIGN KEY (loan_id)
    REFERENCES demo_bank.loans (loan_id),
  CONSTRAINT fk_loan_collateral_currency FOREIGN KEY (currency_code)
    REFERENCES demo_bank.currencies (currency_code),
  CONSTRAINT ck_loan_collateral_type CHECK (collateral_type IN ('PROPERTY', 'VEHICLE', 'DEPOSIT', 'GUARANTEE')),
  CONSTRAINT ck_loan_collateral_value CHECK (market_value > 0)
);

-- ---------------------------------------------------------------------------
-- Cards
-- ---------------------------------------------------------------------------
CREATE TABLE demo_bank.cards (
  card_id        bigint        NOT NULL,
  card_no        char(16)      NOT NULL,
  account_id     bigint        NOT NULL,
  card_type      varchar(10)   NOT NULL,
  expiry_date    date          NOT NULL,
  credit_limit   numeric(20,4) NOT NULL DEFAULT 0,
  status         char(1)       NOT NULL DEFAULT 'A',
  issued_at      timestamp(0)  NOT NULL,
  is_contactless char(1)       NOT NULL DEFAULT 'Y',
  CONSTRAINT pk_cards PRIMARY KEY (card_id),
  CONSTRAINT uq_cards_no UNIQUE (card_no),
  CONSTRAINT fk_cards_account FOREIGN KEY (account_id)
    REFERENCES demo_bank.accounts (account_id),
  CONSTRAINT ck_cards_type CHECK (card_type IN ('DEBIT', 'CREDIT', 'PREPAID')),
  CONSTRAINT ck_cards_status CHECK (status IN ('A', 'B', 'E', 'X')),
  CONSTRAINT ck_cards_limit CHECK (credit_limit >= 0),
  CONSTRAINT ck_cards_contactless CHECK (is_contactless IN ('Y', 'N'))
);

CREATE TABLE demo_bank.card_transactions (
  card_txn_id  bigint        NOT NULL,
  card_id      bigint        NOT NULL,
  posted_at    timestamptz   NOT NULL,
  merchant     varchar(80)   NOT NULL,
  amount       numeric(20,4) NOT NULL,
  currency_code char(3)      NOT NULL,
  is_declined  char(1)       NOT NULL DEFAULT 'N',
  auth_code    varchar(8),
  CONSTRAINT pk_card_transactions PRIMARY KEY (card_txn_id),
  CONSTRAINT fk_card_transactions_card FOREIGN KEY (card_id)
    REFERENCES demo_bank.cards (card_id),
  CONSTRAINT ck_card_transactions_amount CHECK (amount <> 0),
  CONSTRAINT ck_card_transactions_declined CHECK (is_declined IN ('Y', 'N'))
);
CREATE INDEX ix_card_transactions_card ON demo_bank.card_transactions (card_id, posted_at);

-- ---------------------------------------------------------------------------
-- General ledger, rates and fees
-- ---------------------------------------------------------------------------
CREATE TABLE demo_bank.gl_accounts (
  gl_account_id integer     NOT NULL,
  gl_code       varchar(12) NOT NULL,
  gl_name       varchar(80) NOT NULL,
  account_class char(1)     NOT NULL,
  parent_id     integer,
  is_active     char(1)     NOT NULL DEFAULT 'Y',
  CONSTRAINT pk_gl_accounts PRIMARY KEY (gl_account_id),
  CONSTRAINT uq_gl_accounts_code UNIQUE (gl_code),
  -- Self-referencing foreign key: the ledger is a tree.
  CONSTRAINT fk_gl_accounts_parent FOREIGN KEY (parent_id)
    REFERENCES demo_bank.gl_accounts (gl_account_id),
  CONSTRAINT ck_gl_accounts_class CHECK (account_class IN ('A', 'L', 'E', 'R', 'X')),
  CONSTRAINT ck_gl_accounts_active CHECK (is_active IN ('Y', 'N')),
  CONSTRAINT ck_gl_accounts_self CHECK (parent_id IS NULL OR parent_id <> gl_account_id)
);

CREATE TABLE demo_bank.gl_entries (
  gl_entry_id   bigint        NOT NULL DEFAULT nextval('demo_bank.seq_gl_entry'),
  entry_date    date          NOT NULL,
  gl_account_id integer       NOT NULL,
  branch_id     integer       NOT NULL,
  debit         numeric(20,4) NOT NULL DEFAULT 0,
  credit        numeric(20,4) NOT NULL DEFAULT 0,
  currency_code char(3)       NOT NULL,
  reference     varchar(24),
  created_at    timestamptz   NOT NULL,
  CONSTRAINT pk_gl_entries PRIMARY KEY (gl_entry_id),
  CONSTRAINT fk_gl_entries_account FOREIGN KEY (gl_account_id)
    REFERENCES demo_bank.gl_accounts (gl_account_id),
  CONSTRAINT fk_gl_entries_branch FOREIGN KEY (branch_id)
    REFERENCES demo_bank.branches (branch_id),
  CONSTRAINT ck_gl_entries_sides CHECK (debit >= 0 AND credit >= 0 AND (debit = 0 OR credit = 0)),
  CONSTRAINT ck_gl_entries_nonzero CHECK (debit <> 0 OR credit <> 0)
);
CREATE INDEX ix_gl_entries_date_account ON demo_bank.gl_entries (entry_date, gl_account_id);

CREATE TABLE demo_bank.fx_rates (
  from_currency char(3)     NOT NULL,
  to_currency   char(3)     NOT NULL,
  rate_date     date        NOT NULL,
  rate          numeric(18,8) NOT NULL,
  spread        real,
  source        varchar(20) NOT NULL,
  CONSTRAINT pk_fx_rates PRIMARY KEY (from_currency, to_currency, rate_date),
  CONSTRAINT fk_fx_rates_from FOREIGN KEY (from_currency)
    REFERENCES demo_bank.currencies (currency_code),
  CONSTRAINT fk_fx_rates_to FOREIGN KEY (to_currency)
    REFERENCES demo_bank.currencies (currency_code),
  CONSTRAINT ck_fx_rates_pair CHECK (from_currency <> to_currency),
  CONSTRAINT ck_fx_rates_rate CHECK (rate > 0)
);

CREATE TABLE demo_bank.interest_rates (
  rate_id        integer       NOT NULL,
  product_id     integer       NOT NULL,
  currency_code  char(3)       NOT NULL,
  band_from      numeric(20,4) NOT NULL,
  band_to        numeric(20,4) NOT NULL,
  annual_rate    float8        NOT NULL,
  effective_from date          NOT NULL,
  effective_to   date,
  tier_code      char(2)       NOT NULL,
  CONSTRAINT pk_interest_rates PRIMARY KEY (rate_id),
  CONSTRAINT fk_interest_rates_product FOREIGN KEY (product_id)
    REFERENCES demo_bank.products (product_id),
  CONSTRAINT fk_interest_rates_currency FOREIGN KEY (currency_code)
    REFERENCES demo_bank.currencies (currency_code),
  CONSTRAINT ck_interest_rates_band CHECK (band_to > band_from),
  CONSTRAINT ck_interest_rates_dates CHECK (effective_to IS NULL OR effective_to >= effective_from)
);
CREATE INDEX ix_interest_rates_product ON demo_bank.interest_rates (product_id, effective_from);

CREATE TABLE demo_bank.fees (
  fee_id      integer       NOT NULL,
  fee_code    varchar(8)    NOT NULL,
  fee_name    varchar(80)   NOT NULL,
  fee_type    varchar(10)   NOT NULL,
  flat_amount numeric(20,4) NOT NULL DEFAULT 0,
  pct_rate    float8        NOT NULL DEFAULT 0,
  CONSTRAINT pk_fees PRIMARY KEY (fee_id),
  CONSTRAINT uq_fees_code UNIQUE (fee_code),
  CONSTRAINT ck_fees_type CHECK (fee_type IN ('FLAT', 'PERCENT', 'TIERED')),
  CONSTRAINT ck_fees_amounts CHECK (flat_amount >= 0 AND pct_rate >= 0)
);

CREATE TABLE demo_bank.charges (
  charge_id   bigint        NOT NULL,
  account_id  bigint        NOT NULL,
  fee_id      integer       NOT NULL,
  charged_on  date          NOT NULL,
  amount      numeric(20,4) NOT NULL,
  waive_flag  char(1)       NOT NULL DEFAULT 'N',
  narration   varchar(80),
  CONSTRAINT pk_charges PRIMARY KEY (charge_id),
  CONSTRAINT fk_charges_account FOREIGN KEY (account_id)
    REFERENCES demo_bank.accounts (account_id),
  CONSTRAINT fk_charges_fee FOREIGN KEY (fee_id)
    REFERENCES demo_bank.fees (fee_id),
  CONSTRAINT ck_charges_amount CHECK (amount >= 0),
  CONSTRAINT ck_charges_waive CHECK (waive_flag IN ('Y', 'N'))
);

CREATE TABLE demo_bank.exchange_audit (
  exchange_id     bigint        NOT NULL,
  from_currency   char(3)       NOT NULL,
  to_currency     char(3)       NOT NULL,
  rate            numeric(18,8) NOT NULL,
  converted_amount numeric(20,4) NOT NULL,
  requested_at    timestamptz   NOT NULL,
  status          char(1)       NOT NULL DEFAULT 'C',
  CONSTRAINT pk_exchange_audit PRIMARY KEY (exchange_id),
  CONSTRAINT fk_exchange_audit_from FOREIGN KEY (from_currency)
    REFERENCES demo_bank.currencies (currency_code),
  CONSTRAINT fk_exchange_audit_to FOREIGN KEY (to_currency)
    REFERENCES demo_bank.currencies (currency_code),
  CONSTRAINT ck_exchange_audit_pair CHECK (from_currency <> to_currency),
  CONSTRAINT ck_exchange_audit_status CHECK (status IN ('C', 'F', 'R'))
);

-- ---------------------------------------------------------------------------
-- Statements, compliance, notes, limits
-- ---------------------------------------------------------------------------
CREATE TABLE demo_bank.statements (
  statement_id    bigint        NOT NULL,
  account_id      bigint        NOT NULL,
  period_start    date          NOT NULL,
  period_end      date          NOT NULL,
  opening_balance numeric(20,4) NOT NULL,
  closing_balance numeric(20,4) NOT NULL,
  txn_count       integer       NOT NULL,
  generated_at    timestamp(0)  NOT NULL,
  CONSTRAINT pk_statements PRIMARY KEY (statement_id),
  CONSTRAINT uq_statements_period UNIQUE (account_id, period_start, period_end),
  CONSTRAINT fk_statements_account FOREIGN KEY (account_id)
    REFERENCES demo_bank.accounts (account_id),
  CONSTRAINT ck_statements_period CHECK (period_end >= period_start),
  CONSTRAINT ck_statements_count CHECK (txn_count >= 0)
);

CREATE TABLE demo_bank.kyc_checks (
  check_id    bigint      NOT NULL,
  customer_id bigint      NOT NULL,
  check_type  varchar(20) NOT NULL,
  checked_at  timestamptz NOT NULL,
  result      char(1)     NOT NULL,
  detail      jsonb,
  CONSTRAINT pk_kyc_checks PRIMARY KEY (check_id),
  CONSTRAINT fk_kyc_checks_customer FOREIGN KEY (customer_id)
    REFERENCES demo_bank.customers (customer_id),
  CONSTRAINT ck_kyc_checks_type CHECK (check_type IN ('IDENTITY', 'ADDRESS', 'SANCTIONS', 'PEP')),
  CONSTRAINT ck_kyc_checks_result CHECK (result IN ('P', 'F', 'R'))
);
CREATE INDEX ix_kyc_checks_customer ON demo_bank.kyc_checks (customer_id, checked_at);

CREATE TABLE demo_bank.alerts (
  alert_id    bigint      NOT NULL,
  customer_id bigint      NOT NULL,
  raised_at   timestamptz NOT NULL,
  rule_code   varchar(12) NOT NULL,
  severity    smallint    NOT NULL,
  payload     json,
  CONSTRAINT pk_alerts PRIMARY KEY (alert_id),
  CONSTRAINT fk_alerts_customer FOREIGN KEY (customer_id)
    REFERENCES demo_bank.customers (customer_id),
  CONSTRAINT ck_alerts_severity CHECK (severity BETWEEN 1 AND 5),
  CONSTRAINT ck_alerts_rule CHECK (length(rule_code) >= 3)
);

CREATE TABLE demo_bank.rate_bands (
  band_id        integer     NOT NULL,
  product_id     integer     NOT NULL,
  amount_band    numrange    NOT NULL,
  rate_pct       float8      NOT NULL,
  effective_from date        NOT NULL,
  CONSTRAINT pk_rate_bands PRIMARY KEY (band_id),
  CONSTRAINT fk_rate_bands_product FOREIGN KEY (product_id)
    REFERENCES demo_bank.products (product_id),
  CONSTRAINT ck_rate_bands_rate CHECK (rate_pct >= 0),
  CONSTRAINT ck_rate_bands_bounds CHECK (NOT isempty(amount_band))
);

-- Deliberately has NO primary key and no unique constraint: duplicates are
-- legal, and PLAN.md 3.1 compares it as a counted set.
CREATE TABLE demo_bank.audit_log (
  event_at    timestamptz NOT NULL,
  actor       varchar(40) NOT NULL,
  action      varchar(20) NOT NULL,
  object_name varchar(60) NOT NULL,
  detail      text
);
CREATE INDEX ix_audit_log_event_at ON demo_bank.audit_log (event_at);

CREATE TABLE demo_bank.cheques (
  cheque_id  bigint        NOT NULL,
  account_id bigint        NOT NULL,
  cheque_no  char(10)      NOT NULL,
  issued_on  date          NOT NULL,
  amount     numeric(20,4) NOT NULL,
  status     char(1)       NOT NULL DEFAULT 'I',
  payee      varchar(80),
  cleared_on date,
  CONSTRAINT pk_cheques PRIMARY KEY (cheque_id),
  CONSTRAINT uq_cheques_account_no UNIQUE (account_id, cheque_no),
  CONSTRAINT fk_cheques_account FOREIGN KEY (account_id)
    REFERENCES demo_bank.accounts (account_id),
  CONSTRAINT ck_cheques_status CHECK (status IN ('I', 'C', 'S', 'B')),
  CONSTRAINT ck_cheques_amount CHECK (amount > 0),
  CONSTRAINT ck_cheques_cleared CHECK (cleared_on IS NULL OR cleared_on >= issued_on)
);

CREATE TABLE demo_bank.disputes (
  dispute_id   bigint        NOT NULL DEFAULT nextval('demo_bank.seq_dispute'),
  card_txn_id  bigint        NOT NULL,
  customer_id  bigint        NOT NULL,
  opened_at    timestamptz   NOT NULL,
  reason       varchar(40)   NOT NULL,
  amount       numeric(20,4) NOT NULL,
  status       char(1)       NOT NULL DEFAULT 'O',
  resolved_on  date,
  CONSTRAINT pk_disputes PRIMARY KEY (dispute_id),
  CONSTRAINT uq_disputes_txn UNIQUE (card_txn_id),
  CONSTRAINT fk_disputes_card_txn FOREIGN KEY (card_txn_id)
    REFERENCES demo_bank.card_transactions (card_txn_id),
  CONSTRAINT fk_disputes_customer FOREIGN KEY (customer_id)
    REFERENCES demo_bank.customers (customer_id),
  CONSTRAINT ck_disputes_reason CHECK (reason IN ('FRAUD', 'DUPLICATE', 'NOT_RECEIVED', 'WRONG_AMOUNT')),
  CONSTRAINT ck_disputes_status CHECK (status IN ('O', 'R', 'U', 'C')),
  CONSTRAINT ck_disputes_amount CHECK (amount > 0)
);

-- Free text: carries accented characters, NULLs, empty strings and trailing
-- spaces, so the normalisation and encoding rules get exercised (PLAN.md 3.2).
CREATE TABLE demo_bank.notes (
  note_id     bigint       NOT NULL DEFAULT nextval('demo_bank.seq_note'),
  customer_id bigint,
  account_id  bigint,
  note_text   text         NOT NULL,
  author      varchar(40)  NOT NULL,
  created_at  timestamp(0) NOT NULL,
  CONSTRAINT pk_notes PRIMARY KEY (note_id),
  CONSTRAINT fk_notes_customer FOREIGN KEY (customer_id)
    REFERENCES demo_bank.customers (customer_id),
  CONSTRAINT fk_notes_account FOREIGN KEY (account_id)
    REFERENCES demo_bank.accounts (account_id),
  -- A note hangs off a customer, an account, or both.
  CONSTRAINT ck_notes_owner CHECK (customer_id IS NOT NULL OR account_id IS NOT NULL)
);
CREATE INDEX ix_notes_customer ON demo_bank.notes (customer_id);

CREATE TABLE demo_bank.limits (
  limit_id       integer       NOT NULL,
  product_id     integer       NOT NULL,
  limit_code     varchar(16)   NOT NULL,
  max_amount     numeric(20,4) NOT NULL,
  max_count      integer       NOT NULL,
  currency_code  char(3)       NOT NULL,
  effective_from date          NOT NULL,
  CONSTRAINT pk_limits PRIMARY KEY (limit_id),
  CONSTRAINT uq_limits_product_code UNIQUE (product_id, limit_code),
  CONSTRAINT fk_limits_product FOREIGN KEY (product_id)
    REFERENCES demo_bank.products (product_id),
  CONSTRAINT fk_limits_currency FOREIGN KEY (currency_code)
    REFERENCES demo_bank.currencies (currency_code),
  CONSTRAINT ck_limits_amount CHECK (max_amount > 0),
  CONSTRAINT ck_limits_count CHECK (max_count > 0)
);

-- ---------------------------------------------------------------------------
-- Function and trigger
-- ---------------------------------------------------------------------------
-- Logs balance and status changes on accounts into audit_log.
--
-- Determinism: the timestamp comes from the row's own updated_at, never from
-- now(). A generator that used now() would write a different audit_log on every
-- run and verify.sh could never prove the data is reproducible.
CREATE FUNCTION demo_bank.fn_log_account_change() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.balance IS DISTINCT FROM OLD.balance
     OR NEW.status IS DISTINCT FROM OLD.status THEN
    INSERT INTO demo_bank.audit_log (event_at, actor, action, object_name, detail)
    VALUES (COALESCE(NEW.updated_at, OLD.updated_at, timestamptz '2000-01-01 00:00:00+00'),
            'trigger',
            'ACCOUNT_UPDATE',
            'accounts',
            'account ' || NEW.account_no
              || ' balance ' || OLD.balance::text || ' -> ' || NEW.balance::text);
  END IF;
  RETURN NEW;
END;
$$;

CREATE TRIGGER trg_accounts_audit
  AFTER UPDATE ON demo_bank.accounts
  FOR EACH ROW EXECUTE PROCEDURE demo_bank.fn_log_account_change();

-- ---------------------------------------------------------------------------
-- Views (2) and a materialised view (1)
-- ---------------------------------------------------------------------------
CREATE VIEW demo_bank.v_account_balances AS
SELECT a.account_id,
       a.account_no,
       c.customer_no,
       c.last_name,
       b.branch_code,
       a.currency_code,
       a.balance,
       a.available_balance,
       a.status
FROM demo_bank.accounts a
JOIN demo_bank.customers c ON c.customer_id = a.customer_id
JOIN demo_bank.branches b ON b.branch_id = a.branch_id;

CREATE VIEW demo_bank.v_open_disputes AS
SELECT d.dispute_id,
       d.opened_at,
       d.reason,
       d.amount,
       ct.card_id,
       ct.merchant,
       cu.customer_no
FROM demo_bank.disputes d
JOIN demo_bank.card_transactions ct ON ct.card_txn_id = d.card_txn_id
JOIN demo_bank.customers cu ON cu.customer_id = d.customer_id
WHERE d.status = 'O';

-- Refreshed at the end of data.sql, once the accounts exist.
CREATE MATERIALIZED VIEW demo_bank.mv_branch_totals AS
SELECT b.branch_id,
       b.branch_code,
       count(a.account_id) AS account_count,
       coalesce(sum(a.balance), 0)::numeric(20,4) AS total_balance
FROM demo_bank.branches b
LEFT JOIN demo_bank.accounts a ON a.branch_id = b.branch_id
GROUP BY b.branch_id, b.branch_code;
CREATE UNIQUE INDEX ux_mv_branch_totals ON demo_bank.mv_branch_totals (branch_id);
