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
and `dmux run <definition file>`, and writes a results file. Turn that file into the PDF
and Excel reports with `dmux report --results results.json --pdf report.pdf --xlsx
report.xlsx`; it checks the file's SHA-256 first and refuses a file that was edited.
You can also open that results file in the desktop app (see "History" below), which checks it the same way and shows the verdict and findings. Exit codes are 0 for pass, 1 for fail and 2 for warnings, so a
script can act on the result.

A run definition file names the databases and the checks; it never holds a database
password, and Dmux refuses a file that contains a `password` key. Point it at a password
with `password_env` (the name of an environment variable the jump server sets). If the
bank signs its server certificates with its own certificate authority, add `ca_file` (the
path to the CA's PEM file) to the `old` and `new` connection tables, or pass
`--ca-file <PATH>` to `dmux probe`, `dmux inventory` and `dmux capture`.

A saved capture can be compared again later, but only by a version of Dmux that uses the
same comparison rules as the one that made it. Dmux refuses any other capture and asks for
it to be captured again; see [How Dmux decides](../how-dmux-decides.md).

> **TODO (packaging):** link the release page, the SHA-256 checksums and the software bill
> of materials here once the release workflow publishes them.

Before installing, check that the download is really this project's: see
[Verifying a Dmux download](verify-download.md).

### First start

1. Start Dmux. On Linux, run the AppImage after marking it executable, or start it from
   the applications menu if you installed a package.
2. The window opens on **Connect**, with the step list down the left side.
3. Nothing needs an account or a sign-in. Dmux asks only for the database details you
   enter, and keeps passwords in the operating system's keychain (see "How Dmux handles passwords"
   under Connect, below).

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
   with the fingerprint on your paper note and trust it once. If your bank signs its server
   certificates with its own certificate authority, the administrator gives you a
   certificate authority file (a `.pem` or `.crt` text file); with "Verify certificate"
   chosen, enter its path in the field "Your bank's certificate authority file (optional)".
   Dmux then trusts certificates from that authority as well as the public ones, and still
   checks the server's host name.
4. Press **Test connection** on each side, then **Continue**.

**Saved connections.** A connection that tests successfully is saved automatically, so you
do not have to enter it again. The card under it says **Saved**, when it was last tested,
and where its password is: in your system keychain, or only in memory until you close the
app. Each side has a list, **Saved connections**, to fill the form from one you saved
before. The card also has **Retry connection** (test again without retyping the password),
**Edit** (go to the details) and **Delete connection** (asks first, and also deletes the
saved password). If you change the details of a saved connection, the card says so; type
the password again to test the new details.

**What you see.** For each server: its version, its text encoding, whether the connection
is encrypted, whether the login can write or is a superuser, and whether a connection
pooler sits in between. Green ticks mean ready. Dmux warns, in words, about anything it
does not like: a login that can write, an unencrypted connection, an unusual encoding, or
a server version it does not support.

**How Dmux handles passwords.** A password you type is kept in the operating system's
keychain, never in a file, a log or a report. On a machine that has no keychain, Dmux says
so: "No system keychain: Dmux will keep this password in memory until you close the app."
The password then stays in memory for this session only, tied to exactly the connection
details you entered. It is forgotten when you change the connection details, when you
delete the profile, and when you close the app, so you type it again next time. The
command-line version takes the password from an environment variable (`--password-env`),
never from a file.

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
2. Read the two headings: the old database is on the left and the new one on the right,
   each with its server version and text encoding.
3. Look at the line above the list: it says how many differences there are and how many
   items you are looking at.
4. Tick **Show only problems** to hide what matches. Type in the search box to find a
   table, column, index or constraint by name, or by a word in its definition. Search
   works across thousands of tables.
5. Open a table (click the arrow beside its name, or press the right arrow key) to see its
   columns, indexes and constraints.
6. Continue to **Map** when you have read the problems.

**What you see.** One line per item, with the same table, column, index or constraint on
the same line on both sides. Items are matched by name, never by position. An item that
exists on only one side leaves an empty, hatched gap on the other side with a plain
message: "Not in the new database" means it is missing there, "Not in the old database"
means it was added. A changed item shows what it was on the left and what it is now on the
right, for example `numeric(18,2)` and `numeric(19,2)`. Every difference also carries a
word (Missing, Added or Changed), so you never depend on colour: red is missing, green is
added and amber is changed. Identical items are shown plainly. A group that is closed
tells you how many differences are inside it.

Use the arrow keys to move up and down the list, the right and left arrow keys to open
and close a group, Home and End to jump to the first and last line, and Enter to open or
close the highlighted group.

