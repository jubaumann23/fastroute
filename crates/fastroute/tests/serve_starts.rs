//! `fastroute serve` route.starts (router protocol 1.1, capability `starts`): the multi-start count
//! per request. Absent equals the default 4, 1 is deterministic, out-of-range values are bad_request.

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

fn stable(mut result: Value) -> Value {
    result.as_object_mut().unwrap().remove("wall_ms");
    result
}

fn session(dsn: &str) -> Server {
    let mut s = Server::start();
    let h = s.hello(1, json!({}));
    assert!(h["capabilities"].as_array().unwrap().contains(&json!("starts")), "{h}");
    s.ok("load", json!({ "dsn": path(&serve_data(dsn)) }));
    s
}

fn routed(dsn: &str, args: Value) -> (Value, String) {
    let mut s = session(dsn);
    let r = s.ok("route", args);
    let ses = s.ses();
    s.finish();
    (r, ses)
}

#[test]
fn absent_equals_four() {
    for dsn in ["tiny.dsn"] {
        let (ra, sa) = routed(dsn, json!({ "seed": 0 }));
        let (rb, sb) = routed(dsn, json!({ "seed": 0, "starts": 4 }));
        assert_eq!(ra["starts"], json!(4));
        assert_eq!(stable(ra), stable(rb), "{dsn}");
        assert_eq!(sa, sb, "{dsn}");
    }
}

#[test]
fn one_start_is_deterministic_and_echoed() {
    let (ra, sa) = routed("tiny.dsn", json!({ "seed": 0, "starts": 1 }));
    let (rb, sb) = routed("tiny.dsn", json!({ "seed": 0, "starts": 1 }));
    assert_eq!(ra["starts"], json!(1));
    assert_eq!(stable(ra), stable(rb));
    assert_eq!(sa, sb);
}

#[test]
fn out_of_range_is_bad_request() {
    let mut s = session("tiny.dsn");
    for bad in [json!(0), json!(17), json!(-1), json!("4"), json!(2.5), json!(null)] {
        s.err("route", json!({ "seed": 0, "starts": bad }), "bad_request");
    }
    // the session is still usable and the max is accepted
    assert_eq!(s.ok("route", json!({ "seed": 0, "starts": 16 }))["starts"], json!(16));
    s.finish();
}

#[test]
fn starts_with_net_list_from_current() {
    let mut s = session("tiny.dsn");
    s.ok("route", json!({ "seed": 0, "starts": 1 }));
    let nets: Vec<Value> = s.ok("route", json!({ "seed": 0, "starts": 2 }))["nets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["name"].clone())
        .collect();
    assert!(!nets.is_empty());
    let r = s.ok("route", json!({ "seed": 0, "starts": 1, "nets": [nets[0].clone()], "from": "current" }));
    assert_eq!(r["starts"], json!(1));
    s.finish();
}
