//! The structure scope (Java `io.specctra.parser.Structure.readScope/createBoard`,
//! `insertKeepout`, planes, `insertMissingPowerPlanes`, `updateBoardRules`,
//! `setClearanceRule`, `separateHoles`).

use std::collections::HashSet;

use fr_dsn::model::{AngleRestriction as DsnAngle, KeepoutKind, Rule, ShapeKind, Unit as DsnUnit};
use fr_engine::datastructures::ItemIdGenerator;
use fr_engine::ids::{AngleRestriction, ClearanceClassNo, FixedState, LayerNo};
use fr_engine::library::BoardLibrary;
use fr_engine::rules::{BoardRules, ClearanceMatrix, ItemClass};
use fr_engine::structure::{
    Communication, Components, CoordinateTransform, Layer, LayerStructure, SpecctraParserInfo,
    Unit, WriteResolution,
};
use fr_geom::limits::CRIT_INT;
use fr_geom::{Area, IntBox, PolylineShape, Shape, TileShape};

use crate::error::{npe, LResult, LoadError};
use crate::loader::{Board, Loader};
use crate::netlist::NetId;
use crate::requests::{AreaRequest, InsertRequest};
use crate::shapes::{self, PLayer, PShape, ParserLayer, ParserLayers};

pub(crate) fn map_unit(u: DsnUnit) -> Unit {
    match u {
        DsnUnit::Inch => Unit::Inch,
        DsnUnit::Mil => Unit::Mil,
        DsnUnit::Mm => Unit::Mm,
        DsnUnit::Um => Unit::Um,
    }
}

pub(crate) fn map_fixed(f: fr_dsn::model::FixedState) -> FixedState {
    use fr_dsn::model::FixedState as F;
    match f {
        F::Unfixed => FixedState::Unfixed,
        F::ShoveFixed => FixedState::ShoveFixed,
        F::UserFixed => FixedState::UserFixed,
        F::SystemFixed => FixedState::SystemFixed,
    }
}

/// Java `(int) Math.round(x)`.
pub(crate) fn round_i32(x: f64) -> i32 {
    fr_geom::java_compat::math_round_i32(x)
}

/// Java `Structure.containsWireClearancePair`.
pub(crate) fn contains_wire_clearance_pair(pairs: &[String]) -> bool {
    pairs
        .iter()
        .any(|p| p.starts_with("wire_") || p.ends_with("_wire"))
}

/// Java `Structure.appendClearanceClass`.
fn append_clearance_class(rules: &mut BoardRules, name: &str) -> ClearanceClassNo {
    rules.clearance_matrix.append_class(name);
    let result = rules.clearance_matrix.get_no(name);
    let d = rules.get_default_net_class();
    let item_class = match name {
        "via" => Some(ItemClass::Via),
        "pin" => Some(ItemClass::Pin),
        "smd" => Some(ItemClass::Smd),
        "area" => Some(ItemClass::Area),
        _ => None,
    };
    if let Some(ic) = item_class {
        rules.net_classes[d]
            .default_item_clearance_classes
            .set(ic, result);
    }
    result
}

fn set_matrix_value(rules: &mut BoardRules, i: i32, j: i32, layer: LayerNo, value: i32) {
    if layer < 0 {
        rules.clearance_matrix.set_value_all_layers(i, j, value);
    } else {
        rules.clearance_matrix.set_value(i, j, layer, value);
    }
}

