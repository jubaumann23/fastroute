//! Port of `board/model/structure/Component.java` and `Components.java`.
//!
//! Package and logical part references are ids into the board library. The Java undo list of
//! `Components` (`generateSnapshot`/`undo`/`redo`) is not ported: the board restores snapshots
//! by cloning.

use fr_geom::{IntPoint, Point, Vector};
use fr_jcompat::compare_to_ignore_case;

use crate::ids::ComponentNo;
use crate::library::{LogicalPartNo, PackageNo};

/// Java `Math.toRadians` (JDK 9+: `angdeg * DEGREES_TO_RADIANS`).
#[inline]
fn to_radians(deg: f64) -> f64 {
    const DEGREES_TO_RADIANS: f64 = 0.017453292519943295;
    deg * DEGREES_TO_RADIANS
}

fn normalize_rotation(mut r: f64) -> f64 {
    while r >= 360.0 {
        r -= 360.0;
    }
    while r < 0.0 {
        r += 360.0;
    }
    r
}

/// A board component: placement of a library package (Java `Component`).
#[derive(Clone, Debug, PartialEq)]
pub struct Component {
    /// The name of the component.
    pub name: String,
    /// Internal unique id, 1-based.
    pub id: ComponentNo,
    /// If true, the component cannot be moved.
    pub position_fixed: bool,
    /// Library package used when placed on the front side.
    lib_package_front: PackageNo,
    /// Library package used when placed on the back side.
    lib_package_back: PackageNo,
    part_number: Option<String>,
    /// `None` if not yet placed.
    location: Option<Point>,
    rotation_in_degree: f64,
    logical_part: Option<LogicalPartNo>,
    on_front: bool,
}

impl Component {
    #[allow(clippy::too_many_arguments)]
    fn new(
        name: String,
        location: Option<Point>,
        rotation_in_degree: f64,
        on_front: bool,
        package_front: PackageNo,
        package_back: PackageNo,
        id: ComponentNo,
        position_fixed: bool,
        part_number: Option<String>,
    ) -> Self {
        Component {
            name,
            id,
            position_fixed,
            lib_package_front: package_front,
            lib_package_back: package_back,
            part_number,
            location,
            rotation_in_degree: normalize_rotation(rotation_in_degree),
            logical_part: None,
            on_front,
        }
    }

    pub fn get_location(&self) -> Option<&Point> {
        self.location.as_ref()
    }

    pub fn get_rotation_in_degree(&self) -> f64 {
        self.rotation_in_degree
    }

    pub fn is_placed(&self) -> bool {
        self.location.is_some()
    }

    /// If false, the component is placed on the back side of the board.
    pub fn placed_on_front(&self) -> bool {
        self.on_front
    }

    /// Translates the location (the pins on the board must be moved separately).
    pub fn translate_by(&mut self, vector: &Vector) {
        if let Some(location) = &self.location {
            self.location = Some(location.translate_by(vector));
        }
    }

    /// Turns this component by `factor` times 90 degree around `pole`.
    pub fn turn_90_degree(&mut self, factor: i32, pole: &IntPoint) {
        if factor == 0 {
            return;
        }
        self.rotation_in_degree =
            normalize_rotation(self.rotation_in_degree + factor.wrapping_mul(90) as f64);
        if let Some(location) = &self.location {
            self.location = Some(location.turn_90_degree(factor, &Point::Int(*pole)));
        }
    }

    /// Rotates this component by `angle_in_degree` around `pole`.
    pub fn rotate(&mut self, angle_in_degree: f64, pole: &IntPoint, flip_style_rotate_first: bool) {
        if angle_in_degree == 0.0 {
            return;
        }
        let mut turn_angle = angle_in_degree;
        if flip_style_rotate_first && !self.placed_on_front() {
            // take care of the order of mirroring and rotating on the back side of the board
            turn_angle = 360.0 - angle_in_degree;
        }
        self.rotation_in_degree = normalize_rotation(self.rotation_in_degree + turn_angle);
        if let Some(location) = &self.location {
            let rotated = location
                .to_float()
                .rotate(to_radians(angle_in_degree), &pole.to_float())
                .round();
            self.location = Some(Point::Int(rotated));
        }
    }

