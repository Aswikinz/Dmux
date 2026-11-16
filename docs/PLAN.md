# Dmux – Project Plan

Version of 16 November 2026. The owner approves changes to this file; agents propose them through Maya.

Dmux is a desktop app that proves a PostgreSQL migration copied every row and value, built for people who do not write SQL. This plan was rewritten after a full review: the comparison method, the testing approach and the build order all changed to close the risks listed in section 2.

## 1. What Dmux is, and what a PASS means

Dmux connects read-only to an old and a new PostgreSQL database, compares their structure and data, and produces a report a bank can sign. It started with a core banking system upgrade, from Postgres 9.x to a current release, but the product holds nothing specific to that system.

**Three levels of assurance.** The user picks a level per table, and the report always says which level each table got.

| Level | What is compared | What a PASS proves | Cost |
| --- | --- | --- | --- |
| Structure | Tables, columns, types, keys, indexes, constraints, sequences, views, functions | Nothing was left behind or changed shape | Seconds |
| Counts and totals | Row count per table, plus sums of chosen number columns such as balances | The same number of rows arrived, and the money adds up the same | One scan per table |
| Full values | A fingerprint of every value in every mapped column | Every row and every value is identical | One slower scan per table |

**The verdict rule.** The overall result is PASS only when every in-scope table passed at its chosen level, no table changed during the check, and nothing was skipped without a recorded reason. The first page of every report carries a coverage line, for example "1,480 of 1,529 tables fully checked, 40 counts only, 9 ignored".

**What a PASS does not prove.** It does not prove the application works on the new database, that the new database is fast, or that data which was already wrong before the migration is now right. The report says this in plain words.

**In version 1.**

- PostgreSQL to PostgreSQL, any version from 9.4 up
- Live comparison of two databases, and comparison against a saved fingerprint file
- Desktop app for Windows and Linux, plus a command-line runner for servers
- PDF and Excel reports, run history and logs
- Python hooks, built last

**Later.** Other database engines, macOS, column-level fingerprints, built-in SSH tunnels and a Flathub listing.

## 2. Challenges and how the plan handles them

A verification tool has one unforgivable failure: saying PASS when the data differs. Most rows below exist to prevent that, or to keep the tool safe on a production banking database.

### 2.1 Getting the comparison right

| Challenge | What goes wrong | How Dmux handles it |
| --- | --- | --- |
| Sort order differs between servers | Old and new servers sort text differently, so "rows 1 to 50,000" holds different rows on each side and identical data looks broken | The fingerprint does not depend on row order at all (section 3). The earlier "chunks ordered by key" design is dropped |
| Same value, different printed form | Postgres 9.x and 12+ print decimal fractions differently; dates, time zones and binary data depend on settings | Both sides get the same fixed settings; floats and binary data are compared by their bytes, not their printed form |
| Different text encoding | An old LATIN1 database hashes "é" differently from a UTF-8 one, so every accented name looks changed | Every value is converted to UTF-8 before hashing; SQL\_ASCII databases are flagged for review |
| Empty versus missing, and padding | NULL, empty text and trailing spaces can hide a real change or fake one | NULL never equals empty; padding differences are reported, with a recorded option to ignore them |
| Tables with no key, and duplicate rows | No key to line rows up; one missing duplicate goes unnoticed | Rows are compared as a counted set, so a missing duplicate is caught |
| Data changes while checking | On a live database, counts never match and results mean nothing | Each table is read in one consistent snapshot; change counters are read before and after; a table that moved is marked "changed during check", never PASS |
| Both databases are not online together | With an in-place upgrade the old server stops before the new one starts, or the two sit in different network zones | Fingerprints can be saved to a file from one side and compared later (section 3) |
| Things that are not table rows | Sequences left behind, missing indexes, unvalidated constraints, lost large objects, missing statistics | The inventory check covers each of these (section 4.2) |
| Wrong automatic mapping | A "close enough" name match pairs the wrong columns | Only exact name matches are accepted automatically; every other match needs a human tick |
| Partitioned tables | The new database splits one table into partitions, which look like many new tables | Partitions are folded under their parent on both sides |
| A bug in Dmux itself | A column silently left out of the hash gives a false PASS | The report lists the columns covered per table; a planted-difference answer key, mutation tests and an independent spot check guard the core (section 8) |

### 2.2 Staying safe on a production database

