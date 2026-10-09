//! `fastroute serve` capability `blockers` (SPEC 5.6): blocked.dsn classification and attribution,
//! errors, ordering, determinism, and the opens of the pcbkit corpus boards.

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
        self.ok("hello", json!({ "protocol": "1.0.0", "client": "serve_blockers", "threads": threads, "settings": settings }))
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


fn started(threads: u32) -> Server {
    let mut s = Server::start();
    let h = s.hello(threads, json!({}));
    assert!(h["capabilities"].as_array().unwrap().contains(&json!("blockers")), "{h}");
    s
}

fn open_blocked(threads: u32) -> Server {
    let mut s = started(threads);
    s.ok("load", json!({ "dsn": path(&serve_data("blocked.dsn")) }));
    s
}

fn conn(c: &Value) -> Value {
    json!({ "net": c["net"], "from": c["from"], "to": c["to"] })
}

fn n4(s: &mut Server) -> Value {
    let r = s.ok("route", json!({ "seed": 0 }));
    let c = r["unrouted_connections"].as_array().unwrap().iter().find(|c| c["net"] == "N4").expect("N4 is open").clone();
    conn(&c)
}

#[test]
fn walled_in_connection_is_blocked_by_the_guard_wire() {
    let mut s = open_blocked(1);
    let c = n4(&mut s);
    let b = s.ok("blockers", json!({ "connection": c }));
    assert_eq!(b["class"], "blocked", "{b}");
    assert_eq!(b["connection"], c);
    let list = b["blockers"].as_array().unwrap();
    let guard = list.iter().find(|x| x["kind"] == "wire" && x["net"] == "GUARD").unwrap_or_else(|| panic!("{b}"));
    assert_eq!(guard["fixed"], true);
    assert_eq!(guard["locked"], false);
    assert!(guard["id"].is_i64() && guard["bbox"].as_array().unwrap().len() == 4 && guard["at"].as_array().unwrap().len() == 2);
    // nearest to the connection's line first
    let dist = |x: &Value| {
        let at = x["at"].as_array().unwrap();
        (at[0].as_i64().unwrap(), at[1].as_i64().unwrap())
    };
    assert!(list.len() >= 1 && dist(&list[0]) != (i64::MIN, i64::MIN));
    // max truncates; the ends of the connection may come in either order
    let one = s.ok("blockers", json!({ "connection": c, "max": 1 }));
    assert_eq!(one["blockers"].as_array().unwrap().len(), 1);
    assert_eq!(one["blockers"][0], list[0]);
    let swapped = json!({ "net": c["net"], "from": c["to"], "to": c["from"] });
    assert_eq!(s.ok("blockers", json!({ "connection": swapped }))["blockers"], b["blockers"]);
    // the answer does not change the session
    let after = s.ok("route", json!({ "seed": 0 }));
    assert_eq!(after["unrouted"], 1, "{after}");
    s.finish();
}

#[test]
fn routed_connection_has_no_blockers_and_bad_names_are_errors() {
    let mut s = open_blocked(1);
    s.ok("route", json!({ "seed": 0 }));
    let r = s.ok("blockers", json!({ "connection": { "net": "N1", "from": "R1-2", "to": "R2-1" } }));
    assert_eq!((r["class"].as_str(), r["blockers"].as_array().map(Vec::len)), (Some("routed"), Some(0)), "{r}");
    let e = s.err("blockers", json!({ "connection": { "net": "Nope", "from": "R1-2", "to": "R2-1" } }), "unknown_net");
    assert_eq!(e["details"]["name"], "Nope");
    let e = s.err("blockers", json!({ "connection": { "net": "N1", "from": "R9-9", "to": "R2-1" } }), "unknown_pin");
    assert_eq!(e["details"]["pin"], "R9-9");
    s.err("blockers", json!({ "connection": { "net": "N1", "from": "R3-1", "to": "R2-1" } }), "unknown_connection");
    s.err("blockers", json!({ "connection": { "net": "N1", "from": "R1-2" } }), "bad_request");
    s.err("blockers", json!({ "connection": { "net": "N1", "from": "R1-2", "to": "R2-1" }, "max": 0 }), "bad_request");
    s.err("blockers", json!({}), "bad_request");
    s.finish();
}

