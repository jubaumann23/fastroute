//! Differential test of the board model against the Java implementation.
//!
//! The vector files in `testdata/board/*.txt` were produced by `testdata/board/BoardGen.java`
//! (see there for the regeneration commands) from the real Freerouting 2.4.1 jar. Each file
//! contains the setup of a loaded DSN board (layers, rules, library, components), a script of
//! board operations and, after the operations with results, the expected result lines
//! (`= ...`). This test builds the same board, replays the script and compares every result
//! line exactly (intermediate board dumps are compared as FNV-1a hashes of the dump lines).
//!
//! Further files can be checked with `FR_BOARD_VECTORS=<file>[:<file>...] cargo test -p
//! fr-engine --test board_replay`.

use std::fmt::Write as _;
use std::path::PathBuf;

use fr_engine::autoroute::rooms::{IncompleteFreeSpaceExpansionRoom, RoomKey};
use fr_engine::board::*;
use fr_engine::datastructures::ItemIdGenerator;
use fr_engine::ids::{AngleRestriction, FixedState, ItemId, LayerNo, NetNo};
use fr_engine::library::{BoardLibrary, PackagePin, Packages, Padstacks};
use fr_engine::rules::{BoardRules, ClearanceMatrix};
use fr_engine::structure::{Communication, Components, CoordinateTransform, Layer, LayerStructure, SpecctraParserInfo, Unit};
use fr_geom::*;

// ------------------------------------------------------------------------------------------------
// tokens

struct Tok<'a> {
    t: Vec<&'a str>,
    pos: usize,
}

impl<'a> Tok<'a> {
    fn new(s: &'a str) -> Self {
        Tok { t: s.split(' ').filter(|x| !x.is_empty()).collect(), pos: 0 }
    }
    fn next(&mut self) -> &'a str {
        let r = self.t[self.pos];
        self.pos += 1;
        r
    }
    fn peek(&self) -> &'a str {
        self.t[self.pos]
    }
    fn i32(&mut self) -> i32 {
        self.next().parse().unwrap()
    }
    fn bool(&mut self) -> bool {
        self.next() == "1"
    }
    fn f64(&mut self) -> f64 {
        f64::from_bits(u64::from_str_radix(self.next(), 16).unwrap())
    }
    fn str_opt(&mut self) -> Option<String> {
        let s = self.next();
        match s {
            "-" => None,
            "~" => Some(String::new()),
            _ => {
                let bytes: Vec<u8> = (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect();
                Some(String::from_utf8(bytes).unwrap())
            }
        }
    }
    fn str(&mut self) -> String {
        self.str_opt().unwrap_or_default()
    }
    fn nets(&mut self) -> Vec<NetNo> {
        let s = self.next();
        let mut parts = s.split(',');
        let n: usize = parts.next().unwrap().parse().unwrap();
        let v: Vec<NetNo> = parts.map(|p| p.parse().unwrap()).collect();
        assert_eq!(v.len(), n);
        v
    }
    fn point(&mut self) -> Point {
        match self.next() {
            "I" => {
                let x = self.i32();
                let y = self.i32();
                Point::Int(IntPoint::new(x, y))
            }
            "R" => {
                let x: num_bigint::BigInt = self.next().parse().unwrap();
                let y: num_bigint::BigInt = self.next().parse().unwrap();
                let z: num_bigint::BigInt = self.next().parse().unwrap();
                Point::get_instance_big(x, y, z)
            }
            t => panic!("bad point token {t}"),
        }
    }
    fn line(&mut self) -> Line {
        assert_eq!(self.next(), "L");
        let a = self.point();
        let b = self.point();
        match (&a, &b) {
            (Point::Int(a), Point::Int(b)) => Line::from_int_points(*a, *b),
            _ => Line::new(a, b),
        }
    }
    fn lines(&mut self) -> Vec<Line> {
        let n = self.i32();
        (0..n).map(|_| self.line()).collect()
    }
    fn int_box(&mut self) -> IntBox {
        assert_eq!(self.next(), "B");
        let v: Vec<i32> = (0..4).map(|_| self.i32()).collect();
        IntBox::new(v[0], v[1], v[2], v[3])
    }
    fn tile(&mut self) -> TileShape {
        match self.peek() {
            "B" => TileShape::IntBox(self.int_box()),
            "O" => {
                self.next();
                let v: Vec<i32> = (0..8).map(|_| self.i32()).collect();
                TileShape::IntOctagon(IntOctagon::new(v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7]))
            }
            "S" => {
                self.next();
                TileShape::Simplex(Simplex::new(self.lines()))
            }
            t => panic!("bad tile token {t}"),
        }
    }
    fn opt_tile(&mut self) -> Option<TileShape> {
        if self.peek() == "N" {
            self.next();
            None
        } else {
            Some(self.tile())
        }
    }
    fn shape(&mut self) -> Shape {
        match self.peek() {
            "C" => {
                self.next();
                let x = self.i32();
                let y = self.i32();
                let r = self.i32();
                Shape::Circle(Circle::new(IntPoint::new(x, y), r))
            }
            "P" => {
                self.next();
                let n = self.i32();
                let pts: Vec<Point> = (0..n).map(|_| self.point()).collect();
                Shape::Polygon(PolygonShape::new(&Polygon::new(&pts)))
            }
            _ => Shape::Tile(self.tile()),
        }
    }
    fn polyline_shape(&mut self) -> PolylineShape {
        match self.shape() {
            Shape::Tile(t) => PolylineShape::Tile(t),
            Shape::Polygon(p) => PolylineShape::Polygon(p),
            Shape::Circle(_) => panic!("circle is not a polyline shape"),
        }
    }
    fn convex(&mut self) -> ConvexShape {
        match self.shape() {
            Shape::Tile(t) => ConvexShape::Tile(t),
            Shape::Circle(c) => ConvexShape::Circle(c),
            Shape::Polygon(_) => panic!("polygon is not convex"),
        }
    }
    fn area(&mut self) -> Area {
        if self.peek() == "A" {
            self.next();
            let border = self.polyline_shape();
            let n = self.i32();
            let holes: Vec<PolylineShape> = (0..n).map(|_| self.polyline_shape()).collect();
            Area::PolylineArea(PolylineArea::new(border, holes))
        } else {
            Area::Shape(self.shape())
        }
    }
    fn fixed(&mut self) -> FixedState {
        match self.i32() {
            0 => FixedState::Unfixed,
            1 => FixedState::ShoveFixed,
            2 => FixedState::UserFixed,
            3 => FixedState::SystemFixed,
            v => panic!("bad fixed state {v}"),
        }
    }
    fn vector(&mut self) -> Vector {
        let x = self.i32();
        let y = self.i32();
        Vector::get_instance(x, y)
    }
}

