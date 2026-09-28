//! Port of `board/searchtree/ShapeTraceEntries.java`: auxiliary class of the shove functions
//! (used by `RoutingBoard` / the trace shover, porting unit U7).
//!
//! The Java singly linked list of `EntryPoint`s is a `Vec` arena with `next` indices; the list
//! order and all relinking operations are the same.

use fr_geom::{FloatPoint, Point, TileShape};

use crate::ids::{ClearanceClassNo, FixedState, ItemId, LayerNo, NetNo};
use crate::structure::ShapeEntrySide;

use super::basic_board::BasicBoard;
use super::item::{Item, ItemKey, ItemKind};
use super::optimize::tracked::{BorderLines, TLine, TPolyline};

/// Java `ShapeTraceEntries.c_offset_add`.
const C_OFFSET_ADD: f64 = 1.0;

#[derive(Clone, Debug)]
struct EntryPoint {
    trace: ItemKey,
    trace_line_no: i32,
    entry_approx: FloatPoint,
    edge_index: i32,
    stack_level: i32,
    next: Option<usize>,
}

/// Java `ShapeTraceEntries`: used for shoving traces and vias out of a shape.
#[derive(Clone, Debug)]
pub struct ShapeTraceEntries {
    /// Java `shoveViaList`.
    pub shove_via_list: Vec<ItemKey>,
    shape: TileShape,
    layer: LayerNo,
    own_net_nos: Vec<NetNo>,
    clearance_class: ClearanceClassNo,
    from_side: Option<ShapeEntrySide>,
    entries: Vec<EntryPoint>,
    list_anchor: Option<usize>,
    trace_piece_count: i32,
    max_stack_level: i32,
    shape_contains_trace_tails: bool,
    found_obstacle: Option<ItemKey>,
}

/// Java `netNosEqual`.
fn net_nos_equal(a: &[NetNo], b: &[NetNo]) -> bool {
    a.len() == b.len() && a.iter().all(|x| b.contains(x))
}

/// The offset shape used for a trace (`c_offset_add` included), Java inline code of
/// `cutoutTrace`, `nextSubstituteTracePiece` and `storeTrace`.
fn trace_offset_shape(board: &BasicBoard, shape: &TileShape, trace: &Item, clearance_class: ClearanceClassNo, layer: LayerNo) -> TileShape {
    let t = trace.trace();
    if board.default_tree().is_clearance_compensation_used() {
        let compensated = t.half_width() + board.clearance_compensation_value(super::search_tree::DEFAULT_TREE, trace.clearance_class(), t.layer());
        shape.offset(compensated as f64 + C_OFFSET_ADD)
    } else {
        // enlarge the shape in 2 steps for symmetry reasons
        let cl_offset = board.clearance_value(trace.clearance_class(), clearance_class, layer) as f64 + C_OFFSET_ADD;
        shape.offset(t.half_width() as f64).offset(cl_offset)
    }
}

impl BasicBoard {
    /// Java `ShapeTraceEntries.cutoutTrace(trace, shape, clearanceClassIndex)`.
    pub fn cutout_trace(&mut self, trace: ItemKey, shape: &TileShape, clearance_class: ClearanceClassNo) {
        if !self.item(trace).is_on_board() {
            log::warn!("ShapeTraceEntries.cutout_trace : trace is deleted");
            return;
        }
        let item = self.item(trace);
        let layer = item.trace().layer();
        let offset_shape = trace_offset_shape(self, shape, item, clearance_class, layer);
        let trace_lines = item.trace().tpolyline();
        let pieces = trace_lines.cutout(&offset_shape);
        if pieces.len() == 1 && pieces[0].same(&trace_lines) {
            // nothing cut off
            return;
        }
        if pieces.len() == 2
            && offset_shape.is_outside(&pieces[0].polyline.first_corner())
            && offset_shape.is_outside(&pieces[1].polyline.last_corner())
        {
            let mut it = pieces.into_iter();
            let start = it.next().unwrap();
            let end = it.next().unwrap();
            self.fast_cutout_trace(trace, start, end);
        } else {
            let (half_width, nets, cl) = (item.trace().half_width(), item.net_numbers().to_vec(), item.clearance_class());
            self.remove_item(trace);
            for piece in pieces {
                self.insert_trace_without_cleaning_tracked(piece, layer, half_width, &nets, cl, FixedState::Unfixed);
            }
        }
    }
}

