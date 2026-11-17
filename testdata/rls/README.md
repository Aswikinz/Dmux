# RLS fixture (`rls_bank`)

The fixture for the H1 finding: row-level security can silently hide rows from
both scans and produce a false PASS. It is a separate database on the mock
servers — **not** inside `demo_bank`'s answer-keyed tables — so the answer key
(`expected_diffs.json`) and its 129 checks are untouched.

## Table

`public.branch_notes` — 20 rows, primary key `id`.

| Column | Type | Meaning |
|---|---|---|
| `id` | `integer` | primary key, 1..20 |
| `branch_id` | `integer` | `1` for ids 1..10, `2` for ids 11..20 |
| `note` | `text` | `'branch 1 note N'` / `'branch 2 note N'` |

RLS and `FORCE ROW LEVEL SECURITY` are enabled, with one SELECT policy:

```sql
CREATE POLICY branch_notes_visible ON public.branch_notes
  FOR SELECT USING (branch_id = 1);
```

so a role that is neither the owner nor `BYPASSRLS` sees only `branch_id = 1`
(the first 10 rows). The other 10 rows (`branch_id = 2`) are hidden.

## Roles

| Role | Login | Superuser | BYPASSRLS | Privileges |
|---|---|---|---|---|
| `rls_owner` | no | no | no | owns `public.branch_notes` |
| `dmux_rls_reader` | yes | no | no | `SELECT` on `branch_notes` only |
| `dmux_dev` | yes | yes | yes | the mock superuser (`compose.yml`), used only to build |

`dmux_dev` is the superuser in the mock containers, so a superuser **bypasses
RLS entirely** — the engine's normal login would never see the policy. That is
why the fixture ships a separate least-privilege login: `dmux_rls_reader` has
only `SELECT` and no `BYPASSRLS`.

`dmux_rls_reader`'s password comes from `DMUX_RLS_PASSWORD`; the default lives
only in `build.sh` (`dmux_rls_only`) for these throwaway, 127.0.0.1-bound
containers.

## The planted trap

On the **new** side only, `id = 20` (a *hidden*, `branch_id = 2` row) has its
`note` changed. The visible half (`branch_id = 1`) is byte-for-byte identical on
both sides. So:

* a scan that trusts the policy sees 10 identical rows on each side and
  **falsely PASSes**;
* a correct scan that reads all 20 rows sees the `id = 20` difference.

## Build

```
testdata/rls/build.sh                 # old 9.6 (:5433) vs new 17 (:5434)
testdata/rls/build.sh 5435 5437       # other local ports (old, new)
DMUX_RLS_PASSWORD=... testdata/rls/build.sh
```

Idempotent: it drops and recreates `rls_bank`, creates the roles if absent, and
refreshes the reader's password. RLS needs PostgreSQL 9.5+, so a server below
9.5 (e.g. `old94` on `:5435`) is skipped with a message and no policy added.
The script ends with a read-only self-check (exit non-zero on any failure).

## What the engine test must assert

Connect as the reader using these URLs (the script prints them too):

```
DMUX_TEST_RLS_OLD_URL=postgres://dmux_rls_reader:<password>@127.0.0.1:5433/rls_bank
DMUX_TEST_RLS_NEW_URL=postgres://dmux_rls_reader:<password>@127.0.0.1:5434/rls_bank
```

Table name: `public.branch_notes`.

1. With `SET LOCAL row_security = off` (Postgres 9.5+), a `SELECT` on
   `public.branch_notes` must raise SQLSTATE `42501` ("query would be affected
   by row-level security policy"), and Dmux must surface a TOOL_FAULT — never a
   PASS.
2. A naive scan (RLS left on) would return only 10 rows on each side and look
   identical — the exact false PASS H1 describes.
