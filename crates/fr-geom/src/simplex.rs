//! Port of `Simplex.java`: convex shape defined as intersection of half-planes. A half-plane is
//! the positive (right) side of a directed line.

use std::sync::{Arc, OnceLock};

use crate::circle::Circle;
use crate::direction::Direction;
use crate::float_point::{java_max, java_min, FloatPoint};
use crate::int_box::IntBox;
use crate::int_direction::IntDirection;
use crate::int_octagon::IntOctagon;
use crate::int_point::IntPoint;
use crate::java_compat::{INT_MAX_F64, INT_MIN_F64};
use crate::limits::CRIT_INT;
use crate::line::Line;
use crate::point::Point;
use crate::regular_tile_shape::RegularTileShape;
use crate::shape_bounding_directions::ShapeBoundingDirections;
use crate::side::Side;
use crate::tile_shape::{TileShape, TileShapeImpl};
use crate::vector::Vector;

#[derive(Default, Debug)]
struct SimplexCache {
    corners: OnceLock<Box<[OnceLock<Point>]>>,
    float_corners: OnceLock<Box<[OnceLock<FloatPoint>]>>,
    bounding_box: OnceLock<IntBox>,
    bounding_octagon: OnceLock<IntOctagon>,
}

/// Cloning is cheap and shares the lazily computed caches (like a Java reference).
#[derive(Clone)]
pub struct Simplex {
    lines: Arc<[Line]>,
    cache: Arc<SimplexCache>,
}

impl std::fmt::Debug for Simplex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Simplex")
            .field("lines", &self.lines)
            .finish()
    }
}

impl Simplex {
    /// Constructs a Simplex from the directed lines. The simplex is not normalized; use
    /// `TileShape::get_instance_lines` or `Simplex::get_instance` for a normalized simplex.
    pub fn new(lines: Vec<Line>) -> Simplex {
        Simplex {
            lines: lines.into(),
            cache: Arc::new(SimplexCache::default()),
        }
    }

    fn from_arc(lines: Arc<[Line]>) -> Simplex {
        Simplex {
            lines,
            cache: Arc::new(SimplexCache::default()),
        }
    }

    /// Standard implementation for an empty Simplex (`Simplex.EMPTY`).
    pub fn empty() -> Simplex {
        Simplex::new(Vec::new())
    }

    /// The border lines of this simplex.
    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// Creates a Simplex as intersection of the half-planes defined by directed lines.
    pub fn get_instance(lines: &[Line]) -> Simplex {
        if lines.is_empty() {
            return Simplex::empty();
        }
        let mut current_arr: Vec<Line> = lines.to_vec();
        // sort the lines in ascending direction (exact port of Java's TimSort, see java_sort)
        crate::java_sort::sort_by(&mut current_arr, |a, b| a.compare_to(b));
        Simplex::new(current_arr).remove_redundant_lines()
    }

    /// Return true, if this simplex is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Converts to a simpler physical instance (IntBox or IntOctagon), if possible.
    pub fn simplify(&self) -> TileShape {
        if self.is_empty() {
            TileShape::Simplex(Simplex::empty())
        } else if self.is_int_box() {
            TileShape::IntBox(self.bounding_box())
        } else if self.is_int_octagon() {
            TileShape::IntOctagon(self.to_int_octagon().expect("is_int_octagon"))
        } else {
            TileShape::Simplex(self.clone())
        }
    }

    pub fn get_id(&self) -> i32 {
        let mut result: i32 = 0;
        for current in self.lines.iter() {
            result = result.wrapping_mul(31).wrapping_add(current.get_id());
        }
        result
    }

    /// Returns true, if the determinant of the direction of index no - 1 and the direction of
    /// index no is > 0.
    pub fn corner_is_bounded(&self, corner_index: i32) -> bool {
        let len = self.lines.len() as i32;
        let no = if corner_index < 0 {
            log::warn!("corner: no is < 0");
            0
        } else if corner_index >= len {
            log::warn!("corner: index must be less than lines.length - 1");
            len - 1
        } else {
            corner_index
        };
        if len == 1 {
            return false;
        }
        let prev_no = if no == 0 { len - 1 } else { no - 1 };
        let prev_dir = self.lines[prev_no as usize]
            .direction_ref()
            .get_vector()
            .as_int();
        let current_direction = self.lines[no as usize]
            .direction_ref()
            .get_vector()
            .as_int();
        prev_dir.determinant(&current_direction) > 0
    }

    /// Returns true, if the shape of this simplex is contained in a sufficiently large box.
    pub fn is_bounded(&self) -> bool {
        let len = self.lines.len() as i32;
        if len == 0 {
            return true;
        }
        if len < 3 {
            return false;
        }
        (0..len).all(|i| self.corner_is_bounded(i))
    }

    /// Returns the number of edge lines defining this simplex.
    #[inline]
    pub fn border_line_count(&self) -> i32 {
        self.lines.len() as i32
    }

