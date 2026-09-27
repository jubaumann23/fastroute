//! Port of `board/model/structure/Unit.java`.

/// The user units mil, inch, millimeter or micrometer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Unit {
    Mil,
    Inch,
    Mm,
    Um,
}

impl Unit {
    /// Size of the unit in micrometers.
    pub fn micrometers(self) -> f64 {
        match self {
            Unit::Mil => 25.4,
            Unit::Inch => 25_400.0,
            Unit::Mm => 1000.0,
            Unit::Um => 1.0,
        }
    }

    /// Scales `value` from `from_unit` to `to_unit` (`value * from / to`, Java order).
    pub fn scale(value: f64, from_unit: Unit, to_unit: Unit) -> f64 {
        value * from_unit.micrometers() / to_unit.micrometers()
    }

    /// Java `Unit.fromString`: `valueOf(string.toUpperCase())`, `None` if unknown.
    pub fn from_string(string: &str) -> Option<Unit> {
        match string.to_uppercase().as_str() {
            "MIL" => Some(Unit::Mil),
            "INCH" => Some(Unit::Inch),
            "MM" => Some(Unit::Mm),
            "UM" => Some(Unit::Um),
            _ => None,
        }
    }
}

impl std::fmt::Display for Unit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Unit::Mil => "mil",
            Unit::Inch => "inch",
            Unit::Mm => "mm",
            Unit::Um => "um",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_and_parse() {
        assert_eq!(Unit::scale(1.0, Unit::Inch, Unit::Mil), 25_400.0 / 25.4);
        assert_eq!(Unit::scale(2.0, Unit::Mm, Unit::Um), 2000.0);
        assert_eq!(Unit::from_string("Mil"), Some(Unit::Mil));
        assert_eq!(Unit::from_string("um"), Some(Unit::Um));
        assert_eq!(Unit::from_string("cm"), None);
        assert_eq!(Unit::Inch.to_string(), "inch");
    }
}
