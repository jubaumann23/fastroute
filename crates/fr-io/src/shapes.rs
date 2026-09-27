//! DSN shapes -> board geometry (Java `io.specctra.parser.{Shape, Rectangle, Polygon, Circle,
//! PolygonPath, PolylinePath, Layer, LayerStructure}` and the padstack part of
//! `Library.readPadstackScope`).
//!
//! The typed DSN model keeps layer names unresolved; the Java reader resolves them while
//! reading, with per-shape-kind rules that decide whether a shape is dropped (`null`):
//!
//! | kind            | unknown named layer | layer structure not yet known |
//! |-----------------|---------------------|-------------------------------|
//! | `rect`          | falls back to `signal` | falls back to `signal`     |
//! | `polygon`       | `null`              | `null`                        |
//! | `circle`        | `null`              | `null`                        |
//! | `path`          | `null`              | `null`                        |
//! | `polyline_path` | kept, layer `null`  | kept, layer `null`            |
//!
//! `pcb` and `signal` never need the layer structure. Named layers are looked up with
//! [`ParserLayers::get_no`], which falls back to the first/last layer for names containing
//! `Top`/`Bottom` (Electra outline layers).

use fr_dsn::model::{LayerRef, Shape as DsnShape, ShapeKind};
use fr_engine::ids::LayerNo;
use fr_engine::structure::CoordinateTransform;
use fr_geom::java_compat::math_round_i32;
use fr_geom::prelude::*;
use fr_geom::{
    Area, Circle, ConvexShape, FloatPoint, IntBox, IntPoint, Point, PolygonShape, PolylineArea,
    PolylineShape, Shape, Simplex, TileShape,
};

use crate::error::{npe, LResult, LoadError};

/// A layer of the DSN parser layer structure (Java `io.specctra.parser.Layer`).
#[derive(Clone, Debug, PartialEq)]
pub struct ParserLayer {
    pub name: String,
    /// Index in the layer structure (only layers with a known type are counted).
    pub no: LayerNo,
    pub is_signal: bool,
    /// `use_net` names (power planes).
    pub net_names: Vec<String>,
}

/// Java `io.specctra.parser.LayerStructure` (possibly a prefix of the final one, see
/// [`crate::order`]).
#[derive(Clone, Copy, Debug)]
pub struct ParserLayers<'a>(pub &'a [ParserLayer]);

impl ParserLayers<'_> {
    /// Java `LayerStructure.getNo(String)`, including the Electra `Top`/`Bottom` fallback
    /// (`-1` if not found).
    pub fn get_no(&self, name: &str) -> LayerNo {
        if let Some(i) = self.0.iter().position(|l| l.name == name) {
            return i as LayerNo;
        }
        if name.contains("Top") {
            return 0;
        }
        if name.contains("Bottom") {
            return self.0.len() as LayerNo - 1;
        }
        -1
    }

    /// Java `LayerStructure.containsPlane(netName)`.
    pub fn contains_plane(&self, net_name: &str) -> bool {
        self.0
            .iter()
            .any(|l| !l.is_signal && l.net_names.iter().any(|n| n == net_name))
    }
}

/// A resolved Java parser layer: `Layer.PCB`, `Layer.SIGNAL` or a layer of the structure.
#[derive(Clone, Debug, PartialEq)]
pub enum PLayer {
    Pcb,
    Signal,
    Named { no: LayerNo, name: String },
}

impl PLayer {
    /// Java `layer.no` (`-1` for `pcb` and `signal`).
    pub fn no(&self) -> LayerNo {
        match self {
            PLayer::Named { no, .. } => *no,
            _ => -1,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            PLayer::Pcb => "pcb",
            PLayer::Signal => "signal",
            PLayer::Named { name, .. } => name,
        }
    }
}

/// A DSN shape with its layer resolved as the Java reader does; `layer` is `None` only for
/// `polyline_path`s on unknown layers.
#[derive(Clone, Debug)]
pub struct PShape<'a> {
    pub layer: Option<PLayer>,
    pub kind: &'a ShapeKind,
}

