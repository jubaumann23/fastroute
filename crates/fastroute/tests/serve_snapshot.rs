//! `fastroute serve` snapshot/restore (capability `snapshot`, router protocol 1.0 section 5.8).

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


/// The tracked protocol fixture plus two boards of the pcbkit corpus (`reference/pcbkit-corpus`, git
/// ignored: found next to the main checkout, or at `FR_PCBKIT_CORPUS`). A missing corpus fails.
fn corpus_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("FR_PCBKIT_CORPUS") {
        return PathBuf::from(d);
    }
    let local = manifest().join("../../reference/pcbkit-corpus");
    if local.is_dir() {
        return local;
    }
    let out = Command::new("git").args(["rev-parse", "--path-format=absolute", "--git-common-dir"]).current_dir(manifest()).output().unwrap();
    let common = PathBuf::from(String::from_utf8(out.stdout).unwrap().trim());
    let dir = common.parent().unwrap().join("reference/pcbkit-corpus");
    assert!(dir.is_dir(), "pcbkit corpus not found: set FR_PCBKIT_CORPUS (looked in {})", dir.display());
    dir
}

fn corpus_board(name: &str) -> PathBuf {
    corpus_dir().join("det").join(name).join("board.dsn")
}

fn open(threads: u32, dsn: &Path) -> Server {
    let mut s = Server::start();
    let h = s.hello(threads, json!({}));
    assert!(h["capabilities"].as_array().unwrap().contains(&json!("snapshot")), "{h}");
    s.ok("load", json!({ "dsn": path(dsn) }));
    s
}

/// The ways to move on from a base: each is applied identically in the session under test and in a
/// fresh session that replays the base (load, route from scratch). Reloading the base SES instead is
/// NOT equivalent: a route from a loaded SES differs from a route on the live board (contract note).
fn nudge(s: &mut Server, which: usize) -> Value {
    match which {
        0 => s.ok("route", json!({ "seed": 0 })),
        1 => s.ok("route", json!({ "seed": 0, "from": "scratch" })),
        _ => s.ok("route", json!({ "seed": 3, "nets": "all", "from": "current" })),
    }
}

fn check_board(dsn: &Path, threads: u32) {
    let label = format!("{} at {threads} thread(s)", dsn.display());
    let mut s = open(threads, dsn);
    let base = s.ok("route", json!({ "seed": 0, "from": "scratch" }));
    let base_ses = s.ses();
    let snap = s.ok("snapshot", json!({}))["snapshot"].as_str().unwrap().to_string();

    // route -> restore -> SES equal to the snapshot point
    let first = stable(nudge(&mut s, 1));
    let first_ses = s.ses();
    let rest = s.ok("restore", json!({ "snapshot": snap }));
    assert_eq!((rest["unrouted"].clone(), rest["wires"].clone(), rest["vias"].clone()), (base["unrouted"].clone(), base["wires"].clone(), base["vias"].clone()), "{label}");
    assert_eq!(s.ses(), base_ses, "{label}: SES after restore");

    // route after restore equals route after snapshot (results minus wall_ms, and SES)
    let again = stable(nudge(&mut s, 1));
    assert_eq!(again, first, "{label}: route after restore");
    assert_eq!(s.ses(), first_ses, "{label}: SES after route after restore");

    // three nudges from one base via restore, each equal to a fresh-session equivalent
    for which in 0..3 {
        s.ok("restore", json!({ "snapshot": snap }));
        let got = stable(nudge(&mut s, which));
        let got_ses = s.ses();
        let mut fresh = Server::start();
        fresh.hello(threads, json!({}));
        fresh.ok("load", json!({ "dsn": path(dsn) }));
        fresh.ok("route", json!({ "seed": 0, "from": "scratch" }));
        let want = stable(nudge(&mut fresh, which));
        assert_eq!(got, want, "{label}: nudge {which} result");
        assert_eq!(got_ses, fresh.ses(), "{label}: nudge {which} SES");
        fresh.finish();
    }
    s.finish();
}

macro_rules! reproducible {
    ($(#[$meta:meta])* $name:ident, $board:expr, $threads:expr) => {
        #[test]
        $(#[$meta])*
        fn $name() {
            check_board(&$board, $threads);
        }
    };
}

reproducible!(tiny_threads_1, serve_data("tiny.dsn"), 1);
reproducible!(tiny_threads_4, serve_data("tiny.dsn"), 4);
reproducible!(#[ignore = "on-demand (scripts/pcbkit-bench.sh): routes a corpus board several times"] energy_threads_1, corpus_board("energy-12-1"), 1);
reproducible!(#[ignore = "on-demand (scripts/pcbkit-bench.sh): routes a corpus board several times"] energy_threads_4, corpus_board("energy-12-1"), 4);
reproducible!(#[ignore = "on-demand (scripts/pcbkit-bench.sh): routes a corpus board several times"] hb200_threads_1, corpus_board("hb200"), 1);
reproducible!(#[ignore = "on-demand (scripts/pcbkit-bench.sh): routes a corpus board several times"] hb200_threads_4, corpus_board("hb200"), 4);

#[test]
fn restore_semantics_and_errors() {
    let dsn = serve_data("tiny.dsn");
    let mut s = open(1, &dsn);
    s.err("snapshot", json!({ "x": 1 }), "bad_request");
    s.err("restore", json!({}), "bad_request");
    s.err("restore", json!({ "snapshot": "nope" }), "unknown_snapshot");
    let a = s.ok("snapshot", json!({}))["snapshot"].as_str().unwrap().to_string();
    let b = s.ok("snapshot", json!({}))["snapshot"].as_str().unwrap().to_string();
    assert_ne!(a, b);
    let ses0 = s.ses();
    s.ok("route", json!({ "seed": 0, "from": "scratch" }));
    let routed = s.ses();
    assert_ne!(routed, ses0);
    // unknown id leaves the state unchanged
    s.err("restore", json!({ "snapshot": "nope" }), "unknown_snapshot");
    assert_eq!(s.ses(), routed);
    // restore keeps the snapshot unless drop
    let r = s.ok("restore", json!({ "snapshot": a }));
    assert_eq!(r["unrouted"], 4);
    assert_eq!(s.ses(), ses0);
    s.ok("restore", json!({ "snapshot": a, "drop": true }));
    s.err("restore", json!({ "snapshot": a }), "unknown_snapshot");
    s.ok("restore", json!({ "snapshot": b }));
    // load drops all snapshots
    s.ok("load", json!({ "dsn": path(&dsn) }));
    s.err("restore", json!({ "snapshot": b }), "unknown_snapshot");
    s.finish();
}
