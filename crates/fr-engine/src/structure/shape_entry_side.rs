//! Port of `board/model/structure/ShapeEntrySide.java` and `ShapeAndEntrySide.java`.
//!
//! `ShapeAndEntrySide(PolylineTrace trace, int index, ...)` read the tree shape and the
//! compensated half width from the trace and the board's default search tree; here these are
//! passed in explicitly (see [`ShapeAndEntrySide::new`]).

use fr_geom::{FloatLine, FloatPoint, Line, LineSegment, Point, Polyline, Side, TileShape};

/// The border side of a tile shape where a trace enters (Java `ShapeEntrySide`).
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeEntrySide {
    /// Border line index, -1 if not calculated.
    pub no: i32,
    /// Approximate intersection with the border; `None` for Java `null`.
    pub border_intersection: Option<FloatPoint>,
}

impl ShapeEntrySide {
    /// Java `ShapeEntrySide.NOT_CALCULATED`.
    pub const NOT_CALCULATED: ShapeEntrySide = ShapeEntrySide {
        no: -1,
        border_intersection: None,
    };

    /// Values already calculated (Java `ShapeEntrySide(int, FloatPoint)`).
    pub fn new(no: i32, border_intersection: Option<FloatPoint>) -> Self {
        ShapeEntrySide {
            no,
            border_intersection,
        }
    }

    /// Number of the edge line of `shape` where `polyline` enters. Used in the push trace
    /// algorithm; `no` is expected between 1 and `polyline.lines.len() - 2` inclusive.
    pub fn from_polyline(polyline: &Polyline, no: i32, shape: &TileShape) -> Self {
        let mut fromside_no = -1;
        let mut intersection: Option<FloatPoint> = None;
        let mut border_intersection_found = false;
        let mut current_no = no;
        while current_no > 0 {
            let current_seg = LineSegment::from_polyline(polyline, current_no)
                .expect("ShapeEntrySide: line segment index out of range");
            let intersections = current_seg.border_intersections(shape);
            if !intersections.is_empty() {
                fromside_no = intersections[0];
                intersection = Some(
                    current_seg
                        .get_line()
                        .intersection_approx(&shape.border_line(fromside_no)),
                );
                border_intersection_found = true;
                break;
            }
            current_no -= 1;
        }
        if !border_intersection_found {
            // The first corner of polyline is inside shape: take the nearest intersection of
            // polyline.lines[1] with the border of shape to the first corner of polyline.
            let from_point = polyline.corner_approx(0);
            let check_line = &polyline.lines[1];
            let mut min_dist = f64::MAX;
            let edge_count = shape.border_line_count();
            for i in 0..edge_count {
                let current_line = shape.border_line(i);
                let current_intersection = check_line.intersection_approx(&current_line);
                let current_distance = current_intersection.distance(&from_point).abs();
                if current_distance < min_dist {
                    fromside_no = i;
                    intersection = Some(current_intersection);
                    min_dist = current_distance;
                }
            }
        }
        ShapeEntrySide {
            no: fromside_no,
            border_intersection: intersection,
        }
    }

    /// The nearest border side of `shape` to `from_point`. Used in the shove drill item
    /// algorithm.
    pub fn from_point(from_point: &Point, shape: &TileShape) -> Self {
        let border_projection = shape
            .nearest_border_point(from_point)
            .expect("ShapeEntrySide: nearest border point is null");
        let no = shape.contains_on_border_line_no(&border_projection);
        if no < 0 {
            log::warn!("CalcFromSide: this.no >= 0 expected");
        }
        ShapeEntrySide {
            no,
            border_intersection: Some(border_projection.to_float()),
        }
    }

