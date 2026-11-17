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
//! form, which stays UTF-8 text as DECISIONS 4 requires.

use crate::ident::{quote_ident, IdentError};
use serde::{Deserialize, Serialize};

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
    Array(Box<PgType>),
    /// Extension or unrecognised types: printed form, with a warning.
    Unknown(String),
}

const STABLE: &[&str] = &[
    "boolean",
    "uuid",
    "inet",
    "cidr",
    "macaddr",
    "macaddr8",
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
            _ => PgType::Unknown(s.to_string()),
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
    Ok(match (ty, other) {
        (PgType::Float4, PgType::Float8) => format!("encode(float8send({c}::float8), 'hex')"),
        (PgType::Float4, _) => format!("encode(float4send({c}), 'hex')"),
        (PgType::Float8, _) => format!("encode(float8send({c}), 'hex')"),
        (PgType::Numeric(Some((p, s))), PgType::Numeric(Some((p2, s2)))) if s != s2 => {
            let scale = (*s).max(*s2);
            let precision = p.saturating_sub(*s).max(p2.saturating_sub(*s2)) + scale;
            if precision > NUMERIC_MAX_PRECISION {
                return Err(NormaliseError::NumericTooWide);
            }
            format!("{c}::numeric({precision},{scale})::text")
        }
        (PgType::Money, _) => format!("{c}::numeric::text"),
        (PgType::Bytea, _) => format!("md5({c})"),
        (PgType::Array(e), _) if **e == PgType::Money => format!("{c}::numeric[]::text"),
        (PgType::Array(e), PgType::Array(o)) if **e == PgType::Float4 && **o == PgType::Float8 => {
            format!("encode(array_send({c}::float8[]), 'hex')")
        }
        (PgType::Array(e), _) if e.is_float() => format!("encode(array_send({c}), 'hex')"),
        _ => format!("{c}::text"),
    })
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
    match (old, new) {
        (Json, Jsonb) | (Jsonb, Json) => flags.push(Flag::JsonJsonbChange),
        (Timestamp, Timestamptz) | (Timestamptz, Timestamp) => flags.push(Flag::TimestampTzChange),
        (Bpchar, Varchar | Text) | (Varchar | Text, Bpchar) => flags.push(Flag::CharVarcharChange),
        (Numeric(Some((_, s))), Numeric(Some((_, s2)))) if s != s2 => {
            flags.push(Flag::NumericScaleRaised { scale: *s.max(s2) })
        }
        (Numeric(a), Numeric(b)) if a.is_some() != b.is_some() => {
            flags.push(Flag::NumericScaleUnchecked)
        }
        _ => {}
    }
    for t in [old, new] {
        let base = match t {
            Array(e) => e.as_ref(),
            other => other,
        };
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

/// Build the per-side expressions and report flags for one mapped column.
pub fn plan_column(m: &ColumnMapping) -> Result<ColumnPlan, NormaliseError> {
    let oc = quote_ident(&m.old_name)?;
    let nc = quote_ident(&m.new_name)?;
    let (ot, nt) = (&m.old_type, &m.new_type);
    let mut flags = type_flags(ot, nt);
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
            "encode(array_send(\"c\"), 'hex')"
        );
        assert_eq!(same(Array(Box::new(Money))), "\"c\"::numeric[]::text");
        assert_eq!(same(Stable("int4range".into())), "\"c\"::text");
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
        assert_eq!(n, "encode(array_send(\"c\"), 'hex')");
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
        assert_eq!(p.flags.len(), 1);
        assert_eq!(p.old_expr, "\"c\"::text");
        let p = plan_column(&map(Array(Box::new(u.clone())), Array(Box::new(u)))).unwrap();
        assert!(matches!(p.flags[0], Flag::UnknownType { .. }));
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
