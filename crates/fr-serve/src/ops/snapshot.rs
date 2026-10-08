//! Ops `snapshot` / `restore` (capability `snapshot`, SPEC 5.8).
//!
//! A snapshot is a clone of the whole session working state ([`Work`]: the routing board with its
//! settings, and the lock registry). `RoutingBoard` clones are cheap (docs/PORTING.md) and its
//! transient engine caches are rebuilt on use, so a restored board routes like the original.
//!
//! The item id generator lives inside the board (`communication.id_generator`) and would be rolled
//! back by the clone. SPEC 4 says ids are never reused, so `restore` raises the restored generator to
//! the highest id handed out so far. Routing order does not depend on absolute id values (verified by
//! `crates/fastroute/tests/serve_snapshot.rs`: route after restore equals route after snapshot).

use serde_json::{json, Map, Value};

use crate::facts;
use crate::proto::{as_bool, as_str, check_keys, req, ProtoError, R};
use crate::session::Session;

/// True once this module implements the ops and passes its conformance check.
pub const CLAIMED: bool = true;

pub fn handle(session: &mut Session, args: &Map<String, Value>, op: &str) -> R<Value> {
    match op {
        "snapshot" => snapshot(session, args),
        _ => restore(session, args),
    }
}

fn snapshot(session: &mut Session, args: &Map<String, Value>) -> R<Value> {
    check_keys(args, &[], "snapshot")?;
    let work = session.begin()?;
    session.snapshot_counter += 1;
    let id = format!("s{}", session.snapshot_counter);
    session.snapshots.insert(id.clone(), work);
    Ok(json!({ "snapshot": id }))
}

fn restore(session: &mut Session, args: &Map<String, Value>) -> R<Value> {
    use fr_engine::datastructures::{IdGenerator, ItemIdGenerator};
    check_keys(args, &["snapshot", "drop"], "restore")?;
    let id = as_str(req(args, "snapshot", "restore")?, "restore.snapshot")?;
    let drop_it = args.get("drop").map(|v| as_bool(v, "restore.drop")).transpose()?.unwrap_or(false);
    let mut work = match session.snapshots.get(id) {
        Some(w) => w.clone(),
        None => {
            return Err(ProtoError::new("unknown_snapshot", format!("no snapshot '{id}'")).with_details(json!({ "snapshot": id })))
        }
    };
    // ids are never reused (SPEC 4): continue after the highest id of the live board
    let current = session.loaded()?.board.communication.id_generator.max_generated_id();
    let gen = &mut work.board.board.communication.id_generator;
    if gen.max_generated_id() < current {
        *gen = ItemIdGenerator::with_last_generated_id(current);
    }
    let (_, unrouted) = facts::connections(&work.board.board);
    let w = facts::wiring(&work.board.board);
    let result = json!({ "unrouted": unrouted.len(), "wires": w.wires, "vias": w.vias });
    session.commit(work);
    if drop_it {
        session.snapshots.remove(id);
    }
    Ok(result)
}
