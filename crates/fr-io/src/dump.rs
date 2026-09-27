//! Canonical text dump of a [`LoadedDesign`], in the format of the Java harness
//! `testdata/java/DumpBoard.java` (keep both in sync). Used by the parity tests.
//!
//! Items are the requests that [`InsertRequest::inserts_item`] predicts, numbered with the
//! predicted Java ids ([`InsertRequest::consumes_id`]). The Java dump is taken after
//! `normalizeAllTraces` and `adjustPlaneAutorouteSettings`; the latter is applied here, the
//! former is not (trace lines may differ, see the parity test).

use std::fmt::Write;

use fr_engine::ids::{AngleRestriction, FixedState};
use fr_engine::rules::ItemClass;
use fr_geom::{Area, Point, PolylineShape, Shape, TileShape, Vector};
use fr_jcompat::double_to_string as d;

use crate::components::{pin_center, pin_layer_range, pin_shapes, transformed_area};
use crate::loader::LoadedDesign;
use crate::plane::PlaneAdjustment;
use crate::requests::InsertRequest;

fn b(v: bool) -> &'static str {
    if v {
        "1"
    } else {
        "0"
    }
}

pub fn point(p: &Point) -> String {
    match p {
        Point::Int(ip) => format!("{},{}", ip.x, ip.y),
        Point::Rational(r) => format!("r({},{},{})", r.x, r.y, r.z),
    }
}

pub fn vector(v: &Vector) -> String {
    match v {
        Vector::Int(iv) => format!("{},{}", iv.x, iv.y),
        Vector::Rational(r) => format!("r({},{},{})", r.x, r.y, r.z),
    }
}

pub fn tile(t: &TileShape) -> String {
    match t {
        TileShape::IntBox(bx) => format!("box({},{},{},{})", bx.ll.x, bx.ll.y, bx.ur.x, bx.ur.y),
        TileShape::IntOctagon(o) => format!(
            "oct({},{},{},{},{},{},{},{})",
            o.left_x,
            o.bottom_y,
            o.right_x,
            o.top_y,
            o.upper_left_diagonal_x,
            o.lower_right_diagonal_x,
            o.lower_left_diagonal_x,
            o.upper_right_diagonal_x
        ),
        TileShape::Simplex(s) => {
            let lines: Vec<String> = (0..s.border_line_count())
                .map(|i| {
                    let l = s.border_line(i);
                    format!("{};{}", point(&l.a), point(&l.b))
                })
                .collect();
            format!("simplex[{}]", lines.join(" "))
        }
    }
}

pub fn shape(s: &Shape) -> String {
    match s {
        Shape::Tile(t) => tile(t),
        Shape::Circle(c) => format!("circle({},{},{})", c.center.x, c.center.y, c.radius),
        Shape::Polygon(p) => {
            let c: Vec<String> = p.corners.iter().map(point).collect();
            format!("polygon[{}]", c.join(" "))
        }
    }
}

pub fn polyline_shape(p: &PolylineShape) -> String {
    shape(&Shape::from(p.clone()))
}

pub fn area(a: &Area) -> String {
    match a {
        Area::Shape(s) => shape(s),
        Area::PolylineArea(pa) => {
            let mut out = format!("area({}", polyline_shape(pa.get_border()));
            for h in pa.get_holes() {
                out.push_str(" hole ");
                out.push_str(&polyline_shape(h));
            }
            out.push(')');
            out
        }
    }
}

fn fixed(f: FixedState) -> &'static str {
    match f {
        FixedState::Unfixed => "UNFIXED",
        FixedState::ShoveFixed => "SHOVE_FIXED",
        FixedState::UserFixed => "USER_FIXED",
        FixedState::SystemFixed => "SYSTEM_FIXED",
    }
}

