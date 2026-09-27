//! Component insertion (Java `Network.insertComponents` / `insertComponent`) and the pure
//! geometry of component items (Java `Pin.getShape/getCenter/relativeLocation`,
//! `DrillItem.firstLayer`, `ObstacleArea.getArea`, `ComponentOutline.getArea`), which the
//! board step needs and the parity tests use.

use fr_engine::ids::{FixedState, LayerNo};
use fr_engine::library::{Package, Padstack};
use fr_engine::rules::ItemClass;
use fr_engine::structure::Component;
use fr_geom::{Area, FloatPoint, IntPoint, Point, Shape, Vector};

use crate::error::{npe, LResult};
use crate::loader::{KeepoutKind, Loader};
use crate::requests::{AreaRequest, InsertRequest};

/// Java `Math.toRadians`.
pub fn to_radians(deg: f64) -> f64 {
    deg * 0.017453292519943295
}

fn zero() -> IntPoint {
    IntPoint::new(0, 0)
}

/// Java `Pin.relativeLocation()`.
pub fn pin_relative_location(
    component: &Component,
    package: &Package,
    pin_index: i32,
    flip_style_rotate_first: bool,
) -> Vector {
    let pin = package.get_pin(pin_index).expect("pin");
    let mut rel = pin.relative_location.clone();
    let rotation = component.get_rotation_in_degree();
    if !component.placed_on_front() && !flip_style_rotate_first {
        rel = pin.relative_location.mirror_at_y_axis();
    }
    if rotation % 90.0 == 0.0 {
        let factor = rotation as i32 / 90;
        if factor != 0 {
            rel = rel.turn_90_degree(factor);
        }
    } else {
        let approx = rel
            .to_float()
            .rotate(to_radians(rotation), &FloatPoint::ZERO);
        rel = approx.round().difference_by(&Point::ZERO);
    }
    if !component.placed_on_front() && flip_style_rotate_first {
        rel = rel.mirror_at_y_axis();
    }
    rel
}

/// Java `DrillItem.firstLayer()/lastLayer()` of a pin.
pub fn pin_layer_range(component: &Component, padstack: &Padstack) -> (LayerNo, LayerNo) {
    if component.placed_on_front() || padstack.placed_absolute {
        (padstack.from_layer(), padstack.to_layer())
    } else {
        let n = padstack.board_layer_count();
        (n - padstack.to_layer() - 1, n - padstack.from_layer() - 1)
    }
}

/// Java `Pin.getShape(i)` for all `i` (index 0 is the first layer of the pin).
pub fn pin_shapes(
    component: &Component,
    package: &Package,
    padstack: &Padstack,
    pin_index: i32,
    flip_style_rotate_first: bool,
) -> Vec<Option<Shape>> {
    let n = (padstack.to_layer() - padstack.from_layer() + 1).max(0) as usize;
    let mut result = vec![None; n];
    let pin = package.get_pin(pin_index).expect("pin");
    let rotation = component.get_rotation_in_degree();
    let mirror = !component.placed_on_front() && !flip_style_rotate_first;
    let rel = if mirror {
        pin.relative_location.mirror_at_y_axis()
    } else {
        pin.relative_location.clone()
    };
    let location = component.get_location().expect("placed component");
    let translation = location.difference_by(&Point::ZERO);
    let (first_layer, _) = pin_layer_range(component, padstack);
    for (i, slot) in result.iter_mut().enumerate() {
        let padstack_layer = if component.placed_on_front() || padstack.placed_absolute {
            i as LayerNo + first_layer
        } else {
            padstack.board_layer_count() - i as LayerNo - first_layer - 1
        };
        let Some(shape) = padstack.get_shape(padstack_layer) else {
            continue;
        };
        let mut s = shape.to_shape();
        let pin_rotation = pin.rotation_in_degree;
        if pin_rotation % 90.0 == 0.0 {
            let f = pin_rotation as i32 / 90;
            if f != 0 {
                s = s.turn_90_degree(f, &zero());
            }
        } else {
            s = s.rotate_approx(to_radians(pin_rotation), &FloatPoint::ZERO);
        }
        if mirror {
            s = s.mirror_vertical(&zero());
        }
        let mut t = s.translate_by(&rel);
        if rotation % 90.0 == 0.0 {
            let f = rotation as i32 / 90;
            if f != 0 {
                t = t.turn_90_degree(f, &zero());
            }
        } else {
            t = t.rotate_approx(to_radians(rotation), &FloatPoint::ZERO);
        }
        if !component.placed_on_front() && flip_style_rotate_first {
            t = t.mirror_vertical(&zero());
        }
        *slot = Some(t.translate_by(&translation));
    }
    result
}

