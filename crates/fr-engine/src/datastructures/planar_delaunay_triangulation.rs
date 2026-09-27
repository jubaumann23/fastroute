//! Port of `datastructures/PlanarDelaunayTriangulation.java`: randomized incremental Delaunay
//! triangulation with a history DAG for point location (de Berg et al., chapter 9.3). Used by
//! `drc/NetIncompletes` to compute airlines.
//!
//! # Rust representation
//!
//! Corners, edges and triangles live in arenas (`u32` indices). Java edge ids (`newEdgeId`,
//! which orders the result) are `arena index + 1`: every `new Edge` in Java — including the
//! three unused edges of `splitAtInnerPoint` — creates an arena entry, so the ids match.
//! The Java `Storable` objects become caller supplied `O: Copy + PartialEq` keys (Java compares
//! them by identity); the bounding corners have no object (`None`, Java `null`).
//!
//! # The static `Random`
//!
//! Java shuffles the corners with a *static* `Random` shared by all instances, but reseeds it
//! with 99 at the start of every construction. The caller passes that generator explicitly
//! (`&mut JavaRandom`); it is reseeded here, so its state on entry never matters and the
//! result is a pure function of the input. (In Java, concurrent triangulations on several
//! threads race on the shared generator; the explicit parameter removes that.)
//! [`PlanarDelaunayTriangulation::new_with_own_random`] uses a private generator.

use fr_geom::{FloatPoint, IntPoint, Point, Side};
use fr_jcompat::{shuffle, JavaRandom};

const NONE: u32 = u32::MAX;

/// Java `seed`.
pub const SEED: i64 = 99;

/// Java `Limits.CRIT_INT`, the coordinate of the bounding triangle.
const BOUNDING_COOR: i32 = fr_geom::limits::CRIT_INT;

/// The point operations the triangulation needs (Java `Point.sideOf`, `Point.toFloat`).
pub trait TriangulationPoint: Clone {
    /// Java `sideOf(p1, p2)`.
    fn side_of(&self, p1: &Self, p2: &Self) -> Side;
    /// Java `toFloat()`.
    fn to_float(&self) -> FloatPoint;
    /// Java `new IntPoint(x, y)` (the corners of the bounding triangle).
    fn from_int_coordinates(x: i32, y: i32) -> Self;
}

impl TriangulationPoint for Point {
    fn side_of(&self, p1: &Self, p2: &Self) -> Side {
        Point::side_of(self, p1, p2)
    }

    fn to_float(&self) -> FloatPoint {
        Point::to_float(self)
    }

    fn from_int_coordinates(x: i32, y: i32) -> Self {
        Point::Int(IntPoint::new(x, y))
    }
}

impl TriangulationPoint for IntPoint {
    fn side_of(&self, p1: &Self, p2: &Self) -> Side {
        Point::Int(*self).side_of(&Point::Int(*p1), &Point::Int(*p2))
    }

    fn to_float(&self) -> FloatPoint {
        Point::Int(*self).to_float()
    }

    fn from_int_coordinates(x: i32, y: i32) -> Self {
        IntPoint::new(x, y)
    }
}

/// Java `ResultEdge`: a line segment of the triangulation.
#[derive(Clone, Debug, PartialEq)]
pub struct ResultEdge<O, P> {
    pub start_point: P,
    /// Always `Some` except for a degenerate edge ending at a bounding corner (Java `null`).
    pub start_object: Option<O>,
    pub end_point: P,
    pub end_object: Option<O>,
}

#[derive(Clone, Debug)]
struct Corner<O, P> {
    object: Option<O>,
    coor: P,
}

#[derive(Clone, Debug)]
struct Edge {
    start: u32,
    end: u32,
    left: u32,
    right: u32,
}

#[derive(Clone, Debug)]
struct Triangle {
    edge_lines: [u32; 3],
    first_parent: u32,
    children: Vec<u32>,
    is_on_the_left_of_edge_line: Option<[bool; 3]>,
}

/// Java `PlanarDelaunayTriangulation`.
#[derive(Clone, Debug)]
pub struct PlanarDelaunayTriangulation<O, P> {
    corners: Vec<Corner<O, P>>,
    edges: Vec<Edge>,
    triangles: Vec<Triangle>,
    /// Java `searchGraph.anchor`.
    anchor: u32,
    degenerate_edges: Vec<u32>,
}

