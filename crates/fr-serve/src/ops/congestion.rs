//! Op `congestion` (capability `congestion`), SPEC 5.7. The rule, restated:
//!
//! * Grid: `origin` = minimum corner of the boundary bbox (`facts::boundary`), `cols` x `rows` =
//!   `ceil(width / cell)` x `ceil(height / cell)`, row-major arrays of `rows * cols`, row 0 at the
//!   minimum y. A cell is the CLOSED square `[x, x + cell] x [y, y + cell]` (cells share their edges).
//!   `layers` is `"all"` (DSN layer order) or a list of names in the requested order; an unknown name is
//!   `bad_request` with `details.layer`.
//! * `capacity` (per layer, per cell) = the sum over the free spans of the cell's vertical centre line
//!   (`x = cell x + cell / 2`, integer division, `y` over the cell) of `floor(span / pitch)`, plus the
//!   same for its horizontal centre line. `pitch` = default-net-class trace width on that layer
//!   (2 * half width) + the default-class clearance on that layer. Obstacles are the pins, the keepouts
//!   that stop wires and the fixed wiring (traces and vias whose fixed state is not `Unfixed`, i.e.
//!   SystemFixed/UserFixed), each grown by the default clearance on that layer; a free span is the
//!   gap between obstacles. The area outside the boundary bbox is blocked (capacity 0). Obstacle
//!   chords are computed over the grown convex shape and rounded outward to integer units.
//! * `demand` = routed trace segments (centre line) that intersect the cell (closed), on that layer.
//! * `airwire` = unrouted connections whose end-point bounding box overlaps the cell (closed), RUDY
//!   style; connections have no layer, so the count is the same on every layer.
//!
//! Integer arrays only, filled in index order (no hash-map iteration), so the result is deterministic.

use fr_engine::board::{BasicBoard, ItemKind, ObstacleKind};
use fr_engine::ids::FixedState;
use fr_engine::rules::board_rules::BoardRules;
use serde_json::{json, Map, Value};

use crate::facts;
use crate::proto::{as_int, as_str, check_keys, req, ProtoError, R};
use crate::session::Session;

/// True once this module implements the op and passes its conformance check.
pub const CLAIMED: bool = true;

/// The grid geometry.
#[derive(Clone, Copy, Debug)]
pub struct Grid {
    pub x0: i64,
    pub y0: i64,
    pub cell: i64,
    pub cols: i64,
    pub rows: i64,
}

type Span = (i64, i64);

fn ceil_div(a: i64, b: i64) -> i64 {
    -((-a).div_euclid(b))
}

impl Grid {
    pub fn new(b: [i64; 4], cell: i64) -> Grid {
        Grid { x0: b[0], y0: b[1], cell, cols: ceil_div(b[2] - b[0], cell).max(0), rows: ceil_div(b[3] - b[1], cell).max(0) }
    }

