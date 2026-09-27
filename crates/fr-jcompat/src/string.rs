//! `String.compareTo` / `String.compareToIgnoreCase` (UTF-16 code unit semantics).

use std::cmp::Ordering;

/// `String.compareTo(other)`: difference of the first mismatching UTF-16 code units, otherwise
/// the difference of the lengths (in UTF-16 units).
pub fn java_string_compare(a: &str, b: &str) -> i32 {
    if a.is_ascii() && b.is_ascii() {
        return compare_utf16_iter(a.bytes().map(u16::from), b.bytes().map(u16::from), a.len(), b.len());
    }
    let (la, lb) = (a.encode_utf16().count(), b.encode_utf16().count());
    compare_utf16_iter(a.encode_utf16(), b.encode_utf16(), la, lb)
}

/// `String.compareTo` on raw UTF-16 code units.
pub fn java_string_compare_utf16(a: &[u16], b: &[u16]) -> i32 {
    compare_utf16_iter(a.iter().copied(), b.iter().copied(), a.len(), b.len())
}

fn compare_utf16_iter(a: impl Iterator<Item = u16>, b: impl Iterator<Item = u16>, la: usize, lb: usize) -> i32 {
    for (x, y) in a.zip(b) {
        if x != y {
            return x as i32 - y as i32;
        }
    }
    la as i32 - lb as i32
}

/// [`java_string_compare`] as an [`Ordering`].
pub fn java_string_ordering(a: &str, b: &str) -> Ordering {
    java_string_compare(a, b).cmp(&0)
}

/// `Character.toUpperCase(int)` (simple case mapping).
///
/// Exact for code points whose full uppercase mapping is a single code point (all of Latin-1
/// and the vast majority of Unicode); for the few characters whose full mapping expands
/// (e.g. `ß`, `ŉ`, ligatures) the character is returned unchanged, which matches Java except
/// for Greek characters with iota subscript (e.g. U+1FB3), whose Java simple mapping is a titlecase letter.
/// Unicode version differences between Rust's and the JDK's tables are possible.
pub fn java_to_upper_case(cp: u32) -> u32 {
    match char::from_u32(cp) {
        Some(c) => {
            let mut it = c.to_uppercase();
            let first = it.next().expect("non-empty mapping");
            if it.next().is_none() {
                first as u32
            } else {
                cp
            }
        }
        None => cp,
    }
}

/// `Character.toLowerCase(int)` (simple case mapping; see [`java_to_upper_case`]).
pub fn java_to_lower_case(cp: u32) -> u32 {
    if cp == 0x130 {
        // LATIN CAPITAL LETTER I WITH DOT ABOVE: simple lowercase mapping is 'i'
        return 0x69;
    }
    match char::from_u32(cp) {
        Some(c) => {
            let mut it = c.to_lowercase();
            let first = it.next().expect("non-empty mapping");
            if it.next().is_none() {
                first as u32
            } else {
                cp
            }
        }
        None => cp,
    }
}

/// `StringUTF16.compareCodePointCI`.
fn compare_code_point_ci(cp1: u32, cp2: u32) -> i32 {
    let u1 = java_to_upper_case(cp1);
    let u2 = java_to_upper_case(cp2);
    if u1 != u2 {
        let l1 = java_to_lower_case(u1);
        let l2 = java_to_lower_case(u2);
        if l1 != l2 {
            return l1 as i32 - l2 as i32;
        }
    }
    0
}

/// `String.compareToIgnoreCase(other)` (`String.CASE_INSENSITIVE_ORDER`).
pub fn compare_to_ignore_case(a: &str, b: &str) -> i32 {
    let a: Vec<u16> = a.encode_utf16().collect();
    let b: Vec<u16> = b.encode_utf16().collect();
    compare_to_ignore_case_utf16(&a, &b)
}

/// `String.compareToIgnoreCase` on raw UTF-16 code units: compares unit by unit; when both
/// strings are non-LATIN1, a mismatch involving a surrogate pair compares the whole
/// supplementary code point (`StringUTF16.compareToCIImpl`).
pub fn compare_to_ignore_case_utf16(a: &[u16], b: &[u16]) -> i32 {
    let (la, lb) = (a.len(), b.len());
    // With compact strings, a string whose units are all <= 0xFF is stored as LATIN1; the
    // LATIN1 variants (`StringLatin1.compareToCI`, `compareToCI_UTF16`) compare unit by unit
    // and never combine surrogate pairs.
    let latin1 = |s: &[u16]| s.iter().all(|&c| c <= 0xFF);
    if latin1(a) || latin1(b) {
        for (&c1, &c2) in a.iter().zip(b.iter()) {
            if c1 != c2 {
                let d = compare_code_point_ci(c1 as u32, c2 as u32);
                if d != 0 {
                    return d;
                }
            }
        }
        return la as i32 - lb as i32;
    }
    let (mut k1, mut k2) = (0usize, 0usize);
    while k1 < la && k2 < lb {
        let c1 = a[k1] as u32;
        let c2 = b[k2] as u32;
        if c1 == c2 || compare_code_point_ci(c1, c2) == 0 {
            k1 += 1;
            k2 += 1;
            continue;
        }
        let (cp1, adv1) = code_point_including(a, k1);
        let (cp2, adv2) = code_point_including(b, k2);
        let diff = compare_code_point_ci(cp1, cp2);
        if diff != 0 {
            return diff;
        }
        k1 += 1 + adv1;
        k2 += 1 + adv2;
    }
    la as i32 - lb as i32
}

/// Code point containing the unit at `k` (a high surrogate followed by a low one, or a low
/// surrogate preceded by a high one); returns the code point and how many *extra* units to skip.
fn code_point_including(s: &[u16], k: usize) -> (u32, usize) {
    let c = s[k] as u32;
    if (0xD800..0xDC00).contains(&c) && k + 1 < s.len() {
        let d = s[k + 1] as u32;
        if (0xDC00..0xE000).contains(&d) {
            return (0x10000 + ((c - 0xD800) << 10) + (d - 0xDC00), 1);
        }
    }
    if (0xDC00..0xE000).contains(&c) && k > 0 {
        let h = s[k - 1] as u32;
        if (0xD800..0xDC00).contains(&h) {
            return (0x10000 + ((h - 0xD800) << 10) + (c - 0xDC00), 0);
        }
    }
    (c, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare() {
        assert_eq!(java_string_compare("a", "b"), -1);
        assert_eq!(java_string_compare("abc", "ab"), 1);
        assert_eq!(java_string_compare("", "abc"), -3);
        assert_eq!(java_string_compare("Net", "net"), 'N' as i32 - 'n' as i32);
        assert_eq!(compare_to_ignore_case("Net1", "NET1"), 0);
        assert_eq!(compare_to_ignore_case("a", "B"), -1);
        assert_eq!(compare_to_ignore_case("_", "a"), '_' as i32 - 'a' as i32);
    }
}
