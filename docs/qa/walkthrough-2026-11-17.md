# Dmux — non-technical used walkthrough (2026-11-17)

**Who wrote this:** sam + kenji (QA & CI), task T057.
**Method:** drove the real desktop flow step by step and captured a screenshot of
each screen (`docs/qa/screenshots/`). Each finding below names the screen, what
happened, why it matters, a suggested fix, and a severity. No secrets or real
row data appear anywhere here (the screens use the invented `demo_bank` layout).

## Scope and honesty note

The screenshots in `docs/qa/screenshots/` were captured by driving the **real
React screens** in Chromium with the deterministic mock engine (the browser
runtime uses the mock, never the live database — see `src/api/engineSelection.ts`).
The **real release binary** (WebKitGTK) is driven against the live `demo_bank`
databases by `e2e/specs/real-flow.spec.ts` under the CI smoke job; WebKitWebDriver
is not installed on this laptop, so those runs happen on GitHub's runners, not here.
The wording and layout are identical on both runtimes — only the engine behind
them differs (mock vs. live Postgres). Findings about the Connect screen are
therefore reliable for both.

---

## Screens

| # | File | Screen |
|---|------|--------|
| 1 | `01-connect-empty.png` | Connect, before anything is typed |
| 2 | `02-connect-tested.png` | Connect, after both "Test connection" clicks |
| 3 | `03-structure.png` | Compare structure (identical data) |
| 4 | `04-structure-diff.png` | Compare structure (differences) |
| 5 | `05-map.png` | Map tables and columns |
| 6 | `06-choose-checks.png` | Choose checks |
| 7 | `07-run.png` | Run checks (finished) |
| 8 | `08-results-fail.png` | Results (FAIL) |
| 9 | `09-report.png` | Report |

---

## Findings

### 1. Connect — passwords are "stored in your system keychain" with no ask-per-run fallback

* **Screen:** Connect (both databases).
* **What happened:** each profile shows a field labelled "Password (stored in your
  system keychain)". There is no visibly separate "type a password for this run
  only" path, and the app gives no clue what happens on a machine with no keychain.
* **Why it matters:** PLAN.md 4.1 says "where no keychain exists, Dmux asks for
  the password at each run". Today the typed password is sent once to Save/Test
  and cleared; if the OS has no keychain, the later steps (Compare structure, Run)
  cannot use it — they fail with "no password is stored for this profile". This is
  the known gap tracked as **T055** (backend must hold a typed password in memory
  for the session). A non-technical user on a locked-down PC (PLAN 2.3) would hit
  a dead end with no idea why.
* **Suggested fix:** land T055, then show two explicit options ("Save in keychain"
  vs. "Use for this session only") and a plain message when the keychain is absent.
* **Severity:** High (blocking for keychain-less machines; already scheduled).

### 2. Connect — "Test connection" does not say what "OK" means or what to do once it passes

