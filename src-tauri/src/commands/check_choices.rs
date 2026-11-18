//! Save and open a run's check choices: which tables, at which level, which
//! columns are left out, and which columns are added up as control totals.
//!
//! The file is plain JSON, so the same choices can be reused on the next run
//! against the same database.  It names tables and columns only: never a
//! connection, login, password or row value.  Both directions are untrusted:
//! what the webview sends is validated before the save dialog opens, and an
//! opened file is size-limited, parsed strictly (unknown fields refused) and
//! validated the same way.  Paths come from native dialogs opened here, so the
//! webview never names one; a saved file is owner-only (mode 0600 on Unix).

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use dmux_engine::results::CheckLevel;

use crate::error::CommandError;

/// The `format` every check-choices file carries.
pub const FORMAT: &str = "dmux-check-choices";
/// The file version this build writes and reads.
pub const VERSION: u32 = 1;
/// The largest file Dmux opens (a choice per table and column of a very large
/// database fits many times over).
pub const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
/// The most tables one file may name.
const MAX_TABLES: usize = 200_000;
/// The longest name part (PostgreSQL's own limit is 63 bytes; this is generous).
const MAX_NAME: usize = 255;

/// One table's choices, by its old-side `schema.table`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct TableChoice {
    pub table: String,
    pub level: CheckLevel,
    /// Columns not compared in this table.  A key column is always compared.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub left_out_columns: Vec<String>,
    /// Columns added up on both sides as control totals.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub control_totals: Vec<String>,
}

/// A whole check-choices file.  Tables it does not name are not in the run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct CheckChoices {
    pub format: String,
    pub version: u32,
    /// The optional daily run window, "HH:MM-HH:MM".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_window: Option<String>,
    pub tables: Vec<TableChoice>,
}

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::InvalidInput {
        message: message.into(),
    }
}

/// A name part: non-empty, not too long, no dot (a dot separates parts) and no
/// control character.
fn valid_part(p: &str) -> bool {
    !p.is_empty() && p.len() <= MAX_NAME && !p.contains('.') && !p.chars().any(char::is_control)
}

/// Check a choices value: right format and version, `schema.table` names,
/// plain column names, no table or column named twice, and a column never both
/// left out and added up.  Returns it with names sorted, so the same choices
/// always give the same file.  Errors name the table, never a value.
pub fn validate(choices: &CheckChoices) -> Result<CheckChoices, CommandError> {
    if choices.format != FORMAT {
        return Err(invalid("this is not a Dmux check choices file"));
    }
    if choices.version != VERSION {
        return Err(invalid(format!(
            "this check choices file is version {}; this Dmux reads version {VERSION}",
            choices.version
        )));
    }
    if choices.tables.is_empty() {
        return Err(invalid("the check choices name no table"));
    }
    if choices.tables.len() > MAX_TABLES {
        return Err(invalid("the check choices name too many tables"));
    }
    if let Some(w) = &choices.run_window {
        if w.len() != 11 || !w.is_ascii() {
            return Err(invalid("the run window must look like 22:00-05:00"));
        }
    }
    let mut seen = BTreeSet::new();
    let mut tables = Vec::with_capacity(choices.tables.len());
    for t in &choices.tables {
        let ok = t
            .table
            .split_once('.')
            .is_some_and(|(s, n)| valid_part(s) && valid_part(n));
        if !ok {
            return Err(invalid(
                "every table in the check choices must be named as schema.table",
            ));
        }
        if !seen.insert(t.table.clone()) {
            return Err(invalid(format!(
                "the check choices name table {} twice",
                t.table
            )));
        }
        let columns = |list: &[String], what: &str| -> Result<Vec<String>, CommandError> {
            let set: BTreeSet<String> = list.iter().cloned().collect();
            if set.len() != list.len() {
                return Err(invalid(format!(
                    "the check choices name a column twice in the {what} of table {}",
                    t.table
                )));
            }
            if !set.iter().all(|c| valid_part(c)) {
                return Err(invalid(format!(
                    "a column in the {what} of table {} is not a plain column name",
                    t.table
                )));
            }
            Ok(set.into_iter().collect())
        };
        let left_out = columns(&t.left_out_columns, "left-out columns")?;
        let totals = columns(&t.control_totals, "control totals")?;
        if let Some(both) = left_out.iter().find(|c| totals.contains(c)) {
            return Err(invalid(format!(
                "column {both} of table {} is both left out and a control total",
                t.table
            )));
        }
        tables.push(TableChoice {
            table: t.table.clone(),
            level: t.level,
            left_out_columns: left_out,
            control_totals: totals,
        });
    }
    tables.sort_by(|a, b| a.table.cmp(&b.table));
    Ok(CheckChoices {
        format: FORMAT.to_string(),
        version: VERSION,
        run_window: choices.run_window.clone(),
        tables,
    })
}

