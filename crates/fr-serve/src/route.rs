//! Op `route` (SPEC 5.5): the full pipeline (fanout, autorouter, optimizer) on the whole board.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use fr_engine::board::{BasicBoard, RoutingBoard};
use fr_engine::datastructures::StopToken;
use fr_engine::ids::FixedState;
use fr_engine::pipeline::{self, LiveEvent, OptimizerMode, PipelineContext};
use serde_json::{json, Map, Value};

use crate::facts;
use crate::proto::{as_int, as_str, check_keys, req, ProtoError, R};
use crate::session::Session;

/// `route.seed` selects a variant: seed 0 is the stock order, seed > 0 sets hook H5 (`order_seed`).
pub const SEED_CLAIMED: bool = true;
/// `route.nets` as a list: hook H6 (`route_nets` mask) plus the request-scoped fixing of the other nets.
pub const INCREMENTAL_CLAIMED: bool = true;

/// fastroute multi-start of the stock CLI (`--multi-start`, main.rs:70); protocol 1.0 has no setting.
const MULTI_START: usize = 4;

/// The budget timer: stops the pipeline when `budget_ms` of wall time pass before it is done.
struct Budget {
    state: Arc<(Mutex<(bool, bool)>, Condvar)>, // (done, fired)
    timer: Option<std::thread::JoinHandle<()>>,
}

impl Budget {
    fn start(budget_ms: u64, stop: &StopToken) -> Budget {
        let state = Arc::new((Mutex::new((false, false)), Condvar::new()));
        if budget_ms == 0 {
            return Budget { state, timer: None };
        }
        let (st, stop) = (state.clone(), stop.clone());
        let timer = std::thread::spawn(move || {
            let (lock, cv) = &*st;
            let deadline = Instant::now() + Duration::from_millis(budget_ms);
            let mut g = lock.lock().unwrap();
            while !g.0 {
                let now = Instant::now();
                if now >= deadline {
                    g.1 = true;
                    stop.request_stop();
                    return;
                }
                g = cv.wait_timeout(g, deadline - now).unwrap().0;
            }
        });
        Budget { state, timer: Some(timer) }
    }

    /// Ends the timer; true when the budget fired before the pipeline finished.
    fn finish(mut self) -> bool {
        let (lock, cv) = &*self.state;
        let fired = {
            let mut g = lock.lock().unwrap();
            g.0 = true;
            g.1
        };
        cv.notify_all();
        if let Some(t) = self.timer.take() {
            let _ = t.join();
        }
        fired
    }
}

