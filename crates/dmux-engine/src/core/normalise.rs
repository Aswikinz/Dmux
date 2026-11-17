//! Per-column normalisation (PLAN.md 3.2, T003 review section 5).
//!
//! For one mapped column this builds the SQL text expression each side hashes.
//! The same rule is chosen for both sides from both declared types, and only
//! quoted column references are ever placed into the SQL. The expressions
//! assume the canonical session settings (UTC, ISO dates, iso_8601 intervals,
//! extra_float_digits = 0, search_path = pg_catalog), which the fingerprint
//! checks before it runs.
//!
//! Float arrays are hashed as `array_send` bytes, not printed text: the
//! printed form under extra_float_digits = 0 keeps only 15 significant digits,
//! so two different doubles can print alike (a false PASS). `array_send` is
//! exact and identical on 9.4 to 18 (T003 P7); other arrays keep the printed
//! form, which stays UTF-8 text as DECISIONS 4 requires. Geometric types
//! carry floats too and are hashed from their `*_send` bytes (all present on
//! 9.4). Domains are resolved to their base type and use its rule.
//!
//! A type whose printed form is not provably exact (unknown and extension
//! types, composites or ranges holding floats or unknown parts) gets
//! [`Flag::NeedsReview`]: the table can then never be a clean PASS.

use super::CoreError;
use crate::ident::{quote_ident, IdentError};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use tokio_postgres::GenericClient;

