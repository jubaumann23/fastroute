//! Targeted tests for the tricky parts of the port: exact arithmetic paths, Java identity and
//! overflow semantics, octagon algebra, simplex normalization, polylines and convex splitting.

use fr_geom::java_compat::{math_round, JavaRandom};
use fr_geom::prelude::*;
use fr_geom::*;
use num_bigint::BigInt;

fn ip(x: i32, y: i32) -> Point {
    Point::Int(IntPoint::new(x, y))
}

fn line(ax: i32, ay: i32, bx: i32, by: i32) -> Line {
    Line::new_ints(ax, ay, bx, by)
}

/// Simple deterministic generator for test inputs.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 16
    }
    fn range(&mut self, lo: i64, hi: i64) -> i32 {
        (lo + (self.next() % ((hi - lo + 1) as u64)) as i64) as i32
    }
}

// ---------------------------------------------------------------------------------------------
// Line intersection: exact arithmetic
// ---------------------------------------------------------------------------------------------

#[test]
fn line_intersection_i128_path_matches_java_bigint_arithmetic() {
    let mut r = Lcg(7);
    for i in 0..20000 {
        // mix of small, CRIT_INT-sized and full-range int coordinates
        let lim: i64 = match i % 3 {
            0 => 1000,
            1 => limits::CRIT_INT as i64,
            _ => i32::MAX as i64 / 2,
        };
        let l1 = line(
            r.range(-lim, lim),
            r.range(-lim, lim),
            r.range(-lim, lim),
            r.range(-lim, lim),
        );
        let l2 = line(
            r.range(-lim, lim),
            r.range(-lim, lim),
            r.range(-lim, lim),
            r.range(-lim, lim),
        );
        let d1 = l1.int_delta();
        let d2 = l2.int_delta();
        let fast_path = [d1.x == 0, d1.y == 0, d1.x == d1.y, d1.x == -d1.y]
            .iter()
            .any(|b| *b)
            && [d2.x == 0, d2.y == 0, d2.x == d2.y, d2.x == -d2.y]
                .iter()
                .any(|b| *b);
        if fast_path {
            continue;
        }
        assert_eq!(
            l1.intersection(&l2),
            l1.intersection_bigint_reference(&l2),
            "{l1:?} {l2:?}"
        );
    }
}

