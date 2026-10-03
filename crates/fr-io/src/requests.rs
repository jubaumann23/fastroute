//! Item insertion requests: the arguments of the Java `BasicBoard.insert*` calls made while a
//! DSN file is read, in Java call order.
//!
//! Every Java item takes the next id from the board's `ItemIdGenerator` in its constructor,
//! so replaying the requests in order on a fresh board reproduces the Java item ids, provided
//! the board step follows the rules documented on each variant (some calls construct an
//! item without inserting it, some return early without constructing one).

use fr_engine::ids::{ClearanceClassNo, ComponentNo, FixedState, LayerNo, NetNo, PadstackNo};
use fr_geom::{Area, IntPoint, Polyline, PolylineShape, Vector};

/// Arguments of `insertObstacle` / `insertViaObstacle` / `insertComponentObstacle`.
///
/// Board-level keepouts use the 4-argument Java overload, which is the 9-argument one with
/// `translation = Vector.ZERO`, `rotation = 0`, `side_changed = false`, `component_id = 0`
/// and `name = null`.
#[derive(Clone, Debug)]
pub struct AreaRequest {
    /// The relative area (absolute for board-level keepouts).
    pub area: Area,
    pub layer: LayerNo,
    pub translation: Vector,
    pub rotation_in_degree: f64,
    pub side_changed: bool,
    pub clearance_class: ClearanceClassNo,
    pub component_id: ComponentNo,
    pub name: Option<String>,
    pub fixed: FixedState,
}

/// One Java `BasicBoard.insert*` call.
#[derive(Clone, Debug)]
pub enum InsertRequest {
    /// `new RoutingBoard(...)` -> `BasicBoard.insertOutline(outlineShapes, clClass)`:
    /// always the first item (id 1).
    Outline {
        shapes: Vec<PolylineShape>,
        clearance_class: ClearanceClassNo,
    },
    /// `insertObstacle`: always constructs and inserts an `ObstacleArea` (the loader never
    /// emits a request for a `null` area, for which Java returns before constructing).
    Obstacle(AreaRequest),
    /// `insertViaObstacle` -> `ViaObstacleArea`.
    ViaObstacle(AreaRequest),
    /// fastroute: a board `wire_keepout` (blocks traces only; Freerouting: an `ObstacleArea`).
    WireObstacle(AreaRequest),
    /// `insertComponentObstacle` -> `ComponentObstacleArea`.
    ComponentObstacle(AreaRequest),
    /// `insertComponentOutline`. Java returns before constructing (no id) if
    /// `!area.isBounded()`; the loader already drops `null` areas but keeps unbounded ones
    /// so the board step applies the check itself ([`InsertRequest::consumes_id`] predicts it).
    ComponentOutline {
        area: Area,
        is_front: bool,
        translation: Vector,
        rotation_in_degree: f64,
        component_id: ComponentNo,
        is_courtyard: bool,
        is_fabrication: bool,
        is_closed: bool,
        fixed: FixedState,
    },
    /// `insertConductionArea(area, layer, nets, clClass, isObstacle, fixed)`.
    ConductionArea {
        area: Area,
        layer: LayerNo,
        nets: Vec<NetNo>,
        clearance_class: ClearanceClassNo,
        is_obstacle: bool,
        fixed: FixedState,
    },
    /// `insertPin(componentId, pinIndex, nets, clClass, fixed)`.
    Pin {
        component_id: ComponentNo,
        pin_index: i32,
        nets: Vec<NetNo>,
        clearance_class: ClearanceClassNo,
        fixed: FixedState,
    },
    /// `insertTraceWithoutCleaning(polyline, layer, halfWidth, nets, clClass, fixed)`.
    /// Java returns `null` without constructing if `polyline.cornerCount() < 2`; it constructs
    /// the trace (consuming an id) but does not insert it if the first and last corner are
    /// equal and `fixed < USER_FIXED`.
    ///
    /// `try_correct_net`: the trace was inserted without nets, so Java calls
    /// `Wiring.tryCorrectNet` right after the insertion (board step).
    Trace {
        polyline: Polyline,
        layer: LayerNo,
        half_width: i32,
        nets: Vec<NetNo>,
        clearance_class: ClearanceClassNo,
        fixed: FixedState,
        try_correct_net: bool,
    },
    /// `insertVia(padstack, center, nets, clClass, fixed, attachAllowed)`, preceded by the
    /// Java duplicate check `Wiring.viaExists` (board step: skip the call, and do not consume
    /// an id, if an equal via exists). `predicted_duplicate` is the loader's pure prediction
    /// of that check (same center, same first/last layer, equal nets as an earlier via of the
    /// wiring scope); the board check is authoritative.
    Via {
        padstack: PadstackNo,
        center: IntPoint,
        nets: Vec<NetNo>,
        clearance_class: ClearanceClassNo,
        fixed: FixedState,
        attach_allowed: bool,
        predicted_duplicate: bool,
    },
}

