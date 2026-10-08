//! Ops `lock` / `unlock` (capability `locking`): not implemented in this build. The registry is
//! part of the session already so that `route` reports locked nets.

use serde_json::{Map, Value};

use crate::proto::{ProtoError, R};
use crate::session::Session;

/// True once this module implements the ops and passes its conformance check.
pub const CLAIMED: bool = false;

/// Nets and wires frozen by `lock`.
#[derive(Clone, Debug, Default)]
pub struct LockRegistry {
    nets: Vec<String>,
    wires: usize,
}

impl LockRegistry {
    /// Locked nets in DSN network order.
    pub fn locked_nets(&self) -> &[String] {
        &self.nets
    }

    pub fn is_locked_net(&self, name: &str) -> bool {
        self.nets.iter().any(|n| n == name)
    }

    /// Locked wiring items.
    pub fn locked_wire_count(&self) -> usize {
        self.wires
    }
}

pub fn handle(_session: &mut Session, _args: &Map<String, Value>, op: &str) -> R<Value> {
    Err(ProtoError::unsupported(&format!("op '{op}'"), "locking"))
}
