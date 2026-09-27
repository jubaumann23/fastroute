//! Port of the board-changing parts of `board/trace/PolylineTrace.java` (`combine`, `split`,
//! `change`), `PolylineTraceNormalization.java` and `PolylineTraceSearchTreeAdapter.java`.

use fr_geom::{IntOctagon, Line, LineSegment, Point, Polyline};

use crate::ids::ItemId;

use super::basic_board::BasicBoard;
use super::item::{ItemKey, ItemKind};
use super::search_tree::{TreeObject, DEFAULT_TREE};

/// Java `PolylineTraceNormalization.MAX_NORMALIZATION_DEPTH`.
const MAX_NORMALIZATION_DEPTH: i32 = 16;

/// How Java's object identity of the lines of the old and the new polyline in
/// `PolylineTrace.change` is modelled (the port has no line objects).
///
/// Java keeps `Line` objects when polylines are cut, combined or built from line arrays, and
/// creates new objects in `Line.opposite()` (so in `Polyline.reverse()`) and when lines are
/// constructed. A caller of [`BasicBoard::change_trace_with`] must pick the variant that
/// reproduces the Java provenance of the new polyline's lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineIdentity {
    /// Lines with identical defining points are the same object (right when the new polyline
    /// reuses the old line objects and all new lines differ geometrically).
    SamePoints,
    /// No line of the new polyline is an object of the old one (e.g. the new polyline was
    /// created by `reverse()`).
    NoneShared,
}

impl LineIdentity {
    fn same(self, a: &Line, b: &Line) -> bool {
        match self {
            LineIdentity::SamePoints => a.a == b.a && a.b == b.b,
            LineIdentity::NoneShared => false,
        }
    }
}

impl BasicBoard {
    /// Java `PolylineTrace.combine()`: combines this trace with traces at its ends (this trace
    /// remains). Returns true if something was combined.
    pub fn combine_trace(&mut self, key: ItemKey) -> bool {
        let mut something_changed = false;
        while self.item(key).is_on_board() && (self.combine_at_start(key) || self.combine_at_end(key)) {
            something_changed = true;
            self.additional_update_after_change(key);
        }
        something_changed
    }

    /// The single polyline trace to combine with at `corner` (Java loop in `combineAtStart` /
    /// `combineAtEnd`). Returns `(other, other_corner_matches_first)`.
    fn combine_partner(&self, key: ItemKey, corner: &Point, at_start: bool) -> Option<(ItemKey, bool)> {
        let mut contacts = self.trace_normal_contacts_at(key, corner, false);
        // remove conduction areas from the list (ignoreAreas == true)
        contacts.retain(|k| !self.item(k).is_conduction_area());
        if contacts.len() != 1 {
            return None;
        }
        let this = self.item(key);
        let t = this.trace();
        for other_key in contacts.iter() {
            let other = self.item(other_key);
            let Some(ot) = other.as_trace() else { continue };
            if ot.layer == t.layer
                && other.nets_equal(this)
                && ot.half_width == t.half_width
                && other.fixed_state() == this.fixed_state()
                && !self.is_deletion_forbidden(other_key)
                && !self.is_deletion_forbidden(key)
            {
                if at_start {
                    if *corner == other.last_corner() {
                        return Some((other_key, false));
                    } else if *corner == other.first_corner() {
                        return Some((other_key, true));
                    }
                } else if *corner == other.first_corner() {
                    return Some((other_key, false));
                } else if *corner == other.last_corner() {
                    return Some((other_key, true));
                }
            }
        }
        None
    }

    fn other_lines(&self, other: ItemKey, reverse_order: bool) -> Vec<Line> {
        let lines = &self.item(other).trace().polyline.lines;
        if reverse_order {
            lines.iter().rev().map(|l| l.opposite()).collect()
        } else {
            lines.to_vec()
        }
    }

    fn has_default_entries(&self, a: ItemKey, b: ItemKey) -> bool {
        let tree = self.default_tree();
        tree.item_leaves(a).is_some() && tree.item_leaves(b).is_some()
    }

    /// Java `PolylineTraceSearchTreeAdapter.replaceGeometry`.
    pub(crate) fn replace_trace_geometry(&mut self, key: ItemKey, new_polyline: Polyline) {
        self.tree_remove(key);
        self.clear_search_tree_entries(key);
        self.item_mut(key).trace_mut().polyline = new_polyline;
        self.clear_derived_data(key);
        self.tree_insert(key);
    }

