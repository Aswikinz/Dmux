# Dmux for the database administrator

This guide is for the database administrator (DBA) who will let Dmux read the old and
the new database. It covers the login Dmux needs, the checks to run on it, encryption,
connection poolers, when to run, the load to expect, and how to watch and stop a run.

Dmux reads. It never writes, creates, drops or alters anything, and it never takes a lock
beyond a plain read. Everything below is about making that true in practice and proving
it to yourself.

The comparison method itself is described in
[How Dmux decides](../how-dmux-decides.md); this page is the operational side.

## 1. Create the SELECT-only login

Dmux expects a login called `dmux_reader` with SELECT rights only. The script
[`docs/dba/create_dmux_reader.sql`](../dba/create_dmux_reader.sql) creates it. Run it
once on the old database and once on the new.

```bash
psql -d bank -v pw="'choose-a-strong-password'" -v schema=public \
     -f docs/dba/create_dmux_reader.sql
```

Notes on that command:

- `-v pw=` is the password, wrapped in single quotes **inside** the double quotes so that
  PostgreSQL stores it as a quoted literal. Pick a long random password; it is stored
  hashed in the server, and Dmux keeps its copy in the operating system's keychain, not
  in a file. On a machine with no keychain, the desktop app holds a typed password in
  memory for that session only, and the command-line version reads it from an environment
  variable (`--password-env`); neither writes it to disk, and run definition files may not
  contain a password at all.
- Your shell keeps the command line in its history. Clear that history entry afterwards,
  or start the command with a space if your shell is configured to ignore such lines.
- `-v schema=` is the schema Dmux should read. Run the script once per schema. On later
  runs the `CREATE ROLE` line reports "already exists"; that is expected and harmless.

What the script does, line by line:

| Statement | What it achieves |
| --- | --- |
| `CREATE ROLE dmux_reader LOGIN PASSWORD ... NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION` | A normal login with a password and none of the administrative powers. |
| `ALTER ROLE ... SET default_transaction_read_only = on` | Every session this login opens starts read-only, even if the client forgets to ask. |
| `ALTER ROLE ... SET lock_timeout = '5s'` | Dmux gives up on a table rather than queue behind a schema change. |
| `ALTER ROLE ... SET idle_in_transaction_session_timeout = '10min'` | A forgotten session cannot sit on a snapshot for ever. **Remove this line on 9.4 and 9.5**, which do not have the setting. |
| `GRANT CONNECT ON DATABASE ...` | The login may connect to this one database and no other. |
| `GRANT USAGE ON SCHEMA ...` | The login may look inside the schema. |
| `GRANT SELECT ON ALL TABLES / ALL SEQUENCES IN SCHEMA ...` | Reads only. There is no `INSERT`, `UPDATE`, `DELETE`, `CREATE` or `TRUNCATE` anywhere in the script. |

`GRANT SELECT ON ALL TABLES` covers the tables that exist when it runs. If new tables are
created in that schema afterwards, either run the script again for that schema, or let
the schema's owner add a standing rule once:

```sql
ALTER DEFAULT PRIVILEGES FOR ROLE <schema owner> IN SCHEMA <schema>
  GRANT SELECT ON TABLES TO dmux_reader;
```

That statement must be run by, or for, the role that creates the tables; the script cannot
guess that role.

### Prove the login is harmless

Two queries are enough, and they are worth keeping in the run folder as evidence:

```sql
SELECT rolsuper, rolcreatedb, rolcreaterole, rolreplication
FROM pg_roles WHERE rolname = 'dmux_reader';
-- expect: f | f | f | f

SELECT has_table_privilege('dmux_reader', 'public.<a table>', 'SELECT') AS can_read,
       has_table_privilege('dmux_reader', 'public.<a table>', 'INSERT') AS can_write;
-- expect: t | f
```

A third check costs nothing: connect as `dmux_reader` and try a write in a transaction
you roll back. It must fail with "cannot execute INSERT in a read-only transaction".

### To remove the login afterwards

```sql
REASSIGN OWNED BY dmux_reader TO postgres;
DROP OWNED BY dmux_reader;
DROP ROLE dmux_reader;
```

The same three lines are at the foot of the script, so nobody has to look them up later.

### Row-level security (RLS)

