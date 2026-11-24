//! Gerber X3 assembly objects, separate from the bare-board fabrication files.

use super::{Aperture, ApertureFunction, DrillChart, Fabrication, Gerber, Output, file_part, xy};
use crate::{
    board::{
        Footprint,
        definition::{FootprintDefinition, GraphicPurpose, basic_components},
        view::{BoardView, rectangle_points},
        zones::validate_outline,
    },
    library::parts::{ManufacturerPart, Package},
    library_graphics::Geometry,
    schematic::Point,
};
use std::{collections::BTreeMap, fmt::Write as _};

// Main flash diameter mandated by Ucamco Gerber rev. 2026.05 §5.6.10, p. 157.
const MAIN_FLASH_NM: i64 = 300_000;
// Ucamco Gerber rev. 2026.05 §5.6.10, pp. 157–158: outline stroke, pin circle,
// and key-pin four-vertex polygon (diameter of the circumscribed circle).
const OUTLINE_STROKE_NM: i64 = 100_000;
const PIN_FLASH_NM: i64 = 100_000;
const KEY_PIN_FLASH_NM: i64 = 360_000;

impl BoardView<'_> {
    /// Render Gerber X3 assembly layers for the placed parts in this snapshot, top first.
    /// Requires a resolved origin, a copper stack, and unique references across both sides.
    /// Rotation is omitted until the library-to-IPC zero is known; empty sides have no file.
    /// Pinned footprint definitions contribute numbered lands and closed assembly outlines.
    /// This is independent of bare-board fabrication and does not apply assembly variants.
    pub fn gerber_part_layers(self) -> Result<Fabrication, String> {
        let setup = self.setup().ok_or("Initialize the board setup before generating Gerber assembly layers.")?;
        let copper_count = setup.copper_stack().count();
        let origin = self.placement_origin()?;
        let frame = origin.frame();
        let project = self.project();
        let mut writers = [Gerber::new(frame), Gerber::new(frame)];
        let mut references = BTreeMap::new();
        let known_smd = basic_components("resistor-0402");
        for placement in project.board.footprints() {
            let part = project.components.get(&placement.component);
            let historical = project.board.baseline.as_ref().and_then(|board| board.components.get(&placement.component));
            let (reference, value) = if let Some(part) = part {
                (part.reference(), part.display_value())
            } else if let Some(historical) = historical {
                (historical.reference.as_str(), historical.display_value())
            } else {
                return Err(format!("Placement {} has no part reference.", placement.id));
            };
            if let Some(first) = references.insert(reference, placement.id) {
                return Err(format!("Duplicate Gerber reference {reference:?} on placements footprint-{first} and footprint-{}.", placement.id));
            }
            let definition = project.board.definition_for(placement);
            let (package, manufacturer) = package_metadata(placement, part.and_then(|part| part.part.as_ref()));
            let at = frame.to_output(self.assembly_centroid(placement));
            // Ucamco rev. 2026.05 §5.6.16 defines an IPC-7351B zero, not the library zero.
            // The pinned footprint has no such datum, so don't emit .CRot.
            let writer = &mut writers[usize::from(placement.bottom)];
            writer.aperture(Aperture::Circle(MAIN_FLASH_NM), Some(ApertureFunction::PartMain));
            let body = &mut writer.body;
            attribute(body, ".C", reference);
            if let Some(manufacturer) = manufacturer {
                attribute(body, ".CMfr", &manufacturer.manufacturer);
                attribute(body, ".CMPN", &manufacturer.mpn);
            }
            attribute(body, ".CVal", &value);
            if let Some(definition) = definition {
                attribute(body, ".CFtp", &definition.name);
                if let Some(mount) = mount_type(definition, known_smd.as_deref()) {
                    attribute(body, ".CMnt", mount);
                }
            }
            if let Some(package) = package {
                attribute(body, ".CPgN", &package.name);
            }
            let _ = writeln!(body, "X{}Y{}D03*", at.x_nm, at.y_nm);
            // Clear the object dictionary; a subsequent flash may omit optional attributes.
            body.push_str("%TD*%\n");
            if let Some(definition) = definition {
                part_pins(writer, placement, definition, reference);
                part_outlines(writer, placement, definition, reference);
            }
        }
        let base = file_part(&project.name);
        // Ucamco Gerber rev. 2026.05 §5.6.3 and §6.9: physical outer copper layer numbers.
        let files = writers
            .into_iter()
            .zip([(1, "Top", "Top"), (copper_count, "Bot", "Bottom")])
            .filter(|(writer, _)| !writer.body.is_empty())
            .map(|(writer, (layer, side, label))| Output {
                name: format!("{base}-Assembly_{label}.gbr"),
                contents: writer.finish_in_frame(&format!("Component,L{layer},{side}"), "Positive", None),
            })
            .collect();
        // Assembly layers carry no drill files and no board edge, so they have no drill tools
        // and no outline view.
        Ok(Fabrication { origin, files, drill_chart: DrillChart::default(), outline: None })
    }

    /// The Gerber export does not select a vendor profile. Use an explicit centroid from
    /// the board's adopted revisions only when their supplied centroids agree.
    fn assembly_centroid(self, placement: &Footprint) -> Point {
        let origin = Point { x_nm: placement.x_nm, y_nm: placement.y_nm };
        let mut centroids = self
            .assembly_profiles()
            .iter()
            .filter_map(|adoption| self.project().assembly_profiles().get(adoption)?.correction_for(placement)?.centroid);
        match centroids.next() {
            Some(centroid) if centroids.all(|other| other == centroid) => placement.point(centroid),
            _ => origin,
        }
    }
}

