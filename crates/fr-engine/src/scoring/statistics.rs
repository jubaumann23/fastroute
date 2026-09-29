//! Port of `core/scoring/BoardStatistics.java` (board constructor, router/optimizer scores) and
//! the `BoardStatistics*` data classes.
//!
//! Java stores most values as boxed `Integer`/`Float`/`Double` (nullable); they are `Option`s
//! here, `None` for Java `null`. Where Java would unbox a `null` (NullPointerException) the port
//! panics. All `float` arithmetic is done in `f32` with the Java casts and evaluation order;
//! `DoubleStream.sum` uses the compensated sum.

use fr_jcompat::compensated_sum;
use fr_settings::{OptimizerScoreSettings, OptimizerScoringVersion, RouterScoreSettings, RouterScoringVersion, RouterSettings, RoutingCostSettings};

use crate::board::{BasicBoard, ItemKey};
use crate::drc::{all_clearance_violations, clearance_violations, DesignRulesChecker};
use crate::ids::FixedState;
use crate::rules::BoardRules;
use crate::structure::Unit;

use super::bounds::{calculate_bounds, BoardStatisticsBounds};

/// `Constants.FREEROUTING_VERSION` used for the host string when the board has none (never the
/// case for the board constructor, see [`BoardStatistics::new`]).
pub const FREEROUTING_VERSION: &str = "2.4.1";

/// Java `Math.toDegrees` factor (`Math.RADIANS_TO_DEGREES`).
const RADIANS_TO_DEGREES: f64 = 57.29577951308232;

/// Java `java.awt.geom.Rectangle2D.Float`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect2DF {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Java `BoardStatisticsBoard`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoardStatisticsBoard {
    pub bounding_box: Option<Rect2DF>,
    pub size: Option<Rect2DF>,
    pub area_cm2: Option<f32>,
}

/// Java `BoardStatisticsLayers`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoardStatisticsLayers {
    pub total_count: Option<i32>,
    pub signal_count: Option<i32>,
}

/// Java `BoardStatisticsItems`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoardStatisticsItems {
    pub total_count: Option<i32>,
    pub trace_count: Option<i32>,
    pub via_count: Option<i32>,
    pub conduction_area_count: Option<i32>,
    pub drill_item_count: Option<i32>,
    pub pin_count: Option<i32>,
    pub component_outline_count: Option<i32>,
    pub other_count: Option<i32>,
}

/// Java `BoardStatisticsComponents`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoardStatisticsComponents {
    pub total_count: Option<i32>,
}

/// Java `BoardStatisticsPads`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoardStatisticsPads {
    pub total_count: Option<i32>,
}

/// Java `BoardStatisticsNets`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoardStatisticsNets {
    pub total_count: Option<i32>,
    pub class_count: Option<i32>,
}

/// Java `BoardStatisticsConnections`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoardStatisticsConnections {
    pub maximum_count: Option<i32>,
    pub incomplete_count: Option<i32>,
}

/// Java `BoardStatisticsTraces`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoardStatisticsTraces {
    pub total_count: Option<i32>,
    pub total_segment_count: Option<i32>,
    pub total_length: Option<f32>,
    pub total_length_mm: Option<f32>,
    pub total_weighted_length: Option<f32>,
    pub average_length: Option<f32>,
    pub total_vertical_length: Option<f32>,
    pub total_horizontal_length: Option<f32>,
    pub total_angled_length: Option<f32>,
}

/// Java `BoardStatisticsBends`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoardStatisticsBends {
    pub total_count: Option<i32>,
    pub ninety_degree_count: Option<i32>,
    pub forty_five_degree_count: Option<i32>,
    pub other_angle_count: Option<i32>,
}

/// Java `BoardStatisticsVias`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoardStatisticsVias {
    pub total_count: Option<i32>,
    pub through_hole_count: Option<i32>,
    pub blind_count: Option<i32>,
    pub buried_count: Option<i32>,
}

/// Java `BoardStatisticsClearanceViolations`.
#[derive(Clone, Debug, PartialEq)]
pub struct BoardStatisticsClearanceViolations {
    pub total_count: Option<i32>,
    pub pre_existing_count: Option<i32>,
    pub unfixable_count: Option<i32>,
    pub router_introduced_count: Option<i32>,
    pub total_violation_um: Option<f64>,
    pub min_violation_um: Option<f64>,
    pub max_violation_um: Option<f64>,
    pub avg_violation_um: Option<f64>,
}

