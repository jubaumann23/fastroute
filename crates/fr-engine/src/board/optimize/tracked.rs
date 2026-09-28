//! Java object identity of `Line`s and `Polyline`s.
//!
//! The port has no line objects, but Java behaviour depends on them:
//! * `PolylineTrace.change(newPolyline)` compares the lines of the old and new polyline with
//!   `!=` to decide how many search tree entries are reused (and so the tree structure). Traces
//!   share line objects with other traces (split pieces, combined traces, substitute traces of
//!   the shove algorithm, lines of a found trace used to split another one), may contain the same
//!   object twice and may contain equal but distinct objects (e.g. a line flipped twice).
//! * `new Polyline(Line[])` normalizes the line directions *inside the caller's array* when it
//!   filtered nothing; Java code keeps using such arrays, and `Line.translate` /
//!   `intersectionApprox` results depend on the direction of a line.
//!
//! [`TLine`] pairs a line with an identity number, [`TPolyline`] a polyline with the identities
//! of its lines. The functions here reproduce the Java constructors and polyline operations
//! including the identities: surviving input objects keep theirs, `opposite()` and every newly
//! constructed line get a new one. The lines of the traces on the board carry their identities
//! ([`super::super::item::PolylineTrace`]). Identities are unique per process (a global
//! counter); only their equality matters. Polyline identity (`a != b` for polylines) is the
//! identity of the shared line array ([`TPolyline::same`]).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use fr_geom::{Line, LineSegment, Point, Polyline, Side, TileShape};

static NEXT_LINE_ID: AtomicU64 = AtomicU64::new(1);

/// A new line identity (Java: a newly created `Line` object).
#[inline]
pub fn fresh_line_id() -> u64 {
    NEXT_LINE_ID.fetch_add(1, Ordering::Relaxed)
}

/// Fresh identities for `n` new line objects.
pub fn fresh_line_ids(n: usize) -> Arc<[u64]> {
    (0..n).map(|_| fresh_line_id()).collect::<Vec<u64>>().into()
}

/// Source of line identities (a handle of the global counter).
#[derive(Clone, Debug, Default)]
pub struct LineIds;

impl LineIds {
    /// A new identity (Java: a newly created `Line` object).
    #[inline]
    pub fn fresh(&mut self) -> u64 {
        fresh_line_id()
    }

    /// Wraps a newly created line.
    #[inline]
    pub fn line(&mut self, line: Line) -> TLine {
        TLine::fresh(line)
    }
}

/// A line with its Java object identity.
#[derive(Clone, Debug)]
pub struct TLine {
    pub line: Line,
    pub id: u64,
}

impl TLine {
    /// A newly created line object.
    #[inline]
    pub fn fresh(line: Line) -> TLine {
        TLine { line, id: fresh_line_id() }
    }

    /// Java `line.opposite()` (a new object).
    pub fn opposite(&self) -> TLine {
        TLine::fresh(self.line.opposite())
    }
}

/// A polyline with the identities of its lines.
#[derive(Clone, Debug)]
pub struct TPolyline {
    pub polyline: Polyline,
    pub ids: Arc<[u64]>,
}

impl TPolyline {
    /// Wraps a polyline whose lines are all new objects.
    pub fn fresh(polyline: Polyline) -> TPolyline {
        let ids = fresh_line_ids(polyline.lines.len());
        TPolyline { polyline, ids }
    }

    /// Wraps a polyline with known identities.
    pub fn new(polyline: Polyline, ids: Arc<[u64]>) -> TPolyline {
        debug_assert_eq!(polyline.lines.len(), ids.len());
        TPolyline { polyline, ids }
    }

    /// Java `polyline == other` (object identity).
    pub fn same(&self, other: &TPolyline) -> bool {
        Arc::ptr_eq(&self.polyline.lines, &other.polyline.lines)
    }

    /// Number of lines.
    #[inline]
    pub fn len(&self) -> usize {
        self.polyline.lines.len()
    }

