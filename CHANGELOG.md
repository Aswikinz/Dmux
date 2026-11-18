# Changelog

All notable changes to Dmux. Dates are release dates; versions follow
[Semantic Versioning](https://semver.org/).

## Unreleased

### Fixed

- After a run, going back to Choose checks, changing the tables or levels and pressing
  **Run** starts a new run. Before, it showed the finished run again. Changing the choices
  also clears the old run's results from the steps (they stay in History), and a run that
  is still going is never left running unseen: Run takes you back to it, and the screen
  says so.

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