impl<O: Copy + PartialEq, P: TriangulationPoint> PlanarDelaunayTriangulation<O, P> {
    /// Java `new PlanarDelaunayTriangulation(objectList)`: `objects` yields each object with its
    /// triangulation corners (Java `getTriangulationCorners()`), in collection order. `random`
    /// stands for the Java static generator (see the module documentation).
    pub fn new<I, C>(objects: I, random: &mut JavaRandom) -> Self
    where
        I: IntoIterator<Item = (O, C)>,
        C: IntoIterator<Item = P>,
    {
        let mut result = PlanarDelaunayTriangulation {
            corners: Vec::new(),
            edges: Vec::new(),
            triangles: Vec::new(),
            anchor: NONE,
            degenerate_edges: Vec::new(),
        };
        for (object, corners) in objects {
            for coor in corners {
                result.corners.push(Corner { object: Some(object), coor });
            }
        }
        let mut corner_list: Vec<u32> = (0..result.corners.len() as u32).collect();

        // create a random permutation of the corners.
        // use a fixed seed to get reproducible result
        random.set_seed(SEED);
        shuffle(&mut corner_list, random);

        // create a big triangle containing all corners in the list to start with.
        let b0 = result.add_corner(P::from_int_coordinates(BOUNDING_COOR, 0));
        let b1 = result.add_corner(P::from_int_coordinates(0, BOUNDING_COOR));
        let b2 = result.add_corner(P::from_int_coordinates(-BOUNDING_COOR, -BOUNDING_COOR));

        let e0 = result.new_edge(b0, b1);
        let e1 = result.new_edge(b1, b2);
        let e2 = result.new_edge(b2, b0);
        let start_triangle = result.new_triangle([e0, e1, e2], NONE);

        // Set the left triangle of the edge lines to startTriangle.
        // The right triangles remains null.
        for e in [e0, e1, e2] {
            result.edges[e as usize].left = start_triangle;
        }

        // Initialize the search graph.
        result.graph_insert(start_triangle, NONE);

        // Insert the corners in the corner list into the search graph.
        for corner in corner_list {
            let triangle_to_split = result
                .position_locate(corner)
                .expect("NullPointerException: PlanarDelaunayTriangulation: containing triangle not found");
            result.split(triangle_to_split, corner);
        }
        result
    }

    /// Like [`new`](Self::new) with a private generator (same result).
    pub fn new_with_own_random<I, C>(objects: I) -> Self
    where
        I: IntoIterator<Item = (O, C)>,
        C: IntoIterator<Item = P>,
    {
        Self::new(objects, &mut JavaRandom::new(SEED))
    }

    /// Java `getEdgeLines()`: the degenerate edges (between equal corners of different objects)
    /// in creation order, then the edges of the triangulation not touching the bounding
    /// triangle, sorted by edge id.
    pub fn get_edge_lines(&self) -> Vec<ResultEdge<O, P>> {
        let mut result = Vec::new();
        for &e in &self.degenerate_edges {
            result.push(self.result_edge(e));
        }
        if self.anchor != NONE {
            let mut leaf_edges = Vec::new();
            self.get_leaf_edges(self.anchor, &mut leaf_edges);
            leaf_edges.sort_unstable();
            leaf_edges.dedup();
            for e in leaf_edges {
                result.push(self.result_edge(e));
            }
        }
        result
    }

    fn result_edge(&self, e: u32) -> ResultEdge<O, P> {
        let edge = &self.edges[e as usize];
        let start = &self.corners[edge.start as usize];
        let end = &self.corners[edge.end as usize];
        ResultEdge {
            start_point: start.coor.clone(),
            start_object: start.object,
            end_point: end.coor.clone(),
            end_object: end.object,
        }
    }

    /// Java `validate()`: checks the consistency of the triangles (debugging). Faithful to
    /// Java, which ignores the results of the children of an inner node, so this only checks
    /// anything while the anchor is a leaf.
    pub fn validate(&self) -> bool {
        let result = self.triangle_validate(self.anchor);
        if result {
            log::warn!("Delaunay triangulation check passed ok");
        } else {
            log::warn!("Delaunay triangulation check has detected problems");
        }
        result
    }

    // ------------------------------------------------------------------------------------------
    // arena helpers

