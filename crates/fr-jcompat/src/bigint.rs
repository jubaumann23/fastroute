//! `java.math.BigInteger` helpers on top of `num-bigint` (feature `bigint`).

use num_bigint::{BigInt, Sign};
use num_traits::{ToPrimitive, Zero};

/// Java `BigInteger.hashCode()`.
pub fn bigint_hash_code(v: &BigInt) -> i32 {
    let mut digits = v.magnitude().to_u32_digits(); // little endian
    digits.reverse();
    let mut h: i32 = 0;
    for d in digits {
        h = h.wrapping_mul(31).wrapping_add(d as i32);
    }
    match v.sign() {
        Sign::Minus => h.wrapping_neg(),
        Sign::NoSign => 0,
        Sign::Plus => h,
    }
}

/// Java `BigInteger.intValue()`: the low-order 32 bits in two's complement.
pub fn bigint_int_value(v: &BigInt) -> i32 {
    if let Some(i) = v.to_i32() {
        return i;
    }
    let mut digits = v.magnitude().to_u32_digits();
    let low = if digits.is_empty() { 0u32 } else { digits.swap_remove(0) };
    match v.sign() {
        Sign::Minus => (low as i32).wrapping_neg(),
        _ => low as i32,
    }
}

/// Java `BigInteger.longValue()`: the low-order 64 bits in two's complement.
pub fn bigint_long_value(v: &BigInt) -> i64 {
    if let Some(i) = v.to_i64() {
        return i;
    }
    let digits = v.magnitude().to_u64_digits();
    let low = digits.first().copied().unwrap_or(0);
    match v.sign() {
        Sign::Minus => (low as i64).wrapping_neg(),
        _ => low as i64,
    }
}

/// Java `BigInteger.doubleValue()` (correctly rounded, half-even; +-Infinity on overflow).
pub fn bigint_double_value(v: &BigInt) -> f64 {
    if v.is_zero() {
        return 0.0;
    }
    v.to_f64().unwrap_or(if v.sign() == Sign::Minus { f64::NEG_INFINITY } else { f64::INFINITY })
}

/// `BigInteger.valueOf(long)`.
#[inline]
pub fn big(v: i64) -> BigInt {
    BigInt::from(v)
}

/// Java `BigInteger.signum()`.
#[inline]
pub fn bigint_signum(v: &BigInt) -> i32 {
    match v.sign() {
        Sign::Minus => -1,
        Sign::NoSign => 0,
        Sign::Plus => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bigint_helpers() {
        assert_eq!(bigint_hash_code(&big(0)), 0);
        assert_eq!(bigint_hash_code(&big(5)), 5);
        assert_eq!(bigint_hash_code(&big(-5)), -5);
        // 2^32 + 3: mag = [1, 3] -> 31*1 + 3 = 34
        assert_eq!(bigint_hash_code(&big((1i64 << 32) + 3)), 34);
        assert_eq!(bigint_int_value(&big((1i64 << 32) + 3)), 3);
        assert_eq!(bigint_int_value(&big(-((1i64 << 32) + 3))), -3);
        assert_eq!(bigint_int_value(&big(1i64 << 31)), i32::MIN);
        assert_eq!(bigint_long_value(&(big(1) << 64usize)), 0);
        assert_eq!(bigint_long_value(&(-(big(1) << 64usize) - big(5))), -5);
        assert_eq!(bigint_long_value(&(big(1) << 63usize)), i64::MIN);
        assert_eq!(bigint_double_value(&big(9007199254740993)), 9007199254740992.0);
        assert_eq!(bigint_signum(&big(-3)), -1);
    }
}
