//! Port of `core/library/Padstack.java` and `Padstacks.java`.

use std::sync::OnceLock;

use fr_geom::{ConvexShape, Direction, TileShape};
use fr_jcompat::compare_to_ignore_case;

use crate::ids::{LayerNo, PadstackNo};
use crate::structure::LayerStructure;

/// Java `String.equalsIgnoreCase`.
pub(crate) fn equals_ignore_case(a: &str, b: &str) -> bool {
    compare_to_ignore_case(a, b) == 0
}

/// Padstack masks (one optional convex shape per board layer) for pins or vias, located at the
/// origin (Java `Padstack`).
#[derive(Clone, Debug)]
pub struct Padstack {
    pub name: String,
    /// 1-based id in [`Padstacks`].
    pub id: PadstackNo,
    /// Whether vias of the own net may overlap with this padstack.
    pub attach_allowed: bool,
    /// If false, the layers are mirrored when placed on the back side.
    pub placed_absolute: bool,
    shapes: Vec<Option<ConvexShape>>,
    /// Copper shapes synthesized from the drill radius for padstacks without copper
    /// (non-plated holes); they are obstacles only, not real copper.
    pub hole_only: bool,
    cached_drill_radius: OnceLock<f64>,
}

impl Padstack {
    fn new(
        name: String,
        id: PadstackNo,
        shapes: Vec<Option<ConvexShape>>,
        is_drillable: bool,
        placed_absolute: bool,
    ) -> Self {
        Padstack {
            name,
            id,
            attach_allowed: is_drillable,
            placed_absolute,
            shapes,
            hole_only: false,
            cached_drill_radius: OnceLock::new(),
        }
    }

    /// Compares by name ignoring case.
    pub fn compare_to(&self, other: &Padstack) -> i32 {
        compare_to_ignore_case(&self.name, &other.name)
    }

    /// Drill radius in board units: parsed from names like `Via[0-1]_800:400_um` relative to
    /// the smallest pad radius, else 0.45 times the smallest pad radius. Cached.
    pub fn get_drill_radius(&self) -> f64 {
        *self
            .cached_drill_radius
            .get_or_init(|| self.calc_drill_radius())
    }

    fn calc_drill_radius(&self) -> f64 {
        let name = &self.name;
        // Only ASCII ':' and '_' are searched, so byte indices are consistent with Java's
        // UTF-16 indices for the substring operations below.
        if let Some(colon_index) = name.find(':') {
            let after = &name[colon_index..];
            let drill_str = match after.find('_') {
                Some(u) if u > 0 => &name[colon_index + 1..colon_index + u],
                _ => &name[colon_index + 1..],
            };
            if let Some(drill_dia) = parse_java_double(&keep_digits_and_dots(drill_str)) {
                // name.lastIndexOf('_', colonIndex)
                if let Some(last_underscore) = name[..=colon_index].rfind('_') {
                    let outer_str = keep_digits_and_dots(&name[last_underscore + 1..colon_index]);
                    if let Some(outer_dia) = parse_java_double(&outer_str) {
                        if outer_dia > 0.0 {
                            let actual_outer_radius = self.get_smallest_radius();
                            if actual_outer_radius > 0.0 {
                                return actual_outer_radius * (drill_dia / outer_dia);
                            }
                        }
                    }
                    // Java: a NumberFormatException of the outer diameter falls through too.
                }
            }
        }
        self.get_smallest_radius() * 0.45
    }

    fn get_smallest_radius(&self) -> f64 {
        let mut min_radius = f64::MAX;
        for shape in self.shapes.iter().flatten() {
            let b = shape.bounding_box();
            let radius = b.width().min(b.height()) as f64 / 2.0;
            if radius < min_radius {
                min_radius = radius;
            }
        }
        if min_radius == f64::MAX {
            0.0
        } else {
            min_radius
        }
    }

    /// The shape on `layer`; `None` if there is none or `layer` is out of range.
    pub fn get_shape(&self, layer: LayerNo) -> Option<&ConvexShape> {
        if layer < 0 || layer as usize >= self.shapes.len() {
            log::warn!("Padstack.get_layer layer out of range");
            return None;
        }
        self.shapes[layer as usize].as_ref()
    }

    /// The first layer with a shape (`shapes.len()` if there is none).
    pub fn from_layer(&self) -> LayerNo {
        let mut result = 0;
        while (result as usize) < self.shapes.len() && self.shapes[result as usize].is_none() {
            result += 1;
        }
        result
    }