* **Screen:** Connect.
* **What happened:** after Test connection, a card appears ("Server version: …
  Encoding: … Encryption: Not encrypted") but there is no "connected" confirmation,
  no green word, and the only way forward is the "Compare structure" button, which
  was already on screen before the test.
* **Why it matters:** a non-technical user cannot tell whether the test succeeded
  beyond reading raw server numbers. PLAN 4.1 promises Test connection "reports the
  server version, text encoding, encryption state, whether the login can write or
  is a superuser, …" — the write/superuser warnings are present only as
  red/amber text when triggered, and nothing summarises "this connection is OK".
* **Suggested fix:** add a "Connection OK" status line (icon + word) and a
  next-step hint ("Both databases connected — continue to Compare structure").
* **Severity:** Medium.

### 3. Connect — the plaintext-reason helper cites an internal spec name

* **Screen:** Connect (after choosing "Allow unencrypted connection").
* **What happened:** the field's helper text reads "Required. This choice is
  recorded in the report (**PLAN 6.7**)."
* **Why it matters:** "PLAN 6.7" is an internal reference no user has seen. A
  non-technical signatory does not know what it means. PLAN 7 mandates plain words.
* **Suggested fix:** rephrase to "Required. Your choice to connect without
  encryption will be written into the report."
* **Severity:** Low (wording).

### 4. Compare structure — new tables show "added" and "missing" but a column type change is only labelled "Changed" with no before/after

* **Screen:** Compare structure (04-structure-diff).
* **What happened:** the diff tree shows `accounts.balance → Changed`,
  `transactions.memo → Added`, `archived_balances → Missing`, but a "Changed" node
  does not show *what* changed (old vs. new type) inline — the DataGrip-style
  before/after that PLAN 4.2 describes is not obvious at first glance.
* **Why it matters:** "Changed" alone does not tell the reviewer whether a
  widening from `varchar(60)` to `varchar(120)` is harmless or a type change that
  will break data. PLAN 4.2 wants red/green/amber with old and new values.
* **Suggested fix:** surface the old/new values next to a Changed node (or on a
  one-click expand), and colour the "Changed" row amber.
* **Severity:** Medium.

### 5. Map — the needs-attention queue shows an "encoding" item that looks technical and impossible to act on

* **Screen:** Map (05-map).
* **What happened:** the queue lists "encoding LATIN1 → encoding UTF8" as the
  third item needing attention, with only Approve/Ignore. A non-technical user has
  no idea what to do with an encoding change, and Approving it changes nothing
  about the comparison.
* **Why it matters:** the encoding difference between the old (LATIN1) and new
  (UTF-8) databases is expected and harmless (the engine normalises to UTF-8), yet
  it is surfaced as a decision the user must make. This is confusing noise in the
  "needs attention" queue.
* **Suggested fix:** show the encoding difference as an informational note on the
  Compare structure screen rather than a blocking mapping decision.
* **Severity:** Medium.

### 6. Map — the queue hint shows raw keyboard shortcuts with no alternative

* **Screen:** Map (05-map).
* **What happened:** the hint reads "Use ↑ ↓ Enter to approve, Delete to ignore".
  Arrow and Delete glyphs are fine for technical users but unexplained; a mouse
  user sees the same text and may not realise Approve/Ignore buttons are on each row.
* **Why it matters:** minor, but the short explanation violates the "any technical
  term gets a one-line explanation" rule in spirit.
* **Suggested fix:** "Use the arrow keys and Enter to approve, Delete to ignore —
  or click Approve / Ignore on each row."
* **Severity:** Low.

### 7. Choose checks — the preset select does nothing when changed

* **Screen:** Choose checks (06-choose-checks).
* **What happened:** the "Preset" dropdown lists "Full values everywhere" and
  "Counts for all, full values for selected", but selecting a different preset does
  **not** change the table levels below it. (In the mock, presets are inert; the
  real engine also only uses the default.)
* **Why it matters:** a non-technical user who picks "Counts for all" expects the
  table list to change. When it does not, the app looks broken. PLAN 4.4 promises
  presets that "cover the common cases".
* **Suggested fix:** wire the preset selection to re-assign table levels (and
  confirm the choice changed the plan), or remove the dropdown until it works.
* **Severity:** High (visible broken behaviour).

### 8. Choose checks — the check-plan table lists generic `table_XXXX` names and a bare "Estimate: 20s – 100s"

* **Screen:** Choose checks (06-choose-checks).
* **What happened:** every row is `demo_bank.table_0000` … `table_0018` with a
  "Check level" dropdown, and "Control totals" is a raw comma list (`amount, balance`).
  The estimate is shown in seconds ("20s – 100s") with no plain-language framing.
* **Why it matters:** in the real app these are the demo_bank table names
  (`accounts`, `transactions`, …); the mock's `table_XXXX` is a test detail. More
  importantly, a range of seconds means little to a signatory about how long a run
  takes ("about half a minute to two minutes" reads better), and control-total
  columns deserve one-line explanations (PLAN 7).
* **Suggested fix:** humanise the estimate ("about 1–2 minutes, corrected as tables
  finish"); clarify what "control totals" are for.
* **Severity:** Low (the `table_XXXX` names are mock-only; the estimate wording is
  real).

### 9. Run — success and failure look identical at the summary level, and there is no per-table "what it found" until Results

* **Screen:** Run (07-run).
* **What happened:** the screen shows "Finished" and a "Done" badge, plus a long
  list of `Done` rows. There is no indication during the run which tables are
  passing/failing, and "Finished" with a green-ish "Done" badge reads as success
  even though the eventual verdict is FAIL.
* **Why it matters:** a non-technical user may read "Finished… Done" and stop,
  missing the FAIL that sits a click away on the Results screen. PLAN 4.6 wants
  progress *per table* with time remaining; the "Done" list does satisfy progress
  but leans optimistic.
* **Suggested fix:** on the Run screen show "Finished — see Results for the
  verdict" rather than just "Done", and colour/badge any table that did not pass
  once it finishes.
* **Severity:** Medium.

### 10. Results — the verdict "Fail" is a small badge easy to miss, and "PASS"/"FAIL" wording in the coverage line uses lower-case mix

* **Screen:** Results (08-results-fail).
* **What happened:** the overall verdict appears as a badge labelled "Fail"
  (capital-F only), and the coverage line reads "12 of 40 tables fully checked, 10
  counts only, 15 structure only, 3 ignored". The failure card for
  `demo_bank.table_0001` explains "12 rows are missing and 3 rows differ" clearly.
* **Why it matters:** PLAN 7 requires "every PASS, FAIL … carries an icon and a
  word" — the icon+word "Fail" is present, but it is small and the undertaking's
  whole point (never a false PASS) deserves a more prominent, unambiguous statement
  such as "Result: FAIL — the two databases do not match".
* **Suggested fix:** render the overall verdict as a prominent "Result: FAIL (the
  two databases differ)" line, not only a badge.
* **Severity:** Medium.

### 11. Results — the "Pass"/"Fail" per-table verdicts and levels are lower-case and partly technical

* **Screen:** Results (08-results-fail).
* **What happened:** the "All tables" grid shows columns Schema/Table/Level/Verdict
  where verdicts are "Pass"/"Fail" and levels are "full-values"/"counts"/"structure"
  (lower-case, with a hyphen). Drill-down rows show raw keys (`TX-1001`).
* **Why it matters:** "full-values" and "counts" are PLAN 3.1 terms but are not
  explained inline for a non-technical reader; consistency with the badge wording
  ("FAIL"/"PASS") would help.
* **Suggested fix:** use the same "Full values" / "Counts and totals" / "Structure
  only" labels as the Choose checks dropdown, and title-case verdicts.
* **Severity:** Low.

### 12. Report — the export buttons say "Export PDF/Excel" but export is not yet available

* **Screen:** Report (09-report).
* **What happened:** the Report screen shows "Export PDF" and "Export Excel"
  buttons, a "Include row keys" switch, and the Fail badge + coverage. Clicking
  Export does nothing useful in this build (export is a typed NOT_AVAILABLE until
  T024, and a save-location dialog that WebDriver cannot drive).
* **Why it matters:** a user who reaches the sign-off screen and clicks Export
  expects a file. Until T024 lands, the button is a dead end with no explanation.
* **Suggested fix:** when export is unavailable, disable the buttons with a plain
  "report export is not available in this version yet" note instead of a silent
  failure.
* **Severity:** Medium (known gap, T024).

### 13. Wording — "Compare structure" vs "Map tables" vs sidebar step ticks are clear, but "demo_bank" appears verbatim

* **Screen:** several (structure, results, report).
* **What happened:** the public `demo_bank` schema name is shown verbatim in the
  tree and results. This is fine for the public test layout, but confirms the
  app shows raw database names with no friendly label.
* **Why it matters:** cosmetic. A real migration would show `core_banking` /
  `private` schema names, which is correct; nothing test/dev-specific leaks in a
  way that matters.
* **Suggested fix:** none required; note only.
* **Severity:** Information.

---

## Top issues to fix before a human test (PLAN 8, "done when" of phase 4)

1. **Password / no-keychain dead end (finding 1)** — land T055 so a typed password
   works without a keychain, and make the choice visible.
2. **Broken preset dropdown (finding 7)** — presets must actually re-assign table
   levels, or the control should be removed.
3. **"Finished… Done" reads as success (finding 9)** — the Run screen must not
   let a FAIL hide behind a cheery "Done".
4. **Encoding shown as a blocking mapping decision (finding 5)** — move it to an
   informational note.
5. **Missing plain "Connection OK" signal (finding 2)** — Test connection must
   state success/failure in words a non-technical user can act on.

All screenshots are in `docs/qa/screenshots/` (PNG, 1280×800); no secrets or row
data are present (demo_bank only).
