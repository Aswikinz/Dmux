//! The one quoting helper for table, schema and column names (AGENTS.md rule 2).
//!
//! Every identifier is always double-quoted, embedded quotes are doubled, and
//! names Postgres could not hold (empty, NUL, longer than 63 bytes, which the
//! server would silently truncate into a different name) are rejected.

/// Longest identifier Postgres keeps (NAMEDATALEN - 1); longer ones are truncated.
pub const MAX_IDENT_BYTES: usize = 63;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdentError {
    #[error("a name may not be empty")]
    Empty,
    #[error("a name may not contain a NUL character")]
    Nul,
    #[error("a name may not be longer than 63 bytes")]
    TooLong,
}

/// Quote one identifier: `a"b` becomes `"a""b"`.
pub fn quote_ident(name: &str) -> Result<String, IdentError> {
    if name.is_empty() {
        return Err(IdentError::Empty);
    }
    if name.contains('\0') {
        return Err(IdentError::Nul);
    }
    if name.len() > MAX_IDENT_BYTES {
        return Err(IdentError::TooLong);
    }
    let mut out = String::with_capacity(name.len() + 2);
    out.push('"');
    for ch in name.chars() {
        if ch == '"' {
            out.push('"');
        }
        out.push(ch);
    }
    out.push('"');
    Ok(out)
}

/// Quote a schema-qualified name: `"schema"."table"`.
pub fn quote_qualified(schema: &str, name: &str) -> Result<String, IdentError> {
    Ok(format!("{}.{}", quote_ident(schema)?, quote_ident(name)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Read one SQL quoted identifier from the start of `s`; return it and the rest.
    fn read_quoted(s: &str) -> Option<(String, &str)> {
        let mut chars = s.char_indices();
        if chars.next()?.1 != '"' {
            return None;
        }
        let mut out = String::new();
        let mut pending_quote = false;
        for (i, ch) in chars {
            if pending_quote {
                if ch == '"' {
                    out.push('"');
                    pending_quote = false;
                    continue;
                }
                return Some((out, &s[i..]));
            }
            if ch == '"' {
                pending_quote = true;
            } else {
                out.push(ch);
            }
        }
        pending_quote.then_some((out, ""))
    }

    #[test]
    fn quotes_and_rejects() {
        assert_eq!(quote_ident("acct_id").unwrap(), "\"acct_id\"");
        assert_eq!(quote_ident("a\"b").unwrap(), "\"a\"\"b\"");
        assert_eq!(quote_ident(""), Err(IdentError::Empty));
        assert_eq!(quote_ident("a\0b"), Err(IdentError::Nul));
        assert_eq!(quote_ident(&"x".repeat(64)), Err(IdentError::TooLong));
        assert!(quote_ident(&"x".repeat(63)).is_ok());
        assert_eq!(
            quote_qualified("phase0", "Acc\"ts").unwrap(),
            "\"phase0\".\"Acc\"\"ts\""
        );
        assert_eq!(quote_qualified("", "t"), Err(IdentError::Empty));
        assert_eq!(quote_qualified("s", "a\0"), Err(IdentError::Nul));
        assert!(!IdentError::TooLong.to_string().is_empty());
    }

    proptest! {
        #[test]
        fn quoted_ident_is_one_token(name in "[^\u{0}]{1,40}") {
            prop_assume!(name.len() <= MAX_IDENT_BYTES);
            let q = quote_ident(&name).unwrap();
            // The whole output is exactly one quoted identifier holding the input.
            let (parsed, rest) = read_quoted(&q).expect("quoted identifier");
            prop_assert_eq!(parsed, name);
            prop_assert_eq!(rest, "");
        }

        #[test]
        fn hostile_names_stay_inside_quotes(
            a in "[a-z]{0,5}", b in "[a-z ;]{0,5}"
        ) {
            let name = format!("{a}\"; DROP TABLE x; --{b}");
            let q = quote_ident(&name).unwrap();
            let (parsed, rest) = read_quoted(&q).unwrap();
            prop_assert_eq!(parsed, name);
            prop_assert_eq!(rest, "");
        }
    }
}
