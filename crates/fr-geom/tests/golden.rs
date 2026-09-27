//! Differential tests against the Java implementation.
//!
//! `tests/data/golden.txt` was produced by running the original Java classes of
//! `app.freerouting.geometry.planar` (compiled with a no-op `FRLogger` stub) through a generator
//! (`tests/data/Gen.java`) on random inputs. Every line has the form
//! `OP <inputs> | <outputs>`; this test recomputes the outputs with the Rust port and compares
//! the serialized strings exactly (doubles are compared bit for bit). An output of `EXC` means
//! the Java code threw, in which case the Rust port must panic; `HANG` marks inputs for which the
//! Java code loops forever (these are skipped).
//!
//! Regenerating (from the repository root, with a JDK >= 17):
//! ```text
//! mkdir -p /tmp/g/src/app/freerouting/{geometry/planar,datastructures,logger}
//! cp reference/freerouting/src/main/java/app/freerouting/geometry/planar/*.java \
//!    crates/fr-geom/tests/data/Gen.java /tmp/g/src/app/freerouting/geometry/planar/
//! cp reference/freerouting/src/main/java/app/freerouting/datastructures/{BigIntAux,Signum,Stoppable}.java \
//!    /tmp/g/src/app/freerouting/datastructures/
//! # plus a stub app/freerouting/logger/FRLogger.java with no-op static warn/debug/trace methods
//! javac -d /tmp/g/out $(find /tmp/g/src -name '*.java')
//! java -cp /tmp/g/out app.freerouting.geometry.planar.Gen 20260927 150 > crates/fr-geom/tests/data/golden.txt
//! ```
//! Larger runs can be checked without committing them: `FR_GEOM_GOLDEN=<file> cargo test`.

use std::panic::{catch_unwind, AssertUnwindSafe};

use fr_geom::prelude::*;
use fr_geom::*;
use num_bigint::BigInt;

struct Tok<'a> {
    t: Vec<&'a str>,
    pos: usize,
}

impl<'a> Tok<'a> {
    fn new(s: &'a str) -> Self {
        Tok {
            t: s.split_whitespace().collect(),
            pos: 0,
        }
    }
    fn next(&mut self) -> &'a str {
        let r = self.t[self.pos];
        self.pos += 1;
        r
    }
    fn expect(&mut self, s: &str) {
        let n = self.next();
        assert_eq!(n, s, "token mismatch at {}", self.pos);
    }
    fn i32(&mut self) -> i32 {
        self.next().parse().unwrap()
    }
    fn f64(&mut self) -> f64 {
        f64::from_bits(u64::from_str_radix(self.next(), 16).unwrap())
    }
    fn bool(&mut self) -> bool {
        self.next().parse().unwrap()
    }
    fn point(&mut self) -> Point {
        match self.next() {
            "I" => {
                let x = self.i32();
                let y = self.i32();
                Point::Int(IntPoint::new(x, y))
            }
            "R" => {
                let x: BigInt = self.next().parse().unwrap();
                let y: BigInt = self.next().parse().unwrap();
                let z: BigInt = self.next().parse().unwrap();
                Point::Rational(Box::new(RationalPoint::new(x, y, z)))
            }
            t => panic!("bad point token {t}"),
        }
    }
    fn int_point(&mut self) -> IntPoint {
        self.point().as_int()
    }
    fn line(&mut self) -> Line {
        self.expect("L");
        let a = self.point();
        let b = self.point();
        Line::new(a, b)
    }
    fn int_box(&mut self) -> IntBox {
        self.expect("B");
        IntBox::new(self.i32(), self.i32(), self.i32(), self.i32())
    }
    fn octagon(&mut self) -> IntOctagon {
        self.expect("O");
        let v: Vec<i32> = (0..8).map(|_| self.i32()).collect();
        let empty = self.i32() == 1;
        if empty {
            let o = IntOctagon::EMPTY;
            assert_eq!(
                (
                    o.left_x,
                    o.bottom_y,
                    o.right_x,
                    o.top_y,
                    o.upper_left_diagonal_x
                ),
                (v[0], v[1], v[2], v[3], v[4])
            );
            o
        } else {
            IntOctagon::new(v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7])
        }
    }
    fn tile(&mut self) -> TileShape {
        match self.t[self.pos] {
            "B" => TileShape::IntBox(self.int_box()),
            "O" => TileShape::IntOctagon(self.octagon()),
            "S" => {
                self.next();
                let n = self.i32();
                let lines: Vec<Line> = (0..n).map(|_| self.line()).collect();
                TileShape::Simplex(Simplex::new(lines))
            }
            t => panic!("bad tile token {t}"),
        }
    }
}