    /// The side of `shape` at the start of `line_segment`, moved by 2 sides to the left
    /// (`shove_to_the_left`) or right. Used by the trace shover.
    pub fn from_line_segment(
        line_segment: &LineSegment,
        shape: &TileShape,
        shove_to_the_left: bool,
    ) -> Self {
        let start_corner = line_segment.start_point_approx();
        let end_corner = line_segment.end_point_approx();
        let border_line_count = shape.border_line_count();
        let check_line = line_segment.get_line();
        let first_corner = shape.corner_approx(0);
        let mut prev_side: Side = check_line.side_of_float(&first_corner);
        let mut front_side_no = -1;

        for i in 1..=border_line_count {
            let next_corner = if i == border_line_count {
                first_corner
            } else {
                shape.corner_approx(i)
            };
            let next_side = check_line.side_of_float(&next_corner);
            if prev_side != next_side {
                let current_intersection = shape.border_line(i - 1).intersection_approx(check_line);
                if current_intersection.distance_square(&start_corner)
                    < current_intersection.distance_square(&end_corner)
                {
                    front_side_no = i - 1;
                    break;
                }
            }
            prev_side = next_side;
        }
        let no;
        if front_side_no < 0 {
            // Fallback: the nearest side of the shape to the start point of the line segment.
            // (Java also assigns intermediate border intersections here, which are always
            // overwritten below.)
            let start_corner = line_segment.start_point_approx();
            let mut min_distance = f64::MAX;
            let mut nearest_side = 0;
            for i in 0..border_line_count {
                let bl = shape.border_line(i);
                let border_line = FloatLine::new(bl.a.to_float(), bl.b.to_float());
                let projection = border_line.perpendicular_projection(&start_corner);
                let side_start = shape.corner_approx(i);
                let side_end = shape.corner_approx((i + 1) % border_line_count);
                if projection.is_contained_in_box(&side_start, &side_end, 0.01) {
                    let distance = start_corner.distance(&projection);
                    if distance < min_distance {
                        min_distance = distance;
                        nearest_side = i;
                    }
                }
            }
            no = if shove_to_the_left {
                (nearest_side + 2) % border_line_count
            } else {
                (nearest_side + border_line_count - 2) % border_line_count
            };
        } else if shove_to_the_left {
            no = (front_side_no + 2) % border_line_count;
        } else {
            no = (front_side_no + border_line_count - 2) % border_line_count;
        }
        let prev_corner = shape.corner_approx(no);
        let next_corner = shape.corner_approx((no + 1) % border_line_count);
        ShapeEntrySide {
            no,
            border_intersection: Some(prev_corner.middle_point(&next_corner)),
        }
    }
}

/// Shape of a trace segment plus its entry side, used in the shove algorithm to calculate the
/// from-side for pushing and to cut off dog ears of the trace shape (Java `ShapeAndEntrySide`).
#[derive(Clone, Debug)]
pub struct ShapeAndEntrySide {
    pub shape: TileShape,
    pub from_side: Option<ShapeEntrySide>,
}

impl ShapeAndEntrySide {
    /// Java `ShapeAndEntrySide(PolylineTrace trace, int index, boolean orthogonal, boolean
    /// inShoveCheck)` with the trace data passed explicitly:
    /// * `tree_shape` = `trace.getTreeShape(board.searchTreeManager.getDefaultTree(), index)`,
    /// * `polyline` = `trace.polyline()`,
    /// * `compensated_half_width` = `trace.getCompensatedHalfWidth(defaultTree)`.
    pub fn new(
        tree_shape: &TileShape,
        polyline: &Polyline,
        compensated_half_width: i32,
        index: i32,
        orthogonal: bool,
        in_shove_check: bool,
    ) -> Self {
        let mut current_shape: TileShape;
        let mut current_from_side: Option<ShapeEntrySide> = None;
        let mut cut_off_at_start = false;
        let mut cut_off_at_end = false;
        if orthogonal {
            current_shape = TileShape::IntBox(tree_shape.bounding_box());
        } else {
            // prevent dog ears at the start and the end of the substitute trace
            current_shape = TileShape::Simplex(tree_shape.to_simplex());
            let end_cutline = calc_cutline_at_end(index, polyline, compensated_half_width);
            if let Some(end_cutline) = &end_cutline {
                let cut_plane = TileShape::get_instance_line(end_cutline);
                let tmp_shape = current_shape.intersection(&cut_plane);
                // Java `tmpShape != currentShape`: the simplex intersection always creates a
                // new object, so only the emptiness test matters.
                if !tmp_shape.is_empty() {
                    current_shape = TileShape::Simplex(tmp_shape.to_simplex());
                    cut_off_at_end = true;
                }
            }
            let start_cutline = calc_cutline_at_start(index, polyline, compensated_half_width);
            if let Some(start_cutline) = &start_cutline {
                let cut_plane = TileShape::get_instance_line(start_cutline);
                let tmp_shape = current_shape.intersection(&cut_plane);
                if !tmp_shape.is_empty() {
                    current_shape = TileShape::Simplex(tmp_shape.to_simplex());
                    cut_off_at_start = true;
                }
            }
            let mut from_side_index = -1;
            let mut current_cut_line: Option<&Line> = None;
            if cut_off_at_start {
                current_cut_line = start_cutline.as_ref();
                from_side_index = current_shape.border_line_index(current_cut_line.unwrap());
            }
            if from_side_index < 0 && cut_off_at_end {
                current_cut_line = end_cutline.as_ref();
                from_side_index = current_shape.border_line_index(current_cut_line.unwrap());
            }
            if from_side_index >= 0 {
                let border_intersection = current_cut_line
                    .unwrap()
                    .intersection_approx(&current_shape.border_line(from_side_index));
                current_from_side = Some(ShapeEntrySide::new(
                    from_side_index,
                    Some(border_intersection),
                ));
            }
        }
        if current_from_side.is_none() && !in_shove_check {
            // In the shove check this calculation may produce an undesired stack level > 1 in
            // ShapeTraceEntries.
            current_from_side = Some(ShapeEntrySide::from_polyline(
                polyline,
                index,
                &current_shape,
            ));
        }
        ShapeAndEntrySide {
            shape: current_shape,
            from_side: current_from_side,
        }
    }
}