    /// Java `PolylineTrace.combineAtStart(true)`.
    fn combine_at_start(&mut self, key: ItemKey) -> bool {
        let start_corner = self.item(key).first_corner();
        let Some((other, reverse_order)) = self.combine_partner(key, &start_corner, true) else {
            return false;
        };
        let this_lines: Vec<Line> = self.item(key).trace().polyline.lines.to_vec();
        let other_lines = self.other_lines(other, reverse_order);
        let skip_line = other_lines[other_lines.len() - 2].is_equal_or_opposite(&this_lines[1]);
        let mut new_line_count = this_lines.len() + other_lines.len() - 2;
        if skip_line {
            new_line_count -= 1;
        }
        let mut join_pos = other_lines.len() - 1;
        if skip_line {
            join_pos -= 1;
        }
        let mut new_lines: Vec<Line> = other_lines[..join_pos].to_vec();
        new_lines.extend_from_slice(&this_lines[1..]);
        debug_assert_eq!(new_lines.len(), new_line_count);
        let joined = Polyline::from_lines(new_lines);
        let has_tree_entries = self.has_default_entries(key, other);
        if joined.lines.len() != new_line_count || !has_tree_entries {
            // consecutive parallel lines were skipped at the join location or a trace lacks
            // search tree entries: combine without performance optimization
            self.replace_trace_geometry(key, joined);
        } else {
            let mut to_no = other_lines.len() as i32;
            if skip_line {
                to_no -= 1;
            }
            self.merge_entries_in_front(other, key, &joined, other_lines.len() as i32 - 3, to_no);
            self.clear_search_tree_entries(other);
            self.item_mut(key).trace_mut().polyline = joined;
        }
        if self.item(key).trace().polyline.lines.len() < 3 {
            self.remove_item(key);
        }
        self.remove_item(other);
        let layer = self.item(key).trace().layer;
        if let Some(changed_area) = &mut self.changed_area {
            changed_area.join(&start_corner.to_float(), layer);
        }
        true
    }

    /// Java `PolylineTrace.combineAtEnd(true)`.
    fn combine_at_end(&mut self, key: ItemKey) -> bool {
        let end_corner = self.item(key).last_corner();
        let Some((other, reverse_order)) = self.combine_partner(key, &end_corner, false) else {
            return false;
        };
        let this_lines: Vec<Line> = self.item(key).trace().polyline.lines.to_vec();
        let other_lines = self.other_lines(other, reverse_order);
        let skip_line = this_lines[this_lines.len() - 2].is_equal_or_opposite(&other_lines[1]);
        let mut new_line_count = this_lines.len() + other_lines.len() - 2;
        if skip_line {
            new_line_count -= 1;
        }
        let mut join_pos = this_lines.len() - 1;
        if skip_line {
            join_pos -= 1;
        }
        let mut new_lines: Vec<Line> = this_lines[..join_pos].to_vec();
        new_lines.extend_from_slice(&other_lines[1..]);
        debug_assert_eq!(new_lines.len(), new_line_count);
        let joined = Polyline::from_lines(new_lines);
        let has_tree_entries = self.has_default_entries(key, other);
        if joined.lines.len() != new_line_count || !has_tree_entries {
            self.replace_trace_geometry(key, joined);
        } else {
            let mut to_no = this_lines.len() as i32;
            if skip_line {
                to_no -= 1;
            }
            self.merge_entries_at_end(other, key, &joined, this_lines.len() as i32 - 3, to_no);
            self.clear_search_tree_entries(other);
            self.item_mut(key).trace_mut().polyline = joined;
        }
        if self.item(key).trace().polyline.lines.len() < 3 {
            self.remove_item(key);
        }
        self.remove_item(other);
        let layer = self.item(key).trace().layer;
        if let Some(changed_area) = &mut self.changed_area {
            changed_area.join(&end_corner.to_float(), layer);
        }
        true
    }

