//! Java `DsnFile.adjustPlaneAutorouteSettings`, run by `DsnReader.readBoard` after the whole
//! file (including `normalizeAllTraces`) when the DSN has no `autoroute_settings` scope.
//!
//! The computation only needs the conduction areas, the traces' layers and the outline, so it
//! is done here; the board step applies the result ([`PlaneAdjustment`]).
//!
//! **Version difference.** The reference source (`reference/freerouting/src`) and the
//! released `freerouting-2.4.1.jar` differ here:
//!
//! | | source ([`compute`]) | jar 2.4.1 ([`compute_2_4_1`]) |
//! |---|---|---|
//! | layer count | >= 1 | > 2 |
//! | area threshold | 30 % of the outline area | 50 % |
//! | outer layers (0, last) | eligible | skipped |
//! | layers with traces | eligible | skipped |
//! | `splitToConvex` null / unknown net | skipped | NPE |
//!
//! [`LoadedDesign::plane_adjustment`](crate::LoadedDesign::plane_adjustment) follows the
//! source, the parity target (`reference/bin/freerouting-parity.jar`); [`compute_2_4_1`] is
//! kept for comparisons with the released jar.

use fr_engine::ids::{FixedState, LayerNo, NetNo};
use fr_engine::rules::BoardRules;
use fr_engine::structure::LayerStructure;
use fr_geom::{PolylineShape, Vector};

use crate::requests::InsertRequest;
use crate::LoadedDesign;

/// The effect of `adjustPlaneAutorouteSettings`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaneAdjustment {
    /// Nets to mark with `setContainsPlane(true)` (in conduction area order, may repeat).
    pub plane_nets: Vec<NetNo>,
    /// Indices into `LoadedDesign::requests` of the conduction areas whose fixed state is
    /// raised to `USER_FIXED` (if lower).
    pub fix_requests: Vec<usize>,
    /// Layers that received a plane (for logging).
    pub layers: Vec<LayerNo>,
}

impl PlaneAdjustment {
    /// Applies the fixed-state part to a request list (the board step does the same on its
    /// items).
    pub fn apply_to_requests(&self, requests: &mut [InsertRequest]) {
        for &i in &self.fix_requests {
            if let InsertRequest::ConductionArea { fixed, .. } = &mut requests[i] {
                if *fixed < FixedState::UserFixed {
                    *fixed = FixedState::UserFixed;
                }
            }
        }
    }

    /// Applies the net part to board rules.
    pub fn apply_to_rules(&self, rules: &mut BoardRules) {
        for &n in &self.plane_nets {
            if let Some(net) = rules.nets.get_mut(n) {
                net.set_contains_plane(true);
            }
        }
    }
}

fn outline_area(outline: &[PolylineShape]) -> f64 {
    let mut board_area = 0.0;
    for s in outline {
        if let Some(pieces) = s.split_to_convex() {
            for p in pieces {
                board_area += p.area();
            }
        }
    }
    board_area
}

/// Area of a conduction area as `ConductionArea.getArea().splitToConvex()` (the relative
/// area translated by `Vector.ZERO`); `None` if the split fails.
fn conduction_area(area: &fr_geom::Area) -> Option<f64> {
    let zero = Vector::from(fr_geom::IntVector::ZERO);
    let pieces = area.translate_by(&zero).split_to_convex()?;
    let mut current = 0.0;
    for p in pieces {
        current += p.area();
    }
    Some(current)
}

/// Source version (see module docs); `None` if Java returns before changing anything.
pub fn compute(
    ls: &LayerStructure,
    outline: &[PolylineShape],
    requests: &[InsertRequest],
    rules: &BoardRules,
) -> Option<PlaneAdjustment> {
    if ls.layers.is_empty() || ls.layers.iter().any(|l| !l.is_signal) {
        return None;
    }
    let board_area = outline_area(outline);
    if board_area <= 0.0 {
        return None;
    }
    let mut result = PlaneAdjustment::default();
    for (idx, r) in requests.iter().enumerate() {
        let InsertRequest::ConductionArea {
            area, layer, nets, ..
        } = r
        else {
            continue;
        };
        if *layer < 0 || *layer >= ls.layer_count() || !ls.layers[*layer as usize].is_signal {
            continue;
        }
        let Some(current) = conduction_area(area) else {
            continue;
        };
        if current < 0.3 * board_area {
            continue;
        }
        for &n in nets {
            if rules.nets.get(n).is_some() {
                result.plane_nets.push(n);
            }
        }
        if !result.layers.contains(layer) {
            result.layers.push(*layer);
        }
        result.fix_requests.push(idx);
    }
    Some(result)
}

/// Version of the released `freerouting-2.4.1.jar` (decompiled; see module docs). The layers
/// with traces are taken from the inserted trace requests (Java looks at the traces after
/// `normalizeAllTraces`, which does not change trace layers).
pub fn compute_2_4_1(design: &LoadedDesign) -> Option<PlaneAdjustment> {
    let ls = &design.layer_structure;
    let n = ls.layer_count();
    if n <= 2 || ls.layers.iter().any(|l| !l.is_signal) {
        return None;
    }
    let mut layer_has_traces = vec![false; n as usize];
    for r in &design.requests {
        if let InsertRequest::Trace { layer, .. } = r {
            if r.inserts_item() && (0..n).contains(layer) {
                layer_has_traces[*layer as usize] = true;
            }
        }
    }
    let board_area = outline_area(&design.outline_shapes);
    let mut result = PlaneAdjustment::default();
    for (idx, r) in design.requests.iter().enumerate() {
        let InsertRequest::ConductionArea {
            area, layer, nets, ..
        } = r
        else {
            continue;
        };
        let layer = *layer;
        if layer_has_traces[layer as usize]
            || !ls.layers[layer as usize].is_signal
            || layer == 0
            || layer == n - 1
        {
            continue;
        }
        let current = conduction_area(area).unwrap_or(0.0);
        if current < 0.5 * board_area {
            continue;
        }
        result.plane_nets.extend(nets.iter().copied());
        if !result.layers.contains(&layer) {
            result.layers.push(layer);
        }
        result.fix_requests.push(idx);
    }
    Some(result)
}
