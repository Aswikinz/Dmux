# How Dmux decides

This document explains, in plain words, how Dmux reaches its verdict on a migration.
It is written for the people who read the report and sign it: bank staff and auditors
who do not work with databases, and who should not have to take the tool on trust.
Every technical term is explained the first time it is used. The formal description is
in [PLAN.md](PLAN.md) sections 1 to 3, and the design choices taken along the way are
in [DECISIONS.md](DECISIONS.md).

## The method in one paragraph

Dmux connects to the old database and the new database with a login that can only read
(a database is a collection of tables, kept by a database server). It asks each side for a
short summary of every table, called a fingerprint, and compares the summaries. It does not
compare rows side by side. If the summaries match, the rows match. If a part of a summary
differs, Dmux goes back and fetches only the rows in that part, so a report can name the
rows and columns that differ. Row values stay inside each database until that last step,
and only a few thousand short lines per table ever leave it.

Dmux never writes to either database. Every read runs inside a transaction that the
database itself refuses to let write (a transaction is one unit of work; "read only"
means the server rejects any change made inside it).

## What a fingerprint is

Picture the archive room of a bank before and after a move: a million customer files,
each with many fields. Reading both rooms page by page would take weeks. Instead, for
each file, write a short code computed from that file's contents, put the file in one of
4,096 trays chosen by the first digits of the code, and for each tray write down two
things: how many files it holds, and four running totals of the rest of each file's code.

If both rooms hold the same files, every tray holds the same number of files and shows
the same four totals. If one file is missing, one character in one field changed, or one
file is there twice on one side, at least one tray's count or totals change. That is the
whole idea. The trays are called **buckets**, and the short code is a **hash**.

A hash is a fixed-length code computed from a value: the same value always gives the
same code, and changing one character changes the code completely. Dmux uses **md5**, a
hash function that turns any text into 32 characters. Its limits are described under
"The one thing md5 cannot do" below.

The database computes the codes and the totals itself, because moving every value across
the network would be far slower than letting the server do the arithmetic. Only the tray
summaries travel.

## Buckets and totals: why row order and sorting never matter

A row is placed in a tray by its own code, never by its position in the table. The same
rows stored in a different physical order therefore land in exactly the same trays, and
Dmux never asks a server for "rows 1 to 50,000". This matters because two servers can
sort text differently: one may put "a" before "B", the other may not. A
tool that compares ordered slices of rows would report identical data as broken for that
reason alone. Dmux's comparison does not contain a sort of the data anywhere, so the
sorting rules of the two servers cannot affect the result.

The four totals per tray are additions. Two identical rows contribute twice, so a
duplicated row cannot hide. Some tools use a shortcut in which two identical rows cancel
each other out to zero; Dmux does not, and one of its standard tests is exactly that
case.

Buckets default to 4,096 per table, and Dmux raises the number for very large tables so
that a tray holds about 10,000 rows. A smaller table simply uses fewer trays. If a table
has no key column, Dmux buckets the rows by the row code itself and compares them as a
counted set, so a table without a key is still checked.

## Why every value is converted before it is hashed

The two databases are rarely configured identically, and the same value can be stored or
printed differently on each. Dmux removes those differences before hashing, using the
same rules on both sides:

| Kind of value | What Dmux does | Why |
| --- | --- | --- |
| Text | Converted to one universal text form (UTF-8) | An older database may store an accented letter differently. Without the conversion, every accented name would look changed. |
| Decimal numbers | Compared as printed; when the migration changed the number of decimal places, both sides are widened to the larger number | Widening is exact. Narrowing rounds: 1.2344 and 1.2345 would both become 1.23, and a real change would disappear. |
| Fractions ("floating point" numbers) | Compared by their exact bytes, never by how they print | The two servers print the same fraction differently, for example 0.333333333333333 against 0.3333333333333333. The printout is a display habit, not the data. |
| Binary data (scanned images, signatures) | Compared as raw bytes | The printed form of binary data depends on a setting, the bytes do not. |
| Dates and times | Read in one fixed time zone (UTC) and one fixed format | The same moment can be written with different offsets; after conversion they agree. A changed second still differs. |
| "Empty" and "nothing there" | Always different | An empty text field that became a blank (NULL) is a real change, and it is reported as one. |

