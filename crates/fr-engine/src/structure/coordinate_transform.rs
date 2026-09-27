//! Port of `io/CoordinateTransform.java`.
//!
//! `boardToDsn(Shape, Layer)` returns Specctra parser shapes in Java; here the coordinates are
//! returned as [`DsnShapeCoords`] and the io layer attaches the layer.

use fr_geom::{FloatPoint, IntBox, Line, Shape, TileShape, Vector};

/// Transformations between board coordinates and external (DSN) coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoordinateTransform {
    scale_factor: f64,
    base_x: f64,
    base_y: f64,
}

/// Coordinates of a board shape in external coordinates (Java: `Rectangle`, `Polygon`,
/// `Circle` of `io.specctra.parser`).
#[derive(Clone, Debug, PartialEq)]
pub enum DsnShapeCoords {
    /// `[llx, lly, urx, ury]`.
    Rectangle([f64; 4]),
    /// Corner coordinates `x0, y0, x1, y1, ...`.
    Polygon(Vec<f64>),
    Circle {
        diameter: f64,
        x: f64,
        y: f64,
    },
}

impl CoordinateTransform {
    pub fn new(scale_factor: f64, base_x: f64, base_y: f64) -> Self {
        CoordinateTransform {
            scale_factor,
            base_x,
            base_y,
        }
    }

    pub fn scale_factor(&self) -> f64 {
        self.scale_factor
    }

    /// Scales a value from the board to the external coordinate system.
    #[inline]
    pub fn board_to_dsn(&self, value: f64) -> f64 {
        value / self.scale_factor
    }

    /// Transforms a point from the board to the external coordinate system.
    pub fn board_to_dsn_point(&self, point: &FloatPoint) -> [f64; 2] {
        [
            self.board_to_dsn(point.x) + self.base_x,
            self.board_to_dsn(point.y) + self.base_y,
        ]
    }

    /// Transforms points from the board to the external coordinate system.
    pub fn board_to_dsn_points(&self, points: &[FloatPoint]) -> Vec<f64> {
        let mut result = Vec::with_capacity(2 * points.len());
        for p in points {
            result.push(self.board_to_dsn(p.x) + self.base_x);
            result.push(self.board_to_dsn(p.y) + self.base_y);
        }
        result
    }

    /// Transforms lines (their two defining points) to the external coordinate system.
    pub fn board_to_dsn_lines(&self, lines: &[Line]) -> Vec<f64> {
        let mut result = Vec::with_capacity(4 * lines.len());
        for line in lines {
            let a = line.a.to_float();
            let b = line.b.to_float();
            result.push(self.board_to_dsn(a.x) + self.base_x);
            result.push(self.board_to_dsn(a.y) + self.base_y);
            result.push(self.board_to_dsn(b.x) + self.base_x);
            result.push(self.board_to_dsn(b.y) + self.base_y);
        }
        result
    }

    /// Transforms a vector (no translation).
    pub fn board_to_dsn_vector(&self, vector: &Vector) -> [f64; 2] {
        let value = vector.to_float();
        [self.board_to_dsn(value.x), self.board_to_dsn(value.y)]
    }

    /// Transforms a box.
    pub fn board_to_dsn_box(&self, b: &IntBox) -> [f64; 4] {
        [
            b.ll.x as f64 / self.scale_factor + self.base_x,
            b.ll.y as f64 / self.scale_factor + self.base_y,
            b.ur.x as f64 / self.scale_factor + self.base_x,
            b.ur.y as f64 / self.scale_factor + self.base_y,
        ]
    }

    /// Transforms a board shape to external coordinates; `None` where Java warns and returns
    /// null (no such case with the closed `Shape` hierarchy).
    pub fn board_to_dsn_shape(&self, shape: &Shape) -> Option<DsnShapeCoords> {
        match shape {
            Shape::Tile(TileShape::IntBox(b)) => {
                Some(DsnShapeCoords::Rectangle(self.board_to_dsn_box(b)))
            }
            Shape::Tile(_) | Shape::Polygon(_) => Some(DsnShapeCoords::Polygon(
                self.board_to_dsn_points(&shape.corner_approx_arr()),
            )),
            Shape::Circle(c) => {
                let diameter = 2.0 * self.board_to_dsn(c.radius as f64);
                let center = self.board_to_dsn_point(&c.center.to_float());
                Some(DsnShapeCoords::Circle {
                    diameter,
                    x: center[0],
                    y: center[1],
                })
            }
        }
    }

    /// Transforms a point to relative external (vector) coordinates.
    pub fn board_to_dsn_rel_point(&self, point: &FloatPoint) -> [f64; 2] {
        [self.board_to_dsn(point.x), self.board_to_dsn(point.y)]
    }

