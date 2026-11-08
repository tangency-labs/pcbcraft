//! Stable layer identities and a uniform rigid stackup, independent of editor appearance.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use super::materials::{self, Construction, Material};
use crate::{CommandError, Session, fresh_id};

#[cfg(test)]
mod tests;

mod presets;
pub mod profile;
pub use presets::StackupPreset;

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
    /// Build a validated candidate with one fresh technical/documentation layer. Copper must
    /// be changed with its physical stackup, so this builder refuses copper. The source setup
    /// is unchanged; callers adopt the candidate through revision-checked `pcb.set_board_setup`.
    pub fn with_added_technical_layer(&self, name: String, purpose: LayerPurpose, side: Option<Side>) -> Result<Self, CommandError> {
        if purpose == LayerPurpose::Copper {
            return Err(CommandError::InvalidParams("copper layers must be defined with the physical stackup".into()));
        }
        valid_text("layer name", &name).map_err(CommandError::InvalidParams)?;
        if self.layers.iter().any(|layer| layer.name.eq_ignore_ascii_case(&name)) {
            return Err(CommandError::InvalidParams(format!("duplicate layer name: {name}")));
        }
        let mut layers = self.layers.clone();
        layers.push(Layer { id: fresh_id()?, name, purpose, side });
        Self::try_from(SetupRecords { materials: self.materials.clone(), layers, stackup: self.stackup.clone() }).map_err(CommandError::InvalidParams)
    }

    /// Resize the physical copper stack while retaining surface IDs, surviving inner-layer IDs,
    /// finished dimensions and dielectric construction. Removing authored strata is refused;
    /// callers can clear their metadata deliberately before reduction. Adoption validates drawings.
    pub fn with_copper_count(&self, count: u64) -> Result<Self, CommandError> {
        let mut preset = Self::preset(count)?;
        let old: Vec<_> = self.stackup.strata.iter().step_by(2).cloned().collect();
        if old.len() == count as usize {
            return Ok(self.clone());
        }
        let retained_inner = (count as usize - 2).min(old.len() - 2);
        if old.iter().skip(retained_inner+1).take(old.len()-retained_inner-2).any(|s|s.thickness_nm().is_some())
            || self.stackup.strata.iter().skip(count as usize * 2 - 1).any(|s| matches!(s,Stratum::Dielectric{material,construction,thickness_nm,thickness_tolerance_nm,..} if material.is_some()||construction.is_some()||thickness_nm.is_some()||thickness_tolerance_nm.is_some())) {
            return Err(CommandError::InvalidParams("clear the removed copper/dielectric thickness and construction fields before reducing the layer count".into()));
        }
        for index in 0..count as usize {
            let previous = if index == 0 {
                old.first()
            } else if index + 1 == count as usize {
                old.last()
            } else if index <= retained_inner {
                old.get(index)
            } else {
                None
            };
            if let Some(previous) = previous {
                preset.stackup.strata[index * 2] = previous.clone();
            }
            if index * 2 + 1 < preset.stackup.strata.len()
                && let Some(gap) = self.stackup.strata.get(index * 2 + 1)
            {
                preset.stackup.strata[index * 2 + 1] = gap.clone();
            }
        }
        let ids: BTreeSet<_> =
            preset.stackup.strata.iter().filter_map(|s| if let Stratum::Copper { layer, .. } = s { Some(*layer) } else { None }).collect();
        let mut layers: Vec<_> = self.layers.iter().filter(|l| l.purpose != LayerPurpose::Copper || ids.contains(&l.id)).cloned().collect();
        for layer in preset.layers.into_iter().filter(|l| l.purpose == LayerPurpose::Copper) {
            if ids.contains(&layer.id) && !layers.iter().any(|l| l.id == layer.id) {
                layers.push(layer);
            }
        }
        Self::try_from(SetupRecords {
            materials: self.materials.clone(),
            layers,
            stackup: Stackup { id: self.stackup.id, strata: preset.stackup.strata },
        })
        .map_err(CommandError::InvalidParams)
    }

    /// Fit finished laminate thickness by dividing the exact residual among unlocked dielectric
    /// gaps. Copper and locked gaps must have specified thickness. Unknown/non-dielectric lock IDs,
    /// nonpositive residuals and resulting tolerance violations fail without changing the source.
    pub fn with_fitted_dielectrics(&self, target_nm: i64, locked: &BTreeSet<Uuid>) -> Result<Self, CommandError> {
        let invalid = |message: &str| CommandError::InvalidParams(message.into());
        if !(1..=MAX_THICKNESS_NM).contains(&target_nm) {
            return Err(invalid("target laminate thickness must be positive and at most 100 mm"));
        }
        if locked.iter().any(|id| !self.stackup.strata.iter().any(|s| matches!(s,Stratum::Dielectric{id:gap,..}if gap==id))) {
            return Err(invalid("thickness locks must identify dielectric gaps in this stackup"));
        }
        let mut fixed = 0_i64;
        let mut adjustable = Vec::new();
        for (i, stratum) in self.stackup.strata.iter().enumerate() {
            if matches!(stratum,Stratum::Dielectric{id,..}if !locked.contains(id)) {
                adjustable.push(i);
            } else {
                fixed = fixed
                    .checked_add(stratum.thickness_nm().ok_or_else(|| invalid("specify copper and locked dielectric thicknesses before fitting"))?)
                    .ok_or_else(|| invalid("thickness overflow"))?;
            }
        }
        if adjustable.is_empty() {
            return Err(invalid("unlock at least one dielectric gap"));
        }
        let remaining = target_nm.checked_sub(fixed).ok_or_else(|| invalid("target thickness is out of range"))?;
        if remaining < adjustable.len() as i64 {
            return Err(invalid("target thickness must exceed the fixed layers"));
        }
        let mut stackup = self.stackup.clone();
        for (n, i) in adjustable.iter().enumerate() {
            if let Stratum::Dielectric { thickness_nm, .. } = &mut stackup.strata[*i] {
                *thickness_nm = Some(remaining / adjustable.len() as i64 + i64::from((n as i64) < remaining % adjustable.len() as i64));
            }
        }
        Self::try_from(SetupRecords { materials: self.materials.clone(), layers: self.layers.clone(), stackup }).map_err(CommandError::InvalidParams)
    }

    /// Explicit pad-resolution consent permits membership changes only when retained layer IDs
    /// keep their roles/sides and both physical surface IDs survive. Drawing validation separately
    /// rejects removing occupied graphic layers; footprints retain their immutable definitions.
    fn compatible_bindings(&self, other: &Self) -> bool {
        self.layers
            .iter()
            .filter(|l| l.purpose == LayerPurpose::Copper && l.side.is_some())
            .all(|l| other.layers.iter().any(|n| n.id == l.id && n.purpose == l.purpose && n.side == l.side))
            && self.layers.iter().all(|l| other.layers.iter().find(|n| n.id == l.id).is_none_or(|n| n.purpose == l.purpose && n.side == l.side))
    }

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

    /// Build an unapplied initial setup, retaining existing technical-layer identities so old
    /// Board Edge drawings stay attached. The board and its history remain unchanged.
    pub(super) fn initial(board: &super::Board, copper_layers: u64) -> Result<Self, CommandError> {
        let mut setup = Self::preset(copper_layers)?;
        if let Some(data) = &board.drawings
            && !data.layers.is_empty()
        {
            setup.layers.retain(|layer| layer.purpose != LayerPurpose::Outline);
            setup.layers.extend(data.layers.iter().cloned());
            setup = Self::try_from(SetupRecords { materials: setup.materials, layers: setup.layers, stackup: setup.stackup })
                .map_err(CommandError::InvalidParams)?;
        }
        Ok(setup)
    }

    /// Initialize any supported even copper-layer count without inventing material or thickness.
    /// Entropy or validation failures return before the session adopts any data.
    pub(crate) fn preset(copper_layers: u64) -> Result<Self, CommandError> {
        if !(2..=MAX_COPPER as u64).contains(&copper_layers) || !copper_layers.is_multiple_of(2) {
            return Err(CommandError::InvalidParams("select an even copper-layer count from 2 through 64".to_owned()));
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
        layers.push(Layer { id: fresh_id()?, name: "Board Edge".to_owned(), purpose: LayerPurpose::Outline, side: None });
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
    /// Create the initial topology once. Populated boards require explicit opt-in to resolve
    /// existing ComponentSide/AllCopper bindings through the engine's front/all-copper rules;
    /// marker-only pads remain markers. Placements, definitions and mappings are unchanged.
    /// Existing technical drawing layers are promoted with their original identities. Optional
    /// silkscreen settings and a supplied complete setup draft are validated before publication;
    /// occupied drawing layers must remain valid in the adopted catalog.
    pub(crate) fn initialize_board_setup(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        if project.board.setup.is_some() {
            return Err(CommandError::Unavailable("board setup already exists; edit it with pcb.set_board_setup".to_owned()));
        }
        if !project.board.footprints().is_empty() && params["resolve_existing_footprints"].as_bool() != Some(true) {
            return Err(CommandError::Unavailable(
                "initial board setup with existing footprints requires resolve_existing_footprints=true".to_owned(),
            ));
        }
        let count = params["copper_layers"].as_u64().unwrap_or_default();
        let setup = if let Some(value) = params.get("setup") {
            let setup: BoardSetup = serde_json::from_value(value.clone()).map_err(|e| CommandError::InvalidParams(e.to_string()))?;
            if setup.stackup.strata.len().div_ceil(2) as u64 != count {
                return Err(CommandError::InvalidParams("setup must match the requested copper-layer count".into()));
            }
            setup
        } else {
            BoardSetup::initial(&project.board, count)?
        };
        let mut project = project.clone();
        let board = std::sync::Arc::make_mut(&mut project.board);
        board.setup = Some(setup);
        if let Some(settings) = params.get("silkscreen") {
            let settings = serde_json::from_value(settings.clone()).map_err(|e| CommandError::InvalidParams(e.to_string()))?;
            if board.silkscreen.unwrap_or_default() != settings {
                board.silkscreen = Some(settings);
            }
        }
        if let Some(data) = &mut std::sync::Arc::make_mut(&mut project.board).drawings {
            data.layers.clear();
        }
        if let Some(mut data) = project.board.drawings.clone() {
            data.validate(&project).map_err(CommandError::InvalidParams)?;
        }
        self.change(project);
        Ok(())
    }

    /// Validate the entire candidate before one atomic edit. Renames and thickness/material
    /// edits preserve identity. Populated topology changes require explicit consent and compatible
    /// surviving layer roles/surface IDs; occupied drawing-layer removals are always rejected.
    /// Catalog reordering alone is a no-op and does not advance revision or clear redo.
    /// Existing pinned material revisions cannot be rewritten; use a new revision and explicit references.
    /// Optional silkscreen settings join the same validated edit, retaining no-op history semantics.
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
        if !project.board.footprints().is_empty()
            && !before.same_topology(&setup)
            && !(params["resolve_existing_footprints"].as_bool() == Some(true) && before.compatible_bindings(&setup))
        {
            return Err(CommandError::Unavailable(
                "populated layer changes require resolve_existing_footprints=true and compatible retained surface identities and layer roles"
                    .to_owned(),
            ));
        }
        let silkscreen = params
            .get("silkscreen")
            .map(|settings| serde_json::from_value(settings.clone()).map_err(|e| CommandError::InvalidParams(e.to_string())))
            .transpose()?;
        let silk_changed = silkscreen.is_some_and(|settings| project.board.silkscreen.unwrap_or_default() != settings);
        if before != &setup || silk_changed {
            let mut project = project.clone();
            let board = std::sync::Arc::make_mut(&mut project.board);
            board.setup = Some(setup);
            if silk_changed {
                board.silkscreen = silkscreen;
            }
            if let Some(mut data) = project.board.drawings.clone() {
                data.validate(&project).map_err(CommandError::InvalidParams)?;
            }
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