impl Default for BoardStatisticsClearanceViolations {
    fn default() -> Self {
        BoardStatisticsClearanceViolations {
            total_count: None,
            pre_existing_count: Some(0),
            unfixable_count: Some(0),
            router_introduced_count: Some(0),
            total_violation_um: None,
            min_violation_um: None,
            max_violation_um: None,
            avg_violation_um: None,
        }
    }
}

/// Java `BoardStatisticsDifficulty`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoardStatisticsDifficulty {
    pub pin_count: Option<i32>,
    pub signal_layer_count: Option<i32>,
    pub complexity_c: Option<i32>,
    pub difficulty_d: Option<f32>,
    pub board_area_cm2: Option<f32>,
}

/// Java `BoardStatistics.BoardStatisticsFanout`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BoardStatisticsFanout {
    pub total_smd_pins: i32,
    pub pins_to_escape: i32,
    pub escaped_count: i32,
}

/// Java `BoardStatistics`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoardStatistics {
    pub host: Option<String>,
    pub unit: Option<String>,
    pub board: BoardStatisticsBoard,
    pub layers: BoardStatisticsLayers,
    pub items: BoardStatisticsItems,
    pub components: BoardStatisticsComponents,
    pub pads: BoardStatisticsPads,
    pub nets: BoardStatisticsNets,
    pub connections: BoardStatisticsConnections,
    pub traces: BoardStatisticsTraces,
    pub bends: BoardStatisticsBends,
    pub vias: BoardStatisticsVias,
    pub clearance_violations: BoardStatisticsClearanceViolations,
    /// Java `difficulty` (nullable in Java; always present here).
    pub difficulty: BoardStatisticsDifficulty,
    pub bounds: BoardStatisticsBounds,
    pub fanout: BoardStatisticsFanout,
}

/// Java `Unit.toString()` (lower case enum name).
fn unit_name(unit: Unit) -> String {
    match unit {
        Unit::Mil => "mil",
        Unit::Inch => "inch",
        Unit::Mm => "mm",
        Unit::Um => "um",
    }
    .to_string()
}

impl BoardStatistics {
    /// Java `new BoardStatistics(board)`.
    pub fn from_board(board: &BasicBoard) -> BoardStatistics {
        Self::new(board, None, true, true)
    }

    /// Java `new BoardStatistics(board, unit, includeClearanceViolations, includeConnections)`
    /// (the other board constructors pass `true` for the missing flags).
    pub fn new(board: &BasicBoard, unit: Option<Unit>, include_clearance_violations: bool, include_connections: bool) -> BoardStatistics {
        Self::new_with_bounds(board, unit, include_clearance_violations, include_connections, None)
    }

