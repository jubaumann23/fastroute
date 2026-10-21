//! High-speed routing stages of `route` (capabilities `pairs` and `tune`): differential pair coupling
//! and length matching, run in the order the stock CLI runs them (crates/fastroute/src/main.rs):
//! pre-route and couple the pairs and hold them fixed, autoroute, release the pairs (route again if
//! connections are left), couple again, match the pair skew, tune the length groups.
//! Pure fr-serve logic over the public `fr_engine::{diffpair, tuning}` API; no core change.

use fr_engine::board::RoutingBoard;
use fr_engine::datastructures::StopToken;
use fr_engine::diffpair::{DiffPair, PairResult, SkewResult};
use fr_engine::ids::{FixedState, ItemId};
use fr_engine::tuning::{GroupTuning, TuneGroup};
use fr_settings::RouterSettings;
use serde_json::{json, Map, Value};

use crate::ops::lock::LockRegistry;
use crate::proto::{as_obj, as_str, check_keys, req, ProtoError, R};

/// `route.pairs`: differential pairs are coupled and length matched.
pub const PAIRS_CLAIMED: bool = true;
/// `route.tune`: length matching groups.
pub const TUNE_CLAIMED: bool = true;

/// Default allowed length difference of a pair, mm (the CLI's `--pair-skew`).
pub const DEFAULT_PAIR_SKEW_MM: f64 = 0.1;
/// Default tolerance of a tuning group, mm (the CLI's tune file default).
pub const DEFAULT_TUNE_TOLERANCE_MM: f64 = 0.5;
const MAX_COUNT: usize = 256;
const MAX_MM: f64 = 10_000.0;

/// The `pairs`, `pair_skew_mm`, `tune` and `layer_heights_mm` arguments of `route`, validated.
#[derive(Clone, Debug, Default)]
pub struct Request {
    pub pairs: Vec<DiffPair>,
    pub pair_skew_mm: f64,
    pub tune: Vec<TuneGroup>,
    pub layer_heights_mm: Vec<f64>,
}

impl Request {
    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty() && self.tune.is_empty()
    }

    pub fn parse(args: &Map<String, Value>) -> R<Request> {
        let mut out = Request { pair_skew_mm: DEFAULT_PAIR_SKEW_MM, ..Request::default() };
        if let Some(v) = args.get("pairs") {
            if !PAIRS_CLAIMED {
                return Err(ProtoError::unsupported("route.pairs", "pairs"));
            }
            out.pairs = parse_pairs(v)?;
        }
        if let Some(v) = args.get("pair_skew_mm") {
            out.pair_skew_mm = mm(v, 0.0, "route.pair_skew_mm")?;
        }
        if let Some(v) = args.get("tune") {
            if !TUNE_CLAIMED {
                return Err(ProtoError::unsupported("route.tune", "tune"));
            }
            out.tune = parse_tune(v)?;
        }
        if let Some(v) = args.get("layer_heights_mm") {
            let list = v.as_array().ok_or_else(|| ProtoError::bad_request("route.layer_heights_mm must be a list of numbers"))?;
            out.layer_heights_mm = list.iter().map(|x| mm(x, f64::MIN, "route.layer_heights_mm item")).collect::<R<_>>()?;
        }
        Ok(out)
    }
}

fn mm(v: &Value, lo: f64, what: &str) -> R<f64> {
    match v.as_f64() {
        Some(x) if x.is_finite() && x >= lo && x <= MAX_MM => Ok(x),
        _ => Err(ProtoError::bad_request(format!("{what} must be a number in {lo}..={MAX_MM} (mm)"))),
    }
}