Partitions of one table are folded under their parent so they count as one table. Warnings
appear for things that are not errors but need a decision: a table left from an older
version, a column type that no longer exists, a new table whose statistics have not been
gathered yet. When the two text encodings differ, a note says so and explains that this is
expected and needs no action.

**If something goes wrong.**

- *A table you expected is missing*: it may have been renamed. Continue to **Map**, where
  Dmux suggests a match you can tick.
- *A view or function is amber although nothing was meant to change*: different server
  versions print definitions differently. The text difference is shown as "review"; a
  named person decides.
- *The structure check fails to finish*: the login may lack rights on a schema. The
  administrator needs to run the login script for that schema.

> **TODO (wiring):** screenshot of the side-by-side view with one missing, one added and
> one changed item.

### 3. Map

**Purpose.** Say which old table is the same as which new table, and which columns pair
up. Nothing is compared until it is mapped or deliberately ignored.

**Steps.**

1. Optional: add **rename rules** at the top. A schema rename says that everything in the
   old schema `legacy` now lives in the new schema `public`. A prefix or suffix rule says
   that a table whose name starts (or ends) with some text has that text replaced, for
   example `tbl_` becomes nothing. Press **Preview rules** to see how many tables the rules
   pair, then **Apply rules and rebuild mapping**. Rules rename schemas and tables only,
   never columns. Applying rules starts the mapping again, so do it before you pair
   anything by hand.
2. Dmux applies your rules first, then exact name matches, then matches that differ only in
   upper or lower case. You only see what is left, in the **Needs attention** queue.
3. Work through the queue from the top. For an item with no partner Dmux may show a
   **suggestion**: the closest name in the new database and a percentage for how alike the
   names are. A suggestion is only a guess and is never used by itself. Press
   **Use suggestion** to accept it. Weak guesses (under 40 percent) are not shown.
4. If there is no suggestion, or it is wrong, press **Choose counterpart**. A search box
   lists the tables (or, for a column, the columns) that exist only in the new database.
   Type to narrow the list, then click one, or use the arrow keys and Enter. Escape closes
   the list without choosing.
5. Two old tables can never map to the same new table, and neither can two old columns.
   If you try, Dmux refuses and says which old table already has that partner. Pair or
   ignore one of them differently.
6. For a table that should not be checked, choose **Ignore** and type the reason. The
   reason is required and appears in the report.
7. When the queue is empty, press **Choose checks**.

**What you see.** A queue, not a thousand-row board. Suggestions are never accepted
automatically: a "close enough" name match is exactly how the wrong columns get compared.
Items that exist only in the new database are counted below the queue and offered when you
choose a counterpart.

**If something goes wrong.**

- *"Two old tables cannot map to one new table"*: Dmux will not guess which one the new
  table really is. Unpair one or choose a different partner.
- *You ignored a table by mistake*: pair it again from the queue; pairing removes the ignore
  and its reason.
- *You cannot find a table in the picker*: it may already be paired with another old table,
  or it may be in a different schema. A table can only be paired within the schema its old
  schema maps to; use a schema rename rule to change that.
- *The rules paired fewer tables than you expected*: the preview shows how many. A rule only
  pairs a table when the renamed table really exists in the new database.

> **TODO (wiring):** screenshot of the queue with a suggestion and the pairing list.

### 4. Choose checks

**Purpose.** Decide how hard Dmux should look at each table, and see how long that will
take before you start.

**Steps.**

1. Pick a preset: full values for everything; counts for everything with full values for
   selected tables; or structure only. Changing a table by hand switches the preset to
   **Custom**.
2. Adjust individual tables if needed. Each table has one of three levels: **Structure**,
   **Counts and totals**, or **Full values**.
3. To change many tables at once, tick them in the first column, or use the selection tools:
   **Select schema**, **Select by name** (type a pattern; `*` stands for any text, upper
   and lower case do not matter, so `acct_*` finds every table that starts with `acct_`),
   or **Select by size** (larger or smaller than a number of megabytes). Selecting replaces
   the earlier selection. Then choose a level under **Set level for the selection** and
   press **Apply**. Tables with no size on record are skipped by the size selection, and
   the screen says how many.
4. Confirm the control-total columns. Dmux suggests number columns whose names look like
   amounts or balances, **unticked**. Nothing is added up until you tick it. A ticked
   column is added up in every table that has a column with that name.
5. Read the estimate: a range per table and in total.
6. Press **Run**.

**What you see.** A list of tables with their level, their size on disk and an estimated
time. The estimate is a range, because it is measured on your servers at that moment, and
it narrows as tables finish. The note under it says that a failing table costs a second
pass, because Dmux then goes back for the rows.

**If something goes wrong.**

- *The estimate is far longer than the window you have*: select the largest tables by size
  and set them to counts and totals, or ask for a longer window. The report will say which
  tables got which level, so nothing is hidden.
