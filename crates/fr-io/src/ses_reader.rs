//! Specctra session reader (Java `io.specctra.SesReader`): imports the wires and vias of the
//! `(routes (network_out (net ...)))` scope onto a board. Test infrastructure for scoring and
//! optimizer parity.

use fr_dsn::model::{LayerRef, Shape as DsnShape, ShapeKind};
use fr_dsn::sexpr::{Atom, List, Sexpr};
use fr_engine::board::BasicBoard;
use fr_engine::ids::{FixedState, NetNo};
use fr_engine::rules::ItemClass;
use fr_geom::{Point, Polyline};

use crate::library::clean_padstack_name;
use crate::shapes::{self, ParserLayer, ParserLayers};

/// Java `SesImportSummary`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SesImportSummary {
    pub wires_imported: i32,
    pub vias_imported: i32,
    pub errors_encountered: i32,
}

fn is_string_token(a: &Atom) -> bool {
    // The Java scanner returns Integer / Double tokens for numbers.
    a.quoted || a.as_num().is_none()
}

struct Reader<'b> {
    board: &'b mut BasicBoard,
    layers: Vec<ParserLayer>,
    den: f64,
    summary: SesImportSummary,
}

impl Reader<'_> {
    fn net(&mut self, l: &List) {
        let Some(name) = l
            .args()
            .first()
            .and_then(Sexpr::as_atom)
            .filter(|a| is_string_token(a))
        else {
            log::warn!("SesReader.processNetScope: String expected");
            self.summary.errors_encountered += 1;
            return;
        };
        let Some(net) = self.board.rules.nets.get_by_name(&name.text, 1) else {
            log::warn!("SesReader: net not found: '{}' — skipping", name.text);
            self.summary.errors_encountered += 1;
            return;
        };
        let nets = [net.net_number];
        for sub in l.sublists() {
            let ok = if sub.is("wire") {
                self.wire(sub, &nets)
            } else if sub.is("via") {
                self.via(sub, &nets)
            } else {
                true
            };
            if !ok {
                self.summary.errors_encountered += 1;
            }
        }
    }

    /// `Shape.readPolygonPathScope` with the board layer structure; `None` for Java `null`.
    fn read_path(&self, l: &List) -> Option<(i32, f64, Vec<f64>)> {
        let layer_atom = l.args().first().and_then(Sexpr::as_atom)?;
        let mut nums = Vec::new();
        for item in &l.args()[1..] {
            if let Sexpr::Atom(a) = item {
                nums.push(a.as_num());
            }
        }
        if nums.len() < 5 {
            return None;
        }
        let nums: Option<Vec<f64>> = nums.into_iter().collect();
        let nums = nums?;
        let layer = match layer_atom.text.as_str() {
            "pcb" => LayerRef::Pcb,
            "signal" => LayerRef::Signal,
            n => LayerRef::Named(n.to_string()),
        };
        let shape = DsnShape {
            layer,
            kind: ShapeKind::PolygonPath {
                width: nums[0],
                coords: nums[1..].to_vec(),
            },
            line: l.line,
        };
        let resolved = shapes::resolve(&shape, Some(ParserLayers(&self.layers)))?;
        Some((
            resolved.layer.expect("path layer").no(),
            nums[0],
            nums[1..].to_vec(),
        ))
    }

    fn wire(&mut self, l: &List, nets: &[NetNo]) -> bool {
        let mut path = None;
        for sub in l.sublists() {
            if sub.is("path") {
                path = self.read_path(sub);
            }
        }
        let Some((layer, width, coords)) = path else {
            return true; // conduction areas have no path
        };
        let den = self.den;
        let points: Vec<Point> = (0..coords.len() / 2)
            .map(|i| {
                Point::get_instance(
                    fr_jcompat::math_round_i32(coords[2 * i] / den),
                    fr_jcompat::math_round_i32(coords[2 * i + 1] / den),
                )
            })
            .collect();
        let half_width = fr_jcompat::math_round_i32(width / (2.0 * den));
        let board = &mut *self.board;
        let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let polyline = Polyline::from_points(&points);
            let d = board.rules.default_net_class();
            let cl = board.rules.net_classes[d]
                .default_item_clearance_classes
                .get(ItemClass::Trace);
            board.insert_trace(polyline, layer, half_width, nets, cl, FixedState::UserFixed);
        }));
        if ok.is_err() {
            log::warn!("SesReader.processWireScope: failed to import wire");
            return false;
        }
        self.summary.wires_imported += 1;
        true
    }

    fn via(&mut self, l: &List, nets: &[NetNo]) -> bool {
        let args = l.args();
        let Some(name) = args
            .first()
            .and_then(Sexpr::as_atom)
            .filter(|a| is_string_token(a))
        else {
            return false;
        };
        let num = |i: usize| args.get(i).and_then(Sexpr::as_atom).and_then(Atom::as_num);
        let (Some(x), Some(y)) = (num(1), num(2)) else {
            return false;
        };
        // Only sub-scopes may follow the location.
        if args[3..].iter().any(|a| a.as_atom().is_some()) {
            return false;
        }
        let Some(padstack) = self
            .board
            .library
            .padstacks
            .get_by_name(&clean_padstack_name(&name.text))
        else {
            log::warn!(
                "SesReader.processViaScope: via padstack not found: {}",
                name.text
            );
            return false;
        };
        let pid = padstack.id;
        let p = Point::get_instance(
            fr_jcompat::math_round_i32(x / self.den),
            fr_jcompat::math_round_i32(y / self.den),
        );
        let board = &mut *self.board;
        let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let d = board.rules.default_net_class();
            let cl = board.rules.net_classes[d]
                .default_item_clearance_classes
                .get(ItemClass::Via);
            board.insert_via(pid, p, nets, cl, FixedState::UserFixed, true);
        }));
        if ok.is_err() {
            return false;
        }
        self.summary.vias_imported += 1;
        true
    }
}

/// Java `SesReader.read(in, board)`. A malformed file (not `(session ...)`) imports nothing.
pub fn read_ses(src: &[u8], board: &mut BasicBoard) -> SesImportSummary {
    let Ok(top) = fr_dsn::sexpr::parse(src) else {
        log::warn!("SesReader: not a Specctra session file");
        return SesImportSummary::default();
    };
    let Some(session) = top
        .first()
        .and_then(Sexpr::as_list)
        .filter(|l| l.is("session"))
    else {
        log::warn!("SesReader: not a Specctra session file");
        return SesImportSummary::default();
    };
    let layers: Vec<ParserLayer> = board
        .layer_structure
        .layers
        .iter()
        .enumerate()
        .map(|(i, l)| ParserLayer {
            name: l.name.clone(),
            no: i as i32,
            is_signal: l.is_signal,
            net_names: Vec::new(),
        })
        .collect();
    let den = board.communication.coordinate_transform.dsn_to_board(1.0)
        / board.communication.resolution as f64;
    let mut r = Reader {
        board,
        layers,
        den,
        summary: SesImportSummary::default(),
    };
    for routes in session.sublists().filter(|s| s.is("routes")) {
        for nw in routes.sublists().filter(|s| s.is("network_out")) {
            for net in nw.sublists().filter(|s| s.is("net")) {
                r.net(net);
            }
        }
    }
    r.summary
}
