//! `fastroute serve` hello settings for board-edge clearance and minimum trace width (router protocol
//! 1.0, SPEC 5.1). Both are the stock CLI's own keys (`--router.copper_to_edge_clearance_um`,
//! `--router.min_trace_width_um`), applied by the same code on both paths:
//! fr-settings `CliSettings::parse` -> `headless_merger` -> fr-io `post_load.rs:101`
//! (`apply_copper_to_edge_clearance_override`, reached from `prepare_for_routing` in the CLI at
//! main.rs:437 and in serve at load.rs:101) and fr-engine `autoroute/control.rs:346`
//! (`min_trace_half_width`, via `RouterSettings::get_min_trace_width_um`, fr-settings settings.rs:515).
//! Upstream defaults are unchanged (250 um edge clearance, defaults.rs:16; no minimum width).

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use fr_serve::serde_json::{json, Value};

const BIN: &str = env!("CARGO_BIN_EXE_fastroute");
const EDGE_KEY: &str = "router.copper_to_edge_clearance_um";
const WIDTH_KEY: &str = "router.min_trace_width_um";
/// Rounding slack of the integer SES grid (1 um is ten resolution units on the corpus boards).
const GEOMETRY_TOLERANCE_UM: f64 = 1.0;

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A real corpus board (`reference/pcbkit-corpus`, toolkit-generated and never committed), found in
/// this checkout, in the main checkout next to the worktrees, or through `FASTROUTE_CORPUS`. Without
/// the corpus the tests fail loudly rather than skip: the properties only mean something on a real board.
fn board() -> PathBuf {
    let rel = "det/hb200/board.dsn";
    let mut roots: Vec<PathBuf> = std::env::var_os("FASTROUTE_CORPUS").map(PathBuf::from).into_iter().collect();
    roots.push(manifest().join("../../reference/pcbkit-corpus"));
    roots.push(manifest().join("../../../../fastroute/reference/pcbkit-corpus"));
    for r in roots {
        if r.join(rel).is_file() {
            return r.join(rel).canonicalize().unwrap();
        }
    }
    panic!("corpus board {rel} not found: set FASTROUTE_CORPUS to the reference/pcbkit-corpus directory");
}

struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl Server {
    fn start() -> Server {
        let mut child = Command::new(BIN)
            .arg("serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn fastroute serve");
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Server { child, stdin, stdout, next_id: 1 }
    }

    fn call(&mut self, op: &str, args: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{}", json!({ "id": id, "op": op, "args": args })).unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        assert!(self.stdout.read_line(&mut line).unwrap() > 0, "server closed stdout during {op}");
        fr_serve::serde_json::from_str(&line).unwrap()
    }

    fn ok(&mut self, op: &str, args: Value) -> Value {
        let r = self.call(op, args);
        assert_eq!(r["ok"], json!(true), "{op}: {r}");
        r["result"].clone()
    }

    /// The whole hello response (the `build` hash is on the envelope, not in `result`).
    fn hello_full(&mut self, threads: u32, settings: Value) -> Value {
        let r = self.call("hello", json!({ "protocol": "1.0.0", "client": "serve_settings", "threads": threads, "settings": settings }));
        assert_eq!(r["ok"], json!(true), "{r}");
        r
    }

    fn finish(mut self) {
        let r = self.call("shutdown", json!({}));
        assert_eq!(r["result"], json!({}));
        drop(self.stdin.take());
        assert!(self.child.wait().unwrap().success());
    }
}

/// Routes the board over the protocol at seed 0 and returns the SES.
fn serve_ses(dsn: &Path, settings: Value) -> String {
    let mut s = Server::start();
    s.hello_full(1, settings);
    s.ok("load", json!({ "dsn": { "path": dsn.to_str().unwrap() } }));
    s.ok("route", json!({ "seed": 0, "nets": "all", "from": "scratch" }));
    let ses = s.ok("export", json!({ "format": "ses" }))["text"].as_str().unwrap().to_string();
    s.finish();
    ses
}