All of these settings are applied inside each transaction on both sides, so they cannot
leak from one table to the next, and an administrator sees the same settings in the
server log.

## The three levels of assurance

The user chooses a level for each table. The report always says which level each table
actually got, so no reader has to guess.

| Level | What is compared | What a PASS at this level proves | Cost |
| --- | --- | --- | --- |
| Structure | Tables, columns, types, keys, indexes, constraints, sequences, views, functions | Nothing was left behind or changed shape | Seconds |
| Counts and totals | Row count per table, plus sums of chosen number columns such as balances | The same number of rows arrived, and the money adds up the same | One scan per table |
| Full values | A fingerprint of every value in every mapped column | Every row and every value is identical | One slower scan per table |

Counts and full values come from the same scan, so a table checked at full values costs
one pass, not two.

## Every verdict, and what it means

The overall result is PASS only when every table in scope passed at its own level,
nothing changed while it was being read, and nothing was skipped without a recorded
reason. Everything else is one of the following.

| Verdict | What it means | What to do |
| --- | --- | --- |
| **PASS** | Every checked table matched at its chosen level. | Read the coverage line (below) before signing, so you know what was actually covered. |
| **FAIL** | The data differs. Dmux says where: which table, how big the gap is, and for up to 100 changed rows per table, which columns differ. | Give the report to the migration team. A FAIL is a finding about the data, not about the tool. |
| **WIDELY DIFFERENT** | So much of the table differs that listing rows would be useless. Dmux shows a sample of the differing parts instead of the full row list. | Treat it as a failure of the migration of that table and start from the beginning of that table's load. |
| **NEEDS REVIEW** | The values matched, but at least one column was compared through a printed form that Dmux cannot prove is exact (an unusual or add-on data type, for instance). A person must look. | A named reviewer checks the listed columns and records the decision. It is never counted as a PASS on its own. |
| **CHANGED DURING CHECK** | Someone inserted, changed or deleted rows in that table while Dmux was reading it. The counts and totals cannot be trusted, so Dmux refuses to call it a PASS. | Re-run that table during a quiet period, or on a frozen copy or a replica (see [the administrator guide](guide/dba-setup.md)). |
| **TOOL FAULT** | Dmux could not reach a decision: a scan failed, a time limit expired, a table it expected was missing, or the independent spot check disagreed with the fingerprint. This is a statement about the tool, never about the data. | Keep the run log and send it to the Dmux maintainers. Do not sign off on a tool fault, and do not read it as a PASS either. |

A table marked CHANGED DURING CHECK or TOOL FAULT is not a data result. Dmux shows the
words as well as the colour, because status must never rest on colour alone.

To detect a table that moved, Dmux reads the server's own counters for inserts, updates
and deletes before and after reading that table. It also records the position of the
write-ahead log, the server's log of every change, at the start and end of the run.

## The coverage line

The first page of every report carries one line, for example:

> 1,480 of 1,529 tables fully checked, 40 counts only, 9 ignored.

It says exactly what the verdict rests on. "Counts only" means those tables were checked
at the second level, so their values were not compared. "Ignored" means someone chose
not to check them, and each ignored table is listed on the same page with the reason that
was required when it was ignored. A report with many ignored tables can still show PASS,
but the reader can see at a glance how much was left out, and by whom.

## What a PASS does not prove

This paragraph is printed in the report as well, because it is the most misunderstood
part of any verification tool.

- It does not prove that the application works on the new database. Dmux compares data
  and structure, not software behaviour.
- It does not prove that the new database is fast enough.
- It does not prove that data which was already wrong before the migration is now right.
  Dmux compares the new data with the old data, whatever the old data was.