/// A declared column type, parsed from `format_type()` output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PgType {
    Text,
    Varchar,
    Bpchar,
    /// `(precision, scale)`, or `None` for unconstrained numeric.
    Numeric(Option<(u32, u32)>),
    Float4,
    Float8,
    Int2,
    Int4,
    Int8,
    Date,
    Timestamp,
    Timestamptz,
    Time,
    Timetz,
    Interval,
    Bytea,
    Money,
    Json,
    Jsonb,
    Xml,
    /// Built-in types whose printed form is version-stable (bool, uuid, inet,
    /// built-in ranges, ...).
    Stable(String),
    /// point, line, lseg, box, path, polygon, circle: hashed by send bytes.
    Geometric(Geo),
    Array(Box<PgType>),
    /// A domain, compared by the rule of its base type.
    Domain {
        name: String,
        base: Box<PgType>,
    },
    /// A user enum: its label text is exact.
    Enum(String),
    /// A range with its subtype (from the catalog).
    Range {
        name: String,
        sub: Box<PgType>,
    },
    /// A composite type with its field types (from the catalog).
    Composite {
        name: String,
        fields: Vec<PgType>,
    },
    /// Extension or unrecognised types: printed form, with a warning, and
    /// never a clean PASS.
    Unknown(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Geo {
    Point,
    Line,
    Lseg,
    Box,
    Path,
    Polygon,
    Circle,
}

impl Geo {
    fn from_name(n: &str) -> Option<Geo> {
        Some(match n {
            "point" => Geo::Point,
            "line" => Geo::Line,
            "lseg" => Geo::Lseg,
            "box" => Geo::Box,
            "path" => Geo::Path,
            "polygon" => Geo::Polygon,
            "circle" => Geo::Circle,
            _ => return None,
        })
    }

    /// (type name, binary send function); every one exists on 9.4.
    fn sql(self) -> (&'static str, &'static str) {
        match self {
            Geo::Point => ("point", "point_send"),
            Geo::Line => ("line", "line_send"),
            Geo::Lseg => ("lseg", "lseg_send"),
            Geo::Box => ("box", "box_send"),
            Geo::Path => ("path", "path_send"),
            Geo::Polygon => ("polygon", "poly_send"),
            Geo::Circle => ("circle", "circle_send"),
        }
    }
}

const STABLE: &[&str] = &[
    "boolean",
    "uuid",
    "inet",
    "cidr",
    "macaddr",
    "bit",
    "bit varying",
    "name",
    "oid",
    "\"char\"",
    "int4range",
    "int8range",
    "numrange",
    "daterange",
    "tsrange",
    "tstzrange",
];

impl PgType {
    /// Parse the text `format_type(atttypid, atttypmod)` prints.
    pub fn parse(format_type: &str) -> PgType {
        let s = format_type.trim();
        if let Some(inner) = s.strip_suffix("[]") {
            return PgType::Array(Box::new(PgType::parse(inner)));
        }
        let (base, modifier) = match (s.find('('), s.find(')')) {
            (Some(a), Some(b)) if a < b => {
                let base = format!("{}{}", s[..a].trim_end(), &s[b + 1..]);
                (base, Some(&s[a + 1..b]))
            }
            _ => (s.to_string(), None),
        };
        match base.as_str() {
            "text" => PgType::Text,
            "character varying" => PgType::Varchar,
            "character" | "bpchar" => PgType::Bpchar,
            "numeric" => match modifier.map(parse_numeric_mod) {
                None => PgType::Numeric(None),
                Some(Some(ps)) => PgType::Numeric(Some(ps)),
                Some(None) => PgType::Unknown(s.to_string()),
            },
            "real" => PgType::Float4,
            "double precision" => PgType::Float8,
            "smallint" => PgType::Int2,
            "integer" => PgType::Int4,
            "bigint" => PgType::Int8,
            "date" => PgType::Date,
            "timestamp without time zone" => PgType::Timestamp,
            "timestamp with time zone" => PgType::Timestamptz,
            "time without time zone" => PgType::Time,
            "time with time zone" => PgType::Timetz,
            "bytea" => PgType::Bytea,
            "money" => PgType::Money,
            "json" => PgType::Json,
            "jsonb" => PgType::Jsonb,
            "xml" => PgType::Xml,
            b if b.starts_with("interval") => PgType::Interval,
            b if STABLE.contains(&b) => PgType::Stable(b.to_string()),
            b => match Geo::from_name(b) {
                Some(g) if modifier.is_none() => PgType::Geometric(g),
                _ => PgType::Unknown(s.to_string()),
            },
        }
    }

    /// The type a domain stands for (any depth); other types are themselves.
    pub fn base(&self) -> &PgType {
        match self {
            PgType::Domain { base, .. } => base.base(),
            t => t,
        }
    }

    /// Types hashed by exact binary send bytes; the SQL name of the type.
    fn binary_name(&self) -> Option<&'static str> {
        match self.base() {
            PgType::Float4 => Some("float4"),
            PgType::Float8 => Some("float8"),
            PgType::Geometric(g) => Some(g.sql().0),
            _ => None,
        }
    }

    /// The printed form is exact and the same on every supported server.
    fn printed_stable(&self) -> bool {
        use PgType::*;
        match self.base() {
            Text | Varchar | Bpchar | Numeric(_) | Int2 | Int4 | Int8 | Date | Timestamp
            | Timestamptz | Time | Timetz | Interval | Bytea | Json | Jsonb | Stable(_)
            | Enum(_) => true,
            Array(e) => e.printed_stable(),
            Range { sub, .. } => sub.printed_stable(),
            Composite { fields, .. } => fields.iter().all(PgType::printed_stable),
            _ => false,
        }
    }

    /// Compared by a printed form that is not provably exact.
    fn needs_review(&self) -> bool {
        use PgType::*;
        match self.base() {
            Unknown(_) => true,
            Composite { .. } => !self.printed_stable(),
            Range { sub, .. } => sub.binary_name().is_none() && !sub.printed_stable(),
            Array(e) => {
                let e = e.base();
                !(e.binary_name().is_some() || matches!(e, Money | Xml) || e.printed_stable())
            }
            _ => false,
        }
    }

    fn is_text(&self) -> bool {
        matches!(self, PgType::Text | PgType::Varchar | PgType::Bpchar)
    }

    fn is_float(&self) -> bool {
        matches!(self, PgType::Float4 | PgType::Float8)
    }

    fn is_datetime(&self) -> bool {
        matches!(self, PgType::Date | PgType::Timestamp | PgType::Timestamptz)
    }

    fn is_roundable(&self) -> bool {
        self.is_float() || matches!(self, PgType::Numeric(_) | PgType::Money)
    }

    fn is_totalable(&self) -> bool {
        matches!(
            self,
            PgType::Int2 | PgType::Int4 | PgType::Int8 | PgType::Numeric(_) | PgType::Money
        )
    }

    fn name(&self) -> String {
        format!("{self:?}")
    }
}

fn parse_numeric_mod(m: &str) -> Option<(u32, u32)> {
    let mut parts = m.split(',').map(|p| p.trim().parse::<u32>().ok());
    let p = parts.next()??;
    let s = match parts.next() {
        None => 0,
        Some(s) => s?,
    };
    (parts.next().is_none() && s <= p).then_some((p, s))
}

/// The four options a user may tick per column. Each ticked one is reported.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnOptions {
    pub ignore_trailing_spaces: bool,
    pub null_equals_empty: bool,
    pub date_only: bool,
    pub round_decimals: Option<u8>,
}

/// The highest "round to N decimals" accepted.
pub const MAX_ROUND_DECIMALS: u8 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OptionKind {
    IgnoreTrailingSpaces,
    NullEqualsEmpty,
    DateOnly,
    Round(u8),
}

