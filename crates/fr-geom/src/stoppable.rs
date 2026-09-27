//! Port of `app.freerouting.datastructures.Stoppable` (interface for stoppable threads).

/// Interface for stoppable threads.
pub trait Stoppable {
    /// Requests this thread to be stopped.
    fn request_stop(&self);
    /// Returns true, if this thread is requested to be stopped.
    fn is_stop_requested(&self) -> bool;
}