| Challenge | What goes wrong | How Dmux handles it |
| --- | --- | --- |
| "Read-only" is not a full guarantee | A read-only transaction can still call harmful functions, and a superuser login can do anything | A dedicated login with SELECT rights only; Dmux warns when the login can write and refuses superuser logins by default; Dmux writes every query itself |
| Heavy scans slow the bank | Reading every row of a billion-row table competes with business | A limit on parallel scans per database, a run window, an estimate before starting, and a Cancel that really stops the server queries |
| Long reads on a live system | They delay database clean-up and can queue behind schema changes | One short transaction per table, timeouts on statements and locks, and guidance to run on a frozen source or a replica |
| Connection poolers | PgBouncer drops session settings between statements | All settings are applied inside each transaction; a direct connection is recommended and poolers are detected |
| Hardened servers block md5 | On FIPS-mode servers the hash function fails | Detected at connection time; Dmux falls back to hashing in the app, which is slower |

### 2.3 Fitting into a bank

| Challenge | What goes wrong | How Dmux handles it |
| --- | --- | --- |
| The laptop cannot reach the database | Production databases sit behind jump servers | A command-line runner with the same engine runs on the jump server; its results file opens in the desktop app |
| Locked-down PCs | Unsigned installers are blocked, users have no admin rights, machines are offline | Signed builds, a per-user installer, a portable build, an offline installer, no auto-update and no telemetry |
| Old servers and odd logins | Self-signed certificates, old encryption, Kerberos | Dmux shows the certificate fingerprint to trust once; an unencrypted connection needs a visible choice that is recorded in the report; Kerberos login is not supported in version 1 |
| Remote desktops and weak graphics | Blur effects stutter over remote desktop and on Linux | A "reduced effects" theme with solid panels switches on automatically when blur is slow |
| Runs that take hours | The laptop sleeps, the VPN drops, the app is closed | Results are saved per table as they finish; a run resumes where it stopped; sleep is blocked during a run |
| Sensitive data in reports | Account numbers appear in the list of differing rows | Row values are shown on screen only; reports include keys only when ticked; local history can be purged |

### 2.4 Trust and audit

| Challenge | What goes wrong | How Dmux handles it |
| --- | --- | --- |
| A PASS that hides gaps | 200 ignored tables and a green tick | The coverage line and every ignored item, with its reason, sit on the report's first page |
| Can the report be trusted later | A PDF can be edited | The report carries the run ID, tool version, mapping hash and a SHA-256 of the results file, and the run can be repeated from its saved definition. This is tamper-evident, not tamper-proof, and the report says so |
| Nobody on the team can vouch for the code | The team is non-technical | The comparison core stays small and is explained in a plain-language "How Dmux decides" document; an independent database administrator reviews the method before first real use; the first real use runs alongside the existing manual checks |
| Wrong time estimates | First run on unknown hardware | Estimates are shown as a range and corrected as each table finishes |

### 2.5 Building it with AI agents

| Challenge | What goes wrong | How the plan handles it |
| --- | --- | --- |
| The method fails late | Cross-version problems appear after the UI is built | Phase 0 proves the fingerprint on real Postgres 9.4 to 17 servers before any product code |
| Cheaper models invent APIs | Code that looks right and does not build | CI is the judge of "works"; risky pieces are proven small first; the comparison core uses the stronger model |
| Tests weakened to pass | An agent edits the test instead of fixing the bug | Only one agent may change the answer key; CI rejects a change that touches both product code and the key |
| Agents clash or loop | Parallel edits collide, costs run away | One working copy per task, one merge at a time, two fix rounds at most, a spending cap |
| The plan is too big | Everything is half built | Phases with a "done when" test each, and a fixed version 1 list (section 1) |

## 3. How a comparison works

Dmux asks each database for a small fingerprint of every table, compares the fingerprints, and fetches rows only where they differ.

```
Old database --(read-only scan)--> Old fingerprint --+
                                                     +--> Compare bucket by bucket --> every bucket equal --> PASS
New database --(read-only scan)--> New fingerprint --+                             \-> some differ --> Drill down (only those buckets)
                                         |                                                              --> Differences found: missing or
                                         v (optional)                                                       extra rows, changed values by column
                                  Fingerprint file (saved now, compared later)
```

Rows stay inside each database unless their bucket differs. Either fingerprint can be saved to a file, so the two databases need not be online at the same time.

### 3.1 The fingerprint, step by step

