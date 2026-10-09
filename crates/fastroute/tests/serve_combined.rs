//! `fastroute serve` capabilities together (router protocol 1.0): one scripted session that locks,
//! moves, re-routes a subset, snapshots and restores, asserting locked wiring never changes and a
//! targeted re-route after a move changes only unlocked nets.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use fr_serve::serde_json::{json, Value};

const BIN: &str = env!("CARGO_BIN_EXE_fastroute");
const LOCKED: [&str; 2] = ["N1", "N2"];

struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl Server {
    fn start(threads: u32) -> Server {
        let mut child =
            Command::new(BIN).arg("serve").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut s = Server { child, stdin, stdout, next_id: 1 };
        let h = s.ok("hello", json!({ "protocol": "1.0.0", "client": "serve_combined", "threads": threads }));
        let caps = h["capabilities"].as_array().unwrap();
        for c in ["locking", "move", "snapshot", "incremental"] {
            assert!(caps.contains(&json!(c)), "capability {c} missing: {h}");
        }
        s
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
        fr_serve::serde_json::from_str(&resp).unwrap()
    }

    fn ok(&mut self, op: &str, args: Value) -> Value {
        let r = self.call(op, args);
        assert_eq!(r["ok"], json!(true), "{op}: {r}");
        r["result"].clone()
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

/// Balanced text of every `(net NAME ...)` in the SES `network_out`, keyed by net name.
fn net_wiring(ses: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(at) = ses.find("(network_out") else { return out };
    let body = &ses[at..];
    let mut from = 0;
    while let Some(i) = body[from..].find("(net ") {
        let start = from + i;
        let mut depth = 0;
        let mut end = body.len();
        for (k, c) in body[start..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = start + k + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        let text = &body[start..end];
        let name = text["(net ".len()..].split(|c: char| c.is_whitespace() || c == ')').next().unwrap().trim_matches('"');
        out.insert(name.to_string(), text.to_string());
        from = end;
    }
    out
}

fn tiny() -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fr-serve/tests/data/tiny.dsn");
    p.canonicalize().unwrap().to_str().unwrap().to_string()
}

fn move_r4(s: &mut Server, dx: i64) -> Value {
    let places = s.ses();
    let key = "(place R4 ";
    let rest = &places[places.find(key).unwrap() + key.len()..];
    let t: Vec<&str> = rest.split_whitespace().take(4).collect();
    let (x, y): (i64, i64) = (t[0].parse().unwrap(), t[1].parse().unwrap());
    let rot: i64 = t[3].trim_end_matches(')').parse().unwrap();
    s.ok("move", json!({ "moves": [{ "ref": "R4", "x": x + dx, "y": y, "rot": rot, "side": t[2] }] }))
}

fn names(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|n| n.as_str().unwrap().to_string()).collect()
}

fn assert_locked_unchanged(now: &BTreeMap<String, String>, kept: &BTreeMap<String, String>, step: &str) {
    for n in LOCKED {
        assert_eq!(now.get(n), kept.get(n), "{step}: locked net {n} changed");
    }
}

#[test]
fn combined_session_keeps_locked_nets_and_changes_only_affected_ones() {
    for threads in [1, 2] {
        let mut s = Server::start(threads);
        s.ok("load", json!({ "dsn": { "path": tiny() } }));
        s.ok("route", json!({ "seed": 0, "nets": "all", "from": "scratch" }));
        let first = net_wiring(&s.ses());
        let kept: BTreeMap<_, _> = LOCKED.iter().map(|n| (n.to_string(), first.get(*n).cloned().expect("routed"))).collect();
        assert_eq!(names(&s.ok("lock", json!({ "nets": LOCKED }))["nets"]), LOCKED);

        // Move a part that is on no locked net; only its own nets are ripped.
        let moved = move_r4(&mut s, 10000);
        let affected = names(&moved["ripped"]["nets"]);
        assert!(!affected.is_empty() && affected.iter().all(|n| !LOCKED.contains(&n.as_str())), "{moved}");
        let after_move = net_wiring(&s.ses());
        assert_locked_unchanged(&after_move, &kept, "after move");

        s.ok("route", json!({ "seed": 1, "nets": affected, "from": "current" }));
        let routed = net_wiring(&s.ses());
        assert_locked_unchanged(&routed, &kept, "after targeted route");
        for (n, w) in &routed {
            if !affected.contains(n) {
                assert_eq!(Some(w), after_move.get(n), "targeted route changed unaffected net {n}");
            }
        }

        let snap = s.ok("snapshot", json!({}))["snapshot"].as_str().unwrap().to_string();
        let at_snapshot = s.ses();
        move_r4(&mut s, -5000);
        s.ok("route", json!({ "seed": 2, "nets": affected, "from": "current" }));
        let second = net_wiring(&s.ses());
        assert_locked_unchanged(&second, &kept, "after second move and route");
        assert_ne!(s.ses(), at_snapshot, "the second move must change the board");

        s.ok("restore", json!({ "snapshot": snap }));
        assert_eq!(s.ses(), at_snapshot, "restore must reproduce the snapshot export byte for byte");
        // The snapshot captured the locks: a full re-route after restore still keeps them.
        s.ok("route", json!({ "seed": 3, "nets": "all", "from": "current" }));
        assert_locked_unchanged(&net_wiring(&s.ses()), &kept, "after restore and full route");
        s.finish();
    }
}