/// The stock CLI on the same DSN with the same flags (the fork's `-de/-do --no-time-limits` run).
fn stock_ses(dsn: &Path, flags: &[String]) -> String {
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("settings-stock-{}.ses", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let mut cmd = Command::new(BIN);
    cmd.args(["-de", dsn.to_str().unwrap(), "-do", out.to_str().unwrap(), "--no-time-limits"])
        .args(["--router.autorouter.max_threads=1", "--router.optimizer.max_threads=1"])
        .args(flags)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (k, _) in std::env::vars().filter(|(k, _)| k.to_uppercase().starts_with("FREEROUTING__ROUTER__")) {
        cmd.env_remove(k);
    }
    assert!(cmd.status().unwrap().success());
    let ses = std::fs::read_to_string(&out).unwrap();
    let _ = std::fs::remove_file(&out);
    ses
}

// ------------------------------------------------------------------------------ geometry

fn tokens(text: &str) -> Vec<&str> {
    text.split(|c: char| c.is_whitespace() || c == '(' || c == ')').filter(|t| !t.is_empty()).collect()
}

/// The board outline of a DSN as a closed polygon in um: the `(path pcb 0 x y ..)` in `(boundary ..)`.
fn dsn_outline_um(dsn: &str) -> Vec<(f64, f64)> {
    let t = tokens(dsn);
    let res = t.windows(3).find(|w| w[0] == "resolution").map(|w| w[2].parse::<f64>().unwrap()).unwrap_or(1.0);
    let unit_um = match t.windows(2).find(|w| w[0] == "unit").map(|w| w[1]) {
        Some("mm") => 1000.0,
        Some("mil") => 25.4,
        Some("inch") => 25_400.0,
        _ => 1.0,
    };
    let _ = res; // coordinates in a DSN file are in its `unit`, not in resolution steps
    let b = t.iter().position(|x| *x == "boundary").expect("DSN has a boundary");
    assert_eq!(&t[b + 1..b + 3], ["path", "pcb"]);
    let pts: Vec<f64> = t[b + 4..].iter().map_while(|x| x.parse::<f64>().ok()).collect();
    pts.chunks_exact(2).map(|p| (p[0] * unit_um, p[1] * unit_um)).collect()
}

fn dist_point_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let u = if len2 == 0.0 { 0.0 } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) };
    ((p.0 - a.0 - u * dx).hypot(p.1 - a.1 - u * dy)).max(0.0)
}

fn dist_segments(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)) -> f64 {
    let cross = |o: (f64, f64), p: (f64, f64), q: (f64, f64)| (p.0 - o.0) * (q.1 - o.1) - (p.1 - o.1) * (q.0 - o.0);
    if cross(a, b, c) * cross(a, b, d) < 0.0 && cross(c, d, a) * cross(c, d, b) < 0.0 {
        return 0.0;
    }
    [dist_point_segment(a, c, d), dist_point_segment(b, c, d), dist_point_segment(c, a, b), dist_point_segment(d, a, b)]
        .into_iter()
        .fold(f64::INFINITY, f64::min)
}

/// What a session's copper amounts to, in um.
struct Copper {
    /// Narrowest wire width.
    min_width_um: f64,
    /// Smallest gap between any wire edge or via rim and the outline.
    min_edge_gap_um: f64,
    wires: usize,
}