1. **Open.** One transaction per table per side, `REPEATABLE READ READ ONLY`, with fixed settings applied by `SET LOCAL`: UTC time zone, ISO dates, hex binary output, a statement timeout and a 5-second lock timeout.
2. **Hash each row.** The mapped columns are normalised (3.2), encoded so that NULL and empty differ, converted to UTF-8 and hashed with `md5`. The key columns are hashed the same way into a key hash.
3. **Group into buckets.** Rows are grouped by the first digits of the key hash. The default is 4,096 buckets, raised for huge tables to keep about 10,000 rows per bucket.
4. **Summarise each bucket.** The database returns a row count and four sums, each over one 32-bit slice of the row hashes. Sums do not depend on row order, and unlike XOR they count duplicate rows.
5. **Compare.** Only the summaries leave the database: a few thousand lines per table, whatever its size. Equal summaries mean equal values.
6. **Drill down.** For buckets that differ, a second scan returns the key and row hash of just those rows. Dmux lists rows that are missing, extra or changed. For up to 100 changed rows per table it fetches both versions and shows which columns differ.
7. **Stop early.** When more than 10% of buckets differ, the table is reported as "widely different" with a sample, and the full drill-down is skipped.

Row counts and control totals come from the same scan, so "counts plus full values" costs one pass. Tables with no key are bucketed by the row hash itself and compared as a counted set.

**Huge tables** are split into ranges of a date or number key column, such as a transaction date, so the ranges run in parallel and show progress. Dmux never splits by text ranges, because text order differs between servers.

### 3.2 Normalisation rules

The same rule runs on both sides, and every rule is covered by the cross-version test in section 8.

| Data type | Rule |
| --- | --- |
| text, varchar | Exact, as UTF-8 bytes |
| char(n) | Pad spaces at the end ignored |
| numeric | Printed form; when the mapping changes the scale, both sides are cast to the larger scale |
| real, double precision | The 8-byte binary form, never the printed form |
| date, timestamp, timestamptz | ISO format, in UTC |
| bytea | Hashed as raw bytes |
| money | Cast to numeric |
| json, jsonb, xml, arrays, ranges | Printed form; a json to jsonb change is flagged, because jsonb reorders keys |
| Extension and unknown types | Printed form, with a warning in the report |
| NULL | Its own marker, never equal to an empty value |

The user may tick four recorded options per column: ignore trailing spaces, treat NULL and empty as the same, compare the date only, or round to a set number of decimals. Each ticked option is listed in the report. Free-form SQL is not accepted.

### 3.3 Saved fingerprints

`capture` writes one database's inventory and bucket summaries to a fingerprint file. `compare` takes two files, or one file and a live database.

- This covers in-place upgrades, where the old server is stopped before the new one starts, and networks where one machine cannot reach both databases.
- The mapping must be fixed at capture time. An in-place upgrade keeps every name, so the identity mapping works.
- Without a live old database, Dmux reports which tables and how many buckets differ, but cannot list the rows.

### 3.4 Consistency and the independent spot check

- **Change detection.** Before and after each table, Dmux reads that table's insert, update and delete counters. If they moved, the result is "changed during check". The write-ahead log position is recorded at the start and end of every run.
- **Spot check.** For each keyed table, Dmux fetches 200 rows by key from both sides and compares them value by value in the app, without using the hash path. A disagreement between the spot check and the fingerprint is reported as a tool fault, not a data result.

## 4. Features

### 4.1 Connections

- Saved profiles with host, port, database, user and encryption mode, each marked **Old database** or **New database**.
- **Test connection** reports the server version, text encoding, encryption state, whether the login can write or is a superuser, whether a pooler sits in between, and whether `md5` works.
- Passwords live in the operating system's keychain. Where no keychain exists, Dmux asks for the password at each run.
- The setup guide gives administrators a short script that creates a `dmux_reader` login with SELECT rights only.

### 4.2 Inventory and schema diff

- A side-by-side tree of schemas, tables and columns, in the style of DataGrip's diff. Red means missing in the new database, green means added, amber means changed.
- Columns are matched by name, never by position. Partitions are folded under their parent table.
- The inventory covers:
  - tables and columns, with type, length, nullability and default
  - primary, unique, foreign-key and check constraints, including whether each is validated
  - indexes, including whether each is valid
  - sequences, where the new value must be at least the old one
  - views, materialised views, functions, triggers and extensions
  - the count of large objects, and each database's encoding and collation
- View and function definitions are compared by existence and by text. A text difference is amber, "review", because Postgres versions print definitions differently.
- Warnings are raised for old tables stored WITH OIDS, column types removed in newer Postgres, and new tables with no statistics yet, which means ANALYZE has not run.
- A filter shows only problems, and search works across thousands of tables.