Some tables use row-level security: a policy that decides which rows each login may see.
For an ordinary query a policy hides rows without any warning, so a comparison tool that
read such a table normally would compare only the rows it was allowed to see and could
report a PASS for data it never looked at. Dmux does not allow that. It turns row security
off for every read (`row_security = off`), and PostgreSQL then answers with an error
(SQLSTATE 42501) instead of quietly filtering. Dmux reports the table as a **TOOL FAULT**,
never as a PASS, with this message:

> the Dmux login is not allowed to read this table: grant SELECT, and BYPASSRLS if the
> table uses row-level security

The same error also appears when the login simply has no `SELECT` on the table, which is why
the message names both causes. First check the grant (section 1 above). If the table uses
row-level security (`relrowsecurity` on `pg_class`, PostgreSQL 9.5 and later), one of these
must hold:

- the `dmux_reader` login has `BYPASSRLS`, or
- the table's owner has exempted the login from the policy, or
- the table is left without RLS for the comparison window.

`BYPASSRLS` lets the login read every row regardless of policy, which is what Dmux needs to
prove the migration copied every row:

```sql
ALTER ROLE dmux_reader BYPASSRLS;
```

This is a real extra power for a login that is otherwise read-only: it can then read rows a
policy would hide. Give it to the comparison login only, for the comparison window, and
revoke it afterwards:

```sql
ALTER ROLE dmux_reader NOBYPASSRLS;
```

Dmux's connection test reports whether the login has `BYPASSRLS` and lists the tables with
RLS enabled, so the gap is visible before a run rather than discovered half-way through it.

## 2. Why Dmux refuses a superuser

A superuser can do anything, including things that look read-only and are not. A
read-only transaction stops ordinary writes, but it does not stop a superuser from calling
a function that writes, changing settings, or reading files on the server. Running a
comparison tool as a superuser would mean that any bug in the tool, or in one of the
libraries it uses, has the full authority of the server.

So Dmux warns when the login it is given can write, and it refuses a superuser login
unless the user overrides it and records a reason. The reason appears in the run log. A
bank signing off on a migration should treat an override as an exception that needs a
name against it, not as a convenience.

The read-only promise is enforced in three independent places, which is deliberate: the
login's own default, the `BEGIN ... READ ONLY` that opens every transaction, and the fact
that Dmux writes every statement itself, with no free-form SQL accepted from a user, a
mapping file or a script.

## 3. Encryption, certificates and the bank's own CA

Dmux tries a verified encrypted connection first. "Encrypted" means the traffic between
Dmux and the server cannot be read on the network. "Verified" means Dmux checks that the
server on the other end is the one it claims to be, using the server's certificate.

A certificate is a small file the server presents, and its **fingerprint** is a short code
computed from it, in the same spirit as the row fingerprints. The first time Dmux meets a
server whose certificate it does not recognise, it shows the fingerprint and asks the user
to compare it with the value the server team has on record. If they match, the user trusts
it once and Dmux remembers it. If the server's certificate is later replaced, for example
at renewal, Dmux asks again: that is the point of the check, not a fault.

Self-signed certificates are normal on internal database servers and are fine for this
purpose, provided the fingerprint is compared with a value obtained another way, such as
from the server team, and not simply read off the same screen.

The choices the user sees are:

| Mode | Meaning |
| --- | --- |
| Encrypted and verified (the default) | The certificate must be trusted, either because the machine's certificate store already trusts it, or because the user compared its fingerprint once and chose to trust it. |
| Not encrypted | Traffic is readable on the network. Dmux demands an explicit choice, and the choice is printed in the report. |

### A private certificate authority (CA file)

Many banks sign their internal server certificates with their own certificate authority
(CA), which a laptop's built-in list of trusted authorities does not contain. Rather than
trusting each certificate by fingerprint, give Dmux the CA's certificate as a **PEM file**
(a text file that starts with `-----BEGIN CERTIFICATE-----`; it may hold several
certificates). Dmux adds those certificates to the public ones it already trusts. The
server's host name is still checked, so a certificate from the right CA but for the wrong
server is refused.

- In the desktop app, choose the encryption mode "Verify certificate" and fill in the field
  "Your bank's certificate authority file (optional)" on the Connect screen.
- On the command line, `dmux probe`, `dmux inventory` and `dmux capture` take
  `--ca-file <PATH>`. In a run definition file for `dmux run`, put `ca_file` in the `old`
  and `new` connection tables. `ca_file` is part of the run's identity, so a run resumed
  later cannot quietly start trusting different certificates.
