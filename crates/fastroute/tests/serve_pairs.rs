//! `fastroute serve` route.pairs / route.tune (router protocol capabilities `pairs` and `tune`): differential
//! pair coupling, pair skew matching and length matching groups run by `route`, on the small
//! synthetic board `pairs.dsn` (two parallel 20 mm nets, two nets of different length).

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
        self.ok("hello", json!({ "protocol": "1.0.0", "client": "serve_pairs", "threads": threads, "settings": settings }))
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

fn session(threads: u32) -> Server {
    let mut s = Server::start();
    let h = s.hello(threads, json!({}));
    let caps = h["capabilities"].as_array().unwrap();
    assert!(caps.contains(&json!("pairs")) && caps.contains(&json!("tune")), "{h}");
    s.ok("load", json!({ "dsn": path(&serve_data("pairs.dsn")) }));
    s
}

fn pair_args() -> Value {
    json!({
        "seed": 0, "starts": 1,
        "pairs": [{ "p": "USB_P", "n": "USB_N", "gap_mm": 0.5, "skew_mm": 0.1 }],
        "tune": [{ "name": "t", "nets": ["T*"], "tolerance_mm": 0.5 }]
    })
}

fn row<'a>(rows: &'a Value, key: &str, val: &str) -> &'a Value {
    rows.as_array().unwrap().iter().find(|r| r[key] == json!(val)).unwrap_or_else(|| panic!("no {key}={val} in {rows}"))
}

#[test]
fn without_the_arguments_the_result_has_no_pairs_or_tune() {
    let mut s = session(1);
    let r = s.ok("route", json!({ "seed": 0, "starts": 1 }));
    assert!(r.get("pairs").is_none() && r.get("tune").is_none(), "{r}");
    s.finish();
}

#[test]
fn pairs_and_groups_are_reported() {
    let mut s = session(1);
    let r = s.ok("route", pair_args());
    assert_eq!(r["complete"], json!(true), "{r}");
    let p = row(&r["pairs"], "p", "USB_P");
    assert_eq!(p["n"], json!("USB_N"));
    assert_eq!(p["tolerance_mm"], json!(0.1));
    for k in ["status", "kept", "coupled_mm", "p_length_mm", "n_length_mm", "skew_mm", "skew_before_mm", "skew_ok", "message"] {
        assert!(p.get(k).is_some(), "pair row lacks {k}: {p}");
    }
    assert!(p["p_length_mm"].as_f64().unwrap() > 10.0, "{p}");
    // the two nets run side by side for most of their length
    assert_eq!(p["kept"], json!(true), "{p}");
    assert!(p["coupled_mm"].as_f64().unwrap() > 5.0, "{p}");
    // skew_ok agrees with the numbers
    let skew = p["skew_mm"].as_f64().unwrap();
    assert_eq!(p["skew_ok"], json!(skew.abs() <= 0.1 + 1e-6), "{p}");

    assert!(r.get("pairs").is_some() && r["tune"].is_array());
    let g = row(&r["tune"], "name", "t");
    assert_eq!(g["status"], json!("tuned"));
    let nets = g["nets"].as_array().unwrap();
    assert_eq!(nets.len(), 2, "{g}");
    for n in nets {
        assert!(n["after_mm"].as_f64().unwrap() >= n["before_mm"].as_f64().unwrap() - 1e-6, "{n}");
    }
    // without target the longest net (T2) is the target, and T1 gained length
    let t2 = row(&g["nets"], "net", "T2");
    assert!((g["target_mm"].as_f64().unwrap() - t2["before_mm"].as_f64().unwrap()).abs() < 1e-6, "{g}");
    let t1 = row(&g["nets"], "net", "T1");
    assert!(t1["after_mm"].as_f64().unwrap() > t1["before_mm"].as_f64().unwrap() + 1.0, "{t1}");
    assert_eq!(g["short"], json!(nets.iter().filter(|n| n["short"] == json!(true)).count()));
    s.finish();
}

#[test]
fn results_are_deterministic() {
    let run = || {
        let mut s = session(1);
        let r = s.ok("route", pair_args());
        let ses = s.ses();
        s.finish();
        (stable(r), ses)
    };
    let (ra, sa) = run();
    let (rb, sb) = run();
    assert_eq!(ra, rb);
    assert_eq!(sa, sb);
}

