//! Board statistics and scores (porting unit U6, second half).
//!
//! | Module | Java |
//! |---|---|
//! | [`statistics`] | `core/scoring/BoardStatistics` + the `BoardStatistics*` data classes |
//! | [`bounds`] | `core/scoring/BoardStatisticsBoundsCalculator`, `BoardStatisticsBounds` |
//!
//! Not ported: `BenchmarkScoreCalculator`, `ScoringWeightComparison`, `BoardScoreBreakdown` and
//! the file based `BoardStatistics(byte[], FileFormat)` constructor (reporting only).
//!
//! Cost of [`BoardStatistics::new`] (for memoization in the pipeline), in decreasing order:
//! the clearance violations (`include_clearance_violations`, a full-board DRC, see
//! [`crate::drc`]); the incompletes (`include_connections`, connectivity + Delaunay per net);
//! the fanout statistics (contacts of every SMD pin, `unconnected_set` per SMD pin and one
//! clearance check per contacted trace/via); the lower bounds (MST per net, O(terminals²));
//! the rest is a linear pass over the items. The result is a pure function of the board state
//! (and of `pre_existing_clearance_violations_count`).

pub mod bounds;
pub mod statistics;

pub use bounds::{calculate_bounds, BoardStatisticsBounds};
pub use statistics::{
    is_pin_escaped, unescape_unicode, BoardStatistics, BoardStatisticsBends, BoardStatisticsBoard, BoardStatisticsClearanceViolations,
    BoardStatisticsComponents, BoardStatisticsConnections, BoardStatisticsDifficulty, BoardStatisticsFanout, BoardStatisticsItems,
    BoardStatisticsLayers, BoardStatisticsNets, BoardStatisticsPads, BoardStatisticsTraces, BoardStatisticsVias, Rect2DF,
};
