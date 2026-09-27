//! The DSN -> board-construction driver (Java `DsnReader.readBoard` with the
//! `ReadScopeParameter.MinimalBoardManager` shim).

use std::collections::HashSet;

use fr_dsn::model::{ComponentPlacement, Dsn};
use fr_engine::ids::{AngleRestriction, ClearanceClassNo, ComponentNo, LayerNo};
use fr_engine::library::{BoardLibrary, LogicalPartNo, PackageNo};
use fr_engine::rules::BoardRules;
use fr_engine::structure::{Communication, Components, CoordinateTransform, LayerStructure};
use fr_geom::{IntBox, PolylineShape};
use fr_settings::settings::RouterSettings;

use crate::error::{LResult, LoadError};
use crate::netlist::NetList;
use crate::order::{SourceOrder, TopScope};
use crate::part_library::{LogicalPartDef, LogicalPartMapping};
use crate::plane::PlaneAdjustment;
use crate::requests::InsertRequest;
use crate::shapes::{ParserLayer, ParserLayers};

/// Which package keepout list a keepout belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeepoutKind {
    Keepout,
    ViaKeepout,
    PlaceKeepout,
}

/// Everything the board builder needs to reproduce the Java board after `DsnReader.readBoard`.
#[derive(Clone, Debug)]
pub struct LoadedDesign {
    /// The board layer structure (layers with an unknown `type` are dropped).
    pub layer_structure: LayerStructure,
    /// The parser layer structure (with `use_net` names).
    pub parser_layers: Vec<ParserLayer>,
    /// Board rules after the whole file was read (clearance matrix, nets, net classes,
    /// via infos and rules). `adjustPlaneAutorouteSettings` is *not* applied (see
    /// [`LoadedDesign::plane_adjustment`]).
    pub rules: BoardRules,
    /// Padstacks, packages, via padstacks and logical parts.
    pub library: BoardLibrary,
    /// Package keepouts whose Java area is `null` (stored with an empty placeholder area in
    /// `library`, since `fr_engine::library::Keepout::area` is not optional):
    /// `(package id, kind, index)`.
    pub null_package_keepouts: HashSet<(PackageNo, KeepoutKind, usize)>,
    /// The components in Java order (`flip_style_rotate_first` set).
    pub components: Components,
    /// `Component.setLogicalPart` calls of `Network.insertLogicalParts`, in Java order
    /// (fr-engine's `Components` has no mutable accessor; the board step applies them).
    pub logical_part_assignments: Vec<(ComponentNo, Option<LogicalPartNo>)>,
    pub communication: Communication,
    /// `BasicBoard.boundingBox` (outline bounding box plus 1000 units on each side).
    pub bounding_box: IntBox,
    pub outline_shapes: Vec<PolylineShape>,
    pub outline_clearance_class: ClearanceClassNo,
    /// All `BasicBoard.insert*` calls in Java order; the first is always the outline.
    pub requests: Vec<InsertRequest>,
    /// Non-fatal problems (the typed model's parse warnings first).
    pub warnings: Vec<String>,
    /// The `(autoroute_settings ...)` scope Java reads in the structure scope, if any.
    pub autoroute_settings: Option<RouterSettings>,
    /// `DsnFile.adjustPlaneAutorouteSettings`, computed when `autoroute_settings` is `None`
    /// (Java only runs it then). The board step applies it after `normalizeAllTraces`.
    pub plane_adjustment: Option<PlaneAdjustment>,
    pub via_at_smd_allowed: bool,
    pub snap_angle: AngleRestriction,
    pub flip_style_rotate_first: bool,
}

/// The board as far as the loader tracks it.
pub(crate) struct Board {
    pub layer_structure: LayerStructure,
    pub rules: BoardRules,
    pub library: BoardLibrary,
    pub library_read: bool,
    pub null_package_keepouts: HashSet<(PackageNo, KeepoutKind, usize)>,
    pub components: Components,
    pub logical_part_assignments: Vec<(ComponentNo, Option<LogicalPartNo>)>,
    pub communication: Communication,
    pub transform: CoordinateTransform,
    pub bounding_box: IntBox,
    pub outline_shapes: Vec<PolylineShape>,
    pub outline_clearance_class: ClearanceClassNo,
    pub requests: Vec<InsertRequest>,
}

impl Board {
    pub fn layer_count(&self) -> LayerNo {
        self.layer_structure.layer_count()
    }

    pub fn push(&mut self, r: InsertRequest) {
        self.requests.push(r);
    }

    /// Board nets of `ConductionArea` requests so far: `(layer)` list, Java
    /// `board.getConductionAreas()` (only the layers are needed by the loader).
    pub fn conduction_area_layers(&self) -> Vec<LayerNo> {
        self.requests
            .iter()
            .filter_map(|r| match r {
                InsertRequest::ConductionArea { layer, .. } => Some(*layer),
                _ => None,
            })
            .collect()
    }
}