    fn add_corner(&mut self, coor: P) -> u32 {
        self.corners.push(Corner { object: None, coor });
        (self.corners.len() - 1) as u32
    }

    /// Java `new Edge(start, end)` (the id is `index + 1`).
    fn new_edge(&mut self, start: u32, end: u32) -> u32 {
        self.edges.push(Edge { start, end, left: NONE, right: NONE });
        (self.edges.len() - 1) as u32
    }

    fn new_triangle(&mut self, edge_lines: [u32; 3], first_parent: u32) -> u32 {
        self.triangles.push(Triangle {
            edge_lines,
            first_parent,
            children: Vec::new(),
            is_on_the_left_of_edge_line: None,
        });
        (self.triangles.len() - 1) as u32
    }

    #[inline]
    fn side_of(&self, corner: u32, p1: u32, p2: u32) -> Side {
        let c = &self.corners;
        c[corner as usize].coor.side_of(&c[p1 as usize].coor, &c[p2 as usize].coor)
    }

    #[inline]
    fn edge(&self, e: u32) -> &Edge {
        &self.edges[e as usize]
    }

    #[inline]
    fn tri_edges(&self, t: u32) -> [u32; 3] {
        self.triangles[t as usize].edge_lines
    }

    // ------------------------------------------------------------------------------------------
    // TriangleGraph

    /// Java `TriangleGraph.insert(triangle, parent)`.
    fn graph_insert(&mut self, triangle: u32, parent: u32) {
        self.initialize_is_on_the_left_of_edge_line_array(triangle);
        if parent == NONE {
            self.anchor = triangle;
        } else {
            self.triangles[parent as usize].children.push(triangle);
        }
    }

    /// Java `TriangleGraph.positionLocate(corner)`.
    fn position_locate(&self, corner: u32) -> Option<u32> {
        if self.anchor == NONE {
            return None;
        }
        let anchor = &self.triangles[self.anchor as usize];
        if anchor.children.is_empty() {
            return Some(self.anchor);
        }
        for &child in &anchor.children {
            if let Some(result) = self.position_locate_reku(corner, child) {
                return Some(result);
            }
        }
        log::warn!("TriangleGraph.position_locate: containing triangle not found");
        None
    }

    /// Java `positionLocateReku`: depth first search, first matching child wins.
    fn position_locate_reku(&self, corner: u32, triangle: u32) -> Option<u32> {
        // Explicit stack instead of recursion; children are pushed in reverse order so they are
        // visited in list order, exactly like the Java recursion.
        let mut stack = vec![triangle];
        while let Some(t) = stack.pop() {
            if !self.triangle_contains(t, corner) {
                continue;
            }
            let tri = &self.triangles[t as usize];
            if tri.children.is_empty() {
                return Some(t);
            }
            stack.extend(tri.children.iter().rev().copied());
        }
        None
    }

    // ------------------------------------------------------------------------------------------
    // PlanarDelaunayTriangulation

