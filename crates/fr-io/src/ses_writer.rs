//! Specctra session file writer (Java `io.specctra.SesWriter` with the `writeScope` /
//! `writeScopeInt` helpers of `io.specctra.parser.{Rectangle, Polygon, Circle}`,
//! `Resolution.writeScope` and `Parser.writeScope`). The output is byte-identical to Java's.

use std::collections::HashSet;
use std::io::{self, Write};

use fr_engine::board::{BasicBoard, ItemKey, ItemKind};
use fr_engine::datastructures::{IdentifierType, IndentFileWriter};
use fr_engine::ids::{FixedState, NetNo};
use fr_engine::library::Padstack;
use fr_engine::structure::{CoordinateTransform, DsnShapeCoords};
use fr_geom::{Area, FloatPoint, Shape};
use fr_jcompat::double_to_string;

type W<'a> = IndentFileWriter<&'a mut dyn Write>;

/// Java `(int) Math.round(x)`.
fn round(x: f64) -> i32 {
    fr_jcompat::math_round_i32(x)
}

/// Java `String.format(Locale.ENGLISH, "%.3f", v)`: `HALF_UP` rounding of the shortest
/// decimal representation of `v` (like `java.util.Formatter`).
fn format_3(v: f64) -> String {
    let s = format!("{}", v.abs());
    let (ip, fp) = s.split_once('.').unwrap_or((s.as_str(), ""));
    let fb = fp.as_bytes();
    let mut digits: Vec<u8> = ip.bytes().map(|c| c - b'0').collect();
    for i in 0..3 {
        digits.push(fb.get(i).map_or(0, |c| c - b'0'));
    }
    if fb.get(3).is_some_and(|c| *c >= b'5') {
        let mut i = digits.len();
        loop {
            if i == 0 {
                digits.insert(0, 1);
                break;
            }
            i -= 1;
            if digits[i] == 9 {
                digits[i] = 0;
            } else {
                digits[i] += 1;
                break;
            }
        }
    }
    let n = digits.len();
    let text: String = digits.iter().map(|d| (b'0' + d) as char).collect();
    let sign = if v.is_sign_negative() { "-" } else { "" };
    format!("{sign}{}.{}", &text[..n - 3], &text[n - 3..])
}

/// Java `SesWriter.formatPlacementRotation`.
pub fn format_placement_rotation(degrees: f64) -> String {
    let rounded = (degrees * 1000.0).round_ties_even() / 1000.0;
    if (rounded - rounded.round_ties_even()).abs() < 1e-9 {
        // "%.0f" of a (nearly) integral value
        return format!("{rounded:.0}");
    }
    let formatted = format_3(rounded);
    let t = formatted.trim_end_matches('0');
    t.strip_suffix('.').unwrap_or(t).to_string()
}

struct Ctx<'b> {
    board: &'b BasicBoard,
    id: IdentifierType,
    ct: CoordinateTransform,
}

fn write_shape_int(
    file: &mut W<'_>,
    id: &IdentifierType,
    layer: &str,
    s: &DsnShapeCoords,
) -> io::Result<()> {
    match s {
        DsnShapeCoords::Rectangle(c) => {
            file.new_line();
            file.write("(rect ")?;
            id.write(layer, file)?;
            for v in c {
                file.write(" ")?;
                file.write(&round(*v).to_string())?;
            }
            file.write(")")
        }
        DsnShapeCoords::Polygon(c) => {
            file.start_scope(true);
            file.write("polygon ")?;
            id.write(layer, file)?;
            file.write(" ")?;
            file.write("0")?;
            for i in 0..c.len() / 2 {
                file.new_line();
                file.write(&round(c[2 * i]).to_string())?;
                file.write(" ")?;
                file.write(&round(c[2 * i + 1]).to_string())?;
            }
            file.end_scope();
            Ok(())
        }
        DsnShapeCoords::Circle { diameter, x, y } => {
            file.new_line();
            file.write("(circle ")?;
            id.write(layer, file)?;
            for v in [diameter, x, y] {
                file.write(" ")?;
                file.write(&round(*v).to_string())?;
            }
            file.write(")")
        }
    }
}

