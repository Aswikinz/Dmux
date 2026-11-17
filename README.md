<p align="center">
  <img src="src-tauri/icons/128x128.png" alt="Dmux app icon" width="112" />
</p>

<h1 align="center">Dmux</h1>

<p align="center">
  <strong>Prove a PostgreSQL migration copied every row and every value.</strong><br />
  A desktop app and a command-line runner. No SQL needed.
</p>

<p align="center">
  <a href="https://github.com/Aswikinz/Dmux/releases/latest"><strong>Download</strong></a> ·
  <a href="docs/guide/user-guide.md">User guide</a> ·
  <a href="docs/guide/dba-setup.md">For the database administrator</a> ·
  <a href="docs/how-dmux-decides.md">How Dmux decides</a>
</p>

<p align="center">
  <a href="https://github.com/Aswikinz/Dmux/actions/workflows/ci.yml"><img src="https://github.com/Aswikinz/Dmux/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
  <a href="https://github.com/Aswikinz/Dmux/actions/workflows/codeql.yml"><img src="https://github.com/Aswikinz/Dmux/actions/workflows/codeql.yml/badge.svg" alt="CodeQL" /></a>
  <a href="https://codecov.io/gh/Aswikinz/Dmux"><img src="https://codecov.io/gh/Aswikinz/Dmux/branch/main/graph/badge.svg" alt="codecov" /></a>
  <a href="https://sonarcloud.io/summary/new_code?id=Aswikinz_Dmux"><img src="https://sonarcloud.io/api/project_badges/measure?project=Aswikinz_Dmux&metric=alert_status" alt="Quality Gate Status" /></a>
</p>

Dmux connects **read-only** to an old and a new PostgreSQL database, compares their
structure and their data, and produces a report a team can sign. It was started for a
core banking system upgrade, from Postgres 9.x to a current release, but holds nothing
specific to any one system.

- **Every row, every value.** Each table gets a fingerprint of every value, compared
  bucket by bucket, so row order and server sort order never matter.
- **Never a false PASS.** Anything Dmux cannot prove is reported as NEEDS REVIEW,
  CHANGED DURING CHECK or TOOL FAULT, never as PASS.
- **Safe on production.** Read-only transactions, a SELECT-only login, superuser logins
  refused unless you record a reason, and verified encryption by default.
- **Plain words.** A guided seven-step flow, a verdict stated in one sentence, and a PDF
  and Excel report.

## Install