impl PShape<'_> {
    pub fn is_path(&self) -> bool {
        matches!(
            self.kind,
            ShapeKind::PolygonPath { .. } | ShapeKind::PolylinePath { .. }
        )
    }
}

fn named_layer(ls: Option<ParserLayers<'_>>, name: &str) -> Option<PLayer> {
    let ls = ls?;
    let no = ls.get_no(name);
    if no < 0 || no as usize >= ls.0.len() {
        return None;
    }
    Some(PLayer::Named {
        no,
        name: ls.0[no as usize].name.clone(),
    })
}

/// Java `Shape.getLayer(layerStructure, layerName)` (string comparison with the keyword names).
fn get_layer(ls: Option<ParserLayers<'_>>, layer: &LayerRef) -> Option<PLayer> {
    match layer {
        LayerRef::Pcb => Some(PLayer::Pcb),
        LayerRef::Signal => Some(PLayer::Signal),
        LayerRef::Named(n) if n == "pcb" => Some(PLayer::Pcb),
        LayerRef::Named(n) if n == "signal" => Some(PLayer::Signal),
        LayerRef::Named(n) => named_layer(ls, n),
    }
}

/// Resolves the layer of a DSN shape like the Java `Shape.read*Scope` methods; `None` means
/// the Java reader returned `null` for this shape.
pub fn resolve<'a>(shape: &'a DsnShape, ls: Option<ParserLayers<'_>>) -> Option<PShape<'a>> {
    let layer = match &shape.kind {
        ShapeKind::Rect(_) => Some(get_layer(ls, &shape.layer).unwrap_or(PLayer::Signal)),
        ShapeKind::Polygon(_) => match &shape.layer {
            // `Shape.readPolygonScope` compares the scanner token with the keywords, and the
            // scanner matches keywords case-insensitively (a layer named `Signal` is SIGNAL).
            // Quoted names are strings, but the typed model does not keep the quoting of
            // named layers; assume unquoted.
            LayerRef::Pcb => Some(PLayer::Pcb),
            LayerRef::Signal => Some(PLayer::Signal),
            LayerRef::Named(n) if n.eq_ignore_ascii_case("pcb") => Some(PLayer::Pcb),
            LayerRef::Named(n) if n.eq_ignore_ascii_case("signal") => Some(PLayer::Signal),
            LayerRef::Named(n) => Some(named_layer(ls, n)?),
        },
        ShapeKind::Circle(_) | ShapeKind::PolygonPath { .. } => Some(get_layer(ls, &shape.layer)?),
        ShapeKind::PolylinePath { .. } => get_layer(ls, &shape.layer),
    };
    Some(PShape {
        layer,
        kind: &shape.kind,
    })
}

// ---------------------------------------------------------------------------
// Transformations

fn int_points(ct: &CoordinateTransform, coords: &[f64], rel: bool) -> Vec<Point> {
    (0..coords.len() / 2)
        .map(|i| {
            let p = if rel {
                IntPoint::new(
                    math_round_i32(ct.dsn_to_board(coords[2 * i])),
                    math_round_i32(ct.dsn_to_board(coords[2 * i + 1])),
                )
            } else {
                ct.dsn_to_board_point(&coords[2 * i..2 * i + 2]).round()
            };
            Point::Int(p)
        })
        .collect()
}

fn polygon_shape(points: &[Point]) -> LResult<PolygonShape> {
    if points.is_empty() {
        return Err(LoadError::JavaException(
            "ArrayIndexOutOfBoundsException: PolygonShape of an empty polygon".into(),
        ));
    }
    Ok(PolygonShape::from_points(points))
}

