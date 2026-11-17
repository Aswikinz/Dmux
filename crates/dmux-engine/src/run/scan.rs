//! Scans that are not the fingerprint itself: the count-and-totals scan of the
//! CountsAndTotals level, and the changed-row value fetch of the drill-down.
//!
//! Errors keep only the SQLSTATE (the core's [`CoreError`] conversion), because
//! a server message about a row can quote its values (rule 7).

use std::collections::HashMap;

use crate::core::drilldown::values_sql;
use crate::core::fingerprint::{parse_uint, Decimal, Side, Source, TablePlan};
use crate::core::{check_session, text_rows, CoreError};
use crate::ident::quote_qualified;
use tokio_postgres::Transaction;

/// Row count and control totals of one side, without hashing any value.
/// `controls` are the plan's control-total expressions for this side (quoted
/// column references built by the core's normaliser).
pub async fn counts_and_totals(
    txn: &Transaction<'_>,
    schema: &str,
    table: &str,
    controls: &[String],
) -> Result<(u64, Vec<Decimal>), CoreError> {
    let from = quote_qualified(schema, table)?;
    let mut items = vec!["count(*)::pg_catalog.text".to_string()];
    for c in controls {
        items.push(format!("sum({c})::pg_catalog.text"));
    }
    let sql = format!("SELECT {} FROM {from}", items.join(", "));
    check_session(txn).await?;
    let rows = text_rows(txn, &sql).await?;
    let row = rows.first().ok_or(CoreError::Malformed("count result"))?;
    if row.len() != 1 + controls.len() {
        return Err(CoreError::Malformed("count result width"));
    }
    let n = row[0]
        .as_deref()
        .and_then(parse_uint)
        .and_then(|n| u64::try_from(n).ok())
        .ok_or(CoreError::Malformed("row count"))?;
    let mut totals = Vec::with_capacity(controls.len());
    for v in &row[1..] {
        totals.push(match v.as_deref() {
            // sum() over no rows is NULL: the same zero the fingerprint reports.
            None => Decimal::ZERO,
            Some(s) => Decimal::parse(s).ok_or(CoreError::Malformed("control total"))?,
        });
    }
    Ok((n, totals))
}

/// Normalised values of the rows with the given key hashes, by key hash.
pub async fn values_by_key_hash(
    txn: &Transaction<'_>,
    plan: &TablePlan,
    side: Side,
    source: &Source,
    hashes: &[String],
) -> Result<HashMap<String, Vec<Option<String>>>, CoreError> {
    let sql = values_sql(plan, side, source)?;
    check_session(txn).await?;
    let rows = txn.query(sql.as_str(), &[&hashes]).await?;
    let mut out = HashMap::new();
    for r in rows {
        let vals: Vec<Option<String>> = r.try_get(1)?;
        if vals.len() != plan.columns().len() {
            return Err(CoreError::Malformed("column values"));
        }
        out.insert(r.try_get(0)?, vals);
    }
    Ok(out)
}

/// The differing column indexes of one changed row, and plain-language notes
/// about empty text and NULL (no value is ever copied into a note).
pub fn differing_columns(
    names: &[String],
    old: &[Option<String>],
    new: &[Option<String>],
) -> (Vec<usize>, Vec<String>) {
    let mut cols = Vec::new();
    let mut words = Vec::new();
    for (i, (a, b)) in old.iter().zip(new.iter()).enumerate() {
        if a == b {
            continue;
        }
        cols.push(i);
        let name = names.get(i).map(String::as_str).unwrap_or("?");
        match (a.as_deref(), b.as_deref()) {
            (Some(""), None) => words.push(format!("an empty text became NULL in column {name}")),
            (None, Some("")) => words.push(format!("a NULL became an empty text in column {name}")),
            _ => {}
        }
    }
    (cols, words)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn differing_columns_and_plain_words() {
        let names = vec!["id".to_string(), "narration".to_string(), "ts".to_string()];
        let old = vec![
            Some("7".to_string()),
            Some(String::new()),
            Some("a".to_string()),
        ];
        let new = vec![Some("7".to_string()), None, Some("b".to_string())];
        let (cols, words) = differing_columns(&names, &old, &new);
        assert_eq!(cols, vec![1, 2]);
        assert_eq!(words, vec!["an empty text became NULL in column narration"]);
        let (_, words) = differing_columns(&names, &new, &old);
        assert_eq!(
            words,
            vec!["a NULL became an empty text in column narration"]
        );
    }
}