    /// True if the polyline has no lines.
    pub fn is_empty(&self) -> bool {
        self.polyline.lines.is_empty()
    }

    /// The `i`-th line with its identity.
    #[inline]
    pub fn tline(&self, i: usize) -> TLine {
        TLine { line: self.polyline.lines[i].clone(), id: self.ids[i] }
    }

    /// A copy of the line array (Java `System.arraycopy` of `polyline.lines`).
    pub fn tlines(&self) -> Vec<TLine> {
        (0..self.len()).map(|i| self.tline(i)).collect()
    }

    /// The lines `range` of the line array.
    pub fn tlines_range(&self, range: std::ops::Range<usize>) -> Vec<TLine> {
        range.map(|i| self.tline(i)).collect()
    }

    /// Java `new Polyline(Point[])`: all lines are new objects.
    pub fn from_points(points: &[Point]) -> TPolyline {
        TPolyline::fresh(Polyline::from_points(points))
    }

    /// Java `new Polyline(fromCorner, toCorner)`: all lines are new objects.
    pub fn from_two_points(from: &Point, to: &Point) -> TPolyline {
        TPolyline::fresh(Polyline::from_two_points(from, to))
    }

    /// Java `polyline.reverse()` (`opposite()` of every line: new objects).
    pub fn reverse(&self) -> TPolyline {
        TPolyline::fresh(self.polyline.reverse())
    }

    /// Java `new Polyline(Line[] inputLines)`. Like Java, if nothing was filtered, the direction
    /// normalization is written back into `input` (flipped lines become new objects there too).
    pub fn from_lines(input: &mut [TLine]) -> TPolyline {
        let plain: Vec<Line> = input.iter().map(|l| l.line.clone()).collect();
        let polyline = Polyline::from_lines(plain);
        let n = input.len();
        if n >= 3 && polyline.lines.len() == n {
            // nothing filtered: the output lines are the input objects, evtl. flipped in place
            let mut out_ids: Vec<u64> = Vec::with_capacity(n);
            for (i, t) in input.iter_mut().enumerate() {
                let out = &polyline.lines[i];
                if out.a == t.line.a && out.b == t.line.b {
                    out_ids.push(t.id);
                } else {
                    debug_assert!(out.a == t.line.b && out.b == t.line.a, "Polyline: unexpected line change");
                    let id = fresh_line_id();
                    *t = TLine { line: out.clone(), id };
                    out_ids.push(id);
                }
            }
            return TPolyline { polyline, ids: Arc::from(out_ids) };
        }
        if polyline.lines.is_empty() {
            return TPolyline { polyline, ids: Arc::from(Vec::new()) };
        }
        // something was filtered: find the surviving input objects like Java does
        let survivors = filtered_indices(input);
        debug_assert_eq!(survivors.len(), polyline.lines.len(), "Polyline filter replica differs");
        let mut out_ids: Vec<u64> = Vec::with_capacity(survivors.len());
        for (j, &i) in survivors.iter().enumerate() {
            let out = &polyline.lines[j];
            let t = &input[i];
            if out.a == t.line.a && out.b == t.line.b {
                out_ids.push(t.id);
            } else {
                debug_assert!(out.a == t.line.b && out.b == t.line.a, "Polyline filter replica differs");
                out_ids.push(fresh_line_id());
            }
        }
        TPolyline { polyline, ids: Arc::from(out_ids) }
    }

    /// Java `polyline.skipLines(fromNo, toNo)`.
    pub fn skip_lines(&self, from_no: i32, to_no: i32) -> TPolyline {
        let len = self.len() as i32;
        if from_no < 0 || to_no > len - 1 || from_no > to_no {
            return self.clone();
        }
        let mut new_lines: Vec<TLine> = Vec::with_capacity((len - (to_no - from_no + 1)) as usize);
        for i in 0..from_no {
            new_lines.push(self.tline(i as usize));
        }
        for i in (to_no + 1)..len {
            new_lines.push(self.tline(i as usize));
        }
        TPolyline::from_lines(&mut new_lines)
    }

