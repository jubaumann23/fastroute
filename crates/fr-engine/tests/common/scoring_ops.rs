//! The DRC and scoring operations of the U6 replay vectors (`testdata/scoring/*.txt`, generated
//! by `testdata/scoring/ScoreGen.java`). Included by `board_replay.rs`.
//!
//! Setup operations (after `BOARD`): `TOLERANCE`, `HOSTVERSION`, `VIAINFO`, `VIARULE`,
//! `NCRULES`; state operations: `ESCVIA`, `PREEXIST`; `SCORE` dumps the clearance violations,
//! incompletes, airline count, pin escapes, three `BoardStatistics` variants and their scores.
//! `FR_SCORING_EXPAND=1` (with vectors from `ScoreGen ... expand`) compares the hashed blocks
//! line by line; `FR_SCORING_TIMING=1` prints the run times of the DRC/statistics calls.

use std::fmt::Write as _;

use fr_engine::board::*;
use fr_engine::drc::{all_clearance_violations, clearance_violations, ClearanceViolation, DesignRulesChecker};
use fr_engine::ids::ItemId;
use fr_engine::rules::{ViaInfo, ViaRule};
use fr_engine::scoring::{is_pin_escaped, BoardStatistics, Rect2DF};
use fr_engine::structure::Unit;
use fr_settings::{OptimizerScoringVersion, RouterScoringVersion, RouterSettings};

use super::{b, block, d, tile, Tok};

fn key(board: &RoutingBoard, t: &mut Tok) -> ItemKey {
    let id = t.i32();
    board.get_item(ItemId(id)).unwrap_or_else(|| panic!("item {id} not found"))
}

/// Java `BoardGen.str`: hex of the UTF-8 bytes, `-` for null, `~` for empty.
fn hex_str(s: Option<&str>) -> String {
    match s {
        None => "-".into(),
        Some("") => "~".into(),
        Some(s) => s.bytes().map(|x| format!("{x:02x}")).collect(),
    }
}

fn f(v: Option<f32>) -> String {
    v.map_or_else(|| "-".into(), |v| format!("{:x}", v.to_bits()))
}

fn f32h(v: f32) -> String {
    format!("{:x}", v.to_bits())
}

fn dd(v: Option<f64>) -> String {
    v.map_or_else(|| "-".into(), d)
}

fn i(v: Option<i32>) -> String {
    v.map_or_else(|| "-".into(), |v| v.to_string())
}

fn rect(r: Option<Rect2DF>) -> String {
    r.map_or_else(|| "-".into(), |r| format!("{},{},{},{}", f32h(r.x), f32h(r.y), f32h(r.width), f32h(r.height)))
}

fn violation(board: &BasicBoard, v: &ClearanceViolation) -> String {
    format!(
        "{} {} {} {} {} {} {} {}",
        board.item(v.first_item).id().0,
        board.item(v.second_item).id().0,
        v.layer,
        d(v.expected_clearance),
        d(v.actual_clearance),
        tile(&v.shape),
        b(v.is_unfixable(board)),
        v.category(board) as i32
    )
}

/// `FR_SCORING_EXPAND=1` writes the hashed blocks expanded (for vectors generated with
/// `ScoreGen ... expand`).
fn expand() -> bool {
    std::env::var_os("FR_SCORING_EXPAND").is_some()
}

/// Executes a scoring operation. `None` if the operation is unknown.
pub fn execute(board: &mut RoutingBoard, op: &str, t: &mut Tok) -> Option<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    match op {
        "TOLERANCE" => board.rules_mut().clearance_tolerance_um = t.f64(),
        "HOSTVERSION" => {
            let v = t.str_opt();
            board.communication.specctra_parser_info.as_mut().unwrap().host_version = v;
        }
        "VIAINFO" => {
            let name = t.str();
            let ps = t.i32();
            let cl = t.i32();
            let attach = t.bool();
            board.rules_mut().via_infos.add(ViaInfo::new(name, ps, cl, attach)).expect("duplicate via info");
        }
        "VIARULE" => {
            let name = t.str();
            let n = t.i32();
            let rules = board.rules_mut();
            let mut rule = ViaRule::new(name);
            for _ in 0..n {
                let index = t.i32();
                rule.append_via(rules.via_infos.get(index));
            }
            rules.via_rules.add(rule);
        }
        "NCRULES" => {
            let class_index = t.i32();
            let rule_index = t.i32();
            let min = t.f64();
            let max = t.f64();
            let rules = board.rules_mut();
            let rule = if rule_index >= 0 { Some(rules.via_rules.get(rule_index as usize)) } else { None };
            let id = rules.net_classes.get(class_index);
            let class = &mut rules.net_classes[id];
            class.set_via_rule(rule);
            class.set_minimum_trace_length(min);
            class.set_maximum_trace_length(max);
        }
        "ESCVIA" => {
            let k = key(board, t);
            let layer = t.i32();
            if let ItemKind::Via(v) = &mut board.item_mut(k).kind {
                v.is_escape_via = true;
                v.escape_via_smd_layer = layer;
            } else {
                panic!("ESCVIA: not a via");
            }
        }
        "PREEXIST" => board.pre_existing_clearance_violations_count = t.i32(),
        "SCORE" => score(board, &mut out),
        _ => return None,
    }
    Some(out)
}

