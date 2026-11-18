# Changelog

All notable changes to Dmux. Dates are release dates; versions follow
[Semantic Versioning](https://semver.org/).

## 1.4.0 (2026-11-18)

Column-level checks, saved check choices, checks of chosen tables, and faster runs.

### Checks of chosen tables

- When you choose only some tables, the verdict is about those tables. Before, a 2-table
  run could FAIL on thousands of structure differences, warnings and missing tables
  elsewhere in the database. Those are now summed up in one warning, tables with no
  partner outside the run are listed as ignored with the reason, and the coverage line
  still accounts for every table. A full check (nothing left out) is unchanged.
- The main report of such a run says how many tables were not part of it, instead of
  listing thousands of them; **Export tables not in this run** on the Report screen (or
  `dmux report --not-in-run <file>`) writes that list as a separate PDF.

### Column-level checks

- **Columns…** on Choose checks opens a table's columns: leave columns out of the
  comparison, and choose which number columns are added up as control totals. Key columns
  are always compared (rows are matched on them), even if a saved file asks otherwise.
- A left-out column is listed beside its table in the results and in every report with
  the reason "not selected for this run". If it has a structure difference, the run warns
  about it by name, so leaving a column out can never hide a difference silently.
- Control totals now apply to the table you chose them on. Before, ticking `amount` on one
  table added up `amount` in every table that had it. `dmux run --control-total amount`
  and run definition files keep the old meaning (every table); `schema.table.column`
  names one table.
- Clearer Choose checks: the bulk level tools fold away under **Change several tables at
  once**, and **Add up all suggested** / **Add up nothing** replace the row of tick boxes.

### Saved check choices

- **Save these choices…** writes the chosen tables, levels, left-out columns, control
  totals and run window to a JSON file (`dmux-check-choices`, version 1); **Open saved
  choices…** applies one to the next run. The file holds names only. Names the database
  no longer has are listed on screen, never guessed; unknown fields, a wrong format or
  version, and files over 32 MiB are refused.

### Faster checks

- Each table's old and new sides are now read at the same time (definitions, counts,
  fingerprints, the spot check and the row drill-down), instead of one after the other.
  Each side keeps its own parallelism limit; what is compared is unchanged.

### Tables that could not be paired

- Choose checks offers only tables that will be compared. Tables with no partner in the
  other database (and tables you ignored on Map) are no longer listed there.
- Results and every report show unpaired tables in their own **Not mapped** section,
  saying whether each is missing from the new database or only in the new one. A table
  missing from the new database still fails a full check, and the Not mapped section can
  never be left out of a report.

## 1.3.1 (2026-11-18)

Data differences first, everywhere, and less noise.

### Results

- The coverage line always adds up to the tables in scope. Tables that could not be
  checked at all (missing from the new database, only in the new database, unreadable) are
  now named: "32 of 34 tables fully checked, 0 counts only, 0 structure only, 0 ignored,
  2 not checked". The PDF and HTML reports show a **Not checked** tile when there are any.
- Every failure says what kind of problem it is: **data differences** (rows or values
  differ), **structure differences**, or **other reasons** (a tool fault, data changing
  during the check, an item to review). Data differences are listed first. A table that
  changed during the check is never counted as a data difference.
- A **Kind of problem** filter with an **Only data differences** button sits next to the
  verdict filter. The screen says how many results are hidden; the verdict never changes.

### Reports

- **What goes into the report** on the Report screen: data differences are always included
  and always first; structure differences, other failures, warnings and tables that passed
  can each be left out, or press **Only data differences**. Whatever is left out is named
  on the report's first page, and the verdict, coverage and counts still include it. The
  run log always lists everything. On the command line: `dmux report --leave-out
  structure,other,warnings,passing`.
- The PDF and HTML reports open with a **Data differences** section, then **Other
  failures** grouped by kind, and two new tiles: data differences and other failures. The
  Excel workbook has a "Kind of problem" column.
- The HTML report has a search and filter bar for the whole report, which stays in view as
  you scroll: one search box across failures, tables, structure findings, ignored items
  and warnings, a box per kind, and **Only data differences**.

### Compare structure

- A **Show** filter hides indexes and constraints, or other PostgreSQL objects
  (extensions, functions, sequences, views, triggers), so table and column changes stand
  out. **Tables and columns only** does it in one click, and the count says how many
  differences are in the hidden kinds.

