//! The connectivity parts of `Item`, `DrillItem`, `Trace`, `ConductionArea` and
//! `BoardConnectivityQueries`: normal contacts, contact points, connected sets, connection
//! items, tails and cycles.
//!
//! The Java recursions (`getConnectedSetRecu`, `isCycleRecu`) are iterative here with an
//! explicit stack that visits the contacts in the same order (depth first, contacts in
//! `TreeSet` order), so deep nets cannot overflow the stack.

use fr_geom::{ConvexShape, FloatPoint, Point, TileShape};

use crate::ids::{FixedState, ItemId, LayerNo, NetNo};

use super::basic_board::BasicBoard;
use super::item::{Item, ItemKey, ItemKind, StopConnectionOption};
use super::item_list::ItemSet;
use super::search_tree::TreeObject;

/// Java `Item.PROTECT_FANOUT_LENGTH`.
const PROTECT_FANOUT_LENGTH: f64 = 400.0;

fn point_shape(point: &Point) -> ConvexShape {
    ConvexShape::Tile(TileShape::IntBox(TileShape::get_instance_point(point)))
}

impl BasicBoard {
    /// Java `Trace.getNormalContacts(point, ignoreNet)`: the items with a connection point at
    /// `point` on the trace layer.
    pub fn trace_normal_contacts_at(&self, key: ItemKey, point: &Point, ignore_net: bool) -> ItemSet {
        let this = self.item(key);
        let mut result = ItemSet::new();
        if !(*point == this.first_corner() || *point == this.last_corner()) {
            return result;
        }
        let layer = this.trace().layer;
        for o in self.overlapping_objects(&point_shape(point), layer) {
            let TreeObject::Item { key: other_key, id } = o else { continue };
            if other_key == key {
                continue;
            }
            let other = self.item(other_key);
            if !(other.shares_layer(this, self) && (ignore_net || other.shares_net(this))) {
                continue;
            }
            let contact = match &other.kind {
                ItemKind::Trace(_) => *point == other.first_corner() || *point == other.last_corner(),
                ItemKind::Pin(_) | ItemKind::Via(_) => *point == other.center(self),
                ItemKind::ConductionArea(c) => c.area.get_area(self).contains(point),
                _ => false,
            };
            if contact {
                result.insert(ItemId(id), other_key);
            }
        }
        result
    }

    /// Java `Trace.getStartContacts()`.
    pub fn trace_start_contacts(&self, key: ItemKey) -> ItemSet {
        self.trace_normal_contacts_at(key, &self.item(key).first_corner(), false)
    }

    /// Java `Trace.getEndContacts()`.
    pub fn trace_end_contacts(&self, key: ItemKey) -> ItemSet {
        self.trace_normal_contacts_at(key, &self.item(key).last_corner(), false)
    }

    /// Java `getNormalContacts()` (dispatching on the item class).
    pub fn normal_contacts(&self, key: ItemKey) -> ItemSet {
        let this = self.item(key);
        match &this.kind {
            ItemKind::Trace(_) => {
                let mut result = ItemSet::new();
                result.extend_from(&self.trace_normal_contacts_at(key, &this.first_corner(), false));
                result.extend_from(&self.trace_normal_contacts_at(key, &this.last_corner(), false));
                result
            }
            ItemKind::Pin(_) | ItemKind::Via(_) => self.drill_normal_contacts(key),
            ItemKind::ConductionArea(_) => self.conduction_area_normal_contacts(key),
            _ => ItemSet::new(),
        }
    }

    fn drill_normal_contacts(&self, key: ItemKey) -> ItemSet {
        let this = self.item(key);
        let drill_center = this.center(self);
        let mut result = ItemSet::new();
        for o in self.overlapping_objects(&point_shape(&drill_center), -1) {
            let TreeObject::Item { key: other_key, id } = o else { continue };
            if other_key == key {
                continue;
            }
            let other = self.item(other_key);
            if !(other.shares_net(this) && other.shares_layer(this, self)) {
                continue;
            }
            let contact = match &other.kind {
                // exact matching of trace endpoints to the drill center
                ItemKind::Trace(_) => drill_center == other.first_corner() || drill_center == other.last_corner(),
                ItemKind::Pin(_) | ItemKind::Via(_) => drill_center == other.center(self),
                ItemKind::ConductionArea(c) => c.area.get_area(self).contains(&drill_center),
                _ => false,
            };
            if contact {
                result.insert(ItemId(id), other_key);
            }
        }
        result
    }