/// Java `Structure.setClearanceRule`: returns true if `smd_to_turn_gap` was found.
pub(crate) fn set_clearance_rule(
    value: f64,
    pairs: &[String],
    layer: LayerNo,
    ct: &CoordinateTransform,
    rules: &mut BoardRules,
    string_quote: &str,
) -> bool {
    let mut result = false;
    let clearance = round_i32(ct.dsn_to_board(value));
    if pairs.is_empty() {
        if layer < 0 {
            rules.clearance_matrix.set_default_value(clearance);
        } else {
            rules
                .clearance_matrix
                .set_default_value_on_layer(layer, clearance);
        }
        return result;
    }
    if contains_wire_clearance_pair(pairs) {
        for n in ["via", "smd", "pin", "area"] {
            append_clearance_class(rules, n);
        }
    }
    for current in pairs {
        if current.eq_ignore_ascii_case("smd_to_turn_gap") {
            rules.set_pin_edge_to_turn_dist(clearance as f64);
            result = true;
            continue;
        }
        let pair: [String; 2];
        if pairs.len() == 2 {
            let mut p = [pairs[0].clone(), pairs[1].clone()];
            for i in 0..2 {
                p[i] = p[i].replace('"', "");
                if let Some(s) = p[1].strip_prefix('_') {
                    p[1] = s.to_string();
                }
            }
            pair = p;
        } else if !string_quote.is_empty() && current.starts_with(string_quote) {
            let rest = &current[string_quote.len()..];
            let mut it = rest.splitn(2, string_quote);
            let first = it.next().unwrap_or("").to_string();
            match it.next() {
                Some(second) if second.starts_with('_') => {
                    pair = [first, second[1..].to_string()];
                }
                _ => {
                    log::warn!("Structure.set_clearance_rule: '_' expected at '{rest}'");
                    continue;
                }
            }
        } else {
            let mut it = current.splitn(2, '_');
            let first = it.next().unwrap_or("").to_string();
            match it.next() {
                Some(second) => pair = [first, second.to_string()],
                // pairs with more than 1 underline like smd_via_same_net are not implemented
                None => continue,
            }
        }
        let class_no = |rules: &mut BoardRules, name: &str| -> ClearanceClassNo {
            let no = if name == "wire" {
                1
            } else {
                rules.clearance_matrix.get_no(name)
            };
            if no < 0 {
                append_clearance_class(rules, name)
            } else {
                no
            }
        };
        let first = class_no(rules, &pair[0]);
        let second = class_no(rules, &pair[1]);
        set_matrix_value(rules, first, second, layer, clearance);
        set_matrix_value(rules, second, first, layer, clearance);
    }
    result
}

/// Java `Structure.separateHoles`: removes the outline shapes that are holes of another
/// outline shape and returns them (in list order).
fn separate_holes(outline: &mut Vec<PolylineShape>) -> Vec<PolylineShape> {
    struct OutlineShape {
        bb: IntBox,
        convex: Option<Vec<TileShape>>,
        is_hole: bool,
    }
    let mut shapes: Vec<OutlineShape> = outline
        .iter()
        .map(|s| OutlineShape {
            bb: s.bounding_box(),
            convex: s.split_to_convex(),
            is_hole: false,
        })
        .collect();
    let n = shapes.len();
    for i in 0..n {
        for j in 0..n {
            if i == j || shapes[j].is_hole {
                continue;
            }
            // otherShape.boundingBox.contains(currentShape.boundingBox)
            if !shapes[i].bb.is_contained_in(&shapes[j].bb) {
                continue;
            }
            // otherShape.containsAllCorners(currentShape)
            let contains_all = match &shapes[j].convex {
                None => false,
                Some(convex) => {
                    let cur = &outline[i];
                    (0..cur.border_line_count()).all(|c| {
                        let corner = cur.corner(c);
                        convex.iter().any(|t| t.contains(&corner))
                    })
                }
            };
            shapes[i].is_hole = contains_all;
        }
    }
    let mut holes = Vec::new();
    let mut kept = Vec::new();
    for (s, info) in outline.drain(..).zip(shapes.iter()) {
        if info.is_hole {
            holes.push(s);
        } else {
            kept.push(s);
        }
    }
    *outline = kept;
    holes
}