    /// Like [`Self::new`], with the lower bounds precomputed by
    /// [`calculate_bounds`](super::calculate_bounds) (Java caches them per board object).
    pub fn new_with_bounds(
        board: &BasicBoard,
        unit: Option<Unit>,
        include_clearance_violations: bool,
        include_connections: bool,
        bounds: Option<BoardStatisticsBounds>,
    ) -> BoardStatistics {
        let mut s = BoardStatistics::default();
        let bb = board.bounding_box;

        let info = board.communication.specctra_parser_info.as_ref().expect("NullPointerException: specctraParserInfo");
        let host = format!(
            "{},{}",
            info.host_cad.as_deref().unwrap_or("null"),
            info.host_version.as_deref().unwrap_or("null")
        );
        // (host is never null or empty after the concatenation)
        s.host = Some(unescape_unicode(&host));
        s.unit = Some(unit_name(board.communication.unit));

        // Board
        s.board.bounding_box = Some(Rect2DF { x: bb.ur.x as f32, y: bb.ur.y as f32, width: bb.ll.x as f32, height: bb.ll.y as f32 });
        let size = Rect2DF {
            x: 0.0,
            y: 0.0,
            width: (bb.ll.x as f32 - bb.ur.x as f32).abs(),
            height: (bb.ll.y as f32 - bb.ur.y as f32).abs(),
        };
        s.board.size = Some(size);

        // Layers
        let layer_count = board.layer_count();
        let signal_count = board.layer_structure.signal_layer_count();
        s.layers.total_count = Some(layer_count);
        s.layers.signal_count = Some(signal_count);

        // Items
        let (mut total, mut traces, mut vias, mut conduction, mut pins, mut outlines, mut other) = (0i32, 0i32, 0i32, 0i32, 0i32, 0i32, 0i32);
        let items = board.get_items();
        for &k in &items {
            let it = board.item(k);
            total += 1;
            if it.is_trace() {
                traces += 1;
            } else if it.is_via() {
                vias += 1;
            } else if it.is_conduction_area() {
                conduction += 1;
            } else if it.is_pin() {
                pins += 1;
            } else if it.is_component_outline() {
                // (DrillItems other than pins and vias do not exist)
                outlines += 1;
            } else {
                other += 1;
            }
        }
        s.items = BoardStatisticsItems {
            total_count: Some(total),
            trace_count: Some(traces),
            via_count: Some(vias),
            conduction_area_count: Some(conduction),
            drill_item_count: Some(0),
            pin_count: Some(pins),
            component_outline_count: Some(outlines),
            other_count: Some(other),
        };

        s.components.total_count = Some(board.components.count());
        s.pads.total_count = Some(board.get_pins().len() as i32);
        s.nets.total_count = Some(board.rules.nets.max_net_number());
        s.nets.class_count = Some(board.rules.net_classes.count());

        // Traces
        let trace_keys = board.get_traces();
        s.traces.total_count = Some(trace_keys.len() as i32);
        let total_length = compensated_sum(trace_keys.iter().map(|k| board.item(*k).trace().length())) as f32;
        s.traces.total_length = Some(total_length);
        let comm = &board.communication;
        let resolution = if comm.resolution > 0 { comm.resolution } else { 1 } as f64;
        let board_unit_to_mm = Unit::scale(1.0, comm.unit, Unit::Mm) / resolution;
        let board_unit_to_um = Unit::scale(1.0, comm.unit, Unit::Um) / resolution;
        let area_cm2 = (size.width as f64 * board_unit_to_mm * size.height as f64 * board_unit_to_mm / 100.0) as f32;
        s.board.area_cm2 = Some(area_cm2);
        let complexity_c = 1.max(pins.wrapping_mul(signal_count));
        s.difficulty = BoardStatisticsDifficulty {
            pin_count: Some(pins),
            signal_layer_count: Some(signal_count),
            complexity_c: Some(complexity_c),
            difficulty_d: Some(complexity_c as f32),
            board_area_cm2: Some(area_cm2),
        };
        s.bounds = bounds.unwrap_or_else(|| calculate_bounds(board));
        s.traces.total_length_mm = Some((total_length as f64 * board_unit_to_mm) as f32);
        s.traces.average_length = Some(if !trace_keys.is_empty() { total_length / trace_keys.len() as f32 } else { 0.0 });
        let mut segment_count = 0i32;
        let (mut horizontal, mut vertical, mut angled) = (0.0f32, 0.0f32, 0.0f32);
        for &k in &trace_keys {
            let polyline = board.item(k).trace().polyline();
            let corner_count = polyline.lines.len() as i32 - 1;
            if corner_count > 1 {
                segment_count += corner_count - 1;
            }
            for line in polyline.lines.iter() {
                let a = line.a.to_float();
                let b = line.b.to_float();
                let dx = a.x - b.x;
                let dy = a.y - b.y;
                // Math.pow(d, 2) is exactly d * d (fdlibm and the HotSpot intrinsic)
                let length = (dx * dx + dy * dy).sqrt() as f32;
                if a.x == b.x {
                    vertical += length;
                } else if a.y == b.y {
                    horizontal += length;
                } else {
                    angled += length;
                }
            }
        }
        s.traces.total_segment_count = Some(segment_count);
        s.traces.total_horizontal_length = Some(horizontal);
        s.traces.total_vertical_length = Some(vertical);
        s.traces.total_angled_length = Some(angled);

        let mut weighted = 0.0f32;
        let default_class = BoardRules::default_clearance_class();
        for &k in &items {
            let it = board.item(k);
            if !it.is_trace() {
                continue;
            }
            let fixed = it.fixed_state();
            if fixed == FixedState::Unfixed || fixed == FixedState::ShoveFixed {
                let t = it.trace();
                let mut w = t.length() * t.half_width().wrapping_add(board.clearance_value(it.clearance_class(), default_class, t.layer())) as f64;
                if fixed == FixedState::ShoveFixed {
                    w /= 2.0;
                }
                weighted += w as f32;
            }
        }
        s.traces.total_weighted_length = Some(weighted);

        // Connections
        if include_connections {
            let mut drc = DesignRulesChecker::new();
            drc.calculate_all_incompletes(board);
            s.connections.maximum_count = Some(drc.max_connections);
            s.connections.incomplete_count = Some(drc.get_incomplete_count(board));
        }

        // Bends
        let (mut bends_total, mut ninety, mut forty_five, mut other_angle) = (0i32, 0i32, 0i32, 0i32);
        for &k in &trace_keys {
            let polyline = board.item(k).trace().polyline();
            let corner_count = polyline.lines.len() as i32 - 1;
            if corner_count < 3 {
                continue;
            }
            bends_total += corner_count - 2;
            for i in 1..corner_count - 1 {
                let prev = polyline.corner(i - 1).to_float();
                let current = polyline.corner(i).to_float();
                let next = polyline.corner(i + 1).to_float();
                let dx1 = current.x - prev.x;
                let dy1 = current.y - prev.y;
                let dx2 = next.x - current.x;
                let dy2 = next.y - current.y;
                let mut angle = ((fr_geom::jmath::atan2(dy2, dx2) - fr_geom::jmath::atan2(dy1, dx1)) * RADIANS_TO_DEGREES).abs();
                angle = java_min(angle, 360.0 - angle);
                angle = if angle > 180.0 { 360.0 - angle } else { angle };
                if (angle - 90.0).abs() < 1.0 {
                    ninety += 1;
                } else if (angle - 45.0).abs() < 1.0 || (angle - 135.0).abs() < 1.0 {
                    forty_five += 1;
                } else {
                    other_angle += 1;
                }
            }
        }
        s.bends = BoardStatisticsBends {
            total_count: Some(bends_total),
            ninety_degree_count: Some(ninety),
            forty_five_degree_count: Some(forty_five),
            other_angle_count: Some(other_angle),
        };

        // Vias
        let via_keys = board.get_vias();
        let (mut through, mut blind, mut buried) = (0i32, 0i32, 0i32);
        for &k in &via_keys {
            let v = board.item(k);
            let (first, last) = (v.first_layer(board), v.last_layer(board));
            if first == 0 && last == layer_count - 1 {
                through += 1;
            } else if first == 0 || last == layer_count - 1 {
                blind += 1;
            } else {
                buried += 1;
            }
        }
        s.vias = BoardStatisticsVias {
            total_count: Some(via_keys.len() as i32),
            through_hole_count: Some(through),
            blind_count: Some(blind),
            buried_count: Some(buried),
        };

        if include_clearance_violations {
            let violations = all_clearance_violations(board);
            let cv = &mut s.clearance_violations;
            cv.total_count = Some(violations.len() as i32);
            if !violations.is_empty() {
                let mut min_violation = f64::MAX;
                let mut max_violation = 0.0f64;
                let mut sum_violation = 0.0f64;
                for v in &violations {
                    let shortfall = java_max(0.0, v.expected_clearance - v.actual_clearance);
                    let shortfall_um = shortfall * board_unit_to_um;
                    min_violation = java_min(min_violation, shortfall_um);
                    max_violation = java_max(max_violation, shortfall_um);
                    sum_violation += shortfall_um;
                }
                cv.total_violation_um = Some(sum_violation);
                cv.min_violation_um = Some(min_violation);
                cv.max_violation_um = Some(max_violation);
                cv.avg_violation_um = Some(sum_violation / violations.len() as f64);
            } else {
                cv.total_violation_um = Some(0.0);
                cv.min_violation_um = Some(0.0);
                cv.max_violation_um = Some(0.0);
                cv.avg_violation_um = Some(0.0);
            }
            cv.pre_existing_count = Some(board.pre_existing_clearance_violations_count);
            cv.unfixable_count = Some(violations.iter().filter(|v| v.is_unfixable(board)).count() as i32);
            cv.router_introduced_count = Some(0.max((violations.len() as i32).wrapping_sub(board.pre_existing_clearance_violations_count)));
        } else {
            s.clearance_violations = BoardStatisticsClearanceViolations {
                total_count: Some(0),
                pre_existing_count: Some(0),
                unfixable_count: Some(0),
                router_introduced_count: Some(0),
                total_violation_um: Some(0.0),
                min_violation_um: Some(0.0),
                max_violation_um: Some(0.0),
                avg_violation_um: Some(0.0),
            };
        }

        // Convert all length values from the board unit to the preferred unit
        let unit = unit.unwrap_or(Unit::Mm);
        if unit != board.communication.unit {
            let from = board.communication.unit;
            let conv = |v: f32| Unit::scale(v as f64, from, unit) as f32;
            s.unit = Some(unit_name(unit));
            let b = s.board.bounding_box.unwrap();
            s.board.bounding_box = Some(Rect2DF { x: conv(b.x), y: conv(b.y), width: conv(b.width), height: conv(b.height) });
            let sz = s.board.size.unwrap();
            s.board.size = Some(Rect2DF { x: 0.0, y: 0.0, width: conv(sz.width), height: conv(sz.height) });
            let t = &mut s.traces;
            t.total_length = t.total_length.map(conv);
            t.total_weighted_length = t.total_weighted_length.map(conv);
            t.average_length = t.average_length.map(conv);
            t.total_horizontal_length = t.total_horizontal_length.map(conv);
            t.total_vertical_length = t.total_vertical_length.map(conv);
            t.total_angled_length = t.total_angled_length.map(conv);
        }

        // Fanout statistics
        let (mut total_smd, mut escaped, mut already_connected) = (0i32, 0i32, 0i32);
        for pin in board.get_smd_pins() {
            let p = board.item(pin);
            if p.net_count() > 0 {
                total_smd += 1;
                if board.unconnected_set(pin, p.net_number(0)).is_empty() {
                    already_connected += 1;
                }
                if is_pin_escaped(board, pin) {
                    escaped += 1;
                }
            }
        }
        s.fanout = BoardStatisticsFanout { total_smd_pins: total_smd, pins_to_escape: total_smd - already_connected, escaped_count: escaped };
        s
    }