/// The canonical file bytes (validated, sorted, pretty-printed, newline-ended).
pub fn to_bytes(choices: &CheckChoices) -> Result<Vec<u8>, CommandError> {
    let checked = validate(choices)?;
    let mut bytes = serde_json::to_vec_pretty(&checked).map_err(|e| {
        CommandError::internal(format!("the check choices could not be written: {e}"))
    })?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Parse and validate file bytes.  Unknown fields are refused; the parse error
/// is described without echoing the file's contents.
pub fn from_bytes(bytes: &[u8]) -> Result<CheckChoices, CommandError> {
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(invalid("the check choices file is too large"));
    }
    let parsed: CheckChoices = serde_json::from_slice(bytes).map_err(|e| {
        invalid(format!(
            "the check choices file could not be read (line {}, column {})",
            e.line(),
            e.column()
        ))
    })?;
    validate(&parsed)
}

/// Give `path` a `.json` extension when the user left it off.
fn with_json_extension(path: PathBuf) -> PathBuf {
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("json"))
    {
        path
    } else {
        let mut name = path.into_os_string();
        name.push(".json");
        PathBuf::from(name)
    }
}

/// The outcome of a save.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SaveChoicesResult {
    Saved { path: String },
    Cancelled,
}

/// The outcome of an open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum OpenChoicesResult {
    Opened { path: String, choices: CheckChoices },
    Cancelled,
}

/// Save the check choices through a native save dialog.  Invalid choices are
/// refused before the dialog opens.
#[tauri::command]
pub async fn save_check_choices<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    choices: CheckChoices,
) -> Result<SaveChoicesResult, CommandError> {
    use tauri_plugin_dialog::DialogExt;

    let bytes = to_bytes(&choices)?;
    let dialog = app
        .dialog()
        .file()
        .set_title("Save the check choices")
        .set_file_name("dmux-checks.json")
        .add_filter("Dmux check choices", &["json"]);
    let chosen = tauri::async_runtime::spawn_blocking(move || dialog.blocking_save_file())
        .await
        .map_err(|e| CommandError::internal(format!("the save dialog failed: {e}")))?;
    let Some(chosen) = chosen else {
        return Ok(SaveChoicesResult::Cancelled);
    };
    let path = chosen
        .into_path()
        .map_err(|_| invalid("the chosen location is not a local file"))?;
    let path = with_json_extension(path);
    let target = path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::history::write_private_file(&target, &bytes)
    })
    .await
    .map_err(|e| CommandError::internal(format!("the save task failed: {e}")))?
    .map_err(|e| CommandError::Store {
        message: format!("the check choices could not be written: {e}"),
    })?;
    Ok(SaveChoicesResult::Saved {
        path: path.display().to_string(),
    })
}

/// Read a file of at most [`MAX_FILE_BYTES`], refusing a larger one before
/// reading it whole.
fn read_limited(path: &std::path::Path) -> Result<Vec<u8>, CommandError> {
    use std::io::Read;
    let file = std::fs::File::open(path)
        .map_err(|_| invalid("the check choices file could not be opened"))?;
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid("the check choices file could not be read"))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(invalid("the check choices file is too large"));
    }
    Ok(bytes)
}

