//! Port of `app.freerouting.datastructures.Signum`.

/// Implements the mathematical signum function.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Signum {
    Positive,
    Negative,
    Zero,
}

impl Signum {
    /// Returns the signum of value.
    #[inline]
    pub fn of(value: f64) -> Signum {
        if value > 0.0 {
            Signum::Positive
        } else if value < 0.0 {
            Signum::Negative
        } else {
            Signum::Zero
        }
    }

    /// Returns the signum of value as an int. Values are +1, 0 and -1.
    #[inline]
    pub fn as_int(value: f64) -> i32 {
        if value > 0.0 {
            1
        } else if value < 0.0 {
            -1
        } else {
            0
        }
    }

    /// Returns the opposite Signum of this Signum.
    #[inline]
    pub fn negate(self) -> Signum {
        match self {
            Signum::Positive => Signum::Negative,
            Signum::Negative => Signum::Positive,
            Signum::Zero => Signum::Zero,
        }
    }
}