    /// Returns the intersection of the (no - 1)-th with the no-th line of this simplex.
    pub fn corner(&self, corner_index: i32) -> Point {
        let len = self.lines.len() as i32;
        let no = if corner_index < 0 {
            log::warn!("Simplex.corner: no is < 0");
            0
        } else if corner_index >= len {
            log::warn!("Simplex.corner: no must be less than lines.length - 1");
            len - 1
        } else {
            corner_index
        } as usize;
        let arr = self
            .cache
            .corners
            .get_or_init(|| (0..self.lines.len()).map(|_| OnceLock::new()).collect());
        arr[no]
            .get_or_init(|| {
                let prev = if no == 0 {
                    &self.lines[self.lines.len() - 1]
                } else {
                    &self.lines[no - 1]
                };
                self.lines[no].intersection(prev)
            })
            .clone()
    }

    /// Approximation of the intersection of the (no - 1)-th with the no-th line. Panics for an
    /// empty simplex (Java returns null).
    pub fn corner_approx(&self, corner_index: i32) -> FloatPoint {
        let len = self.lines.len() as i32;
        if len == 0 {
            panic!("Simplex.corner_approx: simplex is empty");
        }
        let no = if corner_index < 0 {
            log::warn!("Simplex.corner_approx: no is < 0");
            0
        } else if corner_index >= len {
            log::warn!("Simplex.corner_approx: no must be less than lines.length - 1");
            len - 1
        } else {
            corner_index
        } as usize;
        self.float_corner(no)
    }

    #[inline]
    fn float_corner(&self, no: usize) -> FloatPoint {
        let arr = self
            .cache
            .float_corners
            .get_or_init(|| (0..self.lines.len()).map(|_| OnceLock::new()).collect());
        *arr[no].get_or_init(|| {
            let prev = if no == 0 {
                &self.lines[self.lines.len() - 1]
            } else {
                &self.lines[no - 1]
            };
            self.lines[no].intersection_approx(prev)
        })
    }

    pub fn corner_approx_arr(&self) -> Vec<FloatPoint> {
        (0..self.lines.len())
            .map(|i| self.float_corner(i))
            .collect()
    }

    /// Returns the no-th edge line of this simplex. The edge lines are sorted in ascending
    /// direction. Panics for an empty simplex (Java returns null).
    pub fn border_line(&self, edge_index: i32) -> Line {
        let len = self.lines.len() as i32;
        if len == 0 {
            panic!("Simplex.borderLine : simplex is empty");
        }
        let no = if edge_index < 0 {
            log::warn!("Simplex.borderLine : no is < 0");
            0
        } else if edge_index >= len {
            log::warn!("Simplex.borderLine: no must be less than lines.length - 1");
            len - 1
        } else {
            edge_index
        };
        self.lines[no as usize].clone()
    }

    /// Returns the dimension of this simplex: 2, 1, 0, or -1 (if the simplex is empty).
    pub fn dimension(&self) -> i32 {
        let lines = &self.lines;
        match lines.len() {
            0 => -1,
            n if n > 4 => 2,
            1 => 2, // a half plane
            2 => {
                if lines[0].overlaps(&lines[1]) {
                    1
                } else {
                    2
                }
            }
            3 => {
                if lines[0].overlaps(&lines[1])
                    || lines[0].overlaps(&lines[2])
                    || lines[1].overlaps(&lines[2])
                {
                    // simplex is 1 dimensional and unbounded at one side
                    return 1;
                }
                let intersection = lines[1].intersection(&lines[2]);
                let side_of_line0 = lines[0].side_of(&intersection);
                if side_of_line0 == Side::OnTheRight {
                    return 2;
                }
                if side_of_line0 == Side::OnTheLeft {
                    log::debug!("empty Simplex not normalized");
                    return -1;
                }
                // now the 3 lines intersect in the same point
                0
            }
            _ => {
                // now the simplex has 4 edge lines; check if opposing lines are collinear
                let collinear02 = lines[0].overlaps(&lines[2]);
                let collinear13 = lines[1].overlaps(&lines[3]);
                if collinear02 && collinear13 {
                    return 0;
                }
                if collinear02 || collinear13 {
                    return 1;
                }
                2
            }
        }
    }

    pub fn max_width(&self) -> f64 {
        if !self.is_bounded() {
            return i32::MAX as f64;
        }
        let mut max_distance = INT_MIN_F64;
        let mut max_distance2 = INT_MIN_F64;
        let gravity_point = crate::polyline_shape::PolylineShapeImpl::centre_of_gravity(self);
        for line in self.lines.iter() {
            let current_distance = line.signed_distance(&gravity_point).abs();
            if current_distance > max_distance {
                max_distance2 = max_distance;
                max_distance = current_distance;
            } else if current_distance > max_distance2 {
                max_distance2 = current_distance;
            }
        }
        max_distance + max_distance2
    }

    pub fn min_width(&self) -> f64 {
        if !self.is_bounded() {
            return i32::MAX as f64;
        }
        let mut min_distance = INT_MAX_F64;
        let mut min_distance2 = INT_MAX_F64;
        let gravity_point = crate::polyline_shape::PolylineShapeImpl::centre_of_gravity(self);
        for line in self.lines.iter() {
            let current_distance = line.signed_distance(&gravity_point).abs();
            if current_distance < min_distance {
                min_distance2 = min_distance;
                min_distance = current_distance;
            } else if current_distance < min_distance2 {
                min_distance2 = current_distance;
            }
        }
        min_distance + min_distance2
    }

