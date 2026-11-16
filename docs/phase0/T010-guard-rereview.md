# T010 — URL guard re-review (R1 closure, safety-critical)

Reviewer: Priya · Date: 2026-11-16 · Branch: `task/T010-url-guard`, commit `c0c95a6`
Scope: the T010 fix for my T005 re-review R1 (local-only URL guard bypass via libpq
multi-host comma URLs and `?host=`/`?port=` overrides), plus R2/R3 closure. Files:
NEW `testdata/phase0/guard.sh`, NEW `testdata/phase0/guard_test.sh`, `prove.sh`,
`perf.sh`, `fingerprint_perf.sql` (newline only), `RESULTS.md`, `testdata/README.md`
(7 files, +193/−14 — scope confirmed by `git diff --stat main...HEAD`).

Every probe below ran with **no database contact**: predicate probes are pure bash;
every harness invocation ran with `psql` shadowed by a stub that records a marker
file and exits 97, so even a hypothetical guard failure could not connect anywhere.
The repo was read-only; the only file written is this review record.

---

## 1. Verdict

**APPROVE.** R1 is closed. The anchored allow-list admits exactly the strict
single-host local form and nothing else; I could not construct — by reasoning or by
probe — any input that matches it yet routes libpq to a non-local address. Both
harnesses call the guard on both URLs before any connection, fail closed on every
guard-file failure mode I tested, and derive their auxiliary URLs without ever
changing host or port. The 31-case battery passes on my own run, its wiring checks
are sound, and R2/R3 are closed exactly as described. No false-PASS/false-FAIL path,
no bypass. Five non-blocking notes (§7), none introduced by T010.

---

## 2. The regex — `guard.sh:34`

```
^postgres://[A-Za-z0-9._~%-]+:[A-Za-z0-9._~%-]+@(127\.0\.0\.1|localhost):[0-9]+/[A-Za-z0-9._~%-]+$
```

**Structure.** Anchored both ends; every character of a matching URL is consumed by
the pattern (no wildcards, no skipped spans). A match is exactly
`scheme + user + ':' + password + '@' + host + ':' + digits + '/' + dbname`, with
user/password/dbname non-empty over `[A-Za-z0-9._~%-]`, host literally `127.0.0.1`
or `localhost`, port required and numeric. Raw `?`, `,`, `@` (second), `/` (in
credentials or after dbname), `#`, `&`, `=`, `;`, `[`, `]`, `\`, whitespace and
control characters are all outside the allow-list, so there is no query string, no
multi-host list, no IPv6 literal, no fragment, no trailing path.

**Percent-encoding cannot alter routing — verified against libpq source, not assumed.**
I read `conninfo_uri_parse_options` in `src/interfaces/libpq/fe-connect.c`
(REL_18_STABLE): the parser splits the **raw** string on structural characters
(`@`/`:` for userinfo, `[`/`]` for IPv6, `:`/`/`/`?`/`,` for host/port/dbname/query)
and only then hands each cut-out component to `conninfo_storeval(..., uri_decode=true)`
→ `conninfo_uri_decode`. So `%40`/`%2C`/`%2F`/`%3F` inside user/password/dbname decode
into **values** after the host/port have been fixed; query parameters are parsed only
from a raw `?`, which the regex forbids. Probe-confirmed: these forms are accepted and
are safe (e.g. `.../bank%3Fhost%3Devil` is a dbname containing the literal text
`?host=evil`, not a query).

One subtlety that makes the **literal host match load-bearing**: the host component
*is* percent-decoded after splitting (`conninfo_storeval(options, "host", hostbuf,
..., true)`). A guard that allowed `%` in the host would be bypassable by
`127.0.0.1%2Cevil.com` — raw splitting sees one host, decoding then yields a comma
list. Because the alternation matches `127\.0\.0\.1|localhost` literally with no `%`
admitted, this vector is closed. Same for the port: digits only, nothing to decode.
`%00` in a value truncates that value at the C-string level after parsing — host
unaffected. Invalid encodings (`%zz`, bare `%`) are accepted by the regex but libpq
rejects them loudly; still local. Port `0`, oversized ports, leading zeros: libpq
errors or defaults, never routes elsewhere.

**Bash quirks — probed, not assumed** (bash 5.3.20, glibc):
- Trailing `-` in `[A-Za-z0-9._~%-]` is a literal hyphen (POSIX). An ASCII sweep of
  all 94 printable characters appended to the dbname accepted **exactly**
  `% - . 0-9 A-Z _ a-z ~` under both `LC_ALL=C` and `LC_ALL=en_US.utf8` — no
  structural ASCII character leaks through the class in either locale.
- **Newline injection:** trailing LF, LF×2, CRLF, CR, embedded LF, leading LF, TAB,
  VT, FF, leading/trailing space — all rejected (battery covers trailing LF; I
  probed the rest). Without `REG_NEWLINE`, `^`/`$` anchor the whole string and the
  class excludes control characters; empirically confirmed on this platform.
- **Locale:** core accepts/rejects identical under C, C.utf8, en_US.utf8, POSIX.
  Collation ranges can admit *non-ASCII* letters into the value classes
  (`bänk` accepted under en_US.utf8, rejected under C) — routing-irrelevant, since
  host/port are literal and values cannot redirect. See N1.
- **`nocasematch`:** off in both harnesses; but an inherited `BASHOPTS=nocasematch`
  does turn it on (probe-confirmed) and then `POSTGRES://` / `LOCALHOST` are accepted.
  Still no non-local routing: the host alternation's case variants are loopback
  (DNS is case-insensitive), remote hosts/queries/commas remain rejected, and libpq's
  `uri_prefix_length` uses case-sensitive `strncmp`, so an uppercase scheme is not a
  URI to libpq at all — it degrades to a dbname on the default local host and fails
  loudly. See N2.
