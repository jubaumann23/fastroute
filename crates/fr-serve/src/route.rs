//! Op `route` (SPEC 5.5): the full pipeline (fanout, autorouter, optimizer) on the whole board.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use fr_engine::board::BasicBoard;
use fr_engine::datastructures::StopToken;
use fr_engine::ids::FixedState;
use fr_engine::pipeline::{self, LiveEvent, OptimizerMode, PipelineContext};
use serde_json::{json, Map, Value};

use crate::facts;
use crate::proto::{as_int, as_str, check_keys, req, ProtoError, R};
use crate::session::Session;

/// `route.seed` selects a variant: not yet (any seed is accepted and ignored).
pub const SEED_CLAIMED: bool = false;
/// `route.nets` as a list: not yet.
pub const INCREMENTAL_CLAIMED: bool = false;

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
    match args.get("nets") {
        None => {}
        Some(Value::String(s)) if s == "all" => {}
        Some(Value::String(s)) => return Err(ProtoError::bad_request(format!("route.nets: '{s}' is neither \"all\" nor a list"))),
        Some(Value::Array(list)) => {
            for n in list {
                if as_str(n, "route.nets item")?.is_empty() {
                    return Err(ProtoError::bad_request("route.nets items must be non-empty net names"));
                }
            }
            if !INCREMENTAL_CLAIMED {
                return Err(ProtoError::unsupported("route.nets as a list", "incremental"));
            }
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
    if scratch {
        let b = &mut work.board.board;
        let keys: Vec<_> = b
            .get_items()
            .into_iter()
            .filter(|&k| (b.item(k).is_trace() || b.item(k).is_via()) && b.item(k).fixed_state() == FixedState::Unfixed)
            .collect();
        b.remove_items(keys);
    }

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
    let wall_ms = t0.elapsed().as_millis() as i64;

    let result = summarize(&work.board.board, &work.locks, seed, passes.load(Ordering::SeqCst), budget_hit, wall_ms);
    session.commit(work);
    Ok(result)
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