/// Flash each physical land once at its placed center. Use the package's pad label, not
/// the possibly different schematic terminal ID. Prefer `1`, then `A1`, without guessing
/// a key from pad order. See Ucamco Gerber rev. 2026.05 §§5.6.10, 5.6.14 and 6.9.
fn part_pins(writer: &mut Gerber, placement: &Footprint, definition: &FootprintDefinition, reference: &str) {
    let key = if definition.pads.iter().any(|pad| pad.number == "1") { "1" } else { "A1" };
    for pad in &definition.pads {
        let aperture = if pad.number == key { Aperture::Diamond(KEY_PIN_FLASH_NM) } else { Aperture::Circle(PIN_FLASH_NM) };
        writer.aperture(aperture, Some(ApertureFunction::PartPin));
        writer.body.push_str("%TO.P,");
        attribute_field(&mut writer.body, reference);
        // Pinned pad identifiers are validated ASCII and cannot contain Gerber delimiters.
        let _ = writeln!(writer.body, ",{}*%", pad.number);
        let _ = writeln!(writer.body, "{}D03*", xy(writer.frame, placement.point(pad.center)));
        writer.body.push_str("%TD*%\n");
    }
}

/// Export separate authored Body and Courtyard contours, not a bounding box or silkscreen.
/// Open artwork and lettering are not outlines; never close them by inventing an edge.
fn part_outlines(writer: &mut Gerber, placement: &Footprint, definition: &FootprintDefinition, reference: &str) {
    for rect in &definition.graphics {
        if let Some(function) = outline_function(rect.purpose) {
            let points = rectangle_points(placement, rect.center, rect.width_nm, rect.height_nm, rect.rotation);
            polygon_outline(writer, placement, &points, function, reference);
        }
    }
    for artwork in &definition.artwork {
        let Some(function) = outline_function(artwork.purpose) else { continue };
        match &artwork.graphic.geometry {
            Geometry::Rectangle { min, max } => {
                let points = [*min, Point { x_nm: max.x_nm, y_nm: min.y_nm }, *max, Point { x_nm: min.x_nm, y_nm: max.y_nm }];
                polygon_outline(writer, placement, &points.map(|p| placement.point(p)), function, reference);
            }
            Geometry::Polyline { points, closed } if *closed || points.first() == points.last() => {
                let points: Vec<_> = points.iter().map(|&p| placement.point(p)).collect();
                polygon_outline(writer, placement, &points, function, reference);
            }
            Geometry::Circle { center, radius_nm } | Geometry::Arc { center, radius_nm, sweep_degrees: -360 | 360, .. } => {
                circle_outline(writer, placement.point(*center), *radius_nm, function, reference);
            }
            _ => {}
        }
    }
}

/// Only fabrication and courtyard artwork can supply X3 assembly outlines.
fn outline_function(purpose: GraphicPurpose) -> Option<ApertureFunction> {
    match purpose {
        GraphicPurpose::Fabrication => Some(ApertureFunction::PartBody),
        GraphicPurpose::Courtyard => Some(ApertureFunction::PartCourtyard),
        GraphicPurpose::Silkscreen => None,
    }
}

