# edge_bank: a playground for trying Dmux by hand

Two small databases full of variety: some differences Dmux must ignore, some it
must find, and some that only a person can judge. Use it to learn what each Dmux
verdict looks like before you point Dmux at a real migration.

Everything here is invented. The databases are local throwaway containers bound to
127.0.0.1. Do not point this folder at a real system.

## 1. Build it

Start the local servers (Podman) if they are not running:

```
podman compose -f testdata/compose.yml up -d
```

Then build the playground. It takes about three minutes, most of it the 3 million row table:

```
testdata/edge_bank/build.sh
```

You can run it again any time: it drops and recreates only the database `edge_bank`
on both servers (never `bank`, `demo_bank`, `demo_bank_large` or `rls_bank`).

## 2. What to type into Dmux

| | Old database | New database |
|---|---|---|
| Server | Postgres 9.6, text encoding LATIN1 | Postgres 17, text encoding UTF-8 |
| Host | `127.0.0.1` | `127.0.0.1` |
| Port | `5433` | `5434` |
| Database | `edge_bank` | `edge_bank` |
| User | `edge_reader` | `edge_reader` |
| Password | `edge_reader_only` | `edge_reader_only` |
| Encryption | no encryption (type "local throwaway test server" as the reason) | same |

`edge_reader_only` is a throwaway password for these local containers. It is only
ever sent to 127.0.0.1. `edge_reader` can only read: it has SELECT and nothing else.
On both servers it also has BYPASSRLS, which a login needs to read tables that use
row-level security (see "Two logins" below).

## 3. Run it

**In the desktop app.** Connect old and new with the details above, choose
`Compare structure`, accept the suggested pairs, choose the checks (leave
"Full values" on), and start the run. The tables in the `big` schema take a few
minutes: use them to see the estimate, the progress bar and the time remaining.
To make a quicker run, set the `big` table to "Counts and totals" on the checks screen.

**From the command line.** `run.toml` reads the password from the environment, so it
never sits in a file:

```
export EDGE_READER_PASSWORD=edge_reader_only
cargo run -p dmux-cli -- run testdata/edge_bank/run.toml --out /tmp/edge-results.json
cargo run -p dmux-cli -- report --results /tmp/edge-results.json --pdf /tmp/edge.pdf
```

| File | What it does |
|---|---|
| `run.toml` | Everything, including a full read of the 3 million row table (about 4 minutes). |
| `run-quick.toml` | The same, but the big table is only counted. About a minute. |
| `run-options.toml` | Quick, and turns on "ignore trailing spaces" for `differs.trailing_spaces`. |
| `run-rls.toml` | Quick, but signs in as `edge_reader_rls`, which cannot see row-protected rows. |

The run ends with the overall verdict FAIL, on purpose: the playground is full of
planted differences.

## 4. Every case

"Table says" is the verdict next to the table on the Results screen. Some cases also
produce a "structure" note (about types, columns, indexes, sequences). Both are shown.
Amber (WARNING) notes about encoding, collation and the `ltree` extension appear at
the top of every run and are expected: the two servers really do differ there.

### `same`: must stay quiet

Every table here should be PASS: the data is the same, only its form differs.

| Table | What was done | Dmux should show |
|---|---|---|
| `same.accented_names` | Names with accents (José, Søren, Ångström, Zürich). LATIN1 on the old server, UTF-8 on the new. | PASS |
| `same.float_values` | Decimal numbers (real, double, arrays of them, NaN, Infinity, extreme sizes). Postgres 9.6 and 17 print floats differently. | PASS |
| `same.null_vs_empty` | NULL, empty text and a lone space, mixed, and kept the same on both sides. | PASS (NULL and empty are kept apart, and they match) |
| `same.padded_chars` | `char(8)` and `char(3)` columns, padded with spaces. | PASS |
| `same.shuffled_rows` | The new side stores the rows in the opposite physical order. | PASS |
| `same.reordered_columns` | The new side lists the columns in another order (and rows backwards). | PASS |
| `same.ledger_partitioned` | A plain table on the old side, a partitioned table (two half-year partitions) on the new side. | PASS, plus an amber note "partition list changed" |
| `same.flag_char_to_varchar` | Column changed from `char(1)` to `varchar(1)`. | PASS, plus an amber note "type: character(1) -> character varying(1)" |
| `same.numeric_scale_raised` | `numeric(12,2)` became `numeric(12,4)`. | PASS, plus an amber type note |
| `same.json_kept` | `json` stays `json`, with odd spacing and key order. | PASS |
| `same.rich_types` | Arrays, ranges, an enum, a domain, bytea, time zones, intervals, a uuid, inet and cidr addresses, booleans, money. | PASS |
| `same.keyless_with_duplicates` | No primary key, and many identical rows. The new side keeps every duplicate, stored in another order. | PASS |

### `differs`: Dmux must find these

