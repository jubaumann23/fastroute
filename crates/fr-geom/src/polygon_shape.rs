//! Port of `PolygonShape.java`: shape described by a closed polygon of corner points.
//!
//! The corners are ordered in counterclock sense and normalised, so that the corner with the
//! lowest y-value (then lowest x-value) comes first.

use std::sync::{Arc, OnceLock};

use crate::circle::Circle;
use crate::float_point::{java_max, java_min, FloatPoint};
use crate::int_box::IntBox;
use crate::int_octagon::IntOctagon;
use crate::int_point::IntPoint;
use crate::java_compat::{JavaRandom, INT_MAX_F64, INT_MIN_F64};
use crate::line::Line;
use crate::point::Point;
use crate::polygon::Polygon;
use crate::polyline::Polyline;
use crate::polyline_shape::PolylineShapeImpl;
use crate::regular_tile_shape::RegularTileShape;
use crate::shape_bounding_directions::ShapeBoundingDirections;
use crate::side::Side;
use crate::simplex::Simplex;
use crate::tile_shape::TileShape;
use crate::vector::Vector;

const SEED: i64 = 99;

#[derive(Default, Debug)]
struct PolygonShapeCache {
    bounding_box: OnceLock<IntBox>,
    bounding_octagon: OnceLock<IntOctagon>,
    convex_pieces: OnceLock<Vec<TileShape>>,
}

/// Cloning is cheap and shares the lazily computed caches.
#[derive(Clone)]
pub struct PolygonShape {
    pub corners: Arc<[Point]>,
    cache: Arc<PolygonShapeCache>,
}

impl std::fmt::Debug for PolygonShape {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PolygonShape")
            .field("corners", &self.corners)
            .finish()
    }
}

impl PolygonShape {
    /// Creates a new instance of PolygonShape.
    pub fn new(polygon: &Polygon) -> PolygonShape {
        let reverted;
        let current_polygon = if polygon.winding_number_after_closing() < 0 {
            // the corners of the polygon are in clockwise sense
            reverted = polygon.revert_corners();
            &reverted
        } else {
            polygon
        };
        let current_corners = current_polygon.corners();
        let mut last_corner_no: i32 = current_corners.len() as i32 - 1;
        if last_corner_no > 0 && current_corners[0] == current_corners[last_corner_no as usize] {
            // skip last point
            last_corner_no -= 1;
        }
        let mut last_point_collinear = false;
        if last_corner_no >= 2 {
            let l = last_corner_no as usize;
            last_point_collinear = current_corners[l]
                .side_of(&current_corners[l - 1], &current_corners[0])
                == Side::Collinear;
        }
        if last_point_collinear {
            // skip last point
            last_corner_no -= 1;
        }
        let mut first_corner_no: i32 = 0;
        let mut first_point_collinear = false;
        if last_corner_no - first_corner_no >= 2 {
            first_point_collinear = current_corners[0].side_of(
                &current_corners[1],
                &current_corners[last_corner_no as usize],
            ) == Side::Collinear;
        }
        if first_point_collinear {
            // skip first point
            first_corner_no += 1;
        }
        // search the point with the lowest y and then with the lowest x
        let mut start_corner_no = first_corner_no;
        let mut result: Vec<Point> = Vec::new();
        if first_corner_no <= last_corner_no {
            let mut start_corner = current_corners[start_corner_no as usize].to_float();
            for i in start_corner_no + 1..=last_corner_no {
                let current_corner = current_corners[i as usize].to_float();
                if current_corner.y < start_corner.y
                    || current_corner.y == start_corner.y && current_corner.x < start_corner.x
                {
                    start_corner_no = i;
                    start_corner = current_corner;
                }
            }
            for i in start_corner_no..=last_corner_no {
                result.push(current_corners[i as usize].clone());
            }
            for i in first_corner_no..start_corner_no {
                result.push(current_corners[i as usize].clone());
            }
        } else if current_corners.is_empty() {
            // Java: currentCorners[0] throws for an empty polygon
            panic!("ArrayIndexOutOfBoundsException: PolygonShape from empty polygon");
        }
        PolygonShape::from_normalized_corners(result)
    }

    fn from_normalized_corners(corners: Vec<Point>) -> PolygonShape {
        PolygonShape {
            corners: corners.into(),
            cache: Arc::new(PolygonShapeCache::default()),
        }
    }