    /// The last layer with a shape (-1 if there is none).
    pub fn to_layer(&self) -> LayerNo {
        let mut result = self.shapes.len() as LayerNo - 1;
        while result >= 0 && self.shapes[result as usize].is_none() {
            result -= 1;
        }
        result
    }

    /// The layer count of the board of this padstack.
    pub fn board_layer_count(&self) -> i32 {
        self.shapes.len() as i32
    }

    /// Allowed trace exit directions on `layer` (only for box and octagon pads). If the pad
    /// length is smaller than `factor` times its height, the long side is also allowed.
    pub fn get_trace_exit_directions(&self, layer: LayerNo, factor: f64) -> Vec<Direction> {
        let mut result = Vec::new();
        if layer < 0 || layer as usize >= self.shapes.len() {
            return result;
        }
        let Some(current_shape) = &self.shapes[layer as usize] else {
            return result;
        };
        if !matches!(
            current_shape,
            ConvexShape::Tile(TileShape::IntBox(_) | TileShape::IntOctagon(_))
        ) {
            return result;
        }
        let current_box = current_shape.bounding_box();
        let (w, h) = (current_box.width(), current_box.height());
        let all_dirs = (w.max(h) as f64) < factor * w.min(h) as f64;
        if all_dirs || w >= h {
            result.push(Direction::RIGHT);
            result.push(Direction::LEFT);
        }
        if all_dirs || w <= h {
            result.push(Direction::UP);
            result.push(Direction::DOWN);
        }
        result
    }
}

impl std::fmt::Display for Padstack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

/// `s.replaceAll("[^0-9.]", "")`.
fn keep_digits_and_dots(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_digit() || *c == '.')
        .collect()
}

/// `Double.parseDouble` for strings consisting of ASCII digits and dots only.
fn parse_java_double(s: &str) -> Option<f64> {
    // Java accepts "1.", ".5", "1.5"; rejects "", ".", "1.2.3" — same as Rust for this alphabet.
    s.parse::<f64>().ok()
}

/// The library of padstacks (Java `Padstacks`). Ids are 1-based.
#[derive(Clone, Debug, Default)]
pub struct Padstacks {
    board_layer_count: i32,
    padstacks: Vec<Padstack>,
}

impl Padstacks {
    pub fn new(layer_structure: &LayerStructure) -> Self {
        Padstacks {
            board_layer_count: layer_structure.layer_count(),
            padstacks: Vec::new(),
        }
    }

    /// Layer count of the board layer structure.
    pub fn board_layer_count(&self) -> i32 {
        self.board_layer_count
    }

    /// The padstack with the given name (ignoring case), if any.
    pub fn get_by_name(&self, name: &str) -> Option<&Padstack> {
        self.padstacks
            .iter()
            .find(|p| equals_ignore_case(&p.name, name))
    }

    /// The padstack with the given id; `None` (with a warning) if out of range.
    pub fn get(&self, padstack_id: PadstackNo) -> Option<&Padstack> {
        if padstack_id <= 0 || padstack_id as usize > self.padstacks.len() {
            log::warn!(
                "Padstacks.get: 1 <= padstackId <= {} expected",
                self.padstacks.len()
            );
            return None;
        }
        let result = &self.padstacks[(padstack_id - 1) as usize];
        if result.id != padstack_id {
            log::warn!("Padstacks.get: inconsistent padstack ID");
        }
        Some(result)
    }

    /// Mutable access (Java code mutates `holeOnly`).
    pub fn get_mut(&mut self, padstack_id: PadstackNo) -> Option<&mut Padstack> {
        if padstack_id <= 0 {
            return None;
        }
        self.padstacks.get_mut((padstack_id - 1) as usize)
    }