// ------------------------------------------------------------------------------------------------
// encoding (must match BoardGen.java)

fn b(v: bool) -> &'static str {
    if v {
        "1"
    } else {
        "0"
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

fn dir(dd: &Direction) -> String {
    match dd {
        Direction::Int(i) => format!("D {} {}", i.x, i.y),
        other => format!("BD {other:?}"),
    }
}

fn ln(l: &Line) -> String {
    format!("L {} {}", pt(&l.a), pt(&l.b))
}

fn bx(b: &IntBox) -> String {
    format!("B {} {} {} {}", b.ll.x, b.ll.y, b.ur.x, b.ur.y)
}

fn tile(t: &TileShape) -> String {
    match t {
        TileShape::IntBox(b) => bx(b),
        TileShape::IntOctagon(o) => format!(
            "O {} {} {} {} {} {} {} {}",
            o.left_x, o.bottom_y, o.right_x, o.top_y, o.upper_left_diagonal_x, o.lower_right_diagonal_x, o.lower_left_diagonal_x, o.upper_right_diagonal_x
        ),
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

fn regular(r: &RegularTileShape) -> String {
    match r {
        RegularTileShape::IntBox(b) => tile(&TileShape::IntBox(*b)),
        RegularTileShape::IntOctagon(o) => tile(&TileShape::IntOctagon(*o)),
    }
}

fn nets(n: &[NetNo]) -> String {
    let mut s = n.len().to_string();
    for x in n {
        write!(s, ",{x}").unwrap();
    }
    s
}

fn ids(board: &BasicBoard, set: &ItemSet) -> String {
    let v: Vec<String> = set.ids().map(|i| i.0.to_string()).collect();
    let _ = board;
    format!("[{}]", v.join(","))
}

fn obj(o: &TreeObject) -> String {
    match o {
        TreeObject::Item { id, .. } => id.to_string(),
        TreeObject::Room { id, .. } => format!("R{id}"),
    }
}

fn entries(e: &[TreeEntry]) -> String {
    let v: Vec<String> = e.iter().map(|e| format!("{}:{}", obj(&e.object), e.shape_index)).collect();
    format!("[{}]", v.join(","))
}

fn fnv(lines: &[String]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for l in lines {
        for x in l.bytes().chain(std::iter::once(b'\n')) {
            h ^= x as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    format!("{h:x}")
}

// ------------------------------------------------------------------------------------------------
// replay

type PadstackDef = (String, bool, bool, bool, Vec<Option<ConvexShape>>);
type ComponentDef = (String, Option<Point>, f64, bool, i32, i32, bool);

struct Setup {
    variant: JavaVariant,
    layers: Vec<Layer>,
    bbox: IntBox,
    unit: Unit,
    resolution: i32,
    host_cad: Option<String>,
    flip: bool,
    angle: AngleRestriction,
    hole_clearance: i32,
    edge_turn: f64,
    class_names: Vec<String>,
    cl_rows: Vec<(i32, i32, i32, Vec<i32>)>,
    net_classes: Vec<(String, bool, bool, bool, bool)>,
    nets: Vec<(String, i32, i32, bool)>,
    padstacks: Vec<PadstackDef>,
    packages: Vec<(String, bool, Vec<PackagePin>)>,
    components: Vec<ComponentDef>,
}

impl Setup {
    fn new() -> Self {
        Setup {
            variant: JavaVariant::Source,
            layers: Vec::new(),
            bbox: IntBox::EMPTY,
            unit: Unit::Mil,
            resolution: 1,
            host_cad: None,
            flip: false,
            angle: AngleRestriction::FortyfiveDegree,
            hole_clearance: 0,
            edge_turn: 0.0,
            class_names: Vec::new(),
            cl_rows: Vec::new(),
            net_classes: Vec::new(),
            nets: Vec::new(),
            padstacks: Vec::new(),
            packages: Vec::new(),
            components: Vec::new(),
        }
    }

    fn build_board(&self, outline: Vec<PolylineShape>, outline_cl: i32) -> BasicBoard {
        let ls = LayerStructure::new(self.layers.clone());
        let n = self.class_names.len() as i32;
        let mut m = ClearanceMatrix::new(n, &ls, &self.class_names);
        for (j, layer, max, values) in &self.cl_rows {
            m.set_value(0, *j, *layer, *max);
            for (i, v) in values.iter().enumerate() {
                m.set_value(i as i32, *j, *layer, *v);
            }
        }
        let mut rules = BoardRules::new(ls.clone(), m);
        rules.set_trace_angle_restriction(self.angle);
        rules.set_hole_clearance(self.hole_clearance);
        rules.set_pin_edge_to_turn_dist(self.edge_turn);
        let mut class_ids = Vec::new();
        for (name, shove_fixed, pull_tight, ignore_cycles, ignored) in &self.net_classes {
            let id = rules.net_classes.append(name.clone(), &ls, *ignored);
            let c = &mut rules.net_classes[id];
            c.set_shove_fixed(*shove_fixed);
            c.set_pull_tight(*pull_tight);
            c.set_ignore_cycles_with_areas(*ignore_cycles);
            class_ids.push(id);
        }
        for (name, subnet, class_index, contains_plane) in &self.nets {
            rules.nets.add(name.clone(), *subnet, *contains_plane, class_ids[*class_index as usize]);
        }
        let mut padstacks = Padstacks::new(&ls);
        for (name, attach, absolute, hole_only, shapes) in &self.padstacks {
            let id = padstacks.add(name.clone(), shapes.clone(), *attach, *absolute);
            padstacks.get_mut(id).unwrap().hole_only = *hole_only;
        }
        let mut packages = Packages::new();
        for (name, is_front, pins) in &self.packages {
            packages.add(name.clone(), pins.clone(), None, None, None, Vec::new(), Vec::new(), Vec::new(), *is_front);
        }
        let mut components = Components::new();
        for (name, loc, rot, on_front, front, back, fixed) in &self.components {
            components.add(name.clone(), loc.clone(), *rot, *on_front, *front, *back, *fixed, None);
        }
        components.set_flip_style_rotate_first(self.flip);
        let info = SpecctraParserInfo { host_cad: self.host_cad.clone(), ..SpecctraParserInfo::default() };
        let comm = Communication::new(self.unit, self.resolution, Some(info), CoordinateTransform::new(1.0, 0.0, 0.0), ItemIdGenerator::new());
        BasicBoard::new(self.bbox, ls, outline, outline_cl, rules, BoardLibrary::new(padstacks, packages), components, comm)
    }
}

fn item_line(board: &BasicBoard, key: ItemKey, full: bool) -> String {
    let it = board.item(key);
    let mut s = format!(
        "I {} {} {} {} {} {} {} {} {} {} n {}",
        it.id().0,
        it.class_name(),
        nets(it.net_numbers()),
        it.clearance_class(),
        it.fixed_state() as i32,
        it.component_no(),
        b(it.is_on_board()),
        it.first_layer(board),
        it.last_layer(board),
        bx(&it.bounding_box(board)),
        board.tile_shape_count(key)
    );
    match &it.kind {
        ItemKind::Trace(t) => {
            write!(s, " hw {} {}", t.half_width(), t.polyline().lines.len()).unwrap();
            for l in t.polyline().lines.iter() {
                s.push(' ');
                s.push_str(&ln(l));
            }
        }
        ItemKind::Pin(_) | ItemKind::Via(_) => {
            write!(s, " c {} ps {}", pt(&it.center(board)), it.padstack(board).unwrap().id).unwrap();
        }
        _ => {}
    }
    if full {
        for i in 0..board.tile_shape_count(key) {
            let shape = board.tile_shape(key, i);
            write!(s, " | {} {}", board.shape_layer(key, i), shape.as_ref().map(tile).unwrap_or_else(|| "N".into())).unwrap();
        }
    }
    s
}

fn tree_lines(board: &BasicBoard, t: usize, full: bool) -> (String, Vec<String>) {
    let tree = board.search_tree(t);
    let mat = tree.min_area_tree();
    let leaves = mat.to_array();
    let mut lines = Vec::with_capacity(leaves.len());
    for leaf in &leaves {
        let l = mat.leaf(*leaf).unwrap();
        let depth = if leaves.len() == 1 { 0 } else { mat.distance_to_root(*leaf) };
        let mut s = format!("L {} {} {} {}", obj(&l.object), l.shape_index_in_object, depth, regular(l.bounding_shape));
        if full {
            let shape = board.object_tree_shape(t, l.object, l.shape_index_in_object);
            s.push(' ');
            s.push_str(&shape.as_ref().map(tile).unwrap_or_else(|| "N".into()));
        }
        lines.push(s);
    }
    (format!("TREE {} {}", tree.key(), leaves.len()), lines)
}

fn block(head: String, lines: Vec<String>, expand: bool) -> Vec<String> {
    if expand {
        let mut r = vec![head];
        r.extend(lines);
        r
    } else {
        let h = fnv(&lines);
        vec![format!("{head} hash {h}")]
    }
}

fn find(board: &BasicBoard, id: i32) -> ItemKey {
    board.get_item(ItemId(id)).unwrap_or_else(|| panic!("item {id} not found"))
}

fn tree_index(board: &BasicBoard, cl: i32) -> usize {
    board.search_trees().tree_index(cl).unwrap_or_else(|| panic!("no tree for class {cl}"))
}

/// Executes one operation line and returns its result lines.
fn execute(board_opt: &mut Option<BasicBoard>, setup: &mut Setup, full: &mut bool, line: &str) -> Vec<String> {
    let mut t = Tok::new(line);
    let op = t.next();
    if board_opt.is_none() {
        match op {
            "MODE" => *full = t.next() == "full",
            "VARIANT" => {
                setup.variant = if t.next() == "jar241" { JavaVariant::Jar241 } else { JavaVariant::Source };
            }
            "LAYERS" => {
                let n = t.i32();
                for _ in 0..n {
                    let name = t.str();
                    let signal = t.bool();
                    setup.layers.push(Layer::new(name, signal));
                }
            }
            "BBOX" => setup.bbox = t.int_box(),
            "COMM" => {
                setup.unit = Unit::from_string(t.next()).unwrap();
                setup.resolution = t.i32();
                setup.host_cad = t.str_opt();
            }
            "FLIP" => setup.flip = t.bool(),
            "RULES" => {
                setup.angle = match t.i32() {
                    0 => AngleRestriction::None,
                    1 => AngleRestriction::FortyfiveDegree,
                    _ => AngleRestriction::NinetyDegree,
                };
                setup.hole_clearance = t.i32();
                setup.edge_turn = t.f64();
            }
            "CLASSES" => {
                let n = t.i32();
                setup.class_names = (0..n).map(|_| t.str()).collect();
            }
            "CLROW" => {
                let j = t.i32();
                let layer = t.i32();
                let max = t.i32();
                let values: Vec<i32> = (0..setup.class_names.len()).map(|_| t.i32()).collect();
                setup.cl_rows.push((j, layer, max, values));
            }
            "NETCLASS" => {
                let name = t.str();
                setup.net_classes.push((name, t.bool(), t.bool(), t.bool(), t.bool()));
            }
            "NET" => {
                let no = t.i32();
                assert_eq!(no as usize, setup.nets.len() + 1);
                let name = t.str();
                let subnet = t.i32();
                let class = t.i32();
                setup.nets.push((name, subnet, class, t.bool()));
            }
            "PADSTACK" => {
                let id = t.i32();
                assert_eq!(id as usize, setup.padstacks.len() + 1);
                let name = t.str();
                let attach = t.bool();
                let absolute = t.bool();
                let hole_only = t.bool();
                let n = t.i32();
                let shapes: Vec<Option<ConvexShape>> = (0..n)
                    .map(|_| {
                        if t.peek() == "N" {
                            t.next();
                            None
                        } else {
                            Some(t.convex())
                        }
                    })
                    .collect();
                setup.padstacks.push((name, attach, absolute, hole_only, shapes));
            }
            "PACKAGE" => {
                let id = t.i32();
                assert_eq!(id as usize, setup.packages.len() + 1);
                let name = t.str();
                let is_front = t.bool();
                let n = t.i32();
                let pins: Vec<PackagePin> = (0..n)
                    .map(|_| {
                        let name = t.str();
                        let ps = t.i32();
                        let v = t.vector();
                        let rot = t.f64();
                        PackagePin::new(name, ps, v, rot)
                    })
                    .collect();
                setup.packages.push((name, is_front, pins));
            }
            "COMP" => {
                let id = t.i32();
                assert_eq!(id as usize, setup.components.len() + 1);
                let name = t.str();
                let loc = if t.peek() == "N" {
                    t.next();
                    None
                } else {
                    Some(t.point())
                };
                let rot = t.f64();
                let on_front = t.bool();
                let front = t.i32();
                let back = t.i32();
                let fixed = t.bool();
                setup.components.push((name, loc, rot, on_front, front, back, fixed));
            }
            "BOARD" => {
                let n = t.i32();
                let cl = t.i32();
                let shapes: Vec<PolylineShape> = (0..n).map(|_| t.polyline_shape()).collect();
                let mut board = setup.build_board(shapes, cl);
                board.java_variant = setup.variant;
                *board_opt = Some(board);
            }
            _ => panic!("unexpected setup op {op}"),
        }
        return Vec::new();
    }
    let board = board_opt.as_mut().unwrap();
    let mut out: Vec<String> = Vec::new();
    match op {
        "COMPENSATION" => board.set_clearance_compensation_used(true),
        "KEEPOUT" => {
            let outline = board.get_outline().unwrap();
            board.generate_keepout_outside(outline, true);
        }
        "PIN" => {
            let comp = t.i32();
            let pin_index = t.i32();
            let n = t.nets();
            let cl = t.i32();
            let fixed = t.fixed();
            board.insert_pin(comp, pin_index, &n, cl, fixed);
        }
        "COND" => {
            let layer = t.i32();
            let tr = t.vector();
            let rot = t.f64();
            let side = t.bool();
            let n = t.nets();
            let cl = t.i32();
            let comp = t.i32();
            let name = t.str_opt();
            let is_obstacle = t.bool();
            let fixed = t.fixed();
            let area = t.area();
            board.insert_conduction_area_placed(area, layer, tr, rot, side, &n, cl, comp, name, is_obstacle, fixed);
        }
        "OBS" => {
            let kind = match t.next() {
                "V" => ObstacleKind::ViaKeepout,
                "C" => ObstacleKind::ComponentKeepout,
                _ => ObstacleKind::Keepout,
            };
            let layer = t.i32();
            let tr = t.vector();
            let rot = t.f64();
            let side = t.bool();
            let cl = t.i32();
            let comp = t.i32();
            let name = t.str_opt();
            let fixed = t.fixed();
            let area = t.area();
            board.insert_obstacle_area(kind, area, layer, tr, rot, side, cl, comp, name, fixed);
        }
        "COUTLINE" => {
            let is_front = t.bool();
            let tr = t.vector();
            let rot = t.f64();
            let comp = t.i32();
            let courtyard = t.bool();
            let fab = t.bool();
            let closed = t.bool();
            let fixed = t.fixed();
            let area = t.area();
            board.insert_component_outline(area, is_front, tr, rot, comp, courtyard, fab, closed, fixed);
        }
        "TRACENC" | "TRACE" => {
            let layer: LayerNo = t.i32();
            let hw = t.i32();
            let n = t.nets();
            let cl = t.i32();
            let fixed = t.fixed();
            let lines = t.lines();
            let polyline = Polyline::from_lines(lines);
            if op == "TRACENC" {
                let r = board.insert_trace_without_cleaning(polyline, layer, hw, &n, cl, fixed);
                out.push(match r {
                    None => "-".into(),
                    Some(k) => board.item(k).id().0.to_string(),
                });
            } else {
                board.insert_trace(polyline, layer, hw, &n, cl, fixed);
            }
        }
        "VIA" => {
            let ps = t.i32();
            let c = t.point();
            let n = t.nets();
            let cl = t.i32();
            let fixed = t.fixed();
            let attach = t.bool();
            board.insert_via(ps, c, &n, cl, fixed, attach);
        }
        "NORMALIZE_ALL" => out.push(b(board.normalize_all_traces()).into()),
        "NORMALIZE" => {
            let net = t.i32();
            out.push(b(board.normalize_traces(net)).into());
        }
        "COMBINE" => {
            let net = t.i32();
            out.push(b(board.combine_traces(net)).into());
        }
        "REMOVE" => {
            let k = find(board, t.i32());
            board.remove_item(k);
        }
        "SPLIT" => {
            let p = t.point();
            let layer = t.i32();
            let net = t.i32();
            out.push(b(board.split_traces(&p, layer, net)).into());
        }
        "SPLITAT" => {
            let k = find(board, t.i32());
            let p = t.point();
            let r = board.split_trace_at_point(k, &p);
            out.push(match r {
                None => "-".into(),
                Some(pieces) => {
                    let f = |x: Option<ItemKey>| x.map(|k| board.item(k).id().0.to_string()).unwrap_or_else(|| "-".into());
                    format!("{} {}", f(pieces[0]), f(pieces[1]))
                }
            });
        }
        "DUMP" => {
            let expand = t.next() == "expand";
            let keys = board.get_items();
            let head = format!(
                "ITEMS {} rev {} maxhw {} minhw {}",
                keys.len(),
                board.revision(),
                board.max_trace_half_width(),
                board.min_trace_half_width()
            );
            let lines: Vec<String> = keys.iter().map(|k| item_line(board, *k, *full)).collect();
            out.extend(block(head, lines, expand));
        }
        "TREEDUMP" => {
            let cl = t.i32();
            let expand = t.next() == "expand";
            let ti = tree_index(board, cl);
            let (head, lines) = tree_lines(board, ti, *full);
            out.extend(block(head, lines, expand));
        }
        "QCONN" => {
            let k = find(board, t.i32());
            let it = board.item(k);
            let net = if it.net_count() > 0 { it.net_number(0) } else { -1 };
            let mut s = String::new();
            write!(
                s,
                "{} {} {} {} {} {} {} {} {} {}",
                ids(board, &board.normal_contacts(k)),
                ids(board, &board.all_contacts(k, None)),
                ids(board, &board.connected_set(k, net, false)),
                ids(board, &board.connected_set(k, -1, true)),
                b(board.is_tail(k)),
                ids(board, &board.get_connection_items(k, StopConnectionOption::None)),
                ids(board, &board.get_connection_items(k, StopConnectionOption::Via)),
                ids(board, &board.get_connection_items(k, StopConnectionOption::FanoutVia)),
                ids(board, &board.unconnected_set(k, net)),
                b(board.is_connected(k))
            )
            .unwrap();
            if it.is_trace() {
                write!(
                    s,
                    " {} {} {} {} {}",
                    ids(board, &board.trace_start_contacts(k)),
                    ids(board, &board.trace_end_contacts(k)),
                    b(board.is_overlap(k)),
                    b(board.is_cycle(k)),
                    ids(board, &board.touching_pins_at_end_corners(k))
                )
                .unwrap();
            }
            if it.is_via() {
                write!(s, " {}", b(board.is_fanout_via(k, None))).unwrap();
            }
            out.push(s);
        }
        "QOV" => {
            let layer = t.i32();
            let n = t.nets();
            let q = ConvexShape::Tile(t.tile());
            let mut e = Vec::new();
            board.overlapping_tree_entries(DEFAULT_TREE, &q, layer, &n, &mut e);
            out.push(entries(&e));
        }
        "QCL" => {
            let layer = t.i32();
            let cl = t.i32();
            let n = t.nets();
            let q = ConvexShape::Tile(t.tile());
            let mut e = Vec::new();
            board.overlapping_tree_entries_with_clearance_raw(DEFAULT_TREE, &q, layer, &n, cl, &mut e);
            out.push(entries(&e));
        }
        "QCT" => {
            let layer = t.i32();
            let cl = t.i32();
            let n = t.nets();
            let q = t.tile();
            out.push(b(board.check_trace_shape(&q, layer, &n, cl, None)).into());
        }
        "QCS" => {
            let layer = t.i32();
            let cl = t.i32();
            let n = t.nets();
            let q = t.tile();
            out.push(b(board.check_shape(&Area::Shape(Shape::Tile(q)), layer, &n, cl)).into());
        }
        "QIWC" => {
            let layer = t.i32();
            let cl = t.i32();
            let n = t.nets();
            let q = ConvexShape::Tile(t.tile());
            out.push(ids(board, &board.overlapping_items_with_clearance(&q, layer, &n, cl)));
        }
        "QPK" => {
            let p = t.point();
            let layer = t.i32();
            out.push(ids(board, &board.pick_items(&p, layer, None)));
        }
        "QTAIL" => {
            let p = t.point();
            let layer = t.i32();
            let n = t.nets();
            out.push(match board.get_trace_tail(&p, layer, &n) {
                None => "-".into(),
                Some(k) => board.item(k).id().0.to_string(),
            });
        }
        "QCP" => {
            let layer = t.i32();
            let hw = t.i32();
            let cl = t.i32();
            let n = t.nets();
            let lines = t.lines();
            let polyline = Polyline::from_lines(lines);
            out.push(b(board.check_polyline_trace(&polyline, layer, hw, &n, cl)).into());
        }
        "QSETS" => {
            let net = t.i32();
            let sets = board.get_connected_sets(net);
            let s: String = sets.iter().map(|set| ids(board, set)).collect();
            out.push(if s.is_empty() { "-".into() } else { s });
        }
        "QEDGE" => {
            let mut v: Vec<NetNo> = board.edge_pin_nets().iter().copied().collect();
            v.sort();
            let v: Vec<String> = v.iter().map(|n| n.to_string()).collect();
            out.push(format!("[{}]", v.join(", ")));
        }
        "QEXIT" => {
            let k = find(board, t.i32());
            let it = board.item(k);
            let mut s = String::new();
            for l in it.first_layer(board)..=it.last_layer(board) {
                s.push('[');
                for r in board.pin_trace_exit_restrictions(k, l) {
                    write!(s, "{} {};", dir(&r.direction), d(r.min_length)).unwrap();
                }
                write!(s, "] {} {} {} ", d(board.pin_min_width(k, l)), d(board.pin_max_width(k, l)), board.pin_trace_neckdown_half_width(k, l)).unwrap();
            }
            write!(s, "{} {}", b(board.pin_has_trace_exit_restrictions(k)), ids(board, &board.pin_swappable_pins(k))).unwrap();
            out.push(s);
        }
        "QCONPIN" => {
            let k = find(board, t.i32());
            out.push(format!("{} {}", b(board.check_connection_to_pin(k, true)), b(board.check_connection_to_pin(k, false))));
        }
        "CORRECTPIN" => {
            let k = find(board, t.i32());
            let at_start = t.bool();
            let angle = match t.i32() {
                0 => AngleRestriction::None,
                1 => AngleRestriction::FortyfiveDegree,
                _ => AngleRestriction::NinetyDegree,
            };
            out.push(b(board.correct_connection_to_pin(k, at_start, angle)).into());
        }
        "SWAPPIN" => {
            let k = find(board, t.i32());
            let at_start = t.bool();
            out.push(b(board.swap_connection_to_pin(k, at_start)).into());
        }
        "QMISC" => {
            out.push(format!(
                "{} {} {} {} {} {} {}",
                board.get_conduction_areas().len(),
                board.get_pins().len(),
                board.get_smd_pins().len(),
                board.get_vias().len(),
                board.get_traces().len(),
                d(board.cumulative_trace_length()),
                board.non_45_degree_trace_count()
            ));
        }
        "AUTOTREE" => {
            let cl = t.i32();
            board.get_autoroute_tree(cl);
        }
        "QCOMPLETE" => {
            let cl = t.i32();
            let layer = t.i32();
            let net = t.i32();
            let ignore = t.next();
            let ignore_object = if ignore == "-" {
                None
            } else {
                let id: i32 = ignore[1..].parse().unwrap();
                Some(TreeObject::Item { key: find(board, id), id })
            };
            let ignore_shape = t.opt_tile();
            let room_shape = t.opt_tile();
            let contained = t.tile();
            let ti = tree_index(board, cl);
            let room = IncompleteFreeSpaceExpansionRoom::new(room_shape, layer, Some(contained));
            let result = board.complete_shape(ti, &room, net, ignore_object, ignore_shape.as_ref());
            let mut s = result.len().to_string();
            for r in &result {
                write!(s, " | {} ; {}", tile(r.shape.as_ref().unwrap()), tile(r.contained_shape.as_ref().unwrap())).unwrap();
            }
            out.push(s);
        }
        "ROOMADD" => {
            let cl = t.i32();
            let key = t.i32();
            let id = t.i32();
            let layer = t.i32();
            let shape = t.tile();
            let ti = tree_index(board, cl);
            board.search_trees_mut().tree_mut(ti).insert_room(RoomKey(key as u32), id, shape, layer);
        }
        "ROOMDEL" => {
            let cl = t.i32();
            let key = t.i32();
            let ti = tree_index(board, cl);
            board.search_trees_mut().tree_mut(ti).remove_room(RoomKey(key as u32));
        }
        _ => panic!("unknown op {op}"),
    }
    out
}

fn run_file(path: &PathBuf) {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let lines: Vec<&str> = text.lines().collect();
    let mut setup = Setup::new();
    let mut board: Option<BasicBoard> = None;
    let mut full = false;
    let mut i = 0;
    let mut op_count = 0;
    let mut mismatches = 0;
    while i < lines.len() {
        let line = lines[i];
        i += 1;
        if line.starts_with("= ") {
            panic!("{}:{}: unexpected result line", path.display(), i);
        }
        let mut expected: Vec<&str> = Vec::new();
        while i < lines.len() && lines[i].starts_with("= ") {
            expected.push(&lines[i][2..]);
            i += 1;
        }
        let actual = execute(&mut board, &mut setup, &mut full, line);
        op_count += 1;
        let actual_refs: Vec<&str> = actual.iter().map(|s| s.as_str()).collect();
        if actual_refs != expected {
            mismatches += 1;
            if mismatches <= 5 {
                eprintln!("{}:{}: mismatch after op `{}`", path.display(), i, &line[..line.len().min(200)]);
                let n = expected.len().max(actual.len());
                let mut shown = 0;
                for k in 0..n {
                    let e = expected.get(k).copied().unwrap_or("<none>");
                    let a = actual_refs.get(k).copied().unwrap_or("<none>");
                    if e != a {
                        eprintln!("  line {k}:\n    java: {}\n    rust: {}", &e[..e.len().min(600)], &a[..a.len().min(600)]);
                        shown += 1;
                        if shown >= 3 {
                            break;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(mismatches, 0, "{}: {} of {} operations mismatched", path.display(), mismatches, op_count);
    eprintln!("{}: {} operations match", path.display(), op_count);
}

fn vector_files() -> Vec<PathBuf> {
    if let Ok(v) = std::env::var("FR_BOARD_VECTORS") {
        return v.split(':').map(PathBuf::from).collect();
    }
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/board");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().map(|e| e == "txt").unwrap_or(false))
        .collect();
    files.sort();
    files
}

#[test]
fn board_operations_match_java() {
    let files = vector_files();
    assert!(!files.is_empty(), "no vector files");
    let mut failed = Vec::new();
    for f in &files {
        let r = std::panic::catch_unwind(|| run_file(f));
        if r.is_err() {
            failed.push(f.display().to_string());
        }
    }
    assert!(failed.is_empty(), "failed: {failed:?}");
}

