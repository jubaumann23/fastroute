//! pcbkit W1 `router.strict_width` (fork docs/PCBKIT.md, PATCH LEDGER): a track is routed at its class width except in
//! a declared neck-down (near a pad of its own net, never below `router.min_trace_width_um`); a connection that does
//! not fit at full width is reported unrouted instead of being retried at `router.neck_width_um`. Also pins that a
//! per-layer class width (`layer_rule`) reaches the router. Tiny synthetic boards written at test time; each route
//! takes well under a second in the gate's build.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use fr_serve::serde_json::{json, Value};

const BIN: &str = env!("CARGO_BIN_EXE_fastroute");
const CLASS_UM: f64 = 300.0;
const PITCH_UM: i32 = 400;
const SLACK_UM: f64 = 0.5;

/// Two 5-pin connectors of 200 um pads at 400 um pitch facing each other; pin i of J1 joins pin i of J2.
/// Class width 300 um, clearance 160 um: at the pad row a 300 um track does not fit between the neighbours.
fn fine_pitch_dsn() -> String {
    let pins: String = (0..5).map(|i| format!("(pin PAD {} {} 0)", i + 1, i * PITCH_UM)).collect();
    let nets: String = (1..=5).map(|i| format!("(net N{i} (pins J1-{i} J2-{i}))")).collect();
    let names: String = (1..=5).map(|i| format!("N{i} ")).collect();
    format!(
        r#"(pcb neck.dsn (parser (string_quote ") (space_in_quoted_tokens on) (host_cad "KiCad's Pcbnew") (host_version "10.0.5"))
  (resolution um 10) (unit um)
  (structure
    (layer F.Cu (type signal) (property (index 0))) (layer B.Cu (type signal) (property (index 1)))
    (boundary (path pcb 0  0 0  12000 0  12000 -9000  0 -9000  0 0))
    (via "Via[0-1]_600:300_um")
    (rule (width {CLASS_UM}) (clearance 160)))
  (placement (component "CONN" (place J1 4000 -1500 front 0 (PN a)) (place J2 4000 -7500 front 0 (PN b))))
  (library
    (image "CONN" {pins})
    (padstack PAD (shape (polygon F.Cu 0 -100 300 100 300 100 -300 -100 -300)) (attach off))
    (padstack "Via[0-1]_600:300_um" (shape (circle F.Cu 600)) (shape (circle B.Cu 600)) (attach off)))
  (network {nets}
    (class kicad_default {names} (circuit (use_via "Via[0-1]_600:300_um"))
      (rule (width {CLASS_UM}) (clearance 160)))))"#
    )
}

/// Through-hole pads at 600 um pitch, F.Cu closed to routing, class width 300 um but 220 um on B.Cu.
fn layer_width_dsn() -> String {
    let pins: String = (0..5).map(|i| format!("(pin PAD {} {} 0)", i + 1, i * 600)).collect();
    let nets: String = (1..=5).map(|i| format!("(net N{i} (pins J1-{i} J2-{i}))")).collect();
    let names: String = (1..=5).map(|i| format!("N{i} ")).collect();
    format!(
        r#"(pcb layer.dsn (parser (string_quote ") (space_in_quoted_tokens on) (host_cad "KiCad's Pcbnew") (host_version "10.0.5"))
  (resolution um 10) (unit um)
  (structure
    (layer F.Cu (type signal) (property (index 0))) (layer B.Cu (type signal) (property (index 1)))
    (boundary (path pcb 0  0 0  12000 0  12000 -9000  0 -9000  0 0))
    (via "Via[0-1]_600:300_um")
    (rule (width {CLASS_UM}) (clearance 160))
    (autoroute_settings (layer_rule F.Cu (active off))))
  (placement (component "CONN" (place J1 4000 -1500 front 0 (PN a)) (place J2 4000 -7500 front 0 (PN b))))
  (library
    (image "CONN" {pins})
    (padstack PAD (shape (polygon F.Cu 0 -150 300 150 300 150 -300 -150 -300))
                  (shape (polygon B.Cu 0 -150 300 150 300 150 -300 -150 -300)) (attach off))
    (padstack "Via[0-1]_600:300_um" (shape (circle F.Cu 600)) (shape (circle B.Cu 600)) (attach off)))
  (network {nets}
    (class kicad_default {names} (circuit (use_via "Via[0-1]_600:300_um"))
      (rule (width {CLASS_UM}) (clearance 160))
      (layer_rule B.Cu (rule (width 220))))))"#
    )
}

