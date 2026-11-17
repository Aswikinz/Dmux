# Third-party notices

Dmux redistributes the following third-party software and data. Each item is
listed with the licence under which it may be copied, modified and distributed.

## Liberation Sans fonts

Dmux's report generator (`crates/dmux-report`) embeds subsets of the Liberation
Sans fonts to draw the PDF summary, so a generated report opens identically on
any machine without the font installed.

- **Files:** `crates/dmux-report/fonts/LiberationSans-Regular.ttf`,
  `crates/dmux-report/fonts/LiberationSans-Bold.ttf`
- **Licence:** SIL Open Font License, Version 1.1 (OFL-1.1)
- **Licence text:** `crates/dmux-report/fonts/OFL-1.1.txt`
- **Upstream:** The Liberation Fonts project (Red Hat, Inc.; digitized data
  copyright 2010 Google Corporation, with reserved font name Arimo/Tinos/Cousine).

The full OFL-1.1 licence text is distributed alongside the font files and, in
the desktop installers, is bundled into the application resources so it ships
with every build. The OFL is a permissive font licence: it permits embedding in
documents and redistribution, subject only to the reserved-name and
no-sale-on-its-own terms in the licence text.

## Rust crate dependencies

Dmux's Rust workspace uses a number of open-source crates, resolved and pinned
in `Cargo.lock`. Their licences are enforced by `cargo deny check licenses`
(`deny.toml`): every dependency is Apache-2.0, MIT or another OSI-approved
permissive licence (ISC, BSD, CC0, Zlib, MPL-2.0, Unicode-3.0, CDLA-Permissive
and the like). The report generator depends on `genpdf` and `printpdf`
(Apache-2.0 OR MIT / MIT) for PDF and `rust_xlsxwriter` (MIT OR Apache-2.0) for
Excel.

## Frontend dependencies

See `package.json` and `pnpm-lock.yaml`; frontend dependency licences are
permissive-OSI and audited in CI (`pnpm audit`, and the deny/cargo-audit for the
Rust side).

## CPython (Python for custom checks, Windows builds)

The Windows installers (setup, offline setup, MSI) and the portable zip carry a
private copy of Python, in a `python\` folder next to the app, so custom checks
(hooks) run without a Python installation. The Linux Flatpak uses the Python in
its runtime instead, and the other Linux packages use the system Python.

- **What:** the official CPython 3.12.10 "Windows embeddable package (64-bit)",
  `python-3.12.10-embed-amd64.zip` from python.org. It is downloaded during the
  release build at that exact version and verified against a pinned SHA-256; it
  is not stored in this repository.
- **Licence:** Python Software Foundation License Version 2 (PSF-2.0). The full
  text is the `LICENSE.txt` inside the `python\` folder of every Windows package.
- **Bundled components:** the package also contains OpenSSL 3 (Apache-2.0),
  libffi (MIT), SQLite (public domain), expat (MIT), bzip2 (bzip2 licence),
  xz/liblzma (public domain), zlib (zlib licence), mpdecimal (BSD-2-Clause) and
  the Microsoft Visual C++ runtime DLLs (redistributable under Microsoft's
  terms). python.org publishes a software bill of materials for it; the release
  attaches that file as `sbom-cpython-windows-embed.spdx.json`.
- **Upstream:** the Python Software Foundation, https://www.python.org/
