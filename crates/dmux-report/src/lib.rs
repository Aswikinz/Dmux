//! Dmux report generation (PLAN.md 4.7 and 5).
//!
//! This crate turns a verified [`dmux_engine::results::RunResults`] (written to a
//! JSON file by `dmux compare` / `dmux run`, with or without `--hooks`) into the
//! three artefacts a bank signs and audits:
//!
//! - a PDF summary (`pdf`): page 1 carries the verdict, the coverage line, every
//!   ignored item with its reason, failures first, structure findings, the
//!   run/tool/mapping identity, the SHA-256 of the results file, the "what a PASS
//!   does not prove" paragraph, the md5 threat-model sentence, the
//!   hash-integrity sentence (audit M1 / decision 23), the hook custom checks and their script
//!   hashes, and a sign-off block.  Per-table sections follow.
//! - an Excel detail workbook (`xlsx`): sheets Summary, Tables, Differences,
//!   Structure Findings, Ignored, Warnings Flags, Custom Checks and Run log;
//!   lists longer than Excel's 1,048,576 row limit overflow to CSV, noted in the
//!   workbook.
//! - a run log (`log`): plain text and JSON.
//! - an interactive HTML report (`html`): one offline file with the PDF's
//!   content plus search, filters, sorting and expandable per-table detail.
//!
//! The engine's report contract is that **no row value ever leaves the database**:
//! `RunResults` carries counts, hashes and names only, never row data.  This crate
//! is a second line of defence: the layout module renders only summary fields,
//! never row values or — unless `include_keys` is set — keys.  Tests feed sentinel
//! row values into the fixture and assert they never appear in the PDF text or
//! the XLSX XML.

pub mod html;
pub mod layout;
pub mod log;
pub mod pdf;
pub mod verify;
pub mod xlsx;

pub use layout::{IncludeKeys, LeaveOut, ReportOptions};
pub use verify::{verify_results_file, HookEvidence, Verification};

/// Build the layout for verified results, folding in hook evidence and the
/// displayed verdict (the worse of the inner verdict and every hook finding).
pub fn build_layout(verification: &Verification, options: &ReportOptions) -> layout::Layout {
    let mut layout = layout::build(&verification.results, verification.hooks.as_ref(), options);
    // The displayed overall verdict must never show PASS when a custom check
    // failed: fold hook findings over the inner verdict.
    if let Some(hooks) = &verification.hooks {
        layout.verdict_word =
            layout::verdict_word(verify::displayed_verdict(&verification.results, hooks))
                .to_string();
    }
    layout
}

/// The three artefacts `dmux report` writes, in one call (test/golden entry).
pub fn render_all(
    verification: &Verification,
    options: &ReportOptions,
    pdf_path: &std::path::Path,
    xlsx_path: &std::path::Path,
    log_path: &std::path::Path,
) -> Result<(), ReportError> {
    let layout = build_layout(verification, options);
    pdf::write_summary_layout(&layout, &verification.sha256, pdf_path)?;
    xlsx::write_detail_layout(&layout, &verification.sha256, xlsx_path)?;
    log::write_run_log_locked(
        &verification.results,
        verification.hooks.as_ref(),
        options,
        log_path,
    )?;
    Ok(())
}