    /// Java `split(triangle, corner)`.
    fn split(&mut self, triangle: u32, corner: u32) -> bool {
        // check, if corner is in the interior of this triangle or
        // if corner is contained in an edge line.
        let mut containing_edge = NONE;
        for i in 0..3 {
            let current_edge = self.tri_edges(triangle)[i];
            let e = self.edge(current_edge);
            let current_side = if e.left == triangle {
                self.side_of(corner, e.start, e.end)
            } else {
                self.side_of(corner, e.end, e.start)
            };
            if current_side == Side::OnTheRight {
                // corner is outside this triangle
                log::warn!("PlanarDelaunayTriangulation.split: corner is outside");
                return false;
            } else if current_side == Side::Collinear {
                if containing_edge != NONE {
                    // corner is equal to a corner of this triangle
                    let Some(common_corner) = self.common_corner(current_edge, containing_edge) else {
                        log::warn!("PlanarDelaunayTriangulation.split: common corner expected");
                        return false;
                    };
                    if self.corners[corner as usize].object == self.corners[common_corner as usize].object {
                        return false;
                    }
                    let degenerate = self.new_edge(corner, common_corner);
                    self.degenerate_edges.push(degenerate);
                    return true;
                }
                containing_edge = current_edge;
            }
        }

        if containing_edge == NONE {
            // split triangle into 3 new triangles by adding edges from
            // the corners of  triangle to corner.
            let Some(new_triangles) = self.split_at_inner_point(triangle, corner) else {
                return false;
            };
            for t in new_triangles {
                self.graph_insert(t, triangle);
            }
            for i in 0..3 {
                let e = self.tri_edges(triangle)[i];
                self.legalize_edge(corner, e);
            }
        } else {
            // split this triangle and the neighbour triangle into 4 new triangles by adding edges
            // from the corners of the triangles to corner.
            let neighbour_to_split = self.other_neighbour(containing_edge, triangle);
            let Some(new_triangles) = self.split_at_border_point(triangle, corner, neighbour_to_split) else {
                return false;
            };
            // There are exact four new triangles with the first 2 dividing triangle and
            // the last 2 dividing neighbourToSplit.
            self.graph_insert(new_triangles[0], triangle);
            self.graph_insert(new_triangles[1], triangle);
            self.graph_insert(new_triangles[2], neighbour_to_split);
            self.graph_insert(new_triangles[3], neighbour_to_split);

            for i in 0..3 {
                let e = self.tri_edges(triangle)[i];
                if e != containing_edge {
                    self.legalize_edge(corner, e);
                }
            }
            for i in 0..3 {
                let e = self.tri_edges(neighbour_to_split)[i];
                if e != containing_edge {
                    self.legalize_edge(corner, e);
                }
            }
        }
        true
    }

    /// Java `legalizeEdge(corner, edge)`: flips edge, if it is not a legal Delaunay edge, and
    /// recurses into the other edges of the changed triangle.
    fn legalize_edge(&mut self, corner: u32, edge: u32) -> bool {
        if self.edge_is_legal(edge) {
            return false;
        }
        let (left, right) = (self.edge(edge).left, self.edge(edge).right);
        let triangle_to_change = if self.opposite_corner(left, edge) == Some(corner) {
            right
        } else if self.opposite_corner(right, edge) == Some(corner) {
            left
        } else {
            log::warn!("PlanarDelaunayTriangulation.legalize_edge: edge lines inconsistent");
            return false;
        };
        let flipped_edge = self
            .flip(edge)
            .expect("NullPointerException: PlanarDelaunayTriangulation.legalize_edge: flip failed");

        // Update the search graph.
        let (f_left, f_right) = (self.edge(flipped_edge).left, self.edge(flipped_edge).right);
        self.graph_insert(f_left, left);
        self.graph_insert(f_right, left);
        self.graph_insert(f_left, right);
        self.graph_insert(f_right, right);

        // Call this function recursively for the other edge lines of triangleToChange.
        for i in 0..3 {
            let current_edge = self.tri_edges(triangle_to_change)[i];
            if current_edge != edge {
                self.legalize_edge(corner, current_edge);
            }
        }
        true
    }

    // ------------------------------------------------------------------------------------------
    // Edge

    /// Java `Edge.commonCorner(other)` (corners compared by identity).
    fn common_corner(&self, this: u32, other: u32) -> Option<u32> {
        let (t, o) = (self.edge(this), self.edge(other));
        if o.start == t.start || o.end == t.start {
            Some(t.start)
        } else if o.start == t.end || o.end == t.end {
            Some(t.end)
        } else {
            None
        }
    }

    /// Java `Edge.otherNeighbour(triangle)` (NONE for Java `null`).
    fn other_neighbour(&self, edge: u32, triangle: u32) -> u32 {
        let e = self.edge(edge);
        if triangle == e.left {
            e.right
        } else if triangle == e.right {
            e.left
        } else {
            log::warn!("Edge.other_neighbour: inconsistent neighbour triangle");
            NONE
        }
    }

    /// Java `Edge.isLegal()`.
    fn edge_is_legal(&self, edge: u32) -> bool {
        let e = self.edge(edge);
        if e.left == NONE || e.right == NONE {
            return true;
        }
        let left_opposite = self.opposite_corner(e.left, edge).expect("NullPointerException in Edge.isLegal");
        let right_opposite = self.opposite_corner(e.right, edge).expect("NullPointerException in Edge.isLegal");
        let c = &self.corners;
        let inside_circle = c[right_opposite as usize].coor.to_float().inside_circle(
            &c[e.start as usize].coor.to_float(),
            &c[left_opposite as usize].coor.to_float(),
            &c[e.end as usize].coor.to_float(),
        );
        !inside_circle
    }