impl InsertRequest {
    /// Short Java class name of the item this request creates.
    pub fn kind_name(&self) -> &'static str {
        match self {
            InsertRequest::Outline { .. } => "BoardOutline",
            InsertRequest::Obstacle(_) | InsertRequest::WireObstacle(_) => "ObstacleArea",
            InsertRequest::ViaObstacle(_) => "ViaObstacleArea",
            InsertRequest::ComponentObstacle(_) => "ComponentObstacleArea",
            InsertRequest::ComponentOutline { .. } => "ComponentOutline",
            InsertRequest::ConductionArea { .. } => "ConductionArea",
            InsertRequest::Pin { .. } => "Pin",
            InsertRequest::Trace { .. } => "PolylineTrace",
            InsertRequest::Via { .. } => "Via",
        }
    }

    /// Predicts whether Java constructs an item (and so consumes an id) for this call.
    pub fn consumes_id(&self) -> bool {
        match self {
            InsertRequest::ComponentOutline { area, .. } => area.is_bounded(),
            InsertRequest::Trace { polyline, .. } => polyline.corner_count() >= 2,
            InsertRequest::Via {
                predicted_duplicate,
                ..
            } => !predicted_duplicate,
            _ => true,
        }
    }

    /// Predicts whether the constructed item ends up on the board (before `normalizeAllTraces`).
    pub fn inserts_item(&self) -> bool {
        match self {
            InsertRequest::Trace {
                polyline, fixed, ..
            } => {
                polyline.corner_count() >= 2
                    && !(polyline.first_corner() == polyline.last_corner()
                        && *fixed < FixedState::UserFixed)
            }
            other => other.consumes_id(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fr_geom::IntPoint;

    fn trace(points: &[(i32, i32)], fixed: FixedState) -> InsertRequest {
        let pts: Vec<IntPoint> = points.iter().map(|&(x, y)| IntPoint::new(x, y)).collect();
        InsertRequest::Trace {
            polyline: Polyline::from_int_points(&pts),
            layer: 0,
            half_width: 10,
            nets: vec![],
            clearance_class: 1,
            fixed,
            try_correct_net: true,
        }
    }

    #[test]
    fn id_prediction() {
        let t = trace(&[(0, 0), (10, 0)], FixedState::Unfixed);
        assert!(t.consumes_id() && t.inserts_item());
        // closed loop: constructed (id consumed) but not inserted unless user fixed
        let loop_ = trace(&[(0, 0), (10, 0), (10, 10), (0, 0)], FixedState::Unfixed);
        assert!(loop_.consumes_id() && !loop_.inserts_item());
        let fixed_loop = trace(&[(0, 0), (10, 0), (10, 10), (0, 0)], FixedState::UserFixed);
        assert!(fixed_loop.inserts_item());
        assert_eq!(t.kind_name(), "PolylineTrace");
    }
}