### 4.3 Mapping

- Rules come first: old schema to new schema, and prefix or suffix renames. One rule can map hundreds of tables.
- Then names are matched exactly, then ignoring case. When ignoring case is ambiguous, Dmux asks.
- Fuzzy matches are only suggestions, shown with a confidence score, and each needs a tick.
- The user works through a "needs attention" queue, not a thousand-row board. A picker or drag and drop fixes a single match.
- A table or column can be marked **Ignore** with a required reason, which goes into the report.
- Mappings are saved, versioned and exported as JSON. The mapping's hash is printed in the report.

### 4.4 Choosing checks

- Each table gets a level: Structure, Counts and totals, or Full values.
- Presets cover the common cases, such as "full values for everything" or "counts for all, full values for selected tables".
- Tables can be selected by schema, name pattern or size.
- Dmux suggests control-total columns, such as number columns whose names contain `amt` or `bal`, and the user confirms them.

### 4.5 Time estimates

- Estimates use each table's size on disk, not its row estimate, because a freshly loaded database has no statistics.
- A short timed sample measures hashing speed on each server. The sample uses a row limit, which works on every Postgres version.
- Each estimate is shown as a range, per table and in total, and is corrected as tables finish.
- The estimate notes that a failing table costs a second pass for the drill-down.

### 4.6 Running

- Parallel scans are limited per database: 2 on the old side and 4 on the new side by default.
- A run window can be set, such as 22:00 to 05:00. Dmux pauses outside the window.
- Pause and Cancel stop the queries on the server, not just in the app.
- Results are saved per table as they finish, so a run resumes after a crash, a dropped VPN or a restart.
- Every query runs under the application name `dmux`, so administrators can see it.

### 4.7 Results and report

- The overall verdict and coverage line come first, then tables with failures at the top.
- Each failure card says what is wrong, how big the gap is and what to do next, for example "The new database has 12 fewer rows than the old one in the transaction history table".
- The drill-down view lists missing, extra and changed rows, with the differing columns side by side.
- Exports: a PDF summary with a sign-off block, and an Excel file with the detail. Lists longer than Excel's 1,048,576-row limit go to CSV.
- The run log records the user, time, tool version, profiles, mapping version, options ticked and every query sent.

### 4.8 Command-line runner

- `dmux capture`, `dmux compare` and `dmux run <definition file>` use the same engine as the app.
- Exit codes are 0 for pass, 1 for fail and 2 for warnings, so runs can be scripted.
- A results file written on a jump server opens in the desktop app for review and reporting.

### 4.9 Python hooks

- A script receives the run's results as JSON and returns extra findings, which appear in the report marked as custom checks.
- Scripts never receive database passwords or connections.
- Hooks are off by default. Each script's hash is recorded in the run log, and scripts run with a time limit.
- The Windows build ships its own Python, and the Flatpak uses the Python in its runtime.

## 5. Tech stack and repo layout

The engine is a plain Rust library with no user-interface code, so the desktop app, the command-line runner and the tests all use the same comparison logic.

| Part | Choice | Why |
| --- | --- | --- |
| Desktop shell | Tauri 2 | Small installers; a Rust core with a web UI. Uses WebView2 on Windows and WebKitGTK on Linux |
| Engine | Rust, `tokio`, `tokio-postgres`, `deadpool-postgres` | Fast, streams results, speaks to every Postgres version from 9.4 |
| Encryption | `rustls`, with an OpenSSL fallback | `rustls` needs TLS 1.2; the fallback covers very old servers |
| Frontend | React, TypeScript, Vite, Tailwind, TanStack Table and Virtual | Smooth lists of thousands of tables |
| Local store | SQLite, bundled | Profiles, mappings, run history and logs in one file |
| Secrets | The `keyring` crate | Windows Credential Manager, or the Secret Service on Linux |
| Reports | PDF generated in Rust; Excel through `rust_xlsxwriter` | No dependence on browser printing |
| Test databases | Podman, rootless, with `podman compose` | Postgres 9.6 and 17 containers bound to localhost only |

**Repo layout.**

```
crates/dmux-engine/   connections, inventory, mapping, estimate; src/core/ holds normalise, fingerprint, compare, drilldown
crates/dmux-cli/      capture, compare, run
src-tauri/            thin Tauri commands over the engine
src/                  React UI: design/, components/, screens/
testdata/             compose file, demo_bank generator, expected_diffs.json
private/              git-ignored: licensed data dictionary, its parsed file, the private-shaped generator
docs/                 PLAN.md, TASKS.md, DECISIONS.md, user guide, "How Dmux decides"
.opencode/agents/     one file per agent
```