    /// Java `Edge.flip()`: replaces this edge by the edge between the opposite corners of the
    /// adjacent triangles and returns the new edge.
    fn flip(&mut self, this: u32) -> Option<u32> {
        let (this_left, this_right) = (self.edge(this).left, self.edge(this).right);
        // Create the flipped edge, so that the start corner of this edge is on the left
        // and the end corner of this edge on the right.
        let start = self.opposite_corner(this_right, this).unwrap_or(NONE);
        let end = self.opposite_corner(this_left, this).unwrap_or(NONE);
        let flipped_edge = self.new_edge(start, end);

        let first_parent = this_left;

        // Calculate the index of this edge line in the left and right adjacent triangles.
        let mut left_index = -1i32;
        let mut right_index = -1i32;
        for i in 0..3 {
            if self.tri_edges(this_left)[i] == this {
                left_index = i as i32;
            }
            if self.tri_edges(this_right)[i] == this {
                right_index = i as i32;
            }
        }
        if left_index < 0 || right_index < 0 {
            log::warn!("Edge.flip: edge line inconsistent");
            return None;
        }
        let (li, ri) = (left_index as usize, right_index as usize);
        let left_prev_edge = self.tri_edges(this_left)[(li + 2) % 3];
        let left_next_edge = self.tri_edges(this_left)[(li + 1) % 3];
        let right_prev_edge = self.tri_edges(this_right)[(ri + 2) % 3];
        let right_next_edge = self.tri_edges(this_right)[(ri + 1) % 3];

        // Create the left triangle of the flipped edge.
        let new_left_triangle = self.new_triangle([flipped_edge, left_prev_edge, right_next_edge], first_parent);
        self.edges[flipped_edge as usize].left = new_left_triangle;
        self.replace_neighbour(left_prev_edge, this_left, new_left_triangle);
        self.replace_neighbour(right_next_edge, this_right, new_left_triangle);

        // Create the right triangle of the flipped edge.
        let new_right_triangle = self.new_triangle([flipped_edge, right_prev_edge, left_next_edge], first_parent);
        self.edges[flipped_edge as usize].right = new_right_triangle;
        self.replace_neighbour(right_prev_edge, this_right, new_right_triangle);
        self.replace_neighbour(left_next_edge, this_left, new_right_triangle);

        Some(flipped_edge)
    }

    /// `if (edge.leftTriangle == old) edge.leftTriangle = new; else edge.rightTriangle = new;`
    #[inline]
    fn replace_neighbour(&mut self, edge: u32, old: u32, new: u32) {
        let e = &mut self.edges[edge as usize];
        if e.left == old {
            e.left = new;
        } else {
            e.right = new;
        }
    }

    /// Java `Edge.validate()`.
    fn edge_validate(&self, edge: u32) -> bool {
        let e = self.edge(edge);
        let mut result = true;
        let bounding_edge =
            self.corners[e.start as usize].object.is_none() && self.corners[e.end as usize].object.is_none();
        if e.left == NONE {
            if !bounding_edge {
                log::warn!("Edge.validate: left triangle may be null only for bounding edges");
                result = false;
            }
        } else if !self.tri_edges(e.left).contains(&edge) {
            log::warn!("Edge.validate: left triangle does not contain this edge");
            result = false;
        }
        if e.right == NONE {
            if !bounding_edge {
                log::warn!("Edge.validate: right triangle may be null only for bounding edges");
                result = false;
            }
        } else if !self.tri_edges(e.right).contains(&edge) {
            log::warn!("Edge.validate: right triangle does not contain this edge");
            result = false;
        }
        result
    }

    // ------------------------------------------------------------------------------------------
    // Triangle

    /// Java `Triangle.getCorner(no)`.
    fn get_corner(&self, triangle: u32, no: usize) -> Option<u32> {
        let current_edge = self.edge(self.tri_edges(triangle)[no]);
        if current_edge.left == triangle {
            Some(current_edge.start)
        } else if current_edge.right == triangle {
            Some(current_edge.end)
        } else {
            log::warn!("Triangle.get_corner: inconsistent edge lines");
            None
        }
    }

