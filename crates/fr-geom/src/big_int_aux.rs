//! Port of `app.freerouting.datastructures.BigIntAux` (only what the geometry package needs).

use num_bigint::BigInt;

/// Calculates the determinant of the vectors (x1, y1) and (x2, y2).
#[inline]
pub fn determinant(x1: &BigInt, y1: &BigInt, x2: &BigInt, y2: &BigInt) -> BigInt {
    x1 * y2 - x2 * y1
}

/// Auxiliary function to implement addition and translation in RationalVector and RationalPoint.
/// Coordinates are `[x, y, z]` meaning `(x/z, y/z)`.
pub fn add_rational_coordinates(first: [&BigInt; 3], second: [&BigInt; 3]) -> [BigInt; 3] {
    if first[2] == second[2] {
        // both rational numbers have the same denominator
        [first[0] + second[0], first[1] + second[1], first[2].clone()]
    } else {
        // multiply both denominators for the new denominator
        let z = first[2] * second[2];
        let x = first[0] * second[2] + second[0] * first[2];
        let y = first[1] * second[2] + second[1] * first[2];
        [x, y, z]
    }
}

/// Calculate GCD of a and b interpreted as unsigned integers (Java `BigIntAux.binaryGcd`).
/// The result is returned as the raw 32 bit pattern (callers compare it as a Java `int`).
pub fn binary_gcd(a: i32, b: i32) -> i32 {
    let mut a = a as u32;
    let mut b = b as u32;
    if b == 0 {
        return a as i32;
    }
    if a == 0 {
        return b as i32;
    }
    let za = a.trailing_zeros();
    let zb = b.trailing_zeros();
    let t = za.min(zb);
    a >>= za;
    b >>= zb;
    while a != b {
        if a > b {
            a -= b;
            a >>= a.trailing_zeros();
        } else {
            b -= a;
            b >>= b.trailing_zeros();
        }
    }
    (a << t) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_gcd_values() {
        assert_eq!(binary_gcd(0, 0), 0);
        assert_eq!(binary_gcd(0, 7), 7);
        assert_eq!(binary_gcd(12, 18), 6);
        assert_eq!(binary_gcd(1 << 20, 3 << 10), 1 << 10);
        // Math.abs(Integer.MIN_VALUE) == Integer.MIN_VALUE is treated as unsigned 2^31
        assert_eq!(binary_gcd(i32::MIN, 0), i32::MIN);
        assert_eq!(binary_gcd(i32::MIN, 6), 2);
    }
}
