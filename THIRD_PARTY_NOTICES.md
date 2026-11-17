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
