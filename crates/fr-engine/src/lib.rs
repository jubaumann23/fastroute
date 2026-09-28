//! Freerouting routing engine (headless): board model, search trees,
//! autorouter, DRC and scoring. See `docs/PORTING.md` for the porting plan.
//!
//! Conventions shared by all modules:
//! * Java ids and numbers keep their Java values and types (`i32`), wrapped in
//!   the newtypes of [`ids`] where they identify board objects. Net numbers
//!   are 1-based like in Java (0 = no net).
//! * Nothing holds a back-pointer to the board; methods that need board data
//!   take `&Board`/`&mut Board` (or the specific parts they need).
//! * Each module names the Java source it was ported from.

pub mod autoroute;
pub mod board;
pub mod datastructures;
pub mod drc;
pub mod ids;
pub mod library;
pub mod rules;
pub mod scoring;
pub mod structure;