    pub fn len(&self) -> usize {
        (self.cols * self.rows) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn index(&self, col: i64, row: i64) -> usize {
        (row * self.cols + col) as usize
    }

    /// Closed cell range `(col0, col1, row0, row1)` of the cells whose closed square meets the closed
    /// box `[minx, maxx] x [miny, maxy]`, clamped to the grid; `None` if there is none.
    fn cells_meeting(&self, minx: i64, miny: i64, maxx: i64, maxy: i64) -> Option<(i64, i64, i64, i64)> {
        let c0 = (ceil_div(minx - self.x0, self.cell) - 1).max(0);
        let c1 = ((maxx - self.x0).div_euclid(self.cell)).min(self.cols - 1);
        let r0 = (ceil_div(miny - self.y0, self.cell) - 1).max(0);
        let r1 = ((maxy - self.y0).div_euclid(self.cell)).min(self.rows - 1);
        (c0 <= c1 && r0 <= r1).then_some((c0, c1, r0, r1))
    }

    fn cell_box(&self, col: i64, row: i64) -> (i64, i64, i64, i64) {
        let (x, y) = (self.x0 + col * self.cell, self.y0 + row * self.cell);
        (x, y, x + self.cell, y + self.cell)
    }
}

/// `sum(floor(gap / pitch))` over the gaps of `[lo, hi]` left by `blocked` (open intervals, any order).
pub fn free_capacity(blocked: &mut [Span], lo: i64, hi: i64, pitch: i64) -> i64 {
    if pitch <= 0 || hi <= lo {
        return 0;
    }
    blocked.sort_unstable();
    let (mut at, mut tracks) = (lo, 0);
    for &(a, b) in blocked.iter() {
        if a > at {
            tracks += (a.min(hi) - at) / pitch;
        }
        at = at.max(b);
        if at >= hi {
            return tracks;
        }
    }
    tracks + (hi - at) / pitch
}

/// Closed segment `p-q` meets the closed box `(x0, y0, x1, y1)` (exact, integer).
pub fn segment_meets_box(p: (i64, i64), q: (i64, i64), b: (i64, i64, i64, i64)) -> bool {
    let (x0, y0, x1, y1) = b;
    if p.0.max(q.0) < x0 || p.0.min(q.0) > x1 || p.1.max(q.1) < y0 || p.1.min(q.1) > y1 {
        return false;
    }
    let side = |c: (i64, i64)| -> i128 {
        (q.0 - p.0) as i128 * (c.1 - p.1) as i128 - (q.1 - p.1) as i128 * (c.0 - p.0) as i128
    };
    let signs = [side((x0, y0)), side((x1, y0)), side((x0, y1)), side((x1, y1))];
    !(signs.iter().all(|&s| s > 0) || signs.iter().all(|&s| s < 0))
}

/// Chord of a convex polygon with a vertical (`vertical`) or horizontal line at `c`, rounded outward.
fn chord(corners: &[(f64, f64)], vertical: bool, c: f64) -> Option<Span> {
    let at = |p: (f64, f64)| if vertical { (p.0, p.1) } else { (p.1, p.0) };
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for i in 0..corners.len() {
        let (p, q) = (at(corners[i]), at(corners[(i + 1) % corners.len()]));
        if (p.0 < c && q.0 < c) || (p.0 > c && q.0 > c) {
            continue;
        }
        let v = if p.0 == q.0 { vec![p.1, q.1] } else { vec![p.1 + (q.1 - p.1) * (c - p.0) / (q.0 - p.0)] };
        for y in v {
            lo = lo.min(y);
            hi = hi.max(y);
        }
    }
    (lo <= hi).then(|| (lo.floor() as i64, hi.ceil() as i64))
}

/// Per layer: blocked chords of each cell's vertical and horizontal centre lines.
struct Blocked {
    vert: Vec<Vec<Span>>,
    horz: Vec<Vec<Span>>,
}

fn blocks_wires(board: &BasicBoard, key: fr_engine::board::ItemKey) -> bool {
    let item = board.item(key);
    match &item.kind {
        ItemKind::Pin(_) => true,
        ItemKind::Trace(_) | ItemKind::Via(_) => item.fixed_state() != FixedState::Unfixed,
        ItemKind::ObstacleArea(a) => matches!(a.kind, ObstacleKind::Keepout | ObstacleKind::WireKeepout),
        _ => false,
    }
}

fn add_blocked(g: &Grid, per_layer: &mut [Blocked], layer: usize, clearance: f64, shape: &fr_geom::tile_shape::TileShape) {
    if shape.is_empty() || !shape.is_bounded() {
        return;
    }
    let grown = shape.offset(clearance);
    let corners: Vec<(f64, f64)> = grown.corner_approx_arr().iter().map(|p| (p.x, p.y)).collect();
    if corners.is_empty() {
        return;
    }
    let minx = corners.iter().map(|p| p.0).fold(f64::INFINITY, f64::min).floor() as i64;
    let maxx = corners.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max).ceil() as i64;
    let miny = corners.iter().map(|p| p.1).fold(f64::INFINITY, f64::min).floor() as i64;
    let maxy = corners.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max).ceil() as i64;
    let Some((c0, c1, r0, r1)) = g.cells_meeting(minx, miny, maxx, maxy) else { return };
    for row in r0..=r1 {
        for col in c0..=c1 {
            let (bx0, by0, bx1, by1) = g.cell_box(col, row);
            let (cx, cy) = (bx0 + g.cell / 2, by0 + g.cell / 2);
            let i = g.index(col, row);
            if (minx..=maxx).contains(&cx) {
                if let Some((a, b)) = chord(&corners, true, cx as f64) {
                    per_layer[layer].vert[i].push((a.max(by0), b.min(by1)));
                }
            }
            if (miny..=maxy).contains(&cy) {
                if let Some((a, b)) = chord(&corners, false, cy as f64) {
                    per_layer[layer].horz[i].push((a.max(bx0), b.min(bx1)));
                }
            }
        }
    }
}

