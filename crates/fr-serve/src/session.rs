//! Session state. Every state-changing op works on a [`Work`] clone and commits it only on success,
//! so an error leaves the session exactly as it was (SPEC 1).

use fr_engine::board::RoutingBoard;
use fr_settings::RouterSettings;

use crate::ops::lock::LockRegistry;
use crate::proto::{ProtoError, R};
use crate::settings::SessionSettings;

/// A loaded board and what it was built with.
#[derive(Clone)]
pub struct Board {
    /// The DSN's pcb name (also the SES name).
    pub name: String,
    pub board: RoutingBoard,
    /// Merged router settings of this board (CLI source, environment, the DSN's own).
    pub settings: RouterSettings,
}

/// State changed by an op, cloned from the session and committed on success.
#[derive(Clone)]
pub struct Work {
    pub board: Board,
    pub locks: LockRegistry,
}

pub struct Session {
    /// Set by a successful `hello`.
    pub hello: Option<SessionSettings>,
    /// A `hello` with another protocol MAJOR was answered: only `shutdown` is accepted from now on.
    pub refused: bool,
    pub board: Option<Board>,
    pub locks: LockRegistry,
}

impl Session {
    pub fn new() -> Self {
        Session { hello: None, refused: false, board: None, locks: LockRegistry::default() }
    }

    /// A working copy of the loaded state, or `not_loaded`.
    pub fn begin(&self) -> R<Work> {
        match &self.board {
            Some(b) => Ok(Work { board: b.clone(), locks: self.locks.clone() }),
            None => Err(ProtoError::new("not_loaded", "no board is loaded: send 'load' first")),
        }
    }

    pub fn commit(&mut self, work: Work) {
        self.board = Some(work.board);
        self.locks = work.locks;
    }

    pub fn loaded(&self) -> R<&Board> {
        self.board.as_ref().ok_or_else(|| ProtoError::new("not_loaded", "no board is loaded: send 'load' first"))
    }

    pub fn settings(&self) -> &SessionSettings {
        self.hello.as_ref().expect("handlers run after hello")
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}