/// Write a closed, simple path in its authored order; skip collapsed or self-crossing paths.
/// Validate after placement rounding, relative to the footprint origin so board-edge placements
/// do not inherit zone-coordinate bounds. Ucamco Gerber rev. 2026.05 §5.6.10, p. 157.
fn polygon_outline(writer: &mut Gerber, placement: &Footprint, points: &[Point], function: ApertureFunction, reference: &str) {
    let Some((first, rest)) = points.split_first() else { return };
    let rest = rest.strip_suffix(&[*first]).unwrap_or(rest);
    let relative: Vec<_> =
        std::iter::once(first).chain(rest).map(|p| Point { x_nm: p.x_nm - placement.x_nm, y_nm: p.y_nm - placement.y_nm }).collect();
    if validate_outline(&relative).is_err() {
        return;
    }
    writer.aperture(Aperture::Circle(OUTLINE_STROKE_NM), Some(function));
    attribute(&mut writer.body, ".C", reference);
    let _ = writeln!(writer.body, "{}D02*", xy(writer.frame, *first));
    for &point in rest.iter().chain(std::iter::once(first)) {
        let _ = writeln!(writer.body, "{}D01*", xy(writer.frame, point));
    }
    writer.body.push_str("%TD*%\n");
}

/// Preserve a circular outline as one full arc, not a tessellated path. Restore linear
/// plotting for later objects. See Ucamco Gerber rev. 2026.05 §§4.7.2 and 5.6.10.
fn circle_outline(writer: &mut Gerber, center: Point, radius_nm: i64, function: ApertureFunction, reference: &str) {
    writer.aperture(Aperture::Circle(OUTLINE_STROKE_NM), Some(function));
    attribute(&mut writer.body, ".C", reference);
    let right = xy(writer.frame, Point { x_nm: center.x_nm + radius_nm, y_nm: center.y_nm });
    let _ = writeln!(writer.body, "{right}D02*\nG75*\nG03*\n{right}I{}J0D01*\nG01*", -radius_nm);
    writer.body.push_str("%TD*%\n");
}

/// Match the placed footprint before emitting package metadata. Orderable fields also require
/// the native compatibility verdict: retained selections may no longer meet the requirements.
fn package_metadata<'a>(
    placement: &Footprint,
    selection: Option<&'a crate::part_instances::Selection>,
) -> (Option<&'a Package>, Option<&'a ManufacturerPart>) {
    let Some(selection) = selection else { return (None, None) };
    let manufacturer = selection.manufacturer.as_deref();
    let package_id = selection.package.as_deref().or_else(|| manufacturer.map(|part| part.package.as_str()));
    let package = package_id.and_then(|id| selection.resolved.definition.packages.get(id)).filter(|package| {
        placement.library.as_deref() == Some(package.footprint.library.as_str())
            && placement.definition == package.footprint.id
            && placement.definition_revision == package.footprint.revision
    });
    (package, manufacturer.filter(|_| package.is_some() && selection.compatibility()["status"] == "meets_requirements"))
}

/// The built-in 0402 reflow land pattern documents its mounting process. Match its complete
/// pinned definition, not a name or pad geometry that another source could reuse. Other footprints
/// have no mount-process field; plated lands alone cannot distinguish TH from press-fit.
fn mount_type(definition: &FootprintDefinition, known_smd: Option<&FootprintDefinition>) -> Option<&'static str> {
    (known_smd == Some(definition)).then_some("SMD")
}

/// Write one nonempty object field. Escape Gerber delimiters and backslashes per rev. 2026.05
/// §§3.4.3–3.4.4, plus controls so each command remains on one physical line.
fn attribute(body: &mut String, name: &str, value: &str) {
    if value.is_empty() {
        return;
    }
    let _ = write!(body, "%TO{name},");
    attribute_field(body, value);
    body.push_str("*%\n");
}

/// Append one field without its delimiter, using the same escaping for `.C` and `.P`.
fn attribute_field(body: &mut String, value: &str) {
    for ch in value.chars() {
        if matches!(ch, ',' | '%' | '*' | '\\') || ch.is_control() {
            let _ = write!(body, "\\u{:04X}", u32::from(ch));
        } else {
            body.push(ch);
        }
    }
}