- An empty path, or a CA file together with encryption switched off, is refused. A file that
  is missing or is not valid PEM gives an error that names only the path.

Login with Kerberos is not supported in this version. A login and password is required.

## 4. Connection poolers

A connection pooler is software that shares a small number of server connections among
many clients, such as PgBouncer.

Dmux works through a pooler, but a **direct connection is recommended**. A pooler in
transaction mode hands each transaction a possibly different server connection and drops
per-session settings between statements. Dmux therefore sets everything it depends on
inside each transaction with `SET LOCAL`, so a pooler cannot silently remove a setting
mid-table; but a direct connection removes the question entirely. Dmux detects a pooler
and says so in the connection test, because a pooler changes what an administrator sees in
`pg_stat_activity`, and makes a stuck session harder to attribute to one comparison.

If a pooler is unavoidable, session mode is the closest thing to a direct connection.

## 5. A freeze window, or a replica

Dmux can tell when a table changed while it was being read, but it cannot repair the
result: a table that moved is marked CHANGED DURING CHECK and is never a PASS. So the
practical advice is one of these, in order of preference:

1. **Run on a frozen source.** No writes to the old database for the duration. This is
   the cleanest evidence and the shortest run.
2. **Run on a replica** (a standby copy of the server). Reads do not compete with
   business traffic and do not hold back clean-up on the primary. Both sides need to be
   at the same point in time for a live comparison; a restored copy with no further writes
   is the usual shape. **Pause replay on the replica for the duration of the check**
   (`pg_wal_replay_pause()` on PostgreSQL 10 and later, `pg_xlog_replay_pause()` on 9.6
   and earlier), or check a primary instead. A replica that keeps applying changes moves
   under Dmux's feet. The per-table counters Dmux normally uses to notice a moving table
   are kept per server and are not carried over by replication, so on a replica Dmux
   watches the replay position instead: if replay moves while a table is being checked,
   that table is marked **CHANGED DURING CHECK**, never PASS. Pausing replay avoids those
   wasted results. Resume replay when the run ends.
3. **Run in a quiet window** on the live databases. Set the run window in the app, for
   example 22:00 to 05:00, and accept that busy tables may need a second attempt.

### Change detection needs `track_counts`

Dmux notices that a table moved by reading the server's own counters for inserts, updates
and deletes (the `pg_stat_*` views) before and after reading the table. Those counters only
move when the server setting `track_counts` is **on**, which is the default. Check it on
both servers:

```sql
SHOW track_counts;   -- expect: on
```

If it is off, or if the table is a foreign table, Dmux cannot detect change on that side.
It then marks the table **NEEDS REVIEW** with the reason "change detection unavailable"
(never a PASS), and a resumed run does not reuse that table's stored result. Switch
`track_counts` on, which needs no restart, or run on a frozen source and accept the review
note. Also be aware that a server restart or a statistics reset zeroes the counters, so a
stored result is not reused across one.

Two further points for planning:

- Long reads delay the server's clean-up of dead rows (vacuum) and can therefore grow the
  table on disk. One short transaction per table, rather than one long transaction for the
  whole run, keeps each read short.
- Dmux holds no locks beyond a plain read, but a plain read still waits for a schema
  change in progress. That is what the 5-second lock timeout is for: Dmux stands aside and
  reports the table, rather than blocking the change.

## 6. Timeouts and load

| Control | Default | What it means for you |
| --- | --- | --- |
| Statement timeout | The run window | A single table scan cannot run past the window. Dmux ends it and reports the table. |
| Lock timeout | 5 seconds | Dmux does not wait behind a lock. |
| Parallel scans, old database | 2 | At most two tables read at once, so at most two sessions. |
| Parallel scans, new database | 4 | Same, on the new side. Four is safe on a freshly loaded database; reduce it if the new server is shared. |
| Run window | User's choice | Outside the window Dmux pauses between tables. |

Each side therefore uses a small, bounded number of connections for the scan itself, plus
short catalog queries. Nothing is left connected when a run ends.

A full-values check reads every row of a table once. For planning, the measured rates on
a laptop-class machine were roughly 93,000 to 245,000 rows per second depending on server
version and parallelism; a 20-million-row table took between about 80 and 215 seconds.
Treat those as a starting point and let Dmux's own estimate, which is measured against
your servers, set expectations.

## 7. What Dmux runs

Per table, per side, Dmux opens one transaction:

