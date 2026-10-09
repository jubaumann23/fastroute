//! `fastroute serve` capability `move` (router protocol 1.0, SPEC 5.3): validation, rip-up report,
//! exact place records, atomicity, and move fidelity (a moved board routes exactly like a fresh load
//! of the DSN with the edited place record).

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use fr_serve::serde_json::{json, Value};

const BIN: &str = env!("CARGO_BIN_EXE_fastroute");

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn serve_data(name: &str) -> PathBuf {
    manifest().join("../fr-serve/tests/data").join(name)
}

struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl Server {
    fn start() -> Server {
        Self::start_with(&[])
    }

    fn start_with(env: &[(&str, &str)]) -> Server {
        let mut cmd = Command::new(BIN);
        cmd.arg("serve").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().expect("spawn fastroute serve");
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Server { child, stdin, stdout, next_id: 1 }
    }

    fn raw(&mut self, line: &str) -> Value {
        let stdin = self.stdin.as_mut().unwrap();
        stdin.write_all(line.as_bytes()).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
        let mut resp = String::new();
        assert!(self.stdout.read_line(&mut resp).unwrap() > 0, "server closed stdout after {line}");
        assert!(resp.ends_with('\n') && !resp[..resp.len() - 1].contains('\n'));
        fr_serve::serde_json::from_str(&resp).unwrap()
    }

    fn call(&mut self, op: &str, args: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let r = self.raw(&json!({ "id": id, "op": op, "args": args }).to_string());
        assert_eq!(r["id"], json!(id), "{r}");
        assert!(r["build"].as_str().is_some_and(|b| b.len() == 64), "{r}");
        r
    }

    fn ok(&mut self, op: &str, args: Value) -> Value {
        let r = self.call(op, args);
        assert_eq!(r["ok"], json!(true), "{op}: {r}");
        r["result"].clone()
    }

    fn err(&mut self, op: &str, args: Value, code: &str) -> Value {
        let r = self.call(op, args);
        assert_eq!(r["ok"], json!(false), "{op}: expected {code}, got {r}");
        assert_eq!(r["error"]["code"], json!(code), "{r}");
        assert!(r["error"]["message"].is_string());
        r["error"].clone()
    }

    fn hello(&mut self, threads: u32, settings: Value) -> Value {
        self.ok("hello", json!({ "protocol": "1.0.0", "client": "serve_baseline", "threads": threads, "settings": settings }))
    }

    fn ses(&mut self) -> String {
        self.ok("export", json!({ "format": "ses" }))["text"].as_str().unwrap().to_string()
    }

    fn finish(mut self) {
        let r = self.call("shutdown", json!({}));
        assert_eq!(r["result"], json!({}));
        drop(self.stdin.take());
        assert!(self.child.wait().unwrap().success());
    }
}

fn path(p: &Path) -> Value {
    json!({ "path": p.canonicalize().unwrap().to_str().unwrap() })
}


fn stable(mut result: Value) -> Value {
    result.as_object_mut().unwrap().remove("wall_ms");
    result
}

fn corpus(rel: &str) -> PathBuf {
    let p = manifest().join("../../reference/pcbkit-corpus").join(rel);
    assert!(p.exists(), "corpus file {} is missing (see docs/PCBKIT.md, shared reference/)", p.display());
    p.canonicalize().unwrap()
}

/// `(place REF x y side rot` of a DSN: (x, y, rot) as written.
fn place_record(dsn: &str, name: &str) -> (f64, f64, f64, usize, usize) {
    let key = format!("(place {name} ");
    let at = dsn.find(&key).unwrap_or_else(|| panic!("no place record for {name}"));
    let rest = &dsn[at + key.len()..];
    let toks: Vec<&str> = rest.split_whitespace().take(4).collect();
    let rot_tok = toks[3].trim_end_matches(')');
    let end = at + key.len() + rest.find(rot_tok).unwrap() + rot_tok.len();
    (toks[0].parse().unwrap(), toks[1].parse().unwrap(), rot_tok.parse().unwrap(), at, end)
}