fn parse_pairs(v: &Value) -> R<Vec<DiffPair>> {
    let list = v.as_array().ok_or_else(|| ProtoError::bad_request("route.pairs must be a list"))?;
    if list.len() > MAX_COUNT {
        return Err(ProtoError::bad_request(format!("route.pairs has more than {MAX_COUNT} items")));
    }
    let mut out: Vec<DiffPair> = Vec::with_capacity(list.len());
    for item in list {
        let o = as_obj(item, "route.pairs item")?;
        check_keys(o, &["p", "n", "gap_mm", "layer_gaps_mm", "skew_mm"], "route.pairs item")?;
        let p = as_str(req(o, "p", "route.pairs item")?, "route.pairs item.p")?.to_string();
        let n = as_str(req(o, "n", "route.pairs item")?, "route.pairs item.n")?.to_string();
        if p.is_empty() || n.is_empty() || p == n {
            return Err(ProtoError::bad_request("route.pairs item: p and n must be two different non-empty net names"));
        }
        if out.iter().any(|x| [&x.p, &x.n].iter().any(|m| **m == p || **m == n)) {
            return Err(ProtoError::bad_request(format!("route.pairs: net of pair {p}/{n} is in another pair already")));
        }
        let gap_mm = o.get("gap_mm").map(|g| mm(g, f64::MIN_POSITIVE, "route.pairs item.gap_mm")).transpose()?;
        let skew_mm = o.get("skew_mm").map(|g| mm(g, 0.0, "route.pairs item.skew_mm")).transpose()?;
        let mut layer_gaps_mm = Vec::new();
        if let Some(g) = o.get("layer_gaps_mm") {
            for (layer, val) in as_obj(g, "route.pairs item.layer_gaps_mm")? {
                layer_gaps_mm.push((layer.clone(), mm(val, f64::MIN_POSITIVE, "route.pairs item.layer_gaps_mm value")?));
            }
        }
        out.push(DiffPair { p, n, gap_mm, layer_gaps_mm, skew_mm });
    }
    Ok(out)
}

fn parse_tune(v: &Value) -> R<Vec<TuneGroup>> {
    let list = v.as_array().ok_or_else(|| ProtoError::bad_request("route.tune must be a list"))?;
    if list.len() > MAX_COUNT {
        return Err(ProtoError::bad_request(format!("route.tune has more than {MAX_COUNT} items")));
    }
    let mut out: Vec<TuneGroup> = Vec::with_capacity(list.len());
    for item in list {
        let o = as_obj(item, "route.tune item")?;
        check_keys(o, &["name", "nets", "tolerance_mm", "target_mm"], "route.tune item")?;
        let name = as_str(req(o, "name", "route.tune item")?, "route.tune item.name")?.to_string();
        if name.is_empty() || out.iter().any(|g| g.name == name) {
            return Err(ProtoError::bad_request("route.tune: group names must be non-empty and unique"));
        }
        let nets_v = req(o, "nets", "route.tune item")?.as_array().ok_or_else(|| ProtoError::bad_request("route.tune item.nets must be a list"))?;
        let nets: Vec<String> = nets_v.iter().map(|n| as_str(n, "route.tune item.nets item").map(str::to_string)).collect::<R<_>>()?;
        if nets.is_empty() || nets.iter().any(String::is_empty) {
            return Err(ProtoError::bad_request("route.tune item.nets must hold non-empty net names (`*` matches any text)"));
        }
        let tolerance_mm = o.get("tolerance_mm").map(|t| mm(t, 0.0, "route.tune item.tolerance_mm")).transpose()?.unwrap_or(DEFAULT_TUNE_TOLERANCE_MM);
        let target_mm = o.get("target_mm").map(|t| mm(t, f64::MIN_POSITIVE, "route.tune item.target_mm")).transpose()?;
        out.push(TuneGroup { name, nets, tolerance_mm, target_mm });
    }
    Ok(out)
}

/// `route.pairs` / `route.tune` nets that must exist on the board: unknown pair nets are an error.
pub fn check_nets(board: &RoutingBoard, req: &Request) -> R<()> {
    for p in &req.pairs {
        for name in [&p.p, &p.n] {
            if net_no(board, name).is_none() {
                return Err(ProtoError::new("unknown_net", format!("route.pairs: the board has no net '{name}'")).with_details(json!({ "name": name })));
            }
        }
    }
    Ok(())
}

