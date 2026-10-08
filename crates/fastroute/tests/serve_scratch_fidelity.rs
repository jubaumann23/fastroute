//! `route` with `from: scratch` is placement-pure (SPEC 7): its result equals a fresh load of the same
//! placement plus one scratch route, whatever the session did before (earlier routes, other seeds, moves,
//! locks). Threads 1 and 2 on tiny, det/hb200 and det/energy-12-1.

#![allow(dead_code)]

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

fn text(s: &str) -> Value {
    json!({ "text": s })
}

fn corpus(key: &str) -> PathBuf {
    manifest().join("../../reference/pcbkit-corpus/det").join(key).join("board.dsn")
}

fn dsn_of(key: &str) -> PathBuf {
    if key == "tiny" {
        serve_data("tiny.dsn")
    } else {
        corpus(key)
    }
}

/// Where each board's part is moved (a two-pin passive; `carry: none` so that DSN-fixed fan-out stays
/// where the DSN put it, like in a fresh load of the rewritten place record).
const MOVED: &[(&str, &str)] = &[("tiny", "R4"), ("hb200", "C22"), ("energy-12-1", "C15")];

fn route_args(seed: i64, from: &str) -> Value {
    json!({ "seed": seed, "nets": "all", "from": from })
}

/// A server with `board` loaded from `dsn_text` (inline, so a rewritten copy is possible).
fn session(threads: u32, dsn_text: &str) -> Server {
    let mut s = Server::start();
    s.hello(threads, json!({}));
    s.ok("load", json!({ "dsn": text(dsn_text) }));
    s
}

/// SES of: fresh load, one scratch route. The reference every history must reproduce.
fn fresh(threads: u32, dsn_text: &str) -> String {
    let mut s = session(threads, dsn_text);
    s.ok("route", route_args(0, "scratch"));
    let ses = s.ses();
    s.finish();
    ses
}

/// The DSN with the place record of `name` moved by `dx` (DSN file units); returns the new text and
/// the protocol pose (x, y, rot) of the part (integer DSN units, resolution times file units).
fn moved_text(dsn: &str, name: &str, dx: f64) -> (String, (i64, i64, i64)) {
    let key = format!("(place {name} ");
    let at = dsn.find(&key).unwrap_or_else(|| panic!("no place record for {name}"));
    let line_end = at + dsn[at..].find('\n').unwrap();
    let fields: Vec<&str> = dsn[at + key.len()..line_end].split_whitespace().collect();
    let (x, y): (f64, f64) = (fields[0].parse().unwrap(), fields[1].parse().unwrap());
    let rot: i64 = fields[3].parse().unwrap();
    assert_eq!(fields[2], "front");
    let new_x = format!("{}", x + dx);
    let tail = &dsn[at + key.len()..line_end];
    let rest = &tail[tail.find("front").unwrap()..];
    let rewritten = format!("{}{key}{new_x} {y} {rest}{}", &dsn[..at], &dsn[line_end..]);
    let res = 10.0; // every corpus board here is `(resolution um 10)`
    (rewritten, (((x + dx) * res).round() as i64, (y * res).round() as i64, rot.rem_euclid(360)))
}

fn check_board(key: &str, threads: u32) {
    let dsn = std::fs::read_to_string(dsn_of(key)).unwrap();
    assert!(dsn.contains("(resolution um 10)"));
    let reference = fresh(threads, &dsn);
    assert!(reference.contains("(network_out"));

    // (a) load, route, then route scratch
    let mut s = session(threads, &dsn);
    s.ok("route", route_args(0, "current"));
    s.ok("route", route_args(0, "scratch"));
    assert_eq!(s.ses(), reference, "{key} t{threads}: (a) scratch after route differs from a fresh load");
    s.finish();

    // (b) load, route seed 5, then route scratch seed 0
    let mut s = session(threads, &dsn);
    s.ok("route", route_args(5, "current"));
    s.ok("route", route_args(0, "scratch"));
    assert_eq!(s.ses(), reference, "{key} t{threads}: (b) scratch after seed 5 differs from a fresh load");
    s.finish();

    // (c) load, route, move a part, route scratch == fresh load of the rewritten DSN, scratch route
    let (name, dx) = (MOVED.iter().find(|(k, _)| *k == key).unwrap().1, 500.0);
    let (rewritten, (x, y, rot)) = moved_text(&dsn, name, dx);
    let mut s = session(threads, &dsn);
    s.ok("route", route_args(0, "current"));
    s.ok("move", json!({ "moves": [{ "ref": name, "x": x, "y": y, "rot": rot, "side": "front" }], "carry": "none" }));
    s.ok("route", route_args(0, "scratch"));
    let after_move = s.ses();
    s.finish();
    assert_eq!(after_move, fresh(threads, &rewritten), "{key} t{threads}: (c) scratch after a move differs from a fresh load of the moved placement");
    assert_ne!(after_move, reference, "{key}: the move must change the result, else the test proves nothing");
}

/// The wiring text of one net of a session file.
fn net_block(ses: &str, net: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for line in ses[ses.find("(network_out").unwrap()..].lines().skip(1) {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("(net ") {
            inside = rest.trim().trim_matches('"') == net;
        } else if line == "    )" {
            inside = false;
        }
        if inside {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// (d) N1 locked: the locked wiring stays byte-identical and the rest equals a route with that
/// wiring locked (a session that routed only N1 before locking it; DSN `fix` wires are not the reference:
/// they route the other nets differently from locked wiring).
fn check_locked_tiny(threads: u32) {
    let dsn = std::fs::read_to_string(dsn_of("tiny")).unwrap();
    let mut s = session(threads, &dsn);
    s.ok("route", route_args(0, "current"));
    let routed = s.ses();
    s.ok("lock", json!({ "nets": ["N1"] }));
    s.ok("route", route_args(0, "scratch"));
    let after = s.ses();
    s.finish();
    assert_eq!(net_block(&after, "N1"), net_block(&routed, "N1"), "t{threads}: locked N1 wiring changed");
    assert!(!net_block(&after, "N1").is_empty());

    // the reference reaches the same locked N1 wiring without ever routing the other nets before
    let mut r = session(threads, &dsn);
    r.ok("route", json!({ "seed": 0, "nets": ["N1"], "from": "current" }));
    assert_eq!(net_block(&r.ses(), "N1"), net_block(&routed, "N1"), "t{threads}: N1 routes the same alone, else this reference is void");
    r.ok("lock", json!({ "nets": ["N1"] }));
    r.ok("route", route_args(0, "scratch"));
    let expected = r.ses();
    r.finish();
    assert_eq!(after, expected, "t{threads}: (d) locked scratch differs from a fresh route with N1 locked");
}

#[test]
fn tiny_t1() {
    check_board("tiny", 1);
}

#[test]
fn tiny_t2() {
    check_board("tiny", 2);
}

#[test]
fn tiny_locked_net_t1() {
    check_locked_tiny(1);
}

#[test]
fn tiny_locked_net_t2() {
    check_locked_tiny(2);
}

#[test]
fn hb200_t1() {
    check_board("hb200", 1);
}

#[test]
fn hb200_t2() {
    check_board("hb200", 2);
}

#[test]
fn energy_12_1_t1() {
    check_board("energy-12-1", 1);
}

#[test]
fn energy_12_1_t2() {
    check_board("energy-12-1", 2);
}
