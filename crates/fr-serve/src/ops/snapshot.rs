//! Ops `snapshot` / `restore` (capability `snapshot`): not implemented in this build.

use serde_json::{Map, Value};

use crate::proto::{ProtoError, R};
use crate::session::Session;

/// True once this module implements the ops and passes its conformance check.
pub const CLAIMED: bool = false;

pub fn handle(_session: &mut Session, _args: &Map<String, Value>, op: &str) -> R<Value> {
    Err(ProtoError::unsupported(&format!("op '{op}'"), "snapshot"))
}
