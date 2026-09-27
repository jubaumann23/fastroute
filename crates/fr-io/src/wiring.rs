//! The wiring scope (Java `io.specctra.parser.Wiring.readScope`, `readWireScope`,
//! `readViaScope`): pre-routed traces, conduction areas and vias.

use fr_dsn::model::{NetId as DsnNetId, ShapeKind, Wire, WiringVia};
use fr_engine::ids::{LayerNo, NetNo};
use fr_engine::rules::{BoardRules, ItemClass, NetClassId};
use fr_geom::{Line, Point, Polygon, Polyline};

use crate::error::{npe, LResult, LoadError};
use crate::library::clean_padstack_name;
use crate::loader::Loader;
use crate::order::WiringItem;
use crate::requests::InsertRequest;
use crate::shapes::{self, box_contains, PShape};
use crate::structure::{map_fixed, round_i32};

/// Java `Wiring.getSubnets`: the board nets of a wire/via net reference.
fn get_subnets(net: Option<&DsnNetId>, rules: &BoardRules) -> Vec<(NetNo, NetClassId)> {
    let Some(net) = net else { return Vec::new() };
    if net.subnet > 0 {
        rules
            .nets
            .get_by_name(&net.name, net.subnet)
            .map(|n| vec![(n.net_number, n.get_net_class())])
            .unwrap_or_default()
    } else {
        rules
            .nets
            .get_all_by_name(&net.name)
            .iter()
            .map(|n| (n.net_number, n.get_net_class()))
            .collect()
    }
}

/// A via inserted by the wiring scope, for the duplicate prediction.
struct PlacedVia {
    center: fr_geom::IntPoint,
    first: LayerNo,
    last: LayerNo,
    nets: Vec<NetNo>,
}

/// Java `Item.netsEqual(int[])`: same length and every net contained (`containsNet` is
/// false for net numbers <= 0).
fn nets_equal(a: &[NetNo], b: &[NetNo]) -> bool {
    a.len() == b.len() && b.iter().all(|n| *n > 0 && a.contains(n))
}

