//! `fastroute serve` baseline (router protocol 1.0): every error code reachable at baseline, routing
//! determinism across processes, and byte equality with the stock CLI.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
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

#[test]
fn error_codes() {
    let mut s = Server::start();
    // before the handshake
    let r = s.raw(r#"{"id":1,"op":"route","args":{"seed":0}}"#);
    assert_eq!(r["error"]["code"], "no_hello");
    let r = s.raw("this is not json");
    assert_eq!((r["id"].clone(), r["error"]["code"].clone()), (Value::Null, json!("bad_json")));
    let r = s.raw("[1,2]");
    assert_eq!(r["error"]["code"], "bad_json");
    let r = s.raw(r#"{"op":"hello"}"#);
    assert_eq!((r["id"].clone(), r["error"]["code"].clone()), (Value::Null, json!("bad_request")));
    let r = s.raw(r#"{"id":9,"op":"hello","bogus":1}"#);
    assert_eq!((r["id"].clone(), r["error"]["code"].clone()), (json!(9), json!("bad_request")));
    let r = s.raw(r#"{"id":10,"op":"nope"}"#);
    assert_eq!(r["error"]["code"], "unknown_op");
    s.next_id = 11;
    // hello: bad arguments, then good
    s.err("hello", json!({ "protocol": "1.0.0", "client": "c", "threads": 0 }), "bad_request");
    s.err("hello", json!({ "protocol": "1.0.0", "client": "c", "threads": 1, "extra": true }), "bad_request");
    s.err("hello", json!({ "protocol": "1.0.0", "client": "c", "threads": 1, "settings": { "a": [1] } }), "bad_request");
    let h = s.hello(2, json!({ "router.autorouter.max_threads": 7, "router.nope": 1 }));
    assert_eq!(h["threads"], 2);
    assert_eq!(h["settings"]["unknown"], json!(["router.nope"]));
    assert_eq!(h["settings"]["applied"]["router.autorouter.max_threads"], "2");
    assert_eq!(h["capabilities"], json!(["budget"]));
    s.err("hello", json!({ "protocol": "1.0.0", "client": "c", "threads": 2 }), "bad_state");
    // not loaded
    s.err("route", json!({ "seed": 0 }), "not_loaded");
    s.err("export", json!({ "format": "ses" }), "not_loaded");
    // load errors
    s.err("load", json!({ "dsn": { "path": "/no/such/file.dsn" } }), "io_error");
    s.err("load", json!({ "dsn": { "path": "relative.dsn" } }), "bad_request");
    s.err("load", json!({ "dsn": { "text": "x", "path": "/x" } }), "bad_request");
    s.err("load", json!({ "dsn": path(&serve_data("tiny.dsn")), "bogus": 1 }), "bad_request");
    let e = s.err("load", json!({ "dsn": { "text": "(pcb broken" } }), "bad_dsn");
    assert!(e["details"]["line"].is_u64() && e["details"]["reason"].is_string(), "{e}");
    let e = s.err("load", json!({ "dsn": path(&serve_data("tiny.dsn")), "ses": { "text": "this is not a session" } }), "bad_ses");
    assert!(e["details"]["line"].is_u64() && e["details"]["reason"].is_string(), "{e}");
    s.err("load", json!({ "dsn": path(&serve_data("tiny.dsn")), "ses": { "text": "(pcb nope)" } }), "bad_ses");
    s.err("load", json!({ "dsn": path(&serve_data("tiny.dsn")), "lock_initial": true }), "unsupported");
    // a failed load leaves the session unloaded
    s.err("route", json!({ "seed": 0 }), "not_loaded");
    let l = s.ok("load", json!({ "dsn": path(&serve_data("tiny.dsn")) }));
    assert_eq!(l["name"], "tiny.dsn");
    // route / export errors
    s.err("route", json!({ "seed": 0, "from": "sideways" }), "bad_request");
    s.err("route", json!({}), "bad_request");
    s.err("route", json!({ "seed": -1 }), "bad_request");
    s.err("route", json!({ "seed": 0, "bogus": 1 }), "bad_request");
    s.err("route", json!({ "seed": 0, "nets": ["N1"] }), "unsupported");
    s.err("export", json!({ "format": "dxf" }), "bad_request");
    s.err("export", json!({ "format": "ses", "path": "relative.ses" }), "bad_request");
    s.err("export", json!({ "format": "ses", "path": "/no/such/dir/out.ses" }), "io_error");
    // ops of capabilities this build does not claim
    for op in ["move", "lock", "unlock", "blockers", "congestion", "snapshot", "restore"] {
        s.err(op, json!({}), "unsupported");
    }
    // shutdown takes no arguments
    s.err("shutdown", json!({ "x": 1 }), "bad_request");
    s.finish();
}

#[test]
fn version_mismatch_then_only_shutdown() {
    let mut s = Server::start();
    s.err("hello", json!({ "protocol": "2.0.0", "client": "c", "threads": 1 }), "version_mismatch");
    s.err("hello", json!({ "protocol": "1.0.0", "client": "c", "threads": 1 }), "version_mismatch");
    s.err("load", json!({ "dsn": path(&serve_data("tiny.dsn")) }), "version_mismatch");
    s.finish();
}

#[test]
fn internal_error_on_panic_and_session_survives() {
    let mut s = Server::start_with(&[("FR_SERVE_TEST_PANIC", "export")]);
    s.hello(1, json!({}));
    s.ok("load", json!({ "dsn": path(&serve_data("tiny.dsn")) }));
    s.err("export", json!({ "format": "ses" }), "internal");
    s.ok("route", json!({ "seed": 0, "from": "scratch" }));
    s.finish();
}

#[test]
fn end_of_input_is_an_implicit_shutdown() {
    let mut s = Server::start();
    s.hello(1, json!({}));
    drop(s.stdin.take());
    assert!(s.child.wait().unwrap().success());
}

#[test]
fn tiny_routes_complete_and_two_processes_agree() {
    let run = |threads| {
        let mut s = Server::start();
        s.hello(threads, json!({}));
        let l = s.ok("load", json!({ "dsn": path(&serve_data("tiny.dsn")) }));
        assert_eq!(l["connections"], 4);
        assert_eq!(l["unrouted"], 4);
        assert_eq!(l["layers"], json!(["F.Cu", "B.Cu"]));
        assert_eq!(l["boundary"], json!([100000, -250000, 300000, -100000]));
        let r = s.ok("route", json!({ "seed": 0, "nets": "all", "from": "scratch" }));
        assert_eq!(r["complete"], true, "{r}");
        assert_eq!(r["unrouted_connections"], json!([]));
        assert_eq!(r["seed_used"], false);
        assert_eq!(r["budget_hit"], false);
        let ses = s.ses();
        s.finish();
        (stable(r), ses)
    };
    for threads in [1, 2] {
        assert_eq!(run(threads), run(threads), "threads {threads}");
    }
}

#[test]
fn blocked_connection_is_listed() {
    let mut s = Server::start();
    s.hello(2, json!({}));
    s.ok("load", json!({ "dsn": path(&serve_data("blocked.dsn")) }));
    let r = s.ok("route", json!({ "seed": 0, "from": "scratch" }));
    assert_eq!(r["complete"], false);
    let conns = r["unrouted_connections"].as_array().unwrap();
    assert!(conns.iter().any(|c| c["net"] == "N4" && c["to"] == "R4-1" && c["from"] == "R3-2"), "{r}");
    assert!(r["nets"].as_array().unwrap().iter().any(|n| n["name"] == "N4" && n["status"] == "unrouted"));
    s.finish();
}

#[test]
fn export_to_path_matches_inline_and_budget_reports() {
    let mut s = Server::start();
    s.hello(1, json!({}));
    s.ok("load", json!({ "dsn": path(&serve_data("tiny.dsn")) }));
    let r = s.ok("route", json!({ "seed": 0, "from": "scratch", "budget_ms": 1 }));
    assert!(r["budget_hit"].is_boolean());
    s.ok("route", json!({ "seed": 0, "from": "scratch" }));
    let inline = s.ses();
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("serve-export-{}.ses", std::process::id()));
    let e = s.ok("export", json!({ "format": "ses", "path": out.to_str().unwrap() }));
    assert_eq!(std::fs::read_to_string(&out).unwrap(), inline);
    assert_eq!(e["bytes"].as_u64().unwrap() as usize, inline.len());
    s.finish();
}

// ------------------------------------------------------------------------------ stock equivalence

/// Runs the stock CLI directly on `dsn` (the session is named after the file stem) with the
/// `FREEROUTING__ROUTER__*` environment removed, and returns the SES text.
fn stock_ses(dsn: &Path, threads: u32, extra: &[String]) -> Option<String> {
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    let output = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("serve-stock-{}-{}.ses", std::process::id(), CALLS.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_file(&output);
    let mut cmd = Command::new(BIN);
    cmd.args(["-de", dsn.to_str().unwrap(), "-do", output.to_str().unwrap(), "--no-time-limits"])
        .arg(format!("--router.autorouter.max_threads={threads}"))
        .arg(format!("--router.optimizer.max_threads={threads}"))
        .args(extra)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (k, _) in std::env::vars().filter(|(k, _)| k.to_uppercase().starts_with("FREEROUTING__ROUTER__")) {
        cmd.env_remove(k);
    }
    let status = cmd.status().unwrap();
    let ses = status.success().then(|| std::fs::read_to_string(&output).unwrap());
    let _ = std::fs::remove_file(&output);
    ses
}

/// The stock CLI keeps the unfixed wiring a DSN carries and re-routes it, which is `from: "current"`;
/// `from: "scratch"` is the same thing only on a board without such wiring.
fn stock_from(dsn: &Path) -> &'static str {
    if std::fs::read_to_string(dsn).unwrap().contains("(wire") {
        "current"
    } else {
        "scratch"
    }
}

/// The session `serve` writes, or None when it refuses the board with `bad_dsn`.
fn serve_ses(dsn: &Path, threads: u32, settings: Value) -> Option<String> {
    let mut s = Server::start();
    s.hello(threads, settings);
    if s.call("load", json!({ "dsn": path(dsn) }))["error"]["code"] == "bad_dsn" {
        s.finish();
        return None;
    }
    s.ok("route", json!({ "seed": 0, "nets": "all", "from": stock_from(dsn) }));
    let ses = s.ses();
    s.finish();
    Some(ses)
}

fn boards() -> Vec<PathBuf> {
    let mut v = vec![serve_data("tiny.dsn"), serve_data("blocked.dsn")];
    let mut extra: Vec<PathBuf> = std::fs::read_dir(manifest().join("../fr-io/testdata/dsn"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "dsn"))
        .collect();
    extra.sort();
    v.extend(extra);
    v
}

/// serve and the stock CLI give the same session, or both refuse the board; returns the boards compared.
fn assert_equivalent(settings: Value, flags: &[String]) -> usize {
    let mut compared = 0;
    for dsn in boards() {
        for threads in [1, 2] {
            let (serve, stock) = (serve_ses(&dsn, threads, settings.clone()), stock_ses(&dsn, threads, flags));
            assert_eq!(serve, stock, "{} at {threads} thread(s), flags {flags:?}", dsn.display());
            compared += usize::from(serve.is_some());
        }
    }
    compared
}

#[test]
fn serve_route_equals_stock_cli() {
    assert!(assert_equivalent(json!({}), &[]) >= 10);
}

#[test]
fn serve_route_equals_stock_cli_with_edge_clearance_and_min_width() {
    let flags = ["--router.copper_to_edge_clearance_um=400".to_string(), "--router.min_trace_width_um=100".to_string()];
    let settings = json!({ "router.copper_to_edge_clearance_um": 400, "router.min_trace_width_um": 100 });
    assert!(assert_equivalent(settings, &flags) >= 10);
}

#[test]
fn hello_reports_applied_settings() {
    let mut s = Server::start();
    let h = s.hello(1, json!({ "router.copper_to_edge_clearance_um": 400, "router.min_trace_width_um": 100 }));
    assert_eq!(h["settings"]["applied"]["router.copper_to_edge_clearance_um"], "400");
    assert_eq!(h["settings"]["applied"]["router.min_trace_width_um"], "100");
    assert_eq!(h["settings"]["unknown"], json!([]));
    s.finish();
}

/// A copy of tiny.dsn whose `(pcb ..)` is an absolute path, as toolkit DSNs have.
fn absolute_name_dsn() -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("serve-abs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let text = std::fs::read_to_string(serve_data("tiny.dsn")).unwrap();
    assert!(text.starts_with("(pcb tiny.dsn"));
    let abs = dir.join("some_board.kicad_pcb");
    let out = dir.join("board.dsn");
    std::fs::write(&out, text.replacen("(pcb tiny.dsn", &format!("(pcb {}", abs.display()), 1)).unwrap();
    out
}

#[test]
fn ses_equals_stock_with_absolute_pcb_name() {
    let dsn = absolute_name_dsn();
    for threads in [1, 2] {
        let serve = serve_ses(&dsn, threads, json!({})).expect("serve loads it");
        let stock = stock_ses(&dsn, threads, &[]).expect("stock routes it");
        assert!(serve.starts_with("(session board"), "{}", &serve[..60]);
        assert_eq!(serve, stock, "threads {threads}");
    }
}

#[test]
fn text_load_names_the_session_after_the_pcb_name_stem() {
    let mut s = Server::start();
    s.hello(1, json!({}));
    let text = std::fs::read_to_string(absolute_name_dsn()).unwrap();
    let l = s.ok("load", json!({ "dsn": { "text": text } }));
    assert!(l["name"].as_str().unwrap().ends_with("some_board.kicad_pcb"), "{l}");
    s.ok("route", json!({ "seed": 0, "from": "scratch" }));
    let ses = s.ses();
    let first = ses.lines().next().unwrap();
    assert!(first == "(session \"some_board\"", "{first}");
    s.finish();
}

#[test]
fn environment_does_not_change_serve_results() {
    let run = |env: &[(&str, &str)]| {
        let mut s = Server::start_with(env);
        s.hello(1, json!({}));
        s.ok("load", json!({ "dsn": path(&serve_data("blocked.dsn")) }));
        let r = s.ok("route", json!({ "seed": 0, "from": "scratch" }));
        let ses = s.ses();
        s.finish();
        (stable(r), ses)
    };
    let plain = run(&[]);
    let with_env = run(&[("FREEROUTING__ROUTER__AUTOROUTER__MAX_PASSES", "1")]);
    assert_eq!(plain, with_env);
    assert_eq!(sha(&plain.1), sha(&with_env.1));
}

fn sha(s: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    format!("{:016x}", h.finish())
}