/// The DSN text with REF's pose replaced (file units; the board is `unit um`, `resolution um 10`).
fn edited_dsn(dsn: &str, name: &str, x: f64, y: f64, rot: f64) -> String {
    let (_, _, _, at, end) = place_record(dsn, name);
    let side = if dsn[at..end].contains(" back ") { "back" } else { "front" };
    format!("{}(place {name} {x} {y} {side} {rot}{}", &dsn[..at], &dsn[end..])
}

fn run_session(dsn: &Path, mut script: impl FnMut(&mut Server)) {
    let mut s = Server::start();
    s.hello(1, json!({}));
    s.ok("load", json!({ "dsn": path(dsn) }));
    script(&mut s);
    s.finish();
}

const SCRATCH: fn() -> Value = || json!({ "seed": 0, "nets": "all", "from": "scratch" });

#[test]
fn tiny_move_rips_connected_nets_and_places_exactly() {
    run_session(&serve_data("tiny.dsn"), |s| {
        s.ok("route", SCRATCH());
        let before = s.ses();
        let want = json!({ "ref": "R4", "x": 270000, "y": -210000, "rot": 0, "side": "front" });
        let r = s.ok("move", json!({ "moves": [want] }));
        assert_eq!(r["moved"], 1, "{r}");
        let nets: Vec<&str> = r["ripped"]["nets"].as_array().unwrap().iter().map(|n| n.as_str().unwrap()).collect();
        assert!(!nets.is_empty() && nets.iter().all(|n| ["N3", "N4"].contains(n)), "{r}");
        assert_eq!(nets, ["N3", "N4"], "network order");
        assert!(r["ripped"]["wires"].as_i64().unwrap() > 0);
        assert_eq!(r["carried"], json!({ "wires": 0, "vias": 0 }));
        assert_eq!(r["unrouted"], 2, "{r}");
        let after = s.ses();
        assert!(after.contains("(place R4 270000 -210000 front 0)"), "{after}");
        assert!(before.contains("(place R4 260000 -210000 front 0)"), "{before}");
        assert!(!after.contains("(net N3") && !after.contains("(net N4"), "ripped nets must have no wiring");
        assert!(after.contains("(net N1") && after.contains("(net N2"), "other nets keep their wiring");
    });
}

#[test]
fn failed_moves_leave_the_board_byte_identical() {
    run_session(&serve_data("tiny.dsn"), |s| {
        s.ok("route", SCRATCH());
        let ses = s.ses();
        let r4 = json!({ "ref": "R4", "x": 270000, "y": -210000, "rot": 0, "side": "front" });
        let with = |patch: Value| {
            let mut m = r4.clone();
            for (k, v) in patch.as_object().unwrap() {
                m[k] = v.clone();
            }
            m
        };
        s.err("move", json!({ "moves": [r4, with(json!({ "ref": "R9" }))] }), "unknown_ref");
        s.err("move", json!({ "moves": [with(json!({ "rot": 360 }))] }), "bad_request");
        s.err("move", json!({ "moves": [with(json!({ "rot": -1 }))] }), "bad_request");
        s.err("move", json!({ "moves": [with(json!({ "side": "back" }))] }), "bad_request");
        s.err("move", json!({ "moves": [with(json!({ "side": "top" }))] }), "bad_request");
        s.err("move", json!({ "moves": [r4, r4] }), "bad_request");
        s.err("move", json!({ "moves": [] }), "bad_request");
        s.err("move", json!({ "moves": [with(json!({ "x": 1.5 }))] }), "bad_request");
        s.err("move", json!({ "moves": [{ "ref": "R4" }] }), "bad_request");
        s.err("move", json!({ "moves": [r4], "carry": "some" }), "bad_request");
        s.err("move", json!({ "moves": [r4], "bogus": 1 }), "bad_request");
        assert_eq!(s.ses(), ses, "a refused move must not change the board");
    });
}