    /// Checks if this simplex can be converted into an IntBox.
    pub fn is_int_box(&self) -> bool {
        for (i, current_line) in self.lines.iter().enumerate() {
            if !(current_line.a.is_int_point() && current_line.b.is_int_point()) {
                return false;
            }
            if !current_line.is_orthogonal() {
                return false;
            }
            if !self.corner_is_bounded(i as i32) {
                return false;
            }
        }
        true
    }

    /// Checks if this simplex can be converted into an IntOctagon.
    pub fn is_int_octagon(&self) -> bool {
        for (i, current_line) in self.lines.iter().enumerate() {
            if !(current_line.a.is_int_point() && current_line.b.is_int_point()) {
                return false;
            }
            if !current_line.is_multiple_of_45_degree() {
                return false;
            }
            if !self.corner_is_bounded(i as i32) {
                return false;
            }
        }
        true
    }

    /// Converts this simplex to an IntOctagon. Returns None, if that is not possible, because
    /// not all lines are multiples of 45 degree.
    pub fn to_int_octagon(&self) -> Option<IntOctagon> {
        if !self.is_int_octagon() {
            return None;
        }
        if self.is_empty() {
            return Some(IntOctagon::EMPTY);
        }
        // initialise to the biggest octagon values
        let mut rx = CRIT_INT;
        let mut uy = CRIT_INT;
        let mut lrx = CRIT_INT;
        let mut urx = CRIT_INT;
        let mut lx = -CRIT_INT;
        let mut ly = -CRIT_INT;
        let mut llx = -CRIT_INT;
        let mut ulx = -CRIT_INT;
        for current_line in self.lines.iter() {
            let a = current_line.a.as_int();
            let b = current_line.b.as_int();
            if a.y == b.y {
                if b.x >= a.x {
                    // lower boundary line
                    ly = a.y;
                }
                if b.x <= a.x {
                    // upper boundary line
                    uy = a.y;
                }
            }
            if a.x == b.x {
                if b.y >= a.y {
                    // right boundary line
                    rx = a.x;
                }
                if b.y <= a.y {
                    // left boundary line
                    lx = a.x;
                }
            }
            if a.y < b.y {
                if a.x < b.x {
                    // lower right boundary line
                    lrx = a.x.wrapping_sub(a.y);
                } else if a.x > b.x {
                    // upper right boundary line
                    urx = a.x.wrapping_add(a.y);
                }
            } else if a.y > b.y {
                if a.x < b.x {
                    // lower left boundary line
                    llx = a.x.wrapping_add(a.y);
                } else if a.x > b.x {
                    // upper left boundary line
                    ulx = a.x.wrapping_sub(a.y);
                }
            }
        }
        Some(IntOctagon::new(lx, ly, rx, uy, ulx, lrx, llx, urx).normalize())
    }

    /// Returns the simplex that results from translating its lines by vector.
    pub fn translate_by(&self, vector: &Vector) -> Simplex {
        if *vector == Vector::ZERO {
            return self.clone();
        }
        Simplex::new(self.lines.iter().map(|l| l.translate_by(vector)).collect())
    }

