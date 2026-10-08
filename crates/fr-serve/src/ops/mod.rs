//! Optional ops, one module each. A module flips its `CLAIMED` const when it implements its op and
//! passes the matching conformance check; `hello` lists exactly the claimed capabilities.

pub mod blockers;
pub mod congestion;
pub mod lock;
pub mod move_;
pub mod snapshot;

use serde_json::{Map, Value};

use crate::proto::{ProtoError, R};
use crate::route;
use crate::session::Session;

/// Sorted capability names (SPEC 6). `budget` is implemented by `route`.
pub fn capabilities() -> Vec<&'static str> {
    let mut caps: Vec<&'static str> = [
        ("budget", true),
        ("seed", route::SEED_CLAIMED),
        ("incremental", route::INCREMENTAL_CLAIMED),
        ("move", move_::CLAIMED),
        ("locking", lock::CLAIMED),
        ("blockers", blockers::CLAIMED),
        ("congestion", congestion::CLAIMED),
        ("snapshot", snapshot::CLAIMED),
    ]
    .into_iter()
    .filter_map(|(name, claimed)| claimed.then_some(name))
    .collect();
    caps.sort_unstable();
    caps
}

/// Runs an optional op. An op whose capability is not claimed is `unsupported` before its arguments
/// are looked at.
pub fn dispatch(session: &mut Session, op: &str, args: &Map<String, Value>) -> R<Value> {
    match op {
        "move" => move_::handle(session, args),
        "lock" | "unlock" => lock::handle(session, args, op),
        "blockers" => blockers::handle(session, args),
        "congestion" => congestion::handle(session, args),
        "snapshot" | "restore" => snapshot::handle(session, args, op),
        _ => Err(ProtoError::new("unknown_op", format!("no such op '{op}'"))),
    }
}