    /// Java `ensureDifficulty()`.
    pub fn ensure_difficulty(&mut self) {
        let d = &mut self.difficulty;
        if d.difficulty_d.is_some() {
            return;
        }
        if d.pin_count.is_none_or(|v| v <= 0) {
            d.pin_count = Some(self.items.pin_count.filter(|&p| p > 0).or(self.pads.total_count.filter(|&p| p > 0)).unwrap_or(0));
        }
        if d.signal_layer_count.is_none_or(|v| v <= 0) {
            d.signal_layer_count =
                Some(self.layers.signal_count.filter(|&l| l > 0).or(self.layers.total_count.filter(|&l| l > 0)).unwrap_or(0));
        }
        if d.complexity_c.is_none_or(|v| v <= 0) {
            let pins = d.pin_count.unwrap_or(0);
            let layers = d.signal_layer_count.unwrap_or(0);
            d.complexity_c = Some(1.max(pins.wrapping_mul(layers)));
        }
        if d.difficulty_d.is_none() {
            d.difficulty_d = Some(d.complexity_c.unwrap() as f32);
        }
    }

    /// Java `calculateScore(scoringSettings)` (legacy score, higher is better).
    pub fn calculate_score(&self, scoring: &RoutingCostSettings) -> f32 {
        let maximum_score = self.get_maximum_score(scoring);
        let unrouted_penalty = scoring.unrouted_net_penalty.expect("NullPointerException: unroutedNetPenalty");
        let violation_penalty = scoring.clearance_violation_penalty.expect("NullPointerException: clearanceViolationPenalty");
        let bend_penalty = scoring.bend_penalty.expect("NullPointerException: bendPenalty");
        let penalties = npe(self.connections.incomplete_count) as f32 * unrouted_penalty
            + npe(self.clearance_violations.total_count) as f32 * violation_penalty
            + npe(self.bends.total_count) as f32 * bend_penalty;
        let trace_length_for_cost = match self.traces.total_length_mm {
            Some(v) => v,
            None => npe(self.traces.total_length),
        };
        let costs = (trace_length_for_cost as f64 * scoring.default_preferred_direction_trace_cost.expect("NullPointerException")
            + npe(self.vias.total_count).wrapping_mul(scoring.via_costs.expect("NullPointerException")) as f64) as f32;
        maximum_score - penalties - costs
    }