impl ShapeTraceEntries {
    /// Java constructor. `from_side.no < 0` (or `None`) means it will be calculated.
    pub fn new(shape: TileShape, layer: LayerNo, own_net_nos: &[NetNo], clearance_class: ClearanceClassNo, from_side: Option<ShapeEntrySide>) -> Self {
        ShapeTraceEntries {
            shove_via_list: Vec::new(),
            shape,
            layer,
            own_net_nos: own_net_nos.to_vec(),
            clearance_class,
            from_side,
            entries: Vec::new(),
            list_anchor: None,
            trace_piece_count: 0,
            max_stack_level: 0,
            shape_contains_trace_tails: false,
            found_obstacle: None,
        }
    }

    /// Java `stackDepth()`.
    pub fn stack_depth(&self) -> i32 {
        self.max_stack_level
    }

    /// Java `substituteTraceCount()`.
    pub fn substitute_trace_count(&self) -> i32 {
        self.trace_piece_count
    }

    /// Java `traceTailsInShape()`.
    pub fn trace_tails_in_shape(&self) -> bool {
        self.shape_contains_trace_tails
    }

    /// Java `getFoundObstacle()`.
    pub fn found_obstacle(&self) -> Option<ItemKey> {
        self.found_obstacle
    }

    /// Java `storeItems(itemList, isPadCheck, copperSharingAllowed)`: returns false if the
    /// items contain obstacles which cannot be shoved aside.
    pub fn store_items(&mut self, board: &BasicBoard, items: &[ItemKey], is_pad_check: bool, copper_sharing_allowed: bool) -> bool {
        for &key in items {
            let item = board.item(key);
            if (!is_pad_check && item.is_via_obstacle_area()) || item.is_component_obstacle_area() {
                continue;
            }
            let contains_own_net = item.shares_net_no(&self.own_net_nos);
            if let ItemKind::ConductionArea(c) = &item.kind {
                if contains_own_net || !c.is_obstacle() {
                    continue;
                }
            }
            if item.is_board_outline() && !board.outline_blocks_nets(&self.own_net_nos) {
                continue;
            }
            if board.is_shove_fixed(key) && !contains_own_net {
                self.found_obstacle = Some(key);
                return false;
            }
            match &item.kind {
                ItemKind::Via(_) => {
                    if is_pad_check || !contains_own_net {
                        self.shove_via_list.push(key);
                    }
                }
                ItemKind::Trace(_) => {
                    if !self.store_trace(board, key) {
                        return false;
                    }
                }
                _ => {
                    if contains_own_net {
                        if !copper_sharing_allowed {
                            self.found_obstacle = Some(key);
                            return false;
                        }
                        if is_pad_check && !(item.is_pin() && item.drill_allowed(board)) {
                            self.found_obstacle = Some(key);
                            return false;
                        }
                    } else {
                        self.found_obstacle = Some(key);
                        return false;
                    }
                }
            }
        }
        self.search_from_side(board);
        self.resort(board);
        self.calculate_stack_levels(board)
    }

    /// Java `nextSubstituteTracePiece()`: the next substitute trace (not inserted; consumes an
    /// item id like the Java constructor). `None` at the end of the list.
    pub fn next_substitute_trace_piece(&mut self, board: &mut BasicBoard) -> Option<Item> {
        loop {
            let (first, last) = self.pop_piece(board)?;
            let trace_key = self.entries[first].trace;
            let current_trace = board.item(trace_key);
            let offset_shape = trace_offset_shape(board, &self.shape, current_trace, self.clearance_class, self.layer);
            let edge_count = self.shape.border_line_count();
            let edge_diff = self.entries[last].edge_index - self.entries[first].edge_index;
            // calculate the polyline of the substitute trace
            let mut border = BorderLines::new(&offset_shape);
            let mut piece_lines: Vec<TLine> = Vec::with_capacity((edge_diff + 3).max(2) as usize);
            // start with the intersecting line of the trace at the start entry.
            let first_trace = board.item(self.entries[first].trace).trace().tpolyline();
            piece_lines.push(first_trace.tline(self.entries[first].trace_line_no as usize));
            let mut current_edge_no = self.entries[first].edge_index % edge_count;
            for _ in 1..edge_diff + 2 {
                piece_lines.push(border.line(current_edge_no));
                if current_edge_no == edge_count - 1 {
                    current_edge_no = 0;
                } else {
                    current_edge_no += 1;
                }
            }
            // end with the intersecting line of the trace at the end entry
            let last_trace = board.item(self.entries[last].trace).trace().tpolyline();
            piece_lines.push(last_trace.tline(self.entries[last].trace_line_no as usize));
            let piece_polyline = TPolyline::from_lines(&mut piece_lines);
            if piece_polyline.is_empty() {
                // no valid trace piece, return the next one
                continue;
            }
            let t = board.item(trace_key);
            let (half_width, nets, cl) = (t.trace().half_width(), t.net_numbers().to_vec(), t.clearance_class());
            let id = ItemId(crate::datastructures::IdGenerator::new_id(&mut board.communication.id_generator));
            let layer_count = board.layer_count();
            return Some(Item::new_trace_tracked(id, piece_polyline, self.layer, half_width, &nets, cl, 0, FixedState::Unfixed, layer_count));
        }
    }

