//! Tests of `PlanarDelaunayTriangulation`: JDK ground truth (edge lists of the real Java class,
//! see `testdata/DatastructuresGen.java`) and structural validity on random point sets.

use super::*;
use fr_geom::{IntPoint, Point};
use fr_jcompat::JavaRandom;
use std::collections::BTreeSet;

type Tri = PlanarDelaunayTriangulation<i32, Point>;

fn fmt_edges(edges: &[ResultEdge<i32, Point>]) -> Vec<String> {
    let xy = |p: &Point| match p {
        Point::Int(p) => format!("{} {}", p.x, p.y),
        _ => panic!("rational point"),
    };
    edges
        .iter()
        .map(|e| {
            format!(
                "{} {} {} {}",
                e.start_object.unwrap_or(-1),
                xy(&e.start_point),
                e.end_object.unwrap_or(-1),
                xy(&e.end_point)
            )
        })
        .collect()
}

#[test]
fn jdk_ground_truth_edge_lines() {
    let data = include_str!("../testdata/delaunay.txt");
    let mut lines = data.lines();
    // the "static" generator is deliberately left in a random state between runs
    let mut random = JavaRandom::new(12345);
    let mut cases = 0;
    while let Some(header) = lines.next() {
        let n: usize = header.strip_prefix("D ").unwrap().parse().unwrap();
        let mut objects: Vec<(i32, Vec<Point>)> = Vec::new();
        for _ in 0..n {
            let t: Vec<i32> = lines.next().unwrap()[2..].split(' ').map(|s| s.parse().unwrap()).collect();
            let pts = (0..t[1] as usize).map(|k| Point::Int(IntPoint::new(t[2 + 2 * k], t[3 + 2 * k]))).collect();
            objects.push((t[0], pts));
        }
        let count: usize = lines.next().unwrap().strip_prefix("E ").unwrap().parse().unwrap();
        let expected: Vec<String> = (0..count).map(|_| lines.next().unwrap().to_string()).collect();

        let tri = Tri::new(objects.iter().cloned(), &mut random);
        random.next_int(); // disturb the shared generator
        assert_eq!(fmt_edges(&tri.get_edge_lines()), expected, "case {cases} ({n} objects)");
        assert!(tri.validate_leaves(), "case {cases}");
        cases += 1;
    }
    assert!(cases >= 30);
}

/// Twice the signed area of the triangle (a, b, c), exact.
fn orient(a: &IntPoint, b: &IntPoint, c: &IntPoint) -> i128 {
    (b.x as i128 - a.x as i128) * (c.y as i128 - a.y as i128) - (b.y as i128 - a.y as i128) * (c.x as i128 - a.x as i128)
}

/// Exact in-circle test: > 0 if d is strictly inside the circle through the ccw triangle a,b,c.
fn in_circle(a: &IntPoint, b: &IntPoint, c: &IntPoint, d: &IntPoint) -> i128 {
    let (adx, ady) = (a.x as i128 - d.x as i128, a.y as i128 - d.y as i128);
    let (bdx, bdy) = (b.x as i128 - d.x as i128, b.y as i128 - d.y as i128);
    let (cdx, cdy) = (c.x as i128 - d.x as i128, c.y as i128 - d.y as i128);
    let ad = adx * adx + ady * ady;
    let bd = bdx * bdx + bdy * bdy;
    let cd = cdx * cdx + cdy * cdy;
    adx * (bdy * cd - bd * cdy) - ady * (bdx * cd - bd * cdx) + ad * (bdx * cdy - bdy * cdx)
}