    /// Java `getMaximumScore(scoringSettings)`.
    pub fn get_maximum_score(&self, scoring: &RoutingCostSettings) -> f32 {
        npe(self.connections.maximum_count) as f32 * scoring.unrouted_net_penalty.expect("NullPointerException: unroutedNetPenalty")
    }

    /// Java `getLegacyNormalizedScore`.
    fn legacy_normalized_score(&self, scoring: &RoutingCostSettings) -> f32 {
        let maximum_score = self.get_maximum_score(scoring);
        if maximum_score <= 0.0 {
            return 0.0;
        }
        java_max_f32(0.0, self.calculate_score(scoring) / maximum_score) * 1000.0
    }

    /// Java `getRouterScore(RoutingCostSettings)` (legacy formula).
    pub fn get_router_score_legacy(&self, scoring: &RoutingCostSettings) -> f32 {
        self.legacy_normalized_score(scoring)
    }

    /// Java `getRouterScore(RouterSettings)`: V2 continuous score if configured, else legacy
    /// (`None` = Java `null` settings: the legacy score with the default routing costs).
    pub fn get_router_score(&mut self, settings: Option<&RouterSettings>) -> f32 {
        match settings {
            Some(s) if s.router_scoring.version == Some(RouterScoringVersion::V2Continuous) => self.v2_router_score(&s.router_scoring),
            _ => self.legacy_normalized_score(&legacy_scoring_or_default(settings)),
        }
    }