fn d(v: f64) -> String {
    format!("{:x}", v.to_bits())
}
fn pt(p: &Point) -> String {
    match p {
        Point::Int(p) => format!("I {} {}", p.x, p.y),
        Point::Rational(r) => format!("R {} {} {}", r.x, r.y, r.z),
    }
}
fn ipt(p: &IntPoint) -> String {
    format!("I {} {}", p.x, p.y)
}
fn opt_pt(p: Option<&Point>) -> String {
    p.map(pt).unwrap_or_else(|| "N".into())
}
fn fp(p: &FloatPoint) -> String {
    format!("F {} {}", d(p.x), d(p.y))
}
fn opt_fp(p: Option<&FloatPoint>) -> String {
    p.map(fp).unwrap_or_else(|| "N".into())
}
fn ln(l: &Line) -> String {
    format!("L {} {}", pt(&l.a), pt(&l.b))
}
fn oct(o: &IntOctagon) -> String {
    format!(
        "O {} {} {} {} {} {} {} {} {}",
        o.left_x,
        o.bottom_y,
        o.right_x,
        o.top_y,
        o.upper_left_diagonal_x,
        o.lower_right_diagonal_x,
        o.lower_left_diagonal_x,
        o.upper_right_diagonal_x,
        if o.is_empty() { 1 } else { 0 }
    )
}
fn opt_oct(o: Option<&IntOctagon>) -> String {
    o.map(oct).unwrap_or_else(|| "N".into())
}
fn bx(b: &IntBox) -> String {
    format!("B {} {} {} {}", b.ll.x, b.ll.y, b.ur.x, b.ur.y)
}
fn tile(t: &TileShape) -> String {
    match t {
        TileShape::IntBox(b) => bx(b),
        TileShape::IntOctagon(o) => oct(o),
        TileShape::Simplex(s) => {
            let mut r = format!("S {}", s.border_line_count());
            for i in 0..s.border_line_count() {
                r.push(' ');
                r.push_str(&ln(&s.border_line(i)));
            }
            r
        }
    }
}
fn tiles(ts: Option<&[TileShape]>) -> String {
    match ts {
        None => "N".into(),
        Some(ts) => {
            let mut r = format!("A {}", ts.len());
            for t in ts {
                r.push(' ');
                r.push_str(&tile(t));
            }
            r
        }
    }
}
fn dir(dd: Option<&Direction>) -> String {
    match dd {
        None => "N".into(),
        Some(Direction::Int(i)) => format!("D {} {}", i.x, i.y),
        Some(Direction::BigInt(b)) => format!("BD {} {}", b.x, b.y),
    }
}
fn side(s: Side) -> &'static str {
    match s {
        Side::OnTheLeft => "onTheLeft",
        Side::OnTheRight => "onTheRight",
        Side::Collinear => "collinear",
    }
}
fn signum(s: Signum) -> &'static str {
    match s {
        Signum::Positive => "positive",
        Signum::Negative => "negative",
        Signum::Zero => "zero",
    }
}
fn int_arr(v: &[i32]) -> String {
    format!(
        "[{}]",
        v.iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn run_case(op: &str, t: &mut Tok) -> String {
    match op {
        "LINE" => {
            let l1 = t.line();
            let l2 = t.line();
            let p = t.int_point();
            let tr = t.f64();
            let pp = Point::Int(p);
            format!(
                "{} {} {} {} {} {} {} {} {} {} {}",
                pt(&l1.intersection(&l2)),
                fp(&l1.intersection_approx(&l2)),
                side(l1.side_of(&pp)),
                l1.compare_to(&l2),
                l1.is_parallel(&l2),
                l1 == l2,
                dir(Some(&l1.direction())),
                d(l1.signed_distance(&p.to_float())),
                pt(&p.perpendicular_projection(&l1)),
                dir(l1.perpendicular_direction(&pp).as_ref()),
                ln(&l1.translate(tr))
            )
        }
        "SIMPLEX" => {
            let m = t.i32();
            let ls: Vec<Line> = (0..m).map(|_| t.line()).collect();
            let s = Simplex::get_instance(&ls);
            let ts = TileShape::Simplex(s.clone());
            let mut r = format!(
                "{} {} {} {}",
                tile(&ts),
                s.dimension(),
                s.is_bounded(),
                tile(&s.simplify())
            );
            r.push_str(" C");
            for i in 0..s.border_line_count() {
                r.push_str(&format!(
                    " {} {}",
                    pt(&s.corner(i)),
                    fp(&s.corner_approx(i))
                ));
            }
            r.push_str(&format!(" {}", bx(&s.bounding_box())));
            r.push_str(&format!(" {}", opt_oct(s.bounding_octagon().as_ref())));
            r.push_str(&format!(" {} {}", d(ts.area()), d(ts.circumference())));
            r
        }
        "TILE2" => {
            let a = t.tile();
            let b = t.tile();
            format!(
                "{} {} {} {} {} {}",
                tile(&a.intersection(&b)),
                tile(&a.intersection_with_simplify(&b)),
                a.intersects(&Shape::Tile(b.clone())),
                tiles(a.cutout(&b).as_deref()),
                a.contains_tile_shape(&b),
                int_arr(&a.touching_sides(&b))
            )
        }
        "TILE1" => {
            let a = t.tile();
            let p = t.int_point();
            let off = t.f64();
            let fac = t.i32();
            let tv = IntVector::new(t.i32(), t.i32());
            let pp = Point::Int(p);
            let mut r = format!(
                "{} {} {} {} {} {} {}",
                a.dimension(),
                d(a.area()),
                fp(&a.centre_of_gravity()),
                a.contains(&pp),
                a.contains_inside(&pp),
                a.is_outside(&pp),
                a.contains_on_border_line_no(&pp)
            );
            r.push_str(&format!(
                " {}",
                opt_pt(a.nearest_border_point(&pp).as_ref())
            ));
            let nb = a.nearest_border_points_approx(&p.to_float(), 2);
            r.push_str(&format!(" NB {}", nb.len()));
            for f in &nb {
                r.push_str(&format!(" {}", opt_fp(f.as_ref())));
            }
            r.push_str(&format!(" {}", d(a.distance(&p.to_float()))));
            r.push_str(&format!(" {}", tile(&a.offset(off))));
            r.push_str(&format!(" {}", tile(&a.shrink(off.abs()))));
            r.push_str(&format!(" {}", tile(&a.enlarge(off.abs()))));
            r.push_str(&format!(" {}", tile(&a.turn_90_degree(fac, &p))));
            r.push_str(&format!(
                " {} {}",
                tile(&a.mirror_vertical(&p)),
                tile(&a.mirror_horizontal(&p))
            ));
            r.push_str(&format!(" {}", tile(&a.translate_by(&Vector::Int(tv)))));
            r.push_str(&format!(
                " {} {} {}",
                d(a.length()),
                d(a.max_width()),
                d(a.min_width())
            ));
            r.push_str(&format!(
                " {} {}",
                bx(&a.bounding_box()),
                opt_oct(a.bounding_octagon().as_ref())
            ));
            r.push_str(&format!(" {}", a.get_id()));
            r
        }
        "OCT" => {
            let a = t.octagon();
            let b = t.octagon();
            let bxx = t.int_box();
            let off = t.f64();
            let c = t.int_point();
            let na = a.normalize();
            let nb = b.normalize();
            let to_tiles =
                |v: Vec<IntOctagon>| v.into_iter().map(TileShape::IntOctagon).collect::<Vec<_>>();
            let mut r = format!(
                "{} {} {} {} {} {}",
                oct(&na),
                oct(&nb),
                oct(&na.intersection_int_octagon(&nb)),
                na.intersects_int_octagon(&nb),
                na.overlaps(&nb),
                oct(&na.offset(off))
            );
            r.push_str(&format!(
                " {}",
                tiles(Some(&to_tiles(na.cutout_from_int_octagon(&nb))))
            ));
            r.push_str(&format!(
                " {}",
                tiles(Some(&to_tiles(na.cutout_from_int_box(&bxx))))
            ));
            let boxes: Vec<TileShape> = bxx
                .cutout_from_int_box(&bxx.intersection_int_box(&na.bounding_box()))
                .into_iter()
                .map(TileShape::IntBox)
                .collect();
            r.push_str(&format!(" {}", tiles(Some(&boxes))));
            r.push_str(&format!(
                " {} {} {}",
                oct(&na.union_int_octagon(&nb)),
                na.is_int_box(),
                d(TileShapeImpl::area(&na))
            ));
            r.push_str(&format!(
                " {} {} {}",
                tile(&TileShape::Simplex(na.to_simplex())),
                a.is_empty(),
                a.dimension()
            ));
            let pr = na.nearest_border_projections(&c, 3);
            r.push_str(&format!(" P {}", pr.len()));
            for q in &pr {
                r.push_str(&format!(" {}", ipt(q)));
            }
            for fd in FortyfiveDegreeDirection::VALUES {
                r.push_str(&format!(" {}", ipt(&na.border_point(&c, fd))));
            }
            r
        }
        "POLYLINE" => {
            let m = t.i32();
            let pts: Vec<Point> = (0..m).map(|_| t.point()).collect();
            let hw = t.i32();
            let q = t.int_point();
            let tt = t.tile();
            let pl = Polyline::from_points(&pts);
            let mut r = format!("{}", pl.lines.len());
            for l in pl.lines.iter() {
                r.push_str(&format!(" {}", ln(l)));
            }
            if pl.lines.len() >= 3 {
                r.push_str(" C");
                for i in 0..pl.corner_count() {
                    r.push_str(&format!(" {}", pt(&pl.corner(i))));
                }
                r.push_str(&format!(
                    " {} {}",
                    d(pl.length_approx()),
                    bx(&pl.bounding_box())
                ));
                r.push_str(&format!(" {}", tiles(Some(&pl.offset_shapes(hw)))));
                let rev = pl.reverse();
                r.push_str(&format!(" R {}", rev.lines.len()));
                for l in rev.lines.iter() {
                    r.push_str(&format!(" {}", ln(l)));
                }
                let qq = Point::Int(q);
                match pl.projection_line(&qq) {
                    None => r.push_str(" N"),
                    Some(ls) => r.push_str(&format!(
                        " {} {}",
                        pt(&ls.start_point()),
                        pt(&ls.end_point())
                    )),
                }
                r.push_str(&format!(
                    " {}",
                    opt_fp(pl.nearest_point_approx(&q.to_float()).as_ref())
                ));
                let cut = tt.cutout_polyline(&pl);
                r.push_str(&format!(" CUT {}", cut.len()));
                for c in &cut {
                    r.push_str(&format!(" [{}", c.lines.len()));
                    for l in c.lines.iter() {
                        r.push_str(&format!(" {}", ln(l)));
                    }
                    r.push_str(" ]");
                }
                let ep = tt.entrance_points(&pl);
                r.push_str(&format!(" EP {}", ep.len()));
                for e in &ep {
                    r.push_str(&format!(" {} {}", e[0], e[1]));
                }
            }
            r
        }
        "POLYGON" => {
            let m = t.i32();
            let pts: Vec<Point> = (0..m).map(|_| t.point()).collect();
            let ps = PolygonShape::from_points(&pts);
            let mut r = format!("{}", ps.corners.len());
            for q in ps.corners.iter() {
                r.push_str(&format!(" {}", pt(q)));
            }
            r.push_str(&format!(
                " {} {}",
                ps.is_convex(),
                tiles(ps.split_to_convex().as_deref())
            ));
            let h = ps.convex_hull();
            r.push_str(&format!(" H {}", h.corners.len()));
            for q in h.corners.iter() {
                r.push_str(&format!(" {}", pt(q)));
            }
            r.push_str(&format!(
                " {} {}",
                tile(&ps.bounding_tile()),
                Polygon::new(&pts).winding_number_after_closing()
            ));
            r
        }
        "ROUND" => {
            let v = t.f64();
            format!(
                "{} {} {} {}",
                java_compat::math_round(v),
                java_compat::math_round_i32(v),
                d(java_compat::math_rint(v)),
                v as i32
            )
        }
        "CIRCLE" => {
            let c = t.int_point();
            let rad = t.i32();
            let ms = t.i32();
            let ci = Circle::new(c, rad);
            format!(
                "{} {} {}",
                oct(&ci.bounding_octagon()),
                tile(&ci.bounding_tile_max(ms)),
                bx(&ci.bounding_box())
            )
        }
        "DIR" => {
            let v1 = IntVector::new(t.i32(), t.i32());
            let v2 = IntVector::new(t.i32(), t.i32());
            let tf = t.i32();
            let (vv1, vv2) = (Vector::Int(v1), Vector::Int(v2));
            let d1 = Direction::get_instance(&vv1);
            let d2 = Direction::get_instance(&vv2);
            format!(
                "{} {} {} {} {} {} {} {} {} {}",
                dir(Some(&d1)),
                dir(Some(&d2)),
                d1.compare_to(&d2),
                d1 == d2,
                side(d1.side_of(&d2)),
                signum(d1.projection(&d2)),
                dir(Some(&d1.turn_45_degree(tf))),
                dir(Some(&d1.middle_approx(&d2))),
                d(vv1.angle_approx()),
                side(vv1.side_of(&vv2))
            )
        }
        "LSEG" => {
            let s = t.line();
            let m = t.line();
            let e = t.line();
            let tt = t.tile();
            let w = t.f64();
            let right = t.bool();
            let ls = LineSegment::new(s, m, e);
            let mut r = format!(
                "{} {} {} {}",
                pt(&ls.start_point()),
                pt(&ls.end_point()),
                bx(&ls.bounding_box()),
                oct(&ls.bounding_octagon())
            );
            r.push_str(&format!(" BI {}", int_arr(&ls.border_intersections(&tt))));
            r.push_str(&format!(" {}", tt.is_intersected_interior_by(&ls)));
            let st = ls.stair_approximation(w, right);
            r.push_str(&format!(" ST {}", st.len()));
            for q in &st {
                r.push_str(&format!(" {}", ipt(q)));
            }
            let st45 = ls.stair_approximation_45(w, right);
            r.push_str(&format!(" ST45 {}", st45.len()));
            for q in &st45 {
                r.push_str(&format!(" {}", ipt(q)));
            }
            r.push_str(&format!(" {}", tile(&TileShape::Simplex(ls.to_simplex()))));
            r
        }
        "SORT" => {
            let m = t.i32();
            let mut ls: Vec<(usize, Line)> = (0..m as usize).map(|i| (i, t.line())).collect();
            java_sort::sort_by(&mut ls, |a, b| a.1.compare_to(&b.1));
            ls.iter()
                .map(|(i, _)| i.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        }
        _ => panic!("unknown op {op}"),
    }
}

thread_local! {
    static LAST_PANIC: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

#[test]
fn golden_vectors_match_java() {
    // FR_GEOM_GOLDEN=<file> runs an additional (e.g. much larger) generated vector file instead.
    let external = std::env::var("FR_GEOM_GOLDEN")
        .ok()
        .map(|p| std::fs::read_to_string(p).expect("golden file"));
    let data: &str = external
        .as_deref()
        .unwrap_or(include_str!("data/golden.txt"));
    // Silence the panic messages of the expected "EXC" cases, but remember where the last panic
    // happened for the failure report.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|info| {
        let loc = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        LAST_PANIC.with(|p| *p.borrow_mut() = format!("{loc}: {msg}"));
    }));
    let mut failures = Vec::new();
    let mut count = 0;
    for (line_no, line) in data.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let (input, expected) = line.split_once(" | ").expect("separator");
        if expected == "HANG" {
            // the Java code loops forever for this input (e.g. Simplex.calcDivisionLines);
            // the port does the same, so the case cannot be executed
            continue;
        }
        count += 1;
        let op = input.split_whitespace().next().unwrap();
        let rest = &input[op.len()..];
        let result = catch_unwind(AssertUnwindSafe(|| {
            let mut t = Tok::new(rest);
            run_case(op, &mut t)
        }));
        let got = match result {
            Ok(s) => s,
            Err(_) => "EXC".to_string(),
        };
        if got != expected {
            let panic_info = if got == "EXC" {
                LAST_PANIC.with(|p| p.borrow().clone())
            } else {
                String::new()
            };
            failures.push(format!(
                "line {}: {op} {panic_info}\n  input:    {}\n  expected: {}\n  got:      {}",
                line_no + 1,
                rest.trim(),
                expected,
                got
            ));
        }
    }
    std::panic::set_hook(prev_hook);
    assert!(count > 1000, "too few golden cases: {count}");
    if !failures.is_empty() {
        let n = failures.len();
        let shown: Vec<_> = failures.into_iter().take(40).collect();
        panic!("{n} of {count} golden cases differ:\n{}", shown.join("\n"));
    }
}
