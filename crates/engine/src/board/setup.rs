//! Stable layer identities and a uniform rigid stackup, independent of editor appearance.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use super::materials::{self, Construction, Material};
use crate::{CommandError, Session, fresh_id};

#[cfg(test)]
mod tests;

/// Resource limits for the first format, not identities or fixed layer-number assignments.
const MAX_LAYERS: usize = 128;
const MAX_COPPER: usize = 64;
const MAX_NAME: usize = 80;
/// Maximum supported laminate thickness, in nanometers (100 mm).
const MAX_THICKNESS_NM: i64 = 100_000_000;

/// Which board surface a layer serves; internal copper has no surface side.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Front,
    Back,
}

/// A layer's physical or documentation purpose, independent of its display name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerPurpose {
    Copper,
    Mask,
    Paste,
    Silkscreen,
    Adhesive,
    Fabrication,
    Courtyard,
    Outline,
    Mechanical,
    User,
}

/// One stable layer record. Its order here does not determine physical stack order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    pub id: Uuid,
    pub name: String,
    pub purpose: LayerPurpose,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub side: Option<Side>,
}

/// Copper references a catalog layer; dielectric has its own identity, legacy descriptive label,
/// optional construction and finished thickness. Ply stock thickness is never added to this value.
/// Omitted thickness is unknown. Zero is never an unspecified-thickness sentinel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Stratum {
    Copper {
        layer: Uuid,
        #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
        thickness_nm: Option<i64>,
    },
    Dielectric {
        id: Uuid,
        #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
        material: Option<String>,
        #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
        construction: Option<Construction>,
        /// Symmetric tolerance on finished dielectric thickness; requires a specified thickness.
        #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
        thickness_tolerance_nm: Option<i64>,
        #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
        thickness_nm: Option<i64>,
    },
}

impl Stratum {
    /// Return the specified thickness, retaining unknown values as `None`.
    pub fn thickness_nm(&self) -> Option<i64> {
        match self {
            Self::Copper { thickness_nm, .. } | Self::Dielectric { thickness_nm, .. } => *thickness_nm,
        }
    }

    /// Compare only physical membership and order, excluding editable thickness/material data.
    fn identity(&self) -> (bool, Uuid) {
        match self {
            Self::Copper { layer, .. } => (true, *layer),
            Self::Dielectric { id, .. } => (false, *id),
        }
    }
}

/// One rigid stackup profile, ordered from front to back. Surface coatings are not included.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stackup {
    pub id: Uuid,
    pub strata: Vec<Stratum>,
}

/// Validated board configuration. Deserialization validates all cross-references and bounds,
/// then sorts the catalog by ID without changing the physical order of strata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SetupRecords")]
pub struct BoardSetup {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    materials: Vec<Material>,
    layers: Vec<Layer>,
    stackup: Stackup,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetupRecords {
    #[serde(default)]
    materials: Vec<Material>,
    layers: Vec<Layer>,
    stackup: Stackup,
}

impl TryFrom<SetupRecords> for BoardSetup {
    type Error = String;

