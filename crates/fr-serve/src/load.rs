//! Op `load` (SPEC 5.2): DSN by path or text, optional initial session, board preparation.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use fr_dsn::model::DsnError;
use fr_dsn::Dsn;
use fr_engine::board::{BasicBoard, RoutingBoard, TimeLimitMode, TimeLimitPolicy};
use fr_engine::ids::ItemId;
use fr_settings::{available_processors, headless_merger, DsnFileSettings, EnvironmentSettings};
use serde_json::{json, Map, Value};

use crate::facts;
use crate::ops::lock::LockRegistry;
use crate::proto::{as_bool, as_obj, as_str, check_keys, req, ProtoError, R};
use crate::session::{Board, Origin, Session, Work};

/// Bytes named by `{"path": ..}` or `{"text": ..}` (exactly one).
pub fn source_bytes(v: &Value, what: &str) -> R<Vec<u8>> {
    let o = as_obj(v, what)?;
    check_keys(o, &["path", "text"], what)?;
    match (o.get("path"), o.get("text")) {
        (Some(p), None) => {
            let path = as_str(p, &format!("{what}.path"))?;
            if !std::path::Path::new(path).is_absolute() {
                return Err(ProtoError::bad_request(format!("{what}.path must be absolute")));
            }
            std::fs::read(path).map_err(|e| {
                ProtoError::new("io_error", format!("cannot read '{path}': {e}")).with_details(json!({ "path": path }))
            })
        }
        (None, Some(t)) => Ok(as_str(t, &format!("{what}.text"))?.as_bytes().to_vec()),
        _ => Err(ProtoError::bad_request(format!("{what}: give exactly one of 'path' and 'text'"))),
    }
}

/// The session name the stock CLI would write: the file stem of the DSN path (`main.rs` design_name),
/// or, for inline text, the file stem of the DSN's pcb name.
fn session_name(v: &Value, dsn: &Dsn) -> String {
    let from_path = v.get("path").and_then(Value::as_str);
    let source = from_path.unwrap_or(dsn.name.as_str());
    std::path::Path::new(source).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn bad_dsn(line: u32, reason: String) -> ProtoError {
    ProtoError::new("bad_dsn", format!("line {line}: {reason}")).with_details(json!({ "line": line, "reason": reason }))
}

fn parse_dsn(data: &[u8]) -> R<Dsn> {
    Dsn::parse(data).map_err(|e| match e {
        DsnError::Syntax(p) => bad_dsn(p.line, p.message),
        DsnError::Invalid { line, message } => bad_dsn(line, message),
        DsnError::NotDsn(m) => bad_dsn(1, m),
    })
}

/// `bad_ses` unless `data` is a well-formed Specctra session.
fn check_ses(data: &[u8]) -> R<()> {
    let bad = |line: u32, reason: String| {
        ProtoError::new("bad_ses", format!("line {line}: {reason}")).with_details(json!({ "line": line, "reason": reason }))
    };
    let top = fr_dsn::sexpr::parse(data).map_err(|e| bad(e.line, e.message))?;
    let is_session = top.first().and_then(fr_dsn::sexpr::Sexpr::as_list).is_some_and(|l| l.is("session"));
    if is_session {
        Ok(())
    } else {
        Err(bad(1, "not a Specctra session file".into()))
    }
}

fn unit_name(u: fr_dsn::model::Unit) -> &'static str {
    use fr_dsn::model::Unit;
    match u {
        Unit::Inch => "inch",
        Unit::Mil => "mil",
        Unit::Mm => "mm",
        Unit::Um => "um",
    }
}

/// Builds the routing board from DSN bytes. Mirrors the stock CLI (`crates/fastroute/src/main.rs`
/// `run()`): settings merge :395, board load :405-408, class-pair clearances :409-413, initial
/// session import and release :432-448, outline wiring :449-456, post-load processing :457-475
/// (enhancements on, not `--parity`), and the time limits of `--no-time-limits` :486-494 (count mode,
/// the default factor: deterministic, never the wall clock).
fn build(session: &Session, data: &[u8], dsn: &Dsn, ses: Option<&[u8]>, name: String, lock_initial: bool) -> R<(Board, LockRegistry)> {
    let procs = available_processors();
    let dsn_settings = DsnFileSettings::from_dsn(dsn);
    // Hermetic (SPEC 7): no FREEROUTING__ROUTER__* environment source, so results depend only on the
    // build, the hello settings and the inputs.
    let env = EnvironmentSettings::default();
    let mut settings = headless_merger(&session.settings().cli, &env, Some(&dsn_settings), None, procs).merge(procs);

    let mut board = fr_io::post_load::load_from_specctra_dsn(data, &mut settings)
        .map_err(|e| bad_dsn(0, format!("{e:?}")))?;
    fr_io::network::extend_class_pair_clearances(&mut board, dsn);
    let wiring_ids = |b: &RoutingBoard| -> std::collections::HashSet<ItemId> {
        b.get_items().into_iter().filter(|&k| b.item(k).is_trace() || b.item(k).is_via()).map(|k| b.item(k).id()).collect()
    };
    let before = wiring_ids(&board);
    let own = LockRegistry::user_fixed_ids(&board);
    if fr_io::post_load::prepare_for_routing(&mut board, &mut settings, ses).is_some() && !lock_initial {
        // the session reader fixes what it imports; here it is a starting point that may be ripped
        // (with `lock_initial` it stays fixed: that is the lock)
        let imported: Vec<_> = wiring_ids(&board).difference(&before).copied().collect();
        fr_engine::diffpair::release_pairs(&mut board, &imported);
    }
    board.keep_wiring_inside_outline();
    fr_engine::pipeline::deferred_post_load_processing(&mut board);
    board.fallback_vias_own_class = true;
    board.set_overlap_contacts(true);
    board.mark_stitching_vias();
    board.bridge_trace_ends_to_drill_centers();
    board.time_limits = TimeLimitPolicy {
        mode: TimeLimitMode::Count { factor: fr_engine::datastructures::time_limit::DEFAULT_COUNT_FACTOR },
        fired: Some(Arc::new(AtomicBool::new(false))),
    };
    let locks = LockRegistry::after_load(&mut board, &own, lock_initial);
    let origin = Arc::new(Origin { dsn: data.to_vec(), ses: ses.map(<[u8]>::to_vec), lock_initial });
    Ok((Board { origin, moves: Vec::new(), name, board, settings }, locks))
}

