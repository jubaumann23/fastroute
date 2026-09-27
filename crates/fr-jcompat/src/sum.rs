//! Compensated (Kahan) summation exactly as `DoubleStream.sum()` does it (JDK 18+,
//! `Collectors.sumWithCompensation` + `Collectors.computeFinalSum`).
//!
//! The same arithmetic backs `DoubleStream.average()`, `DoubleSummaryStatistics.getSum()/
//! getAverage()`, `Collectors.summingDouble` and `Collectors.averagingDouble` (sequential streams).

/// Accumulator equivalent to the JDK's `double[3] {high-order sum, compensation, simple sum}`
/// (plus the element count used by the averaging variants).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CompensatedSum {
    sum: f64,
    compensation: f64,
    simple_sum: f64,
    count: u64,
}

impl CompensatedSum {
    pub fn new() -> Self {
        Self::default()
    }

    /// `Collectors.sumWithCompensation(intermediateSum, value)` followed by `simpleSum += value`.
    #[inline]
    pub fn add(&mut self, value: f64) {
        let tmp = value - self.compensation;
        let sum = self.sum;
        let velvel = sum + tmp; // Little wolf of rounding error
        self.compensation = (velvel - sum) - tmp;
        self.sum = velvel;
        self.simple_sum += value;
        self.count += 1;
    }

    /// `Collectors.computeFinalSum(summands)`.
    #[inline]
    pub fn sum(&self) -> f64 {
        // Final sum with better error bounds: subtract the second summand as it is negated.
        let tmp = self.sum - self.compensation;
        if tmp.is_nan() && self.simple_sum.is_infinite() {
            // An infinity of one sign was summed; the compensated sum became NaN.
            self.simple_sum
        } else {
            tmp
        }
    }

    /// Number of values added.
    pub fn count(&self) -> u64 {
        self.count
    }

    /// `DoubleStream.average()`: `None` for an empty stream.
    pub fn average(&self) -> Option<f64> {
        if self.count > 0 {
            Some(self.sum() / self.count as f64)
        } else {
            None
        }
    }

    /// `DoubleSummaryStatistics.getAverage()` (0.0 when empty).
    pub fn average_or_zero(&self) -> f64 {
        self.average().unwrap_or(0.0)
    }
}

impl Extend<f64> for CompensatedSum {
    fn extend<T: IntoIterator<Item = f64>>(&mut self, iter: T) {
        for v in iter {
            self.add(v);
        }
    }
}

impl FromIterator<f64> for CompensatedSum {
    fn from_iter<T: IntoIterator<Item = f64>>(iter: T) -> Self {
        let mut s = CompensatedSum::new();
        s.extend(iter);
        s
    }
}

/// `DoubleStream.sum()` over the values in encounter order.
pub fn compensated_sum<I: IntoIterator<Item = f64>>(values: I) -> f64 {
    values.into_iter().collect::<CompensatedSum>().sum()
}

/// `DoubleStream.average()` over the values in encounter order.
pub fn compensated_average<I: IntoIterator<Item = f64>>(values: I) -> Option<f64> {
    values.into_iter().collect::<CompensatedSum>().average()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beats_naive_sum() {
        let v = [0.1; 10];
        assert_eq!(v.iter().sum::<f64>(), 0.9999999999999999);
        assert_eq!(compensated_sum(v), 1.0);
        assert_eq!(compensated_sum([]), 0.0);
        assert_eq!(compensated_sum([f64::INFINITY, 1.0]), f64::INFINITY);
        assert!(compensated_sum([f64::INFINITY, f64::NEG_INFINITY]).is_nan());
        assert_eq!(compensated_average([1.0, 2.0]), Some(1.5));
        assert_eq!(compensated_average([]), None);
    }
}