    /// Refuse malformed or unsupported constructions before creating an adoptable setup.
    fn try_from(mut records: SetupRecords) -> Result<Self, Self::Error> {
        let material_keys = materials::validate_catalog(&mut records.materials)?;
        if !(2..=MAX_LAYERS).contains(&records.layers.len()) {
            return Err(format!("board setup requires 2 to {MAX_LAYERS} layers"));
        }
        let count = records.stackup.strata.len();
        if !(3..=2 * MAX_COPPER - 1).contains(&count) || count % 2 != 1 {
            return Err("rigid stackup requires alternating copper and dielectric strata, starting and ending with copper".to_owned());
        }
        let mut ids = BTreeSet::new();
        unique_id(&mut ids, records.stackup.id)?;
        let mut names = BTreeSet::new();
        for layer in &records.layers {
            unique_id(&mut ids, layer.id)?;
            valid_text("layer name", &layer.name)?;
            if !names.insert(layer.name.to_lowercase()) {
                return Err(format!("duplicate layer name: {}", layer.name));
            }
            match layer.purpose {
                LayerPurpose::Mask
                | LayerPurpose::Paste
                | LayerPurpose::Silkscreen
                | LayerPurpose::Adhesive
                | LayerPurpose::Fabrication
                | LayerPurpose::Courtyard
                    if layer.side.is_none() =>
                {
                    return Err(format!("layer {} needs an explicit front or back side", layer.name));
                }
                LayerPurpose::Outline if layer.side.is_some() => return Err("outline layers cannot have a surface side".to_owned()),
                _ => {}
            }
        }
        let mut copper = BTreeSet::new();
        let mut ply_count = 0;
        let mut known_thickness = 0_i64;
        for (index, stratum) in records.stackup.strata.iter().enumerate() {
            if let Some(thickness) = stratum.thickness_nm() {
                if !(1..=MAX_THICKNESS_NM).contains(&thickness) {
                    return Err("stratum thickness must be a positive whole number of nanometers, at most 100 mm".to_owned());
                }
                known_thickness += thickness;
            }
            match stratum {
                Stratum::Copper { layer, .. } if index % 2 == 0 => {
                    let record = records
                        .layers
                        .iter()
                        .find(|record| record.id == *layer && record.purpose == LayerPurpose::Copper)
                        .ok_or("stackup copper must reference a copper layer in the catalog")?;
                    if !copper.insert(*layer) {
                        return Err("a copper layer can occur only once in the stackup".to_owned());
                    }
                    let side = if index == 0 {
                        Some(Side::Front)
                    } else if index + 1 == count {
                        Some(Side::Back)
                    } else {
                        None
                    };
                    if record.side != side {
                        return Err("copper surface sides must agree with front-to-back physical stack order".to_owned());
                    }
                }
                Stratum::Dielectric { id, material, construction, thickness_nm, thickness_tolerance_nm } if index % 2 == 1 => {
                    unique_id(&mut ids, *id)?;
                    if let Some(material) = material {
                        valid_text("dielectric material", material)?;
                    }
                    if let Some(tolerance) = thickness_tolerance_nm
                        && (*tolerance < 0
                            || !thickness_nm.is_some_and(|thickness| *tolerance < thickness && *tolerance <= MAX_THICKNESS_NM - thickness))
                    {
                        return Err(
                            "dielectric tolerance requires a specified thickness with positive lower and at most 100 mm upper bounds".to_owned()
                        );
                    }
                    if let Some(construction) = construction {
                        construction.validate(&material_keys)?;
                        for id in construction.ply_ids() {
                            unique_id(&mut ids, id)?;
                            ply_count += 1;
                        }
                    }
                }
                _ => return Err("rigid stackup must alternate copper and dielectric strata".to_owned()),
            }
        }
        if known_thickness > MAX_THICKNESS_NM {
            return Err("specified stackup thickness exceeds 100 mm".to_owned());
        }
        if ply_count > materials::MAX_TOTAL_PLIES {
            return Err(format!("board setup supports at most {} prepreg plies", materials::MAX_TOTAL_PLIES));
        }
        if records.layers.iter().filter(|layer| layer.purpose == LayerPurpose::Copper).count() != copper.len() {
            return Err("every copper layer must occur in the stackup".to_owned());
        }
        records.layers.sort_unstable_by_key(|layer| layer.id);
        Ok(Self { materials: records.materials, layers: records.layers, stackup: records.stackup })
    }
}

impl BoardSetup {
    /// Embedded material revisions in canonical ID/revision order, available without a source library.
    pub fn materials(&self) -> &[Material] {
        &self.materials
    }

    /// Whether saving needs the material-aware board format instead of the legacy layer-only format.
    pub(crate) fn requires_material_format(&self) -> bool {
        !self.materials.is_empty()
            || self.stackup.strata.iter().any(|stratum| {
                matches!(stratum, Stratum::Dielectric { construction: Some(_), .. } | Stratum::Dielectric { thickness_tolerance_nm: Some(_), .. })
            })
    }

