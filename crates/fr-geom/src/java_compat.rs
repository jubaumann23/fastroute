//! Helpers reproducing Java numeric semantics that the Freerouting geometry code relies on.
//!
//! Not a port of a single Java file: it collects the JDK behaviours (`Math.round`, `Math.rint`,
//! `java.util.Random`, `BigInteger.hashCode/intValue/doubleValue`) that the planar package uses.

use num_bigint::{BigInt, Sign};
use num_traits::{ToPrimitive, Zero};

/// Java `Math.round(double)`: nearest `long`, ties toward positive infinity, NaN -> 0,
/// saturating at `Long.MIN_VALUE` / `Long.MAX_VALUE`.
///
/// Note: this is the Java 7+ semantics (`Math.round(0.49999999999999994) == 0`), which differs
/// from the naive `floor(x + 0.5)` only for that kind of edge value.
#[inline]
pub fn math_round(x: f64) -> i64 {
    if x.is_nan() {
        return 0;
    }
    let f = x.floor();
    // x - floor(x) is exact for all finite doubles relevant here (see unit tests).
    let r = if x - f >= 0.5 { f + 1.0 } else { f };
    r as i64 // saturating cast, same as Java's clamping
}

/// Java `(int) Math.round(double)`: the `long` result is narrowed by keeping the low 32 bits.
#[inline]
pub fn math_round_i32(x: f64) -> i32 {
    math_round(x) as i32
}

/// Java `Math.rint(double)`: round half to even.
#[inline]
pub fn math_rint(x: f64) -> f64 {
    x.round_ties_even()
}

/// Java `(int) double` cast: truncation toward zero, saturating, NaN -> 0 (identical to Rust `as`).
#[inline]
pub fn d2i(x: f64) -> i32 {
    x as i32
}

/// `Integer.MAX_VALUE` as double.
pub const INT_MAX_F64: f64 = i32::MAX as f64;
/// `Integer.MIN_VALUE` as double.
pub const INT_MIN_F64: f64 = i32::MIN as f64;

/// Java `Math.acos` / `StrictMath.acos`: port of fdlibm `e_acos.c`, bit-identical to Java
/// (the system libm used by `f64::acos` may differ in the last ulp).
#[allow(clippy::eq_op)] // fdlibm computes NaN as (x - x) / (x - x)
pub fn strict_acos(x: f64) -> f64 {
    const ONE: f64 = 1.0;
    let pi = f64::from_bits(0x400921FB54442D18);
    let pio2_hi = f64::from_bits(0x3FF921FB54442D18);
    let pio2_lo = f64::from_bits(0x3C91A62633145C07);
    let ps0 = f64::from_bits(0x3FC5555555555555);
    let ps1 = f64::from_bits(0xBFD4D61203EB6F7D);
    let ps2 = f64::from_bits(0x3FC9C1550E884455);
    let ps3 = f64::from_bits(0xBFA48228B5688F3B);
    let ps4 = f64::from_bits(0x3F49EFE07501B288);
    let ps5 = f64::from_bits(0x3F023DE10DFDF709);
    let qs1 = f64::from_bits(0xC0033A271C8A2D4B);
    let qs2 = f64::from_bits(0x40002AE59C598AC8);
    let qs3 = f64::from_bits(0xBFE6066C1B8D0159);
    let qs4 = f64::from_bits(0x3FB3B8C5B12E9282);

    let bits = x.to_bits();
    let hx = (bits >> 32) as i32;
    let lx = bits as u32;
    let ix = hx & 0x7fffffff;
    if ix >= 0x3ff00000 {
        // |x| >= 1
        if ((ix - 0x3ff00000) as u32 | lx) == 0 {
            // |x| == 1
            if hx > 0 {
                return 0.0;
            } else {
                return pi + 2.0 * pio2_lo;
            }
        }
        return (x - x) / (x - x); // acos(|x| > 1) is NaN
    }
    if ix < 0x3fe00000 {
        // |x| < 0.5
        if ix <= 0x3c600000 {
            return pio2_hi + pio2_lo; // |x| < 2**-57
        }
        let z = x * x;
        let p = z * (ps0 + z * (ps1 + z * (ps2 + z * (ps3 + z * (ps4 + z * ps5)))));
        let q = ONE + z * (qs1 + z * (qs2 + z * (qs3 + z * qs4)));
        let r = p / q;
        pio2_hi - (x - (pio2_lo - x * r))
    } else if hx < 0 {
        // x < -0.5
        let z = (ONE + x) * 0.5;
        let p = z * (ps0 + z * (ps1 + z * (ps2 + z * (ps3 + z * (ps4 + z * ps5)))));
        let q = ONE + z * (qs1 + z * (qs2 + z * (qs3 + z * qs4)));
        let s = z.sqrt();
        let r = p / q;
        let w = r * s - pio2_lo;
        pi - 2.0 * (s + w)
    } else {
        // x > 0.5
        let z = (ONE - x) * 0.5;
        let s = z.sqrt();
        let df = f64::from_bits(s.to_bits() & 0xFFFF_FFFF_0000_0000);
        let c = (z - df * df) / (s + df);
        let p = z * (ps0 + z * (ps1 + z * (ps2 + z * (ps3 + z * (ps4 + z * ps5)))));
        let q = ONE + z * (qs1 + z * (qs2 + z * (qs3 + z * qs4)));
        let r = p / q;
        let w = r * s + c;
        2.0 * (df + w)
    }
}

