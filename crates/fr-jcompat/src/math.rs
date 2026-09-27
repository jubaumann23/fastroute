//! `java.lang.Math` rounding, primitive casts and `Double.toString` / `Float.toString`.

use std::cmp::Ordering;

/// Java `Math.round(double)` (JDK 7+): `floor(x + 1/2)` computed exactly, NaN -> 0,
/// saturating at `Long.MIN_VALUE` / `Long.MAX_VALUE`.
///
/// `Math.round(0.49999999999999994) == 0` and `Math.round(-0.5) == 0`.
#[inline]
pub fn java_round_f64(x: f64) -> i64 {
    if x.is_nan() {
        return 0;
    }
    let f = x.floor();
    // x - floor(x) is the exact fractional part (always representable).
    let r = if x - f >= 0.5 { f + 1.0 } else { f };
    r as i64 // saturating cast, same as Java's clamping
}

/// Java `Math.round(float)` -> `int` (JDK 7+ semantics, saturating, NaN -> 0).
#[inline]
pub fn java_round_f32(x: f32) -> i32 {
    if x.is_nan() {
        return 0;
    }
    let f = x.floor();
    let r = if x - f >= 0.5 { f + 1.0 } else { f };
    r as i32
}

/// Alias of [`java_round_f64`] (name used by the original `fr-geom::java_compat`).
#[inline]
pub fn math_round(x: f64) -> i64 {
    java_round_f64(x)
}

/// Java `(int) Math.round(double)`: the `long` result narrowed by keeping the low 32 bits.
#[inline]
pub fn math_round_i32(x: f64) -> i32 {
    java_round_f64(x) as i32
}

/// Java `Math.rint(double)`: round half to even.
#[inline]
pub fn java_rint(x: f64) -> f64 {
    x.round_ties_even()
}

/// Alias of [`java_rint`].
#[inline]
pub fn math_rint(x: f64) -> f64 {
    java_rint(x)
}

/// Java `(int) double` cast: truncation toward zero, saturating, NaN -> 0 (same as Rust `as`).
#[inline]
pub fn d2i(x: f64) -> i32 {
    x as i32
}

/// Java `(long) double` cast (same as Rust `as`).
#[inline]
pub fn d2l(x: f64) -> i64 {
    x as i64
}

/// `Integer.MAX_VALUE` as double.
pub const INT_MAX_F64: f64 = i32::MAX as f64;
/// `Integer.MIN_VALUE` as double.
pub const INT_MIN_F64: f64 = i32::MIN as f64;

/// A decimal `0.d1 d2 ... dn`-style significand: `digits` (ASCII, no leading zero, no trailing
/// zero unless single digit) and the scientific exponent `exp` (value = d1.d2...dn × 10^exp).
struct Decimal {
    digits: Vec<u8>,
    exp: i32,
}

/// Splits Rust's `{:e}` output ("1.2345e-7", "5e300") into digits and exponent.
fn parse_sci(s: &str) -> Decimal {
    let (mant, exp) = s.split_once('e').expect("scientific format");
    let exp: i32 = exp.parse().expect("exponent");
    let digits: Vec<u8> = mant.bytes().filter(|b| b.is_ascii_digit()).collect();
    Decimal { digits, exp }
}

