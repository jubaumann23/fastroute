//! Typed Specctra DSN model.
//!
//! This is a pure data view of a DSN file: names are kept as strings and
//! coordinates in DSN units. Resolution of layer names, padstack lookups and
//! the coordinate transform happen when the board is built, mirroring the
//! Java reader (`io.specctra.parser.*`) which interleaves both steps.
//!
//! The readers follow the Java token-level semantics where they matter for
//! routing results (e.g. `(type route)` being USER_FIXED, clearance type
//! lists split on `-`, integer-only subnet numbers). Scopes the Java reader
//! skips are skipped here too.

use crate::sexpr::{self, Atom, List, Sexpr};

#[derive(Debug, Clone, PartialEq)]
pub enum LayerRef {
    /// `pcb`: all layers.
    Pcb,
    /// `signal`: all signal layers.
    Signal,
    Named(String),
}

impl LayerRef {
    fn from_atom(a: &Atom) -> Self {
        // Java compares the raw string against the keyword names "pcb" and "signal".
        match a.text.as_str() {
            "pcb" if !a.quoted => LayerRef::Pcb,
            "signal" if !a.quoted => LayerRef::Signal,
            _ => LayerRef::Named(a.text.clone()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ShapeKind {
    /// x1 y1 x2 y2 (any corner order).
    Rect([f64; 4]),
    /// Closed polygon corner coordinates x0 y0 x1 y1 ...; the aperture width is ignored.
    Polygon(Vec<f64>),
    /// diameter, optional center x, y (missing values are 0).
    Circle([f64; 3]),
    /// `path`: polyline of corners with a width.
    PolygonPath { width: f64, coords: Vec<f64> },
    /// `polyline_path`: sequence of lines given by two points each.
    PolylinePath { width: f64, coords: Vec<f64> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub layer: LayerRef,
    pub kind: ShapeKind,
    pub line: u32,
}

/// A shape with optional holes, e.g. a keepout or plane.
#[derive(Debug, Clone, PartialEq)]
pub struct Area {
    pub name: Option<String>,
    /// First shape is the border, the rest are holes (`window`).
    pub shapes: Vec<Shape>,
    pub clearance_class: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Inch,
    Mil,
    Mm,
    Um,
}

impl Unit {
    pub fn parse(s: &str) -> Option<Unit> {
        match s.to_ascii_lowercase().as_str() {
            "inch" => Some(Unit::Inch),
            "mil" => Some(Unit::Mil),
            "mm" => Some(Unit::Mm),
            "um" => Some(Unit::Um),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Rule {
    Width(f64),
    /// Clearance value and the (possibly empty) list of class-pair strings.
    Clearance { value: f64, class_pairs: Vec<String> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayerRule {
    pub layers: Vec<String>,
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub name: String,
    pub is_signal: bool,
    /// False if the layer had an unknown `(type ...)`; Java drops such layers.
    pub type_ok: bool,
    pub net_names: Vec<String>,
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AngleRestriction {
    None,
    NinetyDegree,
    #[default]
    FortyfiveDegree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepoutKind {
    Keepout,
    ViaKeepout,
    PlaceKeepout,
    /// fastroute: `wire_keepout` blocks traces only (Freerouting reads it as a `keepout`).
    WireKeepout,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Plane {
    pub net: String,
    pub area: Option<Area>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Structure {
    pub layers: Vec<Layer>,
    /// Boundary shapes in file order.
    pub boundaries: Vec<Shape>,
    pub outline_clearance_class: Option<String>,
    /// `None` if there is no `(via ...)` scope; spare vias are appended.
    pub via_padstacks: Option<Vec<String>>,
    pub rules: Vec<Rule>,
    pub keepouts: Vec<(KeepoutKind, Area)>,
    pub planes: Vec<Plane>,
    pub via_at_smd: bool,
    pub flip_style_rotate_first: bool,
    pub snap_angle: Option<AngleRestriction>,
    /// Freerouting-specific `(autoroute_settings ...)`, kept raw.
    pub autoroute_settings: Option<List>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Padstack {
    pub name: String,
    pub shapes: Vec<Shape>,
    /// `(attach on|off)`, default on.
    pub attach: bool,
    /// `(absolute on|off)`, default off.
    pub absolute: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PinDef {
    pub padstack: String,
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub rotation: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub name: String,
    pub is_front: bool,
    pub pins: Vec<PinDef>,
    pub outlines: Vec<Shape>,
    pub keepouts: Vec<Area>,
    pub via_keepouts: Vec<Area>,
    pub place_keepouts: Vec<Area>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Library {
    pub padstacks: Vec<Padstack>,
    pub images: Vec<Image>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ItemClearance {
    pub name: String,
    pub clearance_class: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub name: String,
    /// `None` for an unplaced component (`(place REF)`).
    pub location: Option<[f64; 2]>,
    pub is_front: bool,
    pub rotation: f64,
    pub position_fixed: bool,
    pub pin_clearances: Vec<ItemClearance>,
    pub keepout_clearances: Vec<ItemClearance>,
    pub via_keepout_clearances: Vec<ItemClearance>,
    pub place_keepout_clearances: Vec<ItemClearance>,
    pub part_number: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComponentPlacement {
    pub image: String,
    pub places: Vec<Place>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PinRef {
    pub component: String,
    pub pin: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Net {
    pub name: String,
    /// Explicit subnet number following the name, default 1.
    pub subnet: i32,
    pub pins: Vec<PinRef>,
    /// True if pins came from an `(order ...)` scope.
    pub ordered: bool,
    pub fromtos: Vec<Vec<PinRef>>,
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NetClass {
    pub name: String,
    pub nets: Vec<String>,
    pub rules: Vec<Rule>,
    pub layer_rules: Vec<LayerRule>,
    pub use_via: Vec<String>,
    pub use_layer: Vec<String>,
    pub via_rule: Option<String>,
    pub clearance_class: Option<String>,
    pub shove_fixed: bool,
    pub pull_tight: bool,
    pub min_trace_length: f64,
    pub max_trace_length: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassClass {
    pub classes: Vec<String>,
    pub rules: Vec<Rule>,
    pub layer_rules: Vec<LayerRule>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ViaInfo {
    pub name: String,
    pub padstack: String,
    pub clearance_class: String,
    pub attach: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Network {
    pub nets: Vec<Net>,
    pub classes: Vec<NetClass>,
    pub class_classes: Vec<ClassClass>,
    pub vias: Vec<ViaInfo>,
    pub via_rules: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum FixedState {
    #[default]
    Unfixed,
    ShoveFixed,
    UserFixed,
    SystemFixed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NetId {
    pub name: String,
    /// 0 means all subnets.
    pub subnet: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Wire {
    /// A path (trace) or area (conduction area) shape.
    pub shape: Shape,
    pub holes: Vec<Shape>,
    pub net: Option<NetId>,
    pub clearance_class: Option<String>,
    pub fixed: FixedState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WiringVia {
    pub padstack: String,
    pub x: f64,
    pub y: f64,
    pub net: Option<NetId>,
    pub clearance_class: Option<String>,
    pub fixed: FixedState,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Wiring {
    pub wires: Vec<Wire>,
    pub vias: Vec<WiringVia>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParserInfo {
    pub string_quote: String,
    pub host_cad: Option<String>,
    pub host_version: Option<String>,
    pub constants: Vec<(String, String)>,
    pub write_resolution: Option<(String, i32)>,
    /// False if the file was written by freerouting itself.
    pub generated_by_host: bool,
}

impl Default for ParserInfo {
    fn default() -> Self {
        ParserInfo {
            string_quote: "\"".into(),
            host_cad: None,
            host_version: None,
            constants: Vec::new(),
            write_resolution: None,
            generated_by_host: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Dsn {
    pub name: String,
    pub parser: ParserInfo,
    pub unit: Unit,
    pub resolution: i32,
    pub structure: Structure,
    pub library: Library,
    pub placement: Vec<ComponentPlacement>,
    pub network: Network,
    pub wiring: Wiring,
    /// `(part_library ...)` for pin/gate swap, kept raw.
    pub part_library: Option<List>,
    /// Non-fatal problems found while reading.
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub enum DsnError {
    Syntax(sexpr::ParseError),
    NotDsn(String),
    Invalid { line: u32, message: String },
}

impl std::fmt::Display for DsnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DsnError::Syntax(e) => write!(f, "syntax error: {e}"),
            DsnError::NotDsn(m) => write!(f, "not a Specctra DSN file: {m}"),
            DsnError::Invalid { line, message } => write!(f, "line {line}: {message}"),
        }
    }
}

impl std::error::Error for DsnError {}

fn invalid(line: u32, message: impl Into<String>) -> DsnError {
    DsnError::Invalid {
        line,
        message: message.into(),
    }
}

// ---------------------------------------------------------------------------
// Atom helpers

impl Atom {
    /// Integer literal as the Java scanner recognises it (`[+-]?(0|[1-9][0-9]*)`).
    pub fn as_int(&self) -> Option<i32> {
        if self.quoted {
            return None;
        }
        let digits = self.text.strip_prefix(['+', '-']).unwrap_or(&self.text);
        let ok = !digits.is_empty()
            && digits.bytes().all(|b| b.is_ascii_digit())
            && (digits == "0" || !digits.starts_with('0'));
        if ok {
            self.text.trim_start_matches('+').parse().ok()
        } else {
            None
        }
    }

    /// Any numeric literal (integer or float).
    pub fn as_num(&self) -> Option<f64> {
        if self.quoted {
            return None;
        }
        let b = self.text.as_bytes();
        let first_ok = b
            .first()
            .is_some_and(|c| c.is_ascii_digit() || *c == b'-' || *c == b'+');
        if !first_ok {
            return None;
        }
        self.text.parse::<f64>().ok().filter(|v| v.is_finite())
    }

    fn is_kw(&self, kw: &str) -> bool {
        !self.quoted && self.text.eq_ignore_ascii_case(kw)
    }
}

fn atom_is(s: Option<&Sexpr>, kw: &str) -> bool {
    s.and_then(Sexpr::as_atom).is_some_and(|a| a.is_kw(kw))
}

fn first_atom(l: &List) -> Option<&Atom> {
    l.args().first().and_then(Sexpr::as_atom)
}

fn string_arg(l: &List) -> Option<String> {
    first_atom(l).map(|a| a.text.clone())
}

fn on_off(l: &List) -> bool {
    first_atom(l).is_some_and(|a| a.is_kw("on"))
}

/// All numeric atoms after the head, skipping nested lists like Java does
/// for polygons and paths. Returns `None` if a non-numeric atom is found.
fn numbers(items: &[Sexpr]) -> Option<Vec<f64>> {
    items
        .iter()
        .filter_map(Sexpr::as_atom)
        .map(Atom::as_num)
        .collect()
}

// ---------------------------------------------------------------------------
// Shapes

fn read_shape(l: &List, warnings: &mut Vec<String>) -> Option<Shape> {
    let head = l.head()?.to_ascii_lowercase();
    let args = l.args();
    let layer_atom = args.first().and_then(Sexpr::as_atom)?;
    let layer = LayerRef::from_atom(layer_atom);
    let rest = &args[1..];
    let kind = match head.as_str() {
        "rect" | "rectangle" => {
            let v = numbers(rest)?;
            if v.len() != 4 {
                warnings.push(format!("line {}: rectangle needs 4 numbers", l.line));
                return None;
            }
            ShapeKind::Rect([v[0], v[1], v[2], v[3]])
        }
        "poly" | "polygon" => {
            // First value is the aperture width (ignored), nested scopes skipped.
            let v = numbers(rest)?;
            ShapeKind::Polygon(v.into_iter().skip(1).collect())
        }
        "circ" | "circle" => {
            let v = numbers(rest)?;
            if v.len() > 3 {
                warnings.push(format!("line {}: circle has too many numbers", l.line));
                return None;
            }
            let mut c = [0.0; 3];
            c[..v.len()].copy_from_slice(&v);
            ShapeKind::Circle(c)
        }
        "path" | "polyline_path" => {
            let v = numbers(rest)?;
            if v.len() < 5 {
                warnings.push(format!(
                    "line {}: skipping path with too few coordinates",
                    l.line
                ));
                return None;
            }
            let width = v[0];
            let coords = v[1..].to_vec();
            if head == "path" {
                ShapeKind::PolygonPath { width, coords }
            } else {
                ShapeKind::PolylinePath { width, coords }
            }
        }
        _ => return None,
    };
    Some(Shape {
        layer,
        kind,
        line: l.line,
    })
}

fn is_shape_kw(l: &List) -> bool {
    ["rect", "rectangle", "poly", "polygon", "circ", "circle", "path", "polyline_path"]
        .iter()
        .any(|k| l.is(k))
}

/// `(keepout [name] SHAPE (window SHAPE)* (clearance_class C) ...)`.
fn read_area(l: &List, skip_windows: bool, warnings: &mut Vec<String>) -> Option<Area> {
    let mut name = None;
    if let Some(a) = first_atom(l) {
        if !a.text.is_empty() {
            name = Some(a.text.clone());
        }
    }
    let mut shapes = Vec::new();
    let mut clearance_class = None;
    let mut border_read = false;
    for sub in l.sublists() {
        if !border_read {
            border_read = true;
            match read_shape(sub, warnings) {
                Some(s) => shapes.push(s),
                None => {
                    warnings.push(format!("line {}: could not read area shape", sub.line));
                    return None;
                }
            }
            continue;
        }
        if sub.is("window") {
            if !skip_windows {
                if let Some(s) = sub.sublists().next().and_then(|s| read_shape(s, warnings)) {
                    shapes.push(s);
                }
            }
        } else if sub.is("clearance_class") {
            clearance_class = string_arg(sub);
        }
    }
    if shapes.is_empty() {
        return None;
    }
    Some(Area {
        name,
        shapes,
        clearance_class,
    })
}

// ---------------------------------------------------------------------------
// Rules

/// Splits clearance type strings like Java's `nextStringList('-')`.
fn clearance_type_strings(l: &List) -> Vec<String> {
    let mut out = Vec::new();
    for a in l.args().iter().filter_map(Sexpr::as_atom) {
        let (first, second) = match a.quoted_len {
            Some(n) => (&a.text[..n], Some(&a.text[n..])),
            None => (a.text.as_str(), None),
        };
        if a.quoted && !first.is_empty() {
            // A quoted token is read as a whole by the Java scanner.
            out.push(first.to_string());
        } else {
            out.extend(first.split('-').filter(|s| !s.is_empty()).map(String::from));
        }
        if let Some(rest) = second {
            out.extend(rest.split('-').filter(|s| !s.is_empty()).map(String::from));
        }
    }
    out
}

fn read_rules(l: &List) -> Vec<Rule> {
    let mut out = Vec::new();
    for sub in l.sublists() {
        if sub.is("width") {
            if let Some(v) = first_atom(sub).and_then(Atom::as_num) {
                out.push(Rule::Width(v));
            }
        } else if sub.is("clear") || sub.is("clearance") {
            let Some(value) = first_atom(sub).and_then(Atom::as_num) else {
                continue;
            };
            let class_pairs = sub
                .find("type")
                .map(clearance_type_strings)
                .unwrap_or_default();
            out.push(Rule::Clearance { value, class_pairs });
        }
    }
    out
}

fn read_layer_rule(l: &List) -> LayerRule {
    LayerRule {
        layers: l.leading_atoms().map(|a| a.text.clone()).collect(),
        rules: l.find_all("rule").flat_map(read_rules).collect(),
    }
}

// ---------------------------------------------------------------------------
// Structure

fn read_structure(l: &List, skip_plane_windows: bool, warnings: &mut Vec<String>) -> Structure {
    let mut s = Structure::default();
    for sub in l.sublists() {
        let Some(head) = sub.head() else { continue };
        match head.to_ascii_lowercase().as_str() {
            "boundary" => {
                for item in sub.sublists() {
                    if item.is("clearance_class") {
                        s.outline_clearance_class = string_arg(item);
                    } else if let Some(shape) = read_shape(item, warnings) {
                        s.boundaries.push(shape);
                    }
                }
            }
            "layer" => {
                let Some(name) = string_arg(sub) else { continue };
                let mut layer = Layer {
                    name,
                    is_signal: true,
                    type_ok: true,
                    net_names: Vec::new(),
                    rules: Vec::new(),
                };
                for item in sub.sublists() {
                    if item.is("type") {
                        match first_atom(item).map(|a| a.text.to_ascii_lowercase()) {
                            Some(t) if t == "power" => layer.is_signal = false,
                            Some(t) if t == "signal" || t == "jumper" => {}
                            other => {
                                warnings.push(format!(
                                    "line {}: layer '{}' has unknown type {:?}",
                                    item.line, layer.name, other
                                ));
                                layer.type_ok = false;
                            }
                        }
                    } else if item.is("rule") {
                        layer.rules.extend(read_rules(item));
                    } else if item.is("use_net") {
                        layer
                            .net_names
                            .extend(item.leading_atoms().map(|a| a.text.clone()));
                    }
                }
                s.layers.push(layer);
            }
            "via" => s.via_padstacks = Some(read_via_padstacks(sub)),
            "rule" => s.rules.extend(read_rules(sub)),
            "keepout" | "wire_keepout" | "via_keepout" | "place_keepout" => {
                let kind = match head.to_ascii_lowercase().as_str() {
                    "via_keepout" => KeepoutKind::ViaKeepout,
                    "place_keepout" => KeepoutKind::PlaceKeepout,
                    "wire_keepout" => KeepoutKind::WireKeepout,
                    _ => KeepoutKind::Keepout,
                };
                if let Some(area) = read_area(sub, false, warnings) {
                    s.keepouts.push((kind, area));
                }
            }
            "plane" => {
                if let Some(net) = string_arg(sub) {
                    let area = read_area_after_first(sub, skip_plane_windows, warnings);
                    s.planes.push(Plane { net, area });
                }
            }
            "autoroute_settings" => s.autoroute_settings = Some(sub.clone()),
            "control" => {
                if let Some(v) = sub.find("via_at_smd") {
                    s.via_at_smd = on_off(v);
                }
            }
            "flip_style" => {
                s.flip_style_rotate_first = first_atom(sub).is_some_and(|a| a.is_kw("rotate_first"))
            }
            "snap_angle" => {
                s.snap_angle = match first_atom(sub).map(|a| a.text.to_ascii_lowercase()) {
                    Some(t) if t == "ninety_degree" => Some(AngleRestriction::NinetyDegree),
                    Some(t) if t == "fortyfive_degree" => Some(AngleRestriction::FortyfiveDegree),
                    Some(t) if t == "none" => Some(AngleRestriction::None),
                    _ => s.snap_angle,
                }
            }
            _ => {}
        }
    }
    s
}

/// `(plane NET SHAPE ...)`: the net name takes the place of the area name.
fn read_area_after_first(l: &List, skip_windows: bool, warnings: &mut Vec<String>) -> Option<Area> {
    let mut stripped = l.clone();
    // Remove the net-name atom so read_area sees the shape first.
    if stripped.items.len() > 1 && stripped.items[1].as_atom().is_some() {
        stripped.items.remove(1);
    }
    read_area(&stripped, skip_windows, warnings)
}

fn read_via_padstacks(l: &List) -> Vec<String> {
    let mut normal: Vec<String> = Vec::new();
    let mut spare = Vec::new();
    for item in l.args() {
        match item {
            Sexpr::Atom(a) => normal.push(a.text.clone()),
            Sexpr::List(sub) if sub.is("spare") => spare = read_via_padstacks(sub),
            Sexpr::List(_) => {}
        }
    }
    normal.extend(spare);
    normal
}

// ---------------------------------------------------------------------------
// Library

fn read_padstack(l: &List, warnings: &mut Vec<String>) -> Option<Padstack> {
    let name = string_arg(l)?;
    let mut p = Padstack {
        name,
        shapes: Vec::new(),
        attach: true,
        absolute: false,
    };
    for sub in l.sublists() {
        if sub.is("shape") {
            if let Some(s) = sub.sublists().next().and_then(|s| read_shape(s, warnings)) {
                p.shapes.push(s);
            }
        } else if sub.is("attach") {
            p.attach = on_off(sub);
        } else if sub.is("absolute") {
            p.absolute = on_off(sub);
        }
    }
    Some(p)
}

fn read_rotation(l: &List) -> f64 {
    first_atom(l)
        .and_then(|a| a.text.parse::<f64>().ok())
        .unwrap_or(0.0)
}

fn read_pin_def(l: &List) -> Result<PinDef, DsnError> {
    let err = || invalid(l.line, "malformed pin definition");
    let mut args = l.args().iter().peekable();
    let padstack = args.next().and_then(Sexpr::as_atom).ok_or_else(err)?.text.clone();
    let mut rotation = 0.0;
    if let Some(Sexpr::List(r)) = args.peek() {
        if r.is("rotate") {
            rotation = read_rotation(r);
        }
        args.next();
    }
    let name = args.next().and_then(Sexpr::as_atom).ok_or_else(err)?.text.clone();
    let x = args.next().and_then(Sexpr::as_atom).and_then(Atom::as_num).ok_or_else(err)?;
    let y = args.next().and_then(Sexpr::as_atom).and_then(Atom::as_num).ok_or_else(err)?;
    for rest in args {
        if let Sexpr::List(r) = rest {
            if r.is("rotate") {
                rotation = read_rotation(r);
            }
        }
    }
    Ok(PinDef {
        padstack,
        name,
        x,
        y,
        rotation,
    })
}

fn read_image(l: &List, warnings: &mut Vec<String>) -> Result<Image, DsnError> {
    let name = string_arg(l).ok_or_else(|| invalid(l.line, "image name expected"))?;
    let mut img = Image {
        name,
        is_front: true,
        pins: Vec::new(),
        outlines: Vec::new(),
        keepouts: Vec::new(),
        via_keepouts: Vec::new(),
        place_keepouts: Vec::new(),
    };
    for sub in l.sublists() {
        let Some(head) = sub.head() else { continue };
        match head.to_ascii_lowercase().as_str() {
            "pin" => img.pins.push(read_pin_def(sub)?),
            "side" => img.is_front = !first_atom(sub).is_some_and(|a| a.is_kw("back")),
            "outline" => {
                if let Some(s) = sub.sublists().next().and_then(|s| read_shape(s, warnings)) {
                    img.outlines.push(s);
                }
            }
            "keepout" | "wire_keepout" => {
                if let Some(a) = read_area(sub, false, warnings) {
                    img.keepouts.push(a);
                } else {
                    warnings.push(format!(
                        "line {}: could not read keepout area of image '{}'",
                        sub.line, img.name
                    ));
                }
            }
            "via_keepout" => img.via_keepouts.extend(read_area(sub, false, warnings)),
            "place_keepout" => img.place_keepouts.extend(read_area(sub, false, warnings)),
            _ => {}
        }
    }
    Ok(img)
}

fn read_library(l: &List, warnings: &mut Vec<String>) -> Result<Library, DsnError> {
    let mut lib = Library::default();
    for sub in l.sublists() {
        if sub.is("padstack") {
            match read_padstack(sub, warnings) {
                Some(p) => lib.padstacks.push(p),
                None => return Err(invalid(sub.line, "unexpected padstack identifier")),
            }
        } else if sub.is("image") {
            lib.images.push(read_image(sub, warnings)?);
        }
    }
    Ok(lib)
}

// ---------------------------------------------------------------------------
// Placement

fn read_item_clearance(l: &List) -> Option<ItemClearance> {
    let name = string_arg(l)?;
    let cc = l
        .sublists()
        .find(|s| s.is("clearance_class") || s.is("clearanceClass"))
        .and_then(string_arg)?;
    Some(ItemClearance {
        name,
        clearance_class: cc,
    })
}

fn read_place(l: &List) -> Result<Place, DsnError> {
    let args = l.args();
    let name = args
        .first()
        .and_then(Sexpr::as_atom)
        .ok_or_else(|| invalid(l.line, "component reference expected"))?
        .text
        .clone();
    let mut p = Place {
        name,
        location: None,
        is_front: true,
        rotation: 0.0,
        position_fixed: false,
        pin_clearances: Vec::new(),
        keepout_clearances: Vec::new(),
        via_keepout_clearances: Vec::new(),
        place_keepout_clearances: Vec::new(),
        part_number: None,
    };
    if args.len() == 1 {
        return Ok(p);
    }
    let num = |i: usize| args.get(i).and_then(Sexpr::as_atom).and_then(Atom::as_num);
    let (Some(x), Some(y)) = (num(1), num(2)) else {
        return Err(invalid(l.line, "place: coordinates expected"));
    };
    p.location = Some([x, y]);
    p.is_front = !atom_is(args.get(3), "back");
    p.rotation = num(4).ok_or_else(|| invalid(l.line, "place: rotation expected"))?;
    for sub in l.sublists() {
        let Some(head) = sub.head() else { continue };
        match head.to_ascii_lowercase().as_str() {
            "lock_type" => {
                p.position_fixed = sub.leading_atoms().any(|a| a.is_kw("position"))
            }
            "pin" => p.pin_clearances.extend(read_item_clearance(sub)),
            "keepout" => p.keepout_clearances.extend(read_item_clearance(sub)),
            "via_keepout" => p.via_keepout_clearances.extend(read_item_clearance(sub)),
            "place_keepout" => p.place_keepout_clearances.extend(read_item_clearance(sub)),
            "pn" => p.part_number = string_arg(sub),
            _ => {}
        }
    }
    Ok(p)
}

fn read_placement(l: &List) -> Result<Vec<ComponentPlacement>, DsnError> {
    let mut out = Vec::new();
    for sub in l.find_all("component").chain(l.find_all("comp")) {
        let image = string_arg(sub).ok_or_else(|| invalid(sub.line, "component name expected"))?;
        let places = sub
            .find_all("place")
            .map(read_place)
            .collect::<Result<_, _>>()?;
        out.push(ComponentPlacement { image, places });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Network

fn pin_ref(a: &Atom) -> Option<PinRef> {
    let (text, split) = match a.quoted_len {
        Some(n) => (a.text.as_str(), n),
        None => {
            // Java skips leading hyphens before the component name.
            let t = a.text.trim_start_matches('-');
            (t, t.find('-')?)
        }
    };
    let component = &text[..split];
    let pin = text[split..].strip_prefix('-')?;
    if component.is_empty() {
        return None;
    }
    Some(PinRef {
        component: component.to_string(),
        pin: pin.to_string(),
    })
}

fn read_pins(l: &List, warnings: &mut Vec<String>) -> Vec<PinRef> {
    let mut out = Vec::new();
    for a in l.args().iter().filter_map(Sexpr::as_atom) {
        if a.text.is_empty() {
            continue;
        }
        match pin_ref(a) {
            Some(p) => out.push(p),
            None => warnings.push(format!("line {}: bad pin reference '{}'", a.line, a.text)),
        }
    }
    out
}

fn read_net(l: &List, warnings: &mut Vec<String>) -> Option<Net> {
    let name = string_arg(l)?;
    let subnet = l.args().get(1).and_then(Sexpr::as_atom).and_then(Atom::as_int);
    let mut net = Net {
        name,
        subnet: subnet.unwrap_or(1),
        pins: Vec::new(),
        ordered: false,
        fromtos: Vec::new(),
        rules: Vec::new(),
    };
    for sub in l.sublists() {
        if sub.is("pins") {
            net.pins.extend(read_pins(sub, warnings));
        } else if sub.is("order") {
            net.ordered = true;
            net.pins.extend(read_pins(sub, warnings));
        } else if sub.is("fromto") {
            net.fromtos.push(read_pins(sub, warnings));
        } else if sub.is("rule") {
            net.rules.extend(read_rules(sub));
        } else if sub.is("layer_rule") {
            warnings.push(format!("line {}: net layer_rule not implemented", sub.line));
        }
    }
    Some(net)
}

fn read_circuit(l: &List, c: &mut NetClass) {
    for sub in l.sublists() {
        if sub.is("length") {
            let v: Vec<f64> = sub.leading_atoms().filter_map(Atom::as_num).take(2).collect();
            if v.len() == 2 {
                // Java stores them swapped into (max, min) and reads them back
                // as maxLength = arr[0], minLength = arr[1].
                c.max_trace_length = v[0];
                c.min_trace_length = v[1];
            }
        } else if sub.is("use_via") {
            c.use_via.extend(read_via_padstacks(sub));
        } else if sub.is("use_layer") {
            c.use_layer.extend(sub.leading_atoms().map(|a| a.text.clone()));
        }
    }
}

fn read_net_class(l: &List) -> Option<NetClass> {
    let mut atoms = l.leading_atoms();
    // Java reads an empty name if the class name is missing.
    let name = atoms.next().map(|a| a.text.clone()).unwrap_or_default();
    let nets = atoms
        .filter(|a| !a.text.is_empty())
        .map(|a| a.text.clone())
        .collect();
    let mut c = NetClass {
        name,
        nets,
        rules: Vec::new(),
        layer_rules: Vec::new(),
        use_via: Vec::new(),
        use_layer: Vec::new(),
        via_rule: None,
        clearance_class: None,
        shove_fixed: false,
        pull_tight: true,
        min_trace_length: 0.0,
        max_trace_length: 0.0,
    };
    for sub in l.sublists() {
        let Some(head) = sub.head() else { continue };
        match head.to_ascii_lowercase().as_str() {
            "rule" => c.rules.extend(read_rules(sub)),
            "layer_rule" => c.layer_rules.push(read_layer_rule(sub)),
            "via_rule" => c.via_rule = string_arg(sub),
            "circuit" => read_circuit(sub, &mut c),
            "clearance_class" => c.clearance_class = Some(string_arg(sub)?),
            "shove_fixed" => c.shove_fixed = on_off(sub),
            "pull_tight" => c.pull_tight = on_off(sub),
            _ => {}
        }
    }
    Some(c)
}

fn read_network(l: &List, warnings: &mut Vec<String>) -> Result<Network, DsnError> {
    let mut n = Network::default();
    for sub in l.sublists() {
        let Some(head) = sub.head() else { continue };
        match head.to_ascii_lowercase().as_str() {
            "net" => n.nets.extend(read_net(sub, warnings)),
            "via" => {
                let a: Vec<&Atom> = sub.leading_atoms().collect();
                if a.len() < 3 {
                    return Err(invalid(sub.line, "network via: name padstack class expected"));
                }
                n.vias.push(ViaInfo {
                    name: a[0].text.clone(),
                    padstack: a[1].text.clone(),
                    clearance_class: a[2].text.clone(),
                    attach: a.get(3).is_some_and(|x| x.is_kw("attach")),
                });
            }
            "via_rule" => n
                .via_rules
                .push(sub.leading_atoms().map(|a| a.text.clone()).collect()),
            "class" => n.classes.push(
                read_net_class(sub).ok_or_else(|| invalid(sub.line, "malformed net class"))?,
            ),
            "class_class" => n.class_classes.push(ClassClass {
                classes: sub
                    .find_all("classes")
                    .flat_map(|c| c.leading_atoms().map(|a| a.text.clone()))
                    .collect(),
                rules: sub.find_all("rule").flat_map(read_rules).collect(),
                layer_rules: sub.find_all("layer_rule").map(read_layer_rule).collect(),
            }),
            _ => {}
        }
    }
    Ok(n)
}

// ---------------------------------------------------------------------------
// Wiring

fn read_fixed(l: &List) -> FixedState {
    match first_atom(l).map(|a| a.text.to_ascii_lowercase()) {
        Some(t) if t == "shove_fixed" => FixedState::ShoveFixed,
        Some(t) if t == "fix" => FixedState::SystemFixed,
        Some(t) if t == "normal" => FixedState::Unfixed,
        Some(_) => FixedState::UserFixed,
        None => FixedState::Unfixed,
    }
}

fn read_net_id(l: &List) -> Option<NetId> {
    let name = string_arg(l)?;
    let subnet = l
        .args()
        .get(1)
        .and_then(Sexpr::as_atom)
        .and_then(Atom::as_int)
        .unwrap_or(0);
    Some(NetId { name, subnet })
}

fn read_wiring(l: &List, warnings: &mut Vec<String>) -> Result<Wiring, DsnError> {
    let mut w = Wiring::default();
    for sub in l.sublists() {
        if sub.is("wire") {
            let mut shape = None;
            let mut holes = Vec::new();
            let mut net = None;
            let mut clearance_class = None;
            let mut fixed = FixedState::Unfixed;
            for item in sub.sublists() {
                if is_shape_kw(item) {
                    // The last shape wins, as in Java.
                    shape = read_shape(item, warnings).or(shape);
                } else if item.is("window") {
                    if let Some(s) = item.sublists().next().and_then(|s| read_shape(s, warnings)) {
                        holes.push(s);
                    }
                } else if item.is("net") {
                    net = read_net_id(item);
                } else if item.is("clearance_class") {
                    clearance_class = string_arg(item);
                } else if item.is("type") {
                    fixed = read_fixed(item);
                }
            }
            match shape {
                Some(shape) => w.wires.push(Wire {
                    shape,
                    holes,
                    net,
                    clearance_class,
                    fixed,
                }),
                None => warnings.push(format!("line {}: wire has no shape", sub.line)),
            }
        } else if sub.is("via") {
            let args = sub.args();
            let padstack = args
                .first()
                .and_then(Sexpr::as_atom)
                .ok_or_else(|| invalid(sub.line, "via padstack name expected"))?
                .text
                .clone();
            let num = |i: usize| args.get(i).and_then(Sexpr::as_atom).and_then(Atom::as_num);
            let (Some(x), Some(y)) = (num(1), num(2)) else {
                return Err(invalid(sub.line, "via: coordinates expected"));
            };
            let mut via = WiringVia {
                padstack,
                x,
                y,
                net: None,
                clearance_class: None,
                fixed: FixedState::Unfixed,
            };
            for item in sub.sublists() {
                if item.is("net") {
                    via.net = read_net_id(item);
                } else if item.is("clearance_class") {
                    via.clearance_class = string_arg(item);
                } else if item.is("type") {
                    via.fixed = read_fixed(item);
                }
            }
            w.vias.push(via);
        }
    }
    Ok(w)
}

// ---------------------------------------------------------------------------
// Top level

fn read_parser(l: &List, p: &mut ParserInfo) {
    for sub in l.sublists() {
        let Some(head) = sub.head() else { continue };
        match head.to_ascii_lowercase().as_str() {
            "string_quote" => {
                if let Some(q) = string_arg(sub) {
                    p.string_quote = q;
                }
            }
            "host_cad" => p.host_cad = string_arg(sub),
            "host_version" => p.host_version = string_arg(sub),
            "constant" => {
                let a: Vec<&Atom> = sub.leading_atoms().collect();
                if a.len() == 2 {
                    p.constants.push((a[0].text.clone(), a[1].text.clone()));
                }
            }
            "write_resolution" => {
                let a: Vec<&Atom> = sub.leading_atoms().collect();
                if let (Some(u), Some(v)) = (a.first(), a.get(1).and_then(|x| x.as_int())) {
                    p.write_resolution = Some((u.text.clone(), v));
                }
            }
            "generated_by_freeroute" => p.generated_by_host = false,
            _ => {}
        }
    }
}

impl Dsn {
    pub fn parse(src: &[u8]) -> Result<Dsn, DsnError> {
        let top = sexpr::parse(src).map_err(DsnError::Syntax)?;
        let pcb = top
            .first()
            .and_then(Sexpr::as_list)
            .filter(|l| l.is("pcb"))
            .ok_or_else(|| DsnError::NotDsn("expected '(pcb <name>' header".into()))?;
        Self::from_pcb(pcb)
    }

    pub fn from_pcb(pcb: &List) -> Result<Dsn, DsnError> {
        let mut warnings = Vec::new();
        let name = string_arg(pcb).unwrap_or_default();
        let mut dsn = Dsn {
            name,
            parser: ParserInfo::default(),
            unit: Unit::Mil,
            resolution: 100,
            structure: Structure::default(),
            library: Library::default(),
            placement: Vec::new(),
            network: Network::default(),
            wiring: Wiring::default(),
            part_library: None,
            warnings: Vec::new(),
        };
        for sub in pcb.sublists() {
            let Some(head) = sub.head() else { continue };
            match head.to_ascii_lowercase().as_str() {
                "parser" => read_parser(sub, &mut dsn.parser),
                "resolution" => {
                    let a: Vec<&Atom> = sub.leading_atoms().collect();
                    let unit = a.first().and_then(|u| Unit::parse(&u.text));
                    let value = a.get(1).and_then(|v| v.as_int());
                    match (unit, value) {
                        (Some(u), Some(v)) => {
                            dsn.unit = u;
                            dsn.resolution = v;
                        }
                        _ => return Err(invalid(sub.line, "resolution: unit and integer expected")),
                    }
                }
                "structure" => {
                    // Java skips plane windows for Allegro files.
                    let allegro = dsn
                        .parser
                        .host_cad
                        .as_deref()
                        .is_some_and(|h| h.eq_ignore_ascii_case("allegro"));
                    dsn.structure = read_structure(sub, allegro, &mut warnings)
                }
                "library" => dsn.library = read_library(sub, &mut warnings)?,
                "placement" => dsn.placement = read_placement(sub)?,
                "network" => dsn.network = read_network(sub, &mut warnings)?,
                "wiring" => dsn.wiring = read_wiring(sub, &mut warnings)?,
                "part_library" => dsn.part_library = Some(sub.clone()),
                "place_control" => {
                    if let Some(f) = sub.find("flip_style") {
                        dsn.structure.flip_style_rotate_first |=
                            first_atom(f).is_some_and(|a| a.is_kw("rotate_first"));
                    }
                }
                _ => {}
            }
        }
        dsn.warnings = warnings;
        Ok(dsn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"(pcb "x.dsn"
  (parser (string_quote ") (space_in_quoted_tokens on) (host_cad "KiCad's Pcbnew") (host_version "4.0.7"))
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal) (property (index 0)))
    (layer B.Cu (type signal) (property (index 1)))
    (boundary (path pcb 0  0 0  100 0  100 100  0 100  0 0))
    (keepout "" (polygon signal 0  1 1  2 2  3 1))
    (via "Via[0-1]_600:400_um" "Via[0-1]_1016:485.7_um")
    (rule (width 250) (clearance 200.1) (clearance 50 (type smd_smd)))
  )
  (placement
    (component "C-150P_0"
      (place C1 27475 -24775 front 270 (PN "100uF 16V"))
      (place C9)
    )
  )
  (library
    (image "C-150P_0"
      (outline (path signal 254  889 -1143  254 -1143))
      (pin Round[A]Pad_1524_um (rotate 180) P -1905 0)
      (pin Round[A]Pad_1524_um N 1905 0)
    )
    (padstack Round[A]Pad_1524_um (shape (circle F.Cu 1524)) (shape (circle B.Cu 1524)) (attach off))
  )
  (network
    (net GND (pins C1-N "CR2032-3V"-2))
    (net "Net-(C1-P)" 2 (pins C1-P))
    (class kicad_default "" GND "Net-(C1-P)"
      (circuit (use_via Via[0-1]_600:400_um))
      (rule (width 250) (clearance 200.1))
    )
  )
  (wiring
    (wire (path F.Cu 250  0 0  10 10) (net GND) (type protect))
    (via "Via[0-1]_600:400_um"  5 5 (net GND 1) (type route))
  )
)"#;

    #[test]
    fn sample() {
        let d = Dsn::parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(d.unit, Unit::Um);
        assert_eq!(d.resolution, 10);
        assert_eq!(d.parser.host_cad.as_deref(), Some("KiCad's Pcbnew"));
        assert_eq!(d.structure.layers.len(), 2);
        assert_eq!(d.structure.boundaries.len(), 1);
        assert_eq!(d.structure.keepouts.len(), 1);
        assert_eq!(d.structure.keepouts[0].1.name, None);
        assert_eq!(d.structure.via_padstacks.as_ref().unwrap().len(), 2);
        assert_eq!(
            d.structure.rules[2],
            Rule::Clearance {
                value: 50.0,
                class_pairs: vec!["smd_smd".into()]
            }
        );
        let places = &d.placement[0].places;
        assert_eq!(places[0].rotation, 270.0);
        assert_eq!(places[0].part_number.as_deref(), Some("100uF 16V"));
        assert_eq!(places[1].location, None);
        let img = &d.library.images[0];
        assert_eq!(img.pins[0].rotation, 180.0);
        assert_eq!(img.pins[0].name, "P");
        assert!(!d.library.padstacks[0].attach);
        let n = &d.network.nets;
        assert_eq!(n[0].pins[1].component, "CR2032-3V");
        assert_eq!(n[0].pins[1].pin, "2");
        assert_eq!(n[1].name, "Net-(C1-P)");
        assert_eq!(n[1].subnet, 2);
        assert_eq!(d.network.classes[0].nets, vec!["GND", "Net-(C1-P)"]);
        assert_eq!(d.network.classes[0].use_via, vec!["Via[0-1]_600:400_um"]);
        assert_eq!(d.wiring.wires[0].fixed, FixedState::UserFixed);
        assert_eq!(d.wiring.vias[0].net.as_ref().unwrap().subnet, 1);
        assert_eq!(d.wiring.vias[0].fixed, FixedState::UserFixed);
    }

    #[test]
    fn int_literals() {
        let a = |t: &str| Atom {
            text: t.into(),
            quoted: false,
            quoted_len: None,
            line: 0,
        };
        assert_eq!(a("12").as_int(), Some(12));
        assert_eq!(a("-3").as_int(), Some(-3));
        assert_eq!(a("007").as_int(), None);
        assert_eq!(a("1.5").as_int(), None);
        assert_eq!(a("1.5e3").as_num(), Some(1500.0));
        assert_eq!(a("inf").as_num(), None);
    }
}