fn polygon_path_to_board(
    ct: &CoordinateTransform,
    width: f64,
    coords: &[f64],
    rel: bool,
) -> LResult<Shape> {
    let corners: Vec<FloatPoint> = (0..coords.len() / 2)
        .map(|i| {
            let t = &coords[2 * i..2 * i + 2];
            if rel {
                ct.dsn_to_board_rel(t)
            } else {
                ct.dsn_to_board_point(t)
            }
        })
        .collect();
    let offset = ct.dsn_to_board(width) / 2.0;
    if corners.len() <= 2 {
        let oct = FloatPoint::bounding_octagon(&corners);
        return Ok(Shape::Tile(TileShape::IntOctagon(oct.enlarge(offset))));
    }
    let rounded: Vec<Point> = corners.iter().map(|c| Point::Int(c.round())).collect();
    let result = Shape::Polygon(polygon_shape(&rounded)?);
    if offset > 0.0 {
        return Ok(Shape::Tile(result.bounding_tile().enlarge(offset)));
    }
    Ok(result)
}

/// Java `Shape.transformToBoard`; `Ok(None)` where Java returns `null` (`polyline_path`).
pub fn transform_to_board(kind: &ShapeKind, ct: &CoordinateTransform) -> LResult<Option<Shape>> {
    Ok(Some(match kind {
        ShapeKind::Rect(c) => {
            let ll = ct.dsn_to_board_point(&[c[0].min(c[2]), c[1].min(c[3])]);
            let ur = ct.dsn_to_board_point(&[c[0].max(c[2]), c[1].max(c[3])]);
            Shape::Tile(TileShape::IntBox(IntBox::from_points(
                ll.round(),
                ur.round(),
            )))
        }
        ShapeKind::Polygon(coords) => {
            Shape::Polygon(polygon_shape(&int_points(ct, coords, false))?)
        }
        ShapeKind::Circle(c) => {
            let center = ct.dsn_to_board_point(&[c[1], c[2]]).round();
            let radius = math_round_i32(ct.dsn_to_board(c[0]) / 2.0);
            Shape::Circle(Circle::new(center, radius))
        }
        ShapeKind::PolygonPath { width, coords } => {
            polygon_path_to_board(ct, *width, coords, false)?
        }
        ShapeKind::PolylinePath { .. } => return Ok(None),
    }))
}

/// Java `Shape.transformToBoardRel`; `Ok(None)` where Java returns `null` (`polyline_path`).
pub fn transform_to_board_rel(
    kind: &ShapeKind,
    ct: &CoordinateTransform,
) -> LResult<Option<Shape>> {
    Ok(Some(match kind {
        ShapeKind::Rect(c) => {
            let b: Vec<i32> = c
                .iter()
                .map(|v| math_round_i32(ct.dsn_to_board(*v)))
                .collect();
            let r = if b[1] <= b[3] {
                IntBox::new(b[0], b[1], b[2], b[3])
            } else {
                IntBox::new(b[0], b[3], b[2], b[1])
            };
            Shape::Tile(TileShape::IntBox(r))
        }
        ShapeKind::Polygon(coords) => {
            if coords.len() < 2 {
                Shape::Tile(TileShape::Simplex(Simplex::empty()))
            } else {
                Shape::Polygon(polygon_shape(&int_points(ct, coords, true))?)
            }
        }
        ShapeKind::Circle(c) => {
            let r = math_round_i32(ct.dsn_to_board(c[0]) / 2.0);
            let x = math_round_i32(ct.dsn_to_board(c[1]));
            let y = math_round_i32(ct.dsn_to_board(c[2]));
            Shape::Circle(Circle::new(IntPoint::new(x, y), r))
        }
        ShapeKind::PolygonPath { width, coords } => {
            polygon_path_to_board(ct, *width, coords, true)?
        }
        ShapeKind::PolylinePath { .. } => return Ok(None),
    }))
}