fn net_no(board: &RoutingBoard, name: &str) -> Option<i32> {
    (1..=board.rules.nets.max_net_number()).find(|&n| board.rules.nets.get(n).is_some_and(|x| x.name == name))
}

fn units_per_mm(board: &RoutingBoard) -> f64 {
    board.communication.resolution.max(1) as f64 * 1000.0
}

fn length_mm(board: &RoutingBoard, name: &str) -> Option<f64> {
    net_no(board, name).map(|n| fr_engine::tuning::net_length(board, n) / units_per_mm(board))
}

/// Why a net may not be changed by this request, or `None`.
fn excluded(name: &str, locks: &LockRegistry, scope: Option<&[String]>) -> Option<&'static str> {
    if locks.is_locked_net(name) {
        Some("locked")
    } else if scope.is_some_and(|s| !s.iter().any(|n| n == name)) {
        Some("not in route.nets")
    } else {
        None
    }
}

/// The pairs this request may touch (both nets free), and the skipped ones with the reason.
fn split_pairs(pairs: &[DiffPair], locks: &LockRegistry, scope: Option<&[String]>) -> (Vec<DiffPair>, Vec<(DiffPair, &'static str)>) {
    let mut run = Vec::new();
    let mut skipped = Vec::new();
    for p in pairs {
        match excluded(&p.p, locks, scope).or_else(|| excluded(&p.n, locks, scope)) {
            Some(why) => skipped.push((p.clone(), why)),
            None => run.push(p.clone()),
        }
    }
    (run, skipped)
}

/// What `begin` hands to the later stages.
pub struct State {
    run: Vec<DiffPair>,
    skipped: Vec<(DiffPair, &'static str)>,
    prerouted: Vec<PairResult>,
    held: Vec<ItemId>,
    /// longer net of each held pair (p, n, mm) at the moment it was released (reported, `held_mm`)
    held_mm: Vec<(String, String, f64)>,
    /// longer net of each pre-routed pair (p, n, mm) right after the pre-route, before anything else was routed
    pre_mm: Vec<(String, String, f64)>,
}

/// Stage 1 (before the pipeline): sets the layer heights and pre-routes the pairs, holding their
/// wiring fixed while the rest is routed.
pub fn begin(board: &mut RoutingBoard, settings: &RouterSettings, req: &Request, locks: &LockRegistry, scope: Option<&[String]>, stop: &StopToken) -> State {
    fr_engine::tuning::set_layer_heights_mm(req.layer_heights_mm.clone());
    let (run, skipped) = split_pairs(&req.pairs, locks, scope);
    let (prerouted, held) = if run.is_empty() {
        (Vec::new(), Vec::new())
    } else {
        fr_engine::diffpair::preroute_pairs(board, &run, settings, stop, Some(FixedState::UserFixed))
    };
    let pre_mm = lengths_of(board, &run);
    State { run, skipped, prerouted, held, held_mm: Vec::new(), pre_mm }
}

/// The longer net of each pair, mm.
fn lengths_of(board: &RoutingBoard, pairs: &[DiffPair]) -> Vec<(String, String, f64)> {
    pairs.iter().filter_map(|p| Some((p.p.clone(), p.n.clone(), length_mm(board, &p.p)?.max(length_mm(board, &p.n)?)))).collect()
}

/// True when pairs were pre-routed and are held fixed.
pub fn holds_pairs(state: &State) -> bool {
    !state.held.is_empty()
}

/// Stage 2 (after the first pipeline run): releases the held pairs. True when connections of the
/// nets `routable` (net number -> may change) are left, so the caller routes once more with the pairs free.
pub fn release(board: &mut RoutingBoard, state: &mut State, routable: &dyn Fn(i32) -> bool) -> bool {
    if state.held.is_empty() {
        return false;
    }
    state.held_mm = lengths_of(board, &state.run);
    fr_engine::diffpair::release_pairs(board, &state.held);
    state.held.clear();
    let mut nets = std::collections::BTreeSet::new();
    fr_engine::pipeline::stats::incomplete_count(board, Some(&mut nets));
    nets.into_iter().any(|n| routable(n))
}

/// The longer net of each pre-routed pair right after the pre-route, a complete pad to pad pair that nothing else had
/// touched: what the quick hold's guard measures the finished pair against. (The held pairs are fixed, so this is also
/// their length when they are released, `held_mm`.)
pub fn preroute_lengths(state: &State) -> Vec<(String, String, f64)> {
    state.pre_mm.clone()
}

/// Largest growth of a pair since it was pre-routed, from the report rows of `finish`: the pair's longer net now over
/// its longer net right after its pre-route. Pairs that were not routed (skipped, missing) or held nothing are not judged;
/// None when no pair is judged.
pub fn pair_growth(rows: &[Value], held: &[(String, String, f64)]) -> Option<f64> {
    rows.iter()
        .filter_map(|r| {
            let (p, n) = (r.get("p")?.as_str()?, r.get("n")?.as_str()?);
            let now = r.get("p_length_mm")?.as_f64()?.max(r.get("n_length_mm")?.as_f64()?);
            let before = held.iter().find(|(hp, hn, _)| hp == p && hn == n)?.2;
            (before > 0.0).then(|| now / before)
        })
        .reduce(f64::max)
}

/// Why a quick pair hold result is not kept (`router.pair_quick_check` and the length guard `router.pair_quick_max_growth`), or
/// None when it is kept. `narrow_left` is the narrow segments the finished route has, counted by `ops::widen::narrow_after_widen`
/// (the toolkit's count: `widen`, then `check`); ANY of them fails the result, because the held run is no reference for what the
/// default would leave (round 8: stm32io s22 had 9 narrow segments after the quick run's held part and after the release, the
/// default's full held run ends with none), and the fallback IS the default, so a result with narrow segments is never worse
/// than the default's. `limit` below 1 (0 = off) drops the length condition.
pub fn quick_verdict(growth: Option<f64>, limit: f64, narrow_left: u32, unrouted: usize) -> Option<String> {
    if unrouted > 0 {
        return Some(format!("{unrouted} connection(s) open"));
    }
    if narrow_left > 0 {
        return Some(format!("{narrow_left} narrow segment(s) left"));
    }
    match growth {
        Some(g) if limit >= 1.0 && g > limit => Some(format!("a pair is {:.1} % longer than when it was pre-routed (limit {:.1} %)", (g - 1.0) * 100.0, (limit - 1.0) * 100.0)),
        _ => None,
    }
}

/// Results of stages 3 to 5.
pub struct Report {
    pub pairs: Vec<Value>,
    pub groups: Vec<Value>,
}

/// Stages 3 to 5 (after routing): couple the pairs again, match their skew, tune the groups.
pub fn finish(
    board: &mut RoutingBoard,
    settings: &RouterSettings,
    req: &Request,
    state: State,
    locks: &LockRegistry,
    scope: Option<&[String]>,
    stop: &StopToken,
) -> Report {
    let mut coupled: Vec<PairResult> = Vec::new();
    let mut skews: Vec<SkewResult> = Vec::new();
    if !state.run.is_empty() && !stop.is_stop_requested() {
        crate::route::trace("couple_pairs start");
        coupled = fr_engine::diffpair::couple_pairs(board, &state.run, settings, stop);
        crate::route::trace("couple_pairs done; match_pair_lengths start");
        skews = fr_engine::diffpair::match_pair_lengths(board, &state.run, settings, req.pair_skew_mm, stop);
    }
    crate::route::trace("match_pair_lengths done");
    let mut pairs: Vec<Value> = Vec::with_capacity(req.pairs.len());
    for pair in &req.pairs {
        pairs.push(pair_row(board, req, pair, &state, &coupled, &skews));
    }
    let groups = if stop.is_stop_requested() { Vec::new() } else { tune_groups(board, req, locks, scope) };
    let groups = if groups.is_empty() { req.tune.iter().map(|g| unrun_group(g, "stopped")).collect() } else { groups };
    fr_engine::tuning::set_layer_heights_mm(Vec::new());
    Report { pairs, groups }
}

fn pair_row(board: &RoutingBoard, req: &Request, pair: &DiffPair, state: &State, coupled: &[PairResult], skews: &[SkewResult]) -> Value {
    let tol = pair.skew_mm.unwrap_or(req.pair_skew_mm);
    let mut row = json!({ "p": pair.p, "n": pair.n, "tolerance_mm": tol });
    let o = row.as_object_mut().expect("object");
    if let Some((_, why)) = state.skipped.iter().find(|(s, _)| s.p == pair.p) {
        o.insert("status".into(), json!("skipped"));
        o.insert("kept".into(), json!(false));
        o.insert("message".into(), json!(why));
        return row;
    }
    let (Some(lp), Some(ln)) = (length_mm(board, &pair.p), length_mm(board, &pair.n)) else {
        o.insert("status".into(), json!("missing"));
        o.insert("kept".into(), json!(false));
        return row;
    };
    let last = coupled.iter().find(|r| r.p == pair.p && r.n == pair.n);
    let pre = state.prerouted.iter().find(|r| r.p == pair.p && r.n == pair.n);
    let skew = skews.iter().find(|r| r.p == pair.p && r.n == pair.n);
    let coupled_mm = last.map_or(0.0, |r| r.coupled_after_mm);
    // kept: the last coupling pass left a coupled run (an uncoupled pair is still routed, just not parallel)
    let kept = coupled_mm > 0.0;
    let skew_after = ln - lp;
    o.insert("status".into(), json!(if kept { "coupled" } else { "uncoupled" }));
    o.insert("kept".into(), json!(kept));
    o.insert("coupled_mm".into(), json!(coupled_mm));
    o.insert("preroute_coupled_mm".into(), json!(pre.map_or(0.0, |r| r.coupled_after_mm)));
    if let Some((_, _, pre)) = state.pre_mm.iter().find(|(hp, hn, _)| *hp == pair.p && *hn == pair.n) {
        o.insert("pre_mm".into(), json!(pre));
    }
    if let Some((_, _, held)) = state.held_mm.iter().find(|(hp, hn, _)| *hp == pair.p && *hn == pair.n) {
        // the pair's longer net when the hold ended: what the route made of it is judged against this (P5)
        o.insert("held_mm".into(), json!(held));
    }
    o.insert("p_length_mm".into(), json!(lp));
    o.insert("n_length_mm".into(), json!(ln));
    o.insert("skew_mm".into(), json!(skew_after));
    o.insert("skew_before_mm".into(), json!(skew.map_or(skew_after, |s| s.skew_before_mm)));
    o.insert("skew_ok".into(), json!(tol <= 0.0 || skew_after.abs() <= tol + 1e-6));
    o.insert("on_coupled_mm".into(), json!(skew.map_or(0.0, |s| s.on_coupled_mm)));
    let msg: Vec<&str> = [last.map(|r| r.message.as_str()), skew.map(|s| s.message.as_str())].into_iter().flatten().filter(|m| !m.is_empty()).collect();
    o.insert("message".into(), json!(msg.join("; ")));
    row
}

fn unrun_group(g: &TuneGroup, why: &str) -> Value {
    json!({ "name": g.name, "status": why, "target_mm": g.target_mm.unwrap_or(0.0), "tolerance_mm": g.tolerance_mm, "nets": [], "skipped": [], "short": 0 })
}

/// Net names (exact) of the board matching any pattern of the group (`*` = any text).
fn matching_nets(board: &RoutingBoard, patterns: &[String]) -> Vec<String> {
    (1..=board.rules.nets.max_net_number())
        .filter_map(|n| board.rules.nets.get(n).map(|x| x.name.clone()))
        .filter(|name| patterns.iter().any(|p| glob(p, name)))
        .collect()
}

fn glob(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let mut rest = text;
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            match rest.strip_prefix(part) {
                Some(r) => rest = r,
                None => return false,
            }
        } else if i == parts.len() - 1 {
            return rest.ends_with(part);
        } else if let Some(p) = rest.find(part) {
            rest = &rest[p + part.len()..];
        } else {
            return false;
        }
    }
    true
}