fn calc_cutline_at_end(
    index: i32,
    trace_lines: &Polyline,
    compensated_half_width: i32,
) -> Option<Line> {
    let len = trace_lines.lines.len() as i32;
    if index == len - 3
        || trace_lines
            .corner_approx(len - 2)
            .distance(&trace_lines.corner_approx(index + 1))
            < compensated_half_width as f64
    {
        let current_line = &trace_lines.lines[(len - 1) as usize];
        let is = trace_lines.corner_approx(len - 3);
        let cut_line = if current_line.side_of_float(&is) == Side::OnTheLeft {
            current_line.opposite()
        } else {
            current_line.clone()
        };
        return Some(cut_line);
    }
    None
}

fn calc_cutline_at_start(
    index: i32,
    trace_lines: &Polyline,
    compensated_half_width: i32,
) -> Option<Line> {
    if index == 0
        || trace_lines
            .corner_approx(0)
            .distance(&trace_lines.corner_approx(index))
            < compensated_half_width as f64
    {
        let current_line = &trace_lines.lines[0];
        let is = trace_lines.corner_approx(1);
        let cut_line = if current_line.side_of_float(&is) == Side::OnTheLeft {
            current_line.opposite()
        } else {
            current_line.clone()
        };
        return Some(cut_line);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use fr_geom::{IntBox, IntPoint};

    fn ip(x: i32, y: i32) -> Point {
        Point::Int(IntPoint::new(x, y))
    }

    #[test]
    fn from_point_nearest_side() {
        let shape = TileShape::IntBox(IntBox::new(0, 0, 100, 50));
        let s = ShapeEntrySide::from_point(&ip(50, 45), &shape);
        assert!(s.no >= 0);
        assert_eq!(shape.border_line(s.no).a.to_float().y, 50.0);
        assert_eq!(s.border_intersection, Some(FloatPoint::new(50.0, 50.0)));
    }

    #[test]
    fn from_polyline_enters() {
        // horizontal polyline from (-50, 25) to (50, 25) entering the box through its left side
        let pl = Polyline::from_points(&[ip(-50, 25), ip(50, 25)]);
        let shape = TileShape::IntBox(IntBox::new(0, 0, 100, 50));
        let s = ShapeEntrySide::from_polyline(&pl, 1, &shape);
        assert!(s.no >= 0);
        let bl = shape.border_line(s.no);
        assert_eq!(bl.a.to_float().x, 0.0);
        assert_eq!(bl.b.to_float().x, 0.0);
        assert_eq!(s.border_intersection, Some(FloatPoint::new(0.0, 25.0)));
    }

    #[test]
    fn shape_and_entry_side_orthogonal() {
        let pl = Polyline::from_points(&[ip(-50, 25), ip(50, 25)]);
        let tree_shape = TileShape::IntBox(IntBox::new(-60, 15, 60, 35));
        let r = ShapeAndEntrySide::new(&tree_shape, &pl, 10, 1, true, true);
        assert_eq!(r.shape.bounding_box(), IntBox::new(-60, 15, 60, 35));
        assert!(r.from_side.is_none());
        let r = ShapeAndEntrySide::new(&tree_shape, &pl, 10, 1, true, false);
        assert!(r.from_side.is_some());
    }
}