/// Java `Shape.boundingBox()` in DSN coordinates (`[x1, y1, x2, y2]`, a `Rectangle` returns
/// its own, possibly unordered, coordinates); `None` for `polyline_path` (Java `null`).
/// Reproduces the Java quirks: circles use the diameter as radius, paths add the
/// half width to the maximum x once per coordinate.
pub fn bounding_box(kind: &ShapeKind) -> Option<[f64; 4]> {
    const MAX: f64 = i32::MAX as f64;
    const MIN: f64 = i32::MIN as f64;
    match kind {
        ShapeKind::Rect(c) => Some(*c),
        ShapeKind::Polygon(coords) => {
            let mut b = [MAX, MAX, MIN, MIN];
            for (i, &v) in coords.iter().enumerate() {
                if i % 2 == 0 {
                    b[0] = b[0].min(v);
                    b[2] = b[2].max(v);
                } else {
                    b[1] = b[1].min(v);
                    b[3] = b[3].max(v);
                }
            }
            Some(b)
        }
        ShapeKind::Circle(c) => Some([c[1] - c[0], c[2] - c[0], c[1] + c[0], c[2] + c[0]]),
        ShapeKind::PolygonPath { width, coords } => {
            let offset = width / 2.0;
            let mut b = [MAX, MAX, MIN, MIN];
            for (i, &v) in coords.iter().enumerate() {
                if i % 2 == 0 {
                    b[0] = b[0].min(v - offset);
                    b[2] = b[2].max(v) + offset;
                } else {
                    b[1] = b[1].min(v - offset);
                    b[3] = b[3].max(v + offset);
                }
            }
            Some(b)
        }
        ShapeKind::PolylinePath { .. } => None,
    }
}