    /// Java `cutoutTraces(itemList)`: cuts all traces of foreign nets out of the shape.
    pub fn cutout_traces(&self, board: &mut BasicBoard, items: &[ItemKey]) {
        for &key in items {
            let item = board.item(key);
            if item.is_trace() && !item.shares_net_no(&self.own_net_nos) {
                board.cutout_trace(key, &self.shape, self.clearance_class);
            }
        }
    }

    /// Java `storeTrace(trace)`.
    fn store_trace(&mut self, board: &BasicBoard, trace_key: ItemKey) -> bool {
        let trace = board.item(trace_key);
        let tr = trace.trace();
        let offset_shape = trace_offset_shape(board, &self.shape, trace, self.clearance_class, tr.layer());
        // using enlarge here instead offset causes problems because of a comparison in the
        // constructor of class EntryPoint
        let entries = offset_shape.entrance_points(tr.polyline());
        for entry in &entries {
            let approx = tr.polyline().lines[entry[0] as usize].intersection_approx(&offset_shape.border_line(entry[1]));
            self.insert_entry_point(trace_key, entry[0], entry[1], approx);
        }
        // Look, if an end point of the trace lies in the interior of the shape. This may be the
        // case, if a via touches the shape.
        if !trace.shares_net_no(&self.own_net_nos) {
            if !trace.nets_normal() {
                return false;
            }
            let mut end_corner: Point = trace.first_corner();
            for i in 0..2 {
                if offset_shape.contains(&end_corner) {
                    let contact_list = if i == 0 { board.trace_start_contacts(trace_key) } else { board.trace_end_contacts(trace_key) };
                    let mut contact_count = 0;
                    let mut store_end_corner = true;
                    // check for contact object, which is not shovable
                    for contact_key in contact_list.iter() {
                        let contact = board.item(contact_key);
                        if !contact.is_routable() {
                            self.found_obstacle = Some(contact_key);
                            return false;
                        }
                        if let Some(ct) = contact.as_trace() {
                            // (Java compares the clearance class of the contact with itself here)
                            if (board.is_shove_fixed(contact_key) || ct.half_width() != tr.half_width()) && offset_shape.contains_inside(&end_corner)
                            {
                                self.found_obstacle = Some(contact_key);
                                return false;
                            }
                        } else if contact.is_via() {
                            let via_shape = board.tile_shape_on_layer(contact_key, self.layer).expect("via shape on layer");
                            let compensated = tr.half_width() + board.clearance_compensation_value(super::search_tree::DEFAULT_TREE, trace.clearance_class(), tr.layer());
                            let mut via_trace_diff = via_shape.smallest_radius() - compensated as f64;
                            if !board.default_tree().is_clearance_compensation_used() {
                                let via_clearance = board.clearance_value(contact.clearance_class(), self.clearance_class, self.layer);
                                let trace_clearance = board.clearance_value(trace.clearance_class(), self.clearance_class, self.layer);
                                if trace_clearance > via_clearance {
                                    via_trace_diff += (via_clearance - trace_clearance) as f64;
                                }
                            }
                            if via_trace_diff < 0.0 {
                                // the via is smaller than the trace
                                self.found_obstacle = Some(contact_key);
                                return false;
                            }
                            if via_trace_diff == 0.0 && !offset_shape.contains_inside(&end_corner) {
                                // the via need not to be shoved
                                store_end_corner = false;
                            }
                        }
                        contact_count += 1;
                    }
                    if contact_count == 1 && store_end_corner {
                        let projection = offset_shape.nearest_border_point(&end_corner).expect("nearest border point");
                        let projection_side = offset_shape.contains_on_border_line_no(&projection);
                        let trace_line_segment_no = if i == 0 { 0 } else { tr.polyline().lines.len() as i32 - 1 };
                        if projection_side >= 0 {
                            self.insert_entry_point(trace_key, trace_line_segment_no, projection_side, projection.to_float());
                        }
                    } else if contact_count == 0 && offset_shape.contains_inside(&end_corner) {
                        self.shape_contains_trace_tails = true;
                    }
                }
                end_corner = trace.last_corner();
            }
        }
        self.found_obstacle = Some(trace_key);
        true
    }