    /// Creates a polygon shape from an array of corner points.
    pub fn from_points(corners: &[Point]) -> PolygonShape {
        PolygonShape::new(&Polygon::new(corners))
    }

    /// Returns the no-th corner; None if no is out of range (Java returns null).
    pub fn corner_opt(&self, no: i32) -> Option<Point> {
        if no < 0 || no >= self.corners.len() as i32 {
            log::warn!("PolygonShape.corner: no out of range");
            return None;
        }
        Some(self.corners[no as usize].clone())
    }

    pub fn border_line_count(&self) -> i32 {
        self.corners.len() as i32
    }

    pub fn corner_is_bounded(&self, _no: i32) -> bool {
        true
    }

    fn pieces(&self) -> Vec<TileShape> {
        self.split_to_convex()
            .expect("NullPointerException: splitToConvex failed")
    }

    pub fn intersects_circle(&self, circle: &Circle) -> bool {
        self.pieces().iter().any(|p| p.intersects_circle(circle))
    }

    pub fn intersects_simplex(&self, simplex: &Simplex) -> bool {
        self.pieces().iter().any(|p| p.intersects_simplex(simplex))
    }

    pub fn intersects_int_octagon(&self, oct: &IntOctagon) -> bool {
        self.pieces().iter().any(|p| p.intersects_int_octagon(oct))
    }

    pub fn intersects_int_box(&self, b: &IntBox) -> bool {
        self.pieces().iter().any(|p| p.intersects_int_box(b))
    }

    /// Not implemented in Java (returns null).
    pub fn cutout_polyline(&self, _polyline: &Polyline) -> Option<Vec<Polyline>> {
        log::warn!("PolygonShape.cutout not yet implemented");
        None
    }

    /// Returns this shape for offset 0; None otherwise (not implemented in Java).
    pub fn enlarge(&self, offset: f64) -> Option<PolygonShape> {
        if offset == 0.0 {
            return Some(self.clone());
        }
        log::warn!("PolygonShape.enlarge not yet implemented");
        None
    }

    /// Not implemented in Java (returns 0).
    pub fn border_distance(&self, _point: &FloatPoint) -> f64 {
        log::warn!("PolygonShape.border_distance not yet implemented");
        0.0
    }

    pub fn smallest_radius(&self) -> f64 {
        self.border_distance(&PolylineShapeImpl::centre_of_gravity(self))
    }

    pub fn contains_float(&self, point: &FloatPoint) -> bool {
        self.pieces().iter().any(|p| p.contains_float(point))
    }

    pub fn contains(&self, point: &Point) -> bool {
        !self.is_outside(point)
    }

    pub fn contains_inside(&self, point: &Point) -> bool {
        if self.contains_on_border(point) {
            return false;
        }
        !self.is_outside(point)
    }

    pub fn is_outside(&self, point: &Point) -> bool {
        self.pieces().iter().all(|p| p.is_outside(point))
    }

    /// Not implemented in Java (always false).
    pub fn contains_on_border(&self, _point: &Point) -> bool {
        false
    }

    /// Not implemented in Java (returns 0).
    pub fn distance(&self, _point: &FloatPoint) -> f64 {
        log::warn!("PolygonShape.distance not yet implemented");
        0.0
    }

    pub fn translate_by(&self, vector: &Vector) -> PolygonShape {
        if *vector == Vector::ZERO {
            return self.clone();
        }
        let new_corners: Vec<Point> = self
            .corners
            .iter()
            .map(|c| c.translate_by(vector))
            .collect();
        PolygonShape::from_points(&new_corners)
    }

    pub fn bounding_shape(&self, dirs: &ShapeBoundingDirections) -> RegularTileShape {
        dirs.bounds_polygon(self)
    }

    pub fn bounding_box(&self) -> IntBox {
        *self.cache.bounding_box.get_or_init(|| {
            let mut llx = INT_MAX_F64;
            let mut lly = INT_MAX_F64;
            let mut urx = INT_MIN_F64;
            let mut ury = INT_MIN_F64;
            for c in self.corners.iter() {
                let current = c.to_float();
                llx = java_min(llx, current.x);
                lly = java_min(lly, current.y);
                urx = java_max(urx, current.x);
                ury = java_max(ury, current.y);
            }
            let lower_left = IntPoint::new(llx.floor() as i32, lly.floor() as i32);
            let upper_right = IntPoint::new(urx.ceil() as i32, ury.ceil() as i32);
            IntBox::from_points(lower_left, upper_right)
        })
    }