    /// Java `polyline.combine(other)`: `self` (the same object) if there is no common end
    /// corner.
    pub fn combine(&self, other: &TPolyline) -> TPolyline {
        if self.len() < 3 || other.len() < 3 {
            return self.clone();
        }
        let p = &self.polyline;
        let o = &other.polyline;
        let (combine_at_start, combine_other_at_start) = if p.first_corner() == o.first_corner() {
            (true, true)
        } else if p.first_corner() == o.last_corner() {
            (true, false)
        } else if p.last_corner() == o.first_corner() {
            (false, true)
        } else if p.last_corner() == o.last_corner() {
            (false, false)
        } else {
            return self.clone(); // no common endpoint
        };
        let (tl, ol) = (self.len(), other.len());
        let mut new_lines: Vec<TLine> = Vec::with_capacity(tl + ol - 2);
        if combine_at_start {
            if combine_other_at_start {
                for i in 0..ol - 1 {
                    new_lines.push(other.tline(ol - i - 1).opposite());
                }
            } else {
                new_lines.extend(other.tlines_range(0..ol - 1));
            }
            new_lines.extend(self.tlines_range(1..tl));
        } else {
            new_lines.extend(self.tlines_range(0..tl - 1));
            if combine_other_at_start {
                new_lines.extend(other.tlines_range(1..ol));
            } else {
                for i in 1..ol {
                    new_lines.push(other.tline(ol - i - 1).opposite());
                }
            }
        }
        TPolyline::from_lines(&mut new_lines)
    }

    /// Java `polyline.split(lineIndex, endLine)`: the two pieces sharing `end_line` (and the
    /// line `line_index`), or `None` if nothing was split.
    pub fn split(&self, line_index: i32, end_line: &TLine) -> Option<[TPolyline; 2]> {
        // the decisions and the geometry of fr-geom
        let pieces = self.polyline.split(line_index, &end_line.line)?;
        let li = line_index as usize;
        let new_end_corner = self.polyline.lines[li].intersection(&end_line.line);
        let mut first: Vec<TLine> = self.tlines_range(0..li + 1);
        if self.polyline.corner(line_index - 1) != new_end_corner {
            first.push(end_line.clone());
        }
        let mut second: Vec<TLine> = Vec::with_capacity(self.len() - li + 1);
        if self.polyline.corner(line_index) != new_end_corner {
            second.push(end_line.clone());
        }
        second.extend(self.tlines_range(li..self.len()));
        let r0 = TPolyline::from_lines(&mut first);
        let r1 = TPolyline::from_lines(&mut second);
        debug_assert_eq!(r0.polyline.lines.len(), pieces[0].lines.len());
        debug_assert_eq!(r1.polyline.lines.len(), pieces[1].lines.len());
        Some([r0, r1])
    }

    /// Java `polyline.shorten(newLineCount, lastSegmentLength)`.
    pub fn shorten(&self, new_line_count: i32, last_segment_length: f64) -> TPolyline {
        let p = &self.polyline;
        let last_corner = p.corner_approx(new_line_count - 2);
        let prev_last_corner = p.corner_approx(new_line_count - 3);
        let new_last_corner = Point::Int(prev_last_corner.change_length(&last_corner, last_segment_length).round());
        if new_last_corner == p.corner(p.corner_count() - 2) {
            // skip the last line
            return self.skip_lines(new_line_count - 1, new_line_count - 1);
        }
        let n = new_line_count as usize;
        let mut new_lines: Vec<TLine> = self.tlines_range(0..n - 2);
        let mut first_line_point = p.lines[n - 2].a.clone();
        if first_line_point == new_last_corner {
            first_line_point = p.lines[n - 2].b.clone();
        }
        let new_prev_last_line = Line::new(first_line_point, new_last_corner.clone());
        let last = Line::get_instance(new_last_corner, &new_prev_last_line.direction().turn_45_degree(6));
        new_lines.push(TLine::fresh(new_prev_last_line));
        new_lines.push(TLine::fresh(last));
        let result = TPolyline::from_lines(&mut new_lines);
        debug_assert!(result.polyline.lines.len() == p.shorten(new_line_count, last_segment_length).lines.len());
        result
    }

