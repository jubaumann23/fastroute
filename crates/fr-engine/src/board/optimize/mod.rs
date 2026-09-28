//! Port of `board/optimize` (porting unit U7): pulling traces tight, shoving traces and optimizing
//! via locations.
//!
//! | Module | Java |
//! |---|---|
//! | [`trace_tightener`] | `TraceTightener` (+ `PolylineTrace.pullTight`, `smoothenEndCornersFork`) |
//! | [`tightener45`] | `TraceTightener45` |
//! | [`tightener90`] | `TraceTightener90` |
//! | [`tightener_any_angle`] | `TraceTightenerAnyAngle` |
//! | [`trace_shover`] | `TraceShover` |
//! | [`via_optimizer`] | `ViaOptimizer` |
//! | [`tracked`] | Java object identity of `Line`s / `Polyline`s (no Java class) |

pub mod tightener45;
pub mod tightener90;
pub mod tightener_any_angle;
pub mod trace_shover;
pub mod trace_tightener;
pub mod tracked;
pub mod via_optimizer;

pub use trace_shover::TraceShover;
pub use trace_tightener::TraceTightener;