/// Port of `java.util.Random` (48-bit LCG), only the parts used by the geometry package.
#[derive(Clone, Debug)]
pub struct JavaRandom {
    seed: i64,
}

impl JavaRandom {
    const MULTIPLIER: i64 = 0x5DEE_CE66D;
    const ADDEND: i64 = 0xB;
    const MASK: i64 = (1i64 << 48) - 1;

    pub fn new(seed: i64) -> Self {
        JavaRandom {
            seed: (seed ^ Self::MULTIPLIER) & Self::MASK,
        }
    }

    pub fn set_seed(&mut self, seed: i64) {
        self.seed = (seed ^ Self::MULTIPLIER) & Self::MASK;
    }

    fn next(&mut self, bits: u32) -> i32 {
        self.seed = (self
            .seed
            .wrapping_mul(Self::MULTIPLIER)
            .wrapping_add(Self::ADDEND))
            & Self::MASK;
        ((self.seed as u64) >> (48 - bits)) as i32
    }

    /// `Random.nextInt(int bound)`. Panics if `bound <= 0` (Java throws IllegalArgumentException).
    pub fn next_int(&mut self, bound: i32) -> i32 {
        assert!(bound > 0, "bound must be positive");
        let mut r = self.next(31);
        let m = bound - 1;
        if (bound & m) == 0 {
            // bound is a power of 2
            r = ((bound as i64 * r as i64) >> 31) as i32;
        } else {
            let mut u = r;
            loop {
                r = u % bound;
                // Java relies on int overflow here to reject values from the incomplete last range.
                if u.wrapping_sub(r).wrapping_add(m) >= 0 {
                    break;
                }
                u = self.next(31);
            }
        }
        r
    }
}

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
    let low = if digits.is_empty() {
        0u32
    } else {
        digits.swap_remove(0)
    };
    match v.sign() {
        Sign::Minus => (low as i32).wrapping_neg(),
        _ => low as i32,
    }
}

/// Java `BigInteger.doubleValue()` (correctly rounded, half-even; +-Infinity on overflow).
pub fn bigint_double_value(v: &BigInt) -> f64 {
    if v.is_zero() {
        return 0.0;
    }
    v.to_f64().unwrap_or(if v.sign() == Sign::Minus {
        f64::NEG_INFINITY
    } else {
        f64::INFINITY
    })
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
    fn math_round_matches_java() {
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
        // (int) Math.round(x) keeps the low 32 bits of the long
        assert_eq!(math_round_i32(4294967296.0 + 7.0), 7);
        assert_eq!(math_round_i32(2147483648.0), i32::MIN);
    }

    #[test]
    fn rint_is_half_even() {
        assert_eq!(math_rint(2.5), 2.0);
        assert_eq!(math_rint(3.5), 4.0);
        assert_eq!(math_rint(-2.5), -2.0);
    }

    #[test]
    fn double_to_int_cast_saturates() {
        assert_eq!(d2i(1e20), i32::MAX);
        assert_eq!(d2i(-1e20), i32::MIN);
        assert_eq!(d2i(f64::NAN), 0);
        assert_eq!(d2i(-2.7), -2);
    }

    #[test]
    fn java_random_sequence() {
        // Reference values obtained from OpenJDK 17:
        // r = new Random(99); r.nextInt(i + 3) for i in 0..8
        let mut r = JavaRandom::new(99);
        let v: Vec<i32> = (0..8).map(|i| r.next_int(i + 3)).collect();
        assert_eq!(v, vec![1, 1, 4, 3, 1, 3, 7, 6]);
        assert_eq!(JavaRandom::new(42).next_int(100), 30);
        assert_eq!(JavaRandom::new(0).next_int(10), 0);
        assert_eq!(JavaRandom::new(1).next_int(8), 5); // power-of-two branch
        assert_eq!(JavaRandom::new(1).next_int(10), 5);
    }

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
        assert_eq!(
            bigint_double_value(&big(9007199254740993)),
            9007199254740992.0
        );
    }
}