/// `Shape.writeScope` (double coordinates, `Double.toString`).
fn write_shape(
    file: &mut W<'_>,
    id: &IdentifierType,
    layer: &str,
    s: &DsnShapeCoords,
) -> io::Result<()> {
    let d = |v: f64| double_to_string(v);
    match s {
        DsnShapeCoords::Rectangle(c) => {
            file.new_line();
            file.write("(rect ")?;
            id.write(layer, file)?;
            for v in c {
                file.write(" ")?;
                file.write(&d(*v))?;
            }
            file.write(")")
        }
        DsnShapeCoords::Polygon(c) => {
            file.start_scope(true);
            file.write("polygon ")?;
            id.write(layer, file)?;
            file.write(" ")?;
            file.write("0")?;
            for i in 0..c.len() / 2 {
                file.new_line();
                file.write(&d(c[2 * i]))?;
                file.write(" ")?;
                file.write(&d(c[2 * i + 1]))?;
            }
            file.end_scope();
            Ok(())
        }
        DsnShapeCoords::Circle { diameter, x, y } => {
            file.new_line();
            file.write("(circle ")?;
            id.write(layer, file)?;
            for v in [diameter, x, y] {
                file.write(" ")?;
                file.write(&d(*v))?;
            }
            file.write(")")
        }
    }
}

fn write_fixed_state(file: &mut W<'_>, f: FixedState) -> io::Result<()> {
    if f <= FixedState::ShoveFixed {
        return Ok(());
    }
    file.new_line();
    file.write("(type ")?;
    file.write(if f == FixedState::SystemFixed {
        "fix)"
    } else {
        "protect)"
    })
}