/// `FR_SCORING_TIMING=1` prints the run time of the main calls (release build recommended).
fn timing(board: &BasicBoard) {
    use std::time::Instant;
    let t = Instant::now();
    let violations = all_clearance_violations(board).len();
    let t_cv = t.elapsed();
    let t = Instant::now();
    let mut drc = DesignRulesChecker::new();
    drc.calculate_all_incompletes(board);
    let incompletes = drc.get_incomplete_count(board);
    let t_inc = t.elapsed();
    let t = Instant::now();
    let bounds = fr_engine::scoring::calculate_bounds(board);
    let t_bounds = t.elapsed();
    let t = Instant::now();
    let _ = BoardStatistics::new_with_bounds(board, None, false, false, Some(bounds));
    let t_rest = t.elapsed();
    let t = Instant::now();
    let _ = BoardStatistics::from_board(board);
    let t_full = t.elapsed();
    eprintln!(
        "timing: {} items, {violations} violations, {incompletes} incompletes: clearance violations {t_cv:?}, incompletes {t_inc:?}, \
         bounds {t_bounds:?}, statistics without cv/connections/bounds {t_rest:?}, full statistics {t_full:?}",
        board.get_items().len()
    );
}

fn score(board: &BasicBoard, out: &mut Vec<String>) {
    if std::env::var_os("FR_SCORING_TIMING").is_some() {
        timing(board);
    }
    let all = all_clearance_violations(board);
    out.push(format!("V {}", all.len()));
    for v in &all {
        out.push(format!("v {}", violation(board, v)));
    }
    let mut raw = Vec::new();
    for k in board.get_items() {
        for v in clearance_violations(board, k) {
            raw.push(violation(board, &v));
        }
    }
    out.extend(block(format!("RAW {}", raw.len()), raw, expand()));

    let mut drc = DesignRulesChecker::new();
    drc.calculate_all_incompletes(board);
    let total = drc.get_incomplete_count(board);
    let length_violations = drc.get_length_violation_count(board);
    let mut s = format!("I {} {} {} [", drc.max_connections, total, length_violations);
    for net in 1..=board.rules.nets.max_net_number() {
        let ni = drc.get_net_incompletes(board, net).unwrap();
        if ni.count() > 0 || ni.connected_group_count() > 1 {
            write!(s, " {}:{}/{}", net, ni.count(), ni.connected_group_count()).unwrap();
        }
    }
    let nets: Vec<String> = drc.incomplete_net_numbers(board).iter().map(|n| n.to_string()).collect();
    write!(s, " ] [{}]", nets.join(", ")).unwrap();
    out.push(s);
    // Only the number of airlines is compared: which airlines Java creates depends on identity
    // hash order (see fr_engine::drc::net_incompletes).
    out.push(format!("AIRLINES {}", drc.get_all_airlines(board).len()));
    let escaped: Vec<String> =
        board.get_smd_pins().into_iter().map(|p| format!("{} {}", board.item(p).id().0, b(is_pin_escaped(board, p)))).collect();
    out.extend(block(format!("ESCAPED {}", escaped.len()), escaped, expand()));

    let defaults = fr_settings::default_settings(1);
    let mut v1 = defaults.clone();
    v1.router_scoring.version = Some(RouterScoringVersion::V1Legacy);
    v1.optimizer_scoring.version = Some(OptimizerScoringVersion::V1Legacy);
    let mut v2 = defaults.clone();
    v2.router_scoring.version = Some(RouterScoringVersion::V2Continuous);
    v2.optimizer_scoring.version = Some(OptimizerScoringVersion::V2LowerBound);
    let mut full = BoardStatistics::from_board(board);
    stats(out, "full", &full);
    scores(out, &mut full, &defaults, &v1, &v2);
    let mut no_violations = BoardStatistics::new(board, None, false, true);
    stats(out, "nocv", &no_violations);
    scores(out, &mut no_violations, &defaults, &v1, &v2);
    let mut inch = BoardStatistics::new(board, Some(Unit::Inch), false, false);
    stats(out, "inch", &inch);
    let o = inch.get_optimizer_score(Some(&v2));
    let r = inch.get_router_score(Some(&v2));
    out.push(format!("SC inch {} {}", f32h(o), f32h(r)));
}