/// Something the report must show for this column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Flag {
    /// The declared type differs between the sides.
    TypeChanged {
        old: String,
        new: String,
    },
    /// json on one side, jsonb on the other: jsonb reorders and dedupes keys (P12).
    JsonJsonbChange,
    /// timestamp on one side, timestamptz on the other (P9).
    TimestampTzChange,
    /// char(n) on one side, varchar or text on the other (P6).
    CharVarcharChange,
    /// Both sides cast up to this numeric scale (P4, P8).
    NumericScaleRaised {
        scale: u32,
    },
    /// One numeric side is unconstrained, so no common scale exists; printed
    /// forms are compared as stored (may fail on trailing zeros, never pass wrongly).
    NumericScaleUnchecked,
    /// Printed form of a type whose output may depend on the server (xml).
    PrintedFormWarning {
        type_name: String,
    },
    /// Extension or unrecognised type, compared by printed form.
    UnknownType {
        type_name: String,
    },
    OptionApplied(OptionKind),
    /// A domain, compared by its base type's rule.
    DomainResolved {
        type_name: String,
    },
    /// Compared by a printed form that is not provably exact (unknown or
    /// extension type, composite or range with float or unknown parts). The
    /// table can never be a clean PASS; a person must review it.
    NeedsReview {
        type_name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NormaliseError {
    #[error(transparent)]
    Ident(#[from] IdentError),
    #[error("the option {option:?} does not apply to type {type_name}")]
    InvalidOption {
        option: OptionKind,
        type_name: String,
    },
    #[error("round to at most {MAX_ROUND_DECIMALS} decimals")]
    RoundTooLarge,
    #[error("control totals need integer, numeric or money columns, not {type_name}")]
    ControlTotalType { type_name: String },
    #[error("the common numeric type would exceed precision 1000")]
    NumericTooWide,
}

/// One mapped column: its names and declared types on both sides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnMapping {
    pub old_name: String,
    pub new_name: String,
    pub old_type: PgType,
    pub new_type: PgType,
    #[serde(default)]
    pub options: ColumnOptions,
    /// Also sum this column as a control total.
    #[serde(default)]
    pub control_total: bool,
}

/// The SQL each side hashes for one column, plus what the report must show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnPlan {
    pub old_expr: String,
    pub new_expr: String,
    pub old_control: Option<String>,
    pub new_control: Option<String>,
    pub flags: Vec<Flag>,
}

const NUMERIC_MAX_PRECISION: u32 = 1000;

/// The base expression for one side, given the other side's type.
fn side_expr(c: &str, ty: &PgType, other: &PgType) -> Result<String, NormaliseError> {
    let (ty, other) = (ty.base(), other.base());
    Ok(match (ty, other) {
        (PgType::Float4, PgType::Float8) => format!("encode(float8send({c}::float8), 'hex')"),
        (PgType::Float4, _) => format!("encode(float4send({c}), 'hex')"),
        (PgType::Float8, _) => format!("encode(float8send({c}), 'hex')"),
        (PgType::Geometric(g), _) => format!("encode({}({c}), 'hex')", g.sql().1),
        (PgType::Numeric(Some(a)), PgType::Numeric(Some(b))) if a.1 != b.1 => {
            format!("{c}::{}::text", widened_numeric(*a, *b)?)
        }
        (PgType::Money, _) => format!("{c}::numeric::text"),
        (PgType::Bytea, _) => format!("md5({c})"),
        (PgType::Array(e), PgType::Array(o)) => match (e.base(), o.base()) {
            (PgType::Money, _) => format!("{c}::numeric[]::text"),
            (PgType::Numeric(Some(a)), PgType::Numeric(Some(b))) if a.1 != b.1 => {
                format!("{c}::{}[]::text", widened_numeric(*a, *b)?)
            }
            (PgType::Float4, PgType::Float8) => {
                format!("encode(array_send({c}::float8[]), 'hex')")
            }
            (e, _) => match e.binary_name() {
                // Cast to the base array so a domain's OID never enters the bytes.
                Some(n) => format!("encode(array_send({c}::{n}[]), 'hex')"),
                None => format!("{c}::text"),
            },
        },
        (PgType::Array(e), _) if e.base() == &PgType::Money => format!("{c}::numeric[]::text"),
        (PgType::Array(e), _) if e.binary_name().is_some() => format!(
            "encode(array_send({c}::{}[]), 'hex')",
            e.binary_name().unwrap_or("float8")
        ),
        (PgType::Range { sub, .. }, _) if sub.binary_name().is_some() => {
            format!("encode(range_send({c}), 'hex')")
        }
        _ => format!("{c}::text"),
    })
}

/// The numeric type both sides cast up to: max integer digits + max scale.
fn widened_numeric(a: (u32, u32), b: (u32, u32)) -> Result<String, NormaliseError> {
    let scale = a.1.max(b.1);
    let precision = a.0.saturating_sub(a.1).max(b.0.saturating_sub(b.1)) + scale;
    if precision > NUMERIC_MAX_PRECISION {
        return Err(NormaliseError::NumericTooWide);
    }
    Ok(format!("numeric({precision},{scale})"))
}