    /// Java `PolylineTrace.split(clipShape)`: splits this trace and the traces intersecting it
    /// at the intersection points and removes cycles. Returns the pieces of this trace (just
    /// this trace if it was not split).
    pub fn split_trace(&mut self, key: ItemKey, clip_shape: Option<&IntOctagon>) -> Vec<ItemKey> {
        let mut result: Vec<ItemKey> = Vec::new();
        if !self.item(key).nets_normal() {
            // only normal nets are split
            result.push(key);
            return result;
        }
        let mut own_trace_split = false;
        let line_count = self.item(key).trace().polyline.lines.len();
        let layer = self.item(key).trace().layer;
        for i in 0..line_count.saturating_sub(2) as i32 {
            let lines = self.item(key).trace().polyline.clone();
            if let Some(clip) = clip_shape {
                let segment = LineSegment::from_polyline(&lines, i + 1).expect("LineSegment");
                if !clip.intersects_int_box(&segment.bounding_box()) {
                    continue;
                }
            }
            let current_shape = self.tree_shape(DEFAULT_TREE, key, i).expect("PolylineTrace.split: tree shape is null");
            let current_line_segment = LineSegment::from_polyline(&lines, i + 1).expect("LineSegment");
            let query = fr_geom::ConvexShape::Tile(current_shape);
            // look for intersecting traces with the i-th line segment
            let mut entries = self.overlapping_tree_entries_list(DEFAULT_TREE, &query, layer, &[]);
            let mut pos = 0;
            while pos < entries.len() {
                if !self.item(key).is_on_board() {
                    // this trace has been deleted in a cleanup operation
                    return result;
                }
                let entry = entries[pos];
                pos += 1;
                let TreeObject::Item { key: found_key, .. } = entry.object else { continue };
                if found_key == key {
                    let idx = entry.shape_index;
                    if idx >= i - 1 && idx <= i + 1 {
                        // don't split own trace at this line or at neighbour lines
                        continue;
                    }
                    // try to handle intermediate segments of length 0 by comparing end corners
                    if i < idx {
                        if lines.corner(i + 1) == lines.corner(idx) {
                            continue;
                        }
                    } else if lines.corner(idx + 1) == lines.corner(i) {
                        continue;
                    }
                }
                if !self.item(found_key).shares_net(self.item(key)) {
                    continue;
                }
                match &self.item(found_key).kind {
                    ItemKind::Trace(found_trace) => {
                        let found_segment = LineSegment::from_polyline(&found_trace.polyline, entry.shape_index + 1).expect("LineSegment");
                        let intersecting_lines = found_segment.intersection(&current_line_segment);
                        let mut split_pieces: Vec<ItemKey> = Vec::new();
                        // try splitting the found trace first
                        let mut found_trace_split = false;
                        if found_key != key {
                            for line in &intersecting_lines {
                                let line_index = entry.shape_index + 1;
                                if let Some(pieces) = self.split_trace_at_line(found_key, line_index, line) {
                                    for p in pieces.into_iter().flatten() {
                                        found_trace_split = true;
                                        split_pieces.push(p);
                                    }
                                    if found_trace_split {
                                        // reread the overlapping tree entries, because the board
                                        // has changed
                                        entries = self.overlapping_tree_entries_list(DEFAULT_TREE, &query, layer, &[]);
                                        pos = 0;
                                        break;
                                    }
                                }
                            }
                            if !found_trace_split {
                                split_pieces.push(found_key);
                            }
                        }
                        // now try splitting the own trace
                        let intersecting_lines = current_line_segment.intersection(&found_segment);
                        for line in &intersecting_lines {
                            if let Some(pieces) = self.split_trace_at_line(key, i + 1, line) {
                                own_trace_split = true;
                                // this trace was split itself into 2.
                                if let Some(p0) = pieces[0] {
                                    let sub = self.split_trace(p0, clip_shape);
                                    result.extend(sub);
                                }
                                if let Some(p1) = pieces[1] {
                                    let sub = self.split_trace(p1, clip_shape);
                                    result.extend(sub);
                                }
                                break;
                            }
                        }
                        if found_trace_split || own_trace_split {
                            // something was split, remove cycles containing a split piece
                            for piece in split_pieces {
                                self.remove_if_cycle(piece);
                            }
                            // remove cycles in the own split pieces last to preserve them, if
                            // possible
                            let own: Vec<ItemKey> = result.clone();
                            for piece in own {
                                self.remove_if_cycle(piece);
                            }
                        }
                        if own_trace_split {
                            break;
                        }
                    }
                    ItemKind::Pin(_) | ItemKind::Via(_) => {
                        let split_point = self.item(found_key).center(self);
                        if current_line_segment.contains(&split_point) {
                            let dir = current_line_segment.get_line().direction().turn_45_degree(2);
                            let split_line = Line::from_point_direction(split_point, dir);
                            self.split_trace_at_line(key, i + 1, &split_line);
                        }
                    }
                    ItemKind::ConductionArea(_) if !self.item(key).is_user_fixed() => {
                        let mut ignore_areas = false;
                        if let Some(&n) = self.item(key).net_numbers.first() {
                            if let Some(net) = self.rules.nets.get(n) {
                                ignore_areas = self.rules.net_classes[net.get_net_class()].get_ignore_cycles_with_areas();
                            }
                        }
                        let found_id = self.item(found_key).id();
                        if !ignore_areas && self.trace_start_contacts(key).contains(found_id) && self.trace_end_contacts(key).contains(found_id) {
                            // this trace can be removed because of cycle with conduction area
                            self.remove_item(key);
                            return result;
                        }
                    }
                    _ => {}
                }
            }
            if own_trace_split {
                break;
            }
        }
        if !own_trace_split {
            result.push(key);
        }
        if result.len() > 1 {
            for k in result.clone() {
                self.additional_update_after_change(k);
            }
        }
        result
    }

