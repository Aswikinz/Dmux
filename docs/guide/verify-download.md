# Verifying a Dmux download

This page is for anyone who downloaded Dmux and wants to be sure the file is
really the one this project published, before they install or run it. No
database or programming knowledge is needed; each command below is given in
full.

Dmux installers are **unsigned** until code signing is approved, so Windows may show a
SmartScreen warning and other systems may warn about an unknown publisher. The steps here
let you check a download yourself in the meantime. They do not replace code signing, and
they only show that a file matches what this project's release process published.

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

This is the simplest check, and the only one that needs no extra software. It works on
its own if you cannot install the tools in steps 1 and 2, but then it proves only that the
file matches the `SHA256SUMS` you downloaded from the same page, so use steps 1 and 2 as
well where you can.

**Linux.** In the same folder, run:

```bash
sha256sum -c SHA256SUMS --ignore-missing
```

`--ignore-missing` means you can verify just the files you downloaded, instead
of the whole release. Every file present is checked, and the command fails if
any of them does not match.

**Windows (PowerShell).** Compute the file's SHA-256 and compare it, by eye or with
the command below, to the line for that file in `SHA256SUMS` (open it in Notepad):

```powershell
Get-FileHash .\<file> -Algorithm SHA256
Select-String -Path .\SHA256SUMS -Pattern '<file>'
```

The long code printed by `Get-FileHash` must be identical to the one beside the file name
in `SHA256SUMS` (PowerShell prints it in capitals, the file in lower case; the case does
not matter). If a single character differs, do not install the file.

If all three steps succeed, the download is what the release workflow produced.
The steps are also listed in the project's `SECURITY.md`.
