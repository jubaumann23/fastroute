//! Java integer operator semantics and boxed-type `hashCode` functions.

/// `a << n` for `int` (shift count masked by 31, wrapping).
#[inline]
pub fn java_shl(a: i32, n: i32) -> i32 {
    a.wrapping_shl((n & 31) as u32)
}

/// `a >> n` for `int` (arithmetic, count masked by 31).
#[inline]
pub fn java_shr(a: i32, n: i32) -> i32 {
    a >> (n & 31)
}

/// `a >>> n` for `int` (logical, count masked by 31).
#[inline]
pub fn java_ushr(a: i32, n: i32) -> i32 {
    ((a as u32) >> (n & 31)) as i32
}

/// `a << n` for `long` (count masked by 63).
#[inline]
pub fn java_shl_long(a: i64, n: i32) -> i64 {
    a.wrapping_shl((n & 63) as u32)
}

/// `a >> n` for `long` (count masked by 63).
#[inline]
pub fn java_shr_long(a: i64, n: i32) -> i64 {
    a >> (n & 63)
}

/// `a >>> n` for `long` (count masked by 63).
#[inline]
pub fn java_ushr_long(a: i64, n: i32) -> i64 {
    ((a as u64) >> (n & 63)) as i64
}

/// `Integer.hashCode(v)`.
#[inline]
pub fn int_hash_code(v: i32) -> i32 {
    v
}

/// `Long.hashCode(v)` = `(int) (v ^ (v >>> 32))`.
#[inline]
pub fn long_hash_code(v: i64) -> i32 {
    (v ^ ((v as u64) >> 32) as i64) as i32
}

/// `Double.doubleToLongBits(v)` (NaN canonicalised to `0x7ff8000000000000`).
#[inline]
pub fn double_to_long_bits(v: f64) -> i64 {
    if v.is_nan() {
        0x7ff8_0000_0000_0000
    } else {
        v.to_bits() as i64
    }
}

/// `Float.floatToIntBits(v)` (NaN canonicalised to `0x7fc00000`).
#[inline]
pub fn float_to_int_bits(v: f32) -> i32 {
    if v.is_nan() {
        0x7fc0_0000
    } else {
        v.to_bits() as i32
    }
}

/// `Double.hashCode(v)`.
#[inline]
pub fn double_hash_code(v: f64) -> i32 {
    long_hash_code(double_to_long_bits(v))
}

/// `Float.hashCode(v)`.
#[inline]
pub fn float_hash_code(v: f32) -> i32 {
    float_to_int_bits(v)
}

/// `Boolean.hashCode(v)`.
#[inline]
pub fn boolean_hash_code(v: bool) -> i32 {
    if v {
        1231
    } else {
        1237
    }
}

/// `Arrays.hashCode(int[])` (`null` not representable).
pub fn arrays_hash_code_i32(a: &[i32]) -> i32 {
    a.iter().fold(1i32, |h, &e| h.wrapping_mul(31).wrapping_add(e))
}

/// `String.hashCode()` (over UTF-16 code units).
pub fn string_hash_code(s: &str) -> i32 {
    s.encode_utf16().fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shifts() {
        assert_eq!(java_shl(1, 33), 2);
        assert_eq!(java_shl(1, 31), i32::MIN);
        assert_eq!(java_shl(1, -1), i32::MIN);
        assert_eq!(java_shr(-8, 1), -4);
        assert_eq!(java_shr(-8, 33), -4);
        assert_eq!(java_ushr(-1, 28), 15);
        assert_eq!(java_ushr(-1, 32), -1);
        assert_eq!(java_ushr_long(-1, 60), 15);
        assert_eq!(java_shl_long(1, 64), 1);
        assert_eq!(java_shr_long(-16, 66), -4);
    }

    #[test]
    fn hash_codes() {
        assert_eq!(int_hash_code(-7), -7);
        assert_eq!(long_hash_code(0x1_0000_0001), 0);
        assert_eq!(long_hash_code(-1), 0);
        assert_eq!(long_hash_code(5), 5);
        // Double.hashCode(1.0) == 1072693248, Double.hashCode(0.0) == 0, (-0.0) == -2147483648
        assert_eq!(double_hash_code(1.0), 1072693248);
        assert_eq!(double_hash_code(0.0), 0);
        assert_eq!(double_hash_code(-0.0), i32::MIN);
        assert_eq!(double_hash_code(f64::NAN), 2146959360);
        assert_eq!(float_hash_code(1.0), 1065353216);
        assert_eq!(boolean_hash_code(true), 1231);
        assert_eq!(string_hash_code("hello"), 99162322);
        assert_eq!(arrays_hash_code_i32(&[1, 2, 3]), 30817);
    }
}
