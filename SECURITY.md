# Security policy

## Reporting a vulnerability

Please report vulnerabilities **privately** through GitHub Security Advisories:
open this repository's **Security** tab and choose **"Report a vulnerability"**.
Do not open a public issue for a security problem.

Include what you can: steps to reproduce, the affected versions, and the impact
you observed. Never include real database credentials or customer row data in a
report — a minimal synthetic reproduction is enough.

We aim to acknowledge reports within a few days and will keep you informed as
we investigate and fix. Please give us a reasonable chance to ship a fix before
any public disclosure.

## Safety posture

Dmux verifies database migrations, so it is designed to be safe against the
databases it touches: the engine connects strictly read-only and never writes,
passwords never reach logs or reports, and row values stay on screen — reports
and exports carry counts, hashes and names, with keys included only when the
user explicitly ticks that option. The app makes no network calls except to the
databases you configure, and has no telemetry and no auto-update. The full
safety rules live in `docs/PLAN.md`, section 6.