The four modules `normalise`, `fingerprint`, `compare` and `drilldown` are the **comparison core**. They get the strictest rules in sections 8 and 10.

## 6. Safety and security rules

These rules bind every agent and every release. The security reviewer checks each change against them.

1. **Read-only by construction.** Every statement runs inside `BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY`. The engine contains no statement that writes, creates objects or takes more than a read lock.
2. **Least privilege.** Dmux expects a SELECT-only login, warns when the login can write, and refuses a superuser login unless the user overrides it with a recorded reason.
3. **No injected SQL.** Every table and column name passes through one quoting helper, and every value is a bound parameter. Mapping files, fingerprint files and script output are treated as untrusted input.
4. **Gentle on the server.** Statement timeout, 5-second lock timeout, a parallel-scan limit and working Cancel.
5. **Secrets stay secret.** Passwords are never written to SQLite, logs, reports, error messages or the UI's state.
6. **No row data leaves the machine.** Logs and reports hold counts, hashes and names. Row values appear on screen only, and keys are exported only when the user ticks the option.
7. **Encryption is visible.** Dmux tries a verified encrypted connection first. An untrusted certificate is shown by fingerprint for the user to trust once. An unencrypted connection needs an explicit choice, which is recorded in the report.
8. **No network calls** except to the configured databases. No telemetry and no auto-update.
9. **Tight app permissions.** Tauri capabilities allow only the commands the UI needs, with a strict content security policy and no remote content.
10. **Verifiable releases.** Signed installers, SHA-256 checksums, a software bill of materials, and build actions pinned to exact commits.
11. **Private material stays private.** The licensed data dictionary and anything generated from it live in `private/` and never enter Git.

The repo carries a `SECURITY.md` that explains how to report a vulnerability privately.

## 7. UI and design

The app is a guided, step-by-step flow, so a non-technical user always knows where they are, what to do next and whether it worked.

**Screen flow.** A sidebar shows the steps, each with a tick when done:

1. **Connect.** Pick or add the old and new databases, and test them.
2. **Compare structure.** The side-by-side inventory and schema diff.
3. **Map.** Work through the "needs attention" queue.
4. **Choose checks.** Pick levels and tables, with a live time estimate.
5. **Run.** Progress per table, time remaining, Pause and Cancel.
6. **Results.** Verdict and coverage first, failures at the top.
7. **Report.** Export the PDF and Excel files.

Run history, scripts and settings sit below the steps.

**Design rules.**

- Glassmorphic look: frosted panels with a 16px blur over a soft gradient, and a thin light border.
- Panels stay at least 70% opaque behind text, so contrast meets WCAG AA (4.5:1) in light and dark themes.
- **Reduced effects theme.** Solid panels with the same layout. It switches on automatically when blur renders slowly, such as over remote desktop, and the design must look finished in this theme too.
- Status never relies on colour alone. Every PASS, FAIL, WARNING and "changed during check" carries an icon and a word.
- Plain words everywhere: "old database" and "new database", never "source DSN". Any technical term gets a one-line explanation on hover.
- Scale first. Every list is virtualised, searchable and filterable, and every bulk action works on a selection.
- The whole flow works by keyboard, including mapping.

The frontend agent builds the component library first: panel, button, status badge, step sidebar, data table, diff tree, mapping queue, progress row and error card. Every screen reuses it.

## 8. Testing: verifying the verifier

Dmux must be tested harder than the migration it checks, so the comparison core is covered by seven layers of tests.

| Layer | What it proves | How |
| --- | --- | --- |
| Unit and property tests | Each normalisation and compare rule is right for random data; any random change to a row is always detected | `cargo test` with `proptest` |
| Cross-version identity | The same data loaded into Postgres 9.4, 9.6, 12, 15, 17 and the latest release gives the same fingerprint | Nightly, on real servers in containers |
| Answer key | Every planted difference is found, with no false alarms | The command-line runner against the mock databases, compared with `expected_diffs.json` |
| Mutation testing | When the comparison code is deliberately broken, the tests notice | `cargo-mutants` on the comparison core, nightly |
| UI tests | Screens behave correctly for every result type | Playwright against the UI in a browser, with the engine mocked |
| End-to-end smoke | The real installed app completes a run | `tauri-driver` with WebdriverIO, on Linux and Windows build servers |
| Human test | A non-technical person can finish a run | A teammate with only the user guide |

