# Verifying a Dmux download

This page is for anyone who downloaded Dmux and wants to be sure the file is
really the one this project published, before they install or run it. No
database or programming knowledge is needed; each command below is given in
full.

Dmux releases are not yet code-signed (that is an owner decision waiting on
signing approval), so Windows may show a SmartScreen warning. The steps here let
you check a download yourself even before signing lands.

## What a release gives you

Every release page lists the installers, the command-line program, and three
kinds of evidence you can check:

- **`SHA256SUMS`** — a checksums file. It lists a short code (a SHA-256
  fingerprint) for every file in the release. If a file has been changed even a
  little, its code no longer matches.
- **`SHA256SUMS.cosign.bundle`** — a signature over that checksums file. It is
  made with a short-lived key that only this project's release workflow can
  obtain, so it proves the checksums themselves were not quietly swapped after
  the build.
- **Build-provenance attestations** — a record on GitHub, made automatically
  for each file, saying which repository and workflow produced it.

## Check the file you downloaded

Download the files you want, plus `SHA256SUMS` and
`SHA256SUMS.cosign.bundle`, from the same release page. Put them all in one
folder.

### 1. Confirm the build provenance

Install the [GitHub CLI](https://cli.github.com/), then run this once per file
(replace `<file>` with the actual file name):

```bash
gh attestation verify <file> --repo Aswikinz/Dmux
```

If the file was really produced by this project's release workflow, the command
succeeds and prints details of the attestation.

### 2. Confirm the checksums were signed by Dmux

Install [cosign](https://github.com/sigstore/cosign), then run:

```bash
cosign verify-blob --bundle SHA256SUMS.cosign.bundle \
  --certificate-identity-regexp '^https://github.com/Aswikinz/Dmux/.github/workflows/release.yml@refs/tags/v' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  SHA256SUMS
```

This proves the checksums were signed by Dmux's own release workflow on a
versioned tag, not by a long-lived key someone could reuse.

### 3. Confirm each file matches the checksums

In the same folder, run:

```bash
sha256sum -c SHA256SUMS --ignore-missing
```

`--ignore-missing` means you can verify just the files you downloaded, instead
of the whole release. Every file present is checked, and the command fails if
any of them does not match.

If all three steps succeed, the download is what the release workflow produced.
The steps are also listed in the project's `SECURITY.md`.
