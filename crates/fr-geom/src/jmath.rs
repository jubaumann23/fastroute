//! Single routing point for all transcendental / root functions used by the geometry code
//! (Java `Math.sin/cos/acos/atan2/sqrt/pow`).
//!
//! Implemented with the `libm` crate (a port of fdlibm/musl), i.e. Java `StrictMath` semantics,
//! so results are identical on every platform. Note that HotSpot's `Math.sin/cos` intrinsics may
//! differ from `StrictMath` in the last ulp; swap the implementation here if a different target
//! is wanted. `sqrt` is correctly rounded in every implementation (IEEE 754), it is routed here
//! only for uniformity.
//!
//! Call sites: `sin`/`cos` in `Direction::get_instance_approx`, `FloatPoint::rotate`,
//! `Circle::bounding_tile_max`; `acos` in `Vector::angle_approx_to`; `sqrt` everywhere a
//! length or distance is computed (FloatPoint, FloatLine, IntPoint, Line, IntBox, Circle,
//! Direction via FloatPoint::size).

/// Java `Math.sin` (StrictMath / fdlibm semantics).
#[inline]
pub fn sin(x: f64) -> f64 {
    libm::sin(x)
}

/// Java `Math.cos` (StrictMath / fdlibm semantics).
#[inline]
pub fn cos(x: f64) -> f64 {
    libm::cos(x)
}

/// Java `Math.acos` (StrictMath / fdlibm semantics).
#[inline]
pub fn acos(x: f64) -> f64 {
    libm::acos(x)
}

/// Java `Math.atan2` (StrictMath / fdlibm semantics). Not used by the geometry package itself;
/// provided for the ports that build on it.
#[inline]
pub fn atan2(y: f64, x: f64) -> f64 {
    libm::atan2(y, x)
}

/// Java `Math.pow` (StrictMath / fdlibm semantics). Not used by the geometry package itself.
#[inline]
pub fn pow(x: f64, y: f64) -> f64 {
    libm::pow(x, y)
}

/// Java `Math.sqrt` (correctly rounded).
#[inline]
pub fn sqrt(x: f64) -> f64 {
    libm::sqrt(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_strict_math_reference_values() {
        // StrictMath values from OpenJDK 17
        assert_eq!(acos(0.5).to_bits(), 0x3ff0c152382d7366);
        assert_eq!(acos(-0.3).to_bits(), 0x3ffe0200bbc96ad8);
        assert_eq!(acos(0.99).to_bits(), 0x3fc21df72882bfd8);
        assert_eq!(acos(-0.75).to_bits(), 0x400359d26f93b6c3);
        assert_eq!(acos(1e-10).to_bits(), 0x3ff921fb543d4de0);
        assert_eq!(acos(1.0), 0.0);
        assert!(acos(1.5).is_nan());
        assert_eq!(sqrt(2.0), std::f64::consts::SQRT_2);
    }
}