    fn v2_router_score(&mut self, settings: &RouterScoreSettings) -> f32 {
        self.ensure_difficulty();
        let difficulty = self.difficulty.difficulty_d.map_or(1.0, |d| java_max(1.0, d as f64));
        let connections = self.connections.maximum_count.map_or(0.0, |v| 0.max(v) as f64);
        let incomplete = self.connections.incomplete_count.map_or(0.0, |v| 0.max(v) as f64);
        let violation_count = self.clearance_violations.total_count.map_or(0.0, |v| 0.max(v) as f64);
        let violation_depth = self.clearance_violations.total_violation_um.map_or(0.0, |v| java_max(0.0, v));
        let split = java_min(1.0, java_max(0.0, value_or_default(settings.unrouted_free_fraction, 0.5) as f64));
        let first_half_weight = value_or_default(settings.unrouted_first_half_weight, 1000.0f32 / 3.0f32) as f64;
        let second_half_weight = value_or_default(settings.unrouted_second_half_weight, 2000.0f32 / 3.0f32) as f64;
        let open_fraction = if connections > 0.0 { incomplete / connections } else { 0.0 };
        let (first_half_open, second_half_open) = if connections <= 0.0 {
            (0.0, 0.0)
        } else if split <= 0.0 {
            (0.0, open_fraction)
        } else if split >= 1.0 {
            (open_fraction, 0.0)
        } else {
            (
                java_min(1.0, java_max(0.0, (open_fraction - split) / (1.0 - split))),
                java_min(1.0, open_fraction / split),
            )
        };
        let unrouted_penalty = first_half_weight * first_half_open + second_half_weight * second_half_open;
        let mut drc_penalty = value_or_default(settings.clearance_violation_count_weight, 25.0) as f64 * violation_count / difficulty;
        let depth_scale = java_max(1.0, value_or_default(settings.clearance_violation_depth_scale, 1000.0) as f64);
        drc_penalty += value_or_default(settings.clearance_violation_depth_weight, 300.0) as f64 * violation_depth / depth_scale / difficulty;
        java_max(0.0, 1000.0 - unrouted_penalty - drc_penalty) as f32
    }

    /// Java `getOptimizerScore(RoutingCostSettings)` (legacy formula).
    pub fn get_optimizer_score_legacy(&self, scoring: &RoutingCostSettings) -> f32 {
        self.legacy_normalized_score(scoring)
    }

