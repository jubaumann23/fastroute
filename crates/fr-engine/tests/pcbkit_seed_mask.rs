//! pcbkit hooks H5 (board.order_seed) and H6 (board.route_nets): default off, deterministic
//! per seed, and the mask routes only the selected nets.

use fr_engine::board::{RoutingBoard, TimeLimitPolicy};
use fr_engine::ids::FixedState;
use fr_engine::pipeline::{run_pipeline, PipelineContext};

const N: usize = 6;

/// Two 6-pin parts facing each other, nets joined through a permutation so the connections
/// cross and the routing order matters (KiCad-style DSN, um units).
fn dsn() -> String {
    let pins: String = (0..N).map(|k| format!("(pin PAD {} 0 {})\n", k + 1, k * 1500)).collect();
    let mut nets = String::new();
    let mut names = String::new();
    for k in 0..N {
        nets.push_str(&format!("(net N{k} (pins U1-{} U2-{}))\n", k + 1, (k * 5 + 2) % N + 1));
        names.push_str(&format!("N{k} "));
    }
    format!(
        "(pcb t.dsn\n (parser (string_quote \")(space_in_quoted_tokens on))\n (resolution um 10)\n (unit um)\n \
         (structure\n  (layer F.Cu (type signal) (property (index 0)))\n  (layer B.Cu (type signal) (property (index 1)))\n  \
         (boundary (path pcb 0 0 0 40000 0 40000 -20000 0 -20000 0 0))\n  (via VIA)\n  (rule (width 152.4) (clearance 152.4)))\n \
         (placement (component IC (place U1 8000 -6000 front 0) (place U2 32000 -6000 front 0)))\n \
         (library (image IC {pins})\n  \
         (padstack PAD (shape (polygon F.Cu 0 -400 400 400 400 400 -400 -400 -400)) (attach off))\n  \
         (padstack VIA (shape (circle F.Cu 600)) (shape (circle B.Cu 600)) (attach off)))\n \
         (network {nets}(class kc {names} (circuit (use_via VIA)) (rule (width 152.4) (clearance 152.4))))\n (wiring))\n"
    )
}

fn load() -> (RoutingBoard, fr_settings::RouterSettings) {
    let mut settings = fr_settings::default_settings(1);
    settings.optimizer.enabled = Some(false);
    let mut board = fr_io::post_load::load_from_specctra_dsn(dsn().as_bytes(), &mut settings).expect("load");
    fr_io::post_load::prepare_for_routing(&mut board, &mut settings, None);
    board.time_limits = TimeLimitPolicy::disabled();
    (board, settings)
}

fn route(board: &mut RoutingBoard, settings: &mut fr_settings::RouterSettings) -> Vec<u8> {
    let ctx = PipelineContext { wall_clock_limits: false, ..Default::default() };
    run_pipeline(board, settings, &ctx);
    fr_io::ses_writer::ses_bytes(board, "t")
}

fn net_no(board: &RoutingBoard, name: &str) -> usize {
    (1..=board.rules.nets.max_net_number()).find(|&n| board.rules.nets.get(n).is_some_and(|x| x.name == name)).expect("net") as usize
}

fn trace_nets(board: &RoutingBoard) -> Vec<i32> {
    board.get_traces().iter().map(|&k| board.item(k).net_number(0)).collect()
}

#[test]
fn knobs_none_equals_untouched_run() {
    let (mut a, mut sa) = load();
    let (mut b, mut sb) = load();
    assert!(b.order_seed.is_none() && b.route_nets.is_none());
    assert_eq!(route(&mut a, &mut sa), route(&mut b, &mut sb));
}

#[test]
fn same_seed_is_deterministic_and_seeds_are_reported() {
    let mut outs = Vec::new();
    for seed in [1i64, 1, 2, 3, 4, 5] {
        let (mut b, mut s) = load();
        b.order_seed = Some(seed);
        outs.push((seed, route(&mut b, &mut s)));
    }
    assert_eq!(outs[0].1, outs[1].1, "seed 1 twice must be identical");
    let distinct: std::collections::HashSet<_> = outs.iter().map(|(_, o)| o.clone()).collect();
    eprintln!("seed variance: {} distinct SES outputs over seeds 1,2,3,4,5", distinct.len());
    assert!(distinct.len() > 1, "different seeds should change the routing order on this board");
}

#[test]
fn mask_routes_only_selected_net() {
    let (mut b, mut s) = load();
    let a = net_no(&b, "N0");
    let mut mask = vec![false; b.rules.nets.max_net_number() as usize + 1];
    mask[a] = true;
    b.route_nets = Some(mask);
    route(&mut b, &mut s);
    let nets = trace_nets(&b);
    assert!(!nets.is_empty(), "net N0 must be routed");
    assert!(nets.iter().all(|&n| n as usize == a), "only N0 gets wiring, got {nets:?}");
}

#[test]
fn mask_keeps_user_fixed_wiring_of_other_nets() {
    // full route, then fix everything except N0, clear N0, re-route N0 under a mask
    let (mut b, mut s) = load();
    route(&mut b, &mut s);
    let a = net_no(&b, "N0");
    let fixed_geom = |b: &RoutingBoard| {
        let mut v: Vec<(i32, String)> = b
            .get_traces()
            .into_iter()
            .filter(|&k| b.item(k).net_number(0) as usize != a)
            .map(|k| (b.item(k).net_number(0), format!("{:?}->{:?}", b.item(k).first_corner(), b.item(k).last_corner())))
            .collect();
        v.sort();
        v
    };
    for k in b.get_traces() {
        if b.item(k).net_number(0) as usize != a {
            b.item_mut(k).set_fixed_state(FixedState::UserFixed);
        }
    }
    let n0_items: Vec<_> = b.get_traces().into_iter().filter(|&k| b.item(k).net_number(0) as usize == a).collect();
    assert!(!n0_items.is_empty());
    b.remove_items(n0_items);
    let before = fixed_geom(&b);
    assert!(!before.is_empty());
    let mut mask = vec![false; b.rules.nets.max_net_number() as usize + 1];
    mask[a] = true;
    b.route_nets = Some(mask);
    route(&mut b, &mut s);
    assert_eq!(before, fixed_geom(&b));
    assert!(trace_nets(&b).iter().any(|&n| n as usize == a), "masked net N0 is re-routed");
}
