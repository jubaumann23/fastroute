//! Port of `board/state/ChangedArea.java`: marks changed areas on the board after shoving and
//! optimizing items (one double-precision octagon per layer).

use fr_geom::{FloatPoint, IntBox, IntOctagon, TileShape};

use crate::ids::LayerNo;

/// Java `Math.min(double, double)` (NaN propagating, -0.0 < 0.0).
#[inline]
fn java_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    if a == 0.0 && b == 0.0 {
        return if a.is_sign_negative() { a } else { b };
    }
    if a <= b {
        a
    } else {
        b
    }
}

/// Java `Math.max(double, double)` (NaN propagating, 0.0 > -0.0).
#[inline]
fn java_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    if a == 0.0 && b == 0.0 {
        return if a.is_sign_positive() { a } else { b };
    }
    if a >= b {
        a
    } else {
        b
    }
}

/// Java `Math.min(int, int)` / `Math.max` on the `(int)` casts are plain; `(int) Math.floor(x)`
/// is a saturating cast like Rust `as`.
#[inline]
fn floor_i(x: f64) -> i32 {
    x.floor() as i32
}

#[inline]
fn ceil_i(x: f64) -> i32 {
    x.ceil() as i32
}

/// Mutable octagon with double coordinates (Java `ChangedArea.MutableOctagon`).
#[derive(Clone, Copy, Debug, PartialEq)]
struct MutableOctagon {
    lx: f64,
    ly: f64,
    rx: f64,
    uy: f64,
    ulx: f64,
    lrx: f64,
    llx: f64,
    urx: f64,
}

impl MutableOctagon {
    const EMPTY: MutableOctagon = MutableOctagon {
        lx: i32::MAX as f64,
        ly: i32::MAX as f64,
        rx: i32::MIN as f64,
        uy: i32::MIN as f64,
        ulx: i32::MAX as f64,
        lrx: i32::MIN as f64,
        llx: i32::MAX as f64,
        urx: i32::MIN as f64,
    };

    /// The smallest IntOctagon containing this octagon.
    fn to_int(self) -> IntOctagon {
        if self.rx < self.lx || self.uy < self.ly || self.lrx < self.ulx || self.urx < self.llx {
            return IntOctagon::EMPTY;
        }
        IntOctagon::new(
            floor_i(self.lx),
            floor_i(self.ly),
            ceil_i(self.rx),
            ceil_i(self.uy),
            floor_i(self.ulx),
            ceil_i(self.lrx),
            floor_i(self.llx),
            ceil_i(self.urx),
        )
    }
}

/// Changed area per layer (Java `ChangedArea`).
#[derive(Clone, Debug, PartialEq)]
pub struct ChangedArea {
    arr: Vec<MutableOctagon>,
}

impl ChangedArea {
    pub fn new(layer_count: i32) -> Self {
        ChangedArea {
            arr: vec![MutableOctagon::EMPTY; layer_count.max(0) as usize],
        }
    }

    pub fn layer_count(&self) -> i32 {
        self.arr.len() as i32
    }

    /// Enlarges the octagon on `layer` so that it contains `point`.
    pub fn join(&mut self, point: &FloatPoint, layer: LayerNo) {
        let current = &mut self.arr[layer as usize];
        current.lx = java_min(point.x, current.lx);
        current.ly = java_min(point.y, current.ly);
        current.rx = java_max(current.rx, point.x);
        current.uy = java_max(current.uy, point.y);

        let tmp = point.x - point.y;
        current.ulx = java_min(current.ulx, tmp);
        current.lrx = java_max(current.lrx, tmp);

        let tmp = point.x + point.y;
        current.llx = java_min(current.llx, tmp);
        current.urx = java_max(current.urx, tmp);
    }

    /// Enlarges the octagon on `layer` so that it contains `shape` (Java: no-op for null).
    pub fn join_shape(&mut self, shape: Option<&TileShape>, layer: LayerNo) {
        let Some(shape) = shape else {
            return;
        };
        let corner_count = shape.border_line_count();
        for i in 0..corner_count {
            self.join(&shape.corner_approx(i), layer);
        }
    }

    /// The marking octagon on `layer`.
    pub fn get_area(&self, layer: LayerNo) -> IntOctagon {
        self.arr[layer as usize].to_int()
    }

    /// The bounding box of all layers' octagons (`IntBox.EMPTY` if nothing was joined).
    pub fn surrounding_box(&self) -> IntBox {
        let mut llx = i32::MAX;
        let mut lly = i32::MAX;
        let mut urx = i32::MIN;
        let mut ury = i32::MIN;
        for current in &self.arr {
            llx = llx.min(floor_i(current.lx));
            lly = lly.min(floor_i(current.ly));
            urx = urx.max(ceil_i(current.rx));
            ury = ury.max(ceil_i(current.uy));
        }
        if llx > urx || lly > ury {
            return IntBox::EMPTY;
        }
        IntBox::new(llx, lly, urx, ury)
    }

    /// Resets the marking octagon on `layer` to empty.
    pub fn set_empty(&mut self, layer: LayerNo) {
        self.arr[layer as usize] = MutableOctagon::EMPTY;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_and_query() {
        let mut ca = ChangedArea::new(2);
        assert!(ca.get_area(0).is_empty());
        assert!(ca.surrounding_box().is_empty());
        ca.join(&FloatPoint::new(1.5, -2.5), 1);
        ca.join(&FloatPoint::new(10.2, 3.0), 1);
        let o = ca.get_area(1);
        assert_eq!((o.left_x, o.bottom_y, o.right_x, o.top_y), (1, -3, 11, 3));
        // ulx = min(x - y) = 4.0 ; lrx = max(x - y) = 7.2 ; llx = min(x + y) = -1 ; urx = 13.2
        assert_eq!(
            (
                o.upper_left_diagonal_x,
                o.lower_right_diagonal_x,
                o.lower_left_diagonal_x,
                o.upper_right_diagonal_x
            ),
            (4, 8, -1, 14)
        );
        assert!(ca.get_area(0).is_empty());
        assert_eq!(ca.surrounding_box(), IntBox::new(1, -3, 11, 3));
        ca.set_empty(1);
        assert!(ca.get_area(1).is_empty());
        ca.join_shape(Some(&TileShape::IntBox(IntBox::new(0, 0, 4, 2))), 0);
        assert_eq!(ca.surrounding_box(), IntBox::new(0, 0, 4, 2));
    }
}