- *A table has no suggested totals*: its columns may not be numeric, or the names are
  unfamiliar. Counts alone are still checked.
- *The run window is set outside your quiet hours*: change it in settings before starting.

> **TODO (wiring):** screenshot of the selection tools and the estimate panel.

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
5. Open **Details** on any table in **All tables**, or **Show details** on a failure card, to
   see why that table got its verdict. This works for a table that passed as much as for
   one that failed.
6. Read **What was checked** at the bottom of the page.
7. Continue to **Report** when you are ready to produce the signed document.

**What you see.** The overall verdict first, then the tables with failures at the top.
Rows that differ are shown on screen only: the report never contains row values, and it
contains key values only if you tick that option. A table marked **Needs review** lists
the columns a person must look at.

**Table details.** The detail panel starts with one sentence, "Why this table passed" or
"Why this table failed" (or "needs review" and so on), built only from facts Dmux
recorded. Under it, in plain words:

- *What was tested*: the check level and what it means. Structure only compares the
  definition and reads no rows; counts and totals counts rows and adds up number columns;
  full values compares every value of every compared column.
- *Rows*: how many rows each database holds, and how many are missing, extra or changed.
- *Key columns*: the columns rows were matched by, or a note that the table has no key and
  was compared as a counted set.
- *Buckets*: Dmux summarises rows in groups called buckets and compares each bucket as one
  unit. The panel says how many buckets there were and how many differed.
- *Spot check*: a second, independent method. Dmux fetches a sample of rows from both
  databases and compares them value by value, then checks this agrees with the bucket
  comparison. The panel says how many rows were compared. If the two methods disagree, the
  table is a tool fault, not a data result.
- *Columns compared*: every column with the rule used, for example "compared exactly as
  text", "compared by its exact 8-byte binary form" or "rounded to 2 decimals because you
  ticked that option".
- *Control totals*: for each summed column, the total in the old database, the total in
  the new one and whether they match.
- *Change detection*: whether the insert, update and delete counters were the same before
  and after the table was read. If change detection was not available (for example the
  server setting `track_counts` is off, or the table is a foreign table), the panel says
  so and why, and the table needs review rather than passing.
- *Options and type notes*: every ticked option and every column whose type differs
  between the two databases.
- *Ignored columns*: each with the reason you gave.

The panel never shows row values. Row keys appear only in the drill-down under **Show
rows**. A table that was reused from an earlier run, compared from a saved file, or checked
by structure only has fewer facts; the panel says which are not recorded and never fills
the gap with a guess.

**What was checked.** At the bottom of the page: how many tables were checked at each
level, and for each database whether the connection was encrypted (and whether the
certificate was verified), whether the login is read-only, whether it was a superuser login
(with the reason you typed when you allowed it), and the server version and text encoding.
It also shows the Dmux version, the fingerprint of the mapping and the results SHA-256. The
SHA-256 detects accidental damage; to show the results were not altered, compare it with the
value printed on the signed PDF report.


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

1. Read the verdict and coverage line at the top of the screen.
2. Tick **Include row keys in the exported report** only if the recipient needs key values.
   It is off by default, and row values are never exported.
3. Press **Export PDF** for the summary, **Export Excel** for the detail behind it, or both.
4. A save dialog from your operating system opens. Choose where to save and confirm.
5. The screen then shows the path the file was saved to. If you close the dialog, it says
   the export was cancelled and nothing was saved. If something fails, it says so in plain
   words.
6. Check the first page: the verdict, the coverage line, the ignored items with reasons,
   the run identifier, the tool version, the mapping code and the results checksum.

**What you see.** A PDF sized for a signature, and an Excel file with the detail behind it.
Dmux builds each file from its own copy of the finished run's results and verifies it
before saving, and it will not export a run that is not finished or that failed. The file is
saved readable by you only (on Linux, mode 0600); copy it to a shared place yourself when
you are ready to share it. A list that is longer than Excel's row limit is written as a CSV
file instead. The run log holds the user, time, tool version, profiles, mapping version,
options ticked and every query sent.

**If something goes wrong.**

- *The PDF will not open*: check the file finished saving; a full disk is the usual cause.
- *A colleague questions whether the report was edited*: the checksum on the first page is
  computed from the saved results file, so any later change to that file shows up. Re-run
  the saved definition to produce a second report with its own checksum and compare the two;
  if the data has not changed, the verdicts and coverage will match. The report says plainly
  that these hashes detect corruption and show tampering only when compared with the values
  printed on the signed PDF, so keep the signed PDF where it cannot be changed.
- *You need an older run*: open **Run history** below the steps, choose the run, and export
  again. History can be purged from the same place if the machine holds sensitive material.