    /// Java `shape.cutout(polyline)`: the pieces of the polyline outside the shape (the polyline
    /// itself, the same object, if it does not intersect the shape).
    pub fn cutout(&self, shape: &TileShape) -> Vec<TPolyline> {
        let mut border = BorderLines::new(shape);
        let polyline = &self.polyline;
        let intersection_no = shape.entrance_points(polyline);
        let first_corner = polyline.first_corner();
        let first_corner_is_inside = shape.contains_inside(&first_corner);
        if intersection_no.is_empty() {
            if first_corner_is_inside {
                // polyline is contained completely in this shape
                return Vec::new();
            }
            // polyline is completely outside
            return vec![self.clone()];
        }
        let mut pieces: Vec<TPolyline> = Vec::new();
        let mut current_intersection_no = 0usize;
        let mut current = intersection_no[current_intersection_no];
        let first_intersection = polyline.lines[current[0] as usize].intersection(&shape.border_line(current[1]));
        if !first_corner_is_inside {
            // calculate outside piece at start
            if first_corner != first_intersection {
                let n = current[0] as usize;
                let mut current_lines: Vec<TLine> = self.tlines_range(0..n + 1);
                // close the polyline piece with the intersected edge line.
                current_lines.push(border.line(current[1]));
                let piece = TPolyline::from_lines(&mut current_lines);
                if !piece.is_empty() {
                    pieces.push(piece);
                }
            }
            current_intersection_no += 1;
        }
        while current_intersection_no + 1 < intersection_no.len() {
            // calculate the next outside polyline piece
            current = intersection_no[current_intersection_no];
            let next = intersection_no[current_intersection_no + 1];
            let (curr_no, next_no) = (current[0], next[0]);
            // check that at least 1 corner of polyline between these intersections is not
            // contained in this shape
            let insert_piece = (curr_no + 1..next_no).any(|i| shape.is_outside(&polyline.corner(i)));
            if insert_piece {
                let len = (next_no - curr_no + 3) as usize;
                let mut current_lines: Vec<TLine> = Vec::with_capacity(len);
                current_lines.push(border.line(current[1]));
                current_lines.extend(self.tlines_range(curr_no as usize..curr_no as usize + len - 2));
                current_lines.push(border.line(next[1]));
                let piece = TPolyline::from_lines(&mut current_lines);
                if !piece.is_empty() {
                    pieces.push(piece);
                }
            }
            current_intersection_no += 2;
        }
        if current_intersection_no < intersection_no.len() {
            // calculate outside piece at end
            current = intersection_no[current_intersection_no];
            let n = current[0] as usize;
            let mut current_lines: Vec<TLine> = Vec::with_capacity(self.len() - n + 1);
            current_lines.push(border.line(current[1]));
            current_lines.extend(self.tlines_range(n..self.len()));
            let piece = TPolyline::from_lines(&mut current_lines);
            if !piece.is_empty() {
                pieces.push(piece);
            }
        }
        debug_assert_eq!(pieces.len(), shape.cutout_polyline(polyline).len());
        pieces
    }

    /// Java `new LineSegment(polyline, no)` (the lines `no - 1`, `no`, `no + 1`).
    pub fn segment(&self, no: i32) -> TSegment {
        let n = no as usize;
        TSegment { start: self.tline(n - 1), middle: self.tline(n), end: self.tline(n + 1) }
    }
}

/// The identities of the border lines of a tile shape (Java `borderLine(i)`): a `Simplex` returns
/// its stored line objects, `IntBox` and `IntOctagon` create a new object on every call.
pub struct BorderLines<'a> {
    shape: &'a TileShape,
    simplex_ids: Vec<Option<u64>>,
}