    pub fn bounding_octagon(&self) -> IntOctagon {
        *self.cache.bounding_octagon.get_or_init(|| {
            let mut lx = INT_MAX_F64;
            let mut ly = INT_MAX_F64;
            let mut rx = INT_MIN_F64;
            let mut uy = INT_MIN_F64;
            let mut ulx = INT_MAX_F64;
            let mut lrx = INT_MIN_F64;
            let mut llx = INT_MAX_F64;
            let mut urx = INT_MIN_F64;
            for c in self.corners.iter() {
                let current = c.to_float();
                lx = java_min(lx, current.x);
                ly = java_min(ly, current.y);
                rx = java_max(rx, current.x);
                uy = java_max(uy, current.y);
                let tmp = current.x - current.y;
                ulx = java_min(ulx, tmp);
                lrx = java_max(lrx, tmp);
                let tmp = current.x + current.y;
                llx = java_min(llx, tmp);
                urx = java_max(urx, tmp);
            }
            IntOctagon::new(
                lx.floor() as i32,
                ly.floor() as i32,
                rx.ceil() as i32,
                uy.ceil() as i32,
                ulx.floor() as i32,
                lrx.ceil() as i32,
                llx.floor() as i32,
                urx.ceil() as i32,
            )
        })
    }

    /// Checks, if every line segment between 2 points of the shape is contained completely in
    /// the shape.
    pub fn is_convex(&self) -> bool {
        let corners = &self.corners;
        let n = corners.len();
        if n <= 2 {
            return true;
        }
        let mut prev_point = &corners[n - 1];
        let mut current_point = &corners[0];
        let mut next_point = &corners[1];
        for ind in 0..n {
            if next_point.side_of(prev_point, current_point) == Side::OnTheRight {
                return false;
            }
            prev_point = current_point;
            current_point = next_point;
            next_point = if ind == n - 2 {
                &corners[0]
            } else if ind == n - 1 {
                &corners[1]
            } else {
                &corners[ind + 2]
            };
        }
        // check, if the sum of the interior angles is at most 2 * pi
        let first_line = Line::new(corners[n - 1].clone(), corners[0].clone());
        let mut current_line = Line::new(corners[0].clone(), corners[1].clone());
        let first_direction = first_line.int_direction();
        let current_direction = current_line.int_direction();
        let mut last_det = first_direction.determinant(&current_direction);
        for c in corners.iter().skip(2) {
            current_line = Line::new(current_line.b.clone(), c.clone());
            let current_direction = current_line.int_direction();
            let current_det = first_direction.determinant(&current_direction);
            if last_det <= 0.0 && current_det > 0.0 {
                return false;
            }
            last_det = current_det;
        }
        true
    }

    /// Returns the convex hull of this polygon shape.
    pub fn convex_hull(&self) -> PolygonShape {
        let corners = &self.corners;
        let n = corners.len();
        if n <= 2 {
            return self.clone();
        }
        let mut prev_point = &corners[n - 1];
        let mut current_point = &corners[0];
        for ind in 0..n {
            let next_point = if ind == n - 1 {
                &corners[0]
            } else {
                &corners[ind + 1]
            };
            if next_point.side_of(prev_point, current_point) != Side::OnTheLeft {
                // skip current_point
                let mut new_corners: Vec<Point> = corners.to_vec();
                new_corners.remove(ind);
                let result = PolygonShape::from_points(&new_corners);
                return result.convex_hull();
            }
            prev_point = current_point;
            current_point = next_point;
        }
        self.clone()
    }

    pub fn bounding_tile(&self) -> TileShape {
        let hull = self.convex_hull();
        let c = &hull.corners;
        let n = c.len();
        let mut bounding_lines: Vec<Line> = Vec::with_capacity(n);
        for i in 0..n.saturating_sub(1) {
            bounding_lines.push(Line::new(c[i].clone(), c[i + 1].clone()));
        }
        bounding_lines.push(Line::new(c[n - 1].clone(), c[0].clone()));
        TileShape::get_instance_lines(&bounding_lines)
    }