    /// Java `Triangle.oppositeCorner(edgeLine)`.
    fn opposite_corner(&self, triangle: u32, edge_line: u32) -> Option<u32> {
        let edges = self.tri_edges(triangle);
        let Some(edge_line_no) = edges.iter().position(|&e| e == edge_line) else {
            log::warn!("Triangle.opposite_corner: edgeLine not found");
            return None;
        };
        let next_edge = self.edge(edges[(edge_line_no + 1) % 3]);
        if next_edge.left == triangle {
            Some(next_edge.end)
        } else {
            Some(next_edge.start)
        }
    }

    /// Java `Triangle.contains(corner)`: inside or on the border.
    fn triangle_contains(&self, triangle: u32, corner: u32) -> bool {
        let tri = &self.triangles[triangle as usize];
        let Some(is_left) = tri.is_on_the_left_of_edge_line else {
            log::warn!("Triangle.contains: array isOnTheLeftOfEdgeLine not initialized");
            return false;
        };
        for (&edge, &on_the_left) in tri.edge_lines.iter().zip(&is_left) {
            let e = self.edge(edge);
            let current_side = self.side_of(corner, e.start, e.end);
            if on_the_left {
                if current_side == Side::OnTheRight {
                    return false;
                }
            } else if current_side == Side::OnTheLeft {
                return false;
            }
        }
        true
    }

    /// Java `Triangle.getLeafEdges(resultEdges)` (explicit stack; the set is sorted by the
    /// caller).
    fn get_leaf_edges(&self, triangle: u32, result_edges: &mut Vec<u32>) {
        let mut stack = vec![triangle];
        while let Some(t) = stack.pop() {
            let tri = &self.triangles[t as usize];
            if tri.children.is_empty() {
                for &e in &tri.edge_lines {
                    let edge = self.edge(e);
                    if self.corners[edge.start as usize].object.is_some()
                        && self.corners[edge.end as usize].object.is_some()
                    {
                        // Skip edges containing a bounding corner.
                        result_edges.push(e);
                    }
                }
            } else {
                for &child in &tri.children {
                    if self.triangles[child as usize].first_parent == t {
                        // to prevent traversing nodes more than once
                        stack.push(child);
                    }
                }
            }
        }
    }

    /// Java `Triangle.splitAtInnerPoint(corner)`.
    fn split_at_inner_point(&mut self, this: u32, corner: u32) -> Option<[u32; 3]> {
        // Java creates three edges here that are never used; they consume edge ids.
        for i in 0..3 {
            let c = self.get_corner(this, i).unwrap_or(NONE);
            self.new_edge(c, corner);
        }

        // construct the 3 new triangles.
        let this_edges = self.tri_edges(this);
        let c1 = self.get_corner(this, 1).unwrap_or(NONE);
        let e01 = self.new_edge(c1, corner);
        let c0 = self.get_corner(this, 0).unwrap_or(NONE);
        let e02 = self.new_edge(corner, c0);
        let t0 = self.new_triangle([this_edges[0], e01, e02], this);

        let c2 = self.get_corner(this, 2).unwrap_or(NONE);
        let e11 = self.new_edge(c2, corner);
        let t1 = self.new_triangle([this_edges[1], e11, e01], this);

        let t2 = self.new_triangle([this_edges[2], e02, e11], this);
        let new_triangles = [t0, t1, t2];

        // Set the new neighbour triangles of the edge lines.
        for &t in &new_triangles {
            let current_edge = self.tri_edges(t)[0];
            let e = &mut self.edges[current_edge as usize];
            if e.left == this {
                e.left = t;
            } else {
                e.right = t;
            }
            // The other neighbour triangle remains valid.
        }

        let e = &mut self.edges[e01 as usize]; // newTriangles[0].edgeLines[1]
        e.left = t0;
        e.right = t1;
        let e = &mut self.edges[e11 as usize]; // newTriangles[1].edgeLines[1]
        e.left = t1;
        e.right = t2;
        let e = &mut self.edges[e02 as usize]; // newTriangles[2].edgeLines[1]
        e.left = t0;
        e.right = t2;
        Some(new_triangles)
    }