    /// Java `getOptimizerScore(RouterSettings)`: V2 lower-bound score if configured, else legacy.
    pub fn get_optimizer_score(&self, settings: Option<&RouterSettings>) -> f32 {
        match settings {
            Some(s) if s.optimizer_scoring.version == Some(OptimizerScoringVersion::V2LowerBound) => self.v2_optimizer_score(&s.optimizer_scoring),
            _ => self.legacy_normalized_score(&legacy_scoring_or_default(settings)),
        }
    }

    /// fastroute: the V2 optimizer score without the clamp at 0, so improvements stay visible on
    /// boards whose excess length/vias/bends push the score below 0 (e.g. DAC2020 bm01).
    /// Other scoring versions return the regular score.
    pub fn get_optimizer_score_unclamped(&self, settings: Option<&RouterSettings>) -> f32 {
        match settings {
            Some(s) if s.optimizer_scoring.version == Some(OptimizerScoringVersion::V2LowerBound) => {
                self.v2_optimizer_score_raw(&s.optimizer_scoring) as f32
            }
            _ => self.get_optimizer_score(settings),
        }
    }

    fn v2_optimizer_score(&self, settings: &OptimizerScoreSettings) -> f32 {
        java_max(0.0, self.v2_optimizer_score_raw(settings)) as f32
    }

    fn v2_optimizer_score_raw(&self, settings: &OptimizerScoreSettings) -> f64 {
        let difficulty = self.difficulty.difficulty_d.map_or(1.0, |d| java_max(1.0, d as f64));
        let min_trace_length = self.bounds.min_trace_length_mm.map_or(0.0, |v| java_max(0.0, v as f64));
        let min_via_count = self.bounds.min_via_count.map_or(0.0, |v| 0.max(v) as f64);
        let min_bend_count = self.bounds.min_bend_count.map_or(0.0, |v| 0.max(v) as f64);
        let actual_trace_length = self.traces.total_length_mm.map_or(0.0, |v| java_max(0.0, v as f64));
        let actual_via_count = self.vias.total_count.map_or(0.0, |v| 0.max(v) as f64);
        let actual_bend_count = self.bends.total_count.map_or(0.0, |v| 0.max(v) as f64);
        let length_floor = java_max(0.0, value_or_default(settings.length_floor, 1.0) as f64);
        let difficulty_floor = java_max(1.0, value_or_default(settings.difficulty_scale_floor, 1.0) as f64);
        let length_penalty = value_or_default(settings.excess_wire_length_weight, 1000.0) as f64
            * java_max(0.0, actual_trace_length - min_trace_length)
            / java_max(min_trace_length, length_floor);
        let via_penalty = value_or_default(settings.excess_via_weight, 2000.0) as f64 * java_max(0.0, actual_via_count - min_via_count)
            / java_max(difficulty, difficulty_floor);
        let bend_penalty = value_or_default(settings.excess_bend_weight, 500.0) as f64 * java_max(0.0, actual_bend_count - min_bend_count)
            / java_max(difficulty, difficulty_floor);
        1000.0 - length_penalty - via_penalty - bend_penalty
    }
}

/// Java `BoardStatistics.isPinEscaped(pin)`: the pin has a violation-free trace, a
/// violation-free via with a trace or conduction area contact, or a conduction area contact.
pub fn is_pin_escaped(board: &BasicBoard, pin: ItemKey) -> bool {
    for contact in board.normal_contacts(pin).iter() {
        let c = board.item(contact);
        if c.is_trace() {
            if clearance_violations(board, contact).is_empty() {
                return true;
            }
        } else if c.is_via() {
            if clearance_violations(board, contact).is_empty() {
                for vc in board.normal_contacts(contact).iter() {
                    let v = board.item(vc);
                    if v.is_trace() || v.is_conduction_area() {
                        return true;
                    }
                }
            }
        } else if c.is_conduction_area() {
            return true;
        }
    }
    false
}

/// Java `legacyScoringOrDefault`.
fn legacy_scoring_or_default(settings: Option<&RouterSettings>) -> RoutingCostSettings {
    match settings {
        Some(s) => s.scoring.clone(),
        None => fr_settings::default_settings(1).scoring,
    }
}

fn value_or_default(value: Option<f32>, default: f32) -> f32 {
    value.unwrap_or(default)
}

fn npe<T>(v: Option<T>) -> T {
    v.expect("NullPointerException: unboxing a null statistics value")
}

