//! Port of `Limits.java`: numerical limits and values used by planar geometry.

use num_bigint::BigInt;

/// An upper bound (2^25) so that the product of two integers with absolute value at most
/// CRIT_INT is contained in the mantissa of a double with some space left for addition.
pub const CRIT_INT: i32 = 33_554_432;

/// The biggest double value (2^53), so that all integers smaller than this value are
/// represented exactly as double values.
pub const CRIT_DOUBLE: f64 = 9_007_199_254_740_992.0;

/// `Math.sqrt(2)`.
pub const SQRT2: f64 = std::f64::consts::SQRT_2;

/// `Limits.CRIT_INT_BIG`.
#[inline]
pub fn crit_int_big() -> BigInt {
    BigInt::from(CRIT_INT)
}
