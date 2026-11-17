<p align="center">
  <img src="src-tauri/icons/128x128.png" alt="Dmux app icon" width="128" />
</p>

# Dmux

[![CI](https://github.com/Aswikinz/Dmux/actions/workflows/ci.yml/badge.svg)](https://github.com/Aswikinz/Dmux/actions/workflows/ci.yml)
[![CodeQL](https://github.com/Aswikinz/Dmux/actions/workflows/codeql.yml/badge.svg)](https://github.com/Aswikinz/Dmux/actions/workflows/codeql.yml)
[![codecov](https://codecov.io/gh/Aswikinz/Dmux/branch/main/graph/badge.svg)](https://codecov.io/gh/Aswikinz/Dmux)
[![Quality Gate Status](https://sonarcloud.io/api/project_badges/measure?project=Aswikinz_Dmux&metric=alert_status)](https://sonarcloud.io/summary/new_code?id=Aswikinz_Dmux)

Dmux proves a PostgreSQL migration copied every row and value. It connects
read-only to an old and a new database, compares their structure and data, and
produces a report a team can sign. It was started for a core banking system
upgrade, from Postgres 9.x to a current release, but the product holds nothing
specific to any one system. Its users do not write SQL.

**Status: Phase 0 skeleton.** This repository currently holds an empty Tauri 2
app, an engine library stub and a command-line stub (`dmux capture | compare |
run`, all exit 2 with "not implemented in Phase 0"). The comparison method
itself is being proven against real Postgres 9.4 to 18 servers in
`testdata/phase0/` before any product logic is written.

## Prerequisites

- **Rust 1.100.0** — pinned by `rust-toolchain.toml`; rustup installs it
  automatically.
- **Node.js 26** and **pnpm 12.21.0** — pinned by the `packageManager` field in
  `package.json`.
- **Linux:** the Tauri 2 build dependencies (`libwebkit2gtk-4.1-dev`,
  `build-essential`, `curl`, `wget`, `file`, `libxdo-dev`, `libssl-dev`,
  `libayatana-appindicator3-dev`, `librsvg2-dev`). For the smoke test also
  `webkit2gtk-driver` (WebKitWebDriver) and `xvfb`.
- **Windows:** Visual Studio Build Tools with the C++ workload and the WebView2
  runtime (preinstalled on Windows 10/11 and GitHub's runners).
- **Test databases (optional, for the Phase 0 proofs):** rootless Podman,
  `podman compose -f testdata/compose.yml up -d`.

## Build and test

```bash
pnpm install                  # frontend dependencies (frozen lockfile in CI)
cargo test --workspace        # engine, CLI and app unit tests
pnpm lint                     # ESLint
pnpm test                     # frontend unit tests (Vitest)
pnpm build                    # typecheck + build the UI into dist/
cargo build --workspace       # compile check; the app binary it builds is DEV mode
pnpm dev                      # UI dev server on http://localhost:5173
```

`cargo build --workspace` builds the desktop app in **dev mode**: the debug
binary (`target/debug/dmux-app`) embeds no UI and loads it from the dev server
(`pnpm dev`, http://localhost:5173) at runtime. The app that ships is built by
`pnpm tauri build` — a release binary (`target/release/dmux-app`) with the UI
embedded, plus the installers. Add `--no-bundle` to build just the release
binary without installers.

End-to-end smoke test (needs `tauri-driver` 2.1.0 on PATH, plus WebKitWebDriver
on Linux or an auto-managed msedgedriver on Windows). It drives the **release**
binary, so build it first; set `DMUX_APP_BINARY` to test a different binary:

```bash
cargo install tauri-driver --version 2.1.0 --locked
pnpm tauri build --no-bundle    # release app with the UI embedded
xvfb-run -a pnpm test:e2e       # Linux; on Windows just: pnpm test:e2e
```

CI (`.github/workflows/ci.yml`) runs all of the above on Ubuntu 22.04 and
Windows for every push and pull request, with every action pinned to a commit
SHA. It also uploads Rust (`cargo llvm-cov --workspace --lcov`) and frontend
(`pnpm test --coverage`) coverage to Codecov and scans with SonarCloud.
Coverage thresholds are not enforced yet — the Phase 0 skeleton predates the
real engine and UI code; PLAN.md section 9's gates arrive with them.

## Releases

Pushing a `v*` tag runs `.github/workflows/release.yml`: it checks the tag
against the versions in `src-tauri/tauri.conf.json` and `package.json`, builds
the Linux (deb, rpm, AppImage) and Windows (NSIS, MSI) installers plus static
CLI binaries, attaches SHA-256 checksums and CycloneDX SBOMs, and publishes the
release. Installers are **unsigned** until SignPath approval, so Windows
SmartScreen warns; verify downloads against `SHA256SUMS`.

## Layout

```
crates/dmux-engine/   comparison engine (plain Rust library, no UI code)
crates/dmux-cli/      the dmux command-line runner
src-tauri/            the Tauri 2 desktop app
src/                  React UI (TypeScript, Vite, Tailwind)
e2e/                  WebdriverIO smoke test driving the real app
testdata/             test databases and the Phase 0 proofs
docs/                 the plan, the task board and the decisions log
scripts/              worktree helpers and CI checks
```

## Where to read more

- `docs/PLAN.md` — the full specification: what a PASS means, the comparison
  method, the safety rules and the build phases.
- `docs/DECISIONS.md` — decisions already made.
- `SETUP.md` — local setup and the agent workflow.

## Licence

Apache-2.0. See `LICENSE`. Third-party components (notably the embedded
Liberation fonts) are listed with their licences in
[`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