fn scores(out: &mut Vec<String>, s: &mut BoardStatistics, defaults: &RouterSettings, v1: &RouterSettings, v2: &RouterSettings) {
    let values = [
        s.get_router_score(Some(defaults)),
        s.get_optimizer_score(Some(defaults)),
        s.get_router_score(Some(v1)),
        s.get_optimizer_score(Some(v1)),
        s.get_router_score(Some(v2)),
        s.get_optimizer_score(Some(v2)),
        s.get_router_score(None),
        s.get_optimizer_score(None),
        s.calculate_score(&defaults.scoring),
        s.get_maximum_score(&defaults.scoring),
    ];
    let v: Vec<String> = values.iter().map(|x| f32h(*x)).collect();
    out.push(format!("SC {}", v.join(" ")));
}

fn stats(out: &mut Vec<String>, label: &str, s: &BoardStatistics) {
    out.push(format!("S {} {} {}", label, hex_str(s.host.as_deref()), s.unit.as_deref().unwrap_or("null")));
    out.push(format!("S board {} {} {}", rect(s.board.bounding_box), rect(s.board.size), f(s.board.area_cm2)));
    out.push(format!("S layers {} {}", i(s.layers.total_count), i(s.layers.signal_count)));
    let it = &s.items;
    out.push(format!(
        "S items {} {} {} {} {} {} {} {}",
        i(it.total_count),
        i(it.trace_count),
        i(it.via_count),
        i(it.conduction_area_count),
        i(it.drill_item_count),
        i(it.pin_count),
        i(it.component_outline_count),
        i(it.other_count)
    ));
    out.push(format!(
        "S counts {} {} {} {}",
        i(s.components.total_count),
        i(s.pads.total_count),
        i(s.nets.total_count),
        i(s.nets.class_count)
    ));
    out.push(format!("S connections {} {}", i(s.connections.maximum_count), i(s.connections.incomplete_count)));
    let tr = &s.traces;
    out.push(format!(
        "S traces {} {} {} {} {} {} {} {} {}",
        i(tr.total_count),
        i(tr.total_segment_count),
        f(tr.total_length),
        f(tr.total_length_mm),
        f(tr.total_weighted_length),
        f(tr.average_length),
        f(tr.total_vertical_length),
        f(tr.total_horizontal_length),
        f(tr.total_angled_length)
    ));
    let be = &s.bends;
    out.push(format!(
        "S bends {} {} {} {}",
        i(be.total_count),
        i(be.ninety_degree_count),
        i(be.forty_five_degree_count),
        i(be.other_angle_count)
    ));
    let vi = &s.vias;
    out.push(format!("S vias {} {} {} {}", i(vi.total_count), i(vi.through_hole_count), i(vi.blind_count), i(vi.buried_count)));
    let cv = &s.clearance_violations;
    out.push(format!(
        "S cv {} {} {} {} {} {} {} {}",
        i(cv.total_count),
        i(cv.pre_existing_count),
        i(cv.unfixable_count),
        i(cv.router_introduced_count),
        dd(cv.total_violation_um),
        dd(cv.min_violation_um),
        dd(cv.max_violation_um),
        dd(cv.avg_violation_um)
    ));
    let di = &s.difficulty;
    out.push(format!(
        "S difficulty {} {} {} {} {}",
        i(di.pin_count),
        i(di.signal_layer_count),
        i(di.complexity_c),
        f(di.difficulty_d),
        f(di.board_area_cm2)
    ));
    out.push(format!(
        "S bounds {} {} {}",
        f(s.bounds.min_trace_length_mm),
        i(s.bounds.min_via_count),
        i(s.bounds.min_bend_count)
    ));
    out.push(format!("S fanout {} {} {}", s.fanout.total_smd_pins, s.fanout.pins_to_escape, s.fanout.escaped_count));
}