- **Empty/no-arg:** `local u="${1:-}"` + the `+` quantifiers reject both (probed).
- `[[ $u =~ $re ]]` with unquoted `$re` variable is the correct idiom; no `eval`,
  no word-splitting exposure; backslashes in the input are literal (probed).
- `localhost` resolution is trusted exactly as `127.0.0.1` is (a host whose
  `/etc/hosts`/DNS lies about `localhost` is outside any local tool's threat model).

**Host-suffix and lookalike attacks — all rejected (probed):**
`127.0.0.1.evil.com`, `localhost.evil.com`, `127.0.0.1@evil.com`, `0177.0.0.1`
(octal), `2130706433` (decimal), `127.1`, `[::ffff:127.0.0.1]`, `[127.0.0.1]`,
`127.0.0.2`, `[::1]`, `LOCALHOST`, `EvIl.CoM`, negative/`+`-signed/non-numeric/
missing port, empty user/password/dbname, `postgresql://`, `mysql://`, `POSTGRES://`.

---

## 3. Wiring — guard before any connection, fail-closed, safe derivation

**Order (line numbers from the worktree).** `prove.sh`: `HERE` L28 → source L31 →
guard loop L32–36 → first `psql` L157 (server banner). `perf.sh`: `HERE` L35 →
source L38 → guard loop L39–43 → first `psql` L78 (`--version`, no connection) →
first connection L84 (banner). Nothing connects before the guard in either harness;
neither harness invokes podman. The perf cleanup trap (L64) is registered after the
guard and gated by `STARTED` (set only at L163, post-guard), so no refusal path can
trigger a cleanup connection.

**Source path.** `HERE="$(cd "$(dirname "$0")" && pwd)"` resolves from the script's
own path, not cwd — a cwd hijack cannot substitute `guard.sh`. A lone copy of a
harness without its neighbour fails closed (below). `guard.sh` defines only
`dmux_guard_url`; sourcing has no side effects. The SC1091 disables are on the
source lines only; `guard.sh` itself lints clean.

**Fail-closed matrix (probed with the marker stub; `psql_called=no` in every row):**

| guard.sh state | prove.sh | perf.sh | mechanism |
|---|---|---|---|
| missing | rc=1, no psql | rc=1, no psql | source fails → `set -e` exits |
| unreadable (chmod 000) | rc=1, no psql | — | source fails → `set -e` exits |
| empty file | rc=1, no psql | rc=1, no psql | function undefined → status 127 → `if !` takes the refusal branch |
| comment-only | rc=1, no psql | — | same as empty |

The empty-file case deserves precision: `if ! dmux_guard_url "$u"` enters the
refusal branch on **any** non-zero status, including 127 (command not found). Full
stderr shows both `dmux_guard_url: command not found` and the harness's `refusing:`
line, exit 1, zero psql invocations. So the construction is fail-closed for every
failure mode that yields a non-zero status — there is no fail-open hole here. (A
guard.sh tampered to *define a lying function* is outside the threat model: anyone
who can rewrite guard.sh can rewrite the harness itself.)

**Refusal probes (both harnesses, in-place worktree paths and isolated copies,
stubbed psql, marker never written):** comma multi-host `OLD_URL` (remote second
host), `?host=` `NEW_URL`, `?service=` `OLD_URL`, `?options=` `NEW_URL`,
`POSTGRES://`, host-suffix attack, TAB-suffixed URL, CRLF-suffixed URL — all
`refusing: … only runs against local mock databases`, exit 1, before any psql.
**Port-argument injection** (args flow into the default URLs): `prove.sh
'5433,evil.com:5432'`, `perf.sh '5433,evil.com:5432'`, `prove.sh
'5433/bank?host=evil.com'`, `perf.sh '5433/bank?host=evil.com'` — all refused,
exit 1, no psql. Control case `prove.sh 5433` (valid) reached the stub (rc=97,
marker written), proving the stub would surface any connection attempt.

**Derived URLs.** `perf.sh` uses only the guarded `OLD_URL`/`NEW_URL` everywhere
(load, scans, EXPLAIN, quiescence, cleanup). `prove.sh` derives
`C_URL="${OLD_URL%/bank}/bank_c"` (L407) and `A_URL="${OLD_URL%/bank}/bank_ascii"`
(L428). The suffix strip can only remove a trailing `/bank`, which a guarded URL
can only carry as its dbname (dbname admits no `/`); appending `/bank_c` cannot
touch scheme/userinfo/host/port. Probe-verified host:port invariance for
`dbname=bank` (derived URL is still strict form — the guard re-accepts it),
`dbname=mydb`, and `dbname=bank%2Fx`. For a non-`bank` dbname override the derived
URL gains a second path segment: libpq reads the whole `mydb/bank_c` as the dbname
and fails loudly on the **same local server** — never routes elsewhere. This is
pre-existing behaviour, unchanged by T010 (N5).

---

## 4. The battery — `guard_test.sh`

Ran it myself via the wrapper: **31 passed, 0 failed, exit 0** (4 accept + 25
reject + 2 wiring — counts match the RESULTS claim).

**Wiring-check soundness.** The stub exits 97; a harness that connected would die
at 97 under `set -e`, so requiring `rc=1` **and** `refusing:` on stderr distinguishes
refusal from any post-connection failure. My independent marker-stub probes
confirmed zero psql invocations on refusal, and the valid-URL control confirmed the
stub fires when the guard accepts. The battery's bypass URL targets an unused local
port and is doubly safe (guard + stub). One coverage asymmetry: the wiring checks
override only `OLD_URL`; `NEW_URL` refusal is identical code and I probed it
explicitly (§3).

**R1 coverage.** Every bypass form I demonstrated in T005 is present: comma
multi-host (both-local and remote-second), `?host=&port=`, `?host=` remote,
`?port=`, plus `?service=`/`?options=`, embedded `@` in user and password, plain
remote. Accept cases cover both mock URLs, `localhost`, and a strict-form override.

**Forms missing from the battery but probed by me (all behave correctly):**
uppercase/mixed-case scheme, TAB/CR/CRLF/embedded/leading newline, host-suffix
attacks (`127.0.0.1.evil.com`, `localhost.evil.com`), backslash tricks, fragment
and bare `?`, bare `&`, octal/decimal/short-form loopback, IPv4-mapped IPv6,
bracketed IPv4, negative/signed port, empty user/password, percent-encoded
**accept** cases (these document the safe-by-design accepts), and port-argument
injection. Recommend folding a handful into the battery as a follow-up (N4) —
non-blocking, since the predicate already rejects them all.

---

## 5. Strictness side effects — no legitimate harness path breaks

- Defaults and port-arg URLs are strict form; `localhost` overrides accepted;
  `postgresql://` and credential-less URLs are rejected, and no harness path
  constructs either (grep-verified: the only URL sources are the two defaults,
  the env overrides, and the `/bank_c`//`/bank_ascii` derivations).
- Check names intact: the 82 named `check`/`mut`/`assert_*`/`schema_check` call
  names are byte-identical between `main` and HEAD (diff of extracted name lists),
  plus the one manual `lc_monetary recorded` PASS = **83**. The prove.sh diff
  touches only the guard block. Maya's full re-run (83/83, exit 0) and the small
  perf run (exit 0, identity + verified cleanup) are consistent with this; I did
  not re-run the DB harnesses myself (shared mocks; Maya already verified).
- Non-`bank` dbname overrides degrade loudly and locally in 6.12/6.13 (N5) —
  pre-existing, not a T010 regression.

## 6. R2, R3, RESULTS, README

- **R2 closed.** T005 methodology now reads "before the **first** timed scan of
  each table the harness waits (≤30 s)" — matches `perf.sh:176` (`wait_quiet` once
  per table, scan2 immediately after scan1). The diff shows this is the only change
  to earlier sections (T001/T002/T004 byte-identical per Maya; the diff hunk agrees).
- **R3 closed.** `fingerprint_perf.sql` now ends in `\n` (`od`-verified, both
  templates). `diff -u` against `fingerprint.sql` shows exactly the three intended
  differences: header comment, `statement_timeout '1800s'` + comment, table name.
- **RESULTS T010 section accurate:** case counts (29 predicate = 4+25, +2 wiring =
  31), refusal messages preserved, exit-1-before-connect claims, shellcheck-clean
  claim (re-verified by me on all four scripts), 83/83 and perf re-run claims match
  Maya's verification. No row data or secrets — counts and pass/fail only (rule 7).
- **README accurate:** the strict-form description, the comma/`?host=`/`?port=`/
  `?service=` threat list, and the battery pointer all match the code.

---

## 7. Notes (none blocking, none a bypass, none introduced by T010)

| ID | Sev | Note | Suggested follow-up |
|----|-----|------|---------------------|
| N1 | NOTE | Locale collation can admit non-ASCII letters into the value classes (`bänk` accepted under en_US.utf8). Routing-irrelevant (host/port literal; ASCII sweep clean in C and en_US.utf8). | Optional determinism: pin `LC_ALL=C` inside `dmux_guard_url`. |
| N2 | NOTE | Inherited `BASHOPTS=nocasematch` widens acceptance to `POSTGRES://`/`LOCALHOST`. Still local-only: host case variants are loopback, libpq's scheme check is case-sensitive (uppercase scheme degrades to a dbname on the default local host), queries/commas/remote still rejected. Harnesses never set it. | Optional: neutralize `nocasematch` in the guard; requires env control to trigger at all. |
| N3 | GAP (low, pre-existing, outside R1's URL scope) | **Env-level redirect the URL guard cannot see:** libpq fills unspecified parameters from the environment (`conninfo_add_defaults`), and `hostaddr` beats `host` for the actual connection address (`connectOptions2`: `CHT_HOST_ADDRESS` when hostaddr set). A strict local URI + `PGHOSTADDR=<remote>` (or a `PGSERVICE` service file supplying hostaddr) connects remotely. Source-verified, not probed against a live remote. Requires env control — a stronger primitive than URL-arg control, but the guard now blocks the latter while the former stays open. | One line per harness before connecting: `unset PGHOSTADDR PGSERVICE PGSERVICEFILE` (or document env as trusted). Maya should schedule with the next harness task. |
| N4 | NOTE | Battery gaps listed in §4 (all probed correct by me). | Add ~8 cases incl. one percent-encoding accept and a `NEW_URL` wiring variant. |
| N5 | NOTE (pre-existing) | `C_URL`/`A_URL` with a non-`bank` dbname override gain an extra path segment (not strict form, host/port invariant, loud local failure). | Document the `dbname=bank` requirement for 6.12/6.13, or re-guard derived URLs. |

Positive confirmations: R1 closed with a single shared rule; anchored allow-list
with literal host/port; split-before-decode verified against libpq source; guard
before every connection in both harnesses; fail-closed on missing/empty/unreadable
guard; safe auxiliary-URL derivation; port-arg injection refused; 31/31 battery on
my run; wiring checks provably pre-connection; 83 check names intact; R2/R3 closed;
RESULTS/README accurate; shellcheck clean; no row data or secrets.

---

## 8. What I ran

All read-only against the repo; **no database was contacted by any probe** (every
harness invocation used a marker-recording psql stub exiting 97).

1. `scripts/in-wt.sh T010 bash testdata/phase0/guard_test.sh` — **31 passed,
   0 failed, exit 0**.
2. Extended predicate probes (56 cases beyond the battery): control characters
   (LF/CRLF/CR/TAB/VT/FF, leading/embedded/trailing), uppercase/mixed scheme and
   host, host-suffix attacks, second `@`, backslash, fragment, bare `?`/`&`/`;`,
   octal/decimal/short loopback, IPv4-mapped IPv6, bracketed IPv4, signed/empty
   ports, empty user/password, percent-encoded accept cases (`%40 %2C %2F %3F %26
   %00`, invalid `%zz`/bare `%`), no-arg and empty calls — **all as designed**.
3. ASCII sweep of all 94 printable dbname-suffix characters under `LC_ALL=C` and
   `en_US.utf8` — accepted set exactly the allow-list, both locales; core
   accepts/rejects identical under C, C.utf8, en_US.utf8, POSIX.
4. `env BASHOPTS=nocasematch` probe — case variants accepted, remote/query/comma
   still rejected (N2 analysis).
5. Fail-closed matrix: harness copies with guard.sh missing / empty / comment-only
   / unreadable — all rc=1, zero psql invocations; full stderr captured for the
   empty case (mechanism: 127 → `if !` → refusal branch).
6. Refusal probes on both harnesses (in-place and copies): comma, `?host=`,
   `?service=`, `?options=`, `POSTGRES://`, host-suffix, TAB, CRLF via `OLD_URL`
   and `NEW_URL`, plus four port-argument injections — all exit 1 + `refusing:`,
   marker never written; valid-port control reached the stub (rc=97, marker set).
7. `C_URL`/`A_URL` derivation invariance probe (pure string ops + guard re-check).
8. `shellcheck guard.sh guard_test.sh prove.sh perf.sh` — clean.
9. Read-only: `git diff main...HEAD` (7 files, +193/−14); `diff -u` of the two
   templates + `od` trailing-byte check; check-name list diff main vs HEAD
   (identical, 82 named + 1 manual = 83); libpq `fe-connect.c` (REL_18_STABLE)
   `conninfo_uri_parse_options` / `conninfo_uri_decode` / `conninfo_storeval` /
   `uri_prefix_length` / `conninfo_add_defaults` / `connectOptions2` and the
   libpq-connect docs (URI percent-encoding, multi-host fall-through).

No file was edited except this review record; no commits, no git config changes.