/// Errors reporting can hit: I/O, a bad results file, or a renderer failure.
#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    #[error("results file error: {0}")]
    Verify(#[from] verify::VerifyError),
    #[error("PDF render error: {0}")]
    Pdf(#[from] pdf::PdfError),
    #[error("Excel render error: {0}")]
    Xlsx(#[from] xlsx::XlsxError),
    #[error("log error: {0}")]
    Log(#[from] log::LogError),
    #[error("HTML render error: {0}")]
    Html(#[from] html::HtmlError),
}

/// Write `bytes` to `path` with owner-only permissions on Unix (audit L3: a
/// report is a confidential artefact, so it must never be world-readable).
/// On non-Unix platforms the mode defaults to the OS and the umask.
pub(crate) fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true).mode(0o600);
        opts.open(path)?.write_all(bytes)
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmux_engine::results::{
        finalise, CheckLevel, IgnoredItem, RunResults, TableResult, Verdict, RESULTS_VERSION,
    };
    use std::collections::BTreeMap;

    fn golden_results() -> RunResults {
        let mut tables = BTreeMap::new();
        tables.insert(
            "demo_bank.transaction_history".into(),
            TableResult {
                table: "demo_bank.transaction_history".into(),
                level: CheckLevel::FullValues,
                verdict: Verdict::Fail,
                old_rows: Some(1000),
                new_rows: Some(988),
                columns_covered: Some(vec!["txn_id".into(), "amount".into()]),
                differing_buckets: Some(1),
                missing_buckets: Some(1),
                extra_buckets: Some(0),
                missing_rows: Some(12),
                extra_rows: Some(0),
                changed_rows: Some(0),
                reasons: vec!["row counts differ".into(), "12 rows missing".into()],
                column_notes: vec![],
            },
        );
        tables.insert(
            "demo_bank.no_key_duplicates".into(),
            TableResult {
                table: "demo_bank.no_key_duplicates".into(),
                level: CheckLevel::FullValues,
                verdict: Verdict::Fail,
                old_rows: Some(50),
                new_rows: Some(53),
                columns_covered: Some(vec!["payload".into()]),
                differing_buckets: Some(1),
                missing_buckets: Some(0),
                extra_buckets: Some(1),
                missing_rows: Some(0),
                extra_rows: Some(3),
                changed_rows: Some(0),
                reasons: vec!["3 duplicate rows on the new side".into()],
                column_notes: vec![],
            },
        );
        tables.insert(
            "demo_bank.accounts".into(),
            TableResult {
                table: "demo_bank.accounts".into(),
                level: CheckLevel::FullValues,
                verdict: Verdict::Fail,
                old_rows: Some(20000),
                new_rows: Some(20000),
                columns_covered: Some(vec!["acct_id".into(), "balance".into()]),
                differing_buckets: Some(1),
                missing_buckets: Some(0),
                extra_buckets: Some(0),
                missing_rows: Some(0),
                extra_rows: Some(0),
                changed_rows: Some(1),
                reasons: vec![
                    "bucket summaries differ".into(),
                    "control totals differ".into(),
                ],
                column_notes: vec![],
            },
        );
        tables.insert(
            "demo_bank.loans".into(),
            TableResult {
                table: "demo_bank.loans".into(),
                level: CheckLevel::FullValues,
                verdict: Verdict::Pass,
                old_rows: Some(500),
                new_rows: Some(500),
                columns_covered: Some(vec!["loan_id".into(), "principal".into()]),
                differing_buckets: Some(0),
                missing_buckets: Some(0),
                extra_buckets: Some(0),
                missing_rows: Some(0),
                extra_rows: Some(0),
                changed_rows: Some(0),
                reasons: vec![],
                column_notes: vec![],
            },
        );

        finalise(RunResults {
            results_version: RESULTS_VERSION,
            run_id: "run-golden-1".into(),
            tool_version: "0.1.0".into(),
            mapping_hash: "mapping-hash-abc123".into(),
            verdict: Verdict::Fail,
            coverage_line:
                "3 of 4 tables fully checked, 0 counts only, 0 structure only, 1 ignored".into(),
            fully_checked: 3,
            counts_only: 0,
            ignored: 1,
            structure_only: 0,
            in_scope: 4,
            structure_findings: vec![dmux_engine::results::StructureFinding {
                object: "demo_bank.accounts.idx_missing".into(),
                path: vec!["demo_bank".into(), "accounts".into(), "idx_missing".into()],
                verdict: Verdict::Fail,
                message: "index missing in the new database".into(),
            }],
            definition_hash: "definition-hash-golden".into(),
            tables,
            details: std::collections::BTreeMap::from([(
                "demo_bank.transaction_history".into(),
                dmux_engine::results::TableDetail {
                    changed_columns: vec!["amount".into()],
                    changed_rows: vec![],
                    missing_keys: None,
                    extra_keys: None,
                    control_totals: vec![],
                    plain_words: vec![],
                },
            )]),
            ignored_items: vec![IgnoredItem {
                path: "demo_bank.archived".into(),
                reason: "out of scope: not migrated".into(),
            }],
            changed_during_check: vec!["demo_bank.audit_log".into()],
            warnings: vec![
                "old database is SQL_ASCII; never a clean PASS on encoding".into(),
                "1 table(s) changed during check".into(),
            ],
            wal_start_lsn: Some("0/16B6D40".into()),
            wal_end_lsn: Some("0/16B6E80".into()),
            ..RunResults::default()
        })
    }

    fn temp_dir() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("dmux-report-golden-{}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// A run with one data difference, one structure difference (with a FAIL
    /// and a WARNING structure finding), one pass and two run warnings.
    fn mixed_results() -> RunResults {
        let row = |name: &str, verdict: Verdict, changed: usize| TableResult {
            table: name.into(),
            level: CheckLevel::FullValues,
            verdict,
            old_rows: Some(10),
            new_rows: Some(10),
            columns_covered: None,
            differing_buckets: Some(changed),
            missing_buckets: Some(0),
            extra_buckets: Some(0),
            missing_rows: Some(0),
            extra_rows: Some(0),
            changed_rows: Some(changed),
            reasons: vec![format!("reason for {name}")],
            column_notes: vec![],
        };
        let finding = |object: &str, verdict: Verdict| dmux_engine::results::StructureFinding {
            object: object.into(),
            path: vec![],
            verdict,
            message: format!("finding on {object}"),
        };
        finalise(RunResults {
            verdict: Verdict::Fail,
            tables: BTreeMap::from([
                (
                    "core.balances".into(),
                    row("core.balances", Verdict::Fail, 2),
                ),
                ("core.dropped".into(), row("core.dropped", Verdict::Fail, 0)),
                ("core.ok".into(), row("core.ok", Verdict::Pass, 0)),
            ]),
            structure_findings: vec![
                finding("core.dropped.legacy_flag", Verdict::Fail),
                finding("public.some_function", Verdict::Warning),
            ],
            warnings: vec!["run warning one".into(), "run warning two".into()],
            ..RunResults::default()
        })
    }

    fn leave_all() -> ReportOptions {
        ReportOptions {
            leave_out: LeaveOut {
                structure: true,
                other: true,
                warnings: true,
                passing: true,
            },
            ..Default::default()
        }
    }

    #[test]
    fn data_differences_are_never_left_out_and_page_one_names_what_was() {
        let v = verify::verify_results(&mixed_results()).unwrap();
        let full = build_layout(&v, &ReportOptions::default());
        assert_eq!(full.tables.len(), 3);
        assert!(full.omitted.is_empty());
        // Data differences first.
        assert_eq!(full.tables[0].table, "core.balances");

        let trimmed = build_layout(&v, &leave_all());
        let names: Vec<&str> = trimmed.tables.iter().map(|t| t.table.as_str()).collect();
        assert_eq!(names, vec!["core.balances"]);
        assert!(trimmed.structure_findings.is_empty());
        assert!(trimmed.warnings.is_empty());
        assert_eq!(
            trimmed.omitted,
            vec![
                "1 table with a structure difference",
                "1 table that passed",
                "2 run warnings",
                "2 structure findings",
            ]
        );
        // The verdict and the tiles still count everything.
        assert_eq!(trimmed.verdict_word, "FAIL");
        assert_eq!(
            (trimmed.data_issue_tables, trimmed.other_issue_tables),
            (1, 1)
        );
    }

    #[test]
    fn leaving_out_warnings_keeps_failures_of_every_kind() {
        let v = verify::verify_results(&mixed_results()).unwrap();
        let opts = ReportOptions {
            leave_out: LeaveOut {
                warnings: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let l = build_layout(&v, &opts);
        assert_eq!(l.tables.len(), 3);
        let objects: Vec<&str> = l
            .structure_findings
            .iter()
            .map(|f| f.object.as_str())
            .collect();
        assert_eq!(objects, vec!["core.dropped.legacy_flag"]);
        assert_eq!(l.omitted, vec!["2 run warnings", "1 structure finding"]);
    }

    #[test]
    fn the_leave_out_list_parses_and_data_cannot_be_left_out() {
        assert_eq!(LeaveOut::parse("").unwrap(), LeaveOut::default());
        let l = LeaveOut::parse("structure, warnings").unwrap();
        assert!(l.structure && l.warnings && !l.other && !l.passing);
        assert!(LeaveOut::parse("data").unwrap_err().contains("always"));
        assert!(LeaveOut::parse("rows").is_err());
    }

    #[test]
    fn the_pdf_and_html_put_data_differences_first_and_name_what_was_left_out() {
        let v = verify::verify_results(&mixed_results()).unwrap();
        let l = build_layout(&v, &leave_all());
        let html = html::render(&l, "x");
        let data_at = html.find("<h2>Data differences</h2>").unwrap();
        let other_at = html.find("<h2>Other failures</h2>").unwrap();
        assert!(data_at < other_at);
        assert!(html.contains(
            "Left out of this report when it was made: 1 table with a structure difference"
        ));
        assert!(!html.contains("run warning one"));

        let mut warnings = Vec::new();
        let doc = printpdf::PdfDocument::parse(
            &pdf::render_bytes(&l, "x").unwrap(),
            &printpdf::PdfParseOptions::default(),
            &mut warnings,
        )
        .unwrap();
        let text = doc
            .extract_text()
            .into_iter()
            .map(|p| p.join("\n"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Left out of this report"), "{text}");
        let d = text.find("Data differences").unwrap();
        let o = text.find("Other failures").unwrap();
        assert!(d < o);
        assert!(!text.contains("run warning one"));
    }

    fn pdf_text(bytes: &[u8]) -> String {
        let mut warnings = Vec::new();
        printpdf::PdfDocument::parse(bytes, &printpdf::PdfParseOptions::default(), &mut warnings)
            .unwrap()
            .extract_text()
            .into_iter()
            .map(|p| p.join("\n"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn tables_left_out_of_a_run_go_in_their_own_report_not_on_page_one() {
        let mut r = mixed_results();
        r.ignored_items = vec![
            IgnoredItem {
                path: "core.archive".into(),
                reason: "archived before the migration".into(),
            },
            IgnoredItem {
                path: "core.left_out_one".into(),
                reason: dmux_engine::run::NOT_SELECTED_REASON.into(),
            },
            IgnoredItem {
                path: "core.left_out_two".into(),
                reason: dmux_engine::run::NOT_MAPPED_OUTSIDE_RUN.into(),
            },
        ];
        let v = verify::verify_results(&finalise(r)).unwrap();
        let l = build_layout(&v, &ReportOptions::default());
        assert_eq!(l.ignored_items.len(), 1);
        assert_eq!(l.not_in_run.len(), 2);

        let main = pdf_text(&pdf::render_bytes(&l, "x").unwrap());
        assert!(
            main.contains("core.archive"),
            "a person's ignore stays on page 1"
        );
        assert!(
            main.contains("2 tables were not part of this run"),
            "{main}"
        );
        assert!(!main.contains("core.left_out_one"));
        let html = html::render(&l, "x");
        assert!(html.contains("2 tables were not part of this run"));
        assert!(!html.contains("core.left_out_one"));

        let separate = pdf_text(&pdf::render_not_in_run_bytes(&l, "x").unwrap());
        assert!(separate.contains("Tables not in this run"));
        assert!(separate.contains("core.left_out_one"));
        assert!(separate.contains("core.left_out_two"));
        assert!(!separate.contains("core.archive"));
    }

    #[test]
    fn golden_pipeline_writes_all_three_artefacts() {
        let results = golden_results();
        let verification = verify::verify_results(&results).unwrap();
        let dir = temp_dir();
        let pdf = dir.join("report.pdf");
        let xlsx = dir.join("report.xlsx");
        let log = dir.join("report.txt");

        let options = ReportOptions {
            include_keys: false,
            ..Default::default()
        };
        render_all(&verification, &options, &pdf, &xlsx, &log).unwrap();

        let pdf_bytes = std::fs::read(&pdf).unwrap();
        let xlsx_bytes = std::fs::read(&xlsx).unwrap();
        let log_text = std::fs::read_to_string(&log).unwrap();

        assert!(pdf_bytes.starts_with(b"%PDF"), "PDF magic missing");
        assert!(xlsx_bytes.starts_with(b"PK"), "XLSX is a ZIP");
        assert!(log_text.contains("verdict: FAIL"));
        assert!(log_text.contains("run_id: run-golden-1"));
        assert!(log_text.contains("coverage: 3 of 4 tables fully checked"));
        assert!(log_text.contains("mapping_hash: mapping-hash-abc123"));
        assert!(log_text.contains("warnings: old database is SQL_ASCII"));

        // The typed v2 fields flow into the layout: a structure finding and the
        // definition hash are rendered into both artefacts.
        let layout = build_layout(&verification, &options);
        assert_eq!(layout.structure_findings.len(), 1);
        assert_eq!(layout.definition_hash, "definition-hash-golden");
    }

    #[test]
    fn a_typed_v2_file_round_trips_verification_then_renders() {
        // Produce the results file exactly as the engine does (finalise stamps
        // results_sha256 with its own `sha256_of`), write it to disk, re-read and
        // verify it, then render — the typed v2 fields must survive.
        let results = golden_results();
        let dir = temp_dir();
        let path = dir.join("engine-results.json");
        std::fs::write(&path, serde_json::to_vec_pretty(&results).unwrap()).unwrap();

        let verification = verify::verify_results_file(&path).unwrap();
        assert_eq!(verification.results.structure_findings.len(), 1);
        assert_eq!(
            verification.results.definition_hash,
            "definition-hash-golden"
        );

        let options = ReportOptions {
            include_keys: false,
            ..Default::default()
        };
        let layout = build_layout(&verification, &options);
        assert_eq!(
            layout.structure_findings[0].message,
            "index missing in the new database"
        );
        assert_eq!(layout.verdict_word, "FAIL");
    }

    #[test]
    fn golden_output_is_deterministic() {
        let results = golden_results();
        let verification = verify::verify_results(&results).unwrap();
        let options = ReportOptions {
            include_keys: false,
            ..Default::default()
        };
        let layout = build_layout(&verification, &options);

        let p1 = pdf::render_bytes(&layout, "abc").unwrap();
        let p2 = pdf::render_bytes(&layout, "abc").unwrap();
        assert_eq!(p1, p2, "PDF must be byte-deterministic");

        let mut b1 = xlsx::build_workbook(&layout, xlsx::EXCEL_MAX_ROWS).unwrap();
        let mut b2 = xlsx::build_workbook(&layout, xlsx::EXCEL_MAX_ROWS).unwrap();
        let x1 = b1.workbook.save_to_buffer().unwrap();
        let x2 = b2.workbook.save_to_buffer().unwrap();
        assert_eq!(x1, x2, "XLSX must be byte-deterministic");

        let l1 = log::text_log(&results, None, &options);
        let l2 = log::text_log(&results, None, &options);
        assert_eq!(l1, l2, "log must be deterministic");
    }

    #[cfg(unix)]
    #[test]
    fn report_artefacts_are_written_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let results = golden_results();
        let verification = verify::verify_results(&results).unwrap();
        let dir = temp_dir();
        let pdf_path = dir.join("report.pdf");
        let xlsx_path = dir.join("report.xlsx");
        let log_path = dir.join("report.txt");
        let options = ReportOptions {
            include_keys: false,
            ..Default::default()
        };
        render_all(&verification, &options, &pdf_path, &xlsx_path, &log_path).unwrap();
        for p in [&pdf_path, &xlsx_path, &log_path] {
            let mode = std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{:?} must be owner-only", p);
        }
    }

    #[test]
    fn a_hook_fail_folds_the_displayed_verdict_off_pass() {
        use crate::verify::HookFinding;
        let mut results = golden_results();
        // Force the inner verdict to PASS so only the hook keeps it off PASS.
        for t in results.tables.values_mut() {
            t.verdict = Verdict::Pass;
        }
        results.verdict = Verdict::Pass;
        let results = finalise(results);
        let hooks = HookEvidence {
            custom_checks: vec![HookFinding {
                id: "c1".into(),
                severity: "fail".into(),
                message: "custom fail".into(),
                table: None,
            }],
            ..Default::default()
        };
        let v = Verification {
            results,
            sha256: "x".into(),
            hooks: Some(hooks),
        };
        let layout = build_layout(&v, &ReportOptions::default());
        assert_eq!(layout.verdict_word, "FAIL");
    }
}