    /// Transforms points to relative external coordinates.
    pub fn board_to_dsn_rel_points(&self, points: &[FloatPoint]) -> Vec<f64> {
        let mut result = Vec::with_capacity(2 * points.len());
        for p in points {
            result.push(self.board_to_dsn(p.x));
            result.push(self.board_to_dsn(p.y));
        }
        result
    }

    /// Transforms a box to relative external coordinates.
    pub fn board_to_dsn_rel_box(&self, b: &IntBox) -> [f64; 4] {
        [
            b.ll.x as f64 / self.scale_factor,
            b.ll.y as f64 / self.scale_factor,
            b.ur.x as f64 / self.scale_factor,
            b.ur.y as f64 / self.scale_factor,
        ]
    }

    /// Transforms a board shape to relative external coordinates.
    pub fn board_to_dsn_rel_shape(&self, shape: &Shape) -> Option<DsnShapeCoords> {
        match shape {
            Shape::Tile(TileShape::IntBox(b)) => {
                Some(DsnShapeCoords::Rectangle(self.board_to_dsn_rel_box(b)))
            }
            Shape::Tile(_) | Shape::Polygon(_) => Some(DsnShapeCoords::Polygon(
                self.board_to_dsn_rel_points(&shape.corner_approx_arr()),
            )),
            Shape::Circle(c) => {
                let diameter = 2.0 * self.board_to_dsn(c.radius as f64);
                let center = self.board_to_dsn_rel_point(&c.center.to_float());
                Some(DsnShapeCoords::Circle {
                    diameter,
                    x: center[0],
                    y: center[1],
                })
            }
        }
    }

    /// Scales a value from the external to the board coordinate system.
    #[inline]
    pub fn dsn_to_board(&self, value: f64) -> f64 {
        value * self.scale_factor
    }

    /// Transforms an external tuple to a board point.
    pub fn dsn_to_board_point(&self, tuple: &[f64]) -> FloatPoint {
        let x = self.dsn_to_board(tuple[0] - self.base_x);
        let y = self.dsn_to_board(tuple[1] - self.base_y);
        FloatPoint::new(x, y)
    }

    /// Transforms an external tuple to a board point in relative coordinates.
    pub fn dsn_to_board_rel(&self, tuple: &[f64]) -> FloatPoint {
        FloatPoint::new(self.dsn_to_board(tuple[0]), self.dsn_to_board(tuple[1]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fr_geom::{Circle, IntPoint};

    #[test]
    fn round_trip() {
        let t = CoordinateTransform::new(10.0, 100.0, -50.0);
        for &(x, y) in &[(0.0, 0.0), (12345.0, -678.0), (-1.5e6, 2.5e6)] {
            let p = FloatPoint::new(x, y);
            let d = t.board_to_dsn_point(&p);
            assert_eq!(d, [x / 10.0 + 100.0, y / 10.0 - 50.0]);
            let back = t.dsn_to_board_point(&d);
            assert!((back.x - x).abs() < 1e-6 && (back.y - y).abs() < 1e-6);
            let r = t.board_to_dsn_rel_point(&p);
            assert_eq!(t.dsn_to_board_rel(&r), p);
        }
        // exact arithmetic order: (value - base) * scale
        let d = [1.1, 2.2];
        assert_eq!(
            t.dsn_to_board_point(&d),
            FloatPoint::new((1.1 - 100.0) * 10.0, (2.2 + 50.0) * 10.0)
        );
    }

    #[test]
    fn shapes() {
        let t = CoordinateTransform::new(2.0, 1.0, 1.0);
        let b = IntBox::new(0, 2, 10, 20);
        assert_eq!(t.board_to_dsn_box(&b), [1.0, 2.0, 6.0, 11.0]);
        assert_eq!(t.board_to_dsn_rel_box(&b), [0.0, 1.0, 5.0, 10.0]);
        assert_eq!(
            t.board_to_dsn_shape(&Shape::Tile(TileShape::IntBox(b))),
            Some(DsnShapeCoords::Rectangle([1.0, 2.0, 6.0, 11.0]))
        );
        let c = Circle::new(IntPoint::new(4, 6), 3);
        assert_eq!(
            t.board_to_dsn_shape(&Shape::Circle(c)),
            Some(DsnShapeCoords::Circle {
                diameter: 3.0,
                x: 3.0,
                y: 4.0
            })
        );
        assert_eq!(
            t.board_to_dsn_rel_shape(&Shape::Circle(c)),
            Some(DsnShapeCoords::Circle {
                diameter: 3.0,
                x: 2.0,
                y: 3.0
            })
        );
        assert_eq!(
            t.board_to_dsn_vector(&Vector::from(fr_geom::IntVector::new(4, -2))),
            [2.0, -1.0]
        );
    }
}
