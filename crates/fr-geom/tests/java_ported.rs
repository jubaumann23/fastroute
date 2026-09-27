//! Ports of the Java unit tests in `src/test/java/app/freerouting/geometry/planar`.

use fr_geom::*;
use num_bigint::BigInt;

// ---- IntOctagonTest ----

#[test]
fn normalize_returns_same_instance_when_bounds_are_already_tight() {
    let octagon = IntOctagon::new(0, 0, 10, 10, -10, 10, 0, 20);
    // Java asserts reference identity (assertSame); for the Copy value type the equivalent is
    // that normalize() changes nothing and still is not the EMPTY instance.
    let normalized = octagon.normalize();
    assert_eq!(octagon, normalized);
    assert!(!normalized.is_empty());
}

#[test]
fn normalize_still_allocates_when_bounds_need_tightening() {
    let octagon = IntOctagon::new(0, 0, 10, 20, -10, 10, 0, 20);
    let normalized = octagon.normalize();
    assert_ne!(octagon, normalized);
    assert_eq!(15, normalized.top_y);
}

// ---- PointEqualsHashCodeTest ----

fn hash_of<T: std::hash::Hash>(t: &T) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

fn rational(x: i64, y: i64, z: i64) -> RationalPoint {
    RationalPoint::new(BigInt::from(x), BigInt::from(y), BigInt::from(z))
}

#[test]
fn int_point_equals_and_hash_code_contract() {
    let p1 = IntPoint::new(100, 200);
    let p2 = IntPoint::new(100, 200);
    assert_eq!(p1, p2);
    assert_eq!(p1.hash_code(), p2.hash_code());
    assert_eq!(hash_of(&p1), hash_of(&p2));

    let p3 = IntPoint::new(100, 201);
    assert_ne!(p1, p3);
    let p4 = IntPoint::new(101, 200);
    assert_ne!(p1, p4);
    // Java: new IntPoint(100, 200).hashCode() == 31 * 100 + 200
    assert_eq!(p1.hash_code(), 3300);
    // An IntPoint never equals a RationalPoint (Java getClass() check).
    assert_ne!(
        Point::Int(p1),
        Point::Rational(Box::new(rational(100, 200, 1)))
    );
}

#[test]
fn rational_point_equals_and_hash_code_contract() {
    let p1 = rational(100, 200, 50);
    let p2 = rational(2, 4, 1);
    assert_eq!(p1, p2);
    assert_eq!(p1.hash_code(), p2.hash_code());
    assert_eq!(hash_of(&p1), hash_of(&p2));

    let p3 = rational(-4, 6, 2);
    let p4 = rational(-2, 3, 1);
    assert_eq!(p3, p4);
    assert_eq!(p3.hash_code(), p4.hash_code());

    assert_ne!(p1, p3);
}

#[test]
fn rational_point_infinite_points() {
    let inf1 = rational(10, 20, 0);
    let inf2 = rational(30, 40, 0);
    assert_eq!(inf1, inf2);
    assert_eq!(inf1.hash_code(), inf2.hash_code());
    assert!(inf1.is_infinite());
}

#[test]
fn geometry_types_are_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Point>();
    assert_send_sync::<Line>();
    assert_send_sync::<Simplex>();
    assert_send_sync::<TileShape>();
    assert_send_sync::<Shape>();
    assert_send_sync::<Area>();
    assert_send_sync::<Polyline>();
    assert_send_sync::<LineSegment>();
}

// ---- SealedGeometryHierarchyTest ----
// The Java test checks that the hierarchies are sealed. In Rust the enums are closed by
// construction; these exhaustive matches fail to compile if a variant is added or removed.

#[test]
fn point_hierarchy_has_only_finite_and_rational_implementations() {
    let points = [
        Point::Int(IntPoint::new(1, 2)),
        Point::Rational(Box::new(rational(1, 2, 3))),
    ];
    for p in &points {
        match p {
            Point::Int(_) | Point::Rational(_) => {}
        }
    }
}

#[test]
fn tile_shape_hierarchy_has_only_regular_and_simplex_branches() {
    let tiles = [
        TileShape::IntBox(IntBox::new(0, 0, 1, 1)),
        TileShape::IntOctagon(IntOctagon::EMPTY),
        TileShape::Simplex(Simplex::empty()),
    ];
    for t in &tiles {
        match t.as_regular_tile_shape() {
            Some(RegularTileShape::IntBox(_)) | Some(RegularTileShape::IntOctagon(_)) => {}
            None => assert!(matches!(t, TileShape::Simplex(_))),
        }
    }
}