    /// Java `Triangle.splitAtBorderPoint(corner, neighbourToSplit)`.
    fn split_at_border_point(&mut self, this: u32, corner: u32, neighbour_to_split: u32) -> Option<[u32; 4]> {
        // look for the triangle edge of this and the neighbour triangle containing corner;
        let mut this_touching_edge_no = -1i32;
        let mut neighbour_touching_edge_no = -1i32;
        let mut touching_edge = NONE;
        let mut other_touching_edge = NONE;
        let this_edges = self.tri_edges(this);
        let neighbour_edges = self.tri_edges(neighbour_to_split);
        for i in 0..3 {
            let e = self.edge(this_edges[i]);
            if self.side_of(corner, e.start, e.end) == Side::Collinear {
                this_touching_edge_no = i as i32;
                touching_edge = this_edges[i];
            }
            let e = self.edge(neighbour_edges[i]);
            if self.side_of(corner, e.start, e.end) == Side::Collinear {
                neighbour_touching_edge_no = i as i32;
                other_touching_edge = neighbour_edges[i];
            }
        }
        if this_touching_edge_no < 0 || neighbour_touching_edge_no < 0 {
            log::warn!("Triangle.split_at_border_point: touching edge not found");
            return None;
        }
        if touching_edge != other_touching_edge {
            log::warn!("Triangle.split_at_border_point: edges inconsistent");
            return None;
        }
        let (tn, nn) = (this_touching_edge_no as usize, neighbour_touching_edge_no as usize);

        // Construct the new edge lines that 2 split triangles of this triangle
        // will be on the left side of the new common touching edges.
        let te = self.edge(touching_edge).clone();
        let (first_common_new_edge, second_common_new_edge) = if this == te.left {
            let a = self.new_edge(te.start, corner);
            let b = self.new_edge(corner, te.end);
            (a, b)
        } else {
            let a = self.new_edge(te.end, corner);
            let b = self.new_edge(corner, te.start);
            (a, b)
        };

        // Construct the first split triangle of this triangle.
        let prev_edge = this_edges[(tn + 2) % 3];
        // construct the splitting edge line of this triangle, so that the first split
        // triangle lies on the left side, and the second split triangle on the right side.
        let pe = self.edge(prev_edge).clone();
        let this_splitting_edge = if this == pe.left {
            self.new_edge(corner, pe.start)
        } else {
            self.new_edge(corner, pe.end)
        };
        let t0 = self.new_triangle([prev_edge, first_common_new_edge, this_splitting_edge], this);
        self.replace_neighbour(prev_edge, this, t0);
        self.edges[first_common_new_edge as usize].left = t0;
        self.edges[this_splitting_edge as usize].left = t0;

        // Construct the second split triangle of this triangle.
        let next_edge = this_edges[(tn + 1) % 3];
        let t1 = self.new_triangle([this_splitting_edge, second_common_new_edge, next_edge], this);
        self.edges[this_splitting_edge as usize].right = t1;
        self.edges[second_common_new_edge as usize].left = t1;
        self.replace_neighbour(next_edge, this, t1);

        // construct the first split triangle of neighbourToSplit
        let neighbour_next_edge = neighbour_edges[(nn + 1) % 3];
        // construct the splitting edge line of neighbourToSplit, so that the first split
        // triangle lies on the left side, and the second split triangle on the right side.
        let ne = self.edge(neighbour_next_edge).clone();
        let neighbour_splitting_edge = if neighbour_to_split == ne.left {
            self.new_edge(ne.end, corner)
        } else {
            self.new_edge(ne.start, corner)
        };
        let t2 = self.new_triangle([neighbour_splitting_edge, first_common_new_edge, neighbour_next_edge], neighbour_to_split);
        self.edges[neighbour_splitting_edge as usize].left = t2;
        self.edges[first_common_new_edge as usize].right = t2;
        self.replace_neighbour(neighbour_next_edge, neighbour_to_split, t2);

        // construct the second split triangle of neighbourToSplit
        let prev_edge = neighbour_edges[(nn + 2) % 3];
        let t3 = self.new_triangle([prev_edge, second_common_new_edge, neighbour_splitting_edge], neighbour_to_split);
        self.replace_neighbour(prev_edge, neighbour_to_split, t3);
        self.edges[second_common_new_edge as usize].right = t3;
        self.edges[neighbour_splitting_edge as usize].right = t3;

        Some([t0, t1, t2, t3])
    }