/// Java `Pin.getCenter()`: the nominal center, moved to the centre of gravity of the first
/// pin shape if it is not inside that shape.
pub fn pin_center(
    component: &Component,
    package: &Package,
    padstack: &Padstack,
    pin_index: i32,
    flip_style_rotate_first: bool,
) -> Point {
    let rel = pin_relative_location(component, package, pin_index, flip_style_rotate_first);
    let center = component.get_location().expect("placed").translate_by(&rel);
    let shapes = pin_shapes(
        component,
        package,
        padstack,
        pin_index,
        flip_style_rotate_first,
    );
    match shapes.iter().flatten().next() {
        Some(s) if !s.contains_inside(&center) => Point::Int(s.centre_of_gravity().round()),
        _ => center,
    }
}

/// Java `ObstacleArea.getArea()` (also `ComponentOutline.getArea()` with
/// `side_changed = !is_front`).
pub fn transformed_area(
    relative: &Area,
    translation: &Vector,
    rotation_in_degree: f64,
    side_changed: bool,
    flip_style_rotate_first: bool,
) -> Area {
    let mut a = relative.clone();
    if side_changed && !flip_style_rotate_first {
        a = a.mirror_vertical(&zero());
    }
    if rotation_in_degree != 0.0 {
        if rotation_in_degree % 90.0 == 0.0 {
            a = a.turn_90_degree(rotation_in_degree as i32 / 90, &zero());
        } else {
            a = a.rotate_approx(to_radians(rotation_in_degree), &FloatPoint::ZERO);
        }
    }
    if side_changed && flip_style_rotate_first {
        a = a.mirror_vertical(&zero());
    }
    a.translate_by(translation)
}