    pub fn count(&self) -> i32 {
        self.padstacks.len() as i32
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Padstack> {
        self.padstacks.iter()
    }

    /// Appends a padstack with one optional shape per layer; returns its id.
    pub fn add(
        &mut self,
        name: impl Into<String>,
        shapes: Vec<Option<ConvexShape>>,
        drill_allowed: bool,
        placed_absolute: bool,
    ) -> PadstackNo {
        let id = self.padstacks.len() as PadstackNo + 1;
        self.padstacks.push(Padstack::new(
            name.into(),
            id,
            shapes,
            drill_allowed,
            placed_absolute,
        ));
        id
    }

    /// Appends a padstack named `padstack#<n>`.
    pub fn add_unnamed(&mut self, shapes: Vec<Option<ConvexShape>>) -> PadstackNo {
        let name = format!("padstack#{}", self.padstacks.len() + 1);
        self.add(name, shapes, false, false)
    }

    /// Appends a padstack named `padstack#<n>` with `shape` on the layers `from_layer ..=
    /// to_layer` (clamped to the board layers).
    pub fn add_shape_range(
        &mut self,
        shape: &ConvexShape,
        from_layer: LayerNo,
        to_layer: LayerNo,
    ) -> PadstackNo {
        let n = self.board_layer_count;
        let mut shapes: Vec<Option<ConvexShape>> = vec![None; n.max(0) as usize];
        let first_layer = from_layer.max(0);
        let last_layer = to_layer.min(n - 1);
        for i in first_layer..=last_layer {
            shapes[i as usize] = Some(shape.clone());
        }
        self.add_unnamed(shapes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structure::Layer;
    use fr_geom::{Circle, IntBox, IntPoint};

    fn layers(n: usize) -> LayerStructure {
        LayerStructure::new((0..n).map(|i| Layer::new(format!("L{i}"), true)).collect())
    }

    fn rect(w: i32, h: i32) -> ConvexShape {
        ConvexShape::Tile(TileShape::IntBox(IntBox::new(-w / 2, -h / 2, w / 2, h / 2)))
    }

    #[test]
    fn padstack_queries() {
        let mut ps = Padstacks::new(&layers(4));
        let circle = ConvexShape::Circle(Circle::new(IntPoint::new(0, 0), 300));
        let id = ps.add_shape_range(&circle, 1, 7);
        assert_eq!(id, 1);
        let p = ps.get(id).unwrap();
        assert_eq!(p.name, "padstack#1");
        assert_eq!((p.from_layer(), p.to_layer()), (1, 3));
        assert!(p.get_shape(0).is_none());
        assert!(p.get_shape(4).is_none());
        assert!(p.get_shape(2).is_some());
        assert_eq!(p.board_layer_count(), 4);
        assert!(ps.get(0).is_none() && ps.get(2).is_none());

        let empty = ps.add("Empty", vec![None; 4], true, false);
        let e = ps.get(empty).unwrap();
        assert_eq!((e.from_layer(), e.to_layer()), (4, -1));
        assert_eq!(e.get_drill_radius(), 0.0);
        assert!(ps.get_by_name("EMPTY").is_some());
        assert!(ps.get_by_name("nope").is_none());
        assert!(e.compare_to(ps.get(1).unwrap()) < 0);
    }

    #[test]
    fn drill_radius() {
        let mut ps = Padstacks::new(&layers(2));
        let s = rect(800, 600);
        // name encodes outer 800 and drill 400: radius = 300 * 400 / 800
        let a = ps.add(
            "Via[0-1]_800:400_um",
            vec![Some(s.clone()), Some(s.clone())],
            true,
            false,
        );
        assert_eq!(
            ps.get(a).unwrap().get_drill_radius(),
            300.0 * (400.0 / 800.0)
        );
        // no underscore before the colon -> 0.45 * smallest radius
        let b = ps.add("Pad:400", vec![Some(s.clone()), None], false, false);
        assert_eq!(ps.get(b).unwrap().get_drill_radius(), 300.0 * 0.45);
        // unparsable drill
        let c = ps.add("x_1:abc", vec![Some(s), None], false, false);
        assert_eq!(ps.get(c).unwrap().get_drill_radius(), 300.0 * 0.45);
    }

    #[test]
    fn exit_directions() {
        let mut ps = Padstacks::new(&layers(2));
        let wide = ps.add(
            "wide",
            vec![Some(rect(400, 100)), Some(rect(100, 110))],
            false,
            false,
        );
        let p = ps.get(wide).unwrap();
        assert_eq!(
            p.get_trace_exit_directions(0, 1.5),
            vec![Direction::RIGHT, Direction::LEFT]
        );
        assert_eq!(
            p.get_trace_exit_directions(1, 1.5),
            vec![
                Direction::RIGHT,
                Direction::LEFT,
                Direction::UP,
                Direction::DOWN
            ]
        );
        assert_eq!(
            p.get_trace_exit_directions(1, 1.0),
            vec![Direction::UP, Direction::DOWN]
        );
        assert!(p.get_trace_exit_directions(2, 1.0).is_empty());
        let round = ps.add(
            "round",
            vec![
                Some(ConvexShape::Circle(Circle::new(IntPoint::new(0, 0), 50))),
                None,
            ],
            false,
            false,
        );
        assert!(ps
            .get(round)
            .unwrap()
            .get_trace_exit_directions(0, 2.0)
            .is_empty());
    }
}