    fn conduction_area_normal_contacts(&self, key: ItemKey) -> ItemSet {
        let this = self.item(key);
        let layer = this.first_layer(self);
        let mut result = ItemSet::new();
        for i in 0..self.tile_shape_count(key) {
            let Some(current_shape) = self.tile_shape(key, i) else { continue };
            for o in self.overlapping_objects(&ConvexShape::Tile(current_shape.clone()), layer) {
                let TreeObject::Item { key: other_key, id } = o else { continue };
                if other_key == key {
                    continue;
                }
                let other = self.item(other_key);
                if !(other.shares_net(this) && other.shares_layer(this, self)) {
                    continue;
                }
                let contact = match &other.kind {
                    ItemKind::Trace(_) => current_shape.contains(&other.first_corner()) || current_shape.contains(&other.last_corner()),
                    ItemKind::Pin(_) | ItemKind::Via(_) => current_shape.contains(&other.center(self)),
                    _ => false,
                };
                if contact {
                    result.insert(ItemId(id), other_key);
                }
            }
        }
        result
    }

    /// Java `getAllContacts()` (`layer == None`) and `getAllContacts(layer)`.
    pub fn all_contacts(&self, key: ItemKey, layer: Option<LayerNo>) -> ItemSet {
        let this = self.item(key);
        let mut result = ItemSet::new();
        if !this.is_connectable_class() {
            return result;
        }
        for i in 0..self.tile_shape_count(key) {
            let shape_layer = self.shape_layer(key, i);
            if let Some(l) = layer {
                if shape_layer != l {
                    continue;
                }
            }
            let Some(shape) = self.tile_shape(key, i) else { continue };
            for o in self.overlapping_objects(&ConvexShape::Tile(shape), shape_layer) {
                let TreeObject::Item { key: other_key, id } = o else { continue };
                let other = self.item(other_key);
                if other_key != key && other.is_connectable_class() && other.shares_net(this) {
                    result.insert(ItemId(id), other_key);
                }
            }
        }
        result
    }

    /// Java `isConnected()`.
    pub fn is_connected(&self, key: ItemKey) -> bool {
        !self.all_contacts(key, None).is_empty()
    }

    /// Java `isConnectedOnLayer(layer)`.
    pub fn is_connected_on_layer(&self, key: ItemKey, layer: LayerNo) -> bool {
        !self.all_contacts(key, Some(layer)).is_empty()
    }

    /// Java `this.normalContactPoint(other)` (double dispatch between traces and drill items).
    pub fn normal_contact_point(&self, a: ItemKey, b: ItemKey) -> Option<Point> {
        let ia = self.item(a);
        let ib = self.item(b);
        match (&ia.kind, &ib.kind) {
            (ItemKind::Pin(_) | ItemKind::Via(_), ItemKind::Trace(_)) => self.drill_trace_contact_point(ia, ib),
            (ItemKind::Trace(_), ItemKind::Pin(_) | ItemKind::Via(_)) => self.drill_trace_contact_point(ib, ia),
            (ItemKind::Pin(_) | ItemKind::Via(_), ItemKind::Pin(_) | ItemKind::Via(_)) => {
                // other.normalContactPoint((DrillItem) this) with this = b
                let cb = ib.center(self);
                if ib.shares_layer(ia, self) && cb == ia.center(self) {
                    Some(cb)
                } else {
                    None
                }
            }
            (ItemKind::Trace(_), ItemKind::Trace(_)) => trace_trace_contact_point(ib, ia),
            _ => None,
        }
    }