pub fn handle(session: &mut Session, args: &Map<String, Value>) -> R<Value> {
    check_keys(args, &["seed", "nets", "from", "budget_ms"], "route")?;
    // `seed` is required; with the `seed` capability absent any value is accepted and ignored.
    let seed = as_int(req(args, "seed", "route")?, 0, 2_147_483_647, "route.seed")?;
    let mut listed: Option<Vec<String>> = None;
    match args.get("nets") {
        None => {}
        Some(Value::String(s)) if s == "all" => {}
        Some(Value::String(s)) => return Err(ProtoError::bad_request(format!("route.nets: '{s}' is neither \"all\" nor a list"))),
        Some(Value::Array(list)) => {
            let mut names = Vec::with_capacity(list.len());
            for n in list {
                let name = as_str(n, "route.nets item")?;
                if name.is_empty() {
                    return Err(ProtoError::bad_request("route.nets items must be non-empty net names"));
                }
                names.push(name.to_string());
            }
            if !INCREMENTAL_CLAIMED {
                return Err(ProtoError::unsupported("route.nets as a list", "incremental"));
            }
            listed = Some(names);
        }
        Some(_) => return Err(ProtoError::bad_request("route.nets must be \"all\" or a list of net names")),
    }
    let scratch = match args.get("from").map(|v| as_str(v, "route.from")).transpose()? {
        None | Some("current") => false,
        Some("scratch") => true,
        Some(other) => return Err(ProtoError::bad_request(format!("route.from: '{other}' is neither current nor scratch"))),
    };
    let budget_ms = args.get("budget_ms").map(|v| as_int(v, 0, i64::MAX, "route.budget_ms")).transpose()?.unwrap_or(0);

    let mut work = session.begin()?;
    // Net numbers that may change in this request: the listed nets (all known, none locked-out),
    // or every net but the locked ones.
    let numbers = net_numbers(&work.board.board);
    let routable: Option<Vec<i32>> = match &listed {
        Some(names) => {
            let mut v = Vec::with_capacity(names.len());
            for n in names {
                match numbers.iter().find(|(name, _)| name == n) {
                    Some(&(_, no)) => v.push(no),
                    None => return Err(unknown_net(n)),
                }
            }
            Some(v)
        }
        None => None,
    };
    let locked: Vec<i32> =
        numbers.iter().filter(|(name, _)| work.locks.is_locked_net(name)).map(|&(_, no)| no).collect();
    let mask_wanted = routable.is_some() || !locked.is_empty();
    let allowed = |no: i32| routable.as_ref().map_or(true, |r| r.contains(&no)) && !locked.contains(&no);
    // H6 mask over net numbers (index 0 is "no net").
    let mask: Option<Vec<bool>> = mask_wanted.then(|| {
        let max = work.board.board.rules.nets.max_net_number().max(0) as usize;
        (0..=max).map(|n| n > 0 && allowed(n as i32)).collect()
    });
    // Wiring of nets outside the request is fixed for its duration; remember the exact prior state.
    let mut saved: Vec<(fr_engine::ids::ItemId, FixedState)> = Vec::new();
    if routable.is_some() {
        let b = &mut work.board.board;
        for k in b.get_items() {
            let it = b.item(k);
            if !(it.is_trace() || it.is_via()) || it.fixed_state() != FixedState::Unfixed {
                continue;
            }
            if it.net_numbers().iter().all(|&n| !allowed(n)) {
                saved.push((it.id(), it.fixed_state()));
                b.item_mut(k).set_fixed_state(FixedState::UserFixed);
            }
        }
    }
    if scratch {
        let b = &mut work.board.board;
        let keys: Vec<_> = b
            .get_items()
            .into_iter()
            .filter(|&k| {
                let it = b.item(k);
                (it.is_trace() || it.is_via())
                    && it.fixed_state() == FixedState::Unfixed
                    && (routable.is_none() || it.net_numbers().iter().any(|&n| allowed(n)))
            })
            .collect();
        b.remove_items(keys);
    }
    work.board.board.order_seed = (seed > 0).then_some(seed);
    work.board.board.route_nets = mask;

    let threads = session.settings().threads;
    let stop = StopToken::new();
    let passes = Arc::new(AtomicI32::new(0));
    let counter = passes.clone();
    // The stock CLI with `--no-time-limits`: no stage wall clock limits, the optimizer parallel above one thread.
    let ctx = PipelineContext {
        stop: stop.clone(),
        wall_clock_limits: false,
        optimizer_mode: if threads <= 1 { OptimizerMode::JavaCompat } else { OptimizerMode::Parallel { threads } },
        enhancements: true,
        multi_start: MULTI_START,
        checkpoint: None,
        observer: Some(Arc::new(move |_board, ev| {
            if let LiveEvent::RouterPass { pass_no, .. } = ev {
                counter.fetch_max(*pass_no, Ordering::SeqCst);
            }
        })),
    };
    let t0 = Instant::now();
    let budget = Budget::start(budget_ms as u64, &stop);
    pipeline::run_pipeline(&mut work.board.board, &mut work.board.settings, &ctx);
    let budget_hit = budget.finish();
    // Per-request knobs never outlive the request; the fixed states of the other nets return.
    work.board.board.order_seed = None;
    work.board.board.route_nets = None;
    for (id, state) in saved {
        if let Some(k) = work.board.board.get_item(id) {
            work.board.board.item_mut(k).set_fixed_state(state);
        }
    }
    let wall_ms = t0.elapsed().as_millis() as i64;

    let result = summarize(&work.board.board, &work.locks, seed, passes.load(Ordering::SeqCst), budget_hit, wall_ms);
    session.commit(work);
    Ok(result)
}

/// `(name, net number)` of every net, in net-number (DSN network) order.
fn net_numbers(board: &RoutingBoard) -> Vec<(String, i32)> {
    (1..=board.rules.nets.max_net_number())
        .filter_map(|n| board.rules.nets.get(n).map(|x| (x.name.clone(), n)))
        .collect()
}

fn unknown_net(name: &str) -> ProtoError {
    ProtoError::new("unknown_net", format!("route.nets: the board has no net '{name}'")).with_details(json!({ "name": name }))
}

fn summarize(
    board: &BasicBoard,
    locks: &crate::ops::lock::LockRegistry,
    seed: i64,
    passes: i32,
    budget_hit: bool,
    wall_ms: i64,
) -> Value {
    let (connections, unrouted) = facts::connections(board);
    let w = facts::wiring(board);
    json!({
        "seed": seed,
        "seed_used": SEED_CLAIMED,
        "complete": unrouted.is_empty(),
        "connections": connections,
        "unrouted": unrouted.len(),
        "vias": w.vias,
        "wires": w.wires,
        "length": w.length,
        "violations": facts::violations(board),
        "passes": passes.max(0),
        "budget_hit": budget_hit,
        "wall_ms": wall_ms,
        "nets": facts::net_rows(board, &unrouted, locks),
        "unrouted_connections": unrouted.iter().map(facts::Unrouted::to_json).collect::<Vec<_>>(),
    })
}
