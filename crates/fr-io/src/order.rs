//! Scope order information the typed DSN model does not keep.
//!
//! The Java reader processes the file in one pass, so a few results depend on the relative
//! order of scopes:
//!
//! * top level: `placement`/`part_library` data is only used if read before `network`
//!   (components are inserted at the end of the network scope), `wiring` must follow
//!   `network`, `library` must follow `structure`;
//! * inside `structure`: keepouts, planes and `autoroute_settings` are read with the layers
//!   read *so far*; `autoroute_settings` is only read if no keepout/plane/autoroute scope
//!   came before it; the shapes of one `(boundary ...)` scope are added first-shape-last;
//! * inside `wiring`: wires and vias are inserted in file order (item ids!).
//!
//! [`SourceOrder::from_pcb`] extracts this from the raw s-expression; [`SourceOrder::canonical`]
//! assumes the usual layout (structure, library, placement, network, wiring; layers before
//! keepouts; one shape per boundary; wires before vias).

use fr_dsn::model::Dsn;
use fr_dsn::sexpr::{List, Sexpr};

/// A top-level scope the loader processes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopScope {
    Structure,
    Library,
    Placement,
    Network,
    Wiring,
    PartLibrary,
    PlaceControl,
}

/// One entry of the wiring scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WiringItem {
    /// Index into `dsn.wiring.wires`.
    Wire(usize),
    /// Index into `dsn.wiring.vias`.
    Via(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SourceOrder {
    /// Top-level scopes in file order (repeated scopes are listed once, at their last
    /// position, because the typed model keeps the last one).
    pub top: Vec<TopScope>,
    /// For each `dsn.structure.keepouts[i]`: number of `(layer ...)` scopes (with a name)
    /// read before it (index into `dsn.structure.layers`).
    pub keepout_layers_before: Vec<usize>,
    /// Same for `dsn.structure.planes[i]`.
    pub plane_layers_before: Vec<usize>,
    /// Boundary scopes: indices into `dsn.structure.boundaries`, grouped by `(boundary ...)`.
    pub boundary_groups: Vec<Vec<usize>>,
    /// The `autoroute_settings` scope Java actually reads (the first one, if no
    /// keepout/plane scope precedes it) and the number of layer scopes before it.
    pub autoroute_settings: Option<(List, usize)>,
    /// True if an `autoroute_settings` scope follows a keepout/plane/autoroute scope: Java does
    /// not consume it and the structure scope ends early (not emulated).
    pub structure_desync: bool,
    /// Wiring items in file order.
    pub wiring: Vec<WiringItem>,
    /// `(placement (place_control (flip_style rotate_first)) ...)`: the generic Java scope
    /// reader dispatches `place_control` inside `placement` too; the typed model only reads
    /// it at the top level and in `structure`.
    pub placement_flip_style_rotate_first: bool,
}

fn head_lower(l: &List) -> Option<String> {
    l.head().map(|h| h.to_ascii_lowercase())
}

fn is_shape_kw(h: &str) -> bool {
    matches!(
        h,
        "rect" | "rectangle" | "poly" | "polygon" | "circ" | "circle" | "path" | "polyline_path"
    )
}

impl SourceOrder {
    /// The usual scope order (see module docs).
    pub fn canonical(dsn: &Dsn) -> SourceOrder {
        let s = &dsn.structure;
        let n_layers = s.layers.len();
        let mut wiring: Vec<WiringItem> =
            (0..dsn.wiring.wires.len()).map(WiringItem::Wire).collect();
        wiring.extend((0..dsn.wiring.vias.len()).map(WiringItem::Via));
        SourceOrder {
            top: vec![
                TopScope::Structure,
                TopScope::PlaceControl,
                TopScope::Library,
                TopScope::Placement,
                TopScope::PartLibrary,
                TopScope::Network,
                TopScope::Wiring,
            ],
            keepout_layers_before: vec![n_layers; s.keepouts.len()],
            plane_layers_before: vec![n_layers; s.planes.len()],
            boundary_groups: (0..s.boundaries.len()).map(|i| vec![i]).collect(),
            autoroute_settings: s.autoroute_settings.clone().map(|l| (l, n_layers)),
            structure_desync: false,
            wiring,
            placement_flip_style_rotate_first: false,
        }
    }

    /// Extracts the order from the raw `(pcb ...)` list the model was built from.
    pub fn from_pcb(pcb: &List, dsn: &Dsn) -> SourceOrder {
        let mut order = SourceOrder::canonical(dsn);
        order.top.clear();
        for sub in pcb.sublists() {
            let scope = match head_lower(sub).as_deref() {
                Some("structure") => TopScope::Structure,
                Some("library") => TopScope::Library,
                Some("placement") => TopScope::Placement,
                Some("network") => TopScope::Network,
                Some("wiring") => TopScope::Wiring,
                Some("part_library") => TopScope::PartLibrary,
                Some("place_control") => TopScope::PlaceControl,
                _ => continue,
            };
            order.top.retain(|s| *s != scope);
            order.top.push(scope);
            match scope {
                TopScope::Structure => order.read_structure(sub, dsn),
                TopScope::Wiring => order.read_wiring(sub, dsn),
                TopScope::Placement => {
                    for pc in sub.find_all("place_control") {
                        if let Some(f) = pc.find("flip_style") {
                            let rotate_first =
                                f.args().first().and_then(Sexpr::as_atom).is_some_and(|a| {
                                    !a.quoted && a.text.eq_ignore_ascii_case("rotate_first")
                                });
                            // PlaceControl.readScope only ever sets the flag.
                            order.placement_flip_style_rotate_first |= rotate_first;
                        }
                    }
                }
                _ => {}
            }
        }
        order
    }

    fn read_structure(&mut self, l: &List, dsn: &Dsn) {
        let s = &dsn.structure;
        let mut layers = 0usize;
        let mut layer_structure_created = false;
        let mut keepout_idx = 0usize;
        let mut plane_idx = 0usize;
        let mut boundary_idx = 0usize;
        self.boundary_groups.clear();
        self.autoroute_settings = None;
        self.structure_desync = false;
        self.keepout_layers_before = vec![s.layers.len(); s.keepouts.len()];
        self.plane_layers_before = vec![s.layers.len(); s.planes.len()];
        for sub in l.sublists() {
            let Some(h) = head_lower(sub) else { continue };
            match h.as_str() {
                "layer" => {
                    if sub.args().first().and_then(Sexpr::as_atom).is_some() {
                        layers += 1;
                    }
                }
                "boundary" => {
                    let mut group = Vec::new();
                    for item in sub.sublists() {
                        if boundary_idx < s.boundaries.len()
                            && s.boundaries[boundary_idx].line == item.line
                        {
                            group.push(boundary_idx);
                            boundary_idx += 1;
                        }
                    }
                    if !group.is_empty() {
                        self.boundary_groups.push(group);
                    }
                }
                "keepout" | "wire_keepout" | "via_keepout" | "place_keepout" => {
                    layer_structure_created = true;
                    let border_line = sub.sublists().next().map(|b| b.line);
                    if let Some(k) = s.keepouts.get(keepout_idx) {
                        if k.1.shapes.first().map(|b| b.line) == border_line {
                            self.keepout_layers_before[keepout_idx] = layers;
                            keepout_idx += 1;
                        }
                    }
                }
                "plane" => {
                    layer_structure_created = true;
                    if plane_idx < s.planes.len() {
                        self.plane_layers_before[plane_idx] = layers;
                        plane_idx += 1;
                    }
                }
                "autoroute_settings" => {
                    if layer_structure_created {
                        self.structure_desync = true;
                    } else {
                        layer_structure_created = true;
                        self.autoroute_settings = Some((sub.clone(), layers));
                    }
                }
                _ => {}
            }
        }
        // Boundary shapes not matched by line (should not happen): one group each.
        while boundary_idx < s.boundaries.len() {
            self.boundary_groups.push(vec![boundary_idx]);
            boundary_idx += 1;
        }
    }

    fn read_wiring(&mut self, l: &List, dsn: &Dsn) {
        let w = &dsn.wiring;
        self.wiring.clear();
        let (mut wi, mut vi) = (0usize, 0usize);
        for sub in l.sublists() {
            if sub.is("wire") {
                if let Some(wire) = w.wires.get(wi) {
                    let shape_line = wire.shape.line;
                    let contains = sub.sublists().any(|item| {
                        item.line == shape_line
                            && item
                                .head()
                                .is_some_and(|h| is_shape_kw(&h.to_ascii_lowercase()))
                    });
                    if contains {
                        self.wiring.push(WiringItem::Wire(wi));
                        wi += 1;
                    }
                }
            } else if sub.is("via") && vi < w.vias.len() {
                self.wiring.push(WiringItem::Via(vi));
                vi += 1;
            }
        }
        self.wiring
            .extend((wi..w.wires.len()).map(WiringItem::Wire));
        self.wiring.extend((vi..w.vias.len()).map(WiringItem::Via));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synth_order() {
        let src = include_bytes!("../testdata/dsn/synth_order.dsn");
        let top = fr_dsn::sexpr::parse(src).unwrap();
        let pcb = top[0].as_list().unwrap();
        let dsn = Dsn::from_pcb(pcb).unwrap();
        let o = SourceOrder::from_pcb(pcb, &dsn);
        assert_eq!(
            o.top,
            vec![
                TopScope::Structure,
                TopScope::Library,
                TopScope::Network,
                TopScope::Placement,
                TopScope::Wiring
            ]
        );
        assert_eq!(o.keepout_layers_before, vec![0, 0, 5]);
        assert_eq!(o.boundary_groups, vec![vec![0, 1, 2]]);
        assert!(o.placement_flip_style_rotate_first);
        assert_eq!(
            o.wiring,
            vec![
                WiringItem::Via(0),
                WiringItem::Wire(0),
                WiringItem::Via(1),
                WiringItem::Wire(1),
                WiringItem::Via(2)
            ]
        );
        let c = SourceOrder::canonical(&dsn);
        assert_eq!(c.keepout_layers_before, vec![5, 5, 5]);
        assert_eq!(c.boundary_groups.len(), 3);
    }
}