impl Loader<'_> {
    pub(crate) fn read_wiring(&mut self) -> LResult<()> {
        if self.board.is_none() {
            return Err(npe("Wiring.readScope: board is null"));
        }
        let dsn = self.dsn;
        let mut placed_vias: Vec<PlacedVia> = Vec::new();
        for item in self.order.wiring.clone() {
            match item {
                WiringItem::Wire(i) => self.read_wire(&dsn.wiring.wires[i])?,
                WiringItem::Via(i) => self.read_via(&dsn.wiring.vias[i], &mut placed_vias)?,
            }
        }
        // Java then calls board.normalizeAllTraces() (board step).
        Ok(())
    }

    /// Java `Wiring.readWireScope` (after parsing).
    fn read_wire(&mut self, wire: &Wire) -> LResult<()> {
        let pls = self.parser_layers.clone();
        let Some(shape) = shapes::resolve(&wire.shape, Some(crate::shapes::ParserLayers(&pls)))
        else {
            self.warn(format!(
                "Wiring: wire has no shape at line {}",
                wire.shape.line
            ));
            return Ok(());
        };
        let is_path = shape.is_path();
        let board = self.board()?;
        let found = get_subnets(wire.net.as_ref(), &board.rules);
        let nets: Vec<NetNo> = found.iter().map(|(n, _)| *n).collect();
        let net_class = match found.last() {
            Some((_, c)) => *c,
            None => board.rules.get_default_net_class(),
        };
        let mut cl = match &wire.clearance_class {
            Some(c) => board.rules.clearance_matrix.get_no(c),
            None => -1,
        };
        let layer = shape
            .layer
            .as_ref()
            .ok_or_else(|| npe("Wiring.readWireScope: path layer is null"))?;
        let layer_no = layer.no();
        let (half_width, width) = match shape.kind {
            ShapeKind::PolygonPath { width, .. } | ShapeKind::PolylinePath { width, .. } => {
                (round_i32(board.transform.dsn_to_board(width / 2.0)), *width)
            }
            _ => (0, 0.0),
        };
        let _ = width;
        if layer_no < 0 || layer_no >= board.layer_count() {
            let msg = format!("Wiring: wire ignored — unknown layer '{}'", layer.name());
            self.warn(msg);
            return Ok(());
        }
        let fixed = map_fixed(wire.fixed);
        let ct = board.transform;
        let bounding_box = board.bounding_box;
        let classes = &board.rules.net_classes[net_class].default_item_clearance_classes;
        if !is_path {
            if cl < 0 {
                cl = classes.get(ItemClass::Area);
            }
            let mut area_shapes: Vec<Option<PShape<'_>>> = vec![Some(shape.clone())];
            area_shapes.extend(
                wire.holes
                    .iter()
                    .map(|h| shapes::resolve(h, Some(crate::shapes::ParserLayers(&pls)))),
            );
            let area = shapes::transform_area(&area_shapes, &ct, false)?;
            let board = self.board()?;
            match area {
                Some(a) => board.push(InsertRequest::ConductionArea {
                    area: a,
                    layer: layer_no,
                    nets: nets.clone(),
                    clearance_class: cl,
                    is_obstacle: false,
                    fixed,
                }),
                None => log::warn!("BasicBoard.insert_conduction_area: area is null"),
            }
            return Ok(());
        }
        if cl < 0 {
            cl = classes.get(ItemClass::Trace);
        }
        let polyline = match shape.kind {
            ShapeKind::PolygonPath { coords, .. } => {
                let mut corners = Vec::with_capacity(coords.len() / 2);
                for i in 0..coords.len() / 2 {
                    let c = ct.dsn_to_board_point(&coords[2 * i..2 * i + 2]);
                    if !box_contains(&bounding_box, &c) {
                        let msg = format!(
                            "Wiring: wire corner ({},{}) is outside board bounds",
                            coords[2 * i] as i32,
                            coords[2 * i + 1] as i32
                        );
                        self.warn(msg);
                        return Ok(());
                    }
                    corners.push(Point::Int(c.round()));
                }
                let polygon = Polygon::new(&corners);
                let pc = polygon.corner_array();
                let distinct = pc.iter().skip(1).any(|p| *p != pc[0]);
                if pc.len() < 2 || !distinct {
                    let msg = format!(
                        "Wiring: degenerate wire trace skipped (all {} corners are identical — \
                         zero-length trace) on layer '{}'.",
                        pc.len(),
                        layer.name()
                    );
                    self.warn(msg);
                    return Ok(());
                }
                Polyline::from_polygon(&polygon)
            }
            ShapeKind::PolylinePath { coords, .. } => {
                let lines: Vec<Line> = (0..coords.len() / 4)
                    .map(|i| {
                        let a = ct.dsn_to_board_point(&coords[4 * i..4 * i + 2]).round();
                        let b = ct.dsn_to_board_point(&coords[4 * i + 2..4 * i + 4]).round();
                        Line::new(Point::Int(a), Point::Int(b))
                    })
                    .collect();
                Polyline::from_lines(lines)
            }
            _ => unreachable!(),
        };
        let try_correct_net = nets.is_empty();
        self.board()?.push(InsertRequest::Trace {
            polyline,
            layer: layer_no,
            half_width,
            nets,
            clearance_class: cl,
            fixed,
            try_correct_net,
        });
        Ok(())
    }

    /// Java `Wiring.readViaScope` (after parsing).
    fn read_via(&mut self, via: &WiringVia, placed: &mut Vec<PlacedVia>) -> LResult<()> {
        let via_at_smd = self.via_at_smd;
        let board = self.board()?;
        let cleaned = clean_padstack_name(&via.padstack);
        let Some(padstack) = board.library.padstacks.get_by_name(&cleaned) else {
            return Err(LoadError::ParseError(format!(
                "Wiring: via padstack '{}' not found",
                via.padstack
            )));
        };
        let (pid, from, to, attach) = (
            padstack.id,
            padstack.from_layer(),
            padstack.to_layer(),
            padstack.attach_allowed,
        );
        let found = get_subnets(via.net.as_ref(), &board.rules);
        if via.net.is_some() && found.is_empty() {
            let msg = format!(
                "Wiring: via net '{}' not found",
                via.net.as_ref().map(|n| n.name.as_str()).unwrap_or("")
            );
            self.warn(msg);
        }
        let board = self.board()?;
        // Java bug kept: every net is written to index 0 (the index is never incremented).
        let mut nets: Vec<NetNo> = vec![0; found.len()];
        let mut net_class = board.rules.get_default_net_class();
        for (n, c) in &found {
            nets[0] = *n;
            net_class = *c;
        }
        let mut cl = match &via.clearance_class {
            Some(c) => board.rules.clearance_matrix.get_no(c),
            None => -1,
        };
        if cl < 0 {
            cl = board.rules.net_classes[net_class]
                .default_item_clearance_classes
                .get(ItemClass::Via);
        }
        let center = board.transform.dsn_to_board_point(&[via.x, via.y]).round();
        let predicted_duplicate = placed.iter().any(|p| {
            p.center == center && p.first == from && p.last == to && nets_equal(&p.nets, &nets)
        });
        if predicted_duplicate {
            let msg = format!(
                "Wiring: duplicate via skipped at ({}, {})",
                center.x, center.y
            );
            self.warn(msg);
        } else {
            placed.push(PlacedVia {
                center,
                first: from,
                last: to,
                nets: nets.clone(),
            });
        }
        let board = self.board()?;
        board.push(InsertRequest::Via {
            padstack: pid,
            center,
            nets,
            clearance_class: cl,
            fixed: map_fixed(via.fixed),
            attach_allowed: via_at_smd && attach,
            predicted_duplicate,
        });
        Ok(())
    }
}