    fn nets<'b>(&self, board: &'b BasicBoard, entry: usize) -> &'b [NetNo] {
        board.item(self.entries[entry].trace).net_numbers()
    }

    /// Java `searchFromSide()`.
    fn search_from_side(&mut self, board: &BasicBoard) {
        if let Some(fs) = &self.from_side {
            if fs.no >= 0 {
                return; // from side is already legal
            }
        }
        let mut current = self.list_anchor;
        let mut from_side_no = 0;
        let mut entry_approx = None;
        while let Some(c) = current {
            if board.item(self.entries[c].trace).shares_net_no(&self.own_net_nos) {
                from_side_no = self.entries[c].edge_index;
                entry_approx = Some(self.entries[c].entry_approx);
                break;
            }
            current = self.entries[c].next;
        }
        self.from_side = Some(ShapeEntrySide::new(from_side_no, entry_approx));
    }

    /// Java `resort()`: resorts the intersection points according to the from side and removes
    /// redundant points.
    fn resort(&mut self, board: &BasicBoard) {
        let edge_count = self.shape.border_line_count();
        let from_side = self.from_side.clone().expect("from side");
        if from_side.no < 0 || from_side.no >= edge_count {
            log::warn!("ShapeTraceEntries.resort: from side not calculated");
            return;
        }
        // resort the intersection points, so that they start in the middle of from side.
        let compare_corner_1 = self.shape.corner_approx(from_side.no);
        let compare_corner_2 = if from_side.no == edge_count - 1 { self.shape.corner_approx(0) } else { self.shape.corner_approx(from_side.no + 1) };
        let mut from_point_dist = 0.0;
        let mut from_point_projection: Option<FloatPoint> = None;
        let mut from_side = from_side;
        if let Some(bi) = from_side.border_intersection {
            let projection = bi.projection_approx(&self.shape.border_line(from_side.no));
            from_point_dist = projection.distance_square(&compare_corner_1);
            from_point_projection = Some(projection);
            if from_point_dist >= compare_corner_1.distance_square(&compare_corner_2) {
                from_side = ShapeEntrySide::new(from_side.no, None);
            }
        }
        self.from_side = Some(from_side.clone());
        // search the first intersection point between the side middle and compare_corner_2
        let mut current = self.list_anchor;
        while let Some(c) = current {
            let e = &self.entries[c];
            if e.edge_index > from_side.no {
                break;
            }
            if e.edge_index == from_side.no {
                if from_side.border_intersection.is_some() {
                    let current_projection = e.entry_approx.projection_approx(&self.shape.border_line(from_side.no));
                    let fpp = from_point_projection.as_ref().unwrap();
                    if current_projection.distance_square(&compare_corner_1) >= from_point_dist
                        && current_projection.distance_square(fpp) <= current_projection.distance_square(&compare_corner_1)
                    {
                        break;
                    }
                } else if e.entry_approx.distance_square(&compare_corner_2) <= e.entry_approx.distance_square(&compare_corner_1) {
                    break;
                }
            }
            current = e.next;
        }
        if let Some(c) = current {
            if Some(c) != self.list_anchor {
                self.rotate_entry_list_around_anchor(c, edge_count);
            }
        }
        // remove intersections between two other intersections of the same connected set, so
        // that only first and last intersection is kept.
        let Some(anchor) = self.list_anchor else {
            return;
        };
        let mut prev = anchor;
        let mut prev_net_nos: Vec<NetNo> = self.nets(board, prev).to_vec();
        let mut current = self.entries[anchor].next;
        let mut current_net_nos: Vec<NetNo>;
        let mut next;
        if let Some(c) = current {
            current_net_nos = self.nets(board, c).to_vec();
            next = self.entries[c].next;
        } else {
            next = None;
            current_net_nos = Vec::new();
        }
        let mut before_prev: Option<usize> = None;
        while let Some(n) = next {
            let next_net_nos: Vec<NetNo> = self.nets(board, n).to_vec();
            if net_nos_equal(&prev_net_nos, &current_net_nos) && net_nos_equal(&current_net_nos, &next_net_nos) {
                self.entries[prev].next = Some(n);
            } else {
                before_prev = Some(prev);
                prev = current.unwrap();
                prev_net_nos = current_net_nos.clone();
            }
            current_net_nos = next_net_nos;
            current = Some(n);
            next = self.entries[n].next;
        }
        // remove nodes of own net at start and end of the list
        if current.is_some() && net_nos_equal(&current_net_nos, &self.own_net_nos) {
            self.entries[prev].next = None;
            if net_nos_equal(&prev_net_nos, &self.own_net_nos) {
                match before_prev {
                    Some(bp) => self.entries[bp].next = None,
                    None => self.list_anchor = None,
                }
            }
        }
        if let Some(a) = self.list_anchor {
            if board.item(self.entries[a].trace).nets_equal_nos(&self.own_net_nos) {
                self.list_anchor = self.entries[a].next;
                if let Some(a2) = self.list_anchor {
                    if board.item(self.entries[a2].trace).nets_equal_nos(&self.own_net_nos) {
                        self.list_anchor = self.entries[a2].next;
                    }
                }
            }
        }
    }

    /// Java `calculateStackLevels()`.
    fn calculate_stack_levels(&mut self, board: &BasicBoard) -> bool {
        let Some(anchor) = self.list_anchor else {
            return true;
        };
        let mut current_entry = Some(anchor);
        let mut current_net_nos: Vec<NetNo> = self.nets(board, anchor).to_vec();
        // ignore own net when calculating the stack level
        let mut current_level = if net_nos_equal(&current_net_nos, &self.own_net_nos) { 0 } else { 1 };
        while let Some(ce) = current_entry {
            if self.entries[ce].stack_level < 0 {
                // not yet calculated
                self.trace_piece_count += 1;
                self.entries[ce].stack_level = current_level;
                if current_level > self.max_stack_level {
                    if self.max_stack_level > 1 {
                        self.found_obstacle = Some(self.entries[ce].trace);
                    }
                    self.max_stack_level = current_level;
                }
            }
            // set stack level for all entries of the current net
            let mut check_entry = self.entries[ce].next;
            let mut index_of_next_foreign_set = 0;
            let mut index_of_last_occurrence_of_set = 0;
            let mut next_index = 0;
            let mut last_own_entry: Option<usize> = None;
            let mut first_foreign_entry: Option<usize> = None;
            while let Some(ch) = check_entry {
                next_index += 1;
                if net_nos_equal(self.nets(board, ch), &current_net_nos) {
                    index_of_last_occurrence_of_set = next_index;
                    last_own_entry = Some(ch);
                    self.entries[ch].stack_level = self.entries[ce].stack_level;
                } else if index_of_next_foreign_set == 0 {
                    // first occurrence of a foreign connected set
                    index_of_next_foreign_set = next_index;
                    first_foreign_entry = Some(ch);
                }
                check_entry = self.entries[ch].next;
            }
            if next_index != 0 {
                let next_entry;
                if index_of_next_foreign_set != 0 && index_of_next_foreign_set < index_of_last_occurrence_of_set {
                    // raise level
                    next_entry = first_foreign_entry.unwrap();
                    if self.entries[next_entry].stack_level >= 0 {
                        // already calculated: stack property fails
                        return false;
                    }
                    current_level += 1;
                } else if index_of_last_occurrence_of_set != 0 {
                    next_entry = last_own_entry.unwrap();
                } else {
                    next_entry = first_foreign_entry.unwrap();
                    if self.entries[next_entry].stack_level >= 0 {
                        // already calculated
                        current_level -= 1;
                        if self.entries[next_entry].stack_level != current_level {
                            return false;
                        }
                    }
                }
                current_net_nos = self.nets(board, next_entry).to_vec();
                // remove all entries between current entry and next entry
                self.entries[ce].next = Some(next_entry);
                current_entry = Some(next_entry);
            } else {
                current_entry = None;
            }
        }
        if current_level != 1 {
            log::warn!("ShapeTraceEntries.calculate_stack_levels: currentLevel inconsistent");
            return false;
        }
        true
    }

    /// Java `popPiece()`: the first and last entry of the next piece with the maximal stack
    /// level (removed from the list).
    fn pop_piece(&mut self, board: &BasicBoard) -> Option<(usize, usize)> {
        let Some(anchor) = self.list_anchor else {
            if self.trace_piece_count != 0 {
                log::warn!("ShapeTraceEntries: tracePieceCount is inconsistent");
            }
            return None;
        };
        let mut first = Some(anchor);
        let mut prev_first: Option<usize> = None;
        while let Some(f) = first {
            if self.entries[f].stack_level == self.max_stack_level {
                break;
            }
            prev_first = Some(f);
            first = self.entries[f].next;
        }
        let Some(first) = first else {
            log::warn!("ShapeTraceEntries: maxStackLevel not found");
            return None;
        };
        let mut last = first;
        let mut after_last = self.entries[first].next;
        while let Some(al) = after_last {
            if self.entries[al].stack_level == self.max_stack_level && board.item(self.entries[al].trace).nets_equal(board.item(self.entries[first].trace)) {
                last = al;
                after_last = self.entries[al].next;
            } else {
                break;
            }
        }
        // remove the nodes from first to last inclusive
        match prev_first {
            Some(pf) => self.entries[pf].next = after_last,
            None => self.list_anchor = after_last,
        }
        // recalculate max_stack_level
        self.max_stack_level = 0;
        let mut current = self.list_anchor;
        while let Some(c) = current {
            if self.entries[c].stack_level > self.max_stack_level {
                self.max_stack_level = self.entries[c].stack_level;
            }
            current = self.entries[c].next;
        }
        self.trace_piece_count -= 1;
        if board.item(self.entries[first].trace).nets_equal_nos(&self.own_net_nos) {
            // own net is ignored and may occur only at the lowest level
            return self.pop_piece(board);
        }
        Some((first, last))
    }

    /// Java `insertEntryPoint(trace, traceLineNo, edgeIndex, entryApprox)`: inserts into the
    /// list sorted around the border of the shape.
    fn insert_entry_point(&mut self, trace: ItemKey, trace_line_no: i32, edge_index: i32, entry_approx: FloatPoint) {
        let new_index = self.entries.len();
        self.entries.push(EntryPoint { trace, trace_line_no, entry_approx, edge_index, stack_level: -1, next: None });
        let mut current_prev: Option<usize> = None;
        let mut current_next = self.list_anchor;
        while let Some(cn) = current_next {
            let ce = &self.entries[cn];
            if ce.edge_index > edge_index {
                break;
            }
            if ce.edge_index == edge_index {
                let prev_corner = self.shape.corner_approx(edge_index);
                let next_corner = if edge_index == self.shape.border_line_count() - 1 {
                    self.shape.corner_approx(0)
                } else {
                    self.shape.corner_approx(edge_index + 1)
                };
                if prev_corner.scalar_product(&entry_approx, &next_corner) <= prev_corner.scalar_product(&ce.entry_approx, &next_corner) {
                    break;
                }
            }
            current_prev = Some(cn);
            current_next = ce.next;
        }
        self.entries[new_index].next = current_next;
        match current_prev {
            Some(p) => self.entries[p].next = Some(new_index),
            None => self.list_anchor = Some(new_index),
        }
    }

    /// Java `rotateEntryListAroundAnchor(newAnchor, edgeCount)`.
    fn rotate_entry_list_around_anchor(&mut self, new_anchor: usize, edge_count: i32) {
        let mut current = Some(new_anchor);
        let mut prev = new_anchor;
        while let Some(c) = current {
            prev = c;
            current = self.entries[c].next;
        }
        self.entries[prev].next = self.list_anchor;
        let mut current = self.list_anchor;
        while let Some(c) = current {
            if c == new_anchor {
                break;
            }
            // add edge_count to the side to differentiate points before and after the middle
            // of the from side
            self.entries[c].edge_index += edge_count;
            prev = c;
            current = self.entries[c].next;
        }
        self.entries[prev].next = None;
        self.list_anchor = Some(new_anchor);
    }
}