```
BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL search_path        = pg_catalog;
SET LOCAL timezone           = 'UTC';
SET LOCAL datestyle          = 'ISO, YMD';
SET LOCAL intervalstyle      = 'iso_8601';
SET LOCAL bytea_output       = 'hex';
SET LOCAL extra_float_digits = 0;
SET LOCAL xmloption          = 'document';
SET LOCAL row_security       = 'off';   -- 9.5 and later
SET LOCAL statement_timeout  = '<run window>';
SET LOCAL lock_timeout       = '5s';
SET LOCAL application_name   = 'dmux';
```

Then it runs its own SELECT statements: catalog queries for structure, one scan per table
for counts, totals and bucket summaries, optional follow-up reads for the rows in a part
that differed, and 200 rows by key for the independent spot check. It sets
`search_path = pg_catalog` so that a schema of your own cannot shadow a built-in function
and change a result, and it schema-qualifies the tables it names.

Dmux needs no extensions, no helper functions, no temporary tables, no `pg_stat_statements`
and no write access of any kind. It does not run `ANALYZE`, `VACUUM` or `EXPLAIN ANALYZE`.

## 8. Watching a run

Every connection carries the application name `dmux`, so one query finds all of it:

```sql
SELECT pid,
       usename,
       application_name,
       state,
       backend_start,
       xact_start,
       now() - xact_start AS transaction_age,
       left(query, 80)     AS query_start
FROM pg_stat_activity
WHERE application_name = 'dmux'
ORDER BY backend_start;
```

That column list works on every supported version from 9.4 up. On newer servers you can
also select `wait_event_type` and `wait_event` to see what a session is waiting for.

Reading it:

- `state = 'active'` with a growing `transaction_age` on a large table is a scan doing
  its work.
- `state = 'idle in transaction'` for more than a few seconds is worth a look; on 9.6 and
  later the login's own `idle_in_transaction_session_timeout` ends it after ten minutes.
- `xact_start` tells you how long the current table has been read. Compare it with the
  statement timeout to see whether Dmux is close to giving up.
- The `query` column shows the beginning of the statement. Dmux passes values separately
  from the statement text, so what you see there is table and column names, not account
  data.

To see what a run is doing over time, the app's Run screen shows the same information per
table, and the run log records every statement Dmux sent.

## 9. Stopping a run

In the app, **Cancel** stops the queries on the server, not only the display. Results for
tables already finished are kept, and a run can be resumed.

From the server side, you can do the same:

```sql
-- stop the current statement, keep the session
SELECT pg_cancel_backend(pid)
FROM pg_stat_activity WHERE application_name = 'dmux';

-- last resort: end the session
SELECT pg_terminate_backend(pid)
FROM pg_stat_activity WHERE application_name = 'dmux';
```

Cancelling is always safe here: the transaction is read-only, so there is nothing to roll
back but the read itself. Dmux records the interrupted table as not checked, never as a
PASS, and the run cannot end as a PASS until that table is checked or the gap is recorded
with a reason.

## 10. Checklist before the first real run

- [ ] `dmux_reader` exists on both sides, with `default_transaction_read_only = on`.
- [ ] The four `pg_roles` flags are all false, and `INSERT` is refused (section 1).
- [ ] The script has been run for every schema Dmux will read; a default privilege rule
      covers tables created later.
- [ ] `SHOW track_counts` is `on` on both servers, and replay is paused if a replica is used.
- [ ] RLS-protected tables are covered: `dmux_reader` has `BYPASSRLS`, or each such table
      is exempted, for the comparison window (section 1, "Row-level security").
- [ ] The certificate fingerprint has been compared with the server team's record, and
      the encryption mode is recorded for the report.
- [ ] You know whether a pooler sits in the path; a direct connection is used if possible.
- [ ] A freeze window or a replica is arranged for the tables that need full values.
- [ ] The run window, parallel-scan limits and timeouts are agreed with the Dmux user.
- [ ] Someone knows how to watch `pg_stat_activity` and to cancel, as above.

## Further reading

- [How Dmux decides](../how-dmux-decides.md): the method and every verdict, in plain words.
- [The user guide](user-guide.md): the seven screens the Dmux user works through.
- [`docs/dba/create_dmux_reader.sql`](../dba/create_dmux_reader.sql): the login script.
- [PLAN.md](../PLAN.md) section 6: the safety and security rules Dmux is built to.