impl Loader<'_> {
    /// Java `Network.insertComponents` (placements read before the network scope).
    pub(crate) fn insert_components(&mut self) -> LResult<()> {
        let placements = self.placements.clone();
        for p in placements {
            for place in &p.places {
                self.insert_component(place, &p.image)?;
            }
        }
        Ok(())
    }

    /// Java `Network.insertComponent`.
    fn insert_component(&mut self, place: &fr_dsn::model::Place, lib_key: &str) -> LResult<()> {
        let board = self.board()?;
        if !board.library_read {
            return Err(npe("Network.insertComponent: library packages are null"));
        }
        let front = board
            .library
            .packages
            .get_by_name(lib_key, true)
            .map(|p| p.id);
        let back = board
            .library
            .packages
            .get_by_name(lib_key, false)
            .map(|p| p.id);
        let (Some(front), Some(back)) = (front, back) else {
            log::warn!("Network.insert_component: component package not found at '{lib_key}'");
            return Ok(());
        };
        let ct = board.transform;
        let location = place
            .location
            .map(|c| Point::Int(ct.dsn_to_board_point(&c).round()));
        let rotation = place.rotation;
        let component_id = board.components.add(
            place.name.clone(),
            location.clone(),
            rotation,
            place.is_front,
            front,
            back,
            place.position_fixed,
            place.part_number.clone(),
        );
        let Some(location) = location else {
            return Ok(()); // not yet placed
        };
        let translation = location.difference_by(&Point::ZERO);
        let fixed = if place.position_fixed {
            FixedState::SystemFixed
        } else {
            FixedState::Unfixed
        };
        let package_id = board.components.get(component_id).get_package();
        let package = board.library.packages.get(package_id).clone();

        // Pins.
        for (i, pin) in package.pins().iter().enumerate() {
            let board = self.board.as_mut().expect("board");
            let Some(padstack) = board.library.padstacks.get(pin.padstack_id) else {
                log::warn!("Network.insert_component: pin padstack not found");
                return Ok(());
            };
            let smd = padstack.from_layer() == padstack.to_layer();
            let mut nets = Vec::new();
            for id in self.netlist.get_nets(&place.name, &pin.name) {
                match board.rules.nets.get_by_name(&id.name, id.subnet) {
                    Some(n) => nets.push(n.net_number),
                    None => log::warn!("Network.insert_component: board net not found"),
                }
            }
            let net_class = match nets.first().and_then(|n| board.rules.nets.get(*n)) {
                Some(n) => n.get_net_class(),
                None => board.rules.get_default_net_class(),
            };
            let mut cl = -1;
            if let Some(info) = place
                .pin_clearances
                .iter()
                .rev()
                .find(|c| c.name == pin.name)
            {
                cl = board.rules.clearance_matrix.get_no(&info.clearance_class);
            }
            if cl < 0 {
                let d = &board.rules.net_classes[net_class].default_item_clearance_classes;
                cl = if smd {
                    d.get(ItemClass::Smd)
                } else {
                    d.get(ItemClass::Pin)
                };
            }
            board.push(InsertRequest::Pin {
                component_id,
                pin_index: i as i32,
                nets,
                clearance_class: cl,
                fixed,
            });
        }

        // Keepouts (k = 0 keepouts, 1 via keepouts, 2 place keepouts).
        let board = self.board.as_mut().expect("board");
        let layer_count = board.layer_count();
        for (k, (keepouts, infos)) in [
            (&package.keepouts, &place.keepout_clearances),
            (&package.via_keepouts, &place.via_keepout_clearances),
            (&package.place_keepout_arr, &place.place_keepout_clearances),
        ]
        .into_iter()
        .enumerate()
        {
            let kind = [
                KeepoutKind::Keepout,
                KeepoutKind::ViaKeepout,
                KeepoutKind::PlaceKeepout,
            ][k];
            for (i, ko) in keepouts.iter().enumerate() {
                let mut layer = ko.layer;
                if layer >= layer_count {
                    log::warn!("Network.insert_component: keepout layer is to big");
                    continue;
                }
                if layer >= 0 && !place.is_front {
                    layer = layer_count - ko.layer - 1;
                }
                let d = board.rules.get_default_net_class();
                let mut cl = board.rules.net_classes[d]
                    .default_item_clearance_classes
                    .get(ItemClass::Area);
                if let Some(info) = infos.iter().rev().find(|c| c.name == ko.name) {
                    let c = board.rules.clearance_matrix.get_no(&info.clearance_class);
                    if c > 0 {
                        cl = c;
                    }
                }
                if board.null_package_keepouts.contains(&(package_id, kind, i)) {
                    // insertObstacle(null, ...) returns before constructing an item.
                    continue;
                }
                let layers: Vec<LayerNo> = if layer >= 0 {
                    vec![layer]
                } else {
                    board
                        .layer_structure
                        .layers
                        .iter()
                        .enumerate()
                        .filter(|(_, l)| l.is_signal)
                        .map(|(j, _)| j as LayerNo)
                        .collect()
                };
                for layer in layers {
                    let req = AreaRequest {
                        area: ko.area.clone(),
                        layer,
                        translation: translation.clone(),
                        rotation_in_degree: rotation,
                        side_changed: !place.is_front,
                        clearance_class: cl,
                        component_id,
                        name: Some(ko.name.clone()),
                        fixed,
                    };
                    board.push(match kind {
                        KeepoutKind::Keepout => InsertRequest::Obstacle(req),
                        KeepoutKind::ViaKeepout => InsertRequest::ViaObstacle(req),
                        KeepoutKind::PlaceKeepout => InsertRequest::ComponentObstacle(req),
                    });
                }
            }
        }

        // Outlines as component outline items.
        if let Some(outline) = &package.outline {
            let mut courtyard_idx: i64 = -1;
            if outline.len() > 1 {
                let mut max_area = -1.0;
                for (i, o) in outline.iter().enumerate() {
                    if let Some(o) = o {
                        let area = o.bounding_box().area();
                        if area > max_area {
                            max_area = area;
                            courtyard_idx = i as i64;
                        }
                    }
                }
            }
            for (i, o) in outline.iter().enumerate() {
                let mut is_courtyard = i as i64 == courtyard_idx;
                let widths = package.outline_widths.as_deref();
                if let Some(w) = widths.and_then(|w| w.get(i)) {
                    if *w == 0.0 {
                        is_courtyard = true;
                    }
                }
                let mut is_fabrication = false;
                if !is_courtyard {
                    if let Some(w) = widths.and_then(|w| w.get(i)) {
                        if *w <= 110.0 {
                            is_fabrication = true;
                        }
                    }
                }
                let is_closed = package
                    .outline_is_closed
                    .as_deref()
                    .and_then(|c| c.get(i).copied())
                    .unwrap_or(false);
                let Some(shape) = o else {
                    log::warn!("BasicBoard.insert_component_outline: area is null");
                    continue;
                };
                board.push(InsertRequest::ComponentOutline {
                    area: Area::Shape(shape.clone()),
                    is_front: place.is_front,
                    translation: translation.clone(),
                    rotation_in_degree: rotation,
                    component_id,
                    is_courtyard,
                    is_fabrication,
                    is_closed,
                    fixed,
                });
            }
        }
        Ok(())
    }
}