impl Loader<'_> {
    pub(crate) fn read_structure(&mut self) -> LResult<()> {
        let dsn = self.dsn;
        let s = &dsn.structure;
        if self.board.is_some() {
            // Java: createBoard is only called while the board is null.
            self.warn("Structure: board already created, second structure scope ignored");
            return Ok(());
        }

        // Layers: only layers with a known type enter the layer structure.
        self.parser_layers.clear();
        for l in s.layers.iter().filter(|l| l.type_ok) {
            let no = self.parser_layers.len() as LayerNo;
            self.parser_layers.push(ParserLayer {
                name: l.name.clone(),
                no,
                is_signal: l.is_signal,
                net_names: l.net_names.clone(),
            });
        }
        // Number of valid layers among the first k layer scopes.
        let valid_prefix = |k: usize| s.layers.iter().take(k).filter(|l| l.type_ok).count();

        self.via_at_smd = s.via_at_smd;
        if let Some(a) = s.snap_angle {
            self.snap_angle = match a {
                DsnAngle::None => AngleRestriction::None,
                DsnAngle::NinetyDegree => AngleRestriction::NinetyDegree,
                DsnAngle::FortyfiveDegree => AngleRestriction::FortyfiveDegree,
            };
        }
        if self.order.structure_desync {
            self.warn(
                "Structure: autoroute_settings after a keepout/plane scope; Java ends the structure \
                 scope early there (not emulated)",
            );
        }
        if let Some((scope, k)) = &self.order.autoroute_settings {
            let names: Vec<&str> = self.parser_layers[..valid_prefix(*k)]
                .iter()
                .map(|l| l.name.as_str())
                .collect();
            self.autoroute_settings = fr_settings::read_autoroute_settings(scope, &names);
        }

        // Boundary shapes (read without a layer structure).
        let mut bounding_shape: Option<PShape<'_>> = None;
        let mut outline_shapes: Vec<PShape<'_>> = Vec::new();
        for group in &self.order.boundary_groups {
            // Java adds the additional shapes of a boundary scope before its first shape.
            let seq = group[1..].iter().chain(group.first());
            for &i in seq {
                let Some(ps) = shapes::resolve(&s.boundaries[i], None) else {
                    continue;
                };
                if ps.is_path() {
                    outline_shapes.push(ps);
                    continue;
                }
                match ps.layer {
                    Some(PLayer::Pcb) => {
                        if bounding_shape.is_none() {
                            bounding_shape = Some(ps);
                        } else {
                            outline_shapes.push(ps);
                        }
                    }
                    Some(PLayer::Signal) => outline_shapes.push(ps),
                    _ => log::warn!("Structure.add_boundary_shape: unexpected layer at boundary"),
                }
            }
        }

        self.create_board(bounding_shape, outline_shapes)?;
        if self.flip_style_rotate_first_requested() {
            self.flip_style_rotate_first = true;
            self.board()?.components.set_flip_style_rotate_first(true);
        }

        // Keepouts, via keepouts, place keepouts (each list in file order).
        for kind in [
            KeepoutKind::Keepout,
            KeepoutKind::ViaKeepout,
            KeepoutKind::PlaceKeepout,
        ] {
            for (idx, (k, area)) in s.keepouts.iter().enumerate() {
                // wire keepouts are keepouts in Freerouting: same list, same order
                let list_kind = if *k == KeepoutKind::WireKeepout { KeepoutKind::Keepout } else { *k };
                if list_kind != kind {
                    continue;
                }
                let kind = *k;
                let n = valid_prefix(self.order.keepout_layers_before[idx]);
                let pls = &self.parser_layers[..n];
                let resolved: Vec<Option<PShape<'_>>> = area
                    .shapes
                    .iter()
                    .map(|sh| shapes::resolve(sh, Some(ParserLayers(pls))))
                    .collect();
                if resolved[0].is_none() {
                    // readAreaScope returned null; insertKeepout dereferences it.
                    return Err(npe(
                        "Structure.insertKeepout: keepout area could not be read",
                    ));
                }
                self.insert_keepout(
                    &resolved,
                    area.name.as_deref(),
                    area.clearance_class.as_deref(),
                    kind,
                )?;
            }
        }

        // Planes.
        for (idx, plane) in s.planes.iter().enumerate() {
            self.insert_plane(
                idx,
                plane,
                valid_prefix(self.order.plane_layers_before[idx]),
            )?;
        }
        self.insert_missing_power_planes()?;
        Ok(())
    }

    fn flip_style_rotate_first_requested(&self) -> bool {
        self.dsn.structure.flip_style_rotate_first
    }

    fn create_board(
        &mut self,
        bounding_shape: Option<PShape<'_>>,
        outline_shapes: Vec<PShape<'_>>,
    ) -> LResult<()> {
        let dsn = self.dsn;
        let layer_count = self.parser_layers.len();
        if layer_count == 0 {
            return Err(LoadError::ParseError(
                "Structure.create_board: layers missing in structure scope".into(),
            ));
        }
        let bounding_box: [f64; 4] = match &bounding_shape {
            Some(b) => shapes::bounding_box(b.kind)
                .ok_or_else(|| npe("Structure.createBoard: bounding box of polyline_path"))?,
            None => {
                if outline_shapes.is_empty() {
                    return Err(LoadError::OutlineMissing(
                        "Structure.create_board: outline missing".into(),
                    ));
                }
                let mut bb = shapes::bounding_box(outline_shapes[0].kind)
                    .ok_or_else(|| npe("Structure.createBoard: bounding box of polyline_path"))?;
                for o in &outline_shapes[1..] {
                    let b = shapes::bounding_box(o.kind).ok_or_else(|| {
                        npe("Structure.createBoard: bounding box of polyline_path")
                    })?;
                    bb = shapes::rect_union(&bb, &b);
                }
                bb
            }
        };
        // The fallback outline uses the (possibly synthesized) bounding rectangle.
        let bounding_kind = match &bounding_shape {
            Some(b) => b.kind.clone(),
            None => ShapeKind::Rect(bounding_box),
        };

        let board_layers: Vec<Layer> = self
            .parser_layers
            .iter()
            .map(|l| Layer::new(l.name.clone(), l.is_signal))
            .collect();
        let layer_structure = LayerStructure::new(board_layers);

        // Scale factor between DSN and board coordinates.
        let resolution = dsn.resolution;
        let mut scale_factor: i32 = resolution.max(1);
        let mut max_coor: f64 = 0.0;
        for v in bounding_box {
            max_coor = max_coor.max((v * resolution as f64).abs());
        }
        if max_coor == 0.0 {
            return Err(LoadError::OutlineMissing(
                "Structure.create_board: board bounding box is empty".into(),
            ));
        }
        while 5.0 * max_coor >= CRIT_INT as f64 {
            scale_factor /= 10;
            max_coor /= 10.0;
        }
        let ct = CoordinateTransform::new(scale_factor as f64, 0.0, 0.0);

        let bounds = match shapes::transform_to_board(&ShapeKind::Rect(bounding_box), &ct)? {
            Some(Shape::Tile(TileShape::IntBox(b))) => b,
            _ => unreachable!("rectangle transforms to an IntBox"),
        };
        let bounds = bounds.offset(1000.0);

        let mut board_outline: Vec<PolylineShape> = Vec::new();
        for o in &outline_shapes {
            let kind = match o.kind {
                ShapeKind::PolygonPath { width, coords } if *width != 0.0 => {
                    ShapeKind::PolygonPath {
                        width: 0.0,
                        coords: coords.clone(),
                    }
                }
                k => k.clone(),
            };
            let shape = shapes::transform_to_board(&kind, &ct)?
                .ok_or_else(|| npe("Structure.createBoard: outline shape is null"))?;
            let p = shape.to_polyline_shape().ok_or_else(|| {
                LoadError::JavaException(
                    "ClassCastException: outline shape is not a PolylineShape".into(),
                )
            })?;
            if p.dimension() > 0 {
                board_outline.push(p);
            }
        }
        if board_outline.is_empty() {
            let shape = shapes::transform_to_board(&bounding_kind, &ct)?
                .ok_or_else(|| npe("Structure.createBoard: bounding shape is null"))?;
            let p = shape.to_polyline_shape().ok_or_else(|| {
                LoadError::JavaException(
                    "ClassCastException: bounding shape is not a PolylineShape".into(),
                )
            })?;
            board_outline.push(p);
        }
        let holes = separate_holes(&mut board_outline);

        let clearance_matrix = ClearanceMatrix::get_default_instance(&layer_structure, 0);
        let mut rules = BoardRules::new(layer_structure.clone(), clearance_matrix);

        let p = &dsn.parser;
        let parser_info = SpecctraParserInfo {
            string_quote: p.string_quote.clone(),
            host_cad: p.host_cad.clone(),
            host_version: p.host_version.clone(),
            constants: Some(
                p.constants
                    .iter()
                    .map(|(a, b)| vec![a.clone(), b.clone()])
                    .collect(),
            ),
            write_resolution: p.write_resolution.as_ref().map(|(c, v)| WriteResolution {
                char_name: c.clone(),
                positive_int: *v,
            }),
            dsn_file_generated_by_host: p.generated_by_host,
        };
        let communication = Communication::new(
            map_unit(dsn.unit),
            dsn.resolution,
            Some(parser_info),
            ct,
            ItemIdGenerator::new(),
        );
        if communication.host_is_old_kicad() {
            self.warn(
                "Structure.create_board: The DSN file was exported from an old KiCad version that \
                 has known compatibility issues.",
            );
        }

        self.update_board_rules(&mut rules, &ct);
        rules.set_trace_angle_restriction(self.snap_angle);

        // MinimalBoardManager.createBoard: outline clearance class.
        let outline_cl = match &dsn.structure.outline_clearance_class {
            Some(name) => rules.clearance_matrix.get_no(name).max(0),
            None => {
                let d = rules.get_default_net_class();
                rules.net_classes[d]
                    .default_item_clearance_classes
                    .get(ItemClass::Area)
            }
        };

        let padstacks = fr_engine::library::Padstacks::new(&layer_structure);
        let mut board = Board {
            layer_structure,
            rules,
            library: BoardLibrary::new(padstacks, fr_engine::library::Packages::new()),
            library_read: false,
            null_package_keepouts: HashSet::new(),
            components: Components::new(),
            logical_part_assignments: Vec::new(),
            communication,
            transform: ct,
            bounding_box: bounds,
            outline_shapes: board_outline.clone(),
            outline_clearance_class: outline_cl,
            requests: Vec::new(),
        };
        board.push(InsertRequest::Outline {
            shapes: board_outline,
            clearance_class: outline_cl,
        });
        // The holes of the outline become keepouts on all layers.
        for hole in holes {
            for layer in 0..layer_count as LayerNo {
                board.push(InsertRequest::Obstacle(AreaRequest {
                    area: Area::Shape(Shape::from(hole.clone())),
                    layer,
                    translation: fr_geom::Vector::from(fr_geom::IntVector::ZERO),
                    rotation_in_degree: 0.0,
                    side_changed: false,
                    clearance_class: 0,
                    component_id: 0,
                    name: None,
                    fixed: FixedState::SystemFixed,
                }));
            }
        }
        self.board = Some(board);
        Ok(())
    }

    /// Java `Structure.updateBoardRules`.
    fn update_board_rules(&mut self, rules: &mut BoardRules, ct: &CoordinateTransform) {
        let s = &self.dsn.structure;
        let quote = self.dsn.parser.string_quote.clone();
        let mut smd_to_turn_gap_found = false;
        for r in &s.rules {
            if let Rule::Clearance { value, class_pairs } = r {
                if set_clearance_rule(*value, class_pairs, -1, ct, rules, &quote) {
                    smd_to_turn_gap_found = true;
                }
            }
        }
        for r in &s.rules {
            if let Rule::Width(w) = r {
                let hw = round_i32(ct.dsn_to_board(*w) / 2.0);
                rules.set_default_trace_half_widths(hw);
            }
        }
        // Layer dependent rules, in layer scope order (also of layers with an unknown type).
        let pls = ParserLayers(&self.parser_layers);
        for layer in &s.layers {
            let layer_no = pls.get_no(&layer.name);
            if layer_no < 0 {
                continue;
            }
            for r in &layer.rules {
                match r {
                    Rule::Width(w) => {
                        let hw = round_i32(ct.dsn_to_board(*w) / 2.0);
                        rules.set_default_trace_half_width(layer_no, hw);
                    }
                    Rule::Clearance { value, class_pairs } => {
                        set_clearance_rule(*value, class_pairs, layer_no, ct, rules, &quote);
                    }
                }
            }
        }
        if !smd_to_turn_gap_found {
            let v = rules.get_min_trace_half_width() as f64;
            rules.set_pin_edge_to_turn_dist(v);
        }
    }

    /// Java `Structure.insertKeepout` (both overloads).
    fn insert_keepout(
        &mut self,
        area_shapes: &[Option<PShape<'_>>],
        name: Option<&str>,
        clearance_class: Option<&str>,
        kind: KeepoutKind,
    ) -> LResult<()> {
        let ct = self.board()?.transform;
        let area = shapes::transform_area(area_shapes, &ct, false)?
            .ok_or_else(|| npe("Structure.insertKeepout: keepout area is null"))?;
        if area.dimension() < 2 {
            self.warn(format!(
                "Keepout zone '{}' was skipped because its geometry is degenerate",
                name.unwrap_or("null")
            ));
            return Ok(());
        }
        let layer = area_shapes[0].as_ref().and_then(|s| s.layer.clone());
        let layers: Vec<LayerNo> = match layer {
            Some(PLayer::Signal) => self
                .parser_layers
                .iter()
                .filter(|l| l.is_signal)
                .map(|l| l.no)
                .collect(),
            Some(PLayer::Named { no, .. }) if no >= 0 => vec![no],
            Some(_) => {
                return Err(LoadError::ParseError(
                    "Structure.insert_keepout: unknown layer name".into(),
                ))
            }
            None => return Err(npe("Structure.insertKeepout: keepout layer is null")),
        };
        let board = self.board()?;
        for layer in layers {
            let cl = match clearance_class {
                None => {
                    let d = board.rules.get_default_net_class();
                    board.rules.net_classes[d]
                        .default_item_clearance_classes
                        .get(ItemClass::Area)
                }
                Some(c) => {
                    let no = board.rules.clearance_matrix.get_no(c);
                    if no < 0 {
                        log::warn!("Keepout.insert_keepout: clearance class not found at '{c}'");
                        BoardRules::clearance_class_none()
                    } else {
                        no
                    }
                }
            };
            let req = AreaRequest {
                area: area.clone(),
                layer,
                translation: fr_geom::Vector::from(fr_geom::IntVector::ZERO),
                rotation_in_degree: 0.0,
                side_changed: false,
                clearance_class: cl,
                component_id: 0,
                name: None,
                fixed: FixedState::SystemFixed,
            };
            board.push(match kind {
                KeepoutKind::ViaKeepout => InsertRequest::ViaObstacle(req),
                KeepoutKind::PlaceKeepout => InsertRequest::ComponentObstacle(req),
                KeepoutKind::Keepout => InsertRequest::Obstacle(req),
                KeepoutKind::WireKeepout => InsertRequest::WireObstacle(req),
            });
        }
        Ok(())
    }

    /// Registers a net of the parser netlist and the board if it is not known yet
    /// (`netlist.addNet` + `rules.nets.add(name, subnet, true)`).
    fn add_plane_net(&mut self, name: &str) -> LResult<()> {
        let id = NetId {
            name: name.to_string(),
            subnet: 1,
        };
        if !self.netlist.contains(&id) && self.netlist.add_net(id) {
            self.board()?.rules.add_net(name, 1, true);
        }
        Ok(())
    }

    fn insert_plane(
        &mut self,
        _idx: usize,
        plane: &fr_dsn::model::Plane,
        n_layers: usize,
    ) -> LResult<()> {
        self.add_plane_net(&plane.net)?;
        let board = self.board()?;
        let Some(net) = board.rules.nets.get_by_name(&plane.net, 1) else {
            log::warn!("Plane.read_scope: net not found");
            return Ok(());
        };
        let (net_no, net_class) = (net.net_number, net.get_net_class());
        let ct = board.transform;
        let area = plane
            .area
            .as_ref()
            .ok_or_else(|| npe("Structure: plane area is null"))?;
        let pls = &self.parser_layers[..n_layers];
        let resolved: Vec<Option<PShape<'_>>> = area
            .shapes
            .iter()
            .map(|sh| shapes::resolve(sh, Some(ParserLayers(pls))))
            .collect();
        if resolved[0].is_none() {
            return Err(npe("Structure: plane area could not be read"));
        }
        let plane_area = shapes::transform_area(&resolved, &ct, false)?;
        let layer = resolved[0]
            .as_ref()
            .and_then(|s| s.layer.clone())
            .ok_or_else(|| npe("Structure: plane layer is null"))?;
        let layer_no = layer.no();
        if layer_no < 0 {
            return Err(LoadError::ParseError(
                "Plane.read_scope: unexpected layer name".into(),
            ));
        }
        let board = self.board()?;
        let cl = match &area.clearance_class {
            Some(c) => {
                let no = board.rules.clearance_matrix.get_no(c);
                if no < 0 {
                    log::warn!("Structure.read_scope: clearance class not found");
                    BoardRules::clearance_class_none()
                } else {
                    no
                }
            }
            None => board.rules.net_classes[net_class]
                .default_item_clearance_classes
                .get(ItemClass::Area),
        };
        match plane_area {
            Some(a) => board.push(InsertRequest::ConductionArea {
                area: a,
                layer: layer_no,
                nets: vec![net_no],
                clearance_class: cl,
                is_obstacle: false,
                fixed: FixedState::SystemFixed,
            }),
            None => log::warn!("BasicBoard.insert_conduction_area: area is null"),
        }
        Ok(())
    }

    /// Java `Structure.insertMissingPowerPlanes`.
    fn insert_missing_power_planes(&mut self) -> LResult<()> {
        let layers = self.parser_layers.clone();
        for layer in layers.iter().filter(|l| !l.is_signal) {
            let found = self.board()?.conduction_area_layers().contains(&layer.no);
            if found || layer.net_names.is_empty() {
                continue;
            }
            let net_name = layer.net_names[0].clone();
            self.add_plane_net(&net_name)?;
            let board = self.board()?;
            let Some(net) = board.rules.nets.get_by_name(&net_name, 1) else {
                log::warn!("Structure.insert_missing_power_planes: net not found");
                continue;
            };
            let net_no = net.net_number;
            let bb = board.bounding_box;
            board.push(InsertRequest::ConductionArea {
                area: Area::from(TileShape::IntBox(bb)),
                layer: layer.no,
                nets: vec![net_no],
                clearance_class: BoardRules::clearance_class_none(),
                is_obstacle: false,
                fixed: FixedState::SystemFixed,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fr_engine::structure::Layer;

    fn rules() -> BoardRules {
        let ls = LayerStructure::new(vec![Layer::new("F", true), Layer::new("B", true)]);
        let cm = ClearanceMatrix::get_default_instance(&ls, 0);
        BoardRules::new(ls, cm)
    }

    #[test]
    fn clearance_rules() {
        let ct = CoordinateTransform::new(10.0, 0.0, 0.0);
        let mut r = rules();
        let q = "\"";
        assert!(!set_clearance_rule(20.0, &[], -1, &ct, &mut r, q));
        assert_eq!(r.clearance_matrix.get_value(1, 1, 0, false), 200);
        // default_smd creates "smd" and sets the default net class SMD item class.
        set_clearance_rule(5.0, &["default_smd".into()], -1, &ct, &mut r, q);
        let smd = r.clearance_matrix.get_no("smd");
        assert_eq!(smd, 2);
        assert_eq!(r.clearance_matrix.get_value(1, smd, 1, false), 50);
        let d = r.get_default_net_class();
        assert_eq!(
            r.net_classes[d]
                .default_item_clearance_classes
                .get(ItemClass::Smd),
            smd
        );
        // wire pair: creates via/smd/pin/area; "wire" maps to class 1
        set_clearance_rule(7.0, &["wire_via".into()], 1, &ct, &mut r, q);
        let via = r.clearance_matrix.get_no("via");
        assert_eq!(r.clearance_matrix.get_value(1, via, 1, false), 70);
        assert_eq!(r.clearance_matrix.get_value(1, via, 0, false), 200);
        assert_eq!(r.clearance_matrix.get_class_count(), 6);
        // smd_via_same_net: more than one underline -> "smd", "via_same_net" pair (split limit 2)
        set_clearance_rule(3.0, &["smd_to_turn_gap".into()], -1, &ct, &mut r, q);
        assert_eq!(r.get_pin_edge_to_turn_dist(), 30.0);
        // two class-pair strings: quoted class names
        set_clearance_rule(4.0, &["my cls".into(), "_pin".into()], -1, &ct, &mut r, q);
        let my = r.clearance_matrix.get_no("my cls");
        let pin = r.clearance_matrix.get_no("pin");
        assert!(my > 0);
        assert_eq!(r.clearance_matrix.get_value(my, pin, 0, false), 40);
    }

    #[test]
    fn holes() {
        use fr_geom::IntBox;
        let outer = PolylineShape::Tile(TileShape::IntBox(IntBox::new(0, 0, 100, 100)));
        let inner = PolylineShape::Tile(TileShape::IntBox(IntBox::new(10, 10, 20, 20)));
        let other = PolylineShape::Tile(TileShape::IntBox(IntBox::new(200, 0, 300, 100)));
        let mut v = vec![inner.clone(), outer.clone(), other];
        let h = separate_holes(&mut v);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].bounding_box(), IntBox::new(10, 10, 20, 20));
        assert_eq!(v.len(), 2);
    }
}