fn copper(ses: &str, outline: &[(f64, f64)]) -> Copper {
    let t = tokens(ses);
    let res = t.windows(3).find(|w| w[0] == "resolution").map(|w| w[2].parse::<f64>().unwrap()).expect("SES resolution");
    let unit_um = match t.windows(3).find(|w| w[0] == "resolution").map(|w| w[1]) {
        Some("mm") => 1000.0,
        Some("mil") => 25.4,
        Some("inch") => 25_400.0,
        _ => 1.0,
    } / res;
    let edges: Vec<_> = outline.windows(2).map(|w| (w[0], w[1])).collect();
    let edge_dist = |a, b| edges.iter().map(|&(c, d)| dist_segments(a, b, c, d)).fold(f64::INFINITY, f64::min);
    let (mut min_width_um, mut min_edge_gap_um, mut wires) = (f64::INFINITY, f64::INFINITY, 0);
    let mut i = 0;
    while i < t.len() {
        if t[i] == "path" {
            let width = t[i + 2].parse::<f64>().unwrap() * unit_um;
            let pts: Vec<f64> = t[i + 3..].iter().map_while(|x| x.parse::<f64>().ok()).collect();
            let pts: Vec<(f64, f64)> = pts.chunks_exact(2).map(|p| (p[0] * unit_um, p[1] * unit_um)).collect();
            min_width_um = min_width_um.min(width);
            wires += 1;
            for w in pts.windows(2) {
                min_edge_gap_um = min_edge_gap_um.min(edge_dist(w[0], w[1]) - width / 2.0);
            }
            i += 3 + 2 * pts.len();
        } else if t[i] == "via" && t[i - 1] != "padstack" && t.get(i + 3).is_some_and(|x| x.parse::<f64>().is_ok()) {
            // padstack names are `Via[0-3]_<diameter>:<hole>_um`; the rim is the pad radius from the centre
            let pad_um: f64 = t[i + 1].split('_').nth(1).and_then(|s| s.split(':').next()).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let c = (t[i + 2].parse::<f64>().unwrap() * unit_um, t[i + 3].parse::<f64>().unwrap() * unit_um);
            min_edge_gap_um = min_edge_gap_um.min(edge_dist(c, c) - pad_um / 2.0);
            i += 4;
        } else {
            i += 1;
        }
    }
    Copper { min_width_um, min_edge_gap_um, wires }
}

// ------------------------------------------------------------------------------ runs, shared

/// A small board whose outline sits close to the pads, so an edge clearance of 500 or 800 um binds:
/// tiny.dsn with the outline shrunk to 16 x 10.5 mm. Written once per process.
fn fast_board() -> PathBuf {
    static P: OnceLock<PathBuf> = OnceLock::new();
    P.get_or_init(|| {
        let tiny = std::fs::read_to_string(manifest().join("../fr-serve/tests/data/tiny.dsn")).unwrap();
        let old = "30000 -25000  10000 -25000  10000 -10000  30000 -10000\n            30000 -25000";
        assert!(tiny.contains(old), "tiny.dsn outline changed");
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("serve-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("edge.dsn");
        std::fs::write(&out, tiny.replacen(old, "28000 -23000  12000 -23000  12000 -12500  28000 -12500\n            28000 -23000", 1)).unwrap();
        out
    })
    .clone()
}

/// Session of `dsn` under `settings`, routed once per process and cached.
fn run(dsn: &Path, settings: Value) -> String {
    static CACHE: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());
    let key = format!("{}|{settings}", dsn.display());
    if let Some(hit) = CACHE.lock().unwrap().get(&key) {
        return hit.clone();
    }
    let ses = serve_ses(dsn, settings);
    CACHE.lock().unwrap().insert(key, ses.clone());
    ses
}

const FLOOR_UM: u32 = 140;

// ------------------------------------------------------------------------------ checks, per board