impl<'a> BorderLines<'a> {
    pub fn new(shape: &'a TileShape) -> Self {
        let n = if matches!(shape, TileShape::Simplex(_)) { shape.border_line_count().max(0) as usize } else { 0 };
        BorderLines { shape, simplex_ids: vec![None; n] }
    }

    /// Java `shape.borderLine(no)`.
    pub fn line(&mut self, no: i32) -> TLine {
        let line = self.shape.border_line(no);
        if self.simplex_ids.is_empty() {
            return TLine::fresh(line);
        }
        // (Java clamps the index of a simplex border line)
        let k = (no.max(0) as usize).min(self.simplex_ids.len() - 1);
        let id = *self.simplex_ids[k].get_or_insert_with(fresh_line_id);
        TLine { line, id }
    }
}

/// A Java `LineSegment` with the identities of its three lines.
#[derive(Clone, Debug)]
pub struct TSegment {
    pub start: TLine,
    pub middle: TLine,
    pub end: TLine,
}

impl TSegment {
    /// The fr-geom segment.
    pub fn segment(&self) -> LineSegment {
        LineSegment::new(self.start.line.clone(), self.middle.line.clone(), self.end.line.clone())
    }

    /// Java `sortEndpointsInXY()` (swaps the end lines, same objects).
    fn sort_endpoints_in_xy(&self) -> TSegment {
        let s = self.segment();
        if s.start_point().compare_xy(&s.end_point()) > 0 {
            TSegment { start: self.end.clone(), middle: self.middle.clone(), end: self.start.clone() }
        } else {
            self.clone()
        }
    }

    /// Java `intersection(other)`: the lines (objects of the two segments) through the
    /// intersection points.
    pub fn intersection(&self, other: &TSegment) -> Vec<TLine> {
        let this_seg = self.segment();
        let other_seg = other.segment();
        if !this_seg.bounding_box().intersects_int_box(&other_seg.bounding_box()) {
            return Vec::new();
        }
        let start_point_side = this_seg.start_point().side_of_line(&other.middle.line);
        let end_point_side = this_seg.end_point().side_of_line(&other.middle.line);
        let result = if start_point_side == Side::Collinear && end_point_side == Side::Collinear {
            // there may be an overlap
            let this_sorted = self.sort_endpoints_in_xy();
            let other_sorted = other.sort_endpoints_in_xy();
            let (left, right) = if this_sorted.segment().start_point().compare_xy(&other_sorted.segment().start_point()) <= 0 {
                (this_sorted, other_sorted)
            } else {
                (other_sorted, this_sorted)
            };
            let (ls, rs) = (left.segment(), right.segment());
            let cmp = ls.end_point().compare_xy(&rs.start_point());
            if cmp < 0 {
                Vec::new()
            } else if cmp == 0 {
                vec![left.end.clone()]
            } else if rs.end_point().compare_xy(&ls.end_point()) >= 0 {
                vec![right.start.clone(), left.end.clone()]
            } else {
                vec![right.start.clone(), right.end.clone()]
            }
        } else if start_point_side == end_point_side
            || other_seg.start_point().side_of_line(&self.middle.line) == other_seg.end_point().side_of_line(&self.middle.line)
        {
            Vec::new() // no intersection possible
        } else {
            vec![other.middle.clone()]
        };
        debug_assert_eq!(result.len(), this_seg.intersection(&other_seg).len());
        result
    }
}