fn nets(n: &[i32]) -> String {
    n.iter()
        .map(|x| x.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// Everything but the items, with the given `adjustPlaneAutorouteSettings` result applied.
pub fn dump_header(design: &LoadedDesign, adjustment: Option<&PlaneAdjustment>) -> String {
    let mut w = String::new();
    let mut rules = design.rules.clone();
    if let Some(adj) = adjustment {
        adj.apply_to_rules(&mut rules);
    }
    for (i, l) in design.layer_structure.layers.iter().enumerate() {
        writeln!(w, "layer {i} {} {}", l.name, b(l.is_signal)).unwrap();
    }
    let cm = &rules.clearance_matrix;
    let n = cm.get_class_count();
    for i in 0..n {
        writeln!(w, "cclass {i} {}", cm.get_name(i).unwrap_or("null")).unwrap();
    }
    for i in 0..n {
        for j in 0..n {
            let vals: Vec<String> = (0..cm.get_layer_count())
                .map(|l| cm.get_value(i, j, l, false).to_string())
                .collect();
            writeln!(w, "cval {i} {j} {}", vals.join(" ")).unwrap();
        }
    }
    for i in 0..rules.net_classes.count() {
        let c = &rules.net_classes[rules.net_classes.get(i)];
        let hw: Vec<String> = (0..c.layer_count())
            .map(|l| c.get_trace_half_width(l).to_string())
            .collect();
        let items: Vec<String> = ItemClass::VALUES
            .iter()
            .map(|ic| c.default_item_clearance_classes.get(*ic).to_string())
            .collect();
        let active: Vec<&str> = (0..c.layer_count())
            .map(|l| b(c.is_active_routing_layer(l)))
            .collect();
        let via_rule = c
            .get_via_rule()
            .map(|r| rules.via_rules[r].name.clone())
            .unwrap_or_else(|| "-".into());
        writeln!(
            w,
            "netclass {i} {} tcl={} via_rule={via_rule} hw={} items={} active={} shove={} pull={} minlen={} maxlen={} ignored={}",
            c.get_name(),
            c.get_trace_clearance_class(),
            hw.join(","),
            items.join(","),
            active.join(","),
            b(c.is_shove_fixed()),
            b(c.get_pull_tight()),
            d(c.get_minimum_trace_length()),
            d(c.get_maximum_trace_length()),
            b(c.is_ignored_by_autorouter)
        )
        .unwrap();
    }
    for net in rules.nets.iter() {
        writeln!(
            w,
            "net {} {} {} class={} plane={}",
            net.net_number,
            net.name,
            net.subnet_number,
            rules.net_classes[net.get_net_class()].get_name(),
            b(net.contains_plane())
        )
        .unwrap();
    }
    let padstacks = &design.library.padstacks;
    for id in rules.via_infos.iter() {
        let v = &rules.via_infos[id];
        let p = padstacks
            .get(v.get_padstack())
            .map(|p| p.name.as_str())
            .unwrap_or("null");
        writeln!(
            w,
            "viainfo {} {} cl={} attach={}",
            v.get_name(),
            p,
            v.get_clearance_class_index(),
            b(v.attach_smd_allowed())
        )
        .unwrap();
    }
    for id in rules.via_rules.iter() {
        let r = &rules.via_rules[id];
        let names: Vec<&str> = r
            .vias()
            .iter()
            .map(|v| rules.via_infos[*v].get_name())
            .collect();
        writeln!(w, "viarule {} {}", r.name, names.join(",")).unwrap();
    }
    let angle = match rules.get_trace_angle_restriction() {
        AngleRestriction::None => "NONE",
        AngleRestriction::FortyfiveDegree => "FORTYFIVE_DEGREE",
        AngleRestriction::NinetyDegree => "NINETY_DEGREE",
    };
    writeln!(
        w,
        "rules min_hw={} max_hw={} pin_edge={} angle={angle}",
        rules.get_min_trace_half_width(),
        rules.get_max_trace_half_width(),
        d(rules.get_pin_edge_to_turn_dist())
    )
    .unwrap();
    for p in padstacks.iter() {
        write!(
            w,
            "padstack {} {} attach={} abs={}",
            p.id,
            p.name,
            b(p.attach_allowed),
            b(p.placed_absolute)
        )
        .unwrap();
        for l in 0..p.board_layer_count() {
            let s = p
                .get_shape(l)
                .map(|s| shape(&s.to_shape()))
                .unwrap_or_else(|| "null".into());
            write!(w, " {l}:{s}").unwrap();
        }
        w.push('\n');
    }
    let vias: Vec<String> = design
        .library
        .get_via_padstacks()
        .iter()
        .map(|id| {
            padstacks
                .get(*id)
                .map(|p| p.name.clone())
                .unwrap_or_default()
        })
        .collect();
    writeln!(w, "viapadstacks {}", vias.join(",")).unwrap();
    for p in design.library.packages.iter() {
        writeln!(w, "package {} {} front={}", p.id, p.name, b(p.is_front)).unwrap();
        for pin in p.pins() {
            writeln!(
                w,
                "  pin {} {} {} {}",
                pin.name,
                pin.padstack_id,
                vector(&pin.relative_location),
                d(pin.rotation_in_degree)
            )
            .unwrap();
        }
        if let Some(outline) = &p.outline {
            for (k, o) in outline.iter().enumerate() {
                let s = o.as_ref().map(shape).unwrap_or_else(|| "null".into());
                let wd = p.outline_widths.as_ref().map(|v| v[k]).unwrap_or(0.0);
                let cl = p.outline_is_closed.as_ref().map(|v| v[k]).unwrap_or(false);
                writeln!(w, "  outline {k} {s} w={} closed={}", d(wd), b(cl)).unwrap();
            }
        }
        use crate::loader::KeepoutKind as K;
        for (k, (list, kind)) in [
            (&p.keepouts, K::Keepout),
            (&p.via_keepouts, K::ViaKeepout),
            (&p.place_keepout_arr, K::PlaceKeepout),
        ]
        .into_iter()
        .enumerate()
        {
            for (i, ko) in list.iter().enumerate() {
                let a = if design.null_package_keepouts.contains(&(p.id, kind, i)) {
                    "null".to_string()
                } else {
                    area(&ko.area)
                };
                writeln!(w, "  keepout {k} {} {} {a}", ko.name, ko.layer).unwrap();
            }
        }
    }
    for c in design.components.get_all() {
        let loc = c.get_location().map(point).unwrap_or_else(|| "null".into());
        writeln!(
            w,
            "component {} {} loc={loc} rot={} front={} pkg={} fixed={} pn={}",
            c.id,
            c.name,
            d(c.get_rotation_in_degree()),
            b(c.placed_on_front()),
            c.get_package(),
            b(c.position_fixed),
            c.get_part_number().unwrap_or("null")
        )
        .unwrap();
    }
    writeln!(w, "bbox {}", tile(&TileShape::IntBox(design.bounding_box))).unwrap();
    writeln!(w, "flip {}", b(design.flip_style_rotate_first)).unwrap();
    w
}

/// The predicted item lines: `(id, line)` in id order.
pub fn dump_items(
    design: &LoadedDesign,
    adjustment: Option<&PlaneAdjustment>,
) -> Vec<(i32, String)> {
    let mut requests = design.requests.clone();
    if let Some(adj) = adjustment {
        adj.apply_to_requests(&mut requests);
    }
    let flip = design.flip_style_rotate_first;
    let lib = &design.library;
    let mut out = Vec::new();
    let mut next_id = 0;
    for r in &requests {
        if !r.consumes_id() {
            continue;
        }
        next_id += 1;
        if !r.inserts_item() {
            continue;
        }
        let head = |nets_: &[i32], cl: i32, f: FixedState, comp: i32| {
            format!(
                "item {next_id} {} nets={} cl={cl} fixed={} comp={comp}",
                r.kind_name(),
                nets(nets_),
                fixed(f)
            )
        };
        let line = match r {
            InsertRequest::Outline {
                shapes,
                clearance_class,
            } => {
                let s: Vec<String> = shapes.iter().map(polyline_shape).collect();
                format!(
                    "{} shapes=[{}]",
                    head(&[], *clearance_class, FixedState::SystemFixed, 0),
                    s.join("|")
                )
            }
            InsertRequest::Obstacle(a)
            | InsertRequest::ViaObstacle(a)
            | InsertRequest::ComponentObstacle(a) => {
                let abs = transformed_area(
                    &a.area,
                    &a.translation,
                    a.rotation_in_degree,
                    a.side_changed,
                    flip,
                );
                format!(
                    "{} layer={} name={} rel={} tr={} rot={} side={} abs={}",
                    head(&[], a.clearance_class, a.fixed, a.component_id),
                    a.layer,
                    a.name.as_deref().unwrap_or("null"),
                    area(&a.area),
                    vector(&a.translation),
                    d(a.rotation_in_degree),
                    b(a.side_changed),
                    area(&abs)
                )
            }
            InsertRequest::ConductionArea {
                area: ar,
                layer,
                nets: n,
                clearance_class,
                is_obstacle,
                fixed: f,
            } => {
                let zero = Vector::from(fr_geom::IntVector::ZERO);
                format!(
                    "{} layer={layer} obstacle={} rel={} abs={}",
                    head(n, *clearance_class, *f, 0),
                    b(*is_obstacle),
                    area(ar),
                    area(&ar.translate_by(&zero))
                )
            }
            InsertRequest::ComponentOutline {
                area: ar,
                is_front,
                translation,
                rotation_in_degree,
                component_id,
                is_courtyard,
                is_fabrication,
                is_closed,
                fixed: f,
            } => {
                let abs = transformed_area(ar, translation, *rotation_in_degree, !is_front, flip);
                format!(
                    "{} front={} court={} fab={} closed={} rel={} tr={} rot={} abs={}",
                    head(&[], 0, *f, *component_id),
                    b(*is_front),
                    b(*is_courtyard),
                    b(*is_fabrication),
                    b(*is_closed),
                    area(ar),
                    vector(translation),
                    d(*rotation_in_degree),
                    area(&abs)
                )
            }
            InsertRequest::Pin {
                component_id,
                pin_index,
                nets: n,
                clearance_class,
                fixed: f,
            } => {
                let comp = design.components.get(*component_id);
                let package = lib.packages.get(comp.get_package());
                let pin = package.get_pin(*pin_index).expect("pin");
                let padstack = lib.padstacks.get(pin.padstack_id).expect("padstack");
                let (first, last) = pin_layer_range(comp, padstack);
                let center = pin_center(comp, package, padstack, *pin_index, flip);
                let shapes: Vec<String> = pin_shapes(comp, package, padstack, *pin_index, flip)
                    .iter()
                    .map(|s| s.as_ref().map(shape).unwrap_or_else(|| "null".into()))
                    .collect();
                format!(
                    "{} pin={pin_index} first={first} last={last} center={} shapes=[{}]",
                    head(n, *clearance_class, *f, *component_id),
                    point(&center),
                    shapes.join("|")
                )
            }
            InsertRequest::Via {
                padstack,
                center,
                nets: n,
                clearance_class,
                fixed: f,
                attach_allowed,
                ..
            } => {
                let p = lib.padstacks.get(*padstack).expect("padstack");
                format!(
                    "{} padstack={} center={},{} attach={}",
                    head(n, *clearance_class, *f, 0),
                    p.name,
                    center.x,
                    center.y,
                    b(*attach_allowed)
                )
            }
            InsertRequest::Trace {
                polyline,
                layer,
                half_width,
                nets: n,
                clearance_class,
                fixed: f,
                ..
            } => {
                let c: Vec<String> = (0..polyline.corner_count())
                    .map(|i| point(&polyline.corner(i)))
                    .collect();
                format!(
                    "{} layer={layer} hw={half_width} corners=[{}]",
                    head(n, *clearance_class, *f, 0),
                    c.join(" ")
                )
            }
        };
        out.push((next_id, line));
    }
    out
}

/// Full dump (header + items), as written by the Java harness after `result OK`.
pub fn dump(design: &LoadedDesign) -> String {
    let mut s = String::from("result OK\n");
    let adj = design.plane_adjustment.as_ref();
    s.push_str(&dump_header(design, adj));
    for (_, l) in dump_items(design, adj) {
        s.push_str(&l);
        s.push('\n');
    }
    s
}

/// Item lines of a built board (see [`crate::board::build_board`]) in the Java dump format, in
/// id order. `ComponentOutline` has no public accessors for its relative placement in
/// fr-engine, so its `rel`/`tr`/`rot` fields are written as `?` (the board parity test ignores
/// them there).
pub fn dump_board_items(board: &fr_engine::board::BasicBoard) -> Vec<(i32, String)> {
    use fr_engine::board::ItemKind;
    let mut keys = board.get_items();
    keys.sort_by_key(|k| board.item(*k).id().0);
    let mut out = Vec::new();
    for k in keys {
        let it = board.item(k);
        let id = it.id().0;
        let mut line = format!(
            "item {id} {} nets={} cl={} fixed={} comp={}",
            it.class_name(),
            nets(it.net_numbers()),
            it.clearance_class(),
            fixed(it.fixed_state()),
            it.component_no()
        );
        match &it.kind {
            ItemKind::BoardOutline(o) => {
                let s: Vec<String> = o.shapes().iter().map(polyline_shape).collect();
                write!(line, " shapes=[{}]", s.join("|")).unwrap();
            }
            ItemKind::ConductionArea(c) => {
                let a = c.area();
                write!(
                    line,
                    " layer={} obstacle={} rel={} abs={}",
                    a.layer(),
                    b(c.is_obstacle()),
                    area(a.relative_area()),
                    area(&a.get_area(board))
                )
                .unwrap();
            }
            ItemKind::ObstacleArea(a) => {
                write!(
                    line,
                    " layer={} name={} rel={} tr={} rot={} side={} abs={}",
                    a.layer(),
                    a.name.as_deref().unwrap_or("null"),
                    area(a.relative_area()),
                    vector(a.translation()),
                    d(a.rotation_in_degree()),
                    b(a.side_changed()),
                    area(&a.get_area(board))
                )
                .unwrap();
            }
            ItemKind::ComponentOutline(o) => {
                write!(
                    line,
                    " front={} court={} fab={} closed={} rel=? tr=? rot=? abs={}",
                    b(o.is_front()),
                    b(o.is_courtyard),
                    b(o.is_fabrication),
                    b(o.is_closed),
                    area(&o.get_area(board))
                )
                .unwrap();
            }
            ItemKind::Pin(p) => {
                let shapes: Vec<String> = it
                    .drill_shapes(board)
                    .map(|s| {
                        s.iter()
                            .map(|x| x.as_ref().map(shape).unwrap_or_else(|| "null".into()))
                            .collect()
                    })
                    .unwrap_or_default();
                write!(
                    line,
                    " pin={} first={} last={} center={} shapes=[{}]",
                    p.pin_index,
                    it.first_layer(board),
                    it.last_layer(board),
                    point(&it.center(board)),
                    shapes.join("|")
                )
                .unwrap();
            }
            ItemKind::Via(v) => {
                let p = board
                    .library
                    .padstacks
                    .get(v.padstack_no())
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                write!(
                    line,
                    " padstack={p} center={} attach={}",
                    point(&it.center(board)),
                    b(v.attach_allowed)
                )
                .unwrap();
            }
            ItemKind::Trace(t) => {
                let c: Vec<String> = (0..t.polyline().corner_count())
                    .map(|i| point(&t.polyline().corner(i)))
                    .collect();
                write!(
                    line,
                    " layer={} hw={} corners=[{}]",
                    t.layer(),
                    t.half_width(),
                    c.join(" ")
                )
                .unwrap();
            }
        }
        out.push((id, line));
    }
    out
}
