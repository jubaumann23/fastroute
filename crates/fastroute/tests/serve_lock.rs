//! `fastroute serve` capability `locking`: locked wiring survives re-routes byte for byte, wires lock
//! by id, unlock restores ordinary wiring, `load.lock_initial` locks an imported session.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use fr_serve::serde_json::{json, Value};

const BIN: &str = env!("CARGO_BIN_EXE_fastroute");
const CORPUS: &str = "reference/pcbkit-corpus";

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn tiny() -> String {
    manifest().join("../fr-serve/tests/data/tiny.dsn").canonicalize().unwrap().to_str().unwrap().to_string()
}

/// A corpus file, found in this worktree's `reference` or in the main checkout's.
fn corpus(rel: &str) -> String {
    let local = manifest().join("../..").join(CORPUS).join(rel);
    let main = PathBuf::from("/home/jubau/coolProjects/fastroute").join(CORPUS).join(rel);
    let p = if local.exists() { local } else { main };
    p.canonicalize().unwrap_or_else(|_| panic!("corpus file {rel} missing")).to_str().unwrap().to_string()
}

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
        let h = s.ok("hello", json!({ "protocol": "1.0.0", "client": "serve_lock", "threads": threads }));
        assert!(h["capabilities"].as_array().unwrap().contains(&json!("locking")), "{h}");
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

    fn err(&mut self, op: &str, args: Value, code: &str) {
        let r = self.call(op, args);
        assert_eq!(r["error"]["code"], json!(code), "{op}: {r}");
    }

    fn route(&mut self, seed: i64, from: &str) -> Value {
        self.ok("route", json!({ "seed": seed, "nets": "all", "from": from }))
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

/// The balanced text of `(net NAME ...)` in the SES `network_out`, or None.
fn net_wiring(ses: &str, name: &str) -> Option<String> {
    let routes = &ses[ses.find("(network_out")?..];
    let start = [format!("(net \"{name}\""), format!("(net {name}")].iter().filter_map(|k| routes.find(k.as_str())).min()?;
    let mut depth = 0;
    for (i, c) in routes[start..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(routes[start..start + i + 1].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

fn net_names(route_result: &Value) -> Vec<String> {
    route_result["nets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["wires"].as_i64().unwrap() + n["vias"].as_i64().unwrap() > 0)
        .map(|n| n["name"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn locked_nets_survive_five_seeded_reroutes() {
    for threads in [1, 2] {
        let mut s = Server::start(threads);
        s.ok("load", json!({ "dsn": { "path": tiny() } }));
        s.route(0, "scratch");
        let before = s.ses();
        let kept: Vec<_> = ["N1", "N2"].iter().map(|n| net_wiring(&before, n).expect("routed")).collect();
        let r = s.ok("lock", json!({ "nets": ["N2", "N1"] }));
        assert_eq!(r["nets"], json!(["N1", "N2"]), "DSN network order");
        let locked_wires = r["wires"].as_i64().unwrap();
        assert!(locked_wires > 0);
        for (seed, from) in (0..5).map(|i| (i, if i % 2 == 0 { "scratch" } else { "current" })) {
            let res = s.route(seed, from);
            let ses = s.ses();
            for (n, want) in ["N1", "N2"].iter().zip(&kept) {
                assert_eq!(net_wiring(&ses, n).as_ref(), Some(want), "threads {threads} seed {seed} {from}: {n} changed");
            }
            let row = |name: &str| res["nets"].as_array().unwrap().iter().find(|r| r["name"] == name).unwrap().clone();
            assert_eq!(row("N1")["status"], "locked");
            assert_eq!(row("N2")["status"], "locked");
            assert_eq!(s.ok("lock", json!({ "nets": ["N1"] }))["wires"], json!(locked_wires), "idempotent, no leak");
        }
        s.finish();
    }
}

/// Locked item count and nets now (an empty `lock` is a query).
fn state(s: &mut Server) -> (Value, i64) {
    let r = s.ok("lock", json!({ "nets": [] }));
    (r["nets"].clone(), r["wires"].as_i64().unwrap())
}

#[test]
fn lock_by_wire_id_keeps_the_item_and_routes_the_rest() {
    let mut s = Server::start(1);
    s.ok("load", json!({ "dsn": { "path": tiny() } }));
    s.route(0, "scratch");
    // ids are server assigned: probe them black-box
    let mut found = None;
    for id in 0..400 {
        let r = s.call("lock", json!({ "wires": [id] }));
        if r["ok"] == json!(true) {
            found = Some(id);
            break;
        }
        assert_eq!(r["error"]["code"], "unknown_wire", "{r}");
    }
    let id = found.expect("a wire id below 400");
    assert_eq!(state(&mut s), (json!([]), 1), "one item locked, no net");
    for seed in 1..4 {
        let res = s.route(seed, "scratch");
        assert_eq!(res["complete"], true, "the rest of the net routes: {res}");
        assert_eq!(state(&mut s), (json!([]), 1), "seed {seed}: no duplicate, no loss");
        assert_eq!(s.ok("lock", json!({ "wires": [id] }))["wires"], 1, "seed {seed}: id {id} still names the item");
    }
    s.err("lock", json!({ "wires": [999_999] }), "unknown_wire");
    s.err("lock", json!({ "nets": ["NOPE"] }), "unknown_net");
    s.err("unlock", json!({ "nets": ["NOPE"] }), "unknown_net");
    let r = s.ok("unlock", json!({ "wires": [id] }));
    assert_eq!((r["nets"].clone(), r["wires"].clone()), (json!([]), json!(0)));
    s.finish();
}

#[test]
fn unlock_makes_wiring_ordinary_again() {
    let mut s = Server::start(1);
    s.ok("load", json!({ "dsn": { "path": tiny() } }));
    s.route(0, "scratch");
    assert!(s.ok("lock", json!({ "nets": ["N1", "N3"] }))["wires"].as_i64().unwrap() > 0);
    let r = s.ok("unlock", json!({ "nets": ["N1"] }));
    assert_eq!(r["nets"], json!(["N3"]));
    assert!(r["wires"].as_i64().unwrap() > 0);
    let r = s.ok("unlock", json!({ "all": true }));
    assert_eq!((r["nets"].clone(), r["wires"].clone()), (json!([]), json!(0)));
    s.err("unlock", json!({}), "bad_request");
    s.err("lock", json!({ "all": true }), "bad_request");
    // a failed lock changes nothing
    s.err("lock", json!({ "nets": ["N1", "NOPE"] }), "unknown_net");
    assert_eq!(state(&mut s), (json!([]), 0));
    let res = s.route(0, "scratch");
    assert_eq!(res["complete"], true);
    assert!(!s.ses().contains("(type protect)"), "locks never reach the exported session");
    s.finish();
}

#[test]
fn lock_initial_locks_the_imported_session() {
    let mut s = Server::start(1);
    s.ok("load", json!({ "dsn": { "path": tiny() } }));
    s.route(0, "scratch");
    let ses = s.ses();
    let ses_path = std::env::temp_dir().join(format!("serve_lock_{}.ses", std::process::id()));
    std::fs::write(&ses_path, &ses).unwrap();
    let ses_arg = json!({ "path": ses_path.to_str().unwrap() });
    s.ok("load", json!({ "dsn": { "path": tiny() }, "ses": ses_arg }));
    assert_eq!(state(&mut s), (json!([]), 0), "imported wiring is ordinary without lock_initial");
    let imported = s.ses(); // what the import itself does to the session (normalisation, order)
    s.ok("load", json!({ "dsn": { "path": tiny() }, "ses": ses_arg, "lock_initial": true }));
    let (nets, wires) = state(&mut s);
    assert_eq!(nets, json!([]));
    assert!(wires > 0, "imported items are locked");
    assert_eq!(s.ses(), imported, "locking does not change the written session");
    let names = ["N1", "N2", "N3", "N4"];
    let want: Vec<_> = names.iter().map(|n| net_wiring(&imported, n)).collect();
    let r = s.route(3, "scratch");
    assert_eq!(r["complete"], true);
    let after = s.ses();
    for (n, w) in names.iter().zip(&want) {
        assert_eq!(&net_wiring(&after, n), w, "{n} changed although it was imported locked");
    }
    assert_eq!(state(&mut s).1, wires);
    // load drops locks
    s.ok("load", json!({ "dsn": { "path": tiny() } }));
    assert_eq!(state(&mut s), (json!([]), 0));
    let _ = std::fs::remove_file(ses_path);
    s.finish();
}

#[test]
fn no_lock_leak_on_energy_12() {
    let mut s = Server::start(2);
    s.ok("load", json!({ "dsn": { "path": corpus("det/energy-12-1/board.dsn") } }));
    let first = s.route(0, "scratch");
    let routed = net_names(&first);
    assert!(routed.len() > 2, "energy-12-1 routes more than two nets");
    let lock: Vec<_> = routed[2..].to_vec();
    let free = &routed[..2];
    let locked_wires = s.ok("lock", json!({ "nets": lock }))["wires"].as_i64().unwrap();
    assert!(locked_wires > 0);
    let want_ses = s.ses();
    let want: Vec<_> = lock.iter().map(|n| net_wiring(&want_ses, n)).collect();
    for (seed, from) in [(1, "current"), (2, "scratch"), (3, "current")] {
        s.route(seed, from);
        let ses = s.ses();
        for (n, w) in lock.iter().zip(&want) {
            assert_eq!(&net_wiring(&ses, n), w, "seed {seed} {from}: locked net {n} changed (free nets {free:?})");
        }
        assert_eq!(s.ok("lock", json!({ "nets": lock }))["wires"], json!(locked_wires), "seed {seed} {from}: lock count drifted");
        assert!(!ses.contains("(type protect)"));
    }
    s.finish();
}