/// Java `Rectangle.union`.
pub fn rect_union(a: &[f64; 4], b: &[f64; 4]) -> [f64; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

/// Java `Shape.transformAreaToBoard(Rel)`: the first shape is the border, the others holes.
/// Entries are `None` where the Java shape list contains `null` (NPE when transformed).
/// `Ok(None)` where Java returns `null` (non-polyline border/hole of an area with holes, or a
/// `null` transformed border without holes).
pub fn transform_area(
    shapes: &[Option<PShape<'_>>],
    ct: &CoordinateTransform,
    rel: bool,
) -> LResult<Option<Area>> {
    let transform = |s: &Option<PShape<'_>>| -> LResult<Option<Shape>> {
        let s = s
            .as_ref()
            .ok_or_else(|| npe("Shape.transformAreaToBoard: null shape"))?;
        if rel {
            transform_to_board_rel(s.kind, ct)
        } else {
            transform_to_board(s.kind, ct)
        }
    };
    let Some(first) = shapes.first() else {
        log::warn!("Shape.transform_area_to_board: area.size() > 0 expected");
        return Ok(None);
    };
    let boundary = transform(first)?;
    if shapes.len() == 1 {
        return Ok(boundary.map(Area::Shape));
    }
    let Some(border) = boundary.as_ref().and_then(Shape::to_polyline_shape) else {
        log::warn!("Shape.transform_area_to_board: PolylineShape expected");
        return Ok(None);
    };
    let mut holes = Vec::with_capacity(shapes.len() - 1);
    for h in &shapes[1..] {
        let hole = transform(h)?;
        match hole.as_ref().and_then(Shape::to_polyline_shape) {
            Some(p) => holes.push(p),
            None => {
                log::warn!("Shape.transform_area_to_board: PolylineShape expected");
                return Ok(None);
            }
        }
    }
    Ok(Some(Area::PolylineArea(PolylineArea::new(border, holes))))
}

/// The convex padstack shape of one `(shape ...)` of a padstack, as computed in
/// `Library.readPadstackScope`: convex shapes are used directly, polygons are replaced by
/// the first piece of their convex hull split into convex pieces (simplified if it is a
/// `Simplex`); shapes of dimension < 2 are enlarged by 1 (and dropped if still degenerate).
pub fn padstack_shape(board_shape: Option<Shape>) -> LResult<Option<ConvexShape>> {
    let shape = board_shape.ok_or_else(|| npe("Library.readPadstackScope: null shape"))?;
    let convex = match shape.to_convex_shape() {
        Some(c) => c,
        None => {
            let current = match &shape {
                Shape::Polygon(p) => Shape::Polygon(p.convex_hull()),
                other => other.clone(),
            };
            let pieces = current
                .split_to_convex()
                .ok_or_else(|| npe("Library.readPadstackScope: splitToConvex returned null"))?;
            if pieces.len() != 1 {
                log::warn!("Library.read_padstack_scope: convex shape expected");
            }
            let first = pieces.into_iter().next().ok_or_else(|| {
                LoadError::JavaException(
                    "ArrayIndexOutOfBoundsException: no convex piece in padstack shape".into(),
                )
            })?;
            match first {
                TileShape::Simplex(s) => ConvexShape::Tile(s.simplify()),
                t => ConvexShape::Tile(t),
            }
        }
    };
    if convex.dimension() < 2 {
        log::warn!("Library.read_padstack_scope: padstack shape is not an area, enlarging it");
        let enlarged = convex.offset(1.0);
        if enlarged.dimension() < 2 {
            return Ok(None);
        }
        return Ok(Some(enlarged));
    }
    Ok(Some(convex))
}

/// Java `IntBox.contains(FloatPoint)` (the `TileShape` default implementation).
pub fn box_contains(b: &IntBox, p: &FloatPoint) -> bool {
    TileShapeImpl::contains_float(b, p)
}

/// Java `PolylineShape.dimension()` for an outline shape.
pub fn polyline_dimension(p: &PolylineShape) -> i32 {
    p.dimension()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ls() -> Vec<ParserLayer> {
        ["F.Cu", "In1.Cu", "B.Cu"]
            .iter()
            .enumerate()
            .map(|(i, n)| ParserLayer {
                name: n.to_string(),
                no: i as i32,
                is_signal: i != 1,
                net_names: if i == 1 { vec!["GND".into()] } else { vec![] },
            })
            .collect()
    }

    fn shape(layer: &str, kind: ShapeKind) -> DsnShape {
        let layer = match layer {
            "pcb" => LayerRef::Pcb,
            "signal" => LayerRef::Signal,
            n => LayerRef::Named(n.into()),
        };
        DsnShape {
            layer,
            kind,
            line: 1,
        }
    }

    #[test]
    fn layer_resolution() {
        let l = ls();
        let p = ParserLayers(&l);
        assert_eq!(p.get_no("B.Cu"), 2);
        assert_eq!(p.get_no("TopLayer"), 0);
        assert_eq!(p.get_no("BottomLayer"), 2);
        assert_eq!(p.get_no("X"), -1);
        assert!(p.contains_plane("GND"));
        assert!(!p.contains_plane("VCC"));
        let rect = shape("Nope", ShapeKind::Rect([0.0, 0.0, 1.0, 1.0]));
        assert_eq!(resolve(&rect, Some(p)).unwrap().layer, Some(PLayer::Signal));
        assert_eq!(resolve(&rect, None).unwrap().layer, Some(PLayer::Signal));
        let poly = shape("Nope", ShapeKind::Polygon(vec![0.0; 6]));
        assert!(resolve(&poly, Some(p)).is_none());
        let poly = shape("B.Cu", ShapeKind::Polygon(vec![0.0; 6]));
        assert_eq!(resolve(&poly, Some(p)).unwrap().layer.unwrap().no(), 2);
        assert!(resolve(&poly, None).is_none());
        let circ = shape("pcb", ShapeKind::Circle([1.0, 0.0, 0.0]));
        assert_eq!(resolve(&circ, None).unwrap().layer, Some(PLayer::Pcb));
        let pl = shape(
            "Nope",
            ShapeKind::PolylinePath {
                width: 1.0,
                coords: vec![0.0; 4],
            },
        );
        assert!(resolve(&pl, Some(p)).unwrap().layer.is_none());
        let pp = shape(
            "Nope",
            ShapeKind::PolygonPath {
                width: 1.0,
                coords: vec![0.0; 4],
            },
        );
        assert!(resolve(&pp, Some(p)).is_none());
    }

    #[test]
    fn transforms() {
        let ct = CoordinateTransform::new(10.0, 0.0, 0.0);
        let r = transform_to_board(&ShapeKind::Rect([5.0, 7.0, -1.0, 1.25]), &ct)
            .unwrap()
            .unwrap();
        assert_eq!(r.bounding_box(), IntBox::new(-10, 13, 50, 70));
        let r = transform_to_board_rel(&ShapeKind::Rect([5.0, 7.0, -1.0, 1.25]), &ct)
            .unwrap()
            .unwrap();
        // upper-left / lower-right corners: y swapped, x kept as given
        assert_eq!(r.as_int_box().unwrap(), &IntBox::new(50, 13, -10, 70));
        let c = transform_to_board(&ShapeKind::Circle([3.0, 1.0, 2.0]), &ct)
            .unwrap()
            .unwrap();
        let c = c.as_circle().unwrap();
        assert_eq!((c.center, c.radius), (IntPoint::new(10, 20), 15));
        assert!(transform_to_board(
            &ShapeKind::PolylinePath {
                width: 1.0,
                coords: vec![0.0; 4]
            },
            &ct
        )
        .unwrap()
        .is_none());
        let p = transform_to_board_rel(&ShapeKind::Polygon(vec![1.0]), &ct)
            .unwrap()
            .unwrap();
        assert!(p.is_empty());
        assert!(transform_to_board(&ShapeKind::Polygon(vec![]), &ct).is_err());
        // a 2-point path is the enlarged bounding octagon of its corners
        let path = transform_to_board(
            &ShapeKind::PolygonPath {
                width: 2.0,
                coords: vec![0.0, 0.0, 10.0, 0.0],
            },
            &ct,
        )
        .unwrap()
        .unwrap();
        assert_eq!(path.bounding_box(), IntBox::new(-10, -10, 110, 10));
    }

    #[test]
    fn bounding_boxes() {
        let b = bounding_box(&ShapeKind::Circle([2.0, 10.0, 20.0])).unwrap();
        assert_eq!(b, [8.0, 18.0, 12.0, 22.0]);
        let b = bounding_box(&ShapeKind::PolygonPath {
            width: 2.0,
            coords: vec![0.0, 0.0, 10.0, 5.0, 3.0, 1.0],
        })
        .unwrap();
        // the Java bug adds the half width to max x once per x coordinate
        assert_eq!(b, [-1.0, -1.0, 12.0, 6.0]);
        assert!(bounding_box(&ShapeKind::PolylinePath {
            width: 2.0,
            coords: vec![0.0; 4]
        })
        .is_none());
    }

    #[test]
    fn padstack_shapes() {
        let ct = CoordinateTransform::new(1.0, 0.0, 0.0);
        // degenerate rectangle is enlarged by 1
        let s = transform_to_board_rel(&ShapeKind::Rect([0.0, 0.0, 10.0, 0.0]), &ct).unwrap();
        let p = padstack_shape(s).unwrap().unwrap();
        assert_eq!(p.dimension(), 2);
        assert_eq!(p.bounding_box(), IntBox::new(-1, -1, 11, 1));
        // concave polygon -> convex hull
        let s = transform_to_board_rel(
            &ShapeKind::Polygon(vec![0.0, 0.0, 10.0, 0.0, 5.0, 2.0, 10.0, 10.0, 0.0, 10.0]),
            &ct,
        )
        .unwrap();
        let p = padstack_shape(s).unwrap().unwrap();
        assert_eq!(p.bounding_box(), IntBox::new(0, 0, 10, 10));
        assert!(matches!(p, ConvexShape::Tile(TileShape::IntBox(_))));
        assert!(padstack_shape(None).is_err());
    }
}