/// Open a check choices file through a native open dialog.
#[tauri::command]
pub async fn open_check_choices<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<OpenChoicesResult, CommandError> {
    use tauri_plugin_dialog::DialogExt;

    let dialog = app
        .dialog()
        .file()
        .set_title("Open check choices")
        .add_filter("Dmux check choices", &["json"]);
    let chosen = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_file())
        .await
        .map_err(|e| CommandError::internal(format!("the open dialog failed: {e}")))?;
    let Some(chosen) = chosen else {
        return Ok(OpenChoicesResult::Cancelled);
    };
    let path = chosen
        .into_path()
        .map_err(|_| invalid("the chosen file is not a local file"))?;
    let source = path.clone();
    let bytes = tauri::async_runtime::spawn_blocking(move || read_limited(&source))
        .await
        .map_err(|e| CommandError::internal(format!("the open task failed: {e}")))??;
    let choices = from_bytes(&bytes)?;
    Ok(OpenChoicesResult::Opened {
        path: path.display().to_string(),
        choices,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(name: &str) -> TableChoice {
        TableChoice {
            table: name.into(),
            level: CheckLevel::FullValues,
            left_out_columns: vec![],
            control_totals: vec![],
        }
    }

    fn choices(tables: Vec<TableChoice>) -> CheckChoices {
        CheckChoices {
            format: FORMAT.into(),
            version: VERSION,
            run_window: None,
            tables,
        }
    }

    #[test]
    fn saved_choices_open_again_unchanged_and_always_give_the_same_file() {
        let mut loans = table("core.loans");
        loans.level = CheckLevel::CountsAndTotals;
        loans.control_totals = vec!["principal".into(), "balance".into()];
        let mut accounts = table("core.accounts");
        accounts.left_out_columns = vec!["nickname".into()];
        let mut c = choices(vec![loans, accounts]);
        c.run_window = Some("22:00-05:00".into());
        let bytes = to_bytes(&c).unwrap();
        let back = from_bytes(&bytes).unwrap();
        // Sorted on the way out, so order in the UI never changes the file.
        assert_eq!(back.tables[0].table, "core.accounts");
        assert_eq!(back.tables[1].control_totals, vec!["balance", "principal"]);
        assert_eq!(back.run_window.as_deref(), Some("22:00-05:00"));
        assert_eq!(to_bytes(&back).unwrap(), bytes);
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("\"format\": \"dmux-check-choices\""));
        assert!(text.contains("\"level\": \"counts_and_totals\""));
        assert!(text.ends_with("}\n"));
    }

    #[test]
    fn a_wrong_or_unsafe_file_is_refused_with_a_plain_reason() {
        let ok = choices(vec![table("core.loans")]);
        let refuse = |c: CheckChoices| validate(&c).unwrap_err().to_string();

        let mut c = ok.clone();
        c.format = "something-else".into();
        assert!(refuse(c).contains("not a Dmux check choices file"));
        let mut c = ok.clone();
        c.version = 2;
        assert!(refuse(c).contains("version 2"));
        assert!(refuse(choices(vec![])).contains("no table"));
        for bad in ["loans", "core.", ".loans", "a.b.c", "core.lo\nans"] {
            assert!(
                refuse(choices(vec![table(bad)])).contains("schema.table"),
                "{bad:?}"
            );
        }
        assert!(refuse(choices(vec![table("core.loans"), table("core.loans")])).contains("twice"));
        let mut t = table("core.loans");
        t.left_out_columns = vec!["a".into(), "a".into()];
        assert!(refuse(choices(vec![t])).contains("twice"));
        let mut t = table("core.loans");
        t.control_totals = vec!["x.y".into()];
        assert!(refuse(choices(vec![t])).contains("plain column name"));
        let mut t = table("core.loans");
        t.left_out_columns = vec!["amount".into()];
        t.control_totals = vec!["amount".into()];
        assert!(refuse(choices(vec![t])).contains("both left out and a control total"));
        let mut c = ok;
        c.run_window = Some("whenever".into());
        assert!(refuse(c).contains("run window"));
    }

    #[test]
    fn unknown_fields_and_bad_json_are_refused_without_echoing_the_file() {
        let extra = br#"{"format":"dmux-check-choices","version":1,"tables":[{"table":"core.loans","level":"full_values"}],"password":"hunter2"}"#;
        let e = from_bytes(extra).unwrap_err().to_string();
        assert!(e.contains("could not be read"), "{e}");
        assert!(!e.contains("hunter2"), "{e}");
        let e = from_bytes(b"{ not json secret-value")
            .unwrap_err()
            .to_string();
        assert!(!e.contains("secret-value"), "{e}");
        let bad_level = br#"{"format":"dmux-check-choices","version":1,"tables":[{"table":"core.loans","level":"everything"}]}"#;
        assert!(from_bytes(bad_level).is_err());
    }

    #[test]
    fn the_window_may_save_and_open_check_choices() {
        let caps: serde_json::Value =
            serde_json::from_str(include_str!("../../capabilities/default.json")).unwrap();
        let perms: Vec<&str> = caps["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|p| p.as_str())
            .collect();
        for (perm, command) in [
            ("allow-save-check-choices", "\"save_check_choices\""),
            ("allow-open-check-choices", "\"open_check_choices\""),
        ] {
            assert!(perms.contains(&perm), "{perm}");
            assert!(
                include_str!("../../build.rs").contains(command),
                "{command}"
            );
        }
    }

    #[test]
    fn an_oversized_file_is_refused_before_it_is_parsed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.json");
        let f = std::fs::File::create(&path).unwrap();
        f.set_len(MAX_FILE_BYTES + 1).unwrap();
        assert!(read_limited(&path)
            .unwrap_err()
            .to_string()
            .contains("too large"));
        let small = dir.path().join("small.json");
        std::fs::write(&small, b"{}").unwrap();
        assert_eq!(read_limited(&small).unwrap(), b"{}");
        assert!(from_bytes(&vec![b' '; (MAX_FILE_BYTES + 1) as usize])
            .unwrap_err()
            .to_string()
            .contains("too large"));
    }
}
