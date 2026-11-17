# Dmux user guide

This guide walks a Dmux user through a comparison, screen by screen. It assumes no
knowledge of databases or SQL: the tool is built so that the person who signs the report
can also run it.

> **Status: skeleton.** The seven screens are being wired to the engine. The steps below
> describe the agreed behaviour from [PLAN.md](../PLAN.md) section 7. Where a screenshot
> or an exact button label is still to be confirmed, a **TODO** note says so. Steps are
> expected to stay as written; only the wording on screen may change.

## Before you start

You need:

- the address, port, database name and login for the **old database** and for the **new
  database**. Ask the database administrator; the login must be the read-only
  `dmux_reader` one described in [the administrator guide](dba-setup.md).
- the certificate fingerprint for each server, if the administrator uses self-signed
  certificates. Write it down on paper beforehand: Dmux shows the fingerprint and you
  compare it with that note, never with something on the same screen.
- a quiet period agreed for the run, or the news that you may work on a frozen copy or a
  replica. A busy table is marked "changed during check" and can never be a PASS.
- about ten minutes for the first setup. Once a comparison is saved, a re-run is a few
  clicks.

Words that appear on screen, in one line each:

| Word | What it means here |
| --- | --- |
| Fingerprint | A short code computed from a table's data. Equal codes mean equal data. |
| Table | One set of records in the database, such as all accounts. |
| Structure | The shape of the database: which tables and columns exist and what type each is. |
| Mapping | Which table on the old side corresponds to which table on the new side. |
| Verdict | The overall answer: PASS, FAIL, WIDELY DIFFERENT, NEEDS REVIEW, CHANGED DURING CHECK or TOOL FAULT. |
| Coverage | How many tables were actually checked, and at what level. |

The full method, including what a PASS does not prove, is in
[How Dmux decides](../how-dmux-decides.md).

## Installing Dmux

Dmux is a desktop application for Windows and Linux, with a command-line version for
servers. It makes no network calls except to the databases you configure: no telemetry, no
usage reporting and no automatic updates, so a locked-down machine stays locked down.
macOS is not supported in this version.

### Windows

The file names below are placeholders until the first release is published.

| File | Use it when |
| --- | --- |
| `Dmux_<version>_x64-setup.exe` | The normal choice. Installs for the current user only: no administrator rights, no change to other accounts. It carries the offline WebView2 installer, so the machine does not need internet access. |
| `Dmux_<version>_x64_en-US.msi` | Your IT team deploys software centrally. |
| `Dmux_<version>_x64-portable.zip` | Nothing may be installed at all. Unpack it and run the program from the folder. |

> **TODO (packaging):** confirm the exact file names and the version number for the first
> release; add a screenshot of the first window after install.

**Unsigned builds show a SmartScreen warning.** Windows says "Windows protected your PC"
because the installer has no code-signing certificate yet. Choose **More info**, check that
the publisher line and the file name are the ones your team issued, then **Run anyway**.
Signing is in progress; once it is in place the warning disappears. Never click through the
warning for a file whose origin you have not checked.

### Linux

| File | Use it when |
| --- | --- |
| `.deb` | Ubuntu or Debian. |
| `.rpm` | Fedora or RHEL. |
| `.AppImage` | Any distribution, one file. It needs FUSE 2 on the machine. |
| Flatpak bundle | Attached to the release. A Flathub listing comes later. |

### Command-line version, for a jump server

`dmux` is a single static program with the same engine as the app. Copy it to the jump
server when the laptop cannot reach the database. It runs `dmux capture`, `dmux compare`
and `dmux run <definition file>`, and writes a results file that opens in the desktop app
for review and reporting. Exit codes are 0 for pass, 1 for fail and 2 for warnings, so a
script can act on the result.

> **TODO (packaging):** link the release page, the SHA-256 checksums and the software bill
> of materials here once the release workflow publishes them.

Before installing, check that the download is really this project's: see
[Verifying a Dmux download](verify-download.md).

### First start

1. Start Dmux. On Linux, run the AppImage after marking it executable, or start it from
   the applications menu if you installed a package.
2. The window opens on **Connect**, with the step list down the left side.
3. Nothing needs an account or a sign-in. Dmux asks only for the database details you
   enter, and keeps passwords in the operating system's keychain.

> **TODO (design):** screenshot of the first window, light and dark, plus the reduced
> effects theme used over remote desktop.

## The seven steps

The left side of the window lists the steps, each with a tick when it is done. You can go
back to an earlier step at any time; the later steps remember what you entered. Run
history, scripts and settings sit below the step list.

```
Connect  ->  Compare structure  ->  Map  ->  Choose checks  ->  Run  ->  Results  ->  Report
```

### 1. Connect

**Purpose.** Tell Dmux which two databases to read, and prove it can reach both before
anything else happens.

**Steps.**

1. Choose **Old database** and fill in: host, port, database name, user and password.
   Give the entry a name you will recognise, such as "Core banking, before".