- It does not cover tables, columns or rows that were ignored, unmapped, or checked at a
  lower level. The coverage line and the list of ignored items are part of the verdict.
- It does not make the report impossible to forge. The report carries the run
  identifier, the tool version, a code for the mapping used, and a SHA-256 code of the
  results file, so a later edit is detectable. That is tamper-evident, not tamper-proof.

## The one thing md5 cannot do

Collisions. A hash turns any value into a short code, so in principle two different
values can produce the same code. For md5, a deliberate attacker can construct two
different pieces of data with the same code; this is published cryptanalysis, not a
theory. What that means in practice:

- **What Dmux is for:** accidental migration errors. A row that was not copied, a value
  rounded by a data conversion, an accented name mangled by an encoding change, a
  duplicate that appeared during a reload. Against those, the chance of a false match is
  negligible: a random collision would need something like a 128-bit coincidence, and
  four independent totals per bucket plus the row count make it even less likely.
- **What Dmux is not for:** an operator who deliberately hides a change. Someone who can
  write to the new database and who knows the method could, in principle, craft rows that
  hash to the same codes. Dmux cannot tell you that this has not happened.

The agreed threat model for this version is therefore accidental error, not a malicious
insider (decision 10 in [DECISIONS.md](DECISIONS.md)). If a bank's threat model includes
a malicious operator, Dmux is one control among several and not the control that covers
that case. Two things back the decision up: an independent database administrator reviews
the method before the first real use, and the first real run happens alongside the manual
checks the bank already performs.

## The stop rule

Dmux does not list rows for a table that is broken beyond use. When more than 10 per cent
of the non-empty buckets differ **and** at least 64 buckets differ, the table is reported
as WIDELY DIFFERENT with a sample, and the full row list is skipped (decision 17). The
minimum of 64 buckets is there so that a small table with one changed row is still
investigated row by row, which is cheap and much more useful to the reader.

## Proven database versions

The method was tested on real servers before any product code was written. The fingerprint
method has been proven against PostgreSQL (the database software Dmux supports) in
versions **9.4, 9.6, 12, 15, 17 and 18**, including an old database in LATIN1 encoding and
a new one in UTF-8, which is the usual shape of a bank upgrade.

The results are public and reproducible:
[testdata/phase0/RESULTS.md](../testdata/phase0/RESULTS.md). In summary:

- All eight combinations of old {9.4, 9.6} and new {12, 15, 17, 18} passed the extended
  suite with 83 checks and no failures, on PostgreSQL 9.4.26, 9.6.24, 12.22, 15.20, 17.12
  and 18.7. That is the section "T008: full version matrix (extended proof)".
- Those 83 checks include the 15 original ones: identical data stayed quiet across
  encoding, row order, column order and a `char` to `varchar` change, and each of ten
  planted changes was caught. They add dates with time zones, money, JSON, XML, arrays,
  ranges, intervals, decimal scale changes, empty tables, added and dropped columns,
  collations and an unusual text encoding.
- A 1-million-row and a 20-million-row table gave byte-identical fingerprints on the old
  and the new server.

## The independent spot check

Hashes are compared by arithmetic, so Dmux also checks itself the ordinary way. For each
table that has a key, it fetches 200 rows by key from both sides and compares them value
by value inside the app, without using the fingerprint at all. If that plain comparison
disagrees with the fingerprint, the result is reported as a TOOL FAULT, not as a data
difference: a disagreement means Dmux's own arithmetic is suspect, and the reader must
not be told that their data is wrong when the tool may be.

## Where to read more

- [PLAN.md](PLAN.md) sections 1 to 3: the specification and the normalisation rules.
- [DECISIONS.md](DECISIONS.md): each design decision in two lines.
- [docs/phase0/T003-sql-review.md](phase0/T003-sql-review.md) section 8: the residual
  risks that remain, stated plainly.
- [The administrator guide](guide/dba-setup.md): what Dmux does on your server, and how
  to watch it.
- [The user guide](guide/user-guide.md): the seven screens, step by step.