#[test]
fn two_moves_apply_together_and_validate_first() {
    run_session(&serve_data("tiny.dsn"), |s| {
        s.ok("route", SCRATCH());
        let r = s.ok(
            "move",
            json!({ "moves": [
                { "ref": "R4", "x": 270000, "y": -210000, "rot": 0, "side": "front" },
                { "ref": "R3", "x": 140000, "y": -210000, "rot": 270, "side": "front" } ] }),
        );
        assert_eq!(r["moved"], 2, "{r}");
        let ses = s.ses();
        assert!(ses.contains("(place R4 270000 -210000 front 0)") && ses.contains("(place R3 140000 -210000 front 270)"), "{ses}");
    });
}

/// move, then route from scratch  ==  fresh load of the DSN with the edited place record, then route.
fn assert_move_equals_reload(label: &str, dsn: &Path, name: &str, dx_units: i64, dy_units: i64, new_rot: Option<i64>, pre_route: bool) {
    let text = std::fs::read_to_string(dsn).unwrap();
    assert!(text.contains("(resolution um 10)") && text.contains("(unit um)"), "{label}: test assumes um/10");
    let (x, y, rot, _, _) = place_record(&text, name);
    let rot_n = (rot as i64).rem_euclid(360);
    let new_rot_n = new_rot.unwrap_or(rot_n);
    let edited_text = edited_dsn(&text, name, x + dx_units as f64 / 10.0, y + dy_units as f64 / 10.0, new_rot_n as f64);
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("serve-move-{}-{label}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let edited = dir.join(dsn.file_name().unwrap());
    std::fs::write(&edited, edited_text).unwrap();

    let moved = {
        let mut out = None;
        run_session(dsn, |s| {
            if pre_route {
                s.ok("route", SCRATCH());
            }
            let r = s.ok(
                "move",
                json!({ "moves": [{ "ref": name, "x": ((x * 10.0).round() as i64 + dx_units), "y": ((y * 10.0).round() as i64 + dy_units),
                                    "rot": new_rot_n, "side": "front" }], "carry": "none" }),
            );
            assert_eq!(r["moved"], 1, "{label}: {r}");
            let route = s.ok("route", SCRATCH());
            out = Some((stable(route), s.ses()));
        });
        out.unwrap()
    };
    let reloaded = {
        let mut out = None;
        run_session(&edited, |s| {
            let route = s.ok("route", SCRATCH());
            out = Some((stable(route), s.ses()));
        });
        out.unwrap()
    };
    assert_eq!(moved.0, reloaded.0, "{label}: route result differs");
    assert!(moved.1 == reloaded.1, "{label}: SES differs between move+route and reload+route");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn move_then_route_equals_reload_tiny() {
    assert_move_equals_reload("tiny", &serve_data("tiny.dsn"), "R4", 10000, 0, None, true);
    assert_move_equals_reload("tiny-rot", &serve_data("tiny.dsn"), "R3", 0, 10000, Some(0), true);
}

#[test]
#[ignore = "on-demand (scripts/pcbkit-bench.sh): routes the energy-8 corpus board twice"]
fn move_then_route_equals_reload_corpus_energy8() {
    assert_move_equals_reload("energy8", &corpus("runs/energy-8/base/layout/.route/board.dsn"), "R4", 10000, 0, None, false);
}

#[test]
#[ignore = "on-demand (scripts/pcbkit-bench.sh): routes the heuristic-baseline corpus board twice"]
fn move_then_route_equals_reload_corpus_heuristic() {
    assert_move_equals_reload("heuristic", &corpus("runs/heuristic-baseline/base/layout/.route/board.dsn"), "R6", 10000, -5000, Some(180), false);
}

// ---------------------------------------------------------------------------------------------
// F2 (docs/PCBKIT.md): move, then route from the current board at 2+ threads used to livelock.

/// Kills the server if the test does not finish within `secs`, so a hang fails (the next read sees EOF).
struct Watchdog {
    done: std::sync::mpsc::Sender<()>,
}

impl Watchdog {
    fn start(pid: u32, secs: u64) -> Watchdog {
        let (done, rx) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            if rx.recv_timeout(std::time::Duration::from_secs(secs)) == Err(std::sync::mpsc::RecvTimeoutError::Timeout) {
                eprintln!("watchdog: server {pid} did not answer within {secs} s, killing it");
                let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
            }
        });
        Watchdog { done }
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        let _ = self.done.send(());
    }
}