fn type_flags(old: &PgType, new: &PgType) -> Vec<Flag> {
    use PgType::*;
    let mut flags = Vec::new();
    if old != new {
        flags.push(Flag::TypeChanged {
            old: old.name(),
            new: new.name(),
        });
    }
    for t in [old, new] {
        if let Domain { name, .. } = t {
            flags.push(Flag::DomainResolved {
                type_name: name.clone(),
            });
        }
    }
    let (old, new) = (old.base(), new.base());
    match (old, new) {
        (Json, Jsonb) | (Jsonb, Json) => flags.push(Flag::JsonJsonbChange),
        (Timestamp, Timestamptz) | (Timestamptz, Timestamp) => flags.push(Flag::TimestampTzChange),
        (Bpchar, Varchar | Text) | (Varchar | Text, Bpchar) => flags.push(Flag::CharVarcharChange),
        (Numeric(Some((_, s))), Numeric(Some((_, s2)))) if s != s2 => {
            flags.push(Flag::NumericScaleRaised { scale: *s.max(s2) })
        }
        (Array(a), Array(b)) => {
            if let (Numeric(Some((_, s))), Numeric(Some((_, s2)))) = (a.base(), b.base()) {
                if s != s2 {
                    flags.push(Flag::NumericScaleRaised { scale: *s.max(s2) })
                }
            }
        }
        (Numeric(a), Numeric(b)) if a.is_some() != b.is_some() => {
            flags.push(Flag::NumericScaleUnchecked)
        }
        _ => {}
    }
    for t in [old, new] {
        let base = match t {
            Array(e) => e.base(),
            other => other,
        };
        if t.needs_review() {
            let flag = Flag::NeedsReview {
                type_name: t.name(),
            };
            if !flags.contains(&flag) {
                flags.push(flag);
            }
        }
        let flag = match base {
            Xml => Flag::PrintedFormWarning {
                type_name: "xml".into(),
            },
            Unknown(n) => Flag::UnknownType {
                type_name: n.clone(),
            },
            _ => continue,
        };
        if !flags.contains(&flag) {
            flags.push(flag);
        }
    }
    flags
}

fn check_option(
    option: OptionKind,
    ok: fn(&PgType) -> bool,
    old: &PgType,
    new: &PgType,
) -> Result<(), NormaliseError> {
    for t in [old, new] {
        if !ok(t) {
            return Err(NormaliseError::InvalidOption {
                option,
                type_name: t.name(),
            });
        }
    }
    Ok(())
}

fn control_expr(c: &str, ty: &PgType) -> Result<String, NormaliseError> {
    if !ty.is_totalable() {
        return Err(NormaliseError::ControlTotalType {
            type_name: ty.name(),
        });
    }
    Ok(format!("{c}::numeric"))
}

impl ColumnPlan {
    pub fn needs_review(&self) -> bool {
        self.flags
            .iter()
            .any(|f| matches!(f, Flag::NeedsReview { .. }))
    }
}

/// Build the per-side expressions and report flags for one mapped column.
pub fn plan_column(m: &ColumnMapping) -> Result<ColumnPlan, NormaliseError> {
    let oc = quote_ident(&m.old_name)?;
    let nc = quote_ident(&m.new_name)?;
    let mut flags = type_flags(&m.old_type, &m.new_type);
    let (ot, nt) = (m.old_type.base(), m.new_type.base());
    let opts = &m.options;

    let (mut old_expr, mut new_expr) = if let Some(n) = opts.round_decimals {
        let kind = OptionKind::Round(n);
        if n > MAX_ROUND_DECIMALS {
            return Err(NormaliseError::RoundTooLarge);
        }
        check_option(kind, PgType::is_roundable, ot, nt)?;
        flags.push(Flag::OptionApplied(kind));
        (
            format!("round({oc}::numeric, {n})::text"),
            format!("round({nc}::numeric, {n})::text"),
        )
    } else if opts.date_only {
        check_option(OptionKind::DateOnly, PgType::is_datetime, ot, nt)?;
        flags.push(Flag::OptionApplied(OptionKind::DateOnly));
        (format!("{oc}::date::text"), format!("{nc}::date::text"))
    } else {
        (side_expr(&oc, ot, nt)?, side_expr(&nc, nt, ot)?)
    };
    if opts.round_decimals.is_some() && opts.date_only {
        return Err(NormaliseError::InvalidOption {
            option: OptionKind::DateOnly,
            type_name: ot.name(),
        });
    }
    if opts.ignore_trailing_spaces {
        check_option(OptionKind::IgnoreTrailingSpaces, PgType::is_text, ot, nt)?;
        flags.push(Flag::OptionApplied(OptionKind::IgnoreTrailingSpaces));
        old_expr = format!("rtrim({old_expr})");
        new_expr = format!("rtrim({new_expr})");
    }
    if opts.null_equals_empty {
        check_option(OptionKind::NullEqualsEmpty, PgType::is_text, ot, nt)?;
        flags.push(Flag::OptionApplied(OptionKind::NullEqualsEmpty));
        old_expr = format!("coalesce({old_expr}, '')");
        new_expr = format!("coalesce({new_expr}, '')");
    }
    let (old_control, new_control) = if m.control_total {
        (Some(control_expr(&oc, ot)?), Some(control_expr(&nc, nt)?))
    } else {
        (None, None)
    };
    Ok(ColumnPlan {
        old_expr,
        new_expr,
        old_control,
        new_control,
        flags,
    })
}

