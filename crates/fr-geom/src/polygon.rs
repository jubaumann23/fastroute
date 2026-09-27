//! Port of `Polygon.java`: a list of points where no 2 consecutive points are equal and no 3
//! consecutive points are collinear.

use std::f64::consts::PI;

use crate::java_compat::math_round_i32;
use crate::point::Point;
use crate::side::Side;

#[derive(Clone, Debug)]
pub struct Polygon {
    corners: Vec<Point>,
}

impl Polygon {
    /// Creates a polygon from points. Multiple points and points, which are collinear with their
    /// previous and next point, are removed.
    pub fn new(points: &[Point]) -> Polygon {
        let mut corners: Vec<Point> = points.to_vec();
        if corners.is_empty() {
            return Polygon { corners };
        }
        let mut corner_removed = true;
        while corner_removed {
            corner_removed = false;
            if corners.is_empty() {
                break;
            }
            // remove multiple points
            let mut i = 1;
            while i < corners.len() {
                if corners[i] == corners[i - 1] {
                    corners.remove(i);
                    corner_removed = true;
                } else {
                    i += 1;
                }
            }
            // remove points which are collinear with the previous and next point
            // (only the first such point per pass, like the Java iterator code)
            if corners.len() < 2 {
                continue;
            }
            for k in 1..corners.len().saturating_sub(1) {
                if corners[k].side_of(&corners[k - 1], &corners[k + 1]) == Side::Collinear {
                    corners.remove(k);
                    corner_removed = true;
                    break;
                }
            }
        }
        Polygon { corners }
    }

    /// Returns the array of corners of this polygon.
    pub fn corner_array(&self) -> Vec<Point> {
        self.corners.clone()
    }

    /// Returns the corners as slice.
    pub fn corners(&self) -> &[Point] {
        &self.corners
    }

    /// Reverts the order of the corners of this polygon.
    pub fn revert_corners(&self) -> Polygon {
        let reversed: Vec<Point> = self.corners.iter().rev().cloned().collect();
        Polygon::new(&reversed)
    }

    /// Returns the winding number of this polygon, treated as closed. It is > 0 if the corners
    /// are in counterclock sense and < 0 if they are in clockwise sense.
    pub fn winding_number_after_closing(&self) -> i32 {
        let corners = &self.corners;
        if corners.len() < 2 {
            return 0;
        }
        let first_side_vector = corners[1].difference_by(&corners[0]);
        let mut prev_side_vector = first_side_vector.clone();
        let mut corner_count = corners.len();
        // Skip the last corner, if it is equal to the first corner.
        if corners[0] == corners[corner_count - 1] {
            corner_count -= 1;
        }
        let mut angle_sum = 0.0;
        for i in 1..corner_count.saturating_sub(1) {
            let next_side_vector = corners[i + 1].difference_by(&corners[i]);
            angle_sum += prev_side_vector.angle_approx_to(&next_side_vector);
            prev_side_vector = next_side_vector;
        }
        if corner_count > 1 {
            let next_side_vector = corners[0].difference_by(&corners[corner_count - 1]);
            angle_sum += prev_side_vector.angle_approx_to(&next_side_vector);
            prev_side_vector = next_side_vector;
        }
        angle_sum += prev_side_vector.angle_approx_to(&first_side_vector);
        angle_sum /= 2.0 * PI;
        if angle_sum.abs() < 0.5 {
            log::warn!("Polygon.winding_number_after_closing: winding number != 0 expected");
        }
        math_round_i32(angle_sum)
    }
}