2. Choose **New database** and fill in the same five details.
3. Choose the encryption mode. Leave it on encrypted and verified unless the
   administrator tells you otherwise. If Dmux shows a certificate fingerprint, compare it
   with the fingerprint on your paper note and trust it once.
4. Press **Test connection** on each side, then **Continue**.

**What you see.** For each server: its version, its text encoding, whether the connection
is encrypted, whether the login can write or is a superuser, and whether a connection
pooler sits in between. Green ticks mean ready. Dmux warns, in words, about anything it
does not like: a login that can write, an unencrypted connection, an unusual encoding, or
a server version it does not support.

**If something goes wrong.**

- *"Could not reach the server"*: check the host and port with the administrator, and
  whether a firewall or VPN is in the way.
- *"Password refused"*: the password is not stored until a connection succeeds, so simply
  type it again.
- *"This login can write"* or *"This login is a superuser"*: stop and ask the
  administrator for the read-only `dmux_reader` login. If you must continue anyway, Dmux
  asks for a reason and prints it in the report.
- *"The certificate has changed"*: do not click through. Ask the administrator whether the
  certificate was renewed, and get the new fingerprint from them.

> **TODO (wiring):** screenshot; confirm the labels of the two profile panels and whether
> **Test connection** is one button per side or one for both.

### 2. Compare structure

**Purpose.** See, side by side, what exists in the old database and what exists in the
new one, before any data is compared.

**Steps.**

1. Wait for Dmux to read both structures. Large databases can take a minute.
2. Look at the summary at the top: how many tables are missing on the new side, added,
   changed, and how many match.
3. Tick **Only problems** to hide what matches. Use the search box for a table name.
4. Open a table to see its columns, keys, indexes and constraints side by side.
5. Continue to **Map** when you have read the problems.

**What you see.** A tree of schemas and tables, with columns matched by name rather than
by position. Red means missing on the new side, green means added, amber means changed.
Partitions of one table are folded under their parent so they count as one table. Warnings
appear for things that are not errors but need a decision: a table left from an older
version, a column type that no longer exists, a new table whose statistics have not been
gathered yet.

**If something goes wrong.**

- *A table you expected is missing*: it may have been renamed. Continue to **Map**, where
  Dmux suggests a match you can tick.
- *A view or function is amber although nothing was meant to change*: different server
  versions print definitions differently. The text difference is shown as "review"; a
  named person decides.
- *The structure check fails to finish*: the login may lack rights on a schema. The
  administrator needs to run the login script for that schema.

> **TODO (wiring):** screenshot of the diff tree with one red, one green and one amber
> example; confirm how "review" is displayed.

### 3. Map

**Purpose.** Say which old table is the same as which new table, and which columns pair
up. Nothing is compared until it is mapped or deliberately ignored.

**Steps.**

1. Dmux applies your rules first, then exact name matches, then matches that differ only in
   upper or lower case. You only see what is left.
2. Work through the **Needs attention** queue from the top. Each item shows the old name,
   the suggested new name and a confidence score.
3. Tick a suggestion to accept it, or drag the correct table onto it. One decision can
   cover many tables when a renaming rule applies.
4. For a table that should not be checked, choose **Ignore** and type the reason. The
   reason is required and appears in the report.
5. When the queue is empty, press **Continue**.

**What you see.** A queue, not a thousand-row board. Suggestions are never accepted
automatically: a "close enough" name match is exactly how the wrong columns get compared.
The count of unmapped and ignored tables stays visible at the top.

**If something goes wrong.**

- *"Two candidates match equally well"*: Dmux will not guess. Pick one.
- *You ignored a table by mistake*: reopen it from the ignored list; the reason is removed
  with it.
- *You cannot find a table*: check the case-insensitive option and the search; a table
  folded under a parent partition will not appear on its own.

> **TODO (wiring):** screenshot of the queue with a suggestion and an ignore dialog;
> confirm the wording of the confidence column.

### 4. Choose checks

**Purpose.** Decide how hard Dmux should look at each table, and see how long that will
take before you start.

**Steps.**

1. Pick a preset: full values for everything; counts for everything with full values for
   selected tables; or structure only.
2. Adjust individual tables if needed. Each table has one of three levels: **Structure**,
   **Counts and totals**, or **Full values**.
3. Confirm the control-total columns. Dmux suggests number columns whose names look like
   amounts or balances; tick the ones your auditors expect.
4. Read the estimate: a range per table and in total.
5. Press **Start**.

**What you see.** A list of tables with their level, their size on disk and an estimated
time. The estimate is a range, because it is measured on your servers at that moment, and
it narrows as tables finish. The note under it says that a failing table costs a second
pass, because Dmux then goes back for the rows.

**If something goes wrong.**

- *The estimate is far longer than the window you have*: drop the largest tables to
  counts and totals, or ask for a longer window. The report will say which tables got
  which level, so nothing is hidden.
- *A table has no suggested totals*: its columns may not be numeric, or the names are
  unfamiliar. Tick them by hand or leave counts only.