#[test]
fn random_point_sets_give_valid_delaunay_triangulations() {
    let mut r = JavaRandom::new(2024);
    let mut max_violations = 0;
    let mut total_illegal = 0;
    for case in 0..40 {
        let n = 3 + r.next_int_bound(150) as usize;
        let range = [50, 1000, 100_000][case % 3];
        let mut distinct = BTreeSet::new();
        let mut objects: Vec<(i32, Vec<IntPoint>)> = Vec::new();
        for id in 0..n as i32 {
            let p = IntPoint::new(r.next_int_bound(2 * range) - range, r.next_int_bound(2 * range) - range);
            distinct.insert((p.x, p.y));
            objects.push((id, vec![p]));
        }
        let tri: PlanarDelaunayTriangulation<i32, IntPoint> =
            PlanarDelaunayTriangulation::new(objects.iter().cloned(), &mut JavaRandom::new(0));
        assert!(tri.validate_leaves());
        let triangles = tri.leaf_triangles();
        // Euler: n distinct points inside the bounding triangle give 2n + 1 triangles
        assert_eq!(triangles.len(), 2 * distinct.len() + 1, "case {case}");
        let all_points: Vec<IntPoint> = distinct.iter().map(|&(x, y)| IntPoint::new(x, y)).collect();
        let mut violations = 0;
        for t in &triangles {
            let [a, b, c] = [&t[0].1, &t[1].1, &t[2].1];
            assert!(orient(a, b, c) > 0, "case {case}: triangle not counter clockwise");
            // Delaunay property for triangles of input points. Java's in-circle test
            // (`FloatPoint.insideCircle`) computes the centre from edge slopes, which is NaN or
            // infinite for horizontal/vertical edges (such edges are then never flipped), so a
            // few violations are expected; they are faithful to Java (see the JDK test above).
            if t.iter().all(|c| c.0.is_some())
                && all_points.iter().any(|p| p != a && p != b && p != c && in_circle(a, b, c, p) > 0)
            {
                violations += 1;
            }
        }
        if range >= 100_000 {
            // axis parallel edges are rare with large coordinates
            assert!(violations * 20 <= triangles.len(), "case {case}: {violations} of {} triangles not Delaunay", triangles.len());
        }
        max_violations = max_violations.max(violations);
        total_illegal += tri.illegal_edge_count();
        // result edges: exactly the edges of triangles between input points, no duplicates
        let edges = tri.get_edge_lines();
        let mut from_triangles = BTreeSet::new();
        for t in &triangles {
            for i in 0..3 {
                let (u, v) = (&t[i], &t[(i + 1) % 3]);
                if u.0.is_some() && v.0.is_some() {
                    let (a, b) = ((u.1.x, u.1.y), (v.1.x, v.1.y));
                    from_triangles.insert(if a < b { (a, b) } else { (b, a) });
                }
            }
        }
        let mut from_result = BTreeSet::new();
        let mut degenerate = 0;
        for e in &edges {
            let (a, b) = ((e.start_point.x, e.start_point.y), (e.end_point.x, e.end_point.y));
            if a == b {
                degenerate += 1;
                continue;
            }
            assert!(from_result.insert(if a < b { (a, b) } else { (b, a) }), "duplicate edge");
        }
        assert_eq!(from_result, from_triangles);
        assert_eq!(degenerate, n - distinct.len());
    }
    eprintln!("max non-Delaunay triangles per case: {max_violations}, edges illegal by Java's test: {total_illegal}");
}

#[test]
fn small_inputs() {
    let t: PlanarDelaunayTriangulation<i32, IntPoint> =
        PlanarDelaunayTriangulation::new_with_own_random(Vec::<(i32, Vec<IntPoint>)>::new());
    assert!(t.get_edge_lines().is_empty());
    assert!(t.validate());
    let t: PlanarDelaunayTriangulation<i32, IntPoint> =
        PlanarDelaunayTriangulation::new_with_own_random(vec![(1, vec![IntPoint::new(0, 0)])]);
    assert!(t.get_edge_lines().is_empty());
    // equal corners of one object are ignored, of different objects give a degenerate edge to
    // the corner that was inserted first
    let t: PlanarDelaunayTriangulation<i32, IntPoint> = PlanarDelaunayTriangulation::new_with_own_random(vec![
        (1, vec![IntPoint::new(5, 5)]),
        (2, vec![IntPoint::new(5, 5)]),
        (2, vec![IntPoint::new(5, 5)]),
    ]);
    let e = t.get_edge_lines();
    assert!(!e.is_empty() && e.len() <= 2);
    for e in &e {
        assert_eq!(e.start_point, e.end_point);
        assert_ne!(e.start_object, e.end_object);
    }
}