    /// Java `Triangle.validate()`.
    fn triangle_validate(&self, triangle: u32) -> bool {
        let tri = &self.triangles[triangle as usize];
        let mut result = true;
        if tri.children.is_empty() {
            let mut prev_edge = tri.edge_lines[2];
            for i in 0..3 {
                let current_edge = tri.edge_lines[i];
                if !self.edge_validate(current_edge) {
                    result = false;
                }
                // Check, if the end corner of the previous line equals to the start corner of
                // this line.
                let pe = self.edge(prev_edge);
                let prev_end_corner = if pe.left == triangle { pe.end } else { pe.start };
                let ce = self.edge(current_edge);
                let current_start_corner = if ce.left == triangle {
                    ce.start
                } else if ce.right == triangle {
                    ce.end
                } else {
                    log::warn!("Triangle.validate: edge inconsistent");
                    return false;
                };
                if current_start_corner != prev_end_corner {
                    log::warn!("Triangle.validate: corner inconsistent");
                    result = false;
                }
                prev_edge = current_edge;
            }
        } else {
            for &child in &tri.children {
                if self.triangles[child as usize].first_parent == triangle {
                    // to avoid traversing nodes more than once; Java ignores the result.
                    let _ = self.triangle_validate(child);
                }
            }
        }
        result
    }

    /// Java `Triangle.initializeIsOnTheLeftOfEdgeLineArray()`.
    fn initialize_is_on_the_left_of_edge_line_array(&mut self, triangle: u32) {
        if self.triangles[triangle as usize].is_on_the_left_of_edge_line.is_some() {
            return; // already initialized
        }
        let edges = self.tri_edges(triangle);
        let arr = [
            self.edge(edges[0]).left == triangle,
            self.edge(edges[1]).left == triangle,
            self.edge(edges[2]).left == triangle,
        ];
        self.triangles[triangle as usize].is_on_the_left_of_edge_line = Some(arr);
    }

    // ------------------------------------------------------------------------------------------
    // test support

    /// The current (leaf) triangles as corner triples (counter clockwise), bounding corners
    /// included. For tests.
    #[cfg(test)]
    pub(crate) fn leaf_triangles(&self) -> Vec<[(Option<O>, P); 3]> {
        let mut result = Vec::new();
        let mut stack = vec![self.anchor];
        while let Some(t) = stack.pop() {
            let tri = &self.triangles[t as usize];
            if tri.children.is_empty() {
                let c: Vec<(Option<O>, P)> = (0..3)
                    .map(|i| {
                        let ci = self.get_corner(t, i).unwrap();
                        (self.corners[ci as usize].object, self.corners[ci as usize].coor.clone())
                    })
                    .collect();
                result.push([c[0].clone(), c[1].clone(), c[2].clone()]);
            } else {
                for &child in &tri.children {
                    if self.triangles[child as usize].first_parent == t {
                        stack.push(child);
                    }
                }
            }
        }
        result
    }

    /// Number of edges of the current triangulation that Java's own legality test
    /// (`Edge.isLegal`) would flip. For tests.
    #[cfg(test)]
    pub(crate) fn illegal_edge_count(&self) -> usize {
        let mut edges = Vec::new();
        let mut stack = vec![self.anchor];
        while let Some(t) = stack.pop() {
            let tri = &self.triangles[t as usize];
            if tri.children.is_empty() {
                edges.extend_from_slice(&tri.edge_lines);
            } else {
                for &child in &tri.children {
                    if self.triangles[child as usize].first_parent == t {
                        stack.push(child);
                    }
                }
            }
        }
        edges.sort_unstable();
        edges.dedup();
        edges.into_iter().filter(|&e| !self.edge_is_legal(e)).count()
    }

    /// Leaf-level consistency check (`Triangle.validate` applied to every leaf triangle).
    #[cfg(test)]
    pub(crate) fn validate_leaves(&self) -> bool {
        let mut ok = true;
        let mut stack = vec![self.anchor];
        while let Some(t) = stack.pop() {
            let tri = &self.triangles[t as usize];
            if tri.children.is_empty() {
                ok &= self.triangle_validate(t);
            } else {
                for &child in &tri.children {
                    if self.triangles[child as usize].first_parent == t {
                        stack.push(child);
                    }
                }
            }
        }
        ok
    }
}

#[cfg(test)]
#[path = "tests/planar_delaunay_triangulation.rs"]
mod tests;