    /// Changes the placement side and mirrors the location at the vertical line through `pole`.
    /// Panics if the component is not placed (Java NPE).
    pub fn change_side(&mut self, pole: &IntPoint) {
        self.on_front = !self.on_front;
        let location = self
            .location
            .as_ref()
            .expect("Component.changeSide: location is null");
        self.location = Some(location.mirror_vertical(&Point::Int(*pole)));
    }

    /// Compares by name ignoring case (Java `compareTo`).
    pub fn compare_to(&self, other: &Component) -> i32 {
        compare_to_ignore_case(&self.name, &other.name)
    }

    pub fn get_part_number(&self) -> Option<&str> {
        self.part_number.as_deref()
    }

    /// Information for pin swap and gate swap, if any.
    pub fn get_logical_part(&self) -> Option<LogicalPartNo> {
        self.logical_part
    }

    pub fn set_logical_part(&mut self, logical_part: Option<LogicalPartNo>) {
        self.logical_part = logical_part;
    }

    /// The library package for the current placement side.
    pub fn get_package(&self) -> PackageNo {
        if self.on_front {
            self.lib_package_front
        } else {
            self.lib_package_back
        }
    }

    pub fn package_front(&self) -> PackageNo {
        self.lib_package_front
    }

    pub fn package_back(&self) -> PackageNo {
        self.lib_package_back
    }
}

impl std::fmt::Display for Component {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

/// The list of components on the board (Java `Components`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Components {
    component_arr: Vec<Component>,
    /// If true, back side components are rotated before mirroring, else mirrored before
    /// rotating.
    flip_style_rotate_first: bool,
}

impl Components {
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a component (its items must be inserted into the board separately) and returns
    /// its id. If `on_front` is false, `package_back` is used.
    #[allow(clippy::too_many_arguments)]
    pub fn add(
        &mut self,
        name: impl Into<String>,
        location: Option<Point>,
        rotation_in_degree: f64,
        on_front: bool,
        package_front: PackageNo,
        package_back: PackageNo,
        position_fixed: bool,
        part_number: Option<String>,
    ) -> ComponentNo {
        let id = self.component_arr.len() as ComponentNo + 1;
        self.component_arr.push(Component::new(
            name.into(),
            location,
            rotation_in_degree,
            on_front,
            package_front,
            package_back,
            id,
            position_fixed,
            part_number,
        ));
        id
    }

    /// Adds a component with the generated name `Component#<n>`.
    pub fn add_unnamed(
        &mut self,
        location: Option<Point>,
        rotation: f64,
        on_front: bool,
        component_package: PackageNo,
    ) -> ComponentNo {
        let name = format!("Component#{}", self.component_arr.len() + 1);
        self.add(
            name,
            location,
            rotation,
            on_front,
            component_package,
            component_package,
            false,
            None,
        )
    }

    /// The component with the given name (case-sensitive), if any.
    pub fn get_by_name(&self, name: &str) -> Option<&Component> {
        self.component_arr.iter().find(|c| c.name == name)
    }

    /// The component with the given id (1-based). Panics if out of range (Java
    /// `ArrayIndexOutOfBoundsException`).
    pub fn get(&self, component_id: ComponentNo) -> &Component {
        let result = &self.component_arr[(component_id - 1) as usize];
        if result.id != component_id {
            log::warn!("Components.get: inconsistent component ID");
        }
        result
    }

    fn get_mut(&mut self, component_id: ComponentNo) -> &mut Component {
        &mut self.component_arr[(component_id - 1) as usize]
    }