/// Tunes every group. Locked nets and nets outside `route.nets` are never changed; they still
/// count for the target when it is the longest net (so the others are matched to the real longest).
fn tune_groups(board: &mut RoutingBoard, req: &Request, locks: &LockRegistry, scope: Option<&[String]>) -> Vec<Value> {
    if req.tune.is_empty() {
        return Vec::new();
    }
    let mut rows = Vec::with_capacity(req.tune.len());
    for g in &req.tune {
        let matched = matching_nets(board, &g.nets);
        if matched.is_empty() {
            rows.push(unrun_group(g, "no_match"));
            continue;
        }
        let (free, fixed): (Vec<String>, Vec<String>) = matched.iter().cloned().partition(|n| excluded(n, locks, scope).is_none());
        let longest = matched.iter().filter_map(|n| length_mm(board, n)).fold(0.0, f64::max);
        let target = g.target_mm.unwrap_or(longest);
        let before_fixed: Vec<(String, f64)> = fixed.iter().map(|n| (n.clone(), length_mm(board, n).unwrap_or(0.0))).collect();
        let tuned: Option<GroupTuning> = if free.is_empty() {
            None
        } else {
            let one = TuneGroup { name: g.name.clone(), nets: free.clone(), tolerance_mm: g.tolerance_mm, target_mm: Some(target) };
            fr_engine::tuning::tune_lengths(board, std::slice::from_ref(&one)).into_iter().next()
        };
        let mut nets: Vec<Value> = tuned
            .iter()
            .flat_map(|t| t.nets.iter())
            .map(|n| json!({ "net": n.net, "before_mm": n.before_mm, "after_mm": n.after_mm, "short": n.after_mm < target - g.tolerance_mm - 1e-6 }))
            .collect();
        let short = nets.iter().filter(|n| n["short"] == json!(true)).count();
        let skipped: Vec<Value> = before_fixed
            .iter()
            .map(|(n, l)| json!({ "net": n, "length_mm": l, "reason": excluded(n, locks, scope).unwrap_or("") }))
            .collect();
        nets.sort_by(|a, b| a["net"].as_str().cmp(&b["net"].as_str()));
        rows.push(json!({
            "name": g.name,
            "status": "tuned",
            "target_mm": target,
            "tolerance_mm": g.tolerance_mm,
            "nets": nets,
            "skipped": skipped,
            "short": short,
        }));
    }
    rows
}