The earlier plan had Playwright drive the built app. That does not work on Linux, where Tauri uses WebKitGTK, so the plan now splits UI tests from a small end-to-end smoke test.

### 8.1 Mock databases

- Podman runs a Postgres 9.6 old database and a Postgres 17 new database, both reachable from the laptop only.
- **Public schema, `demo_bank`:** an invented banking layout of about 40 tables. Public tests use only this.
- **Private schema:** generated from a core banking system's licensed data dictionary, already parsed into well over a thousand tables and tens of thousands of columns. Oracle-style types are mapped to Postgres: `varchar(n char)` to `varchar(n)`, `number(p,s)` to `numeric(p,s)`, `date` to `timestamp(0)`, `clob` to `text`, `blob` to `bytea`.
- Data is synthetic, built from a fixed seed, and respects keys and foreign keys. The old database is copied to the new one with `pg_dump` and `pg_restore`, then the differences are planted.
- Three sizes: **small** (about 40 tables, thousands of rows, used on every pull request), **full** (the entire private schema, a few rows each) and **large** (1 million accounts and 20 million transactions, for speed and estimates).

### 8.2 The answer key

**Differences Dmux must report.**

| Planted in the new database | Expected result |
| --- | --- |
| 12 rows deleted from the transaction history | Count FAIL; drill-down lists the 12 keys |
| 3 duplicate rows added to a table with no key | Count FAIL; counted-set comparison shows the extras |
| One balance changed by 0.01 | Full-values FAIL and control-total FAIL; drill-down names the column |
| One character changed in a 20-million-row table | Full-values FAIL; the row is found |
| Empty text turned into NULL | Full-values FAIL, explained in plain words |
| Time of day dropped from a timestamp | Full-values FAIL on that column |
| Column dropped, column added | Schema diff: missing (red), added (green) |
| Column renamed, table renamed | Unmapped pair with a suggested match |
| `varchar(50)` widened to `varchar(100)` | Schema diff: changed (amber) |
| Sequence left behind its old value | Inventory FAIL |
| Index missing; constraint not validated | Inventory FAIL |
| A large object missing | Inventory FAIL |
| A row updated while the check runs | "Changed during check", never PASS |

**Cases where Dmux must stay quiet.** Each of these is identical data that a naive tool reports as different.

| Situation | Expected result |
| --- | --- |
| Old database in LATIN1 with accented names, new one in UTF-8 | PASS |
| The two servers sort text differently | PASS |
| Float columns printed differently by 9.6 and 17 | PASS |
| Table converted to a partitioned table | PASS, shown as one table |
| Columns in a different order | PASS |
| `char(1)` moved to `varchar(1)` | Type change shown amber; values PASS |

**Licensing caution.** The data dictionary is its vendor's copyright and may not be reproduced without permission. The PDF, the parsed file and any schema built from it stay out of the public repo. Public documents say "a core banking system".

## 9. CI, packaging and quality gates

GitHub Actions checks every pull request, runs the heavy tests nightly, and builds the installers when a version tag is pushed.

| Workflow | Trigger | What it does |
| --- | --- | --- |
| `ci.yml` | Every push and pull request | Lint, unit and property tests, coverage, answer-key test on the small `demo_bank` data, UI tests, SonarQube Cloud scan, dependency and secret scans, commit-rule check |
| `codeql.yml` | Pull requests and weekly | GitHub CodeQL for Rust and TypeScript |
| `nightly.yml` | Daily, and on demand | Cross-version matrix, mutation testing, large dataset with an estimate-accuracy check, end-to-end smoke test |
| `release.yml` | A tag such as `v1.0.0` | Builds and signs installers, writes checksums and a bill of materials, creates a draft release |

**Installers.**

| Platform | Package | Notes |
| --- | --- | --- |
| Windows | Setup `.exe` (per user, no admin rights) | Carries the offline WebView2 installer, for PCs with no internet |
| Windows | `.msi` | For IT teams that deploy centrally |
| Windows | Portable `.zip` | Runs with no install, for locked-down PCs |
| Linux | AppImage | One file; needs FUSE 2 on the machine |
| Linux | `.deb` and `.rpm` | Ubuntu and Debian; Fedora and RHEL |
| Linux | Flatpak bundle file | Attached to the release. A Flathub listing comes later, because Flathub requires fully offline builds from source |
| Both | `dmux` command-line binary | Static build, for jump servers |