const MAX_TYPE_DEPTH: u8 = 16;

const TYPE_SQL: &str = "SELECT t.typtype::text, t.typbasetype, t.typtypmod, \
    format_type(t.oid, $2), t.typelem, t.typcategory::text, t.typrelid, n.nspname::text, \
    (SELECT r.rngsubtype FROM pg_range r WHERE r.rngtypid = t.oid) \
    FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace WHERE t.oid = $1";

const FIELDS_SQL: &str = "SELECT atttypid, atttypmod FROM pg_attribute \
    WHERE attrelid = $1 AND attnum > 0 AND NOT attisdropped ORDER BY attnum";

/// Resolve a column's type from the catalog (`atttypid`, `atttypmod`):
/// domains to their base type (recursively), arrays to their element type,
/// ranges to their subtype and composites to their field types, so the
/// planner applies the real rule instead of a printed form. 9.4-safe.
pub async fn resolve_type(
    client: &impl GenericClient,
    oid: u32,
    typmod: i32,
) -> Result<PgType, CoreError> {
    resolve(client, oid, typmod, 0).await
}

type TypeFuture<'a> = Pin<Box<dyn Future<Output = Result<PgType, CoreError>> + 'a>>;

fn resolve<C: GenericClient>(c: &C, oid: u32, typmod: i32, depth: u8) -> TypeFuture<'_> {
    Box::pin(async move {
        if depth > MAX_TYPE_DEPTH {
            return Err(CoreError::Plan("type nesting too deep"));
        }
        let row = c
            .query_opt(TYPE_SQL, &[&oid, &typmod])
            .await?
            .ok_or(CoreError::Malformed("type not found"))?;
        let typtype: String = row.try_get(0)?;
        let name: String = row.try_get(3)?;
        let elem: u32 = row.try_get(4)?;
        let category: String = row.try_get(5)?;
        let schema: String = row.try_get(7)?;
        Ok(match typtype.as_str() {
            "d" => {
                let base = resolve(c, row.try_get(1)?, row.try_get(2)?, depth + 1).await?;
                PgType::Domain {
                    name,
                    base: Box::new(base),
                }
            }
            "b" if category == "A" && elem != 0 => {
                PgType::Array(Box::new(resolve(c, elem, typmod, depth + 1).await?))
            }
            "b" if schema == "pg_catalog" => PgType::parse(&name),
            "e" => PgType::Enum(name),
            "r" => {
                let sub: Option<u32> = row.try_get(8)?;
                let sub = sub.ok_or(CoreError::Malformed("range without subtype"))?;
                PgType::Range {
                    name,
                    sub: Box::new(resolve(c, sub, -1, depth + 1).await?),
                }
            }
            "c" => {
                let relid: u32 = row.try_get(6)?;
                let mut fields = Vec::new();
                for f in c.query(FIELDS_SQL, &[&relid]).await? {
                    fields.push(resolve(c, f.try_get(0)?, f.try_get(1)?, depth + 1).await?);
                }
                PgType::Composite { name, fields }
            }
            _ => PgType::Unknown(name),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use PgType::*;

    fn map(old: PgType, new: PgType) -> ColumnMapping {
        ColumnMapping {
            old_name: "c".into(),
            new_name: "c".into(),
            old_type: old,
            new_type: new,
            options: ColumnOptions::default(),
            control_total: false,
        }
    }

    fn exprs(old: PgType, new: PgType) -> (String, String) {
        let p = plan_column(&map(old, new)).unwrap();
        (p.old_expr, p.new_expr)
    }

    fn same(t: PgType) -> String {
        let (o, n) = exprs(t.clone(), t);
        assert_eq!(o, n);
        o
    }

    #[test]
    fn parses_format_type() {
        let cases = [
            ("text", Text),
            ("character varying(40)", Varchar),
            ("character(8)", Bpchar),
            ("numeric(20,4)", Numeric(Some((20, 4)))),
            ("numeric(5)", Numeric(Some((5, 0)))),
            ("numeric", Numeric(None)),
            ("real", Float4),
            ("double precision", Float8),
            ("smallint", Int2),
            ("integer", Int4),
            ("bigint", Int8),
            ("date", Date),
            ("timestamp(0) without time zone", Timestamp),
            ("timestamp with time zone", Timestamptz),
            ("time without time zone", Time),
            ("time(3) with time zone", Timetz),
            ("interval year to month", Interval),
            ("bytea", Bytea),
            ("money", Money),
            ("json", Json),
            ("jsonb", Jsonb),
            ("xml", Xml),
            ("uuid", Stable("uuid".into())),
            ("int4range", Stable("int4range".into())),
            ("double precision[]", Array(Box::new(Float8))),
            ("numeric(10,2)[]", Array(Box::new(Numeric(Some((10, 2)))))),
            ("public.geometry", Unknown("public.geometry".into())),
            ("numeric(x,2)", Unknown("numeric(x,2)".into())),
            ("numeric(2,5)", Unknown("numeric(2,5)".into())),
            ("numeric(5,2,1)", Unknown("numeric(5,2,1)".into())),
        ];
        for (s, t) in cases {
            assert_eq!(PgType::parse(s), t, "{s}");
        }
    }

    #[test]
    fn reference_rules_per_type() {
        assert_eq!(same(Text), "\"c\"::text");
        assert_eq!(same(Varchar), "\"c\"::text");
        assert_eq!(same(Bpchar), "\"c\"::text");
        assert_eq!(same(Numeric(Some((18, 4)))), "\"c\"::text");
        assert_eq!(same(Float8), "encode(float8send(\"c\"), 'hex')");
        assert_eq!(same(Float4), "encode(float4send(\"c\"), 'hex')");
        assert_eq!(same(Timestamp), "\"c\"::text");
        assert_eq!(same(Timestamptz), "\"c\"::text");
        assert_eq!(same(Interval), "\"c\"::text");
        assert_eq!(same(Bytea), "md5(\"c\")");
        assert_eq!(same(Money), "\"c\"::numeric::text");
        assert_eq!(same(Json), "\"c\"::text");
        assert_eq!(same(Jsonb), "\"c\"::text");
        assert_eq!(same(Xml), "\"c\"::text");
        assert_eq!(same(Array(Box::new(Int4))), "\"c\"::text");
        assert_eq!(
            same(Array(Box::new(Float8))),
            "encode(array_send(\"c\"::float8[]), 'hex')"
        );
        assert_eq!(same(Array(Box::new(Money))), "\"c\"::numeric[]::text");
        assert_eq!(same(Stable("int4range".into())), "\"c\"::text");
    }

    fn dom(base: PgType) -> PgType {
        Domain {
            name: "public.d".into(),
            base: Box::new(base),
        }
    }

    #[test]
    fn geometric_types_hash_send_bytes() {
        for (name, f) in [
            ("point", "point_send"),
            ("line", "line_send"),
            ("lseg", "lseg_send"),
            ("box", "box_send"),
            ("path", "path_send"),
            ("polygon", "poly_send"),
            ("circle", "circle_send"),
        ] {
            let t = PgType::parse(name);
            assert_eq!(same(t.clone()), format!("encode({f}(\"c\"), 'hex')"));
            assert_eq!(
                same(Array(Box::new(t))),
                format!("encode(array_send(\"c\"::{name}[]), 'hex')")
            );
        }
        assert!(matches!(PgType::parse("point(1)"), Unknown(_)));
        assert!(matches!(PgType::parse("macaddr8"), Unknown(_)));
    }

    #[test]
    fn domains_use_the_base_rule() {
        let p = plan_column(&map(dom(Float8), dom(Float8))).unwrap();
        assert_eq!(p.old_expr, "encode(float8send(\"c\"), 'hex')");
        assert!(p.flags.contains(&Flag::DomainResolved {
            type_name: "public.d".into()
        }));
        assert!(!p.needs_review());
        let nested = dom(dom(Array(Box::new(Float4))));
        let (o, n) = exprs(nested, Array(Box::new(Float8)));
        assert_eq!(o, "encode(array_send(\"c\"::float8[]), 'hex')");
        assert_eq!(n, o);
        let (o, _) = exprs(Array(Box::new(dom(Float8))), Array(Box::new(dom(Float8))));
        assert_eq!(o, "encode(array_send(\"c\"::float8[]), 'hex')");
        let (o, n) = exprs(dom(Numeric(Some((10, 2)))), Numeric(Some((10, 4))));
        assert_eq!(o, "\"c\"::numeric(12,4)::text");
        assert_eq!(n, o);
        let mut m = map(dom(Money), Money);
        m.control_total = true;
        m.options.round_decimals = Some(2);
        assert!(plan_column(&m).is_ok());
        let p = plan_column(&map(dom(Unknown("x".into())), Text)).unwrap();
        assert!(p.needs_review());
    }

    #[test]
    fn numeric_arrays_widen_element_wise() {
        let a = Array(Box::new(Numeric(Some((10, 2)))));
        let b = Array(Box::new(Numeric(Some((10, 4)))));
        let p = plan_column(&map(a.clone(), b)).unwrap();
        assert_eq!(p.old_expr, "\"c\"::numeric(12,4)[]::text");
        assert_eq!(p.new_expr, p.old_expr);
        assert!(p.flags.contains(&Flag::NumericScaleRaised { scale: 4 }));
        assert_eq!(same(a), "\"c\"::text");
        let wide = Array(Box::new(Numeric(Some((1000, 1000)))));
        let narrow = Array(Box::new(Numeric(Some((1000, 0)))));
        assert_eq!(
            plan_column(&map(wide, narrow)),
            Err(NormaliseError::NumericTooWide)
        );
        let (o, n) = exprs(Array(Box::new(Money)), Array(Box::new(Text)));
        assert_eq!(o, "\"c\"::numeric[]::text");
        assert_eq!(n, "\"c\"::text");
        let (o, n) = exprs(Array(Box::new(Float8)), Text);
        assert_eq!(o, "encode(array_send(\"c\"::float8[]), 'hex')");
        assert_eq!(n, "\"c\"::text");
        let (o, _) = exprs(Array(Box::new(Money)), Text);
        assert_eq!(o, "\"c\"::numeric[]::text");
    }

    #[test]
    fn printed_forms_that_are_not_exact_need_review() {
        let range = |sub: PgType| Range {
            name: "public.r".into(),
            sub: Box::new(sub),
        };
        let comp = |fields: Vec<PgType>| Composite {
            name: "public.c".into(),
            fields,
        };
        // Float ranges are hashed by bytes: exact, no review.
        let p = plan_column(&map(range(Float8), range(Float8))).unwrap();
        assert_eq!(p.old_expr, "encode(range_send(\"c\"), 'hex')");
        assert!(!p.needs_review());
        assert!(!plan_column(&map(range(Int4), range(Int4)))
            .unwrap()
            .needs_review());
        assert!(plan_column(&map(range(Money), range(Money)))
            .unwrap()
            .needs_review());
        let stable = comp(vec![Int4, Text, dom(Numeric(None)), Enum("e".into())]);
        assert!(!plan_column(&map(stable.clone(), stable))
            .unwrap()
            .needs_review());
        for bad in [
            comp(vec![Int4, Float8]),
            comp(vec![Geometric(Geo::Point)]),
            comp(vec![comp(vec![Unknown("x".into())])]),
            Unknown("public.geometry".into()),
            Array(Box::new(comp(vec![Float4]))),
            Array(Box::new(range(Xml))),
        ] {
            let p = plan_column(&map(bad.clone(), bad)).unwrap();
            assert!(p.needs_review(), "{p:?}");
            assert_eq!(p.old_expr, "\"c\"::text");
        }
        let stable_arrays = [
            Array(Box::new(Float8)),
            Array(Box::new(Money)),
            Array(Box::new(Xml)),
            Array(Box::new(Int4)),
            Array(Box::new(range(Int4))),
        ];
        for ok in stable_arrays {
            assert!(!plan_column(&map(ok.clone(), ok)).unwrap().needs_review());
        }
        assert!(!plan_column(&map(Enum("e".into()), Enum("e".into())))
            .unwrap()
            .needs_review());
    }

    #[test]
    fn numeric_scale_change_casts_both_up_never_down() {
        let p = plan_column(&map(Numeric(Some((20, 4))), Numeric(Some((20, 6))))).unwrap();
        assert_eq!(p.old_expr, "\"c\"::numeric(22,6)::text");
        assert_eq!(p.new_expr, "\"c\"::numeric(22,6)::text");
        assert!(p.flags.contains(&Flag::NumericScaleRaised { scale: 6 }));
        let p = plan_column(&map(Numeric(Some((20, 6))), Numeric(Some((10, 2))))).unwrap();
        assert_eq!(p.old_expr, "\"c\"::numeric(20,6)::text");
        assert_eq!(p.old_expr, p.new_expr);
        let p = plan_column(&map(Numeric(None), Numeric(Some((10, 2))))).unwrap();
        assert_eq!(p.old_expr, "\"c\"::text");
        assert!(p.flags.contains(&Flag::NumericScaleUnchecked));
        assert_eq!(
            plan_column(&map(Numeric(Some((1000, 0))), Numeric(Some((1000, 1000))))),
            Err(NormaliseError::NumericTooWide)
        );
    }

    #[test]
    fn float_widening_and_arrays() {
        let (o, n) = exprs(Float4, Float8);
        assert_eq!(o, "encode(float8send(\"c\"::float8), 'hex')");
        assert_eq!(n, "encode(float8send(\"c\"), 'hex')");
        let (o, n) = exprs(Array(Box::new(Float4)), Array(Box::new(Float8)));
        assert_eq!(o, "encode(array_send(\"c\"::float8[]), 'hex')");
        assert_eq!(n, "encode(array_send(\"c\"::float8[]), 'hex')");
    }

    #[test]
    fn type_change_flags() {
        let p = plan_column(&map(Json, Jsonb)).unwrap();
        assert!(p.flags.contains(&Flag::JsonJsonbChange));
        // Never canonicalised to look equal: the json side keeps its exact text.
        assert_eq!(p.old_expr, "\"c\"::text");
        let p = plan_column(&map(Jsonb, Json)).unwrap();
        assert!(p.flags.contains(&Flag::JsonJsonbChange));
        let p = plan_column(&map(Timestamp, Timestamptz)).unwrap();
        assert!(p.flags.contains(&Flag::TimestampTzChange));
        let p = plan_column(&map(Bpchar, Varchar)).unwrap();
        assert!(p.flags.contains(&Flag::CharVarcharChange));
        assert!(matches!(p.flags[0], Flag::TypeChanged { .. }));
        let p = plan_column(&map(Xml, Xml)).unwrap();
        assert_eq!(
            p.flags,
            vec![Flag::PrintedFormWarning {
                type_name: "xml".into()
            }]
        );
        let u = Unknown("public.geometry".into());
        let p = plan_column(&map(u.clone(), u.clone())).unwrap();
        // The warning is kept, and the column can never be a clean PASS.
        assert_eq!(p.flags.len(), 2);
        assert!(p.needs_review());
        assert!(p.flags.contains(&Flag::UnknownType {
            type_name: "public.geometry".into()
        }));
        assert_eq!(p.old_expr, "\"c\"::text");
        let p = plan_column(&map(Array(Box::new(u.clone())), Array(Box::new(u)))).unwrap();
        assert!(p.needs_review());
        assert!(p
            .flags
            .iter()
            .any(|f| matches!(f, Flag::UnknownType { .. })));
        assert!(plan_column(&map(Int4, Int4)).unwrap().flags.is_empty());
    }

    #[test]
    fn options_apply_and_are_recorded() {
        let mut m = map(Varchar, Bpchar);
        m.options.ignore_trailing_spaces = true;
        m.options.null_equals_empty = true;
        let p = plan_column(&m).unwrap();
        assert_eq!(p.old_expr, "coalesce(rtrim(\"c\"::text), '')");
        assert!(p
            .flags
            .contains(&Flag::OptionApplied(OptionKind::IgnoreTrailingSpaces)));
        assert!(p
            .flags
            .contains(&Flag::OptionApplied(OptionKind::NullEqualsEmpty)));

        let mut m = map(Timestamp, Timestamptz);
        m.options.date_only = true;
        let p = plan_column(&m).unwrap();
        assert_eq!(p.new_expr, "\"c\"::date::text");
        assert!(p.flags.contains(&Flag::OptionApplied(OptionKind::DateOnly)));

        let mut m = map(Float8, Money);
        m.options.round_decimals = Some(2);
        let p = plan_column(&m).unwrap();
        assert_eq!(p.old_expr, "round(\"c\"::numeric, 2)::text");
        assert!(p.flags.contains(&Flag::OptionApplied(OptionKind::Round(2))));
    }

    #[test]
    fn invalid_options_are_rejected() {
        let mut m = map(Int4, Int4);
        m.options.ignore_trailing_spaces = true;
        assert!(matches!(
            plan_column(&m),
            Err(NormaliseError::InvalidOption { .. })
        ));
        let mut m = map(Text, Int4);
        m.options.null_equals_empty = true;
        assert!(plan_column(&m).is_err());
        let mut m = map(Text, Text);
        m.options.date_only = true;
        assert!(plan_column(&m).is_err());
        let mut m = map(Text, Text);
        m.options.round_decimals = Some(2);
        assert!(plan_column(&m).is_err());
        let mut m = map(Numeric(None), Numeric(None));
        m.options.round_decimals = Some(MAX_ROUND_DECIMALS + 1);
        assert_eq!(plan_column(&m), Err(NormaliseError::RoundTooLarge));
        let mut m = map(Date, Date);
        m.options.round_decimals = Some(1);
        assert!(plan_column(&m).is_err());
        let mut m = map(Numeric(None), Numeric(None));
        m.options.round_decimals = Some(1);
        m.options.date_only = true;
        assert!(plan_column(&m).is_err());
        let mut m = map(Text, Text);
        m.old_name = "a\0".into();
        assert!(matches!(plan_column(&m), Err(NormaliseError::Ident(_))));
        assert!(!NormaliseError::NumericTooWide.to_string().is_empty());
    }

    #[test]
    fn control_totals() {
        let mut m = map(Money, Numeric(Some((20, 2))));
        m.control_total = true;
        let p = plan_column(&m).unwrap();
        assert_eq!(p.old_control.as_deref(), Some("\"c\"::numeric"));
        assert_eq!(p.new_control.as_deref(), Some("\"c\"::numeric"));
        let mut m = map(Float8, Float8);
        m.control_total = true;
        assert!(matches!(
            plan_column(&m),
            Err(NormaliseError::ControlTotalType { .. })
        ));
    }
}