    /// Java `DrillItem.normalContactPoint(Trace)` with `this = drill`.
    fn drill_trace_contact_point(&self, drill: &Item, trace: &Item) -> Option<Point> {
        if !drill.shares_layer(trace, self) {
            return None;
        }
        let center = drill.center(self);
        if center == trace.first_corner() || center == trace.last_corner() {
            Some(center)
        } else {
            None
        }
    }

    /// Java `getConnectedSet(netNumber, stopAtPlane)`.
    pub fn connected_set(&self, key: ItemKey, net_number: NetNo, stop_at_plane: bool) -> ItemSet {
        let this = self.item(key);
        let mut result = ItemSet::new();
        if net_number > 0 && !this.contains_net(net_number) {
            return result;
        }
        result.insert(this.id(), key);
        // iterative version of getConnectedSetRecu
        let mut stack: Vec<(Vec<ItemKey>, usize)> = vec![(self.normal_contacts(key).iter().collect(), 0)];
        while let Some((contacts, pos)) = stack.last_mut() {
            if *pos >= contacts.len() {
                stack.pop();
                continue;
            }
            let contact = contacts[*pos];
            *pos += 1;
            let c = self.item(contact);
            if stop_at_plane && c.is_conduction_area() && c.component_no() <= 0 {
                continue;
            }
            if net_number > 0 && !c.contains_net(net_number) {
                continue;
            }
            if result.insert(c.id(), contact) {
                stack.push((self.normal_contacts(contact).iter().collect(), 0));
            }
        }
        result
    }

    /// Java `getUnconnectedSet(netNumber)`.
    pub fn unconnected_set(&self, key: ItemKey, net_number: NetNo) -> ItemSet {
        let this = self.item(key);
        let mut result = ItemSet::new();
        if net_number > 0 && !this.contains_net(net_number) {
            return result;
        }
        if net_number > 0 {
            for k in self.get_connectable_items(net_number) {
                result.insert(self.item(k).id(), k);
            }
        } else {
            for &n in &this.net_numbers {
                for k in self.get_connectable_items(n) {
                    result.insert(self.item(k).id(), k);
                }
            }
        }
        result.remove_all(&self.connected_set(key, net_number, false));
        result
    }

    /// Java `getConnectionItems(stopOption)`: all traces and vias from this item until the next
    /// fork or terminal item.
    pub fn get_connection_items(&self, key: ItemKey, stop_option: StopConnectionOption) -> ItemSet {
        let this = self.item(key);
        let contacts = self.normal_contacts(key);
        let mut result = ItemSet::new();
        if this.is_routable() {
            result.insert(this.id(), key);
        }
        for start_contact in contacts.iter() {
            let mut current = start_contact;
            let Some(mut prev_contact_point) = self.normal_contact_point(key, current) else {
                // no unique contact point
                continue;
            };
            let mut prev_contact_layer = this.first_common_layer(self.item(current), self);
            if this.is_trace() {
                // Check, that there is only 1 contact at this location.
                let check_contacts = self.trace_normal_contacts_at(key, &prev_contact_point, false);
                if check_contacts.len() != 1 {
                    continue;
                }
            }
            // Search from current along the contacts until the next fork or nonroute item.
            loop {
                let ci = self.item(current);
                if !ci.is_routable() {
                    break;
                }
                if ci.is_via() {
                    if stop_option == StopConnectionOption::Via {
                        break;
                    }
                    if stop_option == StopConnectionOption::FanoutVia && self.is_fanout_via(current, Some(&result)) {
                        break;
                    }
                }
                result.insert(ci.id(), current);
                let current_contacts = self.normal_contacts(current);
                let mut next_contact: Option<(ItemKey, Point, LayerNo)> = None;
                let mut fork_found = false;
                for tmp in current_contacts.iter() {
                    let tmp_layer = ci.first_common_layer(self.item(tmp), self);
                    if tmp_layer >= 0 {
                        let Some(tmp_point) = self.normal_contact_point(current, tmp) else {
                            // no unique contact point
                            fork_found = true;
                            break;
                        };
                        if prev_contact_layer != tmp_layer || prev_contact_point != tmp_point {
                            if next_contact.is_some() {
                                // second new contact found
                                fork_found = true;
                                break;
                            }
                            next_contact = Some((tmp, tmp_point, tmp_layer));
                        }
                    }
                }
                match next_contact {
                    Some((next, point, layer)) if !fork_found => {
                        current = next;
                        prev_contact_point = point;
                        prev_contact_layer = layer;
                    }
                    _ => break,
                }
            }
        }
        result
    }