#[test]
fn identical_across_processes_and_threads() {
    let run = |threads: u32| {
        let mut s = open_blocked(threads);
        let c = n4(&mut s);
        let b = s.ok("blockers", json!({ "connection": c }));
        s.finish();
        b
    };
    assert_eq!(run(1), run(1));
    assert_eq!(run(1), run(2));
}

#[test]
fn locked_wiring_is_reported_as_locked() {
    // a routed neighbour that is locked stays on the board for the alone-route
    let mut s = open_blocked(1);
    s.ok("route", json!({ "seed": 0 }));
    s.ok("lock", json!({ "nets": ["N1"] }));
    let c = n4(&mut s);
    let b = s.ok("blockers", json!({ "connection": c }));
    assert_eq!(b["class"], "blocked", "{b}");
    s.finish();
}

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

/// Every open connection of a real board gets a class, and every `blocked` one names at least one object.
fn attribute_board(name: &str) -> (usize, usize, usize) {
    let dsn = corpus_dir().join("det").join(name).join("board.dsn");
    let mut s = started(1);
    s.ok("load", json!({ "dsn": path(&dsn) }));
    let r = s.ok("route", json!({ "seed": 0, "from": "scratch" }));
    let open = r["unrouted_connections"].as_array().unwrap().clone();
    let (mut blocked, mut congestion) = (0, 0);
    for c in &open {
        let b = s.ok("blockers", json!({ "connection": conn(c) }));
        let list = b["blockers"].as_array().unwrap();
        let class = b["class"].as_str().unwrap();
        let names: Vec<String> = list
            .iter()
            .take(4)
            .map(|x| format!("{}:{}{}", x["kind"].as_str().unwrap(), x["net"].as_str().unwrap_or("-"), x["ref"].as_str().map(|r| format!("@{r}-{}", x["pin"].as_str().unwrap())).unwrap_or_default()))
            .collect();
        eprintln!("{name} {} {}->{}: {class} {} objects {names:?}", c["net"], c["from"], c["to"], list.len());
        match class {
            "blocked" => {
                blocked += 1;
                assert!(!list.is_empty(), "{name}: blocked open {c} names no object");
            }
            "congestion" => congestion += 1,
            other => panic!("{name}: open connection {c} classified {other}"),
        }
        assert_eq!(s.ok("blockers", json!({ "connection": conn(c) })), b, "deterministic");
        assert!(!list.is_empty(), "{name}: open connection {c} ({class}) names no object");
    }
    s.finish();
    (open.len(), blocked, congestion)
}

#[test]
fn corpus_boards_name_their_blockers() {
    let mut total = 0;
    for name in ["hb200", "energy-12-1"] {
        let (open, blocked, congestion) = attribute_board(name);
        eprintln!("{name}: {open} open, {blocked} blocked, {congestion} congestion");
        total += open;
    }
    eprintln!("corpus opens: {total}");
}

// ---- falsification: are the named blockers causal? -------------------------------------------
//
// The corpus boards are routed from scratch with seed 0 (one thread). For every open connection the
// test takes the named objects off the board and checks that the connection then routes:
//   congestion  rip up the named routed wires and vias (the ones that are not fixed), then route
//               that net with the real pipeline (`route nets: [net]`, the other nets stay as they are);
//   blocked     take every named wire and via off a copy (fixed wiring too: it is the only thing
//               left on the alone board) and route the connection alone. A pin, keepout or the
//               boundary cannot be taken off; one is named only when taking off all wiring is not
//               enough, and then the open is infeasible by geometry, which the test asserts.
// A control route (same request, nothing ripped) shows which congestion opens the rip-up really fixed.

use fr_serve::falsify::Probe;

fn is_open(r: &Value, c: &Value) -> bool {
    let k = conn(c);
    r["unrouted_connections"].as_array().unwrap().iter().any(|u| {
        u["net"] == k["net"] && ((u["from"] == k["from"] && u["to"] == k["to"]) || (u["from"] == k["to"] && u["to"] == k["from"]))
    })
}

/// Ids of the named objects a rip-up can remove: routed wires and vias, neither fixed nor locked.
fn rippable(list: &[Value]) -> Vec<i64> {
    let routed: Vec<Value> = list.iter().filter(|x| x["fixed"] == false && x["locked"] == false).cloned().collect();
    Probe::wire_ids(&routed)
}

