//! Port of `PolylineArea.java`: an Area whose outside border curve and hole borders consist of
//! straight lines.

use std::sync::{Arc, OnceLock};

use crate::float_point::FloatPoint;
use crate::int_box::IntBox;
use crate::int_octagon::IntOctagon;
use crate::int_point::IntPoint;
use crate::point::Point;
use crate::polyline_shape::PolylineShape;
use crate::stoppable::Stoppable;
use crate::tile_shape::TileShape;
use crate::vector::Vector;

#[derive(Clone)]
pub struct PolylineArea {
    pub border_shape: PolylineShape,
    pub hole_arr: Arc<[PolylineShape]>,
    precalculated_convex_pieces: Arc<OnceLock<Vec<TileShape>>>,
}

impl std::fmt::Debug for PolylineArea {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PolylineArea")
            .field("border_shape", &self.border_shape)
            .field("hole_arr", &self.hole_arr)
            .finish()
    }
}

fn cutout_hole_piece(
    divide_piece: &TileShape,
    hole_piece: &TileShape,
    pieces: &mut Vec<TileShape>,
) {
    let result_pieces = divide_piece
        .cutout(hole_piece)
        .expect("NullPointerException: TileShape.cutout returned null");
    for current_piece in result_pieces {
        if current_piece.dimension() == 2 {
            pieces.push(current_piece);
        }
    }
}

impl PolylineArea {
    pub fn new(border_shape: PolylineShape, hole_arr: Vec<PolylineShape>) -> PolylineArea {
        PolylineArea {
            border_shape,
            hole_arr: hole_arr.into(),
            precalculated_convex_pieces: Arc::new(OnceLock::new()),
        }
    }

    pub fn dimension(&self) -> i32 {
        self.border_shape.dimension()
    }

    pub fn is_bounded(&self) -> bool {
        self.border_shape.is_bounded()
    }

    pub fn is_empty(&self) -> bool {
        self.border_shape.is_empty()
    }

    pub fn is_contained_in(&self, b: &IntBox) -> bool {
        self.border_shape.is_contained_in(b)
    }

    pub fn get_border(&self) -> &PolylineShape {
        &self.border_shape
    }

    pub fn get_holes(&self) -> &[PolylineShape] {
        &self.hole_arr
    }

    pub fn bounding_box(&self) -> IntBox {
        self.border_shape.bounding_box()
    }

    pub fn bounding_octagon(&self) -> Option<IntOctagon> {
        self.border_shape.bounding_octagon()
    }

    pub fn contains_float(&self, point: &FloatPoint) -> bool {
        if !self.border_shape.contains_float(point) {
            return false;
        }
        !self.hole_arr.iter().any(|h| h.contains_float(point))
    }

    pub fn contains(&self, point: &Point) -> bool {
        if !self.border_shape.contains(point) {
            return false;
        }
        !self.hole_arr.iter().any(|h| h.contains_inside(point))
    }

    pub fn nearest_point_approx(&self, from_point: &FloatPoint) -> Option<FloatPoint> {
        let mut min_dist = f64::MAX;
        let mut result = None;
        let convex_shapes = self
            .split_to_convex()
            .expect("NullPointerException: splitToConvex failed");
        for s in convex_shapes {
            let current_nearest_point = s
                .nearest_point_approx(from_point)
                .expect("NullPointerException: nearestPointApprox returned null");
            let current_distance = current_nearest_point.distance_square(from_point);
            if current_distance < min_dist {
                min_dist = current_distance;
                result = Some(current_nearest_point);
            }
        }
        result
    }

    pub fn translate_by(&self, vector: &Vector) -> PolylineArea {
        if *vector == Vector::ZERO {
            return self.clone();
        }
        PolylineArea::new(
            self.border_shape.translate_by(vector),
            self.hole_arr
                .iter()
                .map(|h| h.translate_by(vector))
                .collect(),
        )
    }

    pub fn corner_approx_arr(&self) -> Vec<FloatPoint> {
        let mut result = self.border_shape.corner_approx_arr();
        for h in self.hole_arr.iter() {
            result.extend(h.corner_approx_arr());
        }
        result
    }

    /// Splits this area into convex pieces (not exact, rounded intersections are used). None if
    /// the split failed.
    pub fn split_to_convex(&self) -> Option<Vec<TileShape>> {
        self.split_to_convex_stoppable(None)
    }

    /// Like `split_to_convex`; returns None if a stop is requested via stoppable_thread.
    pub fn split_to_convex_stoppable(
        &self,
        stoppable_thread: Option<&dyn Stoppable>,
    ) -> Option<Vec<TileShape>> {
        if let Some(p) = self.precalculated_convex_pieces.get() {
            return Some(p.clone());
        }
        let convex_border_pieces = self.border_shape.split_to_convex()?;
        let mut current_piece_list: Vec<TileShape> = convex_border_pieces;
        for hole in self.hole_arr.iter() {
            if hole.dimension() < 2 {
                log::warn!("PolylineArea. split_to_convex: dimension 2 for hole expected");
                continue;
            }
            let convex_hole_pieces = hole.split_to_convex()?;
            for current_hole_piece in &convex_hole_pieces {
                let mut new_piece_list: Vec<TileShape> = Vec::new();
                for current_divide_piece in &current_piece_list {
                    if let Some(s) = stoppable_thread {
                        if s.is_stop_requested() {
                            return None;
                        }
                    }
                    cutout_hole_piece(
                        current_divide_piece,
                        current_hole_piece,
                        &mut new_piece_list,
                    );
                }
                current_piece_list = new_piece_list;
            }
        }
        Some(
            self.precalculated_convex_pieces
                .get_or_init(|| current_piece_list)
                .clone(),
        )
    }

    pub fn turn_90_degree(&self, factor: i32, pole: &IntPoint) -> PolylineArea {
        PolylineArea::new(
            self.border_shape.turn_90_degree(factor, pole),
            self.hole_arr
                .iter()
                .map(|h| h.turn_90_degree(factor, pole))
                .collect(),
        )
    }

    pub fn rotate_approx(&self, angle: f64, pole: &FloatPoint) -> PolylineArea {
        PolylineArea::new(
            self.border_shape.rotate_approx(angle, pole),
            self.hole_arr
                .iter()
                .map(|h| h.rotate_approx(angle, pole))
                .collect(),
        )
    }

    pub fn mirror_vertical(&self, pole: &IntPoint) -> PolylineArea {
        PolylineArea::new(
            self.border_shape.mirror_vertical(pole),
            self.hole_arr
                .iter()
                .map(|h| h.mirror_vertical(pole))
                .collect(),
        )
    }

    pub fn mirror_horizontal(&self, pole: &IntPoint) -> PolylineArea {
        PolylineArea::new(
            self.border_shape.mirror_horizontal(pole),
            self.hole_arr
                .iter()
                .map(|h| h.mirror_horizontal(pole))
                .collect(),
        )
    }
}