- *The run window is set outside your quiet hours*: change it in settings before starting.

> **TODO (wiring):** screenshot of the level picker and the estimate panel; confirm the
> three preset names.

### 5. Run

**Purpose.** Watch the comparison happen, and stop it if it is in the way.

**Steps.**

1. Watch the progress list. Each table shows its level, its state and, once it is done,
   its verdict.
2. Leave the machine awake. Dmux blocks sleep during a run; closing the window asks first.
3. Use **Pause** if you need the databases for something else, and resume when ready.
4. Use **Cancel** if the run must stop. It stops the queries on the server, not only the
   display.
5. When every table is done, continue to **Results**.

**What you see.** A row per table with a progress bar, the time left, and words as well as
colours for every state. Tables already finished keep their results, so a dropped VPN, a
closed laptop or a restart does not throw the run away: Dmux resumes where it stopped. A
table being written to by someone else shows "changed during check", never a tick.

**If something goes wrong.**

- *A table sits at the same percentage for a long time*: it may be a very large table, or
  it may be waiting behind a schema change. The statement timeout ends it eventually; the
  run log says which.
- *"Changed during check" on a table you need*: re-run just that table during a quieter
  period, or ask for a frozen copy.
- *"Tool fault"*: this is about Dmux, not about your data. Keep the run log and contact the
  maintainers. Do not sign off on a tool fault.
- *The network dropped*: reconnect and reopen the run from history; finished tables are
  kept.

> **TODO (wiring):** screenshot of the progress list mid-run, and of the pause and cancel
> confirmations.

### 6. Results

**Purpose.** Read the verdict, the coverage, and the failures in that order.

**Steps.**

1. Read the verdict and the coverage line at the top.
2. Read the failure cards: each says what is wrong, how big the gap is and what to do next.
3. Open a failing table to see the rows that are missing, extra or changed, with the
   differing columns side by side.
4. Filter to problems only, and search for a table name.
5. Continue to **Report** when you are ready to produce the signed document.

**What you see.** The overall verdict first, then the tables with failures at the top.
Rows that differ are shown on screen only: the report never contains row values, and it
contains key values only if you tick that option. A table marked **Needs review** lists
the columns a person must look at.

**If something goes wrong.**

- *A failure you believe is not real*: open the table and check the columns listed. Empty
  text turned into a blank, a dropped time of day, or a rounded decimal are the usual
  causes.
- *A table is "widely different"*: so much changed that Dmux shows a sample instead of a
  row list. Treat the table's load as failed and start it again.
- *The coverage line shows many ignored tables*: those tables were not checked. Ask who
  ignored them and why; each reason is on the first page of the report.

> **TODO (wiring):** screenshot of the verdict header, a failure card, and the drill-down
> with two differing columns.

### 7. Report

**Purpose.** Produce the documents the bank signs and files.

**Steps.**

1. Choose the PDF summary, the Excel detail, or both.
2. Tick whether key values may appear in the export. Leave it unticked unless the recipient
   needs them.
3. Fill in the sign-off block: who ran it, who reviewed it, and the date.
4. Press **Export** and choose where to save.
5. Check the first page: the verdict, the coverage line, the ignored items with reasons,
   the run identifier, the tool version, the mapping code and the results checksum.

**What you see.** A PDF sized for a signature, and an Excel file with the detail behind it.
A list that is longer than Excel's row limit is written as a CSV file instead. The run log
holds the user, time, tool version, profiles, mapping version, options ticked and every
query sent.

**If something goes wrong.**

- *The PDF will not open*: check the file finished saving; a full disk is the usual cause.
- *A colleague questions whether the report was edited*: the checksum on the first page is
  computed from the saved results file, so any later change to that file shows up. Re-run
  the saved definition to produce a second report with its own checksum and compare the two;
  if the data has not changed, the verdicts and coverage will match. The report says plainly
  that this is tamper-evident, not tamper-proof.
- *You need an older run*: open **Run history** below the steps, choose the run, and export
  again. History can be purged from the same place if the machine holds sensitive material.

> **TODO (wiring):** screenshot of the export dialog and of a first page with the coverage
> line and sign-off block filled in.

## Below the steps: history, scripts and settings

- **Run history.** Every run, with its verdict, coverage and date. Open one to review or
  re-export it, or to repeat it from its saved definition.
- **Scripts.** Optional custom checks, written in Python by the bank's own team. They
  receive the run's results, never the database passwords or connections, and their
  findings appear in the report marked as custom checks. Off unless switched on.
- **Settings.** Theme, including the reduced effects theme for remote desktops; the run
  window; parallel-scan limits; where history is kept; and purging.

> **TODO (wiring):** screenshots and the final labels for these three pages.

## Keyboard and accessibility

The whole flow works by keyboard, including mapping: arrow keys move through the queue,
space ticks a suggestion, and escape closes a dialog. Status is never shown by colour
alone; every verdict carries a word and an icon.