> **TODO (wiring):** screenshot of the export dialog and of a first page with the coverage
> line and sign-off block filled in.

## Below the steps: history, scripts and settings

- **History.** Press **History** in the sidebar, below the steps. Every finished check is
  listed with its date, verdict, coverage and the two databases. **Open results** shows that
  check on the Results screen again, and from there the Report screen can export it as a
  PDF or Excel file. Before showing a stored check, Dmux verifies its file; if the file was
  changed since Dmux wrote it, Dmux says so and shows nothing. **Run log** shows what the
  check recorded as plain text, and **Save run log** saves it where you choose.
- **Open a results file.** The **Open a results file** button on the History screen opens a
  results file, for example one written by the command-line version on a jump server. Dmux
  checks it exactly as `dmux report` does (its SHA-256, and that a PASS does not hide a
  failing table). A file that fails the check is refused with a plain message and nothing
  is shown. Files larger than 64 MiB are refused.
- **Delete local history.** The **Delete all history** button, after a confirmation, deletes
  every stored results file and the history list on this computer. Saved connections are
  kept. Reports and files you saved elsewhere are not touched. Stored results files are
  readable only by you, and hold counts, names and hashes, never row values.
- **Scripts.** Optional custom checks, written in Python by the bank's own team. A custom check
  runs with your full permissions. Only run scripts you trust. Dmux strips the
  environment and never hands a script the database passwords or connections, but it does
  not sandbox the script itself. On Linux, Dmux refuses a script, or a scripts folder, that
  is group- or world-writable or owned by someone else, because anyone who could edit it
  could run commands as you. Findings appear in the report marked as custom checks.
  Off unless switched on.
- **Settings.** Theme, including the reduced effects theme for remote desktops; the run
  window; parallel-scan limits; where history is kept; and purging.
- **Settings.** The Settings page is at the bottom of the left-hand list, below the steps.
  It has five sections:
  - **Appearance.** Light, dark, or match your computer, and a reduced effects switch for
    remote desktops. The two small icons at the bottom of the left-hand list change the same
    settings.
  - **Run defaults.** The check level a new plan starts from (Dmux's own suggestion unless you
    pick another), and how many scans run at once on the old and the new database (2 and 4
    to begin with, never fewer than 1). Use a lower number on a busy production server. You
    can still change the levels for a single run on the Choose checks step.
  - **Reports.** Whether the "include row keys" box on the Report step starts ticked. It is off
    unless you turn it on, and you still decide for each export.
  - **Custom checks.** See below.
  - **About.** The Dmux version, the folder where Dmux keeps its settings and history (shown
    for information; you cannot change it here), and the licence.

### Custom checks

A custom check is a small Python script written by your own team. After Dmux compares the
databases, it gives each script a summary of the results (table names, counts and verdicts,
never passwords, connections or row values) and the script can add findings of its own.

**A custom check runs with your full permissions. Only run scripts you trust.** Dmux does not
sandbox a script. It starts the script with a nearly empty environment and never hands it a
password or a connection, but a script can still read your files and use your network.

Custom checks are **off** until you turn them on in Settings.

1. Open **Settings**, then **Custom checks**, and switch on "Run custom checks after each
   comparison".
2. Choose **Choose folder** and pick the folder that holds your scripts (files ending in
   `.py`). Dmux lists each script and says whether it will run. On Linux and macOS it
   refuses a script, or a folder, that other users on the computer could change, or that
   belongs to another user, and it tells you why. Fix the permissions and the script is
   accepted. On Windows this check is not available, so the warning above matters even more.
3. Dmux finds Python for you. If the Windows build ships its own Python (a `python` folder
   next to the app), that one is used first. You can choose a different Python with
   **Choose Python**, or go back to automatic.
4. Set the time limit for each script. A script that runs longer is stopped, and the run
   shows a warning.

Every time you start a run while custom checks are on, Dmux shows the warning again and asks
you to confirm before it starts.

On the Results step, the **Custom checks** section lists every finding with its severity
(Failed, Warning or Note) and its message. A failed custom check makes the overall verdict
FAIL, so a run with a failing custom check never shows PASS. A script that could not run, ran
too long or returned something unreadable is shown as a warning and lifts a clean PASS to
WARNING. The exported report includes the findings and the SHA-256 of each script that ran, so
anyone can later check which code produced them.

> **TODO (wiring):** screenshots and the final labels for these three pages.

## Keyboard and accessibility

The whole flow works by keyboard, including mapping: arrow keys move through the queue,
the buttons under an item accept a suggestion or open the search list for a counterpart,
and escape closes the list or a dialog. Status is never shown by colour
alone; every verdict carries a word and an icon.