fn check_edge_clearance_changes_and_is_enforced(dsn: &Path) {
    let outline = dsn_outline_um(&std::fs::read_to_string(dsn).unwrap());
    let loose = run(dsn, json!({ EDGE_KEY: 200 }));
    let tight = run(dsn, json!({ EDGE_KEY: 500 }));
    assert_ne!(loose, tight, "200 um and 500 um edge clearance gave the same session");
    let (c200, c500) = (copper(&loose, &outline), copper(&tight, &outline));
    assert!(c200.wires > 0 && c500.wires > 0);
    eprintln!("edge gap: 200 -> {:.1} um, 500 -> {:.1} um", c200.min_edge_gap_um, c500.min_edge_gap_um);
    assert!(c500.min_edge_gap_um >= 500.0 - GEOMETRY_TOLERANCE_UM, "copper {:.1} um from the outline with 500 um configured", c500.min_edge_gap_um);
    assert!(c200.min_edge_gap_um >= 200.0 - GEOMETRY_TOLERANCE_UM, "200 um run violates its own clearance: {:.1}", c200.min_edge_gap_um);
}

/// The 200 um run keeps its natural gap on its own; a clearance above that must move copper away.
fn check_clearance_above_natural_gap_moves_copper(dsn: &Path) {
    let outline = dsn_outline_um(&std::fs::read_to_string(dsn).unwrap());
    let natural = copper(&run(dsn, json!({ EDGE_KEY: 200 })), &outline);
    let wide = copper(&run(dsn, json!({ EDGE_KEY: 800 })), &outline);
    eprintln!("edge gap: 200 -> {:.1} um, 800 -> {:.1} um", natural.min_edge_gap_um, wide.min_edge_gap_um);
    assert!(natural.min_edge_gap_um < 800.0 - GEOMETRY_TOLERANCE_UM, "800 um cannot bind: the 200 um run keeps {:.1}", natural.min_edge_gap_um);
    assert!(wide.min_edge_gap_um >= 800.0 - GEOMETRY_TOLERANCE_UM, "copper {:.1} um from the outline with 800 um configured", wide.min_edge_gap_um);
}

fn check_serve_equals_stock_with_both_flags(dsn: &Path) {
    let flags = [format!("--{EDGE_KEY}=500"), format!("--{WIDTH_KEY}={FLOOR_UM}")];
    assert_eq!(run(dsn, json!({ EDGE_KEY: 500, WIDTH_KEY: FLOOR_UM })), stock_ses(dsn, &flags));
}

// ------------------------------------------------------------------------------ tests (fast board)

#[test]
fn edge_clearance_changes_the_session_and_is_enforced_geometrically() {
    check_edge_clearance_changes_and_is_enforced(&fast_board());
}

#[test]
fn an_edge_clearance_above_the_natural_gap_moves_copper_away_from_the_outline() {
    check_clearance_above_natural_gap_moves_copper(&fast_board());
}

#[test]
fn a_width_floor_below_every_wire_keeps_the_session_and_the_edge_clearance() {
    let dsn = fast_board();
    let outline = dsn_outline_um(&std::fs::read_to_string(&dsn).unwrap());
    let plain = run(&dsn, json!({ EDGE_KEY: 500 }));
    let floored = run(&dsn, json!({ EDGE_KEY: 500, WIDTH_KEY: FLOOR_UM }));
    // every wire here is the 152.4 um rule width: a 140 um floor binds nowhere (the neck-down clamp
    // itself is unit-tested in fr-engine control.rs)
    assert_eq!(plain, floored);
    let c = copper(&floored, &outline);
    assert!(c.min_width_um >= f64::from(FLOOR_UM) - GEOMETRY_TOLERANCE_UM, "{:.1}", c.min_width_um);
    assert!(c.min_edge_gap_um >= 500.0 - GEOMETRY_TOLERANCE_UM);
}

#[test]
fn serve_session_equals_stock_cli_with_the_same_two_flags() {
    check_serve_equals_stock_with_both_flags(&fast_board());
}

// ------------------------------------------------------------------------------ tests (corpus board)

/// The same checks on the real hb200 corpus board.
mod hb200 {
    use super::*;

    #[test]
    #[ignore = "on-demand (scripts/pcbkit-bench.sh): routes the hb200 corpus board several times"]
    fn edge_clearance_changes_the_session_and_is_enforced_geometrically() {
        check_edge_clearance_changes_and_is_enforced(&board());
    }