pub(crate) struct Loader<'a> {
    pub dsn: &'a Dsn,
    pub order: &'a SourceOrder,
    pub warnings: Vec<String>,
    /// The parser layer structure (all layers with a known type).
    pub parser_layers: Vec<ParserLayer>,
    pub netlist: NetList,
    pub via_at_smd: bool,
    pub snap_angle: AngleRestriction,
    pub flip_style_rotate_first: bool,
    pub placements: Vec<&'a ComponentPlacement>,
    pub logical_parts: Vec<LogicalPartDef>,
    pub logical_part_mappings: Vec<LogicalPartMapping>,
    pub autoroute_settings: Option<RouterSettings>,
    pub board: Option<Board>,
}

impl<'a> Loader<'a> {
    pub fn layers(&self) -> ParserLayers<'_> {
        ParserLayers(&self.parser_layers)
    }

    pub fn warn(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        log::warn!("{msg}");
        self.warnings.push(msg);
    }

    pub fn board(&mut self) -> LResult<&mut Board> {
        self.board
            .as_mut()
            .ok_or_else(|| crate::error::npe("board not created (structure scope missing)"))
    }

    fn run(&mut self) -> LResult<()> {
        for scope in self.order.top.clone() {
            match scope {
                TopScope::Structure => self.read_structure()?,
                TopScope::PlaceControl => {}
                TopScope::Library => self.read_library()?,
                TopScope::Placement => {
                    let dsn = self.dsn;
                    self.placements.extend(dsn.placement.iter());
                    if self.order.placement_flip_style_rotate_first {
                        // PlaceControl.readScope: board.components.setFlipStyleRotateFirst(true)
                        self.flip_style_rotate_first = true;
                        self.board()?.components.set_flip_style_rotate_first(true);
                    }
                }
                TopScope::PartLibrary => self.read_part_library()?,
                TopScope::Network => self.read_network()?,
                TopScope::Wiring => self.read_wiring()?,
            }
        }
        if self.board.is_none() {
            return Err(LoadError::ParseError(
                "no structure scope: the board was never created".into(),
            ));
        }
        Ok(())
    }

    fn finish(mut self) -> LoadedDesign {
        let plane_adjustment = if self.autoroute_settings.is_none() {
            let b = self.board.as_ref().expect("board");
            crate::plane::compute(&b.layer_structure, &b.outline_shapes, &b.requests, &b.rules)
        } else {
            None
        };
        let b = self.board.take().expect("board");
        LoadedDesign {
            layer_structure: b.layer_structure,
            parser_layers: self.parser_layers,
            rules: b.rules,
            library: b.library,
            null_package_keepouts: b.null_package_keepouts,
            components: b.components,
            logical_part_assignments: b.logical_part_assignments,
            communication: b.communication,
            bounding_box: b.bounding_box,
            outline_shapes: b.outline_shapes,
            outline_clearance_class: b.outline_clearance_class,
            requests: b.requests,
            warnings: self.warnings,
            autoroute_settings: self.autoroute_settings,
            plane_adjustment,
            via_at_smd_allowed: self.via_at_smd,
            snap_angle: self.snap_angle,
            flip_style_rotate_first: self.flip_style_rotate_first,
        }
    }
}

/// Loads a parsed DSN assuming the usual scope order ([`SourceOrder::canonical`]).
pub fn load(dsn: &Dsn) -> Result<LoadedDesign, LoadError> {
    load_with_order(dsn, &SourceOrder::canonical(dsn))
}

/// Parses DSN bytes and loads them with the exact scope order of the file.
pub fn load_bytes(src: &[u8]) -> Result<LoadedDesign, LoadError> {
    let top = fr_dsn::sexpr::parse(src).map_err(|e| LoadError::ParseError(e.to_string()))?;
    let pcb = top
        .first()
        .and_then(|s| s.as_list())
        .filter(|l| l.is("pcb"))
        .ok_or_else(|| LoadError::ParseError("Not a Specctra DSN file".into()))?;
    let dsn = Dsn::from_pcb(pcb).map_err(|e| LoadError::ParseError(e.to_string()))?;
    let order = SourceOrder::from_pcb(pcb, &dsn);
    load_with_order(&dsn, &order)
}

/// Loads a parsed DSN with explicit scope order information.
pub fn load_with_order(dsn: &Dsn, order: &SourceOrder) -> Result<LoadedDesign, LoadError> {
    let mut loader = Loader {
        dsn,
        order,
        warnings: dsn.warnings.clone(),
        parser_layers: Vec::new(),
        netlist: NetList::default(),
        via_at_smd: false,
        snap_angle: AngleRestriction::FortyfiveDegree,
        flip_style_rotate_first: false,
        placements: Vec::new(),
        logical_parts: Vec::new(),
        logical_part_mappings: Vec::new(),
        autoroute_settings: None,
        board: None,
    };
    // fr-geom panics where Java throws a runtime exception (degenerate geometry);
    // those escape Java's readBoard as well.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        loader.run().map(|_| loader)
    }));
    match result {
        Ok(Ok(loader)) => Ok(loader.finish()),
        Ok(Err(e)) => Err(e),
        Err(panic) => {
            let msg = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "panic".into());
            Err(LoadError::JavaException(format!(
                "runtime exception: {msg}"
            )))
        }
    }
}