fn point(p: fr_geom::FloatPoint) -> (i64, i64) {
    (p.x.round() as i64, p.y.round() as i64)
}

/// The result for the layers `wanted` (indices into the board's layers).
pub fn compute(board: &BasicBoard, boundary: [i64; 4], cell: i64, wanted: &[usize]) -> Value {
    let g = Grid::new(boundary, cell);
    let layer_count = board.layer_structure.layers.len();
    let mut blocked: Vec<Blocked> =
        (0..layer_count).map(|_| Blocked { vert: vec![Vec::new(); g.len()], horz: vec![Vec::new(); g.len()] }).collect();
    let mut demand = vec![vec![0i64; g.len()]; layer_count];
    let dc = BoardRules::default_clearance_class();
    let clearance = |l: usize| board.rules.clearance_matrix.get_value(dc, dc, l as i32, false) as f64;

    for key in board.get_items() {
        let item = board.item(key);
        if let Some(t) = item.as_trace() {
            let l = t.layer() as usize;
            let n = (t.corner_count() - 1).max(0);
            for s in 0..n {
                let (p, q) = (point(t.polyline().corner_approx(s)), point(t.polyline().corner_approx(s + 1)));
                let Some((c0, c1, r0, r1)) = g.cells_meeting(p.0.min(q.0), p.1.min(q.1), p.0.max(q.0), p.1.max(q.1)) else { continue };
                for row in r0..=r1 {
                    for col in c0..=c1 {
                        if segment_meets_box(p, q, g.cell_box(col, row)) {
                            demand[l][g.index(col, row)] += 1;
                        }
                    }
                }
            }
        }
        if !blocks_wires(board, key) {
            continue;
        }
        for idx in 0..board.tile_shape_count(key) {
            let (Some(shape), l) = (board.tile_shape(key, idx), board.shape_layer(key, idx)) else { continue };
            if l >= 0 && (l as usize) < layer_count {
                add_blocked(&g, &mut blocked, l as usize, clearance(l as usize), &shape);
            }
        }
    }

    let (_, unrouted) = facts::connections(board);
    let mut airwire = vec![0i64; g.len()];
    for u in &unrouted {
        let (a, b) = (u.from_xy, u.to_xy);
        if let Some((c0, c1, r0, r1)) = g.cells_meeting(a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])) {
            for row in r0..=r1 {
                for col in c0..=c1 {
                    airwire[g.index(col, row)] += 1;
                }
            }
        }
    }

    let [bx0, by0, bx1, by1] = boundary;
    let layers: Vec<Value> = wanted
        .iter()
        .map(|&l| {
            let pitch = 2 * board.rules.get_default_trace_half_width(l as i32) as i64 + clearance(l) as i64;
            let mut capacity = vec![0i64; g.len()];
            for row in 0..g.rows {
                for col in 0..g.cols {
                    let i = g.index(col, row);
                    let (cx0, cy0, cx1, cy1) = g.cell_box(col, row);
                    let (cx, cy) = (cx0 + g.cell / 2, cy0 + g.cell / 2);
                    let mut c = 0;
                    if (bx0..=bx1).contains(&cx) {
                        c += free_capacity(&mut blocked[l].vert[i], cy0.max(by0), cy1.min(by1), pitch);
                    }
                    if (by0..=by1).contains(&cy) {
                        c += free_capacity(&mut blocked[l].horz[i], cx0.max(bx0), cx1.min(bx1), pitch);
                    }
                    capacity[i] = c;
                }
            }
            json!({ "name": board.layer_structure.layers[l].name, "capacity": capacity, "demand": demand[l], "airwire": airwire })
        })
        .collect();
    json!({ "origin": [g.x0, g.y0], "cell": cell, "cols": g.cols, "rows": g.rows, "layers": layers })
}