    pub fn count(&self) -> i32 {
        self.component_arr.len() as i32
    }

    /// All components in id order.
    pub fn get_all(&self) -> std::slice::Iter<'_, Component> {
        self.component_arr.iter()
    }

    /// Moves the component with the given id (Java `move`).
    pub fn move_component(&mut self, component_id: ComponentNo, vector: &Vector) {
        self.get_mut(component_id).translate_by(vector);
    }

    /// Turns the component by `factor` times 90 degree around `pole`.
    pub fn turn_90_degree(&mut self, component_id: ComponentNo, factor: i32, pole: &IntPoint) {
        self.get_mut(component_id).turn_90_degree(factor, pole);
    }

    /// Rotates the component by `rotation_in_degree` around `pole`, honouring the flip style.
    pub fn rotate(&mut self, component_id: ComponentNo, rotation_in_degree: f64, pole: &IntPoint) {
        let flip = self.flip_style_rotate_first;
        self.get_mut(component_id)
            .rotate(rotation_in_degree, pole, flip);
    }

    /// Changes the placement side of the component, mirroring at the vertical line through pole.
    pub fn change_side(&mut self, component_id: ComponentNo, pole: &IntPoint) {
        self.get_mut(component_id).change_side(pole);
    }

    pub fn get_flip_style_rotate_first(&self) -> bool {
        self.flip_style_rotate_first
    }

    pub fn set_flip_style_rotate_first(&mut self, value: bool) {
        self.flip_style_rotate_first = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fr_geom::IntVector;

    fn p(x: i32, y: i32) -> Point {
        Point::Int(IntPoint::new(x, y))
    }

    #[test]
    fn add_and_lookup() {
        let mut cs = Components::new();
        let a = cs.add(
            "U1",
            Some(p(10, 0)),
            450.0,
            true,
            1,
            2,
            false,
            Some("PN".into()),
        );
        let b = cs.add_unnamed(None, -90.0, false, 3);
        assert_eq!((a, b), (1, 2));
        assert_eq!(cs.get(1).get_rotation_in_degree(), 90.0);
        assert_eq!(cs.get(2).get_rotation_in_degree(), 270.0);
        assert_eq!(cs.get(2).name, "Component#2");
        assert_eq!(cs.get(2).get_package(), 3);
        assert!(!cs.get(2).is_placed());
        assert_eq!(cs.get_by_name("U1").unwrap().id, 1);
        assert!(cs.get_by_name("u1").is_none());
        assert_eq!(cs.get(1).get_package(), 1);
        cs.change_side(1, &IntPoint::new(0, 0));
        assert_eq!(cs.get(1).get_package(), 2);
        assert_eq!(cs.get(1).get_location(), Some(&p(-10, 0)));
        assert!(cs.get(1).compare_to(cs.get(2)) > 0);
    }

    #[test]
    fn moves() {
        let mut cs = Components::new();
        let id = cs.add("U1", Some(p(10, 0)), 0.0, false, 1, 1, false, None);
        cs.move_component(id, &Vector::from(IntVector::new(5, 5)));
        assert_eq!(cs.get(id).get_location(), Some(&p(15, 5)));
        cs.turn_90_degree(id, 1, &IntPoint::new(0, 0));
        assert_eq!(cs.get(id).get_location(), Some(&p(-5, 15)));
        assert_eq!(cs.get(id).get_rotation_in_degree(), 90.0);
        cs.set_flip_style_rotate_first(true);
        cs.rotate(id, 90.0, &IntPoint::new(0, 0));
        // back side with flip style: rotation += 360 - 90
        assert_eq!(cs.get(id).get_rotation_in_degree(), 0.0);
        assert_eq!(cs.get(id).get_location(), Some(&p(-15, -5)));
        cs.turn_90_degree(id, -3, &IntPoint::new(0, 0));
        assert_eq!(cs.get(id).get_rotation_in_degree(), 90.0);
    }
}