    /// Like Java, this returns 0 for every polygon (the guard `dimension() <= 2` is always true).
    pub fn area(&self) -> f64 {
        if PolylineShapeImpl::dimension(self) <= 2 {
            return 0.0;
        }
        let corners = &self.corners;
        let n = corners.len();
        let mut result = 0.0;
        let mut prev_corner = corners[n - 2].to_float();
        let mut current_corner = corners[n - 1].to_float();
        for c in corners.iter() {
            let next_corner = c.to_float();
            result += current_corner.x * (next_corner.y - prev_corner.y);
            prev_corner = current_corner;
            current_corner = next_corner;
        }
        0.5 * result.abs()
    }

    pub fn nearest_point_approx(&self, from_point: &FloatPoint) -> Option<FloatPoint> {
        let mut min_dist = f64::MAX;
        let mut result = None;
        for s in self.pieces() {
            if let Some(current_nearest_point) = s.nearest_point_approx(from_point) {
                let current_distance = current_nearest_point.distance_square(from_point);
                if current_distance < min_dist {
                    min_dist = current_distance;
                    result = Some(current_nearest_point);
                }
            } else {
                panic!("NullPointerException: nearestPointApprox returned null");
            }
        }
        result
    }

    pub fn turn_90_degree(&self, factor: i32, pole: &IntPoint) -> PolygonShape {
        let pole = Point::Int(*pole);
        let new_corners: Vec<Point> = self
            .corners
            .iter()
            .map(|c| c.turn_90_degree(factor, &pole))
            .collect();
        PolygonShape::from_points(&new_corners)
    }

    pub fn rotate_approx(&self, angle: f64, pole: &FloatPoint) -> PolygonShape {
        if angle == 0.0 {
            return self.clone();
        }
        let new_corners: Vec<Point> = self
            .corners
            .iter()
            .map(|c| Point::Int(c.to_float().rotate(angle, pole).round()))
            .collect();
        PolygonShape::from_points(&new_corners)
    }

    pub fn mirror_vertical(&self, pole: &IntPoint) -> PolygonShape {
        let pole = Point::Int(*pole);
        let new_corners: Vec<Point> = self
            .corners
            .iter()
            .map(|c| c.mirror_vertical(&pole))
            .collect();
        PolygonShape::from_points(&new_corners)
    }

    pub fn mirror_horizontal(&self, pole: &IntPoint) -> PolygonShape {
        let pole = Point::Int(*pole);
        let new_corners: Vec<Point> = self
            .corners
            .iter()
            .map(|c| c.mirror_horizontal(&pole))
            .collect();
        PolygonShape::from_points(&new_corners)
    }

    /// Splits this polygon shape into convex pieces (not exact, rounded intersections are used).
    /// Returns None if the split failed (e.g. self intersections). Uses `java.util.Random` with
    /// the fixed seed 99, like Java, to get reproducible results.
    pub fn split_to_convex(&self) -> Option<Vec<TileShape>> {
        if let Some(p) = self.cache.convex_pieces.get() {
            return Some(p.clone());
        }
        let mut random_generator = JavaRandom::new(SEED);
        let convex_pieces = self.split_to_convex_recu(&mut random_generator)?;
        let pieces: Vec<TileShape> = convex_pieces
            .iter()
            .map(|p| TileShape::get_instance_points(&p.corners))
            .collect();
        Some(self.cache.convex_pieces.get_or_init(|| pieces).clone())
    }