Download the file for your computer from the
[**latest release**](https://github.com/Aswikinz/Dmux/releases/latest).

| Your computer | Download | How to install |
| --- | --- | --- |
| Windows 10 or 11 | `Dmux_<version>_x64-setup.exe` | Double-click it. It installs for your user only and needs no administrator rights. It downloads WebView2 if the PC lacks it. |
| Windows, no internet | `Dmux_<version>_x64-offline-setup.exe` | The same installer, carrying Microsoft's WebView2 component for PCs that cannot download it. About 130 MB larger. |
| Windows, locked down | `Dmux_<version>_x64-portable.zip` | Unzip anywhere and run `dmux-app.exe`. No install, no administrator rights. Needs WebView2 already on the PC (see `README.txt` in the zip). |
| Windows, managed by IT | `Dmux_<version>_x64_en-US.msi` | For central deployment by your IT team. |
| Ubuntu or Debian | `Dmux_<version>_amd64.deb` | `sudo apt install ./Dmux_<version>_amd64.deb` |
| Fedora or RHEL | `Dmux-<version>-1.x86_64.rpm` | `sudo dnf install ./Dmux-<version>-1.x86_64.rpm` |
| Any Linux, sandboxed | `Dmux_<version>_x86_64.flatpak` | `flatpak install --user ./Dmux_<version>_x86_64.flatpak`, then run `flatpak run verify.dmux.desktop`. |
| Any Linux, no install | `Dmux_<version>_amd64.AppImage` | `chmod +x Dmux_*.AppImage`, then run it. Needs FUSE 2. |
| A server, command line only | `dmux-x86_64-unknown-linux-musl` or `dmux-x86_64-pc-windows-msvc.exe` | Copy it to the server and run it; nothing else to install. |

**Windows warns about an unknown publisher.** The installers are not code-signed yet.
Choose **More info → Run anyway** only after checking the download as below.

**Check your download (recommended).** Every release carries `SHA256SUMS`, signed with
Sigstore. On Linux, in the folder with your download:

```bash
sha256sum -c SHA256SUMS --ignore-missing
```

On Windows, in PowerShell, compare the printed value with the line for your file in
`SHA256SUMS`:

```powershell
Get-FileHash .\Dmux_<version>_x64-setup.exe -Algorithm SHA256
```

The full steps, including checking the signature, are in
[Verifying a Dmux download](docs/guide/verify-download.md).

## Use it in five minutes

**1. Ask your database administrator for a read-only login.** They run one script on
each database ([details](docs/guide/dba-setup.md)):

```bash
psql -d bank -v pw="'choose-a-strong-password'" -v schema=public \
     -f docs/dba/create_dmux_reader.sql
```

**2. Open Dmux and connect.** Enter the old and the new database, type the password,
and choose **Test connection** on each. Dmux shows **Connected** and what it checked.
Passwords go to your system keychain; on a computer without one, Dmux keeps the
password in memory until you close the app.

**3. Compare structure, then map.** Dmux lists every table, column, index and constraint
side by side (red is missing, green is added, amber is changed, with the old and new
type). On **Map**, approve or ignore the few pairings it could not make on its own;
every ignored item needs a reason, which goes into the report.

**4. Choose checks and run.** Pick a preset, or set each table to **Structure only**,
**Counts and totals**, or **Full values**. Dmux shows how long it expects to take. Start
the run; you can pause or cancel it at any time, and the verdict appears as soon as it
ends.

**5. Read the results and save the report.** The verdict comes first, then the coverage
line, everything that was ignored and why, and a card for each failing table. On
**Report**, save a PDF summary to sign and an Excel workbook with the detail.

### What the verdicts mean

| Verdict | Meaning |
| --- | --- |
| **PASS** | Every checked table matched at its chosen level. Read the coverage line before signing. |
| **PASS (with options)** | It matched, but only under comparison options someone ticked (listed next to it). |
| **FAIL** | The data differs. Dmux says which table, by how much, and which columns. |
| **WIDELY DIFFERENT** | So much differs that Dmux shows a sample instead of every row. |
| **NEEDS REVIEW** | Values matched, but a person must confirm something Dmux could not prove. |
| **CHANGED DURING CHECK** | Someone changed the table while Dmux read it. Run it again on quiet data. |
| **TOOL FAULT** | Dmux could not decide. This says nothing about your data. |
| **WARNING** | No difference found, but something needs your attention. |

[How Dmux decides](docs/how-dmux-decides.md) explains the method and its limits in plain
language.

## On a server: the command line

The same engine runs on a jump server. Put the connection in a small definition file
(passwords come from environment variables, never from the file):

```toml
# run.toml
level = "full-values"
old = { host = "old-db.internal", port = 5432, db = "bank", user = "dmux_reader", password_env = "DMUX_OLD_PW" }
new = { host = "new-db.internal", port = 5432, db = "bank", user = "dmux_reader", password_env = "DMUX_NEW_PW", ca_file = "/etc/ssl/bank-ca.pem" }
```

```bash
dmux run run.toml --out results.json      # exit code 0 = PASS, 1 = FAIL, 2 = warnings
dmux report --results results.json --pdf report.pdf --xlsx report.xlsx
```

`dmux capture` and `dmux compare` save a database's fingerprint to a file and compare it
later, for upgrades where both databases are never online together. See the
[user guide](docs/guide/user-guide.md) for every option.

## Build from source

You need **Rust 1.100.0** (pinned by `rust-toolchain.toml`), **Node.js 26** and
**pnpm 12.21.0**. On Linux also the Tauri 2 build dependencies
(`libwebkit2gtk-4.1-dev`, `build-essential`, `curl`, `wget`, `file`, `libxdo-dev`,
`libssl-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`); on Windows, Visual
Studio Build Tools with the C++ workload.

```bash
pnpm install
pnpm tauri build              # release app with installers, in target/release/bundle/
cargo build --release -p dmux-cli   # the dmux command-line runner
```

For development:

```bash
pnpm dev                      # UI on http://localhost:5173 with a demo engine
cargo test --workspace        # engine, CLI and app tests
pnpm lint && pnpm test        # ESLint and Vitest
pnpm test:ui                  # Playwright UI and accessibility tests
```

Engine tests that need real servers use the local Podman databases:
`podman compose -f testdata/compose.yml up -d` (Postgres 9.6 and 17; add
`--profile matrix` for 9.4, 12 and 18). The end-to-end tests drive the release
binary through `tauri-driver`:

```bash
cargo install tauri-driver --version 2.1.0 --locked
pnpm tauri build --no-bundle
xvfb-run -a pnpm test:e2e     # Linux; on Windows just: pnpm test:e2e
```

## Releases

Pushing a `v*` tag runs `.github/workflows/release.yml`. It checks the tag against the
app versions, builds the Linux and Windows installers and the static command-line
binaries, writes SHA-256 checksums signed with Sigstore, attaches CycloneDX SBOMs, and
gives every file a build-provenance attestation. See [`CHANGELOG.md`](CHANGELOG.md) for
what changed, and [`SECURITY.md`](SECURITY.md) to report a vulnerability.

## Repository layout

```
crates/dmux-engine/   comparison engine (plain Rust library, no UI code)
crates/dmux-cli/      the dmux command-line runner
crates/dmux-report/   PDF and Excel reports
crates/dmux-hooks/    optional Python custom checks
src-tauri/            the Tauri 2 desktop app
src/                  React UI (TypeScript, Vite, Tailwind)
e2e/                  UI, accessibility and real-app end-to-end tests
testdata/             test databases and the answer key
docs/                 guides, the plan, the task board and the decisions log
scripts/              worktree helpers and CI checks
```

## Licence

Apache-2.0. See [`LICENSE`](LICENSE). Third-party components (notably the embedded
Liberation fonts) are listed in [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
