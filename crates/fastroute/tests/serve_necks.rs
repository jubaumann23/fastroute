//! `widen` and `check` (protocol 1.2) on tiny synthetic boards: x does y, no corpus board.
//!
//! The boards are `tiny.dsn` plus a hand-written initial session (narrow wires, imported unfixed), so
//! the expected widths follow from the clearance rule by hand: class width 1524 units, clearance 1524.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use fr_serve::serde_json::{json, Value};

const BIN: &str = env!("CARGO_BIN_EXE_fastroute");
const CLASS: i64 = 1524;

struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl Server {
    fn start() -> Server {
        let mut child = Command::new(BIN)
            .arg("serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn fastroute serve");
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Server { child, stdin, stdout, next_id: 1 }
    }

    fn call(&mut self, op: &str, args: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let line = json!({ "id": id, "op": op, "args": args }).to_string();
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{line}").unwrap();
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

    fn finish(mut self) {
        self.ok("shutdown", json!({}));
        drop(self.stdin.take());
        assert!(self.child.wait().unwrap().success());
    }
}

fn tiny() -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fr-serve/tests/data/tiny.dsn");
    p.canonicalize().unwrap().to_str().unwrap().to_string()
}

/// A session with the four parts of tiny.dsn and the given `(net ... (wire ...))` blocks.
fn ses(nets: &str) -> String {
    format!(
        "(session tiny (base_design tiny) (placement (resolution um 10) (component \"R_0603\" \
         (place R1 140000 -140000 front 0) (place R2 260000 -140000 front 0) \
         (place R3 140000 -210000 front 90) (place R4 260000 -210000 front 0))) (was_is) \
         (routes (resolution um 10) (library_out (padstack \"Via[0-1]_600:300_um\" \
         (shape (circle F.Cu 6000 0 0)) (shape (circle B.Cu 6000 0 0)) (attach off))) \
         (network_out {nets})))"
    )
}

fn wire(net: &str, layer: &str, width: i64, pts: &[(i64, i64)]) -> String {
    let p: Vec<String> = pts.iter().map(|(x, y)| format!("{x} {y}")).collect();
    format!("(net {net} (wire (path {layer} {width} {})))", p.join("  "))
}

fn open(nets: &str) -> Server {
    let mut s = Server::start();
    s.ok("hello", json!({ "protocol": "1.2.0", "client": "serve_necks", "threads": 1 }));
    s.ok("load", json!({ "dsn": { "path": tiny() }, "ses": { "text": ses(nets) } }));
    s
}

/// `(net, layer, width, [corners])` of every wire path in an SES.
fn paths(ses: &str) -> Vec<(String, i64, Vec<(i64, i64)>)> {
    let mut out = Vec::new();
    let mut net = String::new();
    let toks: Vec<&str> = ses.split_whitespace().collect();
    let mut i = 0;
    while i < toks.len() {
        if toks[i] == "(net" {
            net = toks[i + 1].to_string();
        }
        if toks[i] == "(path" {
            let width: i64 = toks[i + 2].parse().unwrap();
            let mut pts = Vec::new();
            let mut j = i + 3;
            while j + 1 < toks.len() && !toks[j].starts_with(')') && !toks[j + 1].starts_with(')') {
                pts.push((toks[j].parse().unwrap(), toks[j + 1].trim_end_matches(')').parse().unwrap()));
                j += 2;
            }
            out.push((net.clone(), width, pts));
        }
        i += 1;
    }
    out
}

fn export(s: &mut Server) -> String {
    s.ok("export", json!({ "format": "ses" }))["text"].as_str().unwrap().to_string()
}

fn count(r: &Value, key: &str) -> i64 {
    r["counts"][key].as_i64().unwrap()
}

#[test]
fn capabilities_and_errors() {
    let mut s = Server::start();
    let h = s.ok("hello", json!({ "protocol": "1.2.0", "client": "t", "threads": 1 }));
    assert_eq!(h["protocol"], "1.2.0");
    for cap in ["widen", "check"] {
        assert!(h["capabilities"].as_array().unwrap().contains(&json!(cap)), "{h}");
    }
    s.err("widen", json!({}), "not_loaded");
    s.err("check", json!({}), "not_loaded");
    s.ok("load", json!({ "dsn": { "path": tiny() } }));
    s.err("widen", json!({ "bogus": 1 }), "bad_request");
    s.err("widen", json!({ "widths": { "NOPE": 1000 } }), "unknown_net");
    s.err("widen", json!({ "nets": ["NOPE"] }), "unknown_net");
    s.err("widen", json!({ "widths": { "N1": "wide" } }), "bad_request");
    s.err("check", json!({ "max": -1 }), "bad_request");
    s.err("check", json!({ "bogus": 1 }), "bad_request");
    s.finish();
}

