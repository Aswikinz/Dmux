//! The Excel detail workbook (PLAN.md 4.7).
//!
//! `rust_xlsxwriter` (MIT OR Apache-2.0) writes the workbook with sheets:
//! Summary, Tables, Differences (keys only when `include_keys`), Ignored,
//! Warnings Flags (the plan calls it "Warnings/Flags"; `/` is forbidden in an
//! Excel sheet name) and Run log.  When any list exceeds Excel's 1,048,576-row
//! limit, that list is diverted to a CSV next to the workbook and noted in the
//! workbook (PLAN 4.7).  No row value ever appears: the workbook holds counts,
//! names, hashes and reasons only, and keys only when explicitly ticked.

use std::path::Path;

use dmux_engine::results::RunResults;
use rust_xlsxwriter::{Workbook, Worksheet};

use crate::layout::{self, Layout, ReportOptions};

/// Excel's hard row limit (1,048,576 data rows; header is extra).
pub const EXCEL_MAX_ROWS: usize = 1_048_576;

/// Errors writing the workbook.
#[derive(Debug, thiserror::Error)]
pub enum XlsxError {
    #[error("cannot write workbook: {0}")]
    Write(#[from] rust_xlsxwriter::XlsxError),
    #[error("cannot write CSV overflow: {0}")]
    Io(#[from] std::io::Error),
}

/// A list that exceeded the row limit and was diverted to CSV.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overflow {
    pub sheet_name: String,
    pub header: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// The result of building the workbook plus any CSV overflow.
pub struct BuiltWorkbook {
    pub workbook: Workbook,
    pub overflow: Vec<Overflow>,
}

/// Write the detail workbook for `results` to `path`, diverting over-long lists
/// to `<stem>_<sheet>.csv` next to it.
pub fn write_detail(
    results: &RunResults,
    hooks: Option<&crate::verify::HookEvidence>,
    results_sha256: &str,
    options: &ReportOptions,
    path: &Path,
) -> Result<(), XlsxError> {
    let layout = layout::build(results, hooks, options);
    write_detail_layout(&layout, results_sha256, path)
}

/// Write a pre-built layout to disk (used by the shared `render_all` path so the
/// displayed-verdict fold happens once, in the caller).
pub fn write_detail_layout(
    layout: &Layout,
    results_sha256: &str,
    path: &Path,
) -> Result<(), XlsxError> {
    let mut layout = layout.clone();
    // The Summary sheet's "Results SHA-256" is the verified file digest (for an
    // envelope this is the envelope digest, not the nested results' own hash).
    layout.results_sha256 = results_sha256.to_string();
    let mut built = build_workbook(&layout, EXCEL_MAX_ROWS)?;
    let bytes = built.workbook.save_to_buffer()?;
    crate::write_private(path, &bytes)?;

    for overflow in &built.overflow {
        let csv_path = csv_path_for(path, &overflow.sheet_name);
        write_csv(&csv_path, overflow)?;
    }
    Ok(())
}

fn csv_path_for(xlsx_path: &Path, sheet: &str) -> std::path::PathBuf {
    let stem = xlsx_path.with_extension("");
    let mut file = stem
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    file.push('_');
    file.push_str(sheet);
    file.push_str(".csv");
    xlsx_path.with_file_name(file)
}

fn write_csv(path: &std::path::Path, overflow: &Overflow) -> Result<(), XlsxError> {
    let mut csv = String::new();
    let header: Vec<String> = overflow.header.iter().map(|c| csv_escape_cell(c)).collect();
    csv.push_str(&header.join(","));
    csv.push('\n');
    for row in &overflow.rows {
        let escaped: Vec<String> = row.iter().map(|c| csv_escape_cell(c)).collect();
        csv.push_str(&escaped.join(","));
        csv.push('\n');
    }
    crate::write_private(path, csv.as_bytes())?;
    Ok(())
}

/// Escape a cell for CSV: quote per RFC 4180 (wrap in double quotes when the
/// value contains a comma, double quote, newline or carriage return, doubling any
/// inner quote), THEN apply the spreadsheet formula-injection prefix (a `'`
/// before a leading `=`, `+`, `-`, `@`, tab or CR) so a spreadsheet reads it as
/// text, never a formula (PLAN 2.3).
fn csv_escape_cell(cell: &str) -> String {
    let quoted =
        if cell.contains(',') || cell.contains('"') || cell.contains('\n') || cell.contains('\r') {
            format!("\"{}\"", cell.replace('"', "\"\""))
        } else {
            cell.to_string()
        };
    if cell
        .chars()
        .next()
        .is_some_and(|c| matches!(c, '=' | '+' | '-' | '@' | '\t' | '\r'))
    {
        format!("'{quoted}")
    } else {
        quoted
    }
}

/// Build the workbook from a layout, diverting any list longer than `row_limit`
/// rows to a CSV (recorded in the returned `overflow`).  Tests lower `row_limit`
/// to exercise the overflow path without generating a million rows.
pub fn build_workbook(layout: &Layout, row_limit: usize) -> Result<BuiltWorkbook, XlsxError> {
    let mut workbook = Workbook::new();
    let mut overflow: Vec<Overflow> = Vec::new();

    // --- Summary sheet (also records any CSV diversion) ---
    {
        let summary = workbook.add_worksheet();
        summary.set_name("Summary")?;
        let mut row = 0u32;
        summary.write_string(row, 0, "Field")?;
        summary.write_string(row, 1, "Value")?;
        row += 1;
        let pairs: Vec<(String, String)> = vec![
            (
                "Verdict".into(),
                layout::verdict_display(
                    &layout.verdict_word,
                    layout.options_applied,
                    layout.options_recorded,
                ),
            ),
            ("Coverage".into(), layout.coverage_line.clone()),
        ];
        // The compliance record (when the check ran, which databases), minus
        // the run ID and version, which follow with the integrity fields.
        let mut pairs = pairs;
        pairs.extend(
            layout::record_rows(layout)
                .into_iter()
                .filter(|(k, _)| *k != "Run ID" && *k != "Dmux version")
                .map(|(k, v)| (k.to_string(), v)),
        );
        pairs.extend::<Vec<(String, String)>>(vec![
            ("Fully checked".into(), layout.fully_checked.to_string()),
            ("Counts only".into(), layout.counts_only.to_string()),
            ("Structure only".into(), layout.structure_only.to_string()),
            ("In scope".into(), layout.in_scope.to_string()),
            ("Ignored".into(), layout.ignored.to_string()),
            ("Run ID".into(), layout.run_id.clone()),
            ("Tool version".into(), layout.tool_version.clone()),
            ("Mapping hash".into(), layout.mapping_hash.clone()),
            ("Results SHA-256".into(), layout.results_sha256.clone()),
            (
                "Results schema version".into(),
                layout.results_version.clone(),
            ),
            ("Definition hash".into(), layout.definition_hash.clone()),
        ]);
        for (k, v) in pairs {
            summary.write_string(row, 0, &k)?;
            summary.write_string(row, 1, &v)?;
            row += 1;
        }
        if let Some(old) = &layout.capture_sha256_old {
            summary.write_string(row, 0, "Old capture SHA-256")?;
            summary.write_string(row, 1, old)?;
            row += 1;
        }
        if let Some(new) = &layout.capture_sha256_new {
            summary.write_string(row, 0, "New capture SHA-256")?;
            summary.write_string(row, 1, new)?;
            row += 1;
        }
        if !layout.options_recorded {
            summary.write_string(row, 0, "Options applied")?;
            summary.write_string(row, 1, layout::OPTIONS_NOT_RECORDED)?;
            row += 1;
        } else if layout.options_applied {
            summary.write_string(row, 0, "Options applied")?;
            summary.write_string(row, 1, "yes")?;
            row += 1;
        }
        for line in &layout.options_applied_lines {
            summary.write_string(row, 0, "Option applied")?;
            summary.write_string(row, 1, line)?;
            row += 1;
        }
        for line in &layout.connections {
            summary.write_string(row, 0, "Connection")?;
            summary.write_string(row, 1, line)?;
            row += 1;
        }
    }

    // --- Tables sheet ---
    {
        let tables = workbook.add_worksheet();
        tables.set_name("Tables")?;
        let header = [
            "Table",
            "Verdict",
            "Level",
            "Old rows",
            "New rows",
            "Differing buckets",
            "Missing buckets",
            "Extra buckets",
            "Missing rows",
            "Extra rows",
            "Changed rows",
            "Reasons",
            "Changed columns",
            "Control totals",
            "Column notes",
            "Notes",
            "Keyed rows",
        ];
        let mut rows: Vec<Vec<String>> = Vec::new();
        for t in &layout.tables {
            rows.push(vec![
                t.table.clone(),
                t.verdict_word.clone(),
                t.level.clone(),
                t.old_rows.clone().unwrap_or_else(|| "-".into()),
                t.new_rows.clone().unwrap_or_else(|| "-".into()),
                t.differing_buckets.clone(),
                t.missing_buckets.clone(),
                t.extra_buckets.clone(),
                t.missing_rows.clone(),
                t.extra_rows.clone(),
                t.changed_rows.clone(),
                t.reasons.join("; "),
                t.changed_columns.join(", "),
                t.control_totals.join("; "),
                t.column_notes.join("; "),
                t.plain_words.join("; "),
                t.keyed_rows.join("; "),
            ]);
        }
        write_sheet(
            tables,
            &header.map(String::from),
            &rows,
            row_limit,
            "Tables",
            &mut overflow,
        )?;
    }

    // --- Differences sheet (keys only when include_keys) ---
    {
        let differences = workbook.add_worksheet();
        differences.set_name("Differences")?;
        let include_keys = any_keys(&layout.differences);
        let mut dheader = vec![
            "Table".to_string(),
            "Missing rows".to_string(),
            "Extra rows".to_string(),
            "Changed rows".to_string(),
        ];
        if include_keys {
            dheader.push("Keys / covered columns".to_string());
        }
        dheader.push("Reasons".to_string());
        let mut drows: Vec<Vec<String>> = Vec::new();
        for d in &layout.differences {
            let mut cells = vec![
                d.table.clone(),
                d.missing_rows.clone(),
                d.extra_rows.clone(),
                d.changed_rows.clone(),
            ];
            if include_keys {
                cells.push(d.keys.join(", "));
            }
            cells.push(d.reasons.join("; "));
            drows.push(cells);
        }
        write_sheet(
            differences,
            &dheader,
            &drows,
            row_limit,
            "Differences",
            &mut overflow,
        )?;
    }

    // --- Structure Findings sheet ---
    {
        let structure = workbook.add_worksheet();
        structure.set_name("Structure Findings")?;
        let structure_rows: Vec<Vec<String>> = layout
            .structure_findings
            .iter()
            .map(|f| vec![f.object.clone(), f.verdict.clone(), f.message.clone()])
            .collect();
        write_sheet(
            structure,
            &[
                "Object".to_string(),
                "Verdict".to_string(),
                "Message".to_string(),
            ],
            &structure_rows,
            row_limit,
            "Structure Findings",
            &mut overflow,
        )?;
    }

    // --- Custom Checks sheet (hook findings, PLAN 4.9: marked as such) ---
    {
        let custom = workbook.add_worksheet();
        custom.set_name("Custom Checks")?;
        let custom_rows: Vec<Vec<String>> = layout
            .custom_checks
            .iter()
            .map(|c| {
                vec![
                    c.id.clone(),
                    c.severity.clone(),
                    c.table.clone().unwrap_or_default(),
                    c.message.clone(),
                ]
            })
            .collect();
        write_sheet(
            custom,
            &[
                "ID".to_string(),
                "Severity".to_string(),
                "Table".to_string(),
                "Message".to_string(),
            ],
            &custom_rows,
            row_limit,
            "Custom Checks",
            &mut overflow,
        )?;

        // Script hashes also go here, one row per script.
        for (i, s) in layout.hook_scripts.iter().enumerate() {
            custom.write_string((1 + custom_rows.len() + 2 + i) as u32, 0, "script")?;
            custom.write_string((1 + custom_rows.len() + 2 + i) as u32, 1, &s.script)?;
            custom.write_string((1 + custom_rows.len() + 2 + i) as u32, 2, &s.sha256)?;
        }
    }

    // --- Ignored sheet ---
    {
        let ignored = workbook.add_worksheet();
        ignored.set_name("Ignored")?;
        let ignored_rows: Vec<Vec<String>> = layout
            .ignored_items
            .iter()
            .map(|i| vec![i.path.clone(), i.reason.clone()])
            .collect();
        write_sheet(
            ignored,
            &["Path".to_string(), "Reason".to_string()],
            &ignored_rows,
            row_limit,
            "Ignored",
            &mut overflow,
        )?;
    }

    // --- Warnings/Flags sheet ---
    {
        let warnings = workbook.add_worksheet();
        warnings.set_name("Warnings Flags")?;
        let warning_rows: Vec<Vec<String>> =
            layout.warnings.iter().map(|w| vec![w.clone()]).collect();
        write_sheet(
            warnings,
            &["Warning".to_string()],
            &warning_rows,
            row_limit,
            "Warnings",
            &mut overflow,
        )?;
    }

    // --- Run log sheet ---
    {
        let runlog = workbook.add_worksheet();
        runlog.set_name("Run log")?;
        let log_fields: Vec<(String, String)> = vec![
            ("Verdict".into(), layout.verdict_word.clone()),
            ("Run ID".into(), layout.run_id.clone()),
            ("Tool version".into(), layout.tool_version.clone()),
            ("Mapping hash".into(), layout.mapping_hash.clone()),
            ("Results SHA-256".into(), layout.results_sha256.clone()),
            ("Warnings".into(), layout.warnings.join("; ")),
            (
                "Changed during check".into(),
                layout.changed_during_check.join("; "),
            ),
        ];
        runlog.write_string(0, 0, "Field")?;
        runlog.write_string(0, 1, "Value")?;
        for (i, (k, v)) in log_fields.into_iter().enumerate() {
            runlog.write_string(i as u32 + 1, 0, &k)?;
            runlog.write_string(i as u32 + 1, 1, &v)?;
        }
    }

    // Record any CSV diversion on the Summary sheet so a reader sees it (past the
    // field/value rows; row 14 is after the last summary field).
    if !overflow.is_empty() {
        let summary = workbook.worksheet_from_index(0)?;
        summary.write_string(14, 0, "CSV overflow")?;
        let names: Vec<&str> = overflow.iter().map(|o| o.sheet_name.as_str()).collect();
        summary.write_string(
            14,
            1,
            format!(
                "these lists exceeded the Excel row limit and were written to CSV: {}",
                names.join(", ")
            ),
        )?;
    }

    Ok(BuiltWorkbook { workbook, overflow })
}

fn any_keys(diffs: &[layout::Difference]) -> bool {
    diffs.iter().any(|d| !d.keys.is_empty())
}

/// Write `header` + `rows` to `sheet`.  If `rows.len()` exceeds `row_limit`,
/// divert the whole list to CSV (recorded in `overflow`) and leave a pointer row
/// in the sheet instead.
fn write_sheet(
    sheet: &mut Worksheet,
    header: &[String],
    rows: &[Vec<String>],
    row_limit: usize,
    sheet_name: &str,
    overflow: &mut Vec<Overflow>,
) -> Result<(), XlsxError> {
    if rows.len() > row_limit {
        overflow.push(Overflow {
            sheet_name: sheet_name.to_string(),
            header: header.to_vec(),
            rows: rows.to_vec(),
        });
        sheet.write_string(
            1,
            0,
            format!(
                "This list has {} rows and was written to {sheet_name}.csv",
                rows.len()
            ),
        )?;
    } else {
        for (i, c) in header.iter().enumerate() {
            sheet.write_string(0, i as u16, c)?;
        }
        for (i, row) in rows.iter().enumerate() {
            for (j, cell) in row.iter().enumerate() {
                sheet.write_string((i + 1) as u32, j as u16, cell)?;
            }
        }
    }
    Ok(())
}

/// Read a rendered workbook (a ZIP) back and return all strings from the
/// shared-string table and worksheet XMLs, so tests can assert keys/values
/// never leak.  Test-only: reading a workbook back needs the `zip` crate, which
/// is a dev-dependency, not part of the shipped report code.
#[cfg(test)]
fn shared_strings(bytes: &[u8]) -> Result<String, XlsxError> {
    use std::io::Read;
    let reader = std::io::Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(reader).map_err(|e| XlsxError::Io(std::io::Error::other(e)))?;
    let mut out = String::new();
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| XlsxError::Io(std::io::Error::other(e)))?;
        let name = file
            .name()
            .map_err(|e| XlsxError::Io(std::io::Error::other(e)))?
            .into_owned();
        if name == "xl/sharedStrings.xml" || name.starts_with("xl/worksheets/") {
            file.read_to_string(&mut out)?;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{build, Difference, Layout, TableSummary};

    const SENTINEL: &str = "SECRET-ROW-VALUE-777";

    fn layout_with(include_keys: bool) -> Layout {
        let ts = TableSummary {
            table: "demo_bank.accounts".into(),
            level: "full values".into(),
            verdict: "FAIL".into(),
            verdict_word: "FAIL".into(),
            old_rows: Some("100".into()),
            new_rows: Some("99".into()),
            differing_buckets: "1".into(),
            missing_buckets: "0".into(),
            extra_buckets: "0".into(),
            missing_rows: "1".into(),
            extra_rows: "0".into(),
            changed_rows: "0".into(),
            keys: if include_keys {
                vec!["id".into(), SENTINEL.into()]
            } else {
                vec![]
            },
            reasons: vec!["row counts differ".into()],
            changed_columns: vec!["balance".into()],
            plain_words: vec!["one value changed".into()],
            control_totals: vec!["balance: 100.00 -> 99.99".into()],
            column_notes: vec![],
            keyed_rows: if include_keys {
                vec!["keyed 123".into()]
            } else {
                vec![]
            },
        };
        Layout {
            results_version: "1".into(),
            run_id: "run-1".into(),
            tool_version: "0.1.0".into(),
            mapping_hash: "mh".into(),
            results_sha256: "abc".into(),
            verdict_word: "FAIL".into(),
            coverage_line: "0 tables fully checked, 0 counts only, 0 ignored".into(),
            fully_checked: 0,
            counts_only: 0,
            ignored: 0,
            ignored_items: vec![],
            warnings: vec![],
            changed_during_check: vec![],
            wal_start_lsn: None,
            wal_end_lsn: None,
            tables: vec![ts.clone()],
            differences: vec![Difference {
                table: "demo_bank.accounts".into(),
                missing_rows: "1".into(),
                extra_rows: "0".into(),
                changed_rows: "0".into(),
                keys: ts.keys.clone(),
                reasons: ts.reasons.clone(),
            }],
            in_scope: 1,
            structure_only: 0,
            structure_findings: vec![],
            suggestions: vec![],
            definition_hash: String::new(),
            options_recorded: true,
            options_applied: false,
            options_applied_lines: vec![],
            capture_sha256_old: None,
            capture_sha256_new: None,
            connections: vec![],
            custom_checks: vec![],
            hook_scripts: vec![],
            hook_warnings: vec![],
            ..Default::default()
        }
    }

    fn workbook_bytes(layout: &Layout) -> Vec<u8> {
        let mut built = build_workbook(layout, EXCEL_MAX_ROWS).unwrap();
        built.workbook.save_to_buffer().unwrap()
    }

    #[test]
    fn workbook_omits_keys_by_default_and_includes_them_when_ticked() {
        let without = shared_strings(&workbook_bytes(&layout_with(false))).unwrap();
        let with = shared_strings(&workbook_bytes(&layout_with(true))).unwrap();

        assert!(!without.contains(SENTINEL), "keys absent by default");
        assert!(with.contains(SENTINEL), "keys present when ticked");
        assert!(with.contains("id"));
    }

    #[test]
    fn workbook_scans_clean_of_row_value_sentinels() {
        let without = shared_strings(&workbook_bytes(&layout_with(false))).unwrap();
        assert!(!without.contains(SENTINEL));
    }

    #[test]
    fn writes_the_required_sheets() {
        let mut built = build_workbook(&layout_with(false), EXCEL_MAX_ROWS).unwrap();
        let names: Vec<String> = built
            .workbook
            .worksheets()
            .iter()
            .map(|w| w.name().to_string())
            .collect();
        for sheet in [
            "Summary",
            "Tables",
            "Differences",
            "Structure Findings",
            "Ignored",
            "Warnings Flags",
            "Run log",
        ] {
            assert!(
                names.iter().any(|n| n == sheet),
                "missing sheet {sheet}: {names:?}"
            );
        }
    }

    #[test]
    fn csv_overflow_diverts_a_long_list() {
        let mut layout = layout_with(false);
        layout.ignored_items = (0..10)
            .map(|i| crate::layout::Ignored {
                path: format!("t{i}"),
                reason: "x".into(),
            })
            .collect();

        let built = build_workbook(&layout, 3).unwrap();
        assert_eq!(built.overflow.len(), 1);
        assert_eq!(built.overflow[0].sheet_name, "Ignored");
        assert_eq!(built.overflow[0].rows.len(), 10);
    }

    #[test]
    fn formula_like_table_names_are_written_as_text_not_formulas() {
        // A hostile table or column name starting with `=` must render as a
        // literal string, never as a formula a reader's spreadsheet executes.
        let mut layout = layout_with(false);
        let injection = "=HYPERLINK(\"http://evil.example\")";
        layout.tables[0].table = injection.to_string();
        layout.differences[0].table = injection.to_string();

        let xml = shared_strings(&workbook_bytes(&layout)).unwrap();
        // The literal string is present as text...
        assert!(xml.contains("=HYPERLINK"), "text form missing: {xml}");
        // ...and there is no `<f>` (formula) cell carrying it.
        assert!(!xml.contains("<f>"), "formula cell leaked: {xml}");
    }

    #[test]
    fn csv_escape_prefixes_formula_cells() {
        assert_eq!(csv_escape_cell("=2+2"), "'=2+2");
        assert_eq!(csv_escape_cell("+SUM(A1)"), "'+SUM(A1)");
        assert_eq!(csv_escape_cell("-1"), "'-1");
        assert_eq!(csv_escape_cell("@cmd"), "'@cmd");
        assert_eq!(csv_escape_cell("plain text"), "plain text");
    }

    #[test]
    fn csv_quotes_per_rfc4180_and_then_prefixes_formula() {
        // Comma, quote, newline and CR force RFC-4180 double-quoting; a leading
        // `=` still gets the formula prefix AFTER quoting.
        assert_eq!(csv_escape_cell("a,b"), "\"a,b\"");
        assert_eq!(csv_escape_cell("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_escape_cell("line1\nline2"), "\"line1\nline2\"");
        assert_eq!(csv_escape_cell("x\r\ny"), "\"x\r\ny\"");
        assert_eq!(csv_escape_cell("=a,b"), "'\"=a,b\"");
        assert_eq!(csv_escape_cell("=1+1,2"), "'\"=1+1,2\"");
    }

    #[cfg(unix)]
    #[test]
    fn csv_overflow_output_is_written_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("dmux-xlsx-csv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("overflow.csv");
        let overflow = Overflow {
            sheet_name: "Tables".into(),
            header: vec!["a".into()],
            rows: vec![vec!["1".into()]],
        };
        write_csv(&path, &overflow).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "CSV overflow must be owner-only");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn workbook_omits_drilldown_keys_at_rendered_level() {
        // A full build through `build` (typed) with drill-down key sentinels:
        // absent without --include-keys, present with.
        use dmux_engine::results::{
            ChangedRowExport, RunResults, TableDetail, Verdict, RESULTS_VERSION,
        };
        let mut tables = std::collections::BTreeMap::new();
        tables.insert(
            "demo_bank.accounts".into(),
            dmux_engine::results::TableResult {
                table: "demo_bank.accounts".into(),
                level: dmux_engine::results::CheckLevel::FullValues,
                verdict: Verdict::Fail,
                old_rows: Some(10),
                new_rows: Some(9),
                columns_covered: Some(vec!["id".into()]),
                differing_buckets: Some(1),
                missing_buckets: Some(0),
                extra_buckets: Some(0),
                missing_rows: Some(0),
                extra_rows: Some(0),
                changed_rows: Some(1),
                reasons: vec![],
                column_notes: vec![],
            },
        );
        let mut details = std::collections::BTreeMap::new();
        details.insert(
            "demo_bank.accounts".into(),
            TableDetail {
                changed_columns: vec!["balance".into()],
                changed_rows: vec![ChangedRowExport {
                    key: Some(vec![Some(SENTINEL.into())]),
                    columns: vec!["balance".into()],
                }],
                missing_keys: None,
                extra_keys: None,
                control_totals: vec![],
                plain_words: vec![],
            },
        );
        let results = RunResults {
            results_version: RESULTS_VERSION,
            run_id: "r".into(),
            tool_version: "v".into(),
            mapping_hash: "h".into(),
            verdict: Verdict::Fail,
            coverage_line:
                "0 of 1 tables fully checked, 0 counts only, 0 structure only, 0 ignored".into(),
            fully_checked: 0,
            counts_only: 0,
            ignored: 0,
            tables,
            details,
            ..RunResults::default()
        };

        let no_keys = build(&results, None, &ReportOptions::default());
        let no_keys_xml = shared_strings(&workbook_bytes(&no_keys)).unwrap();
        assert!(
            !no_keys_xml.contains(SENTINEL),
            "drill-down key leaked without --include-keys"
        );

        let with_keys = build(
            &results,
            None,
            &ReportOptions {
                include_keys: true,
                ..Default::default()
            },
        );
        let with_keys_xml = shared_strings(&workbook_bytes(&with_keys)).unwrap();
        assert!(
            with_keys_xml.contains(SENTINEL),
            "drill-down key should be present with --include-keys"
        );
    }

    #[test]
    fn bidi_control_chars_are_stripped_by_the_layout() {
        // The layout builder is the sanitisation boundary; a reason carrying a
        // right-to-left override is dropped before any renderer sees it.
        use dmux_engine::results::{RunResults, Verdict, RESULTS_VERSION};
        let mut tables = std::collections::BTreeMap::new();
        tables.insert(
            "t".into(),
            dmux_engine::results::TableResult {
                table: "t".into(),
                level: dmux_engine::results::CheckLevel::FullValues,
                verdict: Verdict::Fail,
                old_rows: None,
                new_rows: None,
                columns_covered: None,
                differing_buckets: None,
                missing_buckets: None,
                extra_buckets: None,
                missing_rows: None,
                extra_rows: None,
                changed_rows: None,
                reasons: vec!["\u{202E}looks benign".to_string()],
                column_notes: vec![],
            },
        );
        let results = RunResults {
            results_version: RESULTS_VERSION,
            run_id: "r".into(),
            tool_version: "v".into(),
            mapping_hash: "h".into(),
            verdict: Verdict::Fail,
            coverage_line: "c".into(),
            fully_checked: 0,
            counts_only: 0,
            ignored: 0,
            tables,
            ignored_items: vec![],
            changed_during_check: vec![],
            warnings: vec![],
            ..RunResults::default()
        };
        let layout = build(&results, None, &ReportOptions::default());
        assert!(
            layout.tables[0].reasons[0].contains("looks benign"),
            "reason preserved"
        );
        assert!(
            !layout.tables[0].reasons[0].contains('\u{202E}'),
            "bidi stripped"
        );
    }
}
