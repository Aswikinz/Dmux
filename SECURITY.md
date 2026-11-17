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

## Verifying a download

Every release publishes, alongside the installers and command-line binaries:

- a **checksums file** (`SHA256SUMS`) listing the SHA-256 fingerprint of each
  file, so you can confirm a download is complete and unmodified;
- a **signature over that checksums file** (`SHA256SUMS.cosign.bundle`), made
  with a short-lived key that only this project's release workflow can obtain;
  it proves the checksums themselves were not quietly swapped after the build;
- a **build-provenance attestation** for every file, recorded on GitHub and
  stating which repository and workflow produced the file.

Installers are **not yet code-signed** (an owner decision, pending signing
approval), so Windows may show a SmartScreen warning. That does not stop you
from verifying a download with the steps below.

### 1. Verify the build provenance

With the [GitHub CLI](https://cli.github.com/) installed, run this once per
file you downloaded (put the file's name in place of `<file>`):

```bash
gh attestation verify <file> --repo Aswikinz/Dmux
```

A successful run prints the attestation details and the command's exit status is
`0`.

### 2. Verify the signature on the checksums

Download `SHA256SUMS` and `SHA256SUMS.cosign.bundle` from the same release page,
then run (with [cosign](https://github.com/sigstore/cosign) installed):

```bash
cosign verify-blob --bundle SHA256SUMS.cosign.bundle \
  --certificate-identity-regexp '^https://github.com/Aswikinz/Dmux/.github/workflows/release.yml@refs/tags/v' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  SHA256SUMS
```

This confirms the checksums were signed by Dmux's own release workflow on a
`v*` tag, not by a long-lived key someone could reuse.

### 3. Check each file's checksum

Put the files you downloaded and `SHA256SUMS` in one folder, then run:

```bash
sha256sum -c SHA256SUMS --ignore-missing
```

`--ignore-missing` lets you verify only the files you actually downloaded; every
file present is checked and the command fails if any does not match.