/// Indices of the input lines kept by Java `removeConsecutiveParallelLines` followed by
/// `removeOverlaps` (only called when something was filtered and the result has >= 3 lines).
fn filtered_indices(lines: &[TLine]) -> Vec<usize> {
    // removeConsecutiveParallelLines
    let mut idx: Vec<usize> = (0..lines.len()).collect();
    if lines.len() >= 3 {
        let mut tmp: Vec<usize> = vec![0];
        for i in 1..lines.len() {
            let last = *tmp.last().unwrap();
            if !lines[last].line.is_parallel(&lines[i].line) {
                tmp.push(i);
            }
        }
        if tmp.len() != lines.len() {
            if tmp.len() < 3 {
                return Vec::new();
            }
            idx = tmp;
        }
    }
    // removeOverlaps
    let l = |k: usize| &lines[idx[k]].line;
    let n = idx.len();
    if n < 4 {
        return idx;
    }
    let mut tmp: Vec<usize> = vec![0; n];
    let mut new_length = 0usize;
    tmp[0] = idx[0];
    if !l(0).is_equal_or_opposite(l(2)) {
        new_length += 1;
    }
    tmp[new_length] = idx[1];
    new_length += 1;
    for (i, &line_index) in idx.iter().enumerate().take(n - 2).skip(2) {
        if lines[tmp[new_length - 1]].line.is_equal_or_opposite(l(i + 1)) {
            new_length -= 1;
        } else {
            tmp[new_length] = line_index;
            new_length += 1;
        }
    }
    tmp[new_length] = idx[n - 2];
    new_length += 1;
    if new_length >= 2 && !l(n - 1).is_equal_or_opposite(&lines[tmp[new_length - 2]].line) {
        tmp[new_length] = idx[n - 1];
        new_length += 1;
    }
    if new_length == n {
        return idx;
    }
    if new_length < 3 {
        return Vec::new();
    }
    tmp.truncate(new_length);
    tmp
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(ax: i32, ay: i32, bx: i32, by: i32) -> Line {
        Line::new_ints(ax, ay, bx, by)
    }

    #[test]
    fn unfiltered_input_keeps_identities_and_is_normalized_in_place() {
        // a horizontal segment with perpendicular end lines; the middle line points backwards
        let mut input = vec![TLine::fresh(l(0, 0, 0, 1)), TLine::fresh(l(10, 0, 0, 0)), TLine::fresh(l(10, 0, 10, 1))];
        let before: Vec<u64> = input.iter().map(|t| t.id).collect();
        let p = TPolyline::from_lines(&mut input);
        assert_eq!(p.len(), 3);
        for (i, t) in input.iter().enumerate() {
            // the caller's array holds the normalized lines
            assert_eq!(t.line.a, p.polyline.lines[i].a);
            assert_eq!(t.line.b, p.polyline.lines[i].b);
            assert_eq!(t.id, p.ids[i]);
        }
        // the end lines are never flipped, the backwards middle line is a new object
        assert_eq!(p.ids[0], before[0]);
        assert_eq!(p.ids[2], before[2]);
        assert_ne!(p.ids[1], before[1]);
    }

    #[test]
    fn filtered_input_keeps_the_first_of_parallel_lines() {
        let a = TLine::fresh(l(0, 0, 0, 1));
        let b = TLine::fresh(l(0, 0, 10, 0));
        let b2 = TLine::fresh(l(0, 0, 20, 0)); // parallel to b: skipped
        let c = TLine::fresh(l(10, 0, 10, 1));
        let mut input = vec![a.clone(), b.clone(), b2, c.clone()];
        let p = TPolyline::from_lines(&mut input);
        assert_eq!(p.len(), 3);
        assert_eq!(p.ids[0], a.id);
        assert_eq!(p.ids[2], c.id);
        // the kept middle line is b (or a new object if it had to be flipped)
        assert!(p.ids[1] == b.id || (p.polyline.lines[1].a == b.line.b && p.polyline.lines[1].b == b.line.a));
    }

    #[test]
    fn split_pieces_share_the_end_line() {
        let p = TPolyline::from_points(&[Point::Int(fr_geom::IntPoint::new(0, 0)), Point::Int(fr_geom::IntPoint::new(100, 0))]);
        let end_line = TLine::fresh(l(50, 0, 50, 1));
        let [a, b] = p.split(1, &end_line).unwrap();
        assert_eq!(a.ids[0], p.ids[0]);
        assert_eq!(b.ids[b.len() - 1], p.ids[2]);
        // the split line of the old polyline is in both pieces, the end line too (evtl. flipped)
        assert_eq!(a.ids[1], p.ids[1]);
        assert_eq!(b.ids[1], p.ids[1]);
    }
}