- Installers are built on GitHub's servers: Ubuntu 22.04 for wide Linux compatibility, and Windows. The Arch laptop builds only the development version.
- Windows signing goes through SignPath, which is free for open source. Apply early, since approval takes time. Until then, Windows shows a SmartScreen warning.
- macOS is left out for now.

**Quality gates.** A pull request merges only when every gate is green.

| Gate | Tool | Pass rule |
| --- | --- | --- |
| Rust coverage | `cargo-llvm-cov` | 85% overall; 95% for the comparison core |
| Frontend coverage | Vitest | 80% |
| Answer key | Command-line runner against mock databases | Every planted difference found; no false alarms |
| Static analysis | SonarQube Cloud | Quality gate passed; no new bugs or vulnerabilities |
| Code scanning | CodeQL | No high or critical alerts |
| Dependencies | `cargo-deny`, `cargo-audit`, `pnpm audit` | No known vulnerabilities; approved licences only |
| Secrets | Gitleaks | Nothing secret, and nothing from `private/` |
| Lint | `clippy` with warnings as errors, ESLint | Clean |
| Protected files | CI script | No change touches both product code and the answer key |
| Commit rules | Commit hook and CI check | Owner's Git identity; no co-author lines |

The nightly mutation and cross-version results gate each release, not each pull request.

## 10. Agent team and workflow

Twelve OpenCode agents build Dmux through OpenRouter. Maya leads and writes no product code; the others each own one area.

| Agent | Role | Model (OpenRouter) | Can edit |
| --- | --- | --- | --- |
| **Maya** | Lead: plans, briefs, merges, keeps the task board | anthropic/claude-opus-5.5 | Task board and decisions log |
| **Arjun** | Rust engine and command-line runner | z-ai/glm-5.3; anthropic/claude-sonnet-5.5 for the comparison core | `crates/`, `src-tauri/` |
| **Leo** | React UI and design system | moonshotai/kimi-k3 | `src/` |
| **Nina** | Mock databases and the answer key | deepseek/deepseek-v4-flash | `testdata/`, `private/` |
| **Omar** | Tests and coverage | deepseek/deepseek-v4-flash | Test files only |
| **Priya** | Reviews every query for Postgres 9.4 to latest | deepseek/deepseek-v4-pro | Nothing |
| **Daniel** | Code review; hunts for any false PASS | anthropic/claude-sonnet-5.5 | Nothing |
| **Zara** | Security review | anthropic/claude-sonnet-5.5 | Nothing |
| **Grace** | Usability review from screenshots | google/gemini-3.7-flash | Nothing |
| **Kenji** | GitHub Actions, packaging, signing | deepseek/deepseek-v4-flash | `.github/`, packaging |
| **Ella** | User guide and docs | deepseek/deepseek-v4-flash | Docs |
| **Sam** | Fast code search | z-ai/glm-5.3-flash | Nothing |

Model IDs and prices change; check openrouter.ai/models before starting, and give the key a spending cap.

**How work flows.**

1. Maya picks a task from `docs/TASKS.md` and creates a separate working copy for it with a Git worktree, on its own branch.
2. The owning agent builds in that copy. Up to three agents build at once.
3. Omar tests, then the reviewers report problems. Fixes go back to the owner, for two rounds at most.
4. Maya merges finished tasks into main one at a time, and asks the owner before every merge and every push.

**Git rules.** Commits use the repo owner's Git name and email, and pushes go out under the owner's GitHub login. A commit hook removes co-author lines, and only Maya can commit.

**Local setup.** Arch Linux, OpenCode, rootless Podman for the test databases, and optionally RustRover's MCP server for Arjun and Daniel. RustRover sees only the main copy of the code, so it helps with background, not with final review.

**Guardrails added in this review.**

- **Stronger model for the core.** The comparison core is about a tenth of the code and most of the risk. Arjun builds it on Claude Sonnet 5.5, and both Daniel and Priya review every change to it.
- **Protected answer key.** Only Nina may change `expected_diffs.json`, and never in the same change as product code.
- **Tests cannot be loosened quietly.** Omar cannot edit product code. A builder who deletes or weakens a test must say why, and Daniel must approve it.
- **Decisions log.** `docs/DECISIONS.md` records each design decision in two lines, so a fresh agent session does not reopen it.
- **CI is the judge.** An agent's claim that something works counts only when CI is green.
- **Human gates.** The owner approves each phase, each merge, each push and each release.

## 11. Build phases

Work runs in nine phases, and the first one proves the method before any product code is written. Maya opens a phase only when the one before it meets its "done when" test.