pub fn handle(session: &mut Session, args: &Map<String, Value>) -> R<Value> {
    check_keys(args, &["dsn", "ses", "lock_initial"], "load")?;
    let dsn_bytes = source_bytes(req(args, "dsn", "load")?, "load.dsn")?;
    let ses_bytes = args.get("ses").map(|v| source_bytes(v, "load.ses")).transpose()?;
    let lock_initial = args.get("lock_initial").map(|v| as_bool(v, "load.lock_initial")).transpose()?.unwrap_or(false);
    let dsn = parse_dsn(&dsn_bytes)?;
    if let Some(s) = &ses_bytes {
        check_ses(s)?;
    }
    let name = session_name(&args["dsn"], &dsn);
    let (loaded, locks) = build(session, &dsn_bytes, &dsn, ses_bytes.as_deref(), name, lock_initial)?;
    let result = describe(&loaded.board, &dsn);
    // `load` replaces the board and drops locks and snapshots.
    session.board = Some(loaded);
    session.locks = locks;
    session.snapshots.clear();
    Ok(result)
}

/// Replaces the working board by what a fresh `load` of the session's inputs gives, with the session's
/// placement and locked wiring put back: the state `route` from scratch starts from.
///
/// Ripping up wiring in place does not restore the board. Items, ids, settings, rules and components
/// of a ripped board equal a fresh load's, yet routing it differs (hb200: 619 wires against 595), so
/// the residue sits in the board's private search trees and watermarks that history leaves behind.
/// The rebuild replays the successful `move`s (carrying DSN-fixed fan-out as they did), re-inserts the
/// locked wiring and raises the id generator to the live maximum (ids are never reused, SPEC 4).
pub fn rebuild_for_scratch(session: &Session, work: &mut Work) -> R<()> {
    use fr_engine::datastructures::{IdGenerator, ItemIdGenerator};
    let origin = work.board.origin.clone();
    let dsn = parse_dsn(&origin.dsn)?;
    let name = work.board.name.clone();
    let (fresh, fresh_locks) = build(session, &origin.dsn, &dsn, origin.ses.as_deref(), name, origin.lock_initial)?;
    let mut replay = Session::new();
    replay.hello = session.hello.clone();
    replay.board = Some(fresh);
    replay.locks = fresh_locks;
    for args in &work.board.moves {
        let mut args = args.clone();
        args.insert("unlock".into(), Value::Bool(true));
        crate::ops::move_::handle(&mut replay, &args)
            .map_err(|e| ProtoError::new("internal", format!("scratch rebuild: replaying a move failed: {}", e.message)))?;
    }
    let (mut board, mut locks) = (replay.board.take().expect("replay holds a board"), replay.locks);

    let live_max = work.board.board.communication.id_generator.max_generated_id();
    crate::ops::lock::carry_locked_wiring(&work.board.board, &work.locks, &mut board.board, &mut locks);
    let gen = &mut board.board.communication.id_generator;
    if gen.max_generated_id() < live_max {
        *gen = ItemIdGenerator::with_last_generated_id(live_max);
    }
    board.moves = std::mem::take(&mut work.board.moves);
    work.board = board;
    work.locks = locks;
    Ok(())
}

/// The `load` result for a prepared board.
fn describe(board: &BasicBoard, dsn: &Dsn) -> Value {
    let (connections, unrouted) = facts::connections(board);
    let w = facts::wiring(board);
    json!({
        "name": dsn.name,
        "resolution": { "unit": unit_name(dsn.unit), "value": dsn.resolution },
        "layers": board.layer_structure.layers.iter().map(|l| l.name.clone()).collect::<Vec<_>>(),
        "boundary": facts::boundary(board),
        "components": dsn.placement.iter().map(|c| c.places.len()).sum::<usize>(),
        "nets": dsn.network.nets.len(),
        "connections": connections,
        "unrouted": unrouted.len(),
        "wires": w.wires,
        "vias": w.vias,
        "fixed": w.fixed,
    })
}