/// Java `Math.max(double, double)` (NaN wins, +0.0 > -0.0).
pub(crate) fn java_max(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && a.to_bits() == (-0.0f64).to_bits() {
        return b;
    }
    if a >= b {
        a
    } else {
        b
    }
}

/// Java `Math.min(double, double)` (NaN wins, -0.0 < +0.0).
pub(crate) fn java_min(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && b.to_bits() == (-0.0f64).to_bits() {
        return b;
    }
    if a <= b {
        a
    } else {
        b
    }
}

/// Java `Math.max(float, float)`.
pub(crate) fn java_max_f32(a: f32, b: f32) -> f32 {
    java_max(a as f64, b as f64) as f32
}

/// Java `TextManager.unescapeUnicode`: replaces `\uXXXX` escapes by the UTF-16 unit (surrogate
/// pairs are combined; a lone surrogate becomes U+FFFD).
pub fn unescape_unicode(text: &str) -> String {
    if !text.contains("\\u") {
        return text.to_string();
    }
    let mut units: Vec<u16> = Vec::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 5 < chars.len() && chars[i + 1] == 'u' && chars[i + 2..i + 6].iter().all(|c| c.is_ascii_hexdigit()) {
            let hex: String = chars[i + 2..i + 6].iter().collect();
            units.push(u16::from_str_radix(&hex, 16).unwrap());
            i += 6;
        } else {
            let mut buf = [0u16; 2];
            units.extend_from_slice(chars[i].encode_utf16(&mut buf));
            i += 1;
        }
    }
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescape_unicode_replaces_escapes() {
        assert_eq!(unescape_unicode("KiCad,9.0"), "KiCad,9.0");
        assert_eq!(unescape_unicode("a\\u00e9b\\u20ac"), "a\u{e9}b\u{20ac}");
        assert_eq!(unescape_unicode("\\ud83d\\ude00!"), "\u{1f600}!");
        assert_eq!(unescape_unicode("\\u12"), "\\u12");
    }

    #[test]
    fn java_min_max_semantics() {
        assert!(java_max(f64::NAN, 1.0).is_nan());
        assert!(java_max(1.0, f64::NAN).is_nan());
        assert!(java_min(1.0, f64::NAN).is_nan());
        assert_eq!(java_max(-0.0, 0.0).to_bits(), 0.0f64.to_bits());
        assert_eq!(java_max(0.0, -0.0).to_bits(), 0.0f64.to_bits());
        assert_eq!(java_min(0.0, -0.0).to_bits(), (-0.0f64).to_bits());
        assert_eq!(java_min(-0.0, 0.0).to_bits(), (-0.0f64).to_bits());
        assert!(java_max_f32(0.0, f32::NAN).is_nan());
    }

    #[test]
    fn ensure_difficulty_falls_back_to_pads_and_layers() {
        let mut s = BoardStatistics::default();
        s.pads.total_count = Some(7);
        s.layers.total_count = Some(3);
        s.ensure_difficulty();
        assert_eq!(s.difficulty.pin_count, Some(7));
        assert_eq!(s.difficulty.signal_layer_count, Some(3));
        assert_eq!(s.difficulty.complexity_c, Some(21));
        assert_eq!(s.difficulty.difficulty_d, Some(21.0));
        let mut empty = BoardStatistics::default();
        empty.ensure_difficulty();
        assert_eq!(empty.difficulty.complexity_c, Some(1));
    }

    #[test]
    fn scores_of_board_without_connections() {
        let settings = fr_settings::default_settings(1);
        let mut s = BoardStatistics::default();
        s.connections.maximum_count = Some(0);
        s.connections.incomplete_count = Some(0);
        s.clearance_violations.total_count = Some(0);
        s.bends.total_count = Some(0);
        s.traces.total_length_mm = Some(0.0);
        s.vias.total_count = Some(0);
        // legacy: maximum score 0 -> 0
        assert_eq!(s.get_router_score_legacy(&settings.scoring), 0.0);
        // V2 router: no connections, no violations -> 1000
        let mut v2 = settings.clone();
        v2.router_scoring.version = Some(RouterScoringVersion::V2Continuous);
        assert_eq!(s.get_router_score(Some(&v2)), 1000.0);
    }
}