impl Ctx<'_> {
    fn write_resolution(&self, file: &mut W<'_>) -> io::Result<()> {
        let c = &self.board.communication;
        file.new_line();
        file.write("(resolution ")?;
        file.write(&c.unit.to_string())?;
        file.write(" ")?;
        file.write(&c.resolution.to_string())?;
        file.write(")")
    }

    fn write_session(
        &self,
        file: &mut W<'_>,
        session_name: &str,
        design_name: &str,
    ) -> io::Result<()> {
        file.start_scope(false);
        file.write("session ")?;
        self.id.write(session_name, file)?;
        file.new_line();
        file.write("(base_design ")?;
        self.id.write(design_name, file)?;
        file.write(")")?;
        self.write_placement(file)?;
        self.write_was_is(file)?;
        self.write_routes(file)?;
        file.end_scope();
        Ok(())
    }

    fn write_placement(&self, file: &mut W<'_>) -> io::Result<()> {
        let b = self.board;
        file.start_scope(true);
        file.write("placement")?;
        self.write_resolution(file)?;
        // Components with at least one item on the board.
        let with_items: HashSet<i32> = b
            .get_items()
            .iter()
            .map(|k| b.item(*k).component_no())
            .filter(|c| *c > 0)
            .collect();
        for pkg in b.library.packages.iter() {
            let mut found = false;
            for c in b.components.get_all() {
                if c.get_package() != pkg.id || !with_items.contains(&c.id) {
                    continue;
                }
                if !found {
                    file.start_scope(true);
                    file.write("component ")?;
                    self.id.write(&pkg.name, file)?;
                    found = true;
                }
                file.new_line();
                file.write("(place ")?;
                self.id.write(&c.name, file)?;
                let loc = self
                    .ct
                    .board_to_dsn_point(&c.get_location().expect("placed").to_float());
                file.write(" ")?;
                file.write(&round(loc[0]).to_string())?;
                file.write(" ")?;
                file.write(&round(loc[1]).to_string())?;
                file.write(if c.placed_on_front() {
                    " front "
                } else {
                    " back "
                })?;
                file.write(&format_placement_rotation(c.get_rotation_in_degree()))?;
                if c.position_fixed && !b.communication.host_cad_is_kicad() {
                    file.new_line();
                    file.write(" (lock_type position)")?;
                }
                file.write(")")?;
            }
            if found {
                file.end_scope();
            }
        }
        file.end_scope();
        Ok(())
    }

    fn pin_name(&self, key: ItemKey) -> Option<String> {
        let b = self.board;
        let it = b.item(key);
        let ItemKind::Pin(p) = &it.kind else {
            return None;
        };
        let comp = b.components.get(it.component_no());
        let pkg = b.library.packages.get(comp.get_package());
        let pin = pkg.get_pin(p.pin_index)?;
        Some(format!("{}\u{0}{}", comp.name, pin.name))
    }

    fn write_was_is(&self, file: &mut W<'_>) -> io::Result<()> {
        let b = self.board;
        file.start_scope(true);
        file.write("was_is")?;
        for k in b.get_pins() {
            let ItemKind::Pin(p) = &b.item(k).kind else {
                continue;
            };
            let Some(to) = p.changed_to else { continue };
            let Some(to_key) = b.get_item(to) else {
                continue;
            };
            file.new_line();
            file.write("(pins ")?;
            for (i, key) in [k, to_key].into_iter().enumerate() {
                if i == 1 {
                    file.write(" ")?;
                }
                if let Some(n) = self.pin_name(key) {
                    let (c, p) = n.split_once('\u{0}').expect("sep");
                    self.id.write(c, file)?;
                    file.write("-")?;
                    self.id.write(p, file)?;
                }
            }
            file.write(")")?;
        }
        file.end_scope();
        Ok(())
    }

    fn write_routes(&self, file: &mut W<'_>) -> io::Result<()> {
        file.start_scope(true);
        file.write("routes ")?;
        self.write_resolution(file)?;
        self.write_parser(file)?;
        self.write_library(file)?;
        self.write_network(file)?;
        file.end_scope();
        Ok(())
    }

    /// Java `Parser.writeScope(file, info, identifierType, reduced = true)`.
    fn write_parser(&self, file: &mut W<'_>) -> io::Result<()> {
        file.start_scope(true);
        file.write("parser")?;
        if let Some(info) = &self.board.communication.specctra_parser_info {
            if let Some(h) = &info.host_cad {
                file.new_line();
                file.write("(host_cad ")?;
                self.id.write(h, file)?;
                file.write(")")?;
            }
            if let Some(h) = &info.host_version {
                file.new_line();
                file.write("(host_version ")?;
                self.id.write(h, file)?;
                file.write(")")?;
            }
            if let Some(consts) = &info.constants {
                for c in consts {
                    file.new_line();
                    file.write("(constant ")?;
                    for s in c {
                        self.id.write(s, file)?;
                        file.write(" ")?;
                    }
                    file.write(")")?;
                }
            }
            if let Some(wr) = &info.write_resolution {
                file.new_line();
                file.write("(write_resolution ")?;
                // Java charName.substring(0, 1)
                let first: String = wr
                    .char_name
                    .encode_utf16()
                    .take(1)
                    .map(|u| char::from_u32(u as u32).unwrap_or('?'))
                    .collect();
                file.write(&first)?;
                file.write(" ")?;
                file.write(&wr.positive_int.to_string())?;
                file.write(")")?;
            }
        }
        file.end_scope();
        Ok(())
    }

    fn write_library(&self, file: &mut W<'_>) -> io::Result<()> {
        let b = self.board;
        file.start_scope(true);
        file.write("library_out ")?;
        let mut written: HashSet<String> = HashSet::new();
        for i in 0..b.library.via_padstack_count() {
            let Some(p) = b.library.get_via_padstack(i) else {
                continue;
            };
            if !written.insert(p.name.clone()) {
                continue;
            }
            self.write_padstack(p, file)?;
        }
        file.end_scope();
        Ok(())
    }

    fn write_padstack(&self, p: &Padstack, file: &mut W<'_>) -> io::Result<()> {
        let b = self.board;
        let n = b.layer_structure.layer_count();
        let mut first = 0;
        while first < n && p.get_shape(first).is_none() {
            first += 1;
        }
        let mut last = n - 1;
        while last >= 0 && p.get_shape(last).is_none() {
            last -= 1;
        }
        if first >= n || last < 0 {
            log::warn!("SesWriter.writePadstack: padstack shape not found");
            return Ok(());
        }
        file.start_scope(true);
        file.write("padstack ")?;
        self.id.write(&p.name, file)?;
        for i in first..=last {
            let Some(s) = p.get_shape(i) else { continue };
            let layer = &b.layer_structure.layers[i as usize].name;
            let coords = self
                .ct
                .board_to_dsn_rel_shape(&s.to_shape())
                .expect("shape");
            file.start_scope(true);
            file.write("shape")?;
            write_shape_int(file, &self.id, layer, &coords)?;
            file.end_scope();
        }
        if !p.attach_allowed {
            file.new_line();
            file.write("(attach off)")?;
        }
        file.end_scope();
        Ok(())
    }

    fn write_network(&self, file: &mut W<'_>) -> io::Result<()> {
        file.start_scope(true);
        file.write("network_out ")?;
        for n in 1..=self.board.rules.nets.max_net_number() {
            self.write_net(n, file)?;
        }
        file.end_scope();
        Ok(())
    }

    fn write_net(&self, net: NetNo, file: &mut W<'_>) -> io::Result<()> {
        let b = self.board;
        let mut header = false;
        for k in b.get_connectable_items(net) {
            let it = b.item(k);
            if it.fixed_state() == FixedState::SystemFixed {
                continue;
            }
            let is_wire = it.is_trace();
            let is_via = it.is_via();
            let is_area = it.is_conduction_area()
                && b.layer_structure.layers[it.first_layer(b) as usize].is_signal;
            if !header && (is_wire || is_via || is_area) {
                file.start_scope(true);
                file.write("net ")?;
                match b.rules.nets.get(net) {
                    Some(n) => self.id.write(&n.name, file)?,
                    None => log::warn!("SesWriter.writeNet: net not found"),
                }
                header = true;
            }
            if is_wire {
                self.write_wire(k, file)?;
            } else if is_via {
                self.write_via(k, file)?;
            } else if is_area {
                self.write_conduction_area(k, file)?;
            }
        }
        if header {
            file.end_scope();
        }
        Ok(())
    }

    /// Java `SesWriter.snappedEndpoint`.
    fn snapped_endpoint(&self, key: ItemKey, start: bool) -> Option<FloatPoint> {
        let b = self.board;
        let wire = b.item(key);
        let corner = if start {
            wire.first_corner()
        } else {
            wire.last_corner()
        };
        let corner_f = corner.to_float();
        let contacts = if start {
            b.trace_start_contacts(key)
        } else {
            b.trace_end_contacts(key)
        };
        let ItemKind::Trace(t) = &wire.kind else {
            return None;
        };
        let layer = t.layer();
        for c in contacts.iter() {
            let d = b.item(c);
            if !d.is_drill_item() {
                continue;
            }
            if layer < d.first_layer(b) || layer > d.last_layer(b) {
                continue;
            }
            let Some(pad) = d.drill_shape(b, layer - d.first_layer(b)) else {
                continue;
            };
            let center = d.center(b).to_float();
            let dist = corner_f.distance(&center);
            if dist <= 0.5 {
                return None;
            }
            if dist <= pad.border_distance(&center) {
                return Some(center);
            }
        }
        None
    }

    fn write_wire(&self, key: ItemKey, file: &mut W<'_>) -> io::Result<()> {
        let b = self.board;
        let it = b.item(key);
        let ItemKind::Trace(t) = &it.kind else {
            unreachable!()
        };
        let layer = &b.layer_structure.layers[t.layer() as usize].name;
        let width = round(self.ct.board_to_dsn((2 * t.half_width()) as f64));
        file.start_scope(true);
        file.write("wire")?;
        let corners = t.polyline().corners();
        let n = corners.len();
        let mut coors: Vec<i32> = Vec::with_capacity(2 * n);
        let mut prev: Option<[i32; 2]> = None;
        for (i, c) in corners.iter().enumerate() {
            let mut p = c.to_float();
            if i == 0 || i == n - 1 {
                if let Some(s) = self.snapped_endpoint(key, i == 0) {
                    p = s;
                }
            }
            let f = self.ct.board_to_dsn_point(&p);
            let cur = [round(f[0]), round(f[1])];
            if i == 0 || prev != Some(cur) {
                coors.extend(cur);
                prev = Some(cur);
            }
        }
        // writePath
        file.start_scope(true);
        file.write("path ")?;
        self.id.write(layer, file)?;
        file.write(" ")?;
        file.write(&width.to_string())?;
        for i in 0..coors.len() / 2 {
            file.new_line();
            file.write(&coors[2 * i].to_string())?;
            file.write(" ")?;
            file.write(&coors[2 * i + 1].to_string())?;
        }
        file.end_scope();
        write_fixed_state(file, it.fixed_state())?;
        file.end_scope();
        Ok(())
    }

    fn write_via(&self, key: ItemKey, file: &mut W<'_>) -> io::Result<()> {
        let b = self.board;
        let it = b.item(key);
        let ItemKind::Via(v) = &it.kind else {
            unreachable!()
        };
        let p = b
            .library
            .padstacks
            .get(v.padstack_no())
            .expect("via padstack");
        file.start_scope(true);
        file.write("via ")?;
        self.id.write(&p.name, file)?;
        file.write(" ")?;
        let loc = self.ct.board_to_dsn_point(&it.center(b).to_float());
        file.write(&round(loc[0]).to_string())?;
        file.write(" ")?;
        file.write(&round(loc[1]).to_string())?;
        write_fixed_state(file, it.fixed_state())?;
        file.end_scope();
        Ok(())
    }

    fn write_conduction_area(&self, key: ItemKey, file: &mut W<'_>) -> io::Result<()> {
        let b = self.board;
        let it = b.item(key);
        if it.net_count() != 1 {
            log::warn!("SesWriter.writeConductionArea: unexpected net count");
            return Ok(());
        }
        let ItemKind::ConductionArea(c) = &it.kind else {
            unreachable!()
        };
        let area = c.area().get_area(b);
        let layer = &b.layer_structure.layers[c.area().layer() as usize].name;
        let (border, holes): (Shape, Vec<Shape>) = match &*area {
            Area::Shape(s) => (s.clone(), Vec::new()),
            Area::PolylineArea(pa) => (
                Shape::from(pa.get_border().clone()),
                pa.get_holes()
                    .iter()
                    .map(|h| Shape::from(h.clone()))
                    .collect(),
            ),
        };
        file.start_scope(true);
        file.write("wire ")?;
        if let Some(s) = self.ct.board_to_dsn_shape(&border) {
            write_shape_int(file, &self.id, layer, &s)?;
        }
        for h in &holes {
            let s = self.ct.board_to_dsn_shape(h).expect("hole");
            file.start_scope(true);
            file.write("window")?;
            write_shape(file, &self.id, layer, &s)?;
            file.end_scope();
        }
        file.end_scope();
        Ok(())
    }
}