    /// Java `PolylineTrace.split(Point)`: splits the trace at `point`. Returns the two pieces
    /// (either may be `None`), or `None` if nothing was split.
    pub fn split_trace_at_point(&mut self, key: ItemKey, point: &Point) -> Option<[Option<ItemKey>; 2]> {
        let lines = self.item(key).trace().polyline.clone();
        for i in 0..lines.lines.len().saturating_sub(2) as i32 {
            let segment = LineSegment::from_polyline(&lines, i + 1).expect("LineSegment");
            if segment.contains(point) {
                let dir = segment.get_line().direction().turn_45_degree(2);
                let split_line = Line::from_point_direction(point.clone(), dir);
                if let Some(result) = self.split_trace_at_line(key, i + 1, &split_line) {
                    return Some(result);
                }
            }
        }
        None
    }

    /// Java private `PolylineTrace.split(lineIndex, newEndLine)`.
    pub(crate) fn split_trace_at_line(&mut self, key: ItemKey, line_index: i32, new_end_line: &Line) -> Option<[Option<ItemKey>; 2]> {
        if !self.item(key).is_on_board() {
            return None;
        }
        // splitting a trace that cannot be deleted would duplicate it
        if self.is_deletion_forbidden(key) {
            return None;
        }
        let split_polylines = self.item(key).trace().polyline.split(line_index, new_end_line)?;
        if self.split_inside_drill_pad_prohibited(key, line_index, new_end_line) {
            return None;
        }
        self.remove_item(key);
        let item = self.item(key);
        let (layer, half_width, nets, cl, fixed) = (
            item.trace().layer,
            item.trace().half_width,
            item.net_numbers.clone(),
            item.clearance_class,
            item.fixed_state,
        );
        let [first, second] = split_polylines;
        let r0 = self.insert_trace_without_cleaning(first, layer, half_width, &nets, cl, fixed);
        let r1 = self.insert_trace_without_cleaning(second, layer, half_width, &nets, cl, fixed);
        Some([r0, r1])
    }

    /// Java `PolylineTrace.splitInsideDrillPadProhibited(lineIndex, line)`.
    fn split_inside_drill_pad_prohibited(&self, key: ItemKey, line_index: i32, line: &Line) -> bool {
        let this = self.item(key);
        let intersection = this.trace().polyline.lines[line_index as usize].intersection(line);
        let overlap_items = self.pick_items(&intersection, this.trace().layer, None);
        let mut pad_found = false;
        for k in overlap_items.iter() {
            let current = self.item(k);
            if !current.shares_net(this) {
                continue;
            }
            if current.is_pin() {
                if current.center(self) == intersection {
                    return false; // split always at the center of a drill item.
                }
                pad_found = true;
            } else if current.is_trace() && ((k != key && current.first_corner() == intersection) || current.last_corner() == intersection) {
                return false;
            }
        }
        pad_found
    }

    /// Java `PolylineTrace.normalize(clipShape)` (`PolylineTraceNormalization.normalize`):
    /// splits this trace and overlapping traces and combines the pieces. Returns true if
    /// something was changed.
    pub fn normalize_trace(&mut self, key: ItemKey, clip_shape: Option<&IntOctagon>) -> bool {
        self.normalize_trace_depth(key, clip_shape, 0)
    }