fn write_dsn(name: &str, text: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("strict-width-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

/// Routes `dsn` over the protocol with `extra` router settings; returns the SES text and the unrouted-net count.
fn route(dsn: &PathBuf, extra: Value) -> (String, u64) {
    let mut child = Command::new(BIN).arg("serve").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut call = |id: i64, op: &str, args: Value| -> Value {
        writeln!(stdin, "{}", json!({ "id": id, "op": op, "args": args })).unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        assert!(stdout.read_line(&mut line).unwrap() > 0, "server closed during {op}");
        let r: Value = fr_serve::serde_json::from_str(&line).unwrap();
        assert_eq!(r["ok"], json!(true), "{op}: {r}");
        r["result"].clone()
    };
    let mut settings = json!({ "router.automatic_neckdown": true });
    settings.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    call(1, "hello", json!({ "protocol": "1.0.0", "client": "pcbkit_strict_width", "threads": 1, "settings": settings }));
    call(2, "load", json!({ "dsn": { "path": dsn.to_str().unwrap() } }));
    let routed = call(3, "route", json!({ "seed": 0, "nets": "all", "from": "scratch" }));
    let unrouted = routed["unrouted"].as_array().map_or_else(|| routed["unrouted"].as_u64().unwrap_or(0), |a| a.len() as u64);
    let ses = call(4, "export", json!({ "format": "ses" }))["text"].as_str().unwrap().to_string();
    call(5, "shutdown", json!({}));
    drop(stdin);
    child.wait().unwrap();
    (ses, unrouted)
}

/// `(layer, width in um)` of every wire of an SES.
fn wire_widths(ses: &str) -> Vec<(String, f64)> {
    let t: Vec<&str> = ses.split(|c: char| c.is_whitespace() || c == '(' || c == ')').filter(|x| !x.is_empty()).collect();
    let res = t.windows(3).find(|w| w[0] == "resolution").map(|w| w[2].parse::<f64>().unwrap()).unwrap();
    let unit_um = match t.windows(3).find(|w| w[0] == "resolution").map(|w| w[1]) {
        Some("mm") => 1000.0,
        Some("mil") => 25.4,
        Some("inch") => 25_400.0,
        _ => 1.0,
    } / res;
    t.windows(3).filter(|w| w[0] == "path").map(|w| (w[1].to_string(), w[2].parse::<f64>().unwrap() * unit_um)).collect()
}

fn narrowest(ses: &str) -> f64 {
    wire_widths(ses).iter().map(|w| w.1).fold(f64::INFINITY, f64::min)
}

#[test]
fn default_off_the_necked_retry_narrows_a_connection_to_the_neck_width() {
    let dsn = write_dsn("neck.dsn", &fine_pitch_dsn());
    let (ses, unrouted) = route(&dsn, json!({ "router.min_trace_width_um": 250, "router.neck_width_um": 100 }));
    assert_eq!(unrouted, 0);
    assert!(narrowest(&ses) < 250.0 - SLACK_UM, "without strict_width the retry should neck below the floor: {}", narrowest(&ses));
}

#[test]
fn strict_width_never_routes_below_the_declared_neck_width_and_reports_unrouted() {
    let dsn = write_dsn("neck.dsn", &fine_pitch_dsn());
    let settings = json!({ "router.min_trace_width_um": 250, "router.neck_width_um": 100, "router.strict_width": true, "router.neck_radius_um": 1000 });
    let (ses, unrouted) = route(&dsn, settings);
    assert!(narrowest(&ses) >= 250.0 - SLACK_UM, "wire of {} um under a 250 um neck floor", narrowest(&ses));
    assert!(unrouted > 0, "connections that do not fit at class width or neck width must be reported unrouted");
}

#[test]
fn strict_width_still_allows_the_declared_neck_at_the_pad() {
    let dsn = write_dsn("neck.dsn", &fine_pitch_dsn());
    let settings = json!({ "router.min_trace_width_um": 150, "router.strict_width": true, "router.neck_radius_um": 1000 });
    let (ses, unrouted) = route(&dsn, settings);
    assert_eq!(unrouted, 0);
    let widths = wire_widths(&ses);
    assert!(widths.iter().any(|w| w.1 < CLASS_UM - SLACK_UM), "the 400 um pitch row needs a neck: {widths:?}");
    assert!(narrowest(&ses) >= 150.0 - SLACK_UM);
    assert!(widths.iter().any(|w| (w.1 - CLASS_UM).abs() < SLACK_UM), "away from the pads the class width is kept: {widths:?}");
}

#[test]
fn a_class_layer_rule_width_is_what_the_router_draws_on_that_layer() {
    let dsn = write_dsn("layer.dsn", &layer_width_dsn());
    let (ses, unrouted) = route(&dsn, json!({}));
    assert_eq!(unrouted, 0);
    let widths = wire_widths(&ses);
    assert!(!widths.is_empty());
    assert!(widths.iter().all(|(layer, w)| layer == "B.Cu" && (w - 220.0).abs() < SLACK_UM), "{widths:?}");
}

#[test]
fn the_strict_last_resort_routes_in_recovery_what_strict_width_left_open() {
    let dsn = write_dsn("neck.dsn", &fine_pitch_dsn());
    let strict = json!({ "router.min_trace_width_um": 250, "router.neck_width_um": 100, "router.strict_width": true, "router.neck_radius_um": 1000, "router.recover_attempts": 2 });
    let (_, open_strict) = route(&dsn, strict.clone());
    assert!(open_strict > 0);
    let mut last = strict;
    last.as_object_mut().unwrap().insert("router.strict_last_resort".into(), json!(true));
    let (ses, open_last) = route(&dsn, last);
    assert!(open_last < open_strict, "the last resort should connect something strict left open ({open_last} vs {open_strict})");
    assert!(narrowest(&ses) < 250.0 - SLACK_UM, "and it does so below the floor, which is what it is for");
}

#[test]
fn a_strict_run_with_the_last_resort_never_leaves_more_open_than_the_plain_run() {
    // W3: without recovery attempts nothing else could connect what strict width left open; the plain fallback does
    let dsn = write_dsn("neck.dsn", &fine_pitch_dsn());
    let plain = json!({ "router.min_trace_width_um": 250, "router.neck_width_um": 100 });
    let (_, open_plain) = route(&dsn, plain.clone());
    let mut strict = plain.clone();
    strict.as_object_mut().unwrap().extend(json!({ "router.strict_width": true, "router.neck_radius_um": 1000 }).as_object().unwrap().clone());
    let (_, open_strict) = route(&dsn, strict.clone());
    assert!(open_strict > open_plain, "the board must make strict width lose connections ({open_strict} vs {open_plain})");
    strict.as_object_mut().unwrap().insert("router.strict_last_resort".into(), json!(true));
    let (_, open_last) = route(&dsn, strict);
    assert!(open_last <= open_plain, "strict with the last resort left {open_last} open, the plain run {open_plain}");
}
