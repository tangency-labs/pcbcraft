//! Assembly reference flashes, separate from the bare-board fabrication files.

use super::{Aperture, ApertureFunction, Fabrication, Gerber, Output, file_part};
use crate::{
    board::{
        Footprint,
        definition::{FootprintDefinition, basic_components},
        view::BoardView,
    },
    library::parts::{ManufacturerPart, Package},
    schematic::Point,
};
use std::{collections::BTreeMap, fmt::Write as _};

// Main flash diameter mandated by Ucamco Gerber rev. 2026.05 §5.6.10, p. 157.
const MAIN_FLASH_NM: i64 = 300_000;

impl BoardView<'_> {
    /// Render Gerber X3 assembly layers for the placed parts in this snapshot, top first.
    /// Requires a resolved origin, a copper stack, and unique references across both sides.
    /// Rotation is omitted until the library-to-IPC zero is known; empty sides have no file.
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
        Ok(Fabrication { origin, files })
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
    for ch in value.chars() {
        if matches!(ch, ',' | '%' | '*' | '\\') || ch.is_control() {
            let _ = write!(body, "\\u{:04X}", u32::from(ch));
        } else {
            body.push(ch);
        }
    }
    body.push_str("*%\n");
}
