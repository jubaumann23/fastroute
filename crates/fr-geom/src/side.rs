//! Port of `Side.java`.

/// Enum Side with the three values ON_THE_LEFT, ON_THE_RIGHT, COLLINEAR.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    OnTheLeft,
    OnTheRight,
    Collinear,
}

impl Side {
    /// Returns ON_THE_LEFT if value > 0, ON_THE_RIGHT if value < 0, and COLLINEAR if value == 0.
    /// (The Java doc comment has the signs swapped; this follows the Java code.)
    #[inline]
    pub fn of(value: f64) -> Side {
        if value > 0.0 {
            Side::OnTheLeft
        } else if value < 0.0 {
            Side::OnTheRight
        } else {
            Side::Collinear
        }
    }

    /// Returns the opposite side of this side.
    #[inline]
    pub fn negate(self) -> Side {
        match self {
            Side::OnTheLeft => Side::OnTheRight,
            Side::OnTheRight => Side::OnTheLeft,
            Side::Collinear => Side::Collinear,
        }
    }
}