/// Java `SesWriter.write(board, out, designName)`: writes the session and flushes (does not
/// close) the stream.
pub fn write_ses(board: &BasicBoard, out: &mut dyn Write, design_name: &str) -> io::Result<()> {
    let session_name = design_name.replace(".dsn", ".ses");
    let reserved = ["(", ")", " ", ";", "-", "_", "/", "~", "{", "}"];
    let quote = board
        .communication
        .specctra_parser_info
        .as_ref()
        .map(|i| i.string_quote.clone())
        .unwrap_or_else(|| "\"".into());
    let id = IdentifierType::new(&reserved, &quote);
    let scale = board.communication.coordinate_transform.dsn_to_board(1.0)
        / board.communication.resolution as f64;
    let ctx = Ctx {
        board,
        id,
        ct: CoordinateTransform::new(scale, 0.0, 0.0),
    };
    let mut file: W<'_> = IndentFileWriter::new(out);
    ctx.write_session(&mut file, &session_name, design_name)?;
    file.flush()?;
    file.into_inner()?;
    Ok(())
}

/// [`write_ses`] into a byte vector.
pub fn ses_bytes(board: &BasicBoard, design_name: &str) -> Vec<u8> {
    let mut v: Vec<u8> = Vec::new();
    write_ses(board, &mut v, design_name).expect("writing to a Vec cannot fail");
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_format() {
        assert_eq!(format_placement_rotation(0.0), "0");
        assert_eq!(format_placement_rotation(339.0), "339");
        assert_eq!(format_placement_rotation(338.5), "338.5");
        assert_eq!(format_placement_rotation(135.25), "135.25");
        assert_eq!(format_placement_rotation(12.0004), "12");
        assert_eq!(format_placement_rotation(12.3456), "12.346");
        assert_eq!(format_placement_rotation(0.0105), "0.01"); // Math.rint: half even
        assert_eq!(format_placement_rotation(0.0115), "0.012");
        assert_eq!(format_placement_rotation(359.9996), "360");
    }
}
