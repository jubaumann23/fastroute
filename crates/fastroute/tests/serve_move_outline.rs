//! `fastroute serve` op `move`: a pose that puts the part's pads outside the bounding box of the
//! board outline is `bad_request` (SPEC 5.3); the request is atomic and the session stays untouched.
//!
//! Lives in the CLI crate (not fr-serve/tests) because only here the `fastroute` binary is built
//! (`CARGO_BIN_EXE_fastroute`); fr-serve keeps its protocol modules private.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use fr_serve::serde_json::{json, Value};

const BIN: &str = env!("CARGO_BIN_EXE_fastroute");

fn tiny() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../fr-serve/tests/data/tiny.dsn")
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
        let line = json!({ "id": id, "op": op, "args": args }).to_string();
        let stdin = self.stdin.as_mut().unwrap();
        stdin.write_all(line.as_bytes()).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
        let mut resp = String::new();
        assert!(self.stdout.read_line(&mut resp).unwrap() > 0, "server closed stdout after {line}");
        let r: Value = fr_serve::serde_json::from_str(&resp).unwrap();
        assert_eq!(r["id"], json!(id), "{r}");
        r
    }

    fn ok(&mut self, op: &str, args: Value) -> Value {
        let r = self.call(op, args);
        assert_eq!(r["ok"], json!(true), "{op}: {r}");
        r["result"].clone()
    }

    fn bad_request(&mut self, args: Value) -> Value {
        let r = self.call("move", args);
        assert_eq!(r["ok"], json!(false), "expected bad_request, got {r}");
        assert_eq!(r["error"]["code"], json!("bad_request"), "{r}");
        r["error"].clone()
    }

    fn ses(&mut self) -> String {
        self.ok("export", json!({ "format": "ses" }))["text"].as_str().unwrap().to_string()
    }

    fn finish(mut self) {
        self.call("shutdown", json!({}));
        drop(self.stdin.take());
        assert!(self.child.wait().unwrap().success());
    }
}

fn session(dsn: &Path) -> Server {
    let mut s = Server::start();
    s.ok("hello", json!({ "protocol": "1.0.0", "client": "move_outline", "threads": 1, "settings": {} }));
    s.ok("load", json!({ "dsn": { "path": dsn.canonicalize().unwrap().to_str().unwrap() } }));
    s
}

fn mv(name: &str, x: i64, y: i64) -> Value {
    json!({ "ref": name, "x": x, "y": y, "rot": 0, "side": "front" })
}

const SCRATCH: fn() -> Value = || json!({ "seed": 0, "nets": "all", "from": "scratch" });

/// tiny.dsn with the outline widened so that the origin lies inside the board.
fn origin_board(dir: &Path) -> PathBuf {
    let text = std::fs::read_to_string(tiny()).unwrap();
    let old = "(path pcb 0  30000 -25000  10000 -25000  10000 -10000  30000 -10000\n            30000 -25000)";
    assert!(text.contains(old));
    let new = "(path pcb 0  30000 -25000  -5000 -25000  -5000 5000  30000 5000\n            30000 -25000)";
    let p = dir.join("origin.dsn");
    std::fs::write(&p, text.replace(old, new)).unwrap();
    p
}

#[test]
fn a_pose_ten_board_widths_away_is_rejected_and_the_session_is_unchanged() {
    let mut s = session(&tiny());
    let routed = s.ok("route", SCRATCH());
    let ses = s.ses();
    let load_boundary = {
        // the board is 200000 units wide (10000..30000 um at 0.1 um)
        json!([100000, -250000, 300000, -100000])
    };
    let e = s.bad_request(json!({ "moves": [mv("R4", 2_270_000, -210_000)] }));
    assert!(e["message"].as_str().unwrap().contains("R4"), "{e}");
    assert_eq!(e["details"]["ref"], json!("R4"), "{e}");
    assert_eq!(e["details"]["boundary"], load_boundary, "{e}");
    let bbox = e["details"]["bbox"].as_array().unwrap();
    assert_eq!(bbox.len(), 4, "{e}");
    assert!(bbox[0].as_i64().unwrap() > 2_000_000, "bbox is the pad box at the requested pose: {e}");
    assert_eq!(s.ses(), ses, "a rejected move must leave the wiring as it was");
    let again = s.ok("route", json!({ "seed": 0, "nets": "all", "from": "current" }));
    assert_eq!(again["unrouted"], routed["unrouted"], "{again}");
    s.finish();
}

#[test]
fn a_unit_slip_down_lands_inside_a_board_that_holds_the_origin_but_a_slip_up_is_rejected() {
    let dir = std::env::temp_dir().join(format!("move_outline_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dsn = origin_board(&dir);
    let mut s = session(&dsn);
    // 27000 um sent as 0.1 um units is 10x too small: it lands inside the board and is accepted
    let r = s.ok("move", json!({ "moves": [mv("R4", 2_700, -2_100)] }));
    assert_eq!(r["moved"], 1, "{r}");
    // 10x too large: 270 mm away
    let e = s.bad_request(json!({ "moves": [mv("R4", 2_700_000, -21_000)] }));
    assert_eq!(e["details"]["ref"], json!("R4"), "{e}");
    s.finish();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_one_millimetre_nudge_still_succeeds() {
    let mut s = session(&tiny());
    s.ok("route", SCRATCH());
    // R4 sits at (260000, -210000); 1 mm = 10000 units
    let r = s.ok("move", json!({ "moves": [mv("R4", 250_000, -210_000)] }));
    assert_eq!(r["moved"], 1, "{r}");
    s.finish();
}

#[test]
fn one_bad_part_in_a_request_moves_none() {
    let mut s = session(&tiny());
    s.ok("route", SCRATCH());
    let ses = s.ses();
    let e = s.bad_request(json!({ "moves": [mv("R4", 250_000, -210_000), mv("R3", 9_000_000, -210_000)] }));
    assert_eq!(e["details"]["ref"], json!("R3"), "{e}");
    assert_eq!(s.ses(), ses, "R4 must not have moved either");
    s.finish();
}
