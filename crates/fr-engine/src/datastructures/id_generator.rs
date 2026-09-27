//! Port of `datastructures/IdGenerator.java` and `board/actions/ItemIdGenerator.java`.

/// Java `IdGenerator`: creates unique identification numbers.
pub trait IdGenerator {
    /// Creates a new unique identification number.
    fn new_id(&mut self) -> i32;
    /// Returns the maximum generated id number so far.
    fn max_generated_id(&self) -> i32;
}

/// Java `ItemIdGenerator`: ids start at 1 and increase by 1. After [`Self::MAX_ID`]
/// (`Integer.MAX_VALUE / 2`) was handed out, the next id is 1 again (one warning per wrap).
///
/// The generator is part of the board state that Java does *not* restore on undo; cloning it is
/// cheap.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ItemIdGenerator {
    last_generated_id: i32,
    /// How often the counter wrapped around (diagnostics only).
    wrap_around_count: i64,
}

impl ItemIdGenerator {
    /// `Integer.MAX_VALUE / 2`.
    pub const MAX_ID: i32 = i32::MAX / 2;

    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a generator whose next id is `last_generated_id + 1` (e.g. to restore a state).
    pub fn with_last_generated_id(last_generated_id: i32) -> Self {
        ItemIdGenerator { last_generated_id, wrap_around_count: 0 }
    }

    /// Number of wrap-arounds so far.
    pub fn wrap_around_count(&self) -> i64 {
        self.wrap_around_count
    }
}

impl IdGenerator for ItemIdGenerator {
    fn new_id(&mut self) -> i32 {
        if self.last_generated_id >= Self::MAX_ID {
            // Wrap around to 1 instead of overflowing into negative territory.
            self.wrap_around_count += 1;
            log::warn!(
                "IdGenerator: ID counter reached {} and wrapped around to 1 (wrap #{}). IDs that were previously assigned to now-deleted items may be assigned again to newly created items. Consider restarting the router to regenerate IDs from scratch.",
                Self::MAX_ID,
                self.wrap_around_count
            );
            self.last_generated_id = 0;
        }
        self.last_generated_id += 1;
        self.last_generated_id
    }

    fn max_generated_id(&self) -> i32 {
        self.last_generated_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_one() {
        let mut g = ItemIdGenerator::new();
        assert_eq!(g.max_generated_id(), 0);
        assert_eq!(g.new_id(), 1);
        assert_eq!(g.new_id(), 2);
        assert_eq!(g.max_generated_id(), 2);
        let mut c = g.clone();
        assert_eq!(c.new_id(), 3);
        assert_eq!(g.new_id(), 3);
    }

    #[test]
    fn wraps_after_max_id() {
        assert_eq!(ItemIdGenerator::MAX_ID, 1_073_741_823);
        let mut g = ItemIdGenerator::with_last_generated_id(ItemIdGenerator::MAX_ID - 1);
        assert_eq!(g.new_id(), ItemIdGenerator::MAX_ID);
        assert_eq!(g.max_generated_id(), ItemIdGenerator::MAX_ID);
        assert_eq!(g.new_id(), 1);
        assert_eq!(g.wrap_around_count(), 1);
        assert_eq!(g.new_id(), 2);
    }
}