const HANG_SECS: u64 = 240;

/// `unit_scale` converts the DSN file unit to board resolution units (tiny.dsn: um, resolution um 10).
/// Scratch route, move R2 and C18 by 1000 units in x, route nets:all from:current; the SES afterwards.
fn move_two_then_route_current(dsn: &Path, threads: u32, refs: [&str; 2], unit_scale: f64) -> String {
    let text = std::fs::read_to_string(dsn).unwrap();
    let mut s = Server::start();
    let _wd = Watchdog::start(s.child.id(), HANG_SECS);
    s.hello(threads, json!({}));
    s.ok("load", json!({ "dsn": path(dsn) }));
    s.ok("route", SCRATCH());
    for name in refs {
        let (x, y, rot, at, end) = place_record(&text, name);
        let side = if text[at..end].contains(" back ") { "back" } else { "front" };
        let mv = json!({ "ref": name, "x": (x * unit_scale).round() as i64 + 1000, "y": (y * unit_scale).round() as i64, "rot": rot.round() as i64 % 360, "side": side });
        s.ok("move", json!({ "moves": [mv] }));
    }
    let r = s.ok("route", json!({ "seed": 0, "nets": "all", "from": "current" }));
    assert!(r["wall_ms"].is_i64(), "{r}");
    let ses = s.ses();
    s.finish();
    ses
}

#[test]
#[ignore = "on-demand (scripts/pcbkit-bench.sh): routes the energy-12-1 corpus board 8 times at 2 and 4 threads"]
fn move_then_route_from_current_returns_at_threads_2_and_4_and_is_deterministic() {
    let dsn = corpus("det/energy-12-1/board.dsn");
    for threads in [2, 4] {
        let a = move_two_then_route_current(&dsn, threads, ["R2", "C18"], 10.0);
        let b = move_two_then_route_current(&dsn, threads, ["R2", "C18"], 10.0);
        assert!(a == b, "two server processes differ at threads {threads} after move + route from current");
    }
}

#[test]
fn a_worker_panic_is_an_internal_error_not_a_hang() {
    let dsn = serve_data("tiny.dsn");
    for (threads, skip) in [(1, "4"), (2, "4")] {
        let mut s = Server::start_with(&[("FR_ENGINE_TEST_PANIC_ROOMS", skip)]);
        let _wd = Watchdog::start(s.child.id(), HANG_SECS);
        s.hello(threads, json!({}));
        s.ok("load", json!({ "dsn": path(&dsn) }));
        let before = s.ses();
        let e = s.err("route", SCRATCH(), "internal");
        assert!(e["message"].as_str().unwrap().contains("route"), "{e}");
        assert_eq!(s.ses(), before, "a failed route leaves the session board unchanged");
        s.finish();
    }
}

/// Fast stand-in for the energy-12-1 F2 test: move two parts, route from the current board at 2 and 4
/// threads; it returns (no livelock) and two server processes agree.
#[test]
fn move_then_route_from_current_returns_and_is_deterministic_on_tiny() {
    let dsn = serve_data("tiny.dsn");
    for threads in [2, 4] {
        let a = move_two_then_route_current(&dsn, threads, ["R3", "R4"], 10.0);
        let b = move_two_then_route_current(&dsn, threads, ["R3", "R4"], 10.0);
        assert!(a == b, "two server processes differ at threads {threads} after move + route from current");
    }
}