#[test]
fn tune_without_pairs_and_unmatched_groups() {
    let mut s = session(1);
    let r = s.ok("route", json!({
        "seed": 0, "starts": 1,
        "tune": [{ "name": "none", "nets": ["NOPE*"] }, { "name": "fixed", "nets": ["T1"], "target_mm": 12.0, "tolerance_mm": 0.3 }]
    }));
    assert!(r.get("pairs").is_none(), "{r}");
    assert_eq!(row(&r["tune"], "name", "none")["status"], json!("no_match"));
    let g = row(&r["tune"], "name", "fixed");
    assert_eq!(g["target_mm"], json!(12.0));
    s.finish();
}

#[test]
fn locked_and_unlisted_nets_are_skipped() {
    let mut s = session(1);
    s.ok("route", json!({ "seed": 0, "starts": 1 }));
    s.ok("lock", json!({ "nets": ["USB_P"] }));
    let r = s.ok("route", json!({
        "seed": 0, "starts": 1, "from": "current",
        "pairs": [{ "p": "USB_P", "n": "USB_N" }],
        "tune": [{ "name": "t", "nets": ["T*"] }]
    }));
    let p = row(&r["pairs"], "p", "USB_P");
    assert_eq!((p["status"].clone(), p["kept"].clone(), p["message"].clone()), (json!("skipped"), json!(false), json!("locked")), "{p}");
    s.ok("unlock", json!({ "nets": ["USB_P"] }));
    // only T1 is in the request: T2 is the target but is never changed
    let r = s.ok("route", json!({
        "seed": 0, "starts": 1, "from": "current", "nets": ["T1"],
        "tune": [{ "name": "t", "nets": ["T*"] }]
    }));
    let g = row(&r["tune"], "name", "t");
    assert_eq!(g["nets"].as_array().unwrap().len(), 1, "{g}");
    let sk = row(&g["skipped"], "net", "T2");
    assert_eq!(sk["reason"], json!("not in route.nets"));
    assert!((g["target_mm"].as_f64().unwrap() - sk["length_mm"].as_f64().unwrap()).abs() < 1e-6, "{g}");
    s.finish();
}

#[test]
fn bad_arguments() {
    let mut s = session(1);
    s.err("route", json!({ "seed": 0, "pairs": [{ "p": "USB_P", "n": "NOPE" }] }), "unknown_net");
    s.err("route", json!({ "seed": 0, "pairs": [{ "p": "USB_P" }] }), "bad_request");
    s.err("route", json!({ "seed": 0, "tune": [{ "name": "g", "nets": [] }] }), "bad_request");
    s.err("route", json!({ "seed": 0, "pair_skew_mm": -1 }), "bad_request");
    // the session is unchanged and still usable
    assert_eq!(s.ok("route", json!({ "seed": 0, "starts": 1 }))["complete"], json!(true));
    s.finish();
}

/// One route of `pairs.dsn` with the given hello settings and server environment: (result without wall time, SES).
fn run_with(env: &[(&str, &str)], settings: Value) -> (Value, String) {
    let mut s = Server::start_with(env);
    s.hello(1, settings);
    s.ok("load", json!({ "dsn": path(&serve_data("pairs.dsn")) }));
    let r = s.ok("route", pair_args());
    let ses = s.ses();
    s.finish();
    (stable(r), ses)
}

/// pcbkit P3: the coupling tries run side by side on a local pool; with no pool (the test hook) they run one after the
/// other, and the route is the same.
#[test]
fn coupling_tries_give_the_same_route_with_and_without_a_pool() {
    let with_pool = run_with(&[], json!({}));
    let without = run_with(&[("FASTROUTE_TEST_NO_POOL", "1")], json!({}));
    assert_eq!(with_pool, without);
}

/// pcbkit P4 (`router.pair_quick_hold`): on a board the held run completes, nothing is cut, so the route is the same;
/// the setting is known to the server and the route stays deterministic.
#[test]
fn quick_pair_hold_changes_nothing_when_the_held_run_completes() {
    let off = run_with(&[], json!({}));
    let on = run_with(&[], json!({ "router.pair_quick_hold": true }));
    assert_eq!(off.0["complete"], json!(true), "{}", off.0);
    assert_eq!(off, on);
    assert_eq!(on, run_with(&[], json!({ "router.pair_quick_hold": true })));
}
