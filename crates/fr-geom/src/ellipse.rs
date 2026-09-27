//! Port of `Ellipse.java`: an ellipse in the plane (float coordinates, not a ConvexShape).

use std::f64::consts::PI;

use crate::float_point::FloatPoint;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ellipse {
    pub center: FloatPoint,
    /// Rotation of the ellipse in radian normed to 0 <= rotation < pi.
    pub rotation: f64,
    pub bigger_radius: f64,
    pub smaller_radius: f64,
}

impl Ellipse {
    pub fn new(center: FloatPoint, rotation: f64, radius1: f64, radius2: f64) -> Ellipse {
        let (bigger_radius, smaller_radius, mut current_rotation) = if radius1 >= radius2 {
            (radius1, radius2, rotation)
        } else {
            (radius2, radius1, rotation + 0.5 * PI)
        };
        while current_rotation >= PI {
            current_rotation -= PI;
        }
        while current_rotation < 0.0 {
            current_rotation += PI;
        }
        Ellipse {
            center,
            rotation: current_rotation,
            bigger_radius,
            smaller_radius,
        }
    }
}