#[test]
fn widen_restores_class_width_where_there_is_room() {
    // N1 and N4 routed at 600 units in free space: every segment goes back to the class width
    let nets = format!(
        "{} {}",
        wire("N1", "F.Cu", 600, &[(160000, -175000), (200000, -175000), (215000, -160000)]),
        wire("N4", "B.Cu", 600, &[(160000, -185000), (240000, -185000)]),
    );
    let mut s = open(&nets);
    let c = s.ok("check", json!({}));
    assert_eq!(count(&c, "width_class"), 3, "{c}");
    let w = s.ok("widen", json!({}));
    assert_eq!((w["narrow_before"].as_i64(), w["widened"].as_i64(), w["left_narrow"].as_i64()), (Some(3), Some(3), Some(0)), "{w}");
    assert_eq!(w["classes"][0]["min_width"], CLASS);
    let after = s.ok("check", json!({}));
    assert_eq!(count(&after, "width_class"), 0, "{after}");
    assert_eq!(count(&after, "clearance_track_track"), 0);
    // the export carries the widened widths
    assert!(paths(&export(&mut s)).iter().all(|(_, w, _)| *w == CLASS));
    // a second widen has nothing to do
    let again = s.ok("widen", json!({}));
    assert_eq!((again["narrow_before"].as_i64(), again["widened"].as_i64()), (Some(0), Some(0)));
    s.finish();
}

#[test]
fn widen_stops_at_the_clearance_of_a_neighbour() {
    // two parallel 600-unit wires 2500 apart (centre to centre). At the class width 1524 each, the
    // edges would be 1000 apart (< 1524). N1 alone can reach d - clearance - other half = 2500 - 1524 - 300 =
    // 676 half width, i.e. a full width of 1352 (bisection stops within 10 units).
    let nets = format!(
        "{} {}",
        wire("N1", "F.Cu", 600, &[(160000, -175000), (240000, -175000)]),
        wire("N3", "F.Cu", 600, &[(160000, -177500), (240000, -177500)]),
    );
    let mut s = open(&nets);
    assert_eq!(count(&s.ok("check", json!({})), "clearance_track_track"), 0);
    let w = s.ok("widen", json!({ "nets": ["N1"] }));
    assert_eq!((w["widened"].as_i64(), w["partial"].as_i64(), w["left_narrow"].as_i64()), (Some(1), Some(1), Some(1)), "{w}");
    let width = w["nets"][0]["min_width"].as_i64().unwrap();
    assert!((1342..=1352).contains(&width), "{w}");
    assert_eq!(w["segments"], 1);
    assert_eq!(w["nets"][0]["net"], "N1");
    let after = s.ok("check", json!({}));
    assert_eq!(count(&after, "clearance_track_track"), 0, "{after}");
    // N3 was not asked for and kept its width
    let widths: Vec<_> = paths(&export(&mut s)).into_iter().map(|(n, w, _)| (n, w)).collect();
    assert!(widths.contains(&("N3".into(), 600)), "{widths:?}");
    // both nets: N3 now has no room at all and stays narrow, the result still has no violation
    let w2 = s.ok("widen", json!({}));
    // N1 sits at its limit; N3 gains the 1-2 units the rest of the gap leaves
    assert!(w2["widened"].as_i64().unwrap() <= 1, "{w2}");
    assert_eq!(w2["left_narrow"], 2, "{w2}");
    assert_eq!(count(&s.ok("check", json!({})), "clearance_track_track"), 0);
    s.finish();
}

#[test]
fn widen_splits_a_trace_so_one_squeeze_does_not_hold_the_rest() {
    // one two-segment N1 trace; N3 runs 2500 below only its second half. The first segment reaches the
    // class width, the second stays partial.
    let nets = format!(
        "{} {}",
        wire("N1", "F.Cu", 600, &[(160000, -175000), (200000, -175000), (215000, -160000)]),
        wire("N3", "F.Cu", 600, &[(165000, -177500), (190000, -177500)]),
    );
    let mut s = open(&nets);
    let w = s.ok("widen", json!({ "nets": ["N1"] }));
    assert_eq!((w["segments"].as_i64(), w["widened"].as_i64(), w["left_narrow"].as_i64()), (Some(2), Some(2), Some(1)), "{w}");
    let widths: Vec<i64> = paths(&export(&mut s)).into_iter().filter(|(n, _, _)| n == "N1").map(|(_, w, _)| w).collect();
    assert!(widths.contains(&CLASS) && widths.iter().any(|w| (1342..=1352).contains(w)), "{widths:?}");
    assert_eq!(count(&s.ok("check", json!({})), "clearance_track_track"), 0);
    s.finish();
}