    /// Java `isTail()`: a trace not contacted at one end, or a via with contacts on at most one
    /// layer (range).
    pub fn is_tail(&self, key: ItemKey) -> bool {
        let this = self.item(key);
        match &this.kind {
            ItemKind::Trace(_) => self.trace_start_contacts(key).is_empty() || self.trace_end_contacts(key).is_empty(),
            ItemKind::Via(_) => {
                let contacts = self.normal_contacts(key);
                if contacts.len() <= 1 {
                    return true;
                }
                let mut it = contacts.iter();
                let first = self.item(it.next().unwrap());
                let (ff, fl) = (first.first_layer(self), first.last_layer(self));
                for k in it {
                    let c = self.item(k);
                    if c.first_layer(self) != ff || c.last_layer(self) != fl {
                        return false;
                    }
                }
                true
            }
            _ => false,
        }
    }

    /// Java `Trace.isOverlap()`: the trace is connected to the same object at both ends.
    pub fn is_overlap(&self, key: ItemKey) -> bool {
        if !self.item(key).is_trace() {
            return false;
        }
        !self.trace_start_contacts(key).is_disjoint(&self.trace_end_contacts(key))
    }

    /// Java `Trace.isCycle()`: the trace can be reached by other items via more than one path.
    pub fn is_cycle(&self, key: ItemKey) -> bool {
        if self.is_overlap(key) {
            return true;
        }
        let this = self.item(key);
        let start_contacts = self.trace_start_contacts(key);
        // a cycle exists if through expanding the start contact we reach this trace again via
        // an end contact
        let mut visited = start_contacts.clone();
        let mut ignore_areas = false;
        if let Some(&n) = this.net_numbers.first() {
            if let Some(net) = self.rules.nets.get(n) {
                ignore_areas = self.rules.net_classes[net.get_net_class()].get_ignore_cycles_with_areas();
            }
        }
        for contact in start_contacts.iter() {
            if self.is_cycle_recu(contact, &mut visited, key, key, ignore_areas) {
                return true;
            }
        }
        false
    }

    /// Java `Item.isCycleRecu(visitedItems, searchItem, comeFromItem, ignoreAreas)` (iterative).
    fn is_cycle_recu(&self, start: ItemKey, visited: &mut ItemSet, search: ItemKey, come_from: ItemKey, ignore_areas: bool) -> bool {
        struct Frame {
            item: ItemKey,
            come_from: ItemKey,
            contacts: Vec<ItemKey>,
            pos: usize,
        }
        let make_frame = |item: ItemKey, come_from: ItemKey| -> Option<Frame> {
            if ignore_areas && self.item(item).is_conduction_area() {
                return None;
            }
            Some(Frame { item, come_from, contacts: self.normal_contacts(item).iter().collect(), pos: 0 })
        };
        let Some(first) = make_frame(start, come_from) else {
            return false;
        };
        let mut stack = vec![first];
        while let Some(frame) = stack.last_mut() {
            if frame.pos >= frame.contacts.len() {
                stack.pop();
                continue;
            }
            let contact = frame.contacts[frame.pos];
            frame.pos += 1;
            if contact == frame.come_from {
                continue;
            }
            if contact == search {
                return true;
            }
            let from = frame.item;
            if visited.insert(self.item(contact).id(), contact) {
                if let Some(f) = make_frame(contact, from) {
                    stack.push(f);
                }
            }
        }
        false
    }