/// Java 19+ shortest-decimal selection (Raffaello Giulietti's algorithm, `DoubleToDecimal`):
/// among the decimals that round to `v`, take those of minimal length `n` (or of length <= 2
/// when `n == 1`, e.g. `4.9E-324` rather than `5E-324`), then the one closest to `v`, and on a
/// tie the one with an even last digit.
///
/// Rust's shortest round-trip output (`{:e}`) has the same length and is the closest, but it
/// rounds exact ties up; ties and the one-digit case are resolved here from the exact decimal
/// expansion of `v`.
///
/// * `shortest`: `{:e}` of `v`;
/// * `rounded(p)`: `{:.p$e}` of `v` (correctly rounded to `p + 1` significant digits);
/// * `exact()`: the exact decimal expansion (`{:.767e}`);
/// * `roundtrips(s)`: whether the decimal string `s` parses back to `v`.
fn java_decimal(
    shortest: String,
    rounded: impl FnOnce(usize) -> String,
    exact: impl FnOnce() -> String,
    roundtrips: impl Fn(&str) -> bool,
) -> Decimal {
    let d = parse_sci(&shortest);
    let n = d.digits.len();
    if n >= 2 {
        // A tie between two n-digit decimals requires v to have exactly n + 1 significant
        // digits, the last one being 5.
        let r = parse_sci(&rounded(n));
        if r.digits.last() != Some(&b'5') {
            return d;
        }
    }
    let target = n.max(2);
    let ex = parse_sci(&exact());
    let digit = |i: usize| ex.digits.get(i).map_or(0u64, |c| (c - b'0') as u64);
    let lo: u64 = (0..target).fold(0, |acc, i| acc * 10 + digit(i));
    let hi = lo + 1;
    let scale = ex.exp - target as i32 + 1; // candidate value = lo * 10^scale
    let rest = if ex.digits.len() > target { &ex.digits[target..] } else { &[][..] };
    // compare the remainder 0.rest with 1/2
    let half_cmp = match rest.first() {
        None => Ordering::Less,
        Some(&c) if c > b'5' => Ordering::Greater,
        Some(&c) if c < b'5' => Ordering::Less,
        Some(_) => {
            if rest[1..].iter().any(|&c| c != b'0') {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        }
    };
    let lo_ok = roundtrips(&format!("{lo}e{scale}"));
    let hi_ok = roundtrips(&format!("{hi}e{scale}"));
    let pick = match (lo_ok, hi_ok) {
        (true, true) => match half_cmp {
            Ordering::Less => lo,
            Ordering::Greater => hi,
            Ordering::Equal => {
                if lo % 2 == 0 {
                    lo
                } else {
                    hi
                }
            }
        },
        (true, false) => lo,
        (false, true) => hi,
        (false, false) => return d,
    };
    let mut digits: Vec<u8> = pick.to_string().into_bytes();
    let exp = scale + digits.len() as i32 - 1;
    while digits.len() > 1 && *digits.last().unwrap() == b'0' {
        digits.pop();
    }
    Decimal { digits, exp }
}

/// Formats a decimal with Java's `Double.toString` layout rules.
fn format_java(neg: bool, d: &Decimal) -> String {
    let mut s = String::with_capacity(26);
    if neg {
        s.push('-');
    }
    let digits = &d.digits;
    let e = d.exp;
    if (-3..7).contains(&e) {
        if e >= 0 {
            let int_len = (e + 1) as usize;
            for i in 0..int_len {
                s.push(*digits.get(i).unwrap_or(&b'0') as char);
            }
            s.push('.');
            if digits.len() > int_len {
                for &c in &digits[int_len..] {
                    s.push(c as char);
                }
            } else {
                s.push('0');
            }
        } else {
            s.push_str("0.");
            for _ in 0..(-e - 1) {
                s.push('0');
            }
            for &c in digits {
                s.push(c as char);
            }
        }
    } else {
        s.push(digits[0] as char);
        s.push('.');
        if digits.len() > 1 {
            for &c in &digits[1..] {
                s.push(c as char);
            }
        } else {
            s.push('0');
        }
        s.push('E');
        s.push_str(&e.to_string());
    }
    s
}

/// `Double.toString(double)` (JDK 19+ shortest-decimal algorithm and layout: plain notation
/// for `1e-3 <= |v| < 1e7` with at least one fractional digit, otherwise `d.dddE[-]n`).
pub fn double_to_string(v: f64) -> String {
    if v.is_nan() {
        return "NaN".to_string();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0.0" } else { "0.0" }.to_string();
    }
    let a = v.abs();
    let d = java_decimal(
        format!("{a:e}"),
        |p| format!("{a:.p$e}"),
        || format!("{a:.767e}"),
        |s| s.parse::<f64>() == Ok(a),
    );
    format_java(v < 0.0, &d)
}

/// `Float.toString(float)` (JDK 19+ algorithm, same layout rules as [`double_to_string`]).
pub fn float_to_string(v: f32) -> String {
    if v.is_nan() {
        return "NaN".to_string();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0.0" } else { "0.0" }.to_string();
    }
    let a = v.abs();
    let wide = a as f64; // exact
    let d = java_decimal(
        format!("{a:e}"),
        |p| format!("{wide:.p$e}"),
        || format!("{wide:.767e}"),
        |s| s.parse::<f32>() == Ok(a),
    );
    format_java(v < 0.0, &d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_matches_java() {
        assert_eq!(math_round(0.5), 1);
        assert_eq!(math_round(-0.5), 0);
        assert_eq!(math_round(1.5), 2);
        assert_eq!(math_round(-1.5), -1);
        assert_eq!(math_round(2.5), 3);
        assert_eq!(math_round(-2.5), -2);
        assert_eq!(math_round(0.49999999999999994), 0);
        assert_eq!(math_round(-0.49999999999999994), 0);
        assert_eq!(math_round(-0.5000000000000001), -1);
        assert_eq!(math_round(4503599627370497.0), 4503599627370497);
        assert_eq!(math_round(f64::NAN), 0);
        assert_eq!(math_round(f64::INFINITY), i64::MAX);
        assert_eq!(math_round(f64::NEG_INFINITY), i64::MIN);
        assert_eq!(math_round(1e300), i64::MAX);
        assert_eq!(math_round_i32(4294967296.0 + 7.0), 7);
        assert_eq!(math_round_i32(2147483648.0), i32::MIN);
        assert_eq!(java_round_f32(0.5), 1);
        assert_eq!(java_round_f32(-0.5), 0);
        assert_eq!(java_round_f32(0.49999997), 0);
        assert_eq!(java_round_f32(3e9), i32::MAX);
    }

    #[test]
    fn rint_is_half_even() {
        assert_eq!(math_rint(2.5), 2.0);
        assert_eq!(math_rint(3.5), 4.0);
        assert_eq!(math_rint(-2.5), -2.0);
    }

    #[test]
    fn casts_saturate() {
        assert_eq!(d2i(1e20), i32::MAX);
        assert_eq!(d2i(-1e20), i32::MIN);
        assert_eq!(d2i(f64::NAN), 0);
        assert_eq!(d2i(-2.7), -2);
    }

    #[test]
    fn to_string_layout() {
        assert_eq!(double_to_string(1.0), "1.0");
        assert_eq!(double_to_string(-0.0), "-0.0");
        assert_eq!(double_to_string(100.0), "100.0");
        assert_eq!(double_to_string(0.001), "0.001");
        assert_eq!(double_to_string(0.0001), "1.0E-4");
        assert_eq!(double_to_string(1e7), "1.0E7");
        assert_eq!(double_to_string(9999999.0), "9999999.0");
        assert_eq!(double_to_string(1.0e23), "1.0E23");
        assert_eq!(double_to_string(2e23), "2.0E23");
        assert_eq!(double_to_string(f64::MIN_POSITIVE * 0.0 + 5e-324), "4.9E-324");
        assert_eq!(double_to_string(f64::MAX), "1.7976931348623157E308");
        assert_eq!(double_to_string(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(float_to_string(f32::from_bits(1)), "1.4E-45");
        assert_eq!(float_to_string(0.1), "0.1");
        assert_eq!(float_to_string(1e10), "1.0E10");
    }
}