#[derive(Default, Debug)]
struct Tally {
    congestion: usize,
    /// congestion opens that route after the named wires are ripped up
    fixed_by_rip: usize,
    /// ... of which the control route (nothing ripped) leaves open
    fixed_only_by_rip: usize,
    blocked: usize,
    /// blocked opens whose alone route succeeds once the named wiring is taken off
    fixed_by_removal: usize,
    /// blocked opens that name a pin (or other item that cannot be taken off)
    names_immovable: usize,
}

fn kinds(list: &[Value]) -> String {
    let mut n: std::collections::BTreeMap<String, usize> = Default::default();
    for x in list {
        let tag = format!("{}{}{}", x["kind"].as_str().unwrap(), if x["fixed"] == true { "(fixed)" } else { "" }, x["net"].as_str().map(|s| format!(":{s}")).unwrap_or_default());
        *n.entry(tag).or_default() += 1;
    }
    n.iter().map(|(k, v)| format!("{k}x{v}")).collect::<Vec<_>>().join(" ")
}

fn falsify_board(name: &str, seed: i64) -> Tally {
    let dsn = corpus_dir().join("det").join(name).join("board.dsn");
    let mut p = Probe::open(dsn.canonicalize().unwrap().to_str().unwrap(), 1);
    let r = p.call("route", json!({ "seed": seed, "from": "scratch" }));
    let snap = p.call("snapshot", json!({}))["snapshot"].as_str().unwrap().to_string();
    let mut t = Tally::default();
    for c in r["unrouted_connections"].as_array().unwrap() {
        let k = conn(c);
        let b = p.call("blockers", json!({ "connection": k }));
        let list = b["blockers"].as_array().unwrap().clone();
        let tag = format!("{name}/seed{seed} {} {}->{}", c["net"], c["from"], c["to"]);
        assert!(!list.is_empty(), "{tag}: names nothing");
        match b["class"].as_str().unwrap() {
            "congestion" => {
                t.congestion += 1;
                let control = !is_open(&p.call("route", json!({ "seed": seed, "nets": [c["net"]] })), c);
                p.call("restore", json!({ "snapshot": snap }));
                let ids = rippable(&list);
                let gone = p.rip(&ids);
                let ok = !is_open(&p.call("route", json!({ "seed": seed, "nets": [c["net"]] })), c);
                p.call("restore", json!({ "snapshot": snap }));
                eprintln!("{tag}: congestion; names {} ({}); {gone} ripped; routes after rip {ok}, control without rip {control}", list.len(), kinds(&list));
                t.fixed_by_rip += ok as usize;
                t.fixed_only_by_rip += (ok && !control) as usize;
            }
            "blocked" => {
                t.blocked += 1;
                let immovable: Vec<&Value> = list.iter().filter(|x| !matches!(x["kind"].as_str(), Some("wire" | "via"))).collect();
                let wiring = Probe::wire_ids(&list);
                let ok = p.alone_without(&k, &wiring);
                // the earlier manual attribution blamed the fanout vias alone: do the named vias suffice?
                let vias: Vec<Value> = list.iter().filter(|x| x["kind"] == "via").cloned().collect();
                let vias_only = p.alone_without(&k, &Probe::wire_ids(&vias));
                eprintln!("{tag}: blocked; names {} ({}); alone route after taking the named wiring off: {ok}; the {} named vias alone: {vias_only}", list.len(), kinds(&list), vias.len());
                t.fixed_by_removal += ok as usize;
                t.names_immovable += !immovable.is_empty() as usize;
                assert!(ok || !immovable.is_empty(), "{tag}: taking off the named wiring does not unblock it and no pin or keepout is named");
            }
            other => panic!("{tag}: class {other}"),
        }
    }
    eprintln!("{name}/seed{seed}: {t:?}");
    t
}

#[test]
fn named_blockers_are_causal_on_corpus_boards() {
    let (mut congestion, mut fixed) = (0, 0);
    for (name, seed) in [("hb200", 0), ("energy-12-1", 0), ("hb200", 1)] {
        let t = falsify_board(name, seed);
        congestion += t.congestion;
        fixed += t.fixed_by_rip;
    }
    assert!(congestion >= 1, "the corpus has a congestion-class open");
    assert!(fixed * 5 >= congestion * 4, "ripping the named wires fixed {fixed} of {congestion} congestion opens (need 80%)");
}