    fn normalize_trace_depth(&mut self, key: ItemKey, clip_shape: Option<&IntOctagon>, depth: i32) -> bool {
        if depth > MAX_NORMALIZATION_DEPTH {
            log::debug!("PolylineTrace.normalize: max normalization depth reached for trace {:?}", self.item(key).id());
            return false;
        }
        let split_pieces = self.split_trace(key, clip_shape);
        let mut result = split_pieces.len() != 1;
        for piece in split_pieces {
            if self.item(piece).is_on_board() {
                let trace_combined = self.combine_trace(piece);
                let item = self.item(piece);
                if item.trace().corner_count() == 2 && item.first_corner() == item.last_corner() {
                    // remove trace with only 1 corner, if deletion is allowed
                    if !self.is_deletion_forbidden(piece) {
                        self.remove_item(piece);
                        result = true;
                    } else {
                        log::debug!("PolylineTrace.normalize: skipping removal of degenerate user-fixed trace {:?}", item.id());
                    }
                } else if trace_combined {
                    self.normalize_trace_depth(piece, clip_shape, depth + 1);
                    result = true;
                }
            }
        }
        result
    }

    /// Java `PolylineTrace.change(newPolyline)`: changes the geometry, reusing the tree entries
    /// of unchanged lines, and normalizes the trace.
    ///
    /// Java compares the lines of the old and new polyline by object identity; this uses
    /// [`LineIdentity::SamePoints`], see [`Self::change_trace_with`].
    pub fn change_trace(&mut self, key: ItemKey, new_polyline: Polyline) {
        self.change_trace_with(key, new_polyline, LineIdentity::SamePoints);
    }

    /// Java `PolylineTrace.change(newPolyline)` with an explicit model of Java's line object
    /// identity (which decides how many tree entries are reused, and so the tree structure).
    pub fn change_trace_with(&mut self, key: ItemKey, new_polyline: Polyline, identity: LineIdentity) {
        if !self.item(key).is_on_board() {
            // Just change the polyline of this trace.
            self.item_mut(key).trace_mut().polyline = new_polyline;
            return;
        }
        self.additional_update_after_change(key);
        let old = self.item(key).trace().polyline.clone();
        let new_len = new_polyline.lines.len();
        let old_len = old.lines.len();
        let last_index = new_len.min(old_len);
        let mut first_diff = last_index;
        for i in 0..last_index {
            if !identity.same(&new_polyline.lines[i], &old.lines[i]) {
                first_diff = i;
                break;
            }
        }
        if first_diff == last_index {
            return; // both polylines are equal, no change necessary
        }
        let mut last_diff: i64 = -1;
        for i in 1..=last_index {
            if !identity.same(&new_polyline.lines[new_len - i], &old.lines[old_len - i]) {
                last_diff = (new_len - i) as i64;
                break;
            }
        }
        if last_diff < 0 {
            return;
        }
        let keep_at_start = (first_diff as i32 - 2).max(0);
        let keep_at_end = (new_len as i32 - last_diff as i32 - 3).max(0);
        self.change_entries(key, &new_polyline, keep_at_start, keep_at_end);
        let layer = self.item(key).trace().layer;
        self.item_mut(key).trace_mut().polyline = new_polyline;
        let clip_shape = self.changed_area.as_ref().map(|c| c.get_area(layer));
        self.normalize_trace(key, clip_shape.as_ref());
    }

    /// Java `ShapeTraceEntries.cutoutTrace` fast path helper `fastCutoutTrace`: replaces `trace`
    /// by the two pieces, reusing its tree entries.
    pub fn fast_cutout_trace(&mut self, trace: ItemKey, start_piece: Polyline, end_piece: Polyline) -> [ItemKey; 2] {
        self.additional_update_after_change(trace);
        let (layer, half_width, nets, cl) = {
            let item = self.item(trace);
            (item.trace().layer, item.trace().half_width, item.net_numbers.clone(), item.clearance_class)
        };
        let lc = self.layer_count();
        let start_id = ItemId(crate::datastructures::IdGenerator::new_id(&mut self.communication.id_generator));
        let start = super::item::Item::new_trace(start_id, start_piece, layer, half_width, &nets, cl, 0, crate::ids::FixedState::Unfixed, lc);
        let start_key = self.items.alloc(start);
        self.items.list_insert(start_key);
        self.items.get_mut(start_key).on_board = true;
        let end_id = ItemId(crate::datastructures::IdGenerator::new_id(&mut self.communication.id_generator));
        let end = super::item::Item::new_trace(end_id, end_piece, layer, half_width, &nets, cl, 0, crate::ids::FixedState::Unfixed, lc);
        let end_key = self.items.alloc(end);
        self.items.list_insert(end_key);
        self.items.get_mut(end_key).on_board = true;
        self.reuse_entries_after_cutout(trace, start_key, end_key);
        self.remove_item(trace);
        [start_key, end_key]
    }

}
