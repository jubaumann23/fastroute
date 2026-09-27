//! The library scope (Java `io.specctra.parser.Library.readScope`, `readPadstackScope`,
//! `Package.readScope`): padstacks and packages.

use fr_dsn::model::{Area as DsnArea, Image, ShapeKind};
use fr_engine::ids::LayerNo;
use fr_engine::library::{Keepout, PackagePin, Packages, Padstacks};
use fr_geom::{Area, ConvexShape, IntVector, Shape, Simplex, TileShape, Vector};

use crate::error::{npe, LResult, LoadError};
use crate::loader::{KeepoutKind, Loader};
use crate::shapes::{self, PLayer, PShape, ParserLayers};
use crate::structure::round_i32;

/// Java `name.replaceAll("\\.\\d+", "")`.
pub fn clean_padstack_name(name: &str) -> String {
    let b = name.as_bytes();
    let mut out = String::with_capacity(name.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'.' && i + 1 < b.len() && b[i + 1].is_ascii_digit() {
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            continue;
        }
        let ch = name[i..].chars().next().expect("char boundary");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Java `name.replaceAll("::\\d+$", "")` (`$` also matches before a final line terminator).
pub fn strip_package_suffix(name: &str) -> String {
    let terminators = ["\r\n", "\n", "\r", "\u{85}", "\u{2028}", "\u{2029}"];
    let (body, term) = terminators
        .iter()
        .find_map(|t| name.strip_suffix(t).map(|b| (b, *t)))
        .unwrap_or((name, ""));
    let strip = |s: &str| -> Option<String> {
        let digits = s.bytes().rev().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        s[..s.len() - digits].strip_suffix("::").map(str::to_string)
    };
    match strip(body) {
        Some(stripped) => format!("{stripped}{term}"),
        None => name.to_string(),
    }
}

/// A package keepout as read by `Shape.readAreaScope` (layers resolved).
struct PkgKeepout<'a> {
    name: Option<String>,
    shapes: Vec<Option<PShape<'a>>>,
}

/// Java `Library.arePackagePinsIdentical`.
fn are_package_pins_identical(existing: &[PackagePin], pins: &[PackagePin]) -> bool {
    if existing.len() != pins.len() {
        return false;
    }
    existing.iter().zip(pins).all(|(p1, p2)| {
        if p1.name != p2.name || p1.padstack_id != p2.padstack_id {
            return false;
        }
        let l1 = p1.relative_location.to_float();
        let l2 = p2.relative_location.to_float();
        if (l1.x - l2.x).abs() > 0.001 || (l1.y - l2.y).abs() > 0.001 {
            return false;
        }
        (p1.rotation_in_degree - p2.rotation_in_degree).abs() <= 0.001
    })
}

/// Java `Library.generateMissingKeepoutNames`.
fn generate_missing_keepout_names(prefix: &str, keepouts: &mut [PkgKeepout<'_>]) {
    if keepouts.iter().all(|k| k.name.is_some()) {
        return;
    }
    for (i, k) in keepouts.iter_mut().enumerate() {
        k.name = Some(format!("{prefix}{}", i + 1));
    }
}

fn empty_area() -> Area {
    Area::Shape(Shape::Tile(TileShape::Simplex(Simplex::empty())))
}

impl Loader<'_> {
    pub(crate) fn read_library(&mut self) -> LResult<()> {
        let dsn = self.dsn;
        let lib = &dsn.library;
        let pls_vec = self.parser_layers.clone();
        let pls = ParserLayers(&pls_vec);
        let board = self
            .board
            .as_mut()
            .ok_or_else(|| npe("Library.readScope: board is null"))?;
        let ct = board.transform;
        let layer_count = board.layer_count();
        let mut padstacks = Padstacks::new(&board.layer_structure);

        // Padstacks (Library.readPadstackScope).
        for p in &lib.padstacks {
            let name = clean_padstack_name(&p.name);
            let shape_list: Vec<PShape<'_>> = p
                .shapes
                .iter()
                .filter_map(|s| shapes::resolve(s, Some(pls)))
                .collect();
            if padstacks.get_by_name(&name).is_some() {
                continue;
            }
            if shape_list.is_empty() {
                log::warn!("Library.read_padstack_scope: shape not found for padstack '{name}'");
                continue;
            }
            let mut arr: Vec<Option<ConvexShape>> = vec![None; layer_count as usize];
            for ps in &shape_list {
                let board_shape = shapes::transform_to_board_rel(ps.kind, &ct)?;
                let convex = shapes::padstack_shape(board_shape)?;
                match &ps.layer {
                    Some(PLayer::Pcb) | Some(PLayer::Signal) => arr.fill(convex),
                    Some(PLayer::Named { name: lname, .. }) => {
                        let no = pls.get_no(lname);
                        if no < 0 || no >= layer_count {
                            return Err(LoadError::ParseError(
                                "Library.read_padstack_scope: layer not found".into(),
                            ));
                        }
                        arr[no as usize] = convex;
                    }
                    None => return Err(npe("Library.readPadstackScope: shape layer is null")),
                }
            }
            padstacks.add(name, arr, p.attach, p.absolute);
        }

        // Packages.
        let mut packages = Packages::new();
        for image in &lib.images {
            self_add_package(
                image,
                &pls,
                &ct,
                layer_count,
                &padstacks,
                &mut packages,
                board,
            )?;
        }
        board.library.padstacks = padstacks;
        board.library.packages = packages;
        board.library_read = true;
        Ok(())
    }
}

fn resolve_area<'a>(a: &'a DsnArea, pls: &ParserLayers<'_>) -> Option<PkgKeepout<'a>> {
    let shapes: Vec<Option<PShape<'a>>> = a
        .shapes
        .iter()
        .map(|s| shapes::resolve(s, Some(*pls)))
        .collect();
    // readAreaScope returns null if the border shape could not be read.
    shapes.first()?.as_ref()?;
    Some(PkgKeepout {
        name: a.name.clone(),
        shapes,
    })
}

#[allow(clippy::too_many_arguments)]
fn self_add_package(
    image: &Image,
    pls: &ParserLayers<'_>,
    ct: &fr_engine::structure::CoordinateTransform,
    _layer_count: LayerNo,
    padstacks: &Padstacks,
    packages: &mut Packages,
    board: &mut crate::loader::Board,
) -> LResult<()> {
    // Package.readScope
    let outline: Vec<PShape<'_>> = image
        .outlines
        .iter()
        .filter_map(|s| shapes::resolve(s, Some(*pls)))
        .collect();
    let mut kinds: [Vec<PkgKeepout<'_>>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for (k, list) in [&image.keepouts, &image.via_keepouts, &image.place_keepouts]
        .into_iter()
        .enumerate()
    {
        for a in list {
            match resolve_area(a, pls) {
                Some(r) => kinds[k].push(r),
                None => log::warn!(
                    "Package.read_scope: could not read keepout area of package '{}'",
                    image.name
                ),
            }
        }
    }

    // Library.readScope: create the board package.
    let mut pins = Vec::with_capacity(image.pins.len());
    for pin in &image.pins {
        let rel_x = round_i32(ct.dsn_to_board(pin.x));
        let rel_y = round_i32(ct.dsn_to_board(pin.y));
        let cleaned = clean_padstack_name(&pin.padstack);
        let Some(ps) = padstacks.get_by_name(&cleaned) else {
            return Err(LoadError::ParseError(format!(
                "Library.read_scope: board padstack '{}' not found",
                pin.padstack
            )));
        };
        pins.push(PackagePin::new(
            pin.name.clone(),
            ps.id,
            Vector::from(IntVector::new(rel_x, rel_y)),
            pin.rotation,
        ));
    }
    let mut outlines: Vec<Option<Shape>> = Vec::with_capacity(outline.len());
    let mut widths = Vec::with_capacity(outline.len());
    let mut closed = Vec::with_capacity(outline.len());
    for o in &outline {
        outlines.push(shapes::transform_to_board_rel(o.kind, ct)?);
        match o.kind {
            ShapeKind::PolygonPath { width, coords }
            | ShapeKind::PolylinePath { width, coords } => {
                widths.push(*width);
                let n = coords.len();
                closed.push(n >= 4 && coords[0] == coords[n - 2] && coords[1] == coords[n - 1]);
            }
            _ => {
                widths.push(0.0);
                closed.push(true);
            }
        }
    }
    generate_missing_keepout_names("keepout_", &mut kinds[0]);
    generate_missing_keepout_names("via_keepout_", &mut kinds[1]);
    generate_missing_keepout_names("place_keepout_", &mut kinds[2]);
    let mut keepout_arrs: [Vec<Keepout>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let mut null_areas: Vec<(KeepoutKind, usize)> = Vec::new();
    for (k, list) in kinds.iter().enumerate() {
        let kind = [
            KeepoutKind::Keepout,
            KeepoutKind::ViaKeepout,
            KeepoutKind::PlaceKeepout,
        ][k];
        for (i, ko) in list.iter().enumerate() {
            let layer = ko.shapes[0]
                .as_ref()
                .and_then(|s| s.layer.clone())
                .ok_or_else(|| npe("Library.readScope: keepout layer is null"))?;
            let area = shapes::transform_area(&ko.shapes, ct, true)?;
            let area = match area {
                Some(a) => a,
                None => {
                    null_areas.push((kind, i));
                    empty_area()
                }
            };
            keepout_arrs[k].push(Keepout::new(
                ko.name.clone().unwrap_or_default(),
                area,
                layer.no(),
            ));
        }
    }
    let [keepouts, via_keepouts, place_keepouts] = keepout_arrs;

    let base = strip_package_suffix(&image.name);
    let mut suffix = 0;
    loop {
        let test_name = if suffix == 0 {
            base.clone()
        } else {
            format!("{base}::{suffix}")
        };
        let (add, identical) = match packages.get_by_name(&test_name, image.is_front) {
            None => (true, false),
            Some(e) if fr_jcompat::compare_to_ignore_case(&e.name, &test_name) != 0 => {
                (true, false)
            }
            Some(e) => (false, are_package_pins_identical(e.pins(), &pins)),
        };
        if add {
            let id = packages.add(
                test_name,
                pins,
                Some(outlines),
                Some(widths),
                Some(closed),
                keepouts,
                via_keepouts,
                place_keepouts,
                image.is_front,
            );
            for (kind, i) in null_areas {
                board.null_package_keepouts.insert((id, kind, i));
            }
            break;
        }
        if identical {
            break;
        }
        suffix += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(clean_padstack_name("Via.1_600.25:400"), "Via_600:400");
        assert_eq!(clean_padstack_name("Round.Pad"), "Round.Pad");
        assert_eq!(clean_padstack_name("a.1.2."), "a.");
        assert_eq!(clean_padstack_name("ÄÖ.12x"), "ÄÖx");
        assert_eq!(strip_package_suffix("SOIC::3"), "SOIC");
        assert_eq!(strip_package_suffix("SOIC::"), "SOIC::");
        assert_eq!(strip_package_suffix("SOIC:3"), "SOIC:3");
    }
}
