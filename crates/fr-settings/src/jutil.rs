//! Small Java-semantics helpers used by the settings code (kept local so this
//! crate does not depend on `fr-jcompat`).

/// `java.lang.Math.round(double)`: rounds half up; NaN -> 0; saturates.
pub(crate) fn java_round(x: f64) -> i64 {
    if x.is_nan() {
        return 0;
    }
    let f = x.floor();
    let r = if x - f >= 0.5 { f + 1.0 } else { f };
    r as i64 // saturating, like Java
}

/// `String.split(String regex)` for a regex that is a single character class:
/// trailing empty strings are removed; an input without any delimiter yields itself.
pub(crate) fn java_split(s: &str, is_delim: impl Fn(char) -> bool) -> Vec<&str> {
    if !s.chars().any(&is_delim) {
        return vec![s];
    }
    let mut parts: Vec<&str> = s.split(is_delim).collect();
    while parts.last().is_some_and(|p| p.is_empty()) {
        parts.pop();
    }
    parts
}

/// `String.trim()`: strips chars `<= ' '` from both ends.
pub(crate) fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// `String.equalsIgnoreCase` (per-char upper/lower comparison).
pub(crate) fn java_equals_ignore_case(a: &str, b: &str) -> bool {
    fn up(c: char) -> char {
        let mut it = c.to_uppercase();
        match (it.next(), it.next()) {
            (Some(u), None) => u,
            _ => c,
        }
    }
    fn low(c: char) -> char {
        let mut it = c.to_lowercase();
        match (it.next(), it.next()) {
            (Some(l), None) => l,
            _ => c,
        }
    }
    let (mut ia, mut ib) = (a.chars(), b.chars());
    loop {
        match (ia.next(), ib.next()) {
            (None, None) => return true,
            (Some(x), Some(y)) => {
                if x == y {
                    continue;
                }
                let (ux, uy) = (up(x), up(y));
                if ux == uy || low(ux) == low(uy) {
                    continue;
                }
                return false;
            }
            _ => return false,
        }
    }
}

/// `String.isBlank()`.
pub(crate) fn java_is_blank(s: &str) -> bool {
    s.chars().all(char::is_whitespace)
}

/// `Integer.parseInt(String)` (no trimming; optional sign; ASCII digits).
pub(crate) fn java_parse_int(s: &str) -> Option<i32> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// `Long.parseLong(String)`.
pub(crate) fn java_parse_long(s: &str) -> Option<i64> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// Decimal part of `Double.parseDouble` / `Float.parseFloat`: trims, accepts an
/// optional `f/F/d/D` suffix, `NaN` and `Infinity`. Hex literals are not supported.
fn java_float_literal(s: &str) -> Option<(bool, &str)> {
    let t = java_trim(s);
    let (neg, body) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    if body == "NaN" || body == "Infinity" {
        return Some((neg, body));
    }
    let body = body.strip_suffix(['f', 'F', 'd', 'D']).unwrap_or(body);
    let ok = !body.is_empty()
        && body
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'))
        && body.bytes().any(|b| b.is_ascii_digit());
    ok.then_some((neg, body))
}

pub(crate) fn java_parse_double(s: &str) -> Option<f64> {
    let (neg, body) = java_float_literal(s)?;
    let v: f64 = match body {
        "NaN" => f64::NAN,
        "Infinity" => f64::INFINITY,
        b => b.parse().ok()?,
    };
    Some(if neg { -v } else { v })
}

pub(crate) fn java_parse_float(s: &str) -> Option<f32> {
    let (neg, body) = java_float_literal(s)?;
    let v: f32 = match body {
        "NaN" => f32::NAN,
        "Infinity" => f32::INFINITY,
        b => b.parse().ok()?,
    };
    Some(if neg { -v } else { v })
}

/// `Boolean.parseBoolean` preceded by ReflectionUtil's `"0"`/`"1"` mapping.
pub(crate) fn reflection_parse_bool(s: &str) -> bool {
    match s {
        "0" => false,
        "1" => true,
        _ => s.eq_ignore_ascii_case("true"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round() {
        assert_eq!(java_round(0.49999999999999994), 0);
        assert_eq!(java_round(-2.5), -2);
        assert_eq!(java_round(2.5), 3);
        assert_eq!(java_round(f64::INFINITY), i64::MAX);
        assert_eq!(java_round(f64::NAN), 0);
    }

    #[test]
    fn split() {
        assert_eq!(java_split("", |c| c == ','), vec![""]);
        assert_eq!(java_split("a,b,,", |c| c == ','), vec!["a", "b"]);
        assert_eq!(java_split(",", |c| c == ','), Vec::<&str>::new());
        assert_eq!(java_split(",a", |c| c == ','), vec!["", "a"]);
    }

    #[test]
    fn parse() {
        assert_eq!(java_parse_int("+5"), Some(5));
        assert_eq!(java_parse_int(" 5"), None);
        assert_eq!(java_parse_double(" 2.5d "), Some(2.5));
        assert_eq!(java_parse_double("inf"), None);
        assert_eq!(java_parse_float("-Infinity"), Some(f32::NEG_INFINITY));
        assert!(java_equals_ignore_case("Kicad_Default", "kicad_default"));
    }
}