    /// The layer catalog in stable ID order, independent of names and stack order.
    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    /// The current uniform rigid construction, with physical order retained.
    pub fn stackup(&self) -> &Stackup {
        &self.stackup
    }

    /// Sum copper and dielectric thickness only when all constituents are specified.
    /// This is laminate thickness, not a finished-board or manufacturing guarantee.
    pub fn laminate_thickness_nm(&self) -> Option<i64> {
        self.stackup.strata.iter().map(Stratum::thickness_nm).sum()
    }

    /// Initialize a two- or four-copper-layer topology with no invented material or thickness.
    /// Entropy or validation failures return before the session adopts any data.
    pub(crate) fn preset(copper_layers: u64) -> Result<Self, CommandError> {
        if !matches!(copper_layers, 2 | 4) {
            return Err(CommandError::InvalidParams("the initial presets support 2 or 4 copper layers".to_owned()));
        }
        let mut layers = Vec::new();
        let mut strata = Vec::new();
        for index in 0..copper_layers {
            let (name, side) = if index == 0 {
                ("Top".to_owned(), Some(Side::Front))
            } else if index + 1 == copper_layers {
                ("Bottom".to_owned(), Some(Side::Back))
            } else {
                (format!("Inner {index}"), None)
            };
            let id = fresh_id()?;
            layers.push(Layer { id, name, purpose: LayerPurpose::Copper, side });
            strata.push(Stratum::Copper { layer: id, thickness_nm: None });
            if index + 1 < copper_layers {
                strata.push(Stratum::Dielectric {
                    id: fresh_id()?,
                    material: None,
                    construction: None,
                    thickness_tolerance_nm: None,
                    thickness_nm: None,
                });
            }
        }
        for (side, prefix) in [(Side::Front, "Front"), (Side::Back, "Back")] {
            for (purpose, name) in [
                (LayerPurpose::Mask, "mask"),
                (LayerPurpose::Paste, "paste"),
                (LayerPurpose::Silkscreen, "silkscreen"),
                (LayerPurpose::Fabrication, "fabrication"),
                (LayerPurpose::Courtyard, "courtyard"),
            ] {
                layers.push(Layer { id: fresh_id()?, name: format!("{prefix} {name}"), purpose, side: Some(side) });
            }
        }
        layers.push(Layer { id: fresh_id()?, name: "Board outline".to_owned(), purpose: LayerPurpose::Outline, side: None });
        Self::try_from(SetupRecords { materials: Vec::new(), layers, stackup: Stackup { id: fresh_id()?, strata } })
            .map_err(CommandError::InvalidParams)
    }

    /// Require identical layer roles and stratum membership for edits on today's populated board.
    /// General pad bindings and explicit layer reassignment have not been implemented yet.
    pub(crate) fn same_topology(&self, other: &Self) -> bool {
        self.layers
            .iter()
            .map(|layer| (layer.id, layer.purpose, layer.side))
            .eq(other.layers.iter().map(|layer| (layer.id, layer.purpose, layer.side)))
            && self.stackup.strata.iter().map(Stratum::identity).eq(other.stackup.strata.iter().map(Stratum::identity))
    }