    /// Java `isFanoutVia(ignoreItems)`.
    pub fn is_fanout_via(&self, key: ItemKey, ignore_items: Option<&ItemSet>) -> bool {
        let is_single_contact_smd_pin = |k: ItemKey| -> bool {
            let c = self.item(k);
            c.is_pin() && c.first_layer(self) == c.last_layer(self) && self.normal_contacts(k).len() <= 1
        };
        for contact in self.normal_contacts(key).iter() {
            if is_single_contact_smd_pin(contact) {
                return true;
            }
            let c = self.item(contact);
            if let Some(t) = c.as_trace() {
                if let Some(ignore) = ignore_items {
                    if ignore.contains(c.id()) {
                        continue;
                    }
                }
                if t.length() >= PROTECT_FANOUT_LENGTH * t.half_width as f64 {
                    continue;
                }
                for tmp in self.normal_contacts(contact).iter() {
                    if is_single_contact_smd_pin(tmp) {
                        return true;
                    }
                    let ti = self.item(tmp);
                    if let Some(tt) = ti.as_trace() {
                        if ti.fixed_state() == FixedState::ShoveFixed && tt.corner_count() == 2 {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// Java `getRatsnestCorners()`.
    pub fn ratsnest_corners(&self, key: ItemKey) -> Vec<Point> {
        let this = self.item(key);
        match &this.kind {
            ItemKind::Pin(_) | ItemKind::Via(_) => vec![this.center(self)],
            ItemKind::Trace(_) => {
                let mut result = Vec::new();
                if self.trace_start_contacts(key).is_empty() {
                    result.push(this.first_corner());
                }
                if self.trace_end_contacts(key).is_empty() {
                    result.push(this.last_corner());
                }
                result
            }
            ItemKind::ConductionArea(c) => {
                let corners: Vec<FloatPoint> = c.area.get_area(self).corner_approx_arr();
                corners.iter().map(|p| Point::Int(p.round())).collect()
            }
            _ => Vec::new(),
        }
    }

    /// Java `Connectable.getTraceConnectionShape(tree, index)`.
    pub fn trace_connection_shape(&self, t: usize, key: ItemKey, index: i32) -> Option<TileShape> {
        let this = self.item(key);
        match &this.kind {
            ItemKind::Pin(_) | ItemKind::Via(_) => Some(TileShape::IntBox(TileShape::get_instance_point(&this.center(self)))),
            ItemKind::Trace(tr) => {
                if index < 0 || index >= tr.tile_shape_count() {
                    log::warn!("PolylineTrace.get_trace_connection_shape index out of range");
                    return None;
                }
                let segment = fr_geom::LineSegment::from_polyline(&tr.polyline, index + 1)?;
                Some(TileShape::Simplex(segment.to_simplex()).simplify())
            }
            ItemKind::ConductionArea(_) => {
                if index < 0 || index >= self.tree_shape_count(t, key) {
                    log::warn!("ConductionArea.get_trace_connection_shape index out of range");
                    return None;
                }
                self.tree_shape(t, key, index)
            }
            _ => None,
        }
    }

    /// Java `getConnectedSets(netNumber)`.
    pub fn get_connected_sets(&self, net_number: NetNo) -> Vec<ItemSet> {
        let mut result = Vec::new();
        if net_number <= 0 {
            return result;
        }
        let mut items_to_handle = ItemSet::new();
        for k in self.get_connectable_items(net_number) {
            items_to_handle.insert(self.item(k).id(), k);
        }
        while let Some(current) = items_to_handle.first() {
            let next_set = self.connected_set(current, net_number, false);
            items_to_handle.remove_all(&next_set);
            result.push(next_set);
        }
        result
    }
}

/// Java `Trace.normalContactPoint(Trace other)` with `this`.
fn trace_trace_contact_point(this: &Item, other: &Item) -> Option<Point> {
    if this.trace().layer != other.trace().layer {
        return None;
    }
    let tf = this.first_corner();
    let tl = this.last_corner();
    let of = other.first_corner();
    let ol = other.last_corner();
    let at_first = tf == of || tf == ol;
    let at_last = tl == of || tl == ol;
    if !(at_first || at_last) || (at_first && at_last) {
        None
    } else if at_first {
        Some(tf)
    } else {
        Some(tl)
    }
}
