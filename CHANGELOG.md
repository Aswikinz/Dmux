# Changelog

All notable changes to Dmux. Dates are release dates; versions follow
[Semantic Versioning](https://semver.org/).

## 1.0.0 (unreleased)

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