    /// Private recursive part of split_to_convex.
    fn split_to_convex_recu(&self, random_generator: &mut JavaRandom) -> Option<Vec<PolygonShape>> {
        let corners = &self.corners;
        let n = corners.len();
        // start with a hashed corner and search the first concave corner
        let mut start_corner_no = random_generator.next_int(n as i32) as usize;
        let mut current_corner = &corners[start_corner_no];
        let mut prev_corner = if start_corner_no != 0 {
            &corners[start_corner_no - 1]
        } else {
            &corners[n - 1]
        };
        // search for the next concave corner from here
        let mut concave_corner_no: i32 = -1;
        for _ in 0..n {
            let next_corner = if start_corner_no < n - 1 {
                &corners[start_corner_no + 1]
            } else {
                &corners[0]
            };
            if next_corner.side_of(prev_corner, current_corner) == Side::OnTheRight {
                // concave corner found
                concave_corner_no = start_corner_no as i32;
                break;
            }
            prev_corner = current_corner;
            current_corner = next_corner;
            start_corner_no = (start_corner_no + 1) % n;
        }
        if concave_corner_no < 0 {
            // no concave corner found, this shape is already convex
            return Some(vec![self.clone()]);
        }
        let concave_corner_no = concave_corner_no as usize;
        let d = DivisionPoint::new(corners, concave_corner_no);
        let projection = d.projection?; // projection not found, maybe polygon has selfintersections

        // construct the result pieces from polygon and the division point
        let n_i = n as i32;
        let mut corner_count: i32 = d.corner_no_after_projection as i32 - concave_corner_no as i32;
        if corner_count < 0 {
            corner_count += n_i;
        }
        corner_count += 1;
        let mut first_arr: Vec<Point> = Vec::with_capacity(corner_count as usize);
        let mut corner_ind = concave_corner_no;
        for _ in 0..corner_count - 1 {
            first_arr.push(corners[corner_ind].clone());
            corner_ind = (corner_ind + 1) % n;
        }
        first_arr.push(Point::Int(projection.round()));
        let mut corner_count: i32 = concave_corner_no as i32 - d.corner_no_after_projection as i32;
        if corner_count < 0 {
            corner_count += n_i;
        }
        corner_count += 2;
        let mut last_arr: Vec<Point> = Vec::with_capacity(corner_count as usize);
        last_arr.push(Point::Int(projection.round()));
        let mut corner_ind = d.corner_no_after_projection;
        for _ in 1..corner_count {
            last_arr.push(corners[corner_ind].clone());
            corner_ind = (corner_ind + 1) % n;
        }
        let last_piece = PolygonShape::from_points(&last_arr);
        let first_piece = PolygonShape::from_points(&first_arr);
        let mut result = first_piece.split_to_convex_recu(random_generator)?;
        let c2 = last_piece.split_to_convex_recu(random_generator)?;
        result.extend(c2);
        Some(result)
    }
}

/// At a concave corner of the closed polygon, a minimal axis parallel division line is
/// constructed, to divide the closed polygon into two.
struct DivisionPoint {
    corner_no_after_projection: usize,
    projection: Option<FloatPoint>,
}