    /// Describe canonical, bounded command inputs. Unknown fields and explicit nulls are rejected.
    pub(crate) fn schema() -> Value {
        let id = json!({"type":"string", "format":"uuid"});
        let name = json!({"type":"string", "minLength":1, "maxLength":MAX_NAME});
        let thickness = json!({"type":"integer", "minimum":1, "maximum":MAX_THICKNESS_NM});
        json!({"type":"object", "additionalProperties":false, "required":["layers","stackup"], "properties": {
            "materials":materials::schema::catalog(),
            "layers":{"type":"array", "minItems":2, "maxItems":MAX_LAYERS, "items":{
                "type":"object", "additionalProperties":false, "required":["id","name","purpose"], "properties":{
                    "id":id, "name":name, "purpose":{"type":"string", "enum":["copper","mask","paste","silkscreen","adhesive","fabrication","courtyard","outline","mechanical","user"]},
                    "side":{"type":"string", "enum":["front","back"]}
                }
            }},
            "stackup":{"type":"object", "additionalProperties":false, "required":["id","strata"], "properties":{
                "id":id, "strata":{"type":"array", "minItems":3, "maxItems":2*MAX_COPPER-1, "items":{"oneOf":[
                    {"type":"object", "additionalProperties":false, "required":["kind","layer"], "properties":{
                        "kind":{"const":"copper"}, "layer":id, "thickness_nm":thickness
                    }},
                    {"type":"object", "additionalProperties":false, "required":["kind","id"], "properties":{
                        "kind":{"const":"dielectric"}, "id":id, "material":name, "thickness_nm":thickness,
                        "construction":materials::schema::construction(),
                        "thickness_tolerance_nm":{"type":"integer","minimum":0,"maximum":MAX_THICKNESS_NM}
                    }}
                ]}}
            }}
        }})
    }
}

impl Session {
    /// Create the initial topology once on an empty board. Existing legacy footprints require
    /// an explicit future layer-binding migration instead of an inferred reassignment.
    pub(crate) fn initialize_board_setup(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        if project.board.setup.is_some() {
            return Err(CommandError::Unavailable("board setup already exists; edit it with pcb.set_board_setup".to_owned()));
        }
        if !project.board.footprints().is_empty() {
            return Err(CommandError::Unavailable(
                "initial board setup requires an empty board; migrating existing footprint layer bindings is not supported yet".to_owned(),
            ));
        }
        let setup = BoardSetup::preset(params["copper_layers"].as_u64().unwrap_or_default())?;
        let mut project = project.clone();
        std::sync::Arc::make_mut(&mut project.board).setup = Some(setup);
        self.change(project);
        Ok(())
    }

    /// Validate the entire candidate before one atomic edit. Renames and thickness/material
    /// edits preserve identity; populated boards reject structural changes until bindings exist.
    /// Catalog reordering alone is a no-op and does not advance revision or clear redo.
    /// Existing pinned material revisions cannot be rewritten; use a new revision and explicit references.
    pub(crate) fn set_board_setup(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let setup: BoardSetup = serde_json::from_value(params["setup"].clone()).map_err(|error| CommandError::InvalidParams(error.to_string()))?;
        let project = self.open_project()?;
        let before = project.board.setup.as_ref().ok_or_else(|| CommandError::Unavailable("initialize board setup first".to_owned()))?;
        if before.stackup.id != setup.stackup.id {
            return Err(CommandError::InvalidParams("editing board setup must retain its stackup profile ID".to_owned()));
        }
        for material in &setup.materials {
            if before.materials.iter().any(|old| old.reference() == material.reference() && old != material) {
                return Err(CommandError::InvalidParams(
                    "changing a pinned material requires a new revision and explicit reference updates".to_owned(),
                ));
            }
        }
        if !project.board.footprints().is_empty() && !before.same_topology(&setup) {
            return Err(CommandError::Unavailable(
                "changing a populated board's layers requires explicit footprint layer reassignment, which is not supported yet".to_owned(),
            ));
        }
        if before != &setup {
            let mut project = project.clone();
            std::sync::Arc::make_mut(&mut project.board).setup = Some(setup);
            self.change(project);
        }
        Ok(())
    }
}

/// Keep IDs unique across layers, dielectrics, prepreg plies and the profile; nil is never authored.
fn unique_id(ids: &mut BTreeSet<Uuid>, id: Uuid) -> Result<(), String> {
    if id.is_nil() || !ids.insert(id) { Err("board setup IDs must be non-nil and unique".to_owned()) } else { Ok(()) }
}

/// Bound human labels without accepting blank, padded or control-character names.
fn valid_text(field: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || value.trim() != value || value.chars().count() > MAX_NAME || value.chars().any(char::is_control) {
        Err(format!("{field} must contain 1 to {MAX_NAME} characters without surrounding whitespace or control characters"))
    } else {
        Ok(())
    }
}

/// An optional field may be absent, but an explicit null is not a valid physical value.
fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(deserializer: D) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}
