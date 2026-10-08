//! `fastroute serve` capabilities `seed` and `incremental` (router protocol 1.0): seeds are
//! deterministic across processes, a net-list route leaves every other net's wiring byte-identical,
//! and the per-request knobs do not outlive the request.

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

fn corpus(key: &str) -> PathBuf {
    manifest().join("../../reference/pcbkit-corpus/det").join(key).join("board.dsn")
}

/// Wiring text per net, in file order, of the `network_out` section of a session file.
fn net_blocks(ses: &str) -> Vec<(String, String)> {
    let body = &ses[ses.find("(network_out").expect("network_out")..];
    let mut out: Vec<(String, String)> = Vec::new();
    for line in body.lines().skip(1) {
        if let Some(rest) = line.trim_start().strip_prefix("(net ") {
            out.push((rest.trim().trim_matches('"').to_string(), String::new()));
        } else if let Some(last) = out.last_mut() {
            last.1.push_str(line);
            last.1.push('\n');
        }
    }
    out
}

/// Everything before `network_out` (the placement): routing must never change it.
fn placement(ses: &str) -> &str {
    &ses[..ses.find("(network_out").unwrap()]
}

fn open(dsn: &Path, threads: u32) -> Server {
    let mut s = Server::start();
    s.hello(threads, json!({}));
    s.ok("load", json!({ "dsn": path(dsn) }));
    s
}

#[test]
fn capabilities_and_errors() {
    let mut s = Server::start();
    let h = s.hello(1, json!({}));
    // the exact claimed set is pinned in serve_baseline.rs; here only this module's two
    for cap in ["incremental", "seed"] {
        assert!(h["capabilities"].as_array().unwrap().contains(&json!(cap)), "{h}");
    }
    s.ok("load", json!({ "dsn": path(&serve_data("tiny.dsn")) }));
    let e = s.err("route", json!({ "seed": 0, "nets": ["N1", "NOPE"] }), "unknown_net");
    assert_eq!(e["details"]["name"], "NOPE");
    // a refused request leaves the board unrouted
    let r = s.ok("route", json!({ "seed": 3, "nets": [] , "from": "scratch" }));
    assert_eq!(r["vias"], 0);
    assert_eq!(r["wires"], 0);
    s.finish();
}

#[test]
fn seeds_are_deterministic_across_processes_and_reported() {
    let dsn = serve_data("tiny.dsn");
    let run = |seed: i64| {
        let mut s = open(&dsn, 1);
        let r = s.ok("route", json!({ "seed": seed, "from": "scratch" }));
        let ses = s.ses();
        s.finish();
        (stable(r), ses)
    };
    for seed in [1, 7, 12345] {
        let (a, b) = (run(seed), run(seed));
        assert_eq!(a, b, "seed {seed}");
        assert_eq!(a.0["seed"], seed);
        assert_eq!(a.0["seed_used"], true);
        assert_eq!(a.0["complete"], true, "{}", a.0);
    }
    let zero = run(0);
    assert_eq!(zero.0["seed_used"], true);
    assert_eq!(zero, run(0));
}

#[test]
fn seed_changes_a_real_board_deterministically() {
    let dsn = corpus("energy-12-1");
    let run = |seed: i64| {
        let mut s = open(&dsn, 1);
        s.ok("route", json!({ "seed": seed, "from": "scratch" }));
        let ses = s.ses();
        s.finish();
        ses
    };
    assert_eq!(run(7), run(7));
    let distinct: std::collections::HashSet<String> = [0, 1, 7, 12345].into_iter().map(run).collect();
    assert!(distinct.len() > 1, "seeds should select different variants on a real board");
}

/// Routes everything, then only `target` from scratch: every other net's wiring is byte-identical.
fn incremental_leaves_others(dsn: &Path, target: Option<&str>) {
    let mut s = open(dsn, 1);
    s.ok("route", json!({ "seed": 0, "from": "scratch" }));
    let before = s.ses();
    let blocks = net_blocks(&before);
    let target = target.map(str::to_string).unwrap_or_else(|| {
        // a routed net: the second block with a wire
        blocks.iter().filter(|(_, t)| t.contains("(wire")).nth(1).expect("a routed net").0.clone()
    });
    let r = s.ok("route", json!({ "seed": 5, "nets": [target.clone()], "from": "scratch" }));
    assert!(r["nets"].is_array());
    let after = s.ses();
    assert_eq!(placement(&before), placement(&after));
    let after_blocks = net_blocks(&after);
    assert_eq!(blocks.len(), after_blocks.len());
    for ((na, ta), (nb, tb)) in blocks.iter().zip(&after_blocks) {
        assert_eq!(na, nb);
        if *na != target {
            assert_eq!(ta, tb, "net {na} changed while routing only {target} on {}", dsn.display());
        }
    }
    s.finish();
}

#[test]
fn incremental_tiny() {
    incremental_leaves_others(&serve_data("tiny.dsn"), Some("N3"));
}

#[test]
fn incremental_corpus_energy() {
    incremental_leaves_others(&corpus("energy-12-1"), None);
}

#[test]
fn incremental_corpus_hb200() {
    incremental_leaves_others(&corpus("hb200"), None);
}

#[test]
fn incremental_corpus_heuristic_baseline() {
    incremental_leaves_others(&corpus("heuristic-baseline-1"), None);
}

#[test]
fn incremental_changes_only_listed_net_from_current() {
    // from: current ripped and re-routes only the listed net as well
    let mut s = open(&serve_data("tiny.dsn"), 2);
    s.ok("route", json!({ "seed": 0, "from": "scratch" }));
    let before = net_blocks(&s.ses());
    s.ok("route", json!({ "seed": 9, "nets": ["N2"] }));
    let after = net_blocks(&s.ses());
    for ((n, a), (_, b)) in before.iter().zip(&after) {
        if n != "N2" {
            assert_eq!(a, b, "net {n}");
        }
    }
    s.finish();
}

#[test]
fn request_knobs_and_fixing_do_not_outlive_the_request() {
    // History: all, incremental N3, then all again (scratch and current). A fresh process replaying
    // the same history must agree byte for byte and the last route must complete the board: a leaked
    // net mask, seed or fixed state would leave nets unrouted or make replays differ. (A session's
    // result depends on its history even without these knobs, so the comparison is replay vs replay.)
    let dsn = serve_data("tiny.dsn");
    for threads in [1, 2] {
        for from in ["scratch", "current"] {
            let replay = || {
                let mut s = open(&dsn, threads);
                s.ok("route", json!({ "seed": 0, "from": "scratch" }));
                s.ok("route", json!({ "seed": 4, "nets": ["N3"], "from": "scratch" }));
                let last = s.ok("route", json!({ "seed": 0, "nets": "all", "from": from }));
                assert_eq!(last["complete"], true, "{last}");
                assert_eq!(last["seed_used"], true);
                let ses = s.ses();
                s.finish();
                (stable(last), ses)
            };
            assert_eq!(replay(), replay(), "threads {threads} from {from}");
        }
    }
}
