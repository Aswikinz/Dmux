//! Time estimates (PLAN.md 4.5): size on disk plus a short timed sample.
//!
//! Ranges are shown per table and in total, and refined as tables finish.  The
//! sample uses a LIMIT, which works on every PostgreSQL version from 9.4.

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio_postgres::Transaction;

use crate::ident::quote_qualified;

/// Errors from estimate queries (ident quoting or the database).
#[derive(Debug, Error)]
pub enum EstimateError {
    #[error("database error: {0}")]
    Db(String),
    #[error("invalid identifier: {0}")]
    Ident(#[from] crate::ident::IdentError),
}

impl From<tokio_postgres::Error> for EstimateError {
    fn from(e: tokio_postgres::Error) -> Self {
        EstimateError::Db(if let Some(db) = e.as_db_error() {
            format!("{}: {}", db.code().code(), db.message())
        } else {
            e.to_string()
        })
    }
}

/// A time range for one table or the whole run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Estimate {
    /// Lower bound, seconds.
    pub low_seconds: u64,
    /// Upper bound, seconds.
    pub high_seconds: u64,
}

impl Estimate {
    pub fn new(low_seconds: u64, high_seconds: u64) -> Self {
        Self {
            low_seconds,
            high_seconds: high_seconds.max(low_seconds),
        }
    }

    pub fn zero() -> Self {
        Self {
            low_seconds: 0,
            high_seconds: 0,
        }
    }
}

/// Size on disk of one table (indexes + TOAST included), 9.4-safe.  The table
/// is found by exact schema and table name (bound parameters), never by
/// parsing a `schema.table` string, so an upper-case or dotted name can never
/// resolve to a different table.  `None` when there is no such table.
pub async fn table_size_bytes(
    txn: &Transaction<'_>,
    schema: &str,
    table: &str,
) -> Result<Option<i64>, EstimateError> {
    let row = txn
        .query_opt(
            "SELECT pg_catalog.pg_total_relation_size(c.oid) FROM pg_catalog.pg_class c \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 AND c.relname = $2",
            &[&schema, &table],
        )
        .await?;
    Ok(match row {
        Some(r) => Some(r.try_get(0)?),
        None => None,
    })
}

/// An estimate from a point value in seconds: half the point to twice the
/// point.  The upper bound covers the second pass a failing table costs for
/// its drill-down (PLAN.md 4.5).
pub fn estimate_from_seconds(seconds: f64) -> Estimate {
    if !seconds.is_finite() || seconds <= 0.0 {
        return Estimate::zero();
    }
    Estimate::new(
        (seconds * 0.5).floor() as u64,
        (seconds * 2.0).ceil() as u64,
    )
}

/// Time one row-identity scan of a sample of rows, without the fingerprint's
/// grouping cost (a bound `LIMIT` sample).  Returns bytes per second hashed.
///
/// `sample_rows` is the number of rows to time.  The query selects the mapped
/// column expressions for a bounded number of rows; this is a timing probe, not
/// a correctness probe, so it simply measures how fast the server can emit the
/// normalised values.
pub async fn timed_sample(
    txn: &Transaction<'_>,
    schema: &str,
    table: &str,
    sample_rows: u64,
) -> Result<Duration, EstimateError> {
    let qualified = quote_qualified(schema, table)?;
    let start = Instant::now();
    txn.query(
        &format!("SELECT * FROM {qualified} LIMIT $1"),
        &[&(sample_rows as i64)],
    )
    .await?;
    Ok(start.elapsed())
}

/// Build a per-table and total estimate from a size and a sampled bytes/second
/// rate.  The range is ±50% around the point estimate, and the total is the sum
/// of the per-table point estimates with the same spread.
pub fn estimate_from_size(size_bytes: i64, bytes_per_second: f64) -> Estimate {
    if bytes_per_second <= 0.0 || size_bytes <= 0 {
        return Estimate::zero();
    }
    let seconds = size_bytes as f64 / bytes_per_second;
    let point = seconds.round() as u64;
    let low = (seconds * 0.5).round() as u64;
    let high = (seconds * 1.5).round() as u64;
    Estimate::new(low.min(point), high.max(point))
}

/// Combine many estimates into one total range.
pub fn total_estimate(estimates: &[Estimate]) -> Estimate {
    let low: u64 = estimates.iter().map(|e| e.low_seconds).sum();
    let high: u64 = estimates.iter().map(|e| e.high_seconds).sum();
    Estimate::new(low, high)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_range_is_symmetric() {
        let e = estimate_from_size(1000, 100.0);
        // 1000 bytes / 100 B/s = 10s, range 5..15.
        assert_eq!(e.low_seconds, 5);
        assert_eq!(e.high_seconds, 15);
    }

    #[test]
    fn estimate_zero_defaults() {
        assert_eq!(estimate_from_size(0, 100.0), Estimate::zero());
        assert_eq!(estimate_from_size(1000, 0.0), Estimate::zero());
        assert_eq!(estimate_from_size(-1, 100.0), Estimate::zero());
    }

    #[test]
    fn total_sums_ranges() {
        let total = total_estimate(&[Estimate::new(1, 2), Estimate::new(3, 4)]);
        assert_eq!(total, Estimate::new(4, 6));
    }

    #[test]
    fn high_is_at_least_low() {
        assert_eq!(Estimate::new(9, 3).high_seconds, 9);
    }

    #[test]
    fn seconds_estimate_covers_a_drilldown_pass() {
        assert_eq!(estimate_from_seconds(10.0), Estimate::new(5, 20));
        assert_eq!(estimate_from_seconds(0.0), Estimate::zero());
        assert_eq!(estimate_from_seconds(f64::NAN), Estimate::zero());
    }
}