#[test]
fn widen_takes_a_per_net_width_and_never_touches_fixed_wiring() {
    let nets = wire("N1", "F.Cu", 600, &[(160000, -175000), (240000, -175000)]);
    let mut s = open(&nets);
    // the client asks for 2000 on N1 (above the class width of 1524)
    let w = s.ok("widen", json!({ "widths": { "N1": 2000 } }));
    assert_eq!(w["widened"], 1, "{w}");
    assert_eq!(paths(&export(&mut s))[0].1, 2000);
    s.finish();
    // locked wiring is never touched: it is counted as left narrow
    let mut s = open(&nets);
    s.ok("lock", json!({ "nets": ["N1"] }));
    let w = s.ok("widen", json!({}));
    assert_eq!((w["narrow_before"].as_i64(), w["widened"].as_i64(), w["left_narrow"].as_i64()), (Some(1), Some(0), Some(1)), "{w}");
    assert_eq!(paths(&export(&mut s))[0].1, 600);
    s.finish();
}

#[test]
fn widen_is_deterministic_and_atomic() {
    let nets = format!(
        "{} {}",
        wire("N1", "F.Cu", 600, &[(160000, -175000), (240000, -175000)]),
        wire("N3", "F.Cu", 600, &[(160000, -177500), (240000, -177500)]),
    );
    let run = || {
        let mut s = open(&nets);
        let w = s.ok("widen", json!({}));
        let out = (export(&mut s), w["widened"].clone(), w["left_narrow"].clone());
        s.finish();
        out
    };
    assert_eq!(run(), run());
    // a refused request leaves the wiring as it was
    let mut s = open(&nets);
    let before = export(&mut s);
    s.err("widen", json!({ "nets": ["N1", "NOPE"] }), "unknown_net");
    assert_eq!(before, export(&mut s));
    s.finish();
}

#[test]
fn check_counts_each_kind_on_hand_built_boards() {
    // clearance track-track: two wires 1000 apart centre to centre at width 600 (edges 400 apart < 1524)
    let nets = format!(
        "{} {}",
        wire("N1", "F.Cu", 1524, &[(160000, -175000), (240000, -175000)]),
        wire("N3", "F.Cu", 1524, &[(160000, -176500), (240000, -176500)]),
    );
    let mut s = open(&nets);
    let c = s.ok("check", json!({ "max": 5 }));
    assert_eq!(count(&c, "clearance_track_track"), 1, "{c}");
    assert_eq!(c["summary"]["clearance"], 1);
    let item = &c["items"][0];
    assert_eq!(item["type"], "clearance_track_track");
    assert!(["N1", "N3"].contains(&item["net"].as_str().unwrap()), "{item}");
    assert_eq!(item["layer"], "F.Cu");
    assert_eq!(item["required"], CLASS);
    assert!(item["at"].as_array().is_some_and(|a| a.len() == 2));
    // all four connections are open: nothing connects a pad
    assert_eq!(count(&c, "unconnected"), 4, "{c}");
    assert!(c["not_checked"].as_array().unwrap().contains(&json!("zone_fill")));
    // the item list is capped and says so
    let capped = s.ok("check", json!({ "max": 1 }));
    assert_eq!(capped["items"].as_array().unwrap().len(), 1);
    assert_eq!(capped["truncated"], true);
    // clearance track-pad: a wire across the R1 pad row, other net
    s.finish();
    let mut s = open(&wire("N3", "F.Cu", 1524, &[(120000, -140000), (160000, -140000)]));
    let c = s.ok("check", json!({}));
    assert!(count(&c, "clearance_track_pad") >= 1, "{c}");
    s.finish();
}

#[test]
fn check_width_and_via_rules_come_from_the_request() {
    let mut s = open(&wire("N1", "F.Cu", 1000, &[(160000, -175000), (240000, -175000)]));
    // class width 1524: below class; the minimum is asked for explicitly
    let c = s.ok("check", json!({ "min_width": 1200 }));
    assert_eq!((count(&c, "width_class"), count(&c, "width_min")), (1, 1), "{c}");
    assert_eq!(c["rules"]["min_width"], 1200);
    let c = s.ok("check", json!({ "min_width": 900 }));
    assert_eq!(count(&c, "width_min"), 0, "{c}");
    // the via padstack is Via[0-1]_600:300_um: pad 6000 units, drill 3000, annular ring 1500
    let c = s.ok("check", json!({ "min_annular": 1400, "min_drill": 2900 }));
    assert_eq!((count(&c, "annular_ring"), count(&c, "drill")), (0, 0));
    s.finish();
}