impl Report {
    pub fn into_json(self) -> (Value, Value) {
        (Value::Array(self.pairs), Value::Array(self.groups))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(v: Value) -> R<Request> {
        Request::parse(v.as_object().unwrap())
    }

    #[test]
    fn quick_verdict_keeps_only_a_clean_short_result() {
        assert_eq!(quick_verdict(Some(1.05), 1.05, 0, 0), None);
        assert_eq!(quick_verdict(None, 1.05, 0, 0), None);
        assert_eq!(quick_verdict(Some(0.9), 1.05, 0, 0), None);
        assert!(quick_verdict(Some(1.2), 1.05, 0, 0).unwrap().contains("20.0 % longer"));
        assert!(quick_verdict(Some(1.0), 1.05, 2, 0).unwrap().contains("narrow"));
        assert!(quick_verdict(Some(1.0), 1.05, 1, 0).unwrap().contains("1 narrow"));
        assert!(quick_verdict(Some(1.0), 1.05, 0, 1).unwrap().contains("open"));
    }

    #[test]
    fn quick_verdict_without_a_length_limit_judges_only_open_and_narrow() {
        assert_eq!(quick_verdict(Some(3.0), 0.0, 0, 0), None, "no length condition when the limit is 0");
        assert_eq!(quick_verdict(None, 0.0, 0, 0), None);
        assert!(quick_verdict(Some(3.0), 0.0, 1, 0).unwrap().contains("narrow"));
        assert!(quick_verdict(Some(3.0), 0.0, 0, 2).unwrap().contains("open"));
        assert!(quick_verdict(None, 0.0, 5, 0).unwrap().contains("5 narrow"));
        assert!(quick_verdict(None, 0.0, 5, 1).unwrap().contains("open"), "open connections are named first");
    }

    #[test]
    fn pair_growth_takes_the_worst_judged_pair() {
        let held = vec![("A_P".to_string(), "A_N".to_string(), 10.0), ("B_P".to_string(), "B_N".to_string(), 0.0)];
        let rows = vec![
            json!({ "p": "A_P", "n": "A_N", "p_length_mm": 11.0, "n_length_mm": 12.0 }),
            json!({ "p": "B_P", "n": "B_N", "p_length_mm": 99.0, "n_length_mm": 99.0 }),
            json!({ "p": "C_P", "n": "C_N", "status": "skipped" }),
        ];
        assert_eq!(pair_growth(&rows, &held), Some(1.2));
        assert_eq!(pair_growth(&rows[1..], &held), None);
    }

    #[test]
    fn absent_means_empty_with_the_cli_defaults() {
        let r = parse(json!({})).unwrap();
        assert!(r.is_empty());
        assert_eq!(r.pair_skew_mm, DEFAULT_PAIR_SKEW_MM);
    }

    #[test]
    fn full_request_parses() {
        let r = parse(json!({
            "pairs": [{ "p": "D_P", "n": "D_N", "gap_mm": 0.15, "layer_gaps_mm": { "In1.Cu": 0.12 }, "skew_mm": 0.05 }],
            "pair_skew_mm": 0.2,
            "tune": [{ "name": "bus", "nets": ["D*", "X"], "tolerance_mm": 0.3 }, { "name": "clk", "nets": ["CLK"], "target_mm": 40 }],
            "layer_heights_mm": [0.0, 0.2, 1.4]
        }))
        .unwrap();
        assert_eq!((r.pairs[0].gap_mm, r.pairs[0].skew_mm), (Some(0.15), Some(0.05)));
        assert_eq!(r.pairs[0].layer_gaps_mm, vec![("In1.Cu".to_string(), 0.12)]);
        assert_eq!(r.tune[0].tolerance_mm, 0.3);
        assert_eq!(r.tune[1].tolerance_mm, DEFAULT_TUNE_TOLERANCE_MM);
        assert_eq!(r.tune[1].target_mm, Some(40.0));
        assert_eq!(r.layer_heights_mm.len(), 3);
    }

    #[test]
    fn bad_requests_are_rejected() {
        for bad in [
            json!({ "pairs": "x" }),
            json!({ "pairs": [{ "p": "A" }] }),
            json!({ "pairs": [{ "p": "A", "n": "A" }] }),
            json!({ "pairs": [{ "p": "A", "n": "B", "gap_mm": 0 }] }),
            json!({ "pairs": [{ "p": "A", "n": "B", "gap_mm": -1 }] }),
            json!({ "pairs": [{ "p": "A", "n": "B", "x": 1 }] }),
            json!({ "pairs": [{ "p": "A", "n": "B" }, { "p": "B", "n": "C" }] }),
            json!({ "pair_skew_mm": -1 }),
            json!({ "tune": [{ "name": "g", "nets": [] }] }),
            json!({ "tune": [{ "name": "g", "nets": ["A"] }, { "name": "g", "nets": ["B"] }] }),
            json!({ "tune": [{ "name": "g", "nets": ["A"], "target_mm": 0 }] }),
            json!({ "layer_heights_mm": [1, "x"] }),
        ] {
            assert_eq!(parse(bad.clone()).unwrap_err().code, "bad_request", "{bad}");
        }
    }

    #[test]
    fn glob_matches_like_the_engine() {
        assert!(glob("D*", "D0") && glob("*_P", "USB_P") && glob("A*C", "ABC") && glob("X", "X"));
        assert!(!glob("D*", "XD") && !glob("X", "XY") && !glob("A*C", "AB"));
    }
}