/// `"all"` or a list of layer names, as indices into the board's layers.
fn wanted_layers(board: &BasicBoard, v: &Value) -> R<Vec<usize>> {
    let names: Vec<&str> = board.layer_structure.layers.iter().map(|l| l.name.as_str()).collect();
    match v {
        Value::String(s) if s == "all" => Ok((0..names.len()).collect()),
        Value::Array(list) => list
            .iter()
            .map(|n| {
                let n = as_str(n, "congestion.layers[]")?;
                names.iter().position(|x| *x == n).ok_or_else(|| {
                    ProtoError::bad_request(format!("unknown layer '{n}'")).with_details(json!({ "layer": n }))
                })
            })
            .collect(),
        _ => Err(ProtoError::bad_request("congestion.layers must be \"all\" or a list of layer names")),
    }
}

pub fn handle(session: &mut Session, args: &Map<String, Value>) -> R<Value> {
    check_keys(args, &["cell", "layers"], "congestion")?;
    let cell = as_int(req(args, "cell", "congestion")?, 1, 1 << 40, "congestion.cell")?;
    let board = session.loaded()?;
    let wanted = wanted_layers(&board.board, req(args, "layers", "congestion")?)?;
    Ok(compute(&board.board, facts::boundary(&board.board), cell, &wanted))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_of_two_cells_by_hand() {
        // cell 1000, pitch 200. Cell 1 free: 1000/200 = 5 per line, 10 in all.
        assert_eq!(free_capacity(&mut [], 0, 1000, 200) * 2, 10);
        // Cell 2: a keepout grown to x 1200..1800 / y 200..800 leaves, on x = 1500, [0,200] and [800,1000]
        // (1 + 1) and, on y = 500, [1000,1200] and [1800,2000] (1 + 1): 4.
        let v = free_capacity(&mut [(200, 800)], 0, 1000, 200);
        let h = free_capacity(&mut [(1200, 1800)], 1000, 2000, 200);
        assert_eq!((v, h, v + h), (2, 2, 4));
    }

    #[test]
    fn spans_merge_and_clip() {
        assert_eq!(free_capacity(&mut [(50, 300), (200, 500)], 0, 1000, 100), 0 + 5);
        assert_eq!(free_capacity(&mut [(-50, 20), (900, 2000)], 0, 1000, 100), 8);
        assert_eq!(free_capacity(&mut [], 0, 99, 100), 0);
    }

    #[test]
    fn segment_box_tests() {
        let b = (0, 0, 10, 10);
        assert!(segment_meets_box((-5, 5), (15, 5), b));
        assert!(segment_meets_box((10, 10), (20, 20), b)); // touches the corner
        assert!(!segment_meets_box((11, 0), (11, 10), b));
        assert!(!segment_meets_box((-5, 8), (8, 21), b)); // diagonal passes above the corner
        assert!(segment_meets_box((-5, 5), (5, 15), b));
    }

    #[test]
    fn grid_dims() {
        let g = Grid::new([10, -25, 31, -10], 10);
        assert_eq!((g.cols, g.rows), (3, 2));
        assert_eq!(g.cells_meeting(10, -25, 10, -25), Some((0, 0, 0, 0)));
        assert_eq!(g.cells_meeting(20, -15, 20, -15), Some((0, 1, 0, 1)));
    }
}
