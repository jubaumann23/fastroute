//! Port of `FortyfiveDegreeDirection.java`.

use crate::int_direction::IntDirection;

/// Enum for the eight 45-degree directions starting from right in counterclock sense to down45.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FortyfiveDegreeDirection {
    Right,
    Right45,
    Up,
    Up45,
    Left,
    Left45,
    Down,
    Down45,
}

impl FortyfiveDegreeDirection {
    /// `FortyfiveDegreeDirection.values()`.
    pub const VALUES: [FortyfiveDegreeDirection; 8] = [
        FortyfiveDegreeDirection::Right,
        FortyfiveDegreeDirection::Right45,
        FortyfiveDegreeDirection::Up,
        FortyfiveDegreeDirection::Up45,
        FortyfiveDegreeDirection::Left,
        FortyfiveDegreeDirection::Left45,
        FortyfiveDegreeDirection::Down,
        FortyfiveDegreeDirection::Down45,
    ];

    pub fn get_direction(self) -> IntDirection {
        match self {
            FortyfiveDegreeDirection::Right => IntDirection::RIGHT,
            FortyfiveDegreeDirection::Right45 => IntDirection::RIGHT45,
            FortyfiveDegreeDirection::Up => IntDirection::UP,
            FortyfiveDegreeDirection::Up45 => IntDirection::UP45,
            FortyfiveDegreeDirection::Left => IntDirection::LEFT,
            FortyfiveDegreeDirection::Left45 => IntDirection::LEFT45,
            FortyfiveDegreeDirection::Down => IntDirection::DOWN,
            FortyfiveDegreeDirection::Down45 => IntDirection::DOWN45,
        }
    }

    /// Java `ordinal()`.
    pub fn ordinal(self) -> i32 {
        self as i32
    }
}