impl DivisionPoint {
    fn new(corners: &[Point], concave_corner_no: usize) -> DivisionPoint {
        let n = corners.len();
        let concave_corner = corners[concave_corner_no].to_float();
        let before_concave_corner = if concave_corner_no != 0 {
            corners[concave_corner_no - 1].to_float()
        } else {
            corners[n - 1].to_float()
        };
        let after_concave_corner = if concave_corner_no == n - 1 {
            corners[0].to_float()
        } else {
            corners[concave_corner_no + 1].to_float()
        };

        let search_right =
            before_concave_corner.y > concave_corner.y || concave_corner.y > after_concave_corner.y;
        let search_left =
            before_concave_corner.y < concave_corner.y || concave_corner.y < after_concave_corner.y;
        let search_up =
            before_concave_corner.x < concave_corner.x || concave_corner.x < after_concave_corner.x;
        let search_down =
            before_concave_corner.x > concave_corner.x || concave_corner.x > after_concave_corner.x;

        let mut min_projection_dist = INT_MAX_F64;
        let mut min_projection: Option<FloatPoint> = None;
        let mut corner_no_after_min_projection = 0usize;

        let mut corner_no_after_curr_projection = (concave_corner_no + 2) % n;
        let mut corner_before_curr_projection = if corner_no_after_curr_projection != 0 {
            &corners[corner_no_after_curr_projection - 1]
        } else {
            &corners[n - 1]
        };
        let mut corner_before_projection_approx = corner_before_curr_projection.to_float();

        let loop_end = n as i32 - 2;
        for _ in 0..loop_end {
            let corner_after_curr_projection = &corners[corner_no_after_curr_projection];
            let corner_after_projection_approx = corner_after_curr_projection.to_float();
            if corner_before_projection_approx.y != corner_after_projection_approx.y {
                // try a horizontal division
                let (min_y, max_y) =
                    if corner_after_projection_approx.y > corner_before_projection_approx.y {
                        (
                            corner_before_projection_approx.y,
                            corner_after_projection_approx.y,
                        )
                    } else {
                        (
                            corner_after_projection_approx.y,
                            corner_before_projection_approx.y,
                        )
                    };
                if concave_corner.y >= min_y && concave_corner.y <= max_y {
                    let current_line = Line::new(
                        corner_before_curr_projection.clone(),
                        corner_after_curr_projection.clone(),
                    );
                    let x_intersection = current_line.function_in_y_value_approx(concave_corner.y);
                    let current_distance = (x_intersection - concave_corner.x).abs();
                    // Make sure, that the new shape will not be concave at the projection point.
                    // That might happen, if the boundary curve runs back in itself.
                    let projection_ok = current_distance < min_projection_dist
                        && (search_right
                            && x_intersection > concave_corner.x
                            && concave_corner.y <= corner_after_projection_approx.y
                            || search_left
                                && x_intersection < concave_corner.x
                                && concave_corner.y >= corner_after_projection_approx.y);
                    if projection_ok {
                        min_projection_dist = current_distance;
                        corner_no_after_min_projection = corner_no_after_curr_projection;
                        min_projection = Some(FloatPoint::new(x_intersection, concave_corner.y));
                    }
                }
            }
            if corner_before_projection_approx.x != corner_after_projection_approx.x {
                // try a vertical division
                let (min_x, max_x) =
                    if corner_after_projection_approx.x > corner_before_projection_approx.x {
                        (
                            corner_before_projection_approx.x,
                            corner_after_projection_approx.x,
                        )
                    } else {
                        (
                            corner_after_projection_approx.x,
                            corner_before_projection_approx.x,
                        )
                    };
                if concave_corner.x >= min_x && concave_corner.x <= max_x {
                    let current_line = Line::new(
                        corner_before_curr_projection.clone(),
                        corner_after_curr_projection.clone(),
                    );
                    let y_intersection = current_line.function_value_approx(concave_corner.x);
                    let current_distance = (y_intersection - concave_corner.y).abs();
                    // make sure, that the new shape will be convex at the projection point
                    let projection_ok = current_distance < min_projection_dist
                        && (search_up
                            && y_intersection > concave_corner.y
                            && concave_corner.x >= corner_after_projection_approx.x
                            || search_down
                                && y_intersection < concave_corner.y
                                && concave_corner.x <= corner_after_projection_approx.x);
                    if projection_ok {
                        min_projection_dist = current_distance;
                        corner_no_after_min_projection = corner_no_after_curr_projection;
                        min_projection = Some(FloatPoint::new(concave_corner.x, y_intersection));
                    }
                }
            }
            corner_before_curr_projection = corner_after_curr_projection;
            corner_before_projection_approx = corner_after_projection_approx;
            if corner_no_after_curr_projection == n - 1 {
                corner_no_after_curr_projection = 0;
            } else {
                corner_no_after_curr_projection += 1;
            }
        }
        if min_projection_dist == INT_MAX_F64 {
            log::warn!(
                "PolygonShape.DivisionPoint: projection not found for concave corner #{} at ({}, {}) in polygon with {} corners",
                concave_corner_no,
                concave_corner.x,
                concave_corner.y,
                n
            );
        }
        DivisionPoint {
            corner_no_after_projection: corner_no_after_min_projection,
            projection: min_projection,
        }
    }
}

impl PolylineShapeImpl for PolygonShape {
    fn border_line_count(&self) -> i32 {
        self.corners.len() as i32
    }

    /// Panics for an out of range index (Java logs a warning and returns null).
    fn corner(&self, no: i32) -> Point {
        self.corner_opt(no)
            .expect("NullPointerException: PolygonShape.corner out of range")
    }

    /// Panics for an out of range index (Java logs a warning and returns null).
    fn border_line(&self, no: i32) -> Line {
        let n = self.corners.len() as i32;
        if no < 0 || no >= n {
            log::warn!("PolygonShape.borderLine: no out of range");
            panic!("NullPointerException: PolygonShape.borderLine out of range");
        }
        let next_corner = if no == n - 1 {
            &self.corners[0]
        } else {
            &self.corners[no as usize + 1]
        };
        Line::new(self.corners[no as usize].clone(), next_corner.clone())
    }

    fn corner_is_bounded(&self, _no: i32) -> bool {
        true
    }

    fn is_empty(&self) -> bool {
        self.corners.is_empty()
    }

    fn is_bounded(&self) -> bool {
        true
    }

    fn dimension(&self) -> i32 {
        match self.corners.len() {
            0 => -1,
            1 => 0,
            2 => 1,
            _ => 2,
        }
    }

    fn bounding_box(&self) -> IntBox {
        PolygonShape::bounding_box(self)
    }
}