### Choose checks

- After a run, going back to Choose checks, changing the tables or levels and pressing
  **Run** starts a new run. Before, it showed the finished run again. A run that is still
  going is never left running unseen: Run takes you back to it.
- Both search boxes are labelled, and the list of levels can be searched by level ("full
  values") or by control-total column ("amount") as well as by name.

## 1.3.0 (2026-11-18)

Built for very large databases, with a cleaner Results screen.

### Very large databases

- A run on a large database no longer stops with "mapping file too large". The app
  checked the mapping it already held as if it were a file, against a 10 MiB limit that a
  schema with thousands of tables passes easily. A mapping in memory is now checked
  without a size limit, and the limit for mapping files is 256 MiB (about four million
  columns). Mapping files are parsed once instead of twice.
- Saved captures can now be up to 8 GiB (was 64 MiB, about 170 large tables). Reading a
  capture checks one table at a time and the file hash is streamed, so comparing a
  capture needs about twice its size in memory instead of about eleven times; writing one
  needs no extra memory.
- Results files opened in the app and inventory files for `dmux diff` can be up to 1 GiB
  (was 64 MiB), and custom checks receive results of up to 1 GiB (was 1 MiB), so they
  run on large databases instead of being skipped.
- Every limit is still checked before a file is read, and every other check on untrusted
  files is unchanged.

### Results

- The ignored items are shown as a count; **Show ignored items** lists each one with its
  reason. The Report screen does the same. Exported reports still list every ignored item
  on the first page.
- A **Show** filter, with a box for each verdict and a **Hide warnings** button, narrows
  the failures and the list of all tables, so failures stand out. The screen says how many
  results are hidden, and the overall verdict is never filtered.

## 1.2.0 (2026-11-18)

Choose exactly which tables a run reads.

### Choose checks

- A new **Tables to check** list, grouped by schema: tick a whole schema, single tables, the
  tables matching a search, or **Select all**. Nothing is chosen at first, so a run during
  busy hours reads only the tables you pick. **Run** waits until at least one table is
  chosen.
- Tables you leave out are never read. Each is recorded as "not selected for this run" in
  the results, the coverage line and every report, so nothing is left out silently.
- Levels, the bulk tools, the totals and the time estimate cover only the chosen tables.

### Map

- **Show what was mapped** lists every paired table, with its columns underneath, the old
  and new names, and how each pair was made. It can be searched and narrowed to the
  renamed or undecided pairs.
- **Save mapping** now saves a file. Before, it did nothing you could see. A save dialog
  opens, and the screen says where the file went and its mapping code, or that saving was
  cancelled or failed. The file is owner-only and can be reused with `dmux run`.

## 1.1.0 (2026-11-18)

Reports that are easier to read and record when the check ran.

### Reports

- The PDF report is redesigned: the Dmux logo and "Page N of M" on every page, the
  verdict in a banner with what it means, the counts at a glance, failures as cards with
  their reasons, and the tables, findings and ignored items as aligned tables. Row keys,
  when included, are grouped per table instead of one line each.
- A new interactive HTML report: one offline file that opens in any browser, with search,
  filters by verdict and level, a "not passing only" switch, sortable columns and
  expandable per-table detail. It can load nothing from outside the file. Export it from
  the Report screen or with `dmux report --html <file>`.
- Every report (PDF, HTML, Excel and the run log) carries a check record for compliance:
  when the check started and finished, how long it took, when the report was generated,
  and which databases were compared (name, server and login, never a password). Results
  files from earlier versions say these were not recorded.
- Structure findings are listed gravest first.

### Desktop app

- The Connect screen leaves room between the saved-connections list and the form.
- Choose checks keeps each way of selecting tables (by schema, by name, by size) together.
- The Run screen removes Pause and Cancel once the run has ended.
- History shows "Open results" and "Run log" side by side.
- The starting check level in Settings uses the same drop-down as the rest of the app.

## 1.0.0 (2026-11-18)

The first stable version. It closes the findings of the security and false-PASS
audit, so no audit finding still allows a false PASS.

### Safer verdicts

- Row-level security can no longer hide rows: Dmux reads with `row_security = off`,
  and a table it is not allowed to read in full is a TOOL FAULT, never a PASS.
- Ticked comparison options and type changes are listed per column. A PASS that
  relied on them reads **PASS (with options)** in the app, the PDF and the workbook.
- Change detection that cannot work (`track_counts` off, foreign tables) makes the
  table NEEDS REVIEW instead of a silent PASS.
- Two old tables, or two old columns, mapping to one new target are refused. Rename
  rules apply to table names only.
- Saved fingerprint files record the normalisation rules they were made with. A file
  made under other rules, or one that does not say, is refused. A file whose
  "needs review" flag was edited is refused. A file made by another Dmux release with
  the same rules gives a warning. Files over 64 MiB are refused.

### Connections

- Verified TLS with the bank's own certificate authority: the `--ca-file` option, the
  `ca_file` key in run definitions, and a field on the Connect screen.
- The trust-once certificate flow works again.
- A superuser login can only be used with a typed reason, which is recorded in the
  results and the report.
- On a machine with no system keychain, a typed password is kept in memory for the
  session only and forgotten when the details change or the app closes.
- Run definition files may no longer contain a password.

### Desktop app

- Export PDF and Excel reports from the Report screen, through a native save dialog.
  The report is made from the app's own verified copy of the run's results.
- The verdict is stated prominently, in one spelling everywhere, as soon as a run
  ends.
- Presets on Choose checks really change the check levels. Encoding differences are
  shown as information, and changed column types show their old and new types.
- New look: the app icon and its colours, a clearer layout, light motion that follows
  the system's reduced-motion setting, and working light and dark modes.
- Accessibility is checked on every step, in light and dark mode.
- Structure comparison no longer breaks on sequences at the 64-bit maximum, and long
  lists of differing rows are shown a page at a time.

### Completing version 1

- **Hardened servers:** where a server cannot run md5 (FIPS mode), Dmux hashes the rows
  itself, with results identical to the server's, so either side can fall back.
- **Huge tables** are split into ranges of a number or date key and scanned in parallel;
  the parts add up exactly to the whole table.
- **Saved connections** stay saved, with retry, edit and delete. **History** keeps every
  finished check (verified before it is shown again), opens a results file from a jump
  server, shows and saves the run log, and can be purged.
- **Compare structure** shows the old and new databases side by side, aligned row by
  row, searchable and fast with thousands of tables.
- **Results** explain every table, including a PASS: what was compared, how each column
  was compared, keys, buckets, spot check, change detection, control totals and why it
  passed or failed, plus what was checked for the whole run.
- **Map** shows fuzzy suggestions to accept, a picker to pair items by hand, and a rule
  editor; **Choose checks** selects tables by schema, name or size and asks you to confirm
  control-total columns.
- **Run control:** a run window (for example 22:00 to 05:00), the computer kept awake
  during a run, and live time remaining corrected as tables finish.
- **Settings:** appearance, run defaults, report defaults, custom checks (Python scripts,
  off by default, with an honest warning) and an About section.
- **Installers:** a portable Windows zip, an offline installer that carries WebView2, a
  Flatpak bundle, and Windows builds that ship their own Python for custom checks.
- **Test playground:** an `edge_bank` database pair with dozens of edge cases to try by
  hand (`testdata/edge_bank/`).

### Reports and documentation

- The PDF prints the results hash and the capture hashes on page 1, with honest
  wording: the hashes detect corruption, and show tampering only when compared with
  the signed PDF.
- Custom checks (Python hooks) warn that they run with your full permissions. Scripts
  that other users can change are refused.
- The guides cover row-level security, replicas, `track_counts`, the CA file, password
  handling, report export and how to verify a download.

### Upgrading from 0.3.0

- Capture fingerprint files again with 1.0.0. Older captures are refused.
- Remove `password` from run definition files. Use `password_env` or the keychain.
- Logins that need to read row-level-security tables need `BYPASSRLS` (see the
  administrator guide).

## 0.3.0 (2026-11-17)

Desktop app wired to the engine, with PDF and Excel reports from the command line.

## 0.2.0 (2026-11-17)

`dmux capture`, `dmux compare` and `dmux run`, graded against the answer key.

## 0.1.0 (2026-11-17)

Engine structure, the comparison core, and the UI on a mock engine.

## 0.0.1 (2026-11-17)

Phase 0 skeleton.