#[test]
fn line_intersection_rational_and_parallel_results() {
    // x = 0 line (vertical) and line through (0,0),(3,1)... use general lines with rational
    // intersection
    let l1 = line(0, 0, 3, 1);
    let l2 = line(0, 1, 1, 0);
    let p = l1.intersection(&l2);
    match &p {
        Point::Rational(rp) => {
            // intersection is (3/4, 1/4)
            assert_eq!(
                **rp,
                RationalPoint::new(BigInt::from(3), BigInt::from(1), BigInt::from(4))
            );
        }
        _ => panic!("rational point expected, got {p:?}"),
    }
    let f = p.to_float();
    assert_eq!((f.x, f.y), (0.75, 0.25));
    // parallel lines: z == 0
    let l3 = line(0, 0, 2, 1);
    let l4 = line(0, 5, 2, 6);
    assert!(l3.intersection(&l4).is_infinite());
    assert_eq!(l3.intersection_approx(&l4).x, i32::MAX as f64);
    // integer intersection beyond CRIT_INT becomes a RationalPoint with z = 1
    let big = limits::CRIT_INT;
    let l5 = line(0, 0, 2, 1);
    let l6 = line(2 * big + 2, 0, 2 * big + 2, 1);
    match l5.intersection(&l6) {
        Point::Rational(rp) => assert_eq!(rp.z, BigInt::from(1)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn line_intersection_fast_paths() {
    // vertical x=5 with horizontal y=7
    assert_eq!(line(5, 0, 5, 3).intersection(&line(0, 7, 9, 7)), ip(5, 7));
    // vertical with right diagonal through (1,2): y = x + 1
    assert_eq!(line(5, 0, 5, 3).intersection(&line(1, 2, 2, 3)), ip(5, 6));
    // horizontal y=4 with left diagonal through (1,2): x + y = 3
    assert_eq!(line(0, 4, 1, 4).intersection(&line(1, 2, 2, 1)), ip(-1, 4));
    // diagonal with horizontal and vertical
    assert_eq!(line(0, 0, 1, 1).intersection(&line(0, 3, 1, 3)), ip(3, 3));
    assert_eq!(line(0, 0, 1, -1).intersection(&line(4, 0, 4, 1)), ip(4, -4));
}

#[test]
fn line_side_equality_and_direction() {
    let l = line(0, 0, 10, 0);
    assert_eq!(l.side_of(&ip(5, 5)), Side::OnTheRight); // the line is on the right of the point
    assert_eq!(l.side_of(&ip(5, -5)), Side::OnTheLeft);
    assert_eq!(l.side_of(&ip(20, 0)), Side::Collinear);
    // Java Line.equals: same point set and same orientation
    assert_eq!(l, line(-3, 0, 7, 0));
    assert_ne!(l, line(7, 0, -3, 0));
    assert!(l.is_equal_or_opposite(&line(7, 0, -3, 0)));
    // direction is normalized by the gcd
    assert_eq!(
        line(0, 0, 6, 4).direction(),
        Direction::Int(IntDirection::new(3, 2))
    );
    // Line(a, dir) keeps the given (non normalized) direction, like Java
    let dir = Direction::Int(IntDirection::new(1, 1).turn_45_degree(1)); // (0, 2)
    let l2 = Line::from_point_direction(ip(1, 1), dir.clone());
    assert!(matches!(l2.direction(), Direction::Int(d) if d.x == 0 && d.y == 2));
    assert_eq!(l2.b, ip(1, 3));
    // translate moves to the left for positive distances
    let t = line(0, 0, 10, 0).translate(3.0);
    assert_eq!(t.a, ip(0, 3));
}

#[test]
fn line_compare_orders_by_direction_angle() {
    let dirs = [
        (1, 0),
        (1, 1),
        (0, 1),
        (-1, 1),
        (-1, 0),
        (-1, -1),
        (0, -1),
        (1, -1),
    ];
    let lines: Vec<Line> = dirs.iter().map(|(x, y)| line(0, 0, *x, *y)).collect();
    for i in 0..lines.len() {
        for j in 0..lines.len() {
            assert_eq!(
                lines[i].compare_to(&lines[j]),
                (i.cmp(&j) as i32),
                "{i} {j}"
            );
        }
    }
    // Direction::compare_to agrees for IntDirection and BigIntDirection
    let d1 = Direction::Int(IntDirection::new(1, 2));
    let d2 = Direction::BigInt(Box::new(BigIntDirection::new(
        BigInt::from(-3),
        BigInt::from(1),
    )));
    assert_eq!(d1.compare_to(&d2), -1);
    assert_eq!(d2.compare_to(&d1), 1);
    assert_ne!(d1, d2);
}

// ---------------------------------------------------------------------------------------------
// Overflow to BigInteger / rational arithmetic
// ---------------------------------------------------------------------------------------------

#[test]
fn big_coordinates_switch_to_rational_types() {
    let c = limits::CRIT_INT;
    assert!(matches!(Point::get_instance(c, 0), Point::Int(_)));
    assert!(matches!(Point::get_instance(c + 1, 0), Point::Rational(_)));
    assert!(matches!(
        Vector::get_instance(0, -(c + 1)),
        Vector::Rational(_)
    ));
    // Point.getInstance(BigInteger...) reduces x/z, y/z when x is divisible by z
    let p = Point::get_instance_big(BigInt::from(-10), BigInt::from(6), BigInt::from(-2));
    assert_eq!(p, ip(5, -3));
    // rational + int arithmetic
    let r = Point::Rational(Box::new(RationalPoint::new(
        BigInt::from(1),
        BigInt::from(1),
        BigInt::from(2),
    )));
    let moved = r.translate_by(&Vector::Int(IntVector::new(1, 0)));
    assert_eq!(moved.to_float(), FloatPoint::new(1.5, 0.5));
    let diff = moved.difference_by(&ip(1, 0));
    assert_eq!(diff.to_float(), FloatPoint::new(0.5, 0.5));
    assert_eq!(r.compare_x(&ip(0, 0)), 1);
    assert_eq!(ip(0, 0).compare_x(&r), -1);
    assert_eq!(r.compare_y(&ip(1, 1)), -1);
    // huge directions become BigIntDirection
    let v = Vector::Rational(Box::new(RationalVector::new(
        BigInt::from(3_000_000_000i64),
        BigInt::from(1),
        BigInt::from(1),
    )));
    assert!(matches!(Direction::get_instance(&v), Direction::BigInt(_)));
    // IntVector normalization with Integer.MIN_VALUE (Java Math.abs overflow) keeps the vector
    let d = IntVector::new(i32::MIN, 0).to_normalized_direction();
    assert_eq!((d.x, d.y), (i32::MIN, 0));
}

#[test]
fn perpendicular_projection_int_and_rational() {
    let l = line(0, 0, 10, 0);
    assert_eq!(ip(3, 7).perpendicular_projection(&l), ip(3, 0));
    let diag = line(0, 0, 2, 1);
    // projection of (1, 0) on y = x/2 is (4/5, 2/5)
    let p = ip(1, 0).perpendicular_projection(&diag);
    assert_eq!(p.to_float(), FloatPoint::new(0.8, 0.4));
    assert!(matches!(p, Point::Rational(_)));
}

#[test]
fn java_int_overflow_is_reproduced_on_saturated_coordinates() {
    // bounding boxes of unbounded shapes carry saturated coordinates; Java silently overflows
    let b = IntBox::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX);
    let o = b.to_int_octagon();
    assert_eq!(o.upper_left_diagonal_x, i32::MIN.wrapping_sub(i32::MAX));
    assert_eq!(b.width(), -1);
    let _ = o.normalize();
    let l = Line::new_ints(i32::MAX, 0, i32::MIN, 1);
    let _ = l.compare_to(&line(0, 0, 1, 0));
    let d = IntDirection::new(2_000_000_000, 2_000_000_000).turn_45_degree(1);
    assert_eq!((d.x, d.y), (0, 2_000_000_000i32.wrapping_mul(2)));
}

// ---------------------------------------------------------------------------------------------
// Rounding
// ---------------------------------------------------------------------------------------------

#[test]
fn float_point_rounding_uses_java_semantics() {
    assert_eq!(FloatPoint::new(-2.5, 2.5).round(), IntPoint::new(-2, 3));
    assert_eq!(
        FloatPoint::new(-0.5, 0.49999999999999994).round(),
        IntPoint::new(0, 0)
    );
    assert_eq!(math_round(-1e300), i64::MIN);
    assert_eq!(FloatPoint::new(1e20, -1e20).round(), IntPoint::new(-1, 0)); // long -> int narrowing
    assert_eq!(
        FloatPoint::new(7.5, -7.5).round_to_grid(5, 5),
        IntPoint::new(10, -10)
    );
    let right = FloatPoint::new(1.5, 1.5).round_to_the_right(&Direction::RIGHT);
    assert_eq!(right, IntPoint::new(2, 1));
    let left = FloatPoint::new(1.5, 1.5).round_to_the_left(&Direction::RIGHT);
    assert_eq!(left, IntPoint::new(2, 2));
}

#[test]
fn strict_acos_matches_known_java_values() {
    // StrictMath.acos values from OpenJDK 17 (fdlibm), covering all branches
    assert_eq!(java_compat::strict_acos(0.5).to_bits(), 0x3ff0c152382d7366);
    assert_eq!(java_compat::strict_acos(-0.3).to_bits(), 0x3ffe0200bbc96ad8);
    assert_eq!(java_compat::strict_acos(0.99).to_bits(), 0x3fc21df72882bfd8);
    assert_eq!(
        java_compat::strict_acos(-0.75).to_bits(),
        0x400359d26f93b6c3
    );
    assert_eq!(
        java_compat::strict_acos(1e-10).to_bits(),
        0x3ff921fb543d4de0
    );
    assert_eq!(java_compat::strict_acos(1.0), 0.0);
    assert!(java_compat::strict_acos(1.5).is_nan());
}

// ---------------------------------------------------------------------------------------------
// Octagons and boxes
// ---------------------------------------------------------------------------------------------

#[test]
fn octagon_empty_is_identity_based_like_java() {
    assert!(IntOctagon::EMPTY.is_empty());
    assert_eq!(IntOctagon::EMPTY.dimension(), -1);
    // same coordinates, but not the EMPTY instance: Java isEmpty() returns false
    let e = IntOctagon::EMPTY;
    let lookalike = IntOctagon::new(
        e.left_x,
        e.bottom_y,
        e.right_x,
        e.top_y,
        e.upper_left_diagonal_x,
        e.lower_right_diagonal_x,
        e.lower_left_diagonal_x,
        e.upper_right_diagonal_x,
    );
    assert!(!lookalike.is_empty());
    // ... but normalize() maps it to EMPTY
    assert!(lookalike.normalize().is_empty());
    // the octagon of an empty box is not EMPTY either
    assert!(!IntBox::EMPTY.to_int_octagon().is_empty());
    assert!(IntBox::EMPTY.is_empty());
}

#[test]
fn octagon_intersection_and_containment() {
    let a = IntBox::new(0, 0, 10, 10).to_int_octagon();
    let b = IntBox::new(5, 5, 20, 20).to_int_octagon();
    let i = a.intersection_int_octagon(&b);
    assert_eq!(i.bounding_box(), IntBox::new(5, 5, 10, 10));
    assert!(i.is_int_box());
    assert!(a.intersects_int_octagon(&b));
    assert!(a.overlaps(&b));
    // touching only at a corner: intersects but no overlap
    let c = IntBox::new(10, 10, 12, 12).to_int_octagon();
    assert!(a.intersects_int_octagon(&c));
    assert!(!a.overlaps(&c));
    assert_eq!(a.intersection_int_octagon(&c).dimension(), 0);
    // disjoint
    let d = IntBox::new(11, 0, 12, 1).to_int_octagon();
    assert!(!a.intersects_int_octagon(&d));
    assert!(a.intersection_int_octagon(&d).is_empty());
    // containment
    let inner = IntOctagon::new(2, 2, 8, 8, -4, 4, 6, 14).normalize();
    assert!(inner.is_contained_in_int_octagon(&a));
    assert!(a.contains_regular(&RegularTileShape::IntOctagon(inner)));
    assert!(!inner.contains_regular(&RegularTileShape::IntOctagon(a)));
    // diamond: normalize tightens the axis parallel bounds
    let diamond = IntOctagon::new(-100, -100, 100, 100, -5, 5, -5, 5).normalize();
    assert_eq!(
        (
            diamond.left_x,
            diamond.right_x,
            diamond.bottom_y,
            diamond.top_y
        ),
        (-5, 5, -5, 5)
    );
    assert!(!diamond.is_int_box());
    assert_eq!(diamond.area(), 50.0);
    // octagon offset keeps 45 degree geometry
    let off = inner.offset(2.0);
    assert!(inner.is_contained_in_int_octagon(&off));
}

#[test]
fn intersects_is_consistent_with_intersection_for_random_octagons() {
    let mut r = Lcg(99);
    for _ in 0..2000 {
        let mut o = || {
            let lx = r.range(-50, 50);
            let ly = r.range(-50, 50);
            let w = r.range(0, 40);
            let h = r.range(0, 40);
            let b = IntBox::new(lx, ly, lx + w, ly + h);
            let s = (w + h) / 4;
            IntOctagon::new(
                b.ll.x,
                b.ll.y,
                b.ur.x,
                b.ur.y,
                b.ll.x - b.ur.y + r.range(0, s as i64),
                b.ur.x - b.ll.y - r.range(0, s as i64),
                b.ll.x + b.ll.y + r.range(0, s as i64),
                b.ur.x + b.ur.y - r.range(0, s as i64),
            )
            .normalize()
        };
        let a = o();
        let b = o();
        if a.is_empty() || b.is_empty() {
            continue;
        }
        let i = a.intersection_int_octagon(&b);
        // Java's intersects() only compares pairs of parallel bounds: it is a necessary
        // condition, it may return true although the intersection is empty.
        if !i.is_empty() {
            assert!(a.intersects_int_octagon(&b), "{a:?} {b:?}");
            assert!(i.is_contained_in_int_octagon(&a) && i.is_contained_in_int_octagon(&b));
        }
        // the octagon algebra agrees with the general simplex algebra
        let ts = TileShape::IntOctagon(a).intersection(&TileShape::Simplex(b.to_simplex()));
        assert_eq!(ts.is_empty(), i.is_empty());
    }
}

#[test]
fn box_cutout_covers_the_difference() {
    let outer = IntBox::new(0, 0, 100, 100);
    let hole = IntBox::new(20, 30, 50, 60);
    let pieces = hole.cutout_from_int_box(&outer);
    assert_eq!(pieces.len(), 4);
    let area: f64 = pieces.iter().map(|p| p.area()).sum();
    assert_eq!(area, outer.area() - hole.area());
    for p in &pieces {
        assert!(!p.overlaps(&hole));
    }
}

// ---------------------------------------------------------------------------------------------
// Simplex
// ---------------------------------------------------------------------------------------------

#[test]
fn simplex_from_lines_normalizes_and_simplifies() {
    // counterclockwise square (interior on the left of each directed line), shuffled with
    // redundant and duplicate lines
    let lines = vec![
        line(10, 0, 10, 10),
        line(0, 10, 0, 0),
        line(0, 0, 10, 0),
        line(10, 10, 0, 10),
        line(-5, 0, -5, -1), // redundant (left of x = -5 contains the square)
        line(20, 0, 30, 0),  // duplicate of the lower line
        line(100, -10, -10, 100), // redundant: x + y <= 90
    ];
    let s = Simplex::get_instance(&lines);
    assert_eq!(s.border_line_count(), 4);
    assert_eq!(s.dimension(), 2);
    assert!(s.is_bounded());
    assert!(s.is_int_box());
    match s.simplify() {
        TileShape::IntBox(b) => assert_eq!(b, IntBox::new(0, 0, 10, 10)),
        other => panic!("{other:?}"),
    }
    // corners start at the smallest direction (RIGHT) in counterclock sense
    let corners: Vec<Point> = (0..4).map(|i| s.corner(i)).collect();
    assert_eq!(corners, vec![ip(0, 0), ip(10, 0), ip(10, 10), ip(0, 10)]);

    // a triangle stays a Simplex
    let tri = TileShape::get_instance_points(&[ip(0, 0), ip(10, 0), ip(3, 7)]);
    let tri = tri.as_simplex().expect("triangle is a simplex");
    assert_eq!(tri.border_line_count(), 3);
    assert_eq!(TileShapeImpl::area(tri), 35.0);
    assert!(tri.contains(&ip(3, 2)));
    assert!(tri.contains_on_border(&ip(5, 0)));
    assert!(!tri.contains_inside(&ip(5, 0)));
    assert!(tri.is_outside(&ip(-1, 0)));

    // contradictory half planes give the empty simplex
    let empty = Simplex::get_instance(&[line(0, 0, 10, 0), line(10, 5, 0, 5)]);
    assert!(!empty.is_empty()); // two opposite lines with overlapping half planes: a strip
    assert_eq!(empty.dimension(), 2);
    let really_empty = Simplex::get_instance(&[line(0, 5, 10, 5), line(10, 0, 0, 0)]);
    assert!(really_empty.is_empty());
    // a 1-dimensional simplex (segment)
    let seg = LineSegment::new(line(0, 0, 0, 1), line(0, 0, 10, 0), line(10, 0, 10, 1));
    let seg_simplex = seg.to_simplex();
    assert_eq!(seg_simplex.dimension(), 1);
}

#[test]
fn simplex_intersection_order_follows_java_double_dispatch() {
    let a = TileShape::get_instance_points(&[ip(0, 0), ip(10, 0), ip(3, 7)]);
    let b = TileShape::IntBox(IntBox::new(2, 1, 20, 20));
    let ab = a.intersection(&b);
    let ba = b.intersection(&a);
    // same shape either way
    assert_eq!(ab.border_line_count(), ba.border_line_count());
    for i in 0..ab.border_line_count() {
        assert_eq!(ab.border_line(i), ba.border_line(i));
    }
    assert_eq!(
        TileShapeImpl::area(ab.as_simplex().unwrap()),
        TileShapeImpl::area(ba.as_simplex().unwrap())
    );
    assert!(a.intersects(&Shape::Tile(b.clone())));
    // cutout of a simplex from a box: the pieces do not overlap the cut shape's interior
    let pieces = b.cutout(&a).expect("pieces");
    for p in &pieces {
        let inner = p.intersection(&a);
        assert!(inner.dimension() < 2, "{p:?}");
    }
}

// ---------------------------------------------------------------------------------------------
// Polylines
// ---------------------------------------------------------------------------------------------

#[test]
fn polyline_corners_are_the_input_points() {
    let pts = vec![ip(0, 0), ip(100, 0), ip(100, 50), ip(130, 80), ip(200, 81)];
    let pl = Polyline::from_points(&pts);
    assert_eq!(pl.lines.len(), pts.len() + 1);
    assert_eq!(pl.corners(), pts);
    assert_eq!(pl.first_corner(), pts[0]);
    assert_eq!(pl.last_corner(), pts[4]);
    assert!(!pl.is_orthogonal());
    let rev = pl.reverse();
    let mut rp = pts.clone();
    rp.reverse();
    assert_eq!(rev.corners(), rp);
    // collinear and duplicate points are removed by the Polygon
    let pl2 = Polyline::from_points(&[ip(0, 0), ip(5, 0), ip(5, 0), ip(10, 0), ip(10, 10)]);
    assert_eq!(pl2.corners(), vec![ip(0, 0), ip(10, 0), ip(10, 10)]);
    // combine at a common end corner
    let pl3 = Polyline::from_points(&[ip(10, 10), ip(20, 10)]);
    let combined = pl2.combine(Some(&pl3));
    assert_eq!(
        combined.corners(),
        vec![ip(0, 0), ip(10, 0), ip(10, 10), ip(20, 10)]
    );
    // split at line 2 with a vertical line x = 5 through the first horizontal segment
    let split = combined.split(1, &line(5, -5, 5, 5)).expect("split");
    assert_eq!(split[0].corners(), vec![ip(0, 0), ip(5, 0)]);
    assert_eq!(
        split[1].corners(),
        vec![ip(5, 0), ip(10, 0), ip(10, 10), ip(20, 10)]
    );
    // offset shapes: one convex shape per segment, containing the segment
    let shapes = combined.offset_shapes(3);
    assert_eq!(shapes.len(), 3);
    for (i, s) in shapes.iter().enumerate() {
        assert!(s.contains(&combined.corner(i as i32)));
        assert!(s.contains(&combined.corner(i as i32 + 1)));
    }
    assert_eq!(combined.length_approx(), 30.0);
    assert!(combined.contains(&ip(10, 5)));
    assert!(!combined.contains(&ip(11, 5)));
}

#[test]
fn polyline_from_lines_normalizes_directions() {
    // the middle line is given in the "wrong" direction and is turned around
    let lines = vec![line(0, -1, 0, 1), line(10, 0, 0, 0), line(10, -1, 10, 1)];
    let pl = Polyline::from_lines(lines);
    assert_eq!(pl.corners(), vec![ip(0, 0), ip(10, 0)]);
    assert_eq!(pl.lines[1].direction(), Direction::RIGHT);
    // parallel consecutive lines are skipped
    let pl2 = Polyline::from_lines(vec![
        line(0, -1, 0, 1),
        line(0, 0, 5, 0),
        line(1, 0, 6, 0),
        line(10, -1, 10, 1),
    ]);
    assert_eq!(pl2.lines.len(), 3);
}

// ---------------------------------------------------------------------------------------------
// Polygon shapes and splitting into convex pieces
// ---------------------------------------------------------------------------------------------

fn shoelace(pts: &[Point]) -> f64 {
    let n = pts.len();
    let mut s = 0.0;
    for i in 0..n {
        let a = pts[i].to_float();
        let b = pts[(i + 1) % n].to_float();
        s += a.x * b.y - b.x * a.y;
    }
    0.5 * s.abs()
}

#[test]
fn polygon_shape_normalizes_corner_order() {
    // clockwise input is reverted; the corner with the lowest y (then x) comes first
    let ps = PolygonShape::from_points(&[ip(0, 10), ip(10, 10), ip(10, 0), ip(0, 0)]);
    assert_eq!(
        ps.corners.to_vec(),
        vec![ip(0, 0), ip(10, 0), ip(10, 10), ip(0, 10)]
    );
    assert!(ps.is_convex());
    // Java's PolygonShape.area() returns 0 for every polygon (sic)
    assert_eq!(ps.area(), 0.0);
}

#[test]
fn split_to_convex_of_an_l_shape_covers_the_polygon() {
    let pts = vec![
        ip(0, 0),
        ip(60, 0),
        ip(60, 20),
        ip(20, 20),
        ip(20, 50),
        ip(0, 50),
    ];
    let ps = PolygonShape::from_points(&pts);
    assert!(!ps.is_convex());
    let pieces = ps.split_to_convex().expect("split");
    assert!(pieces.len() >= 2);
    let total: f64 = pieces.iter().map(|p| p.area()).sum();
    assert_eq!(total, shoelace(&pts));
    // every piece is convex and inside the polygon
    for p in &pieces {
        assert_eq!(p.dimension(), 2);
        for i in 0..p.border_line_count() {
            assert!(ps.contains(&p.corner(i)));
        }
    }
    assert!(ps.contains(&ip(10, 40)));
    assert!(!ps.contains(&ip(40, 40)));
    // cached: the same pieces are returned again
    let again = ps.split_to_convex().unwrap();
    assert_eq!(again.len(), pieces.len());
    let hull = ps.convex_hull();
    assert!(hull.is_convex());
    assert_eq!(hull.corners.len(), 5);
}

#[test]
fn polyline_area_with_hole_splits_around_the_hole() {
    let border = PolylineShape::Tile(TileShape::IntBox(IntBox::new(0, 0, 100, 100)));
    let hole = PolylineShape::Tile(TileShape::IntBox(IntBox::new(40, 40, 60, 60)));
    let area = PolylineArea::new(border, vec![hole]);
    let pieces = area.split_to_convex().expect("pieces");
    let total: f64 = pieces.iter().map(|p| p.area()).sum();
    assert_eq!(total, 100.0 * 100.0 - 20.0 * 20.0);
    assert!(!area.contains(&ip(50, 50)));
    assert!(area.contains(&ip(40, 50))); // on the hole border: contained
    assert!(area.contains(&ip(10, 10)));
    for p in &pieces {
        assert!(!p.contains_inside(&ip(50, 50)));
    }
    let a = Area::PolylineArea(area.clone());
    assert_eq!(a.get_holes().len(), 1);
    assert_eq!(a.bounding_box(), IntBox::new(0, 0, 100, 100));
}

// ---------------------------------------------------------------------------------------------
// Circle, directions, misc
// ---------------------------------------------------------------------------------------------

#[test]
fn circle_bounds_contain_the_circle() {
    let c = Circle::new(IntPoint::new(100, -50), 37);
    let o = c.bounding_octagon();
    assert_eq!(o, IntOctagon::new(63, -87, 137, -13, 98, 203, -2, 103));
    // Java quirk (preserved): the upper-left and lower-left diagonals use floor instead of ceil,
    // so the octagon cuts the circle slightly there; the other sides contain it.
    let at = |deg: f64| {
        let a = deg.to_radians();
        FloatPoint::new(100.0 + 37.0 * a.cos(), -50.0 + 37.0 * a.sin())
    };
    assert!(!o.contains_float(&at(135.0)));
    assert!(!o.contains_float(&at(225.0)));
    for deg in [0.0, 30.0, 45.0, 60.0, 90.0, 180.0, 270.0, 300.0, 315.0] {
        assert!(o.contains_float(&at(deg)), "{deg}");
    }
    let tile = c.bounding_tile_max(10);
    assert!(tile.border_line_count() > 8);
    assert!(
        Shape::Circle(c).intersects(&Shape::Tile(TileShape::IntBox(IntBox::new(
            130, -50, 200, 0
        ))))
    );
    assert!(
        !Shape::Circle(c).intersects(&Shape::Tile(TileShape::IntBox(IntBox::new(
            140, -50, 200, 0
        ))))
    );
    assert_eq!(Circle::new(IntPoint::new(0, 0), -5).radius, 5);
}

#[test]
fn polygon_intersects_polygon_panics_like_java_stack_overflow() {
    let p = Shape::Polygon(PolygonShape::from_points(&[ip(0, 0), ip(10, 0), ip(0, 10)]));
    let r = std::panic::catch_unwind(|| p.intersects(&p));
    assert!(r.is_err());
}

#[test]
fn java_random_matches_polygon_split_seed() {
    // PolygonShape.splitToConvex uses new Random(99); first values for bound 6 and 5
    let mut r = JavaRandom::new(99);
    let a = r.next_int(6);
    let b = r.next_int(5);
    assert!((0..6).contains(&a) && (0..5).contains(&b));
}

/// Calls every public TileShape / Shape / Area method on every variant to make sure the trait
/// forwarding contains no accidental recursion and no panics for well formed bounded shapes.
#[test]
fn smoke_test_all_methods_on_all_variants() {
    let tri = TileShape::get_instance_points(&[ip(0, 0), ip(40, 0), ip(10, 30)]);
    let shapes = vec![
        TileShape::IntBox(IntBox::new(0, 0, 30, 20)),
        TileShape::IntOctagon(IntOctagon::new(0, 0, 30, 30, -20, 20, 10, 50).normalize()),
        tri,
    ];
    let p = ip(5, 5);
    let fp = FloatPoint::new(5.0, 5.0);
    let pole = IntPoint::new(3, 4);
    let pl = Polyline::from_points(&[ip(-10, 5), ip(50, 5), ip(50, 60)]);
    for s in &shapes {
        let other = TileShape::IntBox(IntBox::new(10, 5, 60, 60));
        let _ = (
            s.border_line_count(),
            s.corner(0),
            s.border_line(0),
            s.corner_is_bounded(0),
        );
        let _ = (
            s.is_empty(),
            s.is_bounded(),
            s.dimension(),
            s.bounding_box(),
            s.corner_approx(1),
        );
        let _ = (
            s.corner_approx_arr(),
            s.bounded_corners(),
            s.equals_corner(&p),
            s.circumference(),
        );
        let _ = (
            s.centre_of_gravity(),
            s.is_contained_in(&IntBox::new(-100, -100, 100, 100)),
        );
        let _ = (
            s.index_of_left_most_corner(&fp),
            s.index_of_right_most_corner(&fp),
            s.polar_line_segment(&fp),
        );
        let _ = (
            s.prev_no(0),
            s.next_no(0),
            s.intersects_line(&line(0, 0, 1, 1)),
        );
        let _ = (
            s.left_most_corner(&ip(100, 100)),
            s.right_most_corner(&ip(100, 100)),
        );
        let _ = (
            s.simplify(),
            s.get_id(),
            s.is_int_box(),
            s.is_int_octagon(),
            s.border_line_index(&line(0, 0, 1, 0)),
        );
        let _ = (
            s.to_simplex(),
            s.offset(2.0),
            s.offset(-2.0),
            s.max_width(),
            s.min_width(),
        );
        let _ = (
            s.translate_by(&Vector::Int(IntVector::new(3, 4))),
            s.bounding_octagon(),
            s.enlarge(2.0),
        );
        let _ = (
            s.bounding_shape(&OrthogonalBoundingDirections::INSTANCE),
            s.bounding_shape(&FortyfiveDegreeBoundingDirections::INSTANCE),
        );
        let _ = (
            s.area(),
            s.is_outside(&p),
            s.contains(&p),
            s.contains_float(&fp),
            s.contains_float_tol(&fp, 0.5),
        );
        let _ = (
            s.contains_tile_shape(&other),
            s.contains_inside(&p),
            s.side_of_border(&fp, 0.1),
        );
        let _ = (
            s.contains_on_border_line_no(&p),
            s.contains_on_border(&p),
            s.contains_approx(&other),
        );
        let _ = (
            s.distance(&FloatPoint::new(100.0, 100.0)),
            s.border_distance(&fp),
            s.smallest_radius(),
        );
        let _ = (
            s.nearest_point(&ip(100, 100)),
            s.nearest_point_approx(&fp),
            s.nearest_border_point(&p),
        );
        let _ = (
            s.nearest_border_point_approx(&fp),
            s.nearest_border_points_approx(&fp, 3),
            s.index_of_nearest_corner(&p),
        );
        let _ = (
            s.diagonal_corner_segment(),
            s.nearest_relative_outside_locations(&other, 2),
            s.shrink(1.0),
            s.length(),
        );
        let _ = (
            s.touching_sides(&other),
            s.distance_to_the_left(&line(0, 100, 1, 100)),
            s.side_of(&line(0, 0, 1, 1)),
        );
        let _ = (
            s.turn_90_degree(1, &pole),
            s.rotate_approx(0.3, &pole.to_float()),
            s.mirror_vertical(&pole),
            s.mirror_horizontal(&pole),
        );
        let _ = (
            s.intersecting_border_line_no(&p, &Direction::RIGHT),
            s.cutout_polyline(&pl),
            s.entrance_points(&pl),
        );
        let _ = (
            s.split_to_convex(),
            s.divide_into_sections(7.0),
            s.intersection_with_simplify(&other),
        );
        let seg = LineSegment::new(line(-5, 0, -5, 1), line(-5, 2, 60, 3), line(60, 0, 60, 1));
        let _ = (
            s.is_intersected_interior_by(&seg),
            s.intersection(&other),
            s.cutout(&other),
            other.cutout(s),
        );
        let _ = (
            s.intersects(&Shape::Circle(Circle::new(IntPoint::new(0, 0), 5))),
            s.get_border(),
            s.get_holes(),
        );
        let shape = Shape::Tile(s.clone());
        let _ = (
            shape.enlarge(1.0),
            shape.bounding_tile(),
            shape.split_to_convex(),
            shape.cutout_polyline(&pl),
        );
        let area = Area::Shape(shape.clone());
        let _ = (
            area.turn_90_degree(2, &pole),
            area.corner_approx_arr(),
            area.nearest_point_approx(&fp),
        );
        let convex = s.to_convex_shape();
        let _ = (
            convex.offset(1.0),
            convex.shrink(1.0),
            convex.bounding_shape(&ShapeBoundingDirections::Orthogonal),
        );
    }
    let circle = Shape::Circle(Circle::new(IntPoint::new(0, 0), 10));
    let polygon = Shape::Polygon(PolygonShape::from_points(&[
        ip(0, 0),
        ip(40, 0),
        ip(40, 40),
        ip(20, 10),
        ip(0, 40),
    ]));
    for s in [&circle, &polygon] {
        let _ = (
            s.is_empty(),
            s.dimension(),
            s.bounding_box(),
            s.bounding_octagon(),
            s.contains(&p),
            s.contains_float(&fp),
        );
        let _ = (
            s.circumference(),
            s.area(),
            s.centre_of_gravity(),
            s.is_outside(&p),
            s.contains_inside(&p),
        );
        let _ = (
            s.distance(&fp),
            s.bounding_tile(),
            s.border_distance(&fp),
            s.smallest_radius(),
            s.enlarge(0.0),
        );
        let _ = (
            s.split_to_convex(),
            s.turn_90_degree(1, &pole),
            s.mirror_vertical(&pole),
            s.translate_by(&Vector::ZERO),
        );
        let _ = s.intersects(&Shape::Tile(TileShape::IntBox(IntBox::new(0, 0, 5, 5))));
        let _ = s.intersects_int_octagon(&IntBox::new(0, 0, 5, 5).to_int_octagon());
    }
}
