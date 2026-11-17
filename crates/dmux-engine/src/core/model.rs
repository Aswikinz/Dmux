//! Test-only model of the fingerprint: md5, Postgres `record_out` and the
//! bucket sums, computed in Rust. The property tests use it, and the database
//! tests check that Postgres gives exactly the same summaries.

use super::fingerprint::BucketSummary;
use std::collections::BTreeMap;

pub fn md5_hex(data: &[u8]) -> String {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: Vec<u32> = (0..64)
        .map(|i| ((i as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32)
        .collect();
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_le_bytes());
    let mut h: [u32; 4] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476];
    for chunk in msg.chunks(64) {
        let m: Vec<u32> = chunk
            .chunks(4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
            .collect();
        let [mut a, mut b, mut c, mut d] = h;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d]) {
            *x = x.wrapping_add(y);
        }
    }
    h.iter()
        .flat_map(|w| w.to_le_bytes())
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Postgres `record_out` for a row of text fields (NULL prints as nothing).
pub fn record_text(fields: &[Option<String>]) -> String {
    let parts: Vec<String> = fields
        .iter()
        .map(|f| match f {
            None => String::new(),
            Some(v) => {
                let quote = v.is_empty()
                    || v.chars().any(|c| {
                        matches!(c, '"' | '\\' | '(' | ')' | ',' | ' ' | '\t' | '\n' | '\r')
                            || c == '\u{b}'
                            || c == '\u{c}'
                    });
                if !quote {
                    return v.clone();
                }
                let mut s = String::from('"');
                for c in v.chars() {
                    if c == '"' || c == '\\' {
                        s.push(c);
                    }
                    s.push(c);
                }
                s.push('"');
                s
            }
        })
        .collect();
    format!("({})", parts.join(","))
}

/// Summaries for rows of already-normalised text values; `key` empty = keyless.
pub fn summarise(rows: &[Vec<Option<String>>], key: &[usize], digits: u8) -> Vec<BucketSummary> {
    let mut map: BTreeMap<u32, BucketSummary> = BTreeMap::new();
    for r in rows {
        let rh = md5_hex(record_text(r).as_bytes());
        let kh = if key.is_empty() {
            rh.clone()
        } else {
            let k: Vec<Option<String>> = key.iter().map(|i| r[*i].clone()).collect();
            md5_hex(record_text(&k).as_bytes())
        };
        let bucket = u32::from_str_radix(&kh[..digits as usize], 16).unwrap();
        let e = map.entry(bucket).or_insert(BucketSummary {
            bucket,
            n: 0,
            s: [0; 4],
        });
        e.n += 1;
        for j in 0..4 {
            e.s[j] += u128::from(u32::from_str_radix(&rh[j * 8..j * 8 + 8], 16).unwrap());
        }
    }
    map.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn md5_vectors() {
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            md5_hex("1234567890".repeat(8).as_bytes()),
            "57edf4a22be3c955ac49da2e2107b67a"
        );
    }

    #[test]
    fn record_text_rules() {
        let f = |v: &[Option<&str>]| {
            record_text(&v.iter().map(|x| x.map(String::from)).collect::<Vec<_>>())
        };
        assert_eq!(f(&[Some("a"), None, Some("")]), "(a,,\"\")");
        assert_eq!(
            f(&[Some("a b"), Some("x\"y\\")]),
            "(\"a b\",\"x\"\"y\\\\\")"
        );
    }

    fn rows_strategy() -> impl Strategy<Value = Vec<Vec<Option<String>>>> {
        prop::collection::vec(
            prop::collection::vec(prop::option::of("[a-z ,()\"\\\\é]{0,6}"), 3),
            1..40,
        )
        .prop_map(|mut rows| {
            for (i, r) in rows.iter_mut().enumerate() {
                r[0] = Some(i.to_string()); // unique key
            }
            rows
        })
    }

    proptest! {
        #[test]
        fn permutation_never_changes_fingerprint(rows in rows_strategy(), seed in any::<u64>()) {
            let mut shuffled = rows.clone();
            let n = shuffled.len();
            let mut x = seed | 1;
            for i in (1..n).rev() {
                x ^= x << 13; x ^= x >> 7; x ^= x << 17;
                shuffled.swap(i, (x % (i as u64 + 1)) as usize);
            }
            prop_assert_eq!(summarise(&rows, &[0], 3), summarise(&shuffled, &[0], 3));
            prop_assert_eq!(summarise(&rows, &[], 2), summarise(&shuffled, &[], 2));
        }

        #[test]
        fn any_single_value_change_changes_a_bucket(
            rows in rows_strategy(), r in any::<prop::sample::Index>(),
            c in 1usize..3, v in prop::option::of("[a-z ]{0,4}")
        ) {
            let mut changed = rows.clone();
            let i = r.index(rows.len());
            prop_assume!(changed[i][c] != v);
            changed[i][c] = v;
            prop_assert_ne!(summarise(&rows, &[0], 3), summarise(&changed, &[0], 3));
            prop_assert_ne!(summarise(&rows, &[], 3), summarise(&changed, &[], 3));
        }

        #[test]
        fn duplicates_are_counted(rows in rows_strategy(), r in any::<prop::sample::Index>()) {
            let mut dup = rows.clone();
            dup.push(rows[r.index(rows.len())].clone());
            prop_assert_ne!(summarise(&rows, &[], 3), summarise(&dup, &[], 3));
            // Two copies of a pair must not cancel, as XOR would.
            let mut twice = dup.clone();
            twice.push(dup.last().unwrap().clone());
            prop_assert_ne!(summarise(&rows, &[], 3), summarise(&twice, &[], 3));
        }
    }
}
