//! `java.util.Random` (48-bit linear congruential generator) and `Collections.shuffle`.

/// Port of `java.util.Random`.
///
/// Not emulated: `nextGaussian` (uses `StrictMath.log/sqrt`), streams, `nextLong(bound)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JavaRandom {
    seed: i64,
}

impl JavaRandom {
    const MULTIPLIER: i64 = 0x5_DEEC_E66D;
    const ADDEND: i64 = 0xB;
    const MASK: i64 = (1i64 << 48) - 1;
    /// `0x1.0p-53`
    const DOUBLE_UNIT: f64 = 1.0 / (1u64 << 53) as f64;
    /// `0x1.0p-24f`
    const FLOAT_UNIT: f32 = 1.0 / (1u32 << 24) as f32;

    /// `new Random(seed)`.
    pub fn new(seed: i64) -> Self {
        JavaRandom { seed: Self::initial_scramble(seed) }
    }

    #[inline]
    fn initial_scramble(seed: i64) -> i64 {
        (seed ^ Self::MULTIPLIER) & Self::MASK
    }

    /// `Random.setSeed(long)` (also clears nothing else: `nextGaussian` is not emulated).
    pub fn set_seed(&mut self, seed: i64) {
        self.seed = Self::initial_scramble(seed);
    }

    /// The internal 48-bit state (after scrambling), for debugging/snapshots.
    pub fn state(&self) -> i64 {
        self.seed
    }

    /// `protected int next(int bits)`, `1 <= bits <= 32`.
    #[inline]
    pub fn next(&mut self, bits: u32) -> i32 {
        debug_assert!((1..=32).contains(&bits));
        self.seed = self.seed.wrapping_mul(Self::MULTIPLIER).wrapping_add(Self::ADDEND) & Self::MASK;
        // (int) (nextseed >>> (48 - bits))
        ((self.seed as u64) >> (48 - bits)) as i32
    }

    /// `Random.nextInt()`.
    #[inline]
    pub fn next_int(&mut self) -> i32 {
        self.next(32)
    }

    /// `Random.nextInt(int bound)`. Panics if `bound <= 0` (Java throws `IllegalArgumentException`).
    pub fn next_int_bound(&mut self, bound: i32) -> i32 {
        assert!(bound > 0, "bound must be positive");
        let mut r = self.next(31);
        let m = bound - 1;
        if (bound & m) == 0 {
            // power of two
            r = ((bound as i64 * r as i64) >> 31) as i32;
        } else {
            let mut u = r;
            loop {
                r = u % bound;
                // Java relies on int overflow to reject values from the incomplete last range.
                if u.wrapping_sub(r).wrapping_add(m) >= 0 {
                    break;
                }
                u = self.next(31);
            }
        }
        r
    }

    /// `Random.nextInt(int origin, int bound)` (JDK 17+ `RandomSupport.boundedNextInt`).
    /// Panics if `origin >= bound`.
    pub fn next_int_range(&mut self, origin: i32, bound: i32) -> i32 {
        assert!(origin < bound, "bound must be greater than origin");
        let mut r = self.next_int();
        let n = bound.wrapping_sub(origin);
        let m = n.wrapping_sub(1);
        if (n & m) == 0 {
            r = (r & m).wrapping_add(origin);
        } else if n > 0 {
            let mut u = ((r as u32) >> 1) as i32;
            loop {
                r = u % n;
                if u.wrapping_add(m).wrapping_sub(r) >= 0 {
                    break;
                }
                u = ((self.next_int() as u32) >> 1) as i32;
            }
            r = r.wrapping_add(origin);
        } else {
            while r < origin || r >= bound {
                r = self.next_int();
            }
        }
        r
    }

    /// `Random.nextLong()`.
    #[inline]
    pub fn next_long(&mut self) -> i64 {
        let hi = self.next(32) as i64;
        let lo = self.next(32) as i64;
        (hi << 32).wrapping_add(lo)
    }

    /// `Random.nextBoolean()`.
    #[inline]
    pub fn next_boolean(&mut self) -> bool {
        self.next(1) != 0
    }

    /// `Random.nextDouble()`.
    #[inline]
    pub fn next_double(&mut self) -> f64 {
        let hi = self.next(26) as i64;
        let lo = self.next(27) as i64;
        ((hi << 27) + lo) as f64 * Self::DOUBLE_UNIT
    }

    /// `Random.nextFloat()`.
    #[inline]
    pub fn next_float(&mut self) -> f32 {
        self.next(24) as f32 * Self::FLOAT_UNIT
    }
}

/// `Collections.shuffle(list, rnd)` for a `RandomAccess` list (or any list: the non-random-access
/// path copies to an array and performs the same swaps).
pub fn shuffle<T>(list: &mut [T], rnd: &mut JavaRandom) {
    let n = list.len();
    if n < 2 {
        return;
    }
    for i in (1..n).rev() {
        let j = rnd.next_int_bound((i + 1) as i32) as usize;
        list.swap(i, j);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_values() {
        assert_eq!(JavaRandom::new(42).next_int(), -1170105035);
        let mut r = JavaRandom::new(42);
        let v: Vec<i32> = (0..10).map(|_| r.next_int_bound(10)).collect();
        // verified with JDK 25
        assert_eq!(v, vec![0, 3, 8, 4, 0, 5, 5, 8, 9, 3]);
        // values from the original fr-geom java_compat tests (OpenJDK 17)
        let mut r = JavaRandom::new(99);
        let v: Vec<i32> = (0..8).map(|i| r.next_int_bound(i + 3)).collect();
        assert_eq!(v, vec![1, 1, 4, 3, 1, 3, 7, 6]);
        assert_eq!(JavaRandom::new(42).next_int_bound(100), 30);
        assert_eq!(JavaRandom::new(1).next_int_bound(8), 5);
    }

    #[test]
    fn set_seed_resets() {
        let mut a = JavaRandom::new(5);
        a.next_long();
        a.set_seed(9);
        let mut b = JavaRandom::new(9);
        assert_eq!(a.next_int(), b.next_int());
    }

    #[test]
    fn shuffle_small() {
        let mut v: Vec<i32> = vec![];
        shuffle(&mut v, &mut JavaRandom::new(1));
        let mut v = vec![7];
        shuffle(&mut v, &mut JavaRandom::new(1));
        assert_eq!(v, vec![7]);
    }
}