    #[test]
    #[ignore = "on-demand (scripts/pcbkit-bench.sh): routes the hb200 corpus board several times"]
    fn an_edge_clearance_above_the_natural_gap_moves_copper_away_from_the_outline() {
        check_clearance_above_natural_gap_moves_copper(&board());
    }

    #[test]
    #[ignore = "on-demand (scripts/pcbkit-bench.sh): routes the hb200 corpus board several times"]
    fn min_trace_width_is_a_floor_on_every_exported_wire() {
        let dsn = board();
        let outline = dsn_outline_um(&std::fs::read_to_string(&dsn).unwrap());
        let natural = copper(&run(&dsn, json!({ EDGE_KEY: 500 })), &outline);
        let floored_ses = run(&dsn, json!({ EDGE_KEY: 500, WIDTH_KEY: FLOOR_UM }));
        let floored = copper(&floored_ses, &outline);
        eprintln!("narrowest wire: no floor {:.1} um, floor {FLOOR_UM} -> {:.1} um", natural.min_width_um, floored.min_width_um);
        assert!(natural.min_width_um >= 100.0 - GEOMETRY_TOLERANCE_UM);
        // the binding case: without the floor the router necks below FLOOR_UM, with it nothing does
        assert!(natural.min_width_um < f64::from(FLOOR_UM) - GEOMETRY_TOLERANCE_UM, "floor {FLOOR_UM} cannot bind: no wire is narrower");
        assert!(floored.min_width_um >= f64::from(FLOOR_UM) - GEOMETRY_TOLERANCE_UM, "wire of {:.1} um under a {FLOOR_UM} um floor", floored.min_width_um);
        assert_ne!(run(&dsn, json!({ EDGE_KEY: 500 })), floored_ses);
        assert!(floored.min_edge_gap_um >= 500.0 - GEOMETRY_TOLERANCE_UM);
    }

    #[test]
    #[ignore = "on-demand (scripts/pcbkit-bench.sh): routes the hb200 corpus board with the stock CLI and serve"]
    fn serve_session_equals_stock_cli_with_the_same_two_flags() {
        check_serve_equals_stock_with_both_flags(&board());
    }
}

#[test]
fn hello_echoes_applied_and_sorted_unknown_and_settings_change_the_cache_key() {
    let mut a = Server::start();
    let ha = a.hello_full(1, json!({ EDGE_KEY: 500, WIDTH_KEY: 100, "router.zzz": 1, "gui.aaa": true, "router.nope": "x" }));
    let (applied, unknown) = (ha["result"]["settings"]["applied"].clone(), ha["result"]["settings"]["unknown"].clone());
    assert_eq!(applied[EDGE_KEY], "500");
    assert_eq!(applied[WIDTH_KEY], "100");
    assert_eq!(unknown, json!(["gui.aaa", "router.nope", "router.zzz"]));
    a.finish();

    let mut b = Server::start();
    let hb = b.hello_full(1, json!({ EDGE_KEY: 200, WIDTH_KEY: 100 }));
    assert_eq!(hb["result"]["settings"]["applied"][EDGE_KEY], "200");
    assert_eq!(hb["result"]["settings"]["unknown"], json!([]));
    b.finish();

    // same build hash, different applied settings: the route cache key (build + settings) differs
    assert_eq!(ha["build"], hb["build"]);
    assert!(ha["build"].as_str().is_some_and(|s| s.len() == 64));
    assert_ne!(ha["result"]["settings"]["applied"], hb["result"]["settings"]["applied"]);
}

#[test]
fn defaults_are_upstream_when_the_client_sends_nothing() {
    let mut s = Server::start();
    let h = s.hello_full(1, json!({}));
    let applied = h["result"]["settings"]["applied"].as_object().unwrap();
    assert!(!applied.contains_key(EDGE_KEY) && !applied.contains_key(WIDTH_KEY), "{applied:?}");
    s.finish();
}
