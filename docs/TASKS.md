# Task board

Maya owns this file. Other agents read it and never edit it.

Status: todo, doing, review, done, blocked.

## Phase 0: prove the method (no product code until this phase is done)

| ID | Task | Owner | Status | Depends on |
|----|------|-------|--------|------------|
| T001 | Start the mock databases with Podman. Run `testdata/phase0/prove.sh` with old = 9.6 and new = 17. Save the output to `testdata/phase0/RESULTS.md`. | nina | done | |
| T002 | Start the `matrix` profile. Run `prove.sh` for every pair: old in {9.4, 9.6}, new in {12, 17, 18}. Add the outputs to `RESULTS.md`. | nina | done | T001 |
| T003 | Review `testdata/phase0/*.sql` for Postgres 9.4 to latest: version-specific behaviour, normalisation gaps, anything that can make equal data look different or different data look equal. | priya | done | |
| T004 | Extend `prove.sh` with the cases from T003 and PLAN.md 3.2: timestamptz, money, json and jsonb, arrays, intervals, numeric scale change, NULL bytea, empty table, a column added on the new side, and old and new databases created with different collations. | nina | done | T002, T003 |
| T005 | Time the fingerprint on 1 million and 20 million rows on both servers. Record rows per second and megabytes per second in `RESULTS.md`. | nina | done | T004 |
| T006 | Repo skeleton: cargo workspace (`dmux-engine`, `dmux-cli`), empty Tauri 2 app, pnpm. CI builds it on Ubuntu 22.04 and Windows, and one `tauri-driver` smoke test opens the window. Pin every action to a commit SHA. Include the commit-rules and protected-files checks, plus a public-clean guard whose vendor patterns come from a GitHub secret (decision 11), never from public code. Local verification (build, smoke, actionlint) in this task; CI proven green on GitHub after T009's push. | kenji | done | |
| T007 | Phase 0 gate: summarise results for the owner. Done when identical data fingerprints identically on every version pair, every planted change is reported, and both builds are green. | maya | todo | T004, T005, T006, T008, T009, T010, T011 |
| T008 | Add a Postgres 15 container to the compose matrix (owner approved: PLAN.md section 8 lists 15). Run the extended `prove.sh` for old in {9.4, 9.6} against new 15, and re-run the extended script on the other pairs so every version pair covers the new cases. Append to `RESULTS.md`. | nina | done | T004 |
| T009 | Scrub every vendor trace from all tracked files (decision 11), rewrite the unpushed history into clean commits so no trace survives in any blob, ensure AGENTS.md appears in no pushed commit (decision 12; it is already untracked/ignored/hook-blocked) and SETUP.md describes it as local-only, run the full pre-public checklist (gitleaks, filename grep, history grep incl. AGENTS.md, commit rules), then publish to GitHub after the Opus 5.5 final approval (decision 13). After publishing: watch the GitHub Actions runs (gh run list/watch), confirm every workflow triggers and completes; if any run fails, diagnose, fix (Kenji's lane for CI files), re-push and re-watch until CI is green on both platforms. Owner instruction: "after published make sure that github CI runs, if it fails fix it". | maya | doing | T006 |
| T010 | Harden the local-only URL guard in `prove.sh` AND `perf.sh` (Priya's T005 R1: the shared pattern accepts libpq multi-host comma URLs and `?host=`/`?port=` overrides that route elsewhere). Reject anything but a strict single-host local URL; add regression checks for the bypass forms. Also apply R2 (RESULTS wording: quiescence wait runs before the first timed scan of each table) and R3 (trailing newline in `fingerprint_perf.sql`). Re-run prove.sh (83/83) and a small perf.sh run to prove no regression. | nina | done | T005 |
| T011 | Harness exit-semantics hardening. The Opus T008 gate claimed prove.sh always exits 0; Maya DISPROVED it (sabotaged copy exits 1, "80 passed, 3 failed") — root cause: prove.sh lacks a trailing EOF newline, so line-based review read the echo as the last line. Fix: add trailing newlines to every harness file that lacks one; add a COMMITTED regression test (sabotage experiment: copy harness to temp, inject a version-dependent expression into a template, run against the mocks, assert exit 1 and FAIL>0, clean up) so exit-on-FAIL is permanently protected; re-run prove.sh 83/83 and guard battery 51/51. Optional cosmetic: the "empty table: one row inserted" line prints "0 bucket(s)" (display only, result correct). | nina | done | T008 |

## Phase 0 gate notes (carry-forwards for T007's summary)

- Opus T008 gate: "83 checks" includes one record-only line (lc_monetary) and one duplicated query — about 81 distinct assertions; the gate summary must say so honestly.
- Opus T008 gate: compose images use floating tags (postgres:15 etc.); exact versions are recorded in RESULTS.md; consider pinning by digest in a Phase 1 nightly task.
- Opus T008 gate carry-forward #1 (prove.sh always exits 0) was DISPROVED by Maya's sabotage run (exit 1, "80 passed, 3 failed"); root cause was the missing EOF newline misleading line-based review. T011 adds the newline and a committed exit-semantics regression test.
- Opus T011 gate (APPROVE, mutation-tested the regression test itself): Phase 1 follow-ups — (a) exit_test.sh cleanup leftovers only `warn` (lines ~125-139); make them `bad` so leftover schemas fail the test (Nina); (b) nightly workflow with mock containers for prove.sh/exit_test.sh/perf.sh, one at a time, reading each script's own exit code (Kenji); (c) engine rule: never PASS a both-empty table without an explicit count (already P5/6.10 — carry into Phase 3 briefs). Per-build CI: guard_test.sh (DB-free), shellcheck on harness scripts, EOF-newline check. T009: genericize "Opus gate" wording in RESULTS.md to "independent gate reviewer (decision 13)" during the scrub.

## Phase 1: foundation

Maya adds these tasks when the owner approves the Phase 0 gate, following PLAN.md
section 11. Keep each task small enough for one agent session.
