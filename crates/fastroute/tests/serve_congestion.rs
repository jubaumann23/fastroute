//! `fastroute serve` capability `congestion` (SPEC 5.7): grid shape, the capacity rule on a hand-checked
//! two-cell board, demand and airwire before/after a route, and determinism across processes.

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
        self.ok("hello", json!({ "protocol": "1.0.0", "client": "serve_congestion", "threads": threads, "settings": settings }))
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

/// Two 1000-unit cells side by side (um, resolution 1): width 100, clearance 100, so pitch 200. A copper
/// keepout on F.Cu in the right cell, x 1300..1700 and y 300..700 (grown by 100: 1200..1800, 200..800).
const TWO_CELLS: &str = r#"(pcb two_cells
  (parser (string_quote ") (space_in_quoted_tokens on) (host_cad "t") (host_version "1"))
  (resolution um 1)
  (unit um)
  (structure
    (layer F.Cu (type signal) (property (index 0)))
    (layer B.Cu (type signal) (property (index 1)))
    (boundary (path pcb 0 0 0 2000 0 2000 1000 0 1000 0 0))
    (keepout "" (rect F.Cu 1300 300 1700 700))
    (rule (width 100) (clearance 100))
  )
  (placement)
  (library)
  (network)
  (wiring)
)
"#;

fn started() -> Server {
    let mut s = Server::start();
    let h = s.hello(1, json!({}));
    assert!(h["capabilities"].as_array().unwrap().contains(&json!("congestion")), "{h}");
    s
}

fn ints(v: &Value) -> Vec<i64> {
    v.as_array().unwrap().iter().map(|n| n.as_i64().unwrap()).collect()
}

fn layer<'a>(g: &'a Value, name: &str) -> &'a Value {
    g["layers"].as_array().unwrap().iter().find(|l| l["name"] == name).unwrap()
}

#[test]
fn two_cell_board_capacity_by_hand() {
    let mut s = started();
    let load = s.ok("load", json!({ "dsn": { "text": TWO_CELLS } }));
    assert_eq!(load["boundary"], json!([0, 0, 2000, 1000]), "{load}");
    let g = s.ok("congestion", json!({ "cell": 1000, "layers": "all" }));
    assert_eq!((&g["origin"], &g["cols"], &g["rows"], &g["cell"]), (&json!([0, 0]), &json!(2), &json!(1), &json!(1000)));
    // left cell: x = 500 and y = 500 are free over 1000 units: 5 + 5 tracks of pitch 200 on both layers;
    // right cell on F.Cu: x = 1500 leaves [0,200] and [800,1000] (1 + 1), y = 500 leaves [1000,1200] and
    // [1800,2000] (1 + 1) = 4; on B.Cu the keepout is absent: 10.
    assert_eq!(ints(&layer(&g, "F.Cu")["capacity"]), vec![10, 4]);
    assert_eq!(ints(&layer(&g, "B.Cu")["capacity"]), vec![10, 10]);
    assert_eq!(ints(&layer(&g, "F.Cu")["demand"]), vec![0, 0]);
    assert_eq!(ints(&layer(&g, "F.Cu")["airwire"]), vec![0, 0]);
    // a requested subset keeps the requested order; an unknown layer is a bad_request naming it
    let g2 = s.ok("congestion", json!({ "cell": 1000, "layers": ["B.Cu", "F.Cu"] }));
    let names: Vec<&str> = g2["layers"].as_array().unwrap().iter().map(|l| l["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["B.Cu", "F.Cu"]);
    let e = s.err("congestion", json!({ "cell": 1000, "layers": ["Nope"] }), "bad_request");
    assert_eq!(e["details"]["layer"], "Nope");
    s.err("congestion", json!({ "cell": 0, "layers": "all" }), "bad_request");
    s.err("congestion", json!({ "cell": 1000 }), "bad_request");
    s.finish();
}

#[test]
fn cells_outside_the_boundary_have_no_capacity() {
    let mut s = started();
    let load = s.ok("load", json!({ "dsn": path(&serve_data("tiny.dsn")) }));
    let [x1, y1, x2, y2] = ints(&load["boundary"])[..] else { panic!() };
    let cell = (x2 - x1).max(y2 - y1) * 7 / 10; // two cells each way, the second hangs out
    let g = s.ok("congestion", json!({ "cell": cell, "layers": "all" }));
    assert_eq!((g["cols"].as_i64(), g["rows"].as_i64()), (Some(2), Some(2)));
    for l in g["layers"].as_array().unwrap() {
        let cap = ints(&l["capacity"]);
        assert!(cap[0] > 0, "{l}");
        // cell (1,1): both centre lines lie outside the boundary
        assert_eq!(cap[3], 0, "{l}");
    }
    s.finish();
}

#[test]
fn demand_rises_and_airwires_fall_after_a_route() {
    let mut s = started();
    let load = s.ok("load", json!({ "dsn": path(&serve_data("tiny.dsn")) }));
    let cell = 1000 * 10; // 1 mm in 0.1 um units
    let before = s.ok("congestion", json!({ "cell": cell, "layers": "all" }));
    let sum = |g: &Value, key: &str| -> i64 { g["layers"].as_array().unwrap().iter().map(|l| ints(&l[key]).iter().sum::<i64>()).sum() };
    assert_eq!(sum(&before, "demand"), 0);
    assert!(sum(&before, "airwire") > 0);
    let n = (before["cols"].as_i64().unwrap() * before["rows"].as_i64().unwrap()) as usize;
    for l in before["layers"].as_array().unwrap() {
        for k in ["capacity", "demand", "airwire"] {
            assert_eq!(l[k].as_array().unwrap().len(), n);
        }
    }
    assert!(load["unrouted"].as_i64().unwrap() > 0);
    let r = s.ok("route", json!({ "seed": 0 }));
    assert_eq!(r["unrouted"], 0, "{r}");
    let after = s.ok("congestion", json!({ "cell": cell, "layers": "all" }));
    assert!(sum(&after, "demand") > 0);
    assert_eq!(sum(&after, "airwire"), 0);
    s.finish();
}

#[test]
fn identical_across_processes() {
    let run = || {
        let mut s = started();
        s.ok("load", json!({ "dsn": path(&serve_data("tiny.dsn")) }));
        s.ok("route", json!({ "seed": 0 }));
        let g = s.ok("congestion", json!({ "cell": 5000, "layers": "all" }));
        s.finish();
        g
    };
    assert_eq!(run(), run());
}