| Table | What was done (on the new side) | Dmux should show |
|---|---|---|
| `differs.missing_rows` | 3 rows deleted (ids 17, 118, 250). | FAIL: 297 rows against 300, 3 rows missing |
| `differs.extra_rows` | 2 rows added (ids 9001, 9002). | FAIL: 302 rows against 300, 2 extra rows |
| `differs.wide_one_value` | A table with 31 columns; one value in one row changed (column `c17`). | FAIL: 1 changed row, column `c17` named |
| `differs.balances` | One balance is higher by 0.01 (account 123). | FAIL: 1 changed row in column `balance`; the control total of `balance` differs by 0.01 |
| `differs.empty_to_null` | Three empty texts became NULL. | FAIL, with the plain words "an empty text became NULL in column middle_name" |
| `differs.time_dropped` | Three timestamps lost their time of day (kept the date, now midnight). | FAIL: 3 changed rows in `opened_at` |
| `differs.keyless_duplicate_removed` | A table with no key and repeated rows; one copy of one row is gone. | FAIL: 99 rows against 100, one row missing |
| `differs.trailing_spaces` | Two spaces added to the end of 5 names. | FAIL with `run.toml`. With `run-options.toml` (ignore trailing spaces on `name`) it becomes PASS (with options). |
| `differs.json_to_jsonb` | Column changed from `json` to `jsonb`, which reformats the text. | FAIL, plus a FAIL note "type: json -> jsonb" |
| `differs.varchar_widened` | `varchar(20)` widened to `varchar(40)`. | Table PASS, plus an amber note about the type |
| `differs.widely_different` | 40 percent of the 20,000 rows have a different amount. | WIDELY DIFFERENT (more than 10 percent of the buckets differ) |

### `structure`: the shape changed

| Table or object | What was done (on the new side) | Dmux should show |
|---|---|---|
| `structure.dropped_column` | Column `legacy_flag` dropped. | FAIL: "column missing in new: legacy_flag" |
| `structure.added_column` | Column `risk_rating` added. | NEEDS REVIEW: the new column exists only in the new database and its values are not checked; amber note "column added in new" |
| `structure.renamed_column` | Column `phone` renamed `phone_number`. | FAIL ("phone has no counterpart") and amber "column added in new: phone_number"; the mapping screen suggests `phone` -> `phone_number` |
| `structure.client_notes` | Table renamed `structure.customer_notes`. | `client_notes` FAIL "table missing in new", `customer_notes` WARNING "table added in new"; the mapping screen suggests `client_notes` -> `customer_notes` |
| `structure.invoice_seq_behind` | A sequence at 5000 on the old side, at 4200 on the new. | FAIL: "sequence last_value regressed: 5000 -> 4200" |
| `structure.receipt_seq_dropped` | A sequence that was not migrated. | FAIL: "sequence missing in new" |
| `structure.missing_index` | The index `missing_index_email_idx` was not created. | Table PASS (the rows match) plus a FAIL note "index missing in new" |
| `structure.not_valid_constraint` | A check constraint exists but was added NOT VALID (never checked against the existing rows). | Table PASS plus a FAIL note "constraint amount_not_negative is not validated in new" |
| Large objects (database level) | Three exist on the old side; one is missing on the new side. | FAIL: "large object count changed: 3 -> 2". Dmux counts large objects but does not compare their contents. |

### `tricky`

| Table | What was done | Dmux should show |
|---|---|---|
| `tricky.unknown_type` | A column of type `ltree` (from an extension). Identical on both sides. | NEEDS REVIEW: "a column was compared by an inexact printed form" |
| `tricky.rls_branch_notes` | Row-level security: a login without BYPASSRLS sees only branch 1 (rows 1 to 10). Row 20 (branch 2, hidden) changed on the new side. | With `edge_reader` (BYPASSRLS): FAIL, 1 changed row. With `edge_reader_rls`: TOOL FAULT, see below. |
| `tricky.big_ids` and `tricky.big_ids_seq` | The largest possible bigint (9223372036854775807) as a key and as a sequence value, identical on both sides. | PASS, no note (large numbers survive exactly) |
| `tricky.big_ids_seq_off_by_one` | A sequence at the largest bigint on the old side and one lower on the new. | FAIL: "last_value regressed: 9223372036854775807 -> 9223372036854775806" |

### `big`: to watch the estimate

| Table | What was done | Dmux should show |
|---|---|---|
| `big.ledger_entries` | 3 million rows, identical on both sides. | PASS. Before the run Dmux gives an estimate (a few minutes); during it you see progress and time remaining. |

## 5. Two logins: row-level security

`edge_reader` has BYPASSRLS, so it sees every row and `tricky.rls_branch_notes`
gives an honest FAIL. `edge_reader_rls` has the same SELECT rights but not BYPASSRLS.
A login that cannot see all rows must never give a PASS, so Dmux refuses to compare
that table. To see it:

```
cargo run -p dmux-cli -- run testdata/edge_bank/run-rls.toml --out /tmp/edge-rls.json
```

or type `edge_reader_rls` (same password) as the user in the desktop app. Expect a
TOOL FAULT for that table and for the whole run (every other table still shows its own verdict), with the message "the Dmux login is not allowed to read this table: grant SELECT, and BYPASSRLS if the table uses row-level security".

## 6. Check it automatically

```
testdata/edge_bank/check.sh              # quick run, compares every row of the table above
EDGE_FULL=1 testdata/edge_bank/check.sh  # also reads all 3 million rows
```

It prints one line per case and "N passed, 0 failed". It skips (and exits 0) when
the databases are not there, and does nothing in CI unless `DMUX_EDGE_CHECK=1` is set.
The expected verdicts live in `cases.tsv`.

## 7. What is in the folder

| File | Purpose |
|---|---|
| `build.sh` | Builds both databases, plants the differences, creates the logins. |
| `old.sql` | Creates and fills the old database (valid on Postgres 9.4 and up). |
| `new.sql` | Plants the differences in the new database (Postgres 10 and up). |
| `grants.sql` | Gives the read-only logins SELECT. |
| `run*.toml` | Dmux run definitions for the command line. |
| `check.sh`, `cases.tsv` | The automatic check. |