    /// Returns the smallest box with int coordinates containing all corners of this simplex. The
    /// coordinates are Integer.MAX_VALUE if the simplex is not bounded.
    pub fn bounding_box(&self) -> IntBox {
        if self.lines.is_empty() {
            return IntBox::EMPTY;
        }
        *self.cache.bounding_box.get_or_init(|| {
            let mut llx = INT_MAX_F64;
            let mut lly = INT_MAX_F64;
            let mut urx = INT_MIN_F64;
            let mut ury = INT_MIN_F64;
            for i in 0..self.lines.len() {
                let current = self.float_corner(i);
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

    /// Calculates a bounding octagon of the Simplex. Returns None, if the Simplex is not
    /// bounded.
    pub fn bounding_octagon(&self) -> Option<IntOctagon> {
        if let Some(o) = self.cache.bounding_octagon.get() {
            return Some(*o);
        }
        let mut lx = INT_MAX_F64;
        let mut ly = INT_MAX_F64;
        let mut rx = INT_MIN_F64;
        let mut uy = INT_MIN_F64;
        let mut ulx = INT_MAX_F64;
        let mut lrx = INT_MIN_F64;
        let mut llx = INT_MAX_F64;
        let mut urx = INT_MIN_F64;
        for i in 0..self.lines.len() {
            let current = self.float_corner(i);
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
        let crit = CRIT_INT as f64;
        if java_min(lx, ly) < -crit
            || java_max(rx, uy) > crit
            || java_min(ulx, llx) < -crit
            || java_max(lrx, urx) > crit
        {
            // result is not bounded
            return None;
        }
        let result = IntOctagon::new(
            lx.floor() as i32,
            ly.floor() as i32,
            rx.ceil() as i32,
            uy.ceil() as i32,
            ulx.floor() as i32,
            lrx.ceil() as i32,
            llx.floor() as i32,
            urx.ceil() as i32,
        );
        Some(*self.cache.bounding_octagon.get_or_init(|| result))
    }

    pub fn bounding_tile(&self) -> Simplex {
        self.clone()
    }

    /// Java `dirs.bounds(this)`; None if the 45 degree bounding octagon does not exist.
    pub fn bounding_shape(&self, dirs: &ShapeBoundingDirections) -> Option<RegularTileShape> {
        dirs.bounds_simplex(self)
    }

    /// Returns the simplex offsetted by width. If width > 0, the offset is to the outer, else to
    /// the inner.
    pub fn offset(&self, width: f64) -> Simplex {
        if width == 0.0 {
            return self.clone();
        }
        let new_lines: Vec<Line> = self.lines.iter().map(|l| l.translate(-width)).collect();
        let offset_simplex = Simplex::new(new_lines);
        if width < 0.0 {
            return offset_simplex.remove_redundant_lines();
        }
        offset_simplex
    }

    /// Returns this simplex enlarged by offset, intersected with the enlarged bounding octagon.
    pub fn enlarge(&self, offset: f64) -> Simplex {
        if offset == 0.0 {
            return self.clone();
        }
        let offset_simplex = self.offset(offset);
        let bounding_oct = match self.bounding_octagon() {
            Some(o) => o,
            None => return Simplex::empty(),
        };
        let offset_oct = bounding_oct.offset(offset);
        offset_simplex.intersection_simplex(&offset_oct.to_simplex())
    }

    /// Returns the number of the rightmost corner seen from from_point (Java
    /// `indexOfRightMostCorner(Point)`).
    pub fn index_of_right_most_corner_point(&self, from_point: &Point) -> i32 {
        let pole = from_point;
        let mut right_most_corner = self.corner(0);
        let mut result = 0;
        for i in 1..self.lines.len() as i32 {
            let current_corner = self.corner(i);
            if current_corner.side_of(pole, &right_most_corner) == Side::OnTheRight {
                right_most_corner = current_corner;
                result = i;
            }
        }
        result
    }

    /// Returns the intersection of box with this simplex.
    pub fn intersection_int_box(&self, b: &IntBox) -> Simplex {
        self.intersection_simplex(&b.to_simplex())
    }

    /// Package private `intersection(IntOctagon)`.
    pub fn intersection_int_octagon(&self, other: &IntOctagon) -> Simplex {
        self.intersection_simplex(&other.to_simplex())
    }

    /// Returns the intersection of this simplex and other.
    pub fn intersection_simplex(&self, other: &Simplex) -> Simplex {
        if self.is_empty() || other.is_empty() {
            return Simplex::empty();
        }
        let mut new_arr: Vec<Line> = Vec::with_capacity(self.lines.len() + other.lines.len());
        new_arr.extend(self.lines.iter().cloned());
        new_arr.extend(other.lines.iter().cloned());
        crate::java_sort::sort_by(&mut new_arr, |a, b| a.compare_to(b));
        Simplex::new(new_arr).remove_redundant_lines()
    }

    /// Returns the intersection of this simplex and the TileShape other.
    pub fn intersection(&self, other: &TileShape) -> TileShape {
        TileShape::Simplex(self.clone()).intersection(other)
    }

    pub fn intersects_simplex(&self, other: &Simplex) -> bool {
        !self.intersection_simplex(other).is_empty()
    }

    pub fn intersects_int_box(&self, b: &IntBox) -> bool {
        self.intersects_simplex(&b.to_simplex())
    }

    pub fn intersects_int_octagon(&self, octagon: &IntOctagon) -> bool {
        self.intersects_simplex(&octagon.to_simplex())
    }

    pub fn intersects_circle(&self, circle: &Circle) -> bool {
        circle.intersects_simplex(self)
    }

    /// Returns the edge number if line is a border line of this simplex, otherwise -1.
    pub fn border_line_index(&self, line: &Line) -> i32 {
        for (i, l) in self.lines.iter().enumerate() {
            if line == l {
                return i as i32;
            }
        }
        -1
    }

    /// Enlarges the simplex by removing the edge line with index no. The result may get
    /// unbounded.
    pub fn remove_border_line(&self, no: i32) -> Simplex {
        if no < 0 || no >= self.lines.len() as i32 {
            return self.clone();
        }
        let mut new_lines: Vec<Line> = self.lines.to_vec();
        new_lines.remove(no as usize);
        Simplex::new(new_lines)
    }

    pub fn to_simplex(&self) -> Simplex {
        self.clone()
    }

    /// Cuts shape out of this simplex (Java returns null for some cut shapes; see
    /// `cutout_from_simplex`).
    pub fn cutout(&self, shape: &TileShape) -> Option<Vec<TileShape>> {
        shape.cutout_from_simplex(self)
    }

    /// Cuts this simplex out of outer_simplex. Divides the resulting shape into simplices along
    /// the minimal distance lines from the vertices of the inner simplex to the outer simplex.
    /// Returns None if this simplex is not 2-dimensional (Java returns null).
    pub fn cutout_from_simplex(&self, outer_simplex: &Simplex) -> Option<Vec<Simplex>> {
        if self.dimension() < 2 {
            log::warn!("Simplex.cutout_from only implemented for 2-dim simplex");
            return None;
        }
        let inner_simplex = self.intersection_simplex(outer_simplex);
        if inner_simplex.dimension() < 2 {
            // nothing to cutout from outer_simplex
            return Some(vec![outer_simplex.clone()]);
        }
        let inner_corner_count = inner_simplex.lines.len();
        let mut division_line_arr: Vec<Vec<Line>> = Vec::with_capacity(inner_corner_count);
        for inner_corner_no in 0..inner_corner_count {
            match inner_simplex.calc_division_lines(inner_corner_no, outer_simplex) {
                Some(l) => division_line_arr.push(l),
                None => {
                    log::warn!("Simplex.cutout_from: division line is null");
                    return Some(vec![outer_simplex.clone()]);
                }
            }
        }
        let mut check_cross_first_line = false;
        let prev_division_line: Option<Line> = None;
        let first_division_line = division_line_arr[0][0].clone();
        let first_direction: IntDirection = first_division_line.int_direction();
        let mut result_list: Vec<Simplex> = Vec::new();

        for inner_corner_no in 0..inner_corner_count {
            let next_corner_no = (inner_corner_no + 1) % inner_corner_count;
            let next_division_line = division_line_arr[next_corner_no][0].clone();
            let current_division_lines = &division_line_arr[inner_corner_no];
            if current_division_lines.len() == 2 {
                // 2 division lines are necessary (sharp corner). Construct an unbounded simplex
                // from current_division_lines[1] and [0] and intersect it with the outer simplex
                let current_direction = current_division_lines[0].int_direction();
                let mut merge_prev_division_line = false;
                let mut merge_first_division_line = false;
                if let Some(pdl) = &prev_division_line {
                    let prev_dir = pdl.int_direction();
                    if current_direction.determinant(&prev_dir) > 0.0 {
                        // the previous division line may intersect current_division_lines[0]
                        merge_prev_division_line = true;
                    }
                }
                if !check_cross_first_line {
                    check_cross_first_line = inner_corner_no > 0
                        && current_direction.determinant(&first_direction) > 0.0;
                }
                if check_cross_first_line {
                    let current_dir2 = current_division_lines[1].int_direction();
                    if current_dir2.determinant(&first_direction) < 0.0 {
                        // The current piece has an intersection area with the first piece.
                        merge_first_division_line = true;
                    }
                }
                let mut piece_lines: Vec<Line> = Vec::with_capacity(4);
                piece_lines.push(Line::new(
                    current_division_lines[1].b.clone(),
                    current_division_lines[1].a.clone(),
                ));
                piece_lines.push(current_division_lines[0].clone());
                if merge_prev_division_line {
                    piece_lines.push(prev_division_line.clone().expect("checked"));
                }
                if merge_first_division_line {
                    piece_lines.push(Line::new(
                        first_division_line.b.clone(),
                        first_division_line.a.clone(),
                    ));
                }
                let current_piece = Simplex::new(piece_lines);
                result_list.push(current_piece.intersection_simplex(outer_simplex));
            }
            // construct an unbounded simplex from next_division_line, inner_simplex.line
            // [inner_corner_no] and the last current division line and intersect it with the
            // outer simplex
            let merge_next_division_line = next_division_line.b != next_division_line.a;
            let last_curr_division_line =
                current_division_lines[current_division_lines.len() - 1].clone();
            let last_curr_dir = last_curr_division_line.int_direction();
            let merge_last_curr_division_line =
                last_curr_division_line.b != last_curr_division_line.a;
            let mut merge_prev_division_line = false;
            let mut merge_first_division_line = false;
            if let Some(pdl) = &prev_division_line {
                let prev_dir = pdl.int_direction();
                if last_curr_dir.determinant(&prev_dir) > 0.0 {
                    // the previous division line may intersect the last current division line
                    merge_prev_division_line = true;
                }
            }
            if !check_cross_first_line {
                check_cross_first_line = inner_corner_no > 0
                    && last_curr_dir.determinant(&first_direction) > 0.0
                    && last_curr_dir
                        .get_vector()
                        .scalar_product(&first_direction.get_vector())
                        < 0.0;
                // scalar product checked to ignore backcrossing at small inner_corner_no
            }
            if check_cross_first_line {
                let next_dir = next_division_line.int_direction();
                if next_dir.determinant(&first_direction) < 0.0 {
                    // The current piece has an intersection area with the first piece.
                    merge_first_division_line = true;
                }
            }
            let mut piece_lines: Vec<Line> = Vec::with_capacity(5);
            let current_line = &inner_simplex.lines[inner_corner_no];
            piece_lines.push(Line::new(current_line.b.clone(), current_line.a.clone()));
            if merge_next_division_line {
                piece_lines.push(Line::new(
                    next_division_line.b.clone(),
                    next_division_line.a.clone(),
                ));
            }
            if merge_last_curr_division_line {
                piece_lines.push(last_curr_division_line.clone());
            }
            if merge_prev_division_line {
                piece_lines.push(prev_division_line.clone().expect("checked"));
            }
            if merge_first_division_line {
                piece_lines.push(Line::new(
                    first_division_line.b.clone(),
                    first_division_line.a.clone(),
                ));
            }
            let current_piece = Simplex::new(piece_lines);
            result_list.push(current_piece.intersection_simplex(outer_simplex));
            // Java ends the loop body with the dead store `nextDivisionLine = prevDivisionLine;`
            // prevDivisionLine is never assigned in the Java code and stays null.
        }
        Some(result_list)
    }

    /// Package private `cutoutFrom(IntOctagon)`.
    pub fn cutout_from_int_octagon(&self, oct: &IntOctagon) -> Option<Vec<Simplex>> {
        self.cutout_from_simplex(&oct.to_simplex())
    }

    /// Package private `cutoutFrom(IntBox)`.
    pub fn cutout_from_int_box(&self, b: &IntBox) -> Option<Vec<Simplex>> {
        self.cutout_from_simplex(&b.to_simplex())
    }

    /// Removes lines, which are redundant in the definition of the shape of this simplex.
    /// Assumes that the lines of this simplex are sorted. Panics for an empty simplex (Java:
    /// ArrayIndexOutOfBoundsException).
    pub fn remove_redundant_lines(&self) -> Simplex {
        let src = &self.lines;
        let arr_len = src.len();
        // `lines` holds indices into `src` (Java holds object references)
        let mut lines: Vec<usize> = Vec::with_capacity(arr_len);
        // copy the sorted lines while skipping multiple lines
        lines.push(0);
        let mut prev = 0usize;
        for i in 1..arr_len {
            if !src[i].fast_equals(&src[prev]) {
                lines.push(i);
                prev = i;
            }
        }
        let mut new_length: i32 = lines.len() as i32;
        lines.resize(arr_len, 0);

        // precalculated array, on which side of this line the previous and the next line
        // do intersect
        let mut intersection_sides: Vec<Option<Side>> = vec![None; new_length as usize];

        let mut try_again = new_length > 2;
        let mut index_of_last_removed_line: i32 = new_length;
        while try_again {
            try_again = false;
            let mut prev_ind: i32 = new_length - 1;
            let mut prev_line = lines[prev_ind as usize];
            let mut current_line = lines[0];
            let mut ind: i32 = 0;
            while ind < new_length {
                let mut next_ind: i32 = if ind == new_length - 1 { 0 } else { ind + 1 };
                let next_line = lines[next_ind as usize];

                let mut remove_line = false;
                let prev_dir = src[prev_line].int_direction();
                let next_dir = src[next_line].int_direction();
                let det = prev_dir.determinant(&next_dir);
                if det != 0.0 {
                    // prev_line and next_line are not parallel
                    let side = match intersection_sides[ind as usize] {
                        Some(s) => s,
                        None => {
                            let s = src[current_line]
                                .side_of_intersection(&src[prev_line], &src[next_line]);
                            intersection_sides[ind as usize] = Some(s);
                            s
                        }
                    };
                    if det > 0.0 {
                        // direction of next_line is bigger than direction of prev_line: if the
                        // intersection of prev_line and next_line is on the left of current_line,
                        // current_line does not contribute to the shape of the simplex
                        remove_line = side != Side::OnTheLeft;
                    } else if side == Side::OnTheLeft {
                        // direction of next_line is smaller than direction of prev_line
                        let current_direction = src[current_line].int_direction();
                        if prev_dir.determinant(&current_direction) > 0.0 {
                            // the halfplane defined by current_line does not intersect with the
                            // simplex defined by prev_line and next_line: the simplex is empty
                            new_length = 0;
                            break;
                        }
                    }
                } else {
                    // prev_line and next_line are parallel
                    if src[prev_line].side_of(&src[next_line].a) == Side::OnTheLeft {
                        // the half-planes defined by prev_line and next_line do not intersect
                        new_length = 0;
                        break;
                    }
                }
                if remove_line {
                    try_again = true;
                    new_length -= 1;
                    for i in ind..new_length {
                        lines[i as usize] = lines[i as usize + 1];
                        intersection_sides[i as usize] = intersection_sides[i as usize + 1];
                    }
                    if new_length < 3 {
                        try_again = false;
                        break;
                    }
                    // reset 3 precalculated intersection sides
                    if ind == 0 {
                        prev_ind = new_length - 1;
                    }
                    intersection_sides[prev_ind as usize] = None;
                    next_ind = if ind >= new_length { 0 } else { ind };
                    intersection_sides[next_ind as usize] = None;
                    ind -= 1;
                    index_of_last_removed_line = ind;
                } else {
                    prev_line = current_line;
                    prev_ind = ind;
                }
                current_line = next_line;
                if !try_again && ind >= index_of_last_removed_line {
                    // tried all lines without removing one
                    break;
                }
                ind += 1;
            }
            if new_length == 0 {
                try_again = false;
            }
        }

        if new_length == 2 {
            let l0 = &src[lines[0]];
            let l1 = &src[lines[1]];
            if l0.is_parallel(l1) {
                if l0.direction_ref() == l1.direction_ref() {
                    // one of the two remaining lines is redundant
                    if l1.side_of(&l0.a) == Side::OnTheLeft {
                        lines[0] = lines[1];
                    }
                    new_length -= 1;
                } else {
                    // the two remaining lines have opposite direction; the simplex may be empty
                    if l1.side_of(&l0.a) == Side::OnTheLeft {
                        new_length = 0;
                    }
                }
            }
        }
        if new_length as usize == arr_len {
            return self.clone(); // nothing removed
        }
        if new_length == 0 {
            return Simplex::empty();
        }
        let result: Arc<[Line]> = lines[..new_length as usize]
            .iter()
            .map(|&i| src[i].clone())
            .collect();
        Simplex::from_arc(result)
    }

    /// For each corner of this inner simplex 1 or 2 perpendicular projections onto lines of the
    /// outer simplex are constructed, so that the resulting pieces after cutting out the inner
    /// simplex are convex.
    fn calc_division_lines(
        &self,
        inner_corner_no: usize,
        outer_simplex: &Simplex,
    ) -> Option<Vec<Line>> {
        let current_inner_line = &self.lines[inner_corner_no];
        let prev_inner_line = if inner_corner_no != 0 {
            &self.lines[inner_corner_no - 1]
        } else {
            &self.lines[self.lines.len() - 1]
        };
        let intersection = current_inner_line.intersection_approx(prev_inner_line);
        if intersection.x >= INT_MAX_F64 {
            log::warn!("Simplex.calc_division_lines: intersection expected");
            return None;
        }
        let inner_corner = intersection.round();
        let c_tolerance = 0.0001;
        let is_exact = (inner_corner.x as f64 - intersection.x).abs() < c_tolerance
            && (inner_corner.y as f64 - intersection.y).abs() < c_tolerance;
        if !is_exact {
            // it is assumed that the corners of the original inner simplex are exact and the not
            // exact corners come from the intersection with the outer simplex. Because these
            // corners lie on the border of the outer simplex, no division is necessary
            return Some(vec![prev_inner_line.clone()]);
        }
        let inner_corner_point = Point::Int(inner_corner);
        let degenerate = || {
            Some(vec![Line::new(
                inner_corner_point.clone(),
                inner_corner_point.clone(),
            )])
        };
        let mut first_projection_dir = IntDirection::NULL;
        let mut second_projection_dir = IntDirection::NULL;
        let prev_inner_dir = prev_inner_line.direction_ref().opposite().as_int();
        let next_inner_dir = current_inner_line.int_direction();
        let outer_len = outer_simplex.lines.len();
        let mut outer_line_no = 0usize;

        // search the first outer line, so that the perpendicular projection of the inner corner
        // onto this line is visible from inner_corner to the left of prev_inner_line.
        let mut min_distance = INT_MAX_F64;
        for _ in 0..outer_len {
            let outer_line = &outer_simplex.lines[outer_line_no];
            let current_projection_dir =
                match inner_corner_point.perpendicular_direction_opt(outer_line) {
                    Some(d) => d.as_int(),
                    None => return degenerate(),
                };
            let projection_visible = prev_inner_dir.determinant(&current_projection_dir) >= 0.0;
            if projection_visible {
                let mut current_distance =
                    outer_line.signed_distance(&inner_corner.to_float()).abs();
                let second_division_necessary =
                    current_projection_dir.determinant(&next_inner_dir) < 0.0;
                // may occur at a sharp angle
                let mut current_second_projection_dir = current_projection_dir;
                if second_division_necessary {
                    // search the first projection_dir between current_projection_dir and
                    // next_inner_dir, that is visible from next_inner_line
                    let mut second_projection_visible = false;
                    let mut tmp_outer_line_no = outer_line_no;
                    while !second_projection_visible {
                        if tmp_outer_line_no == outer_len - 1 {
                            tmp_outer_line_no = 0;
                        } else {
                            tmp_outer_line_no += 1;
                        }
                        current_second_projection_dir = match inner_corner_point
                            .perpendicular_direction_opt(&outer_simplex.lines[tmp_outer_line_no])
                        {
                            Some(d) => d.as_int(),
                            // inner corner is on outer_line
                            None => return degenerate(),
                        };
                        if current_projection_dir.determinant(&current_second_projection_dir) < 0.0
                        {
                            // the angle between current_projection_dir and
                            // current_second_projection_dir would be already bigger than 180
                            current_distance = INT_MAX_F64;
                            break;
                        }
                        second_projection_visible =
                            current_second_projection_dir.determinant(&next_inner_dir) >= 0.0;
                    }
                    current_distance += outer_simplex.lines[tmp_outer_line_no]
                        .signed_distance(&inner_corner.to_float())
                        .abs();
                }
                if current_distance < min_distance {
                    min_distance = current_distance;
                    first_projection_dir = current_projection_dir;
                    second_projection_dir = current_second_projection_dir;
                }
            }
            if outer_line_no == outer_len - 1 {
                outer_line_no = 0;
            } else {
                outer_line_no += 1;
            }
        }
        if min_distance == INT_MAX_F64 {
            log::warn!("Simplex.calc_division_lines: division not found");
            return None;
        }
        if first_projection_dir == second_projection_dir {
            Some(vec![Line::from_point_direction(
                inner_corner_point,
                Direction::Int(first_projection_dir),
            )])
        } else {
            Some(vec![
                Line::from_point_direction(
                    inner_corner_point.clone(),
                    Direction::Int(first_projection_dir),
                ),
                Line::from_point_direction(
                    inner_corner_point,
                    Direction::Int(second_projection_dir),
                ),
            ])
        }
    }
}

impl crate::polyline_shape::PolylineShapeImpl for Simplex {
    fn border_line_count(&self) -> i32 {
        self.lines.len() as i32
    }
    fn corner(&self, no: i32) -> Point {
        Simplex::corner(self, no)
    }
    fn border_line(&self, no: i32) -> Line {
        Simplex::border_line(self, no)
    }
    fn corner_is_bounded(&self, no: i32) -> bool {
        Simplex::corner_is_bounded(self, no)
    }
    fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
    fn is_bounded(&self) -> bool {
        Simplex::is_bounded(self)
    }
    fn dimension(&self) -> i32 {
        Simplex::dimension(self)
    }
    fn bounding_box(&self) -> IntBox {
        Simplex::bounding_box(self)
    }
    fn corner_approx(&self, no: i32) -> FloatPoint {
        Simplex::corner_approx(self, no)
    }
    fn corner_approx_arr(&self) -> Vec<FloatPoint> {
        Simplex::corner_approx_arr(self)
    }
    fn corner_approx_opt(&self, no: i32) -> Option<FloatPoint> {
        if self.lines.is_empty() {
            return None;
        }
        Some(Simplex::corner_approx(self, no))
    }
}

impl TileShapeImpl for Simplex {
    fn to_tile_shape(&self) -> TileShape {
        TileShape::Simplex(self.clone())
    }
    fn simplify(&self) -> TileShape {
        Simplex::simplify(self)
    }
    fn get_id(&self) -> i32 {
        Simplex::get_id(self)
    }
    fn is_int_box(&self) -> bool {
        Simplex::is_int_box(self)
    }
    fn is_int_octagon(&self) -> bool {
        Simplex::is_int_octagon(self)
    }
    fn intersection_int_box_tile(&self, other: &IntBox) -> TileShape {
        TileShape::Simplex(self.intersection_int_box(other))
    }
    fn intersection_int_octagon_tile(&self, other: &IntOctagon) -> TileShape {
        TileShape::Simplex(self.intersection_int_octagon(other))
    }
    fn intersection_simplex_tile(&self, other: &Simplex) -> TileShape {
        TileShape::Simplex(self.intersection_simplex(other))
    }
    fn border_line_index(&self, line: &Line) -> i32 {
        Simplex::border_line_index(self, line)
    }
    fn to_simplex(&self) -> Simplex {
        self.clone()
    }
    fn offset_tile(&self, distance: f64) -> TileShape {
        TileShape::Simplex(self.offset(distance))
    }
    fn max_width(&self) -> f64 {
        Simplex::max_width(self)
    }
    fn min_width(&self) -> f64 {
        Simplex::min_width(self)
    }
    fn translate_by_tile(&self, vector: &Vector) -> TileShape {
        TileShape::Simplex(self.translate_by(vector))
    }
    fn cutout_tile(&self, shape: &TileShape) -> Option<Vec<TileShape>> {
        shape.cutout_from_simplex(self)
    }
    fn cutout_from_int_box_tile(&self, shape: &IntBox) -> Option<Vec<TileShape>> {
        self.cutout_from_int_box(shape)
            .map(|v| v.into_iter().map(TileShape::Simplex).collect())
    }
    fn cutout_from_int_octagon_tile(&self, shape: &IntOctagon) -> Option<Vec<TileShape>> {
        self.cutout_from_int_octagon(shape)
            .map(|v| v.into_iter().map(TileShape::Simplex).collect())
    }
    fn cutout_from_simplex_tile(&self, shape: &Simplex) -> Option<Vec<TileShape>> {
        self.cutout_from_simplex(shape)
            .map(|v| v.into_iter().map(TileShape::Simplex).collect())
    }
    fn bounding_octagon_opt(&self) -> Option<IntOctagon> {
        self.bounding_octagon()
    }
    fn enlarge_tile(&self, offset: f64) -> TileShape {
        TileShape::Simplex(self.enlarge(offset))
    }
    fn intersects_int_box(&self, other: &IntBox) -> bool {
        Simplex::intersects_int_box(self, other)
    }
    fn intersects_int_octagon(&self, other: &IntOctagon) -> bool {
        Simplex::intersects_int_octagon(self, other)
    }
    fn intersects_simplex(&self, other: &Simplex) -> bool {
        Simplex::intersects_simplex(self, other)
    }
    fn intersects_circle(&self, other: &Circle) -> bool {
        Simplex::intersects_circle(self, other)
    }
    fn bounding_shape_tile(&self, dirs: &ShapeBoundingDirections) -> Option<RegularTileShape> {
        self.bounding_shape(dirs)
    }
}
