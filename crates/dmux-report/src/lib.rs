//! Dmux report generation (PLAN.md 4.7 and 5).
//!
//! This crate turns a [`dmux_engine::results::RunResults`] (already written to a
//! JSON file by `dmux compare` / `dmux run`) into the three artefacts a bank
//! signs and audits:
//!
//! - a PDF summary (`pdf`): page 1 carries the verdict, the coverage line, every
//!   ignored item with its reason, failures first, the run/tool/mapping identity,
//!   the SHA-256 of the results file, the "what a PASS does not prove" paragraph,
//!   the md5 threat-model sentence and the tamper-evident-not-proof sentence, and
//!   a sign-off block.  Per-table sections follow.
//! - an Excel detail workbook (`xlsx`): sheets Summary, Tables, Differences,
//!   Ignored, Warnings/Flags and Run log; lists longer than Excel's 1,048,576
//!   row limit overflow to CSV, noted in the workbook.
//! - a run log (`log`): plain text and JSON.
//!
//! The engine's report contract is that **no row value ever leaves the database**:
//! `RunResults` carries counts, hashes and names only, never row data.  This crate
//! is a second line of defence: the layout module below contains the only place a
//! value is joined to text, and it renders only summary fields, never row values
//! or — unless `include_keys` is set — keys.  Tests feed sentinel row values into
//! the fixture and assert they never appear in the PDF text or the XLSX XML.

pub mod layout;
pub mod log;
pub mod pdf;
pub mod v2;
pub mod verify;
pub mod xlsx;

pub use layout::{IncludeKeys, ReportOptions};
pub use verify::{verify_results_file, Verification};

/// The three artefacts `dmux report` writes, in one call.
pub fn render_all(
    results: &dmux_engine::results::RunResults,
    raw: &serde_json::Value,
    file_sha256: &str,
    options: &ReportOptions,
    pdf_path: &std::path::Path,
    xlsx_path: &std::path::Path,
    log_path: &std::path::Path,
) -> Result<(), ReportError> {
    pdf::write_summary(results, raw, file_sha256, options, pdf_path)?;
    xlsx::write_detail(results, raw, file_sha256, options, xlsx_path)?;
    log::write_run_log(results, raw, options, log_path)?;
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmux_engine::results::{
        finalise, CheckLevel, IgnoredItem, RunResults, TableResult, Verdict, RESULTS_VERSION,
    };
    use std::collections::BTreeMap;

    /// An answer-key-like run: planted failures (12 deleted rows, 3 duplicate
    /// rows, one changed balance), a PASSED table, an ignored table with its
    /// required reason, and run-level warnings / a change-during-check note.
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
            },
        );

        finalise(RunResults {
            results_version: RESULTS_VERSION,
            run_id: "run-golden-1".into(),
            tool_version: "0.1.0".into(),
            mapping_hash: "mapping-hash-abc123".into(),
            verdict: Verdict::Fail,
            coverage_line: "3 of 4 tables fully checked, 0 counts only, 1 ignored".into(),
            fully_checked: 3,
            counts_only: 0,
            ignored: 1,
            tables,
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
            results_sha256: String::new(),
        })
    }

    fn temp_dir() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("dmux-report-golden-{}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        p
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
        };
        render_all(
            &results,
            &verification.raw,
            &verification.sha256,
            &options,
            &pdf,
            &xlsx,
            &log,
        )
        .unwrap();

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
    }

    #[test]
    fn golden_output_is_deterministic() {
        let results = golden_results();
        let options = ReportOptions {
            include_keys: false,
        };
        let layout = layout::build(&results, &options);

        let p1 = pdf::render_bytes(&layout, "abc").unwrap();
        let p2 = pdf::render_bytes(&layout, "abc").unwrap();
        assert_eq!(p1, p2, "PDF must be byte-deterministic");

        let mut b1 = xlsx::build_workbook(&layout, xlsx::EXCEL_MAX_ROWS).unwrap();
        let mut b2 = xlsx::build_workbook(&layout, xlsx::EXCEL_MAX_ROWS).unwrap();
        let x1 = b1.workbook.save_to_buffer().unwrap();
        let x2 = b2.workbook.save_to_buffer().unwrap();
        assert_eq!(x1, x2, "XLSX must be byte-deterministic");

        let l1 = log::text_log(&results, &serde_json::Value::Null, &options);
        let l2 = log::text_log(&results, &serde_json::Value::Null, &options);
        assert_eq!(l1, l2, "log must be deterministic");
    }
}