0. **Prove the method.** Priya and Arjun run the fingerprint queries by script against Postgres 9.4, 9.6, 12 and 17 containers, loaded with awkward data: accents in LATIN1, floats, different sort orders, NULLs and padding. Kenji builds an empty Tauri app on Windows and Linux in CI and runs one end-to-end smoke test.
   - Done when: identical data gives identical fingerprints on every version, every planted change alters the fingerprint, and both installers build.
1. **Foundation.** Repo, CI, Podman test databases, the `demo_bank` generator, the private generator and the answer key.
   - Done when: CI is green, and one command builds both databases and `expected_diffs.json`, identically on every run.
2. **Engine, structure.** Connections, inventory, schema diff, mapping, and the `dmux` command-line skeleton.
   - Done when: every structural item in the answer key is found on the small and full datasets.
3. **Engine, data.** Normalisation, fingerprints, drill-down, control totals, saved fingerprint files, change detection, spot check, estimates and resume.
   - Done when: the answer key passes in full, including the "stay quiet" cases; the cross-version matrix is green; mutation testing finds no surviving mutants in the comparison core; the large dataset finishes inside its estimated range.
4. **UI.** The design system first, then the seven screens, wired to the engine.
   - Done when: UI tests pass for every result type, the end-to-end smoke test passes, and Grace and Daniel approve.
5. **Reports and history.** PDF and Excel export, run log and run history.
   - Done when: a report from the small dataset shows the verdict, coverage line, ignored items and sign-off block correctly.
6. **Python hooks.**
   - Done when: a sample script's findings appear in the report, and a script that hangs is stopped by the time limit.
7. **Hardening and release.** Full security review, signing, installers, the user guide and the "How Dmux decides" document.
   - Done when: `v1.0.0` installs and runs on a clean Windows 11 PC and a clean Ubuntu machine, and an independent database administrator has reviewed the method.
8. **Pilot.** Use Dmux on a rehearsal migration alongside the existing manual checks, and have a non-technical teammate run it with only the user guide.
   - Done when: Dmux and the manual checks agree, and every point of confusion is fixed.

If time runs short, phase 6 moves after the pilot. Nothing else in version 1 depends on it.

## 12. Open questions

- [ ] **The name.** "dmux" is already a known open-source tool that runs AI coding agents in parallel with Git worktrees, and the name is taken on PyPI and MacPorts. Keep the name with a distinct app ID and package name, or pick another?
- [ ] **How will the real migration run:** in-place upgrade, dump and restore, or replication? This decides whether both databases are online together or saved fingerprints are needed.
- [ ] Exact Postgres versions, text encodings and collations of the old and new databases.
- [ ] Can one machine reach both databases, or must Dmux run on a jump server? Is that server Windows or Linux?
- [ ] Is a SELECT-only login available on both sides, and how long is the freeze window? The window sets how many tables can get a full-values check.
- [ ] Rows and gigabytes of the ten largest tables.
- [ ] Does the upgrade rename or reshape any tables or columns?
- [ ] Which tables need a full-values check for sign-off, and which control totals do the auditors expect?
- [ ] Does the bank's policy allow an open-source tool, and one built with AI coding agents, on production data? Who approves it?
- [ ] Who is the independent database administrator who will review the method?
- [ ] The report format auditors expect, and who signs it.
- [ ] Licence for the repo. Apache-2.0 is suggested.

## Sources

- [formkit/dmux, the existing tool with the same name](https://github.com/formkit/dmux) and [dmux on PyPI](https://pypi.org/project/dmux/)
- [WebdriverIO: Tauri platform support for end-to-end tests](https://webdriver.io/docs/desktop-testing/tauri/platform-support)
- [Tauri: Flatpak distribution](https://v2.tauri.app/distribute/flatpak/) and [AppImage distribution](https://v2.tauri.app/distribute/appimage/)
- [GitHub: CodeQL Rust scanning generally available](https://github.blog/changelog/2025-10-14-codeql-scanning-rust-and-c-c-without-builds-is-now-generally-available)
- [SonarSource: Rust support in SonarQube Cloud](https://www.sonarsource.com/blog/rust-support-in-sonarqube-cloud/)
- [OpenCode: Agents](https://opencode.ai/docs/agents/) and [BenchLM: OpenRouter models and prices](https://benchlm.ai/hosts/openrouter)
- A core banking system's data dictionary (licensed), provided by the project owner; kept out of the repo

The Postgres behaviours in sections 2 and 3 come from Claude's own knowledge of PostgreSQL, not from pages opened for this review. Phase 0 tests each one on real servers.
