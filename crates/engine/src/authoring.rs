//! Native library-item documents own validation and history independently of projects.
pub mod ball_grid;
#[cfg(test)]
mod basics_tests;
pub mod courtyard;
mod operations;
pub mod publishing;
#[cfg(test)]
mod publishing_tests;
pub mod remap;
use crate::{
    board::definition as footprint,
    library,
    schematic::{ElectricalType, Point, Shape, SymbolDefinition, TextAlign},
};
pub use ball_grid::{JEDEC_SKIP_LETTERS, generate_identifiers, natural_cmp, natural_sort, row_label};
pub use operations::next_number;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

/// Maximum editor document size, including metadata and stable primitive identities.
pub const MAX_BYTES: usize = 1 << 20;
const MAX_SYMBOL_OBJECTS: usize = 512;
/// Maximum primitives in one footprint drawing; no document kind accepts more, so it also bounds object-ID lists.
pub const MAX_FOOTPRINT_OBJECTS: usize = footprint::MAX_PADS + footprint::MAX_ARTWORK + 256;
const _: () = assert!(MAX_FOOTPRINT_OBJECTS >= MAX_SYMBOL_OBJECTS);
const FORMAT: &str = "pcbcraft-library-item/1";
const MULTI_UNIT_FORMAT: &str = "pcbcraft-library-item/2";
const NPTH_FORMAT: &str = "pcbcraft-library-item/3";
const SLOT_FORMAT: &str = "pcbcraft-library-item/4";
const PIN_FUNCTION_FORMAT: &str = "pcbcraft-library-item/5";
const HISTORY: usize = 64;

/// A definition editor is distinct from schematic-sheet and board-placement editors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Symbol,
    Footprint,
}

/// Optional definition metadata uses the same authoring transaction as name and Value edits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolSettings {
    pub behavior: Option<library::Behavior>,
    pub defaults: Option<library::Defaults>,
    /// Omission preserves the binding; an explicit null clears it.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "present_category")]
    pub category: Option<Option<library::parts::CategoryBinding>>,
}

/// Preserve the distinction between an omitted category update and an explicit null.
fn present_category<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Option<library::parts::CategoryBinding>>, D::Error> {
    Option::deserialize(deserializer).map(Some)
}

/// Generated reference settings owned by the footprint definition, independent of board instances.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FootprintReferenceSettings {
    pub enabled: bool,
    pub offset: Point,
}

/// Primitive identity survives renumbering; terminal/pad numbers remain electrical metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Primitive {
    Pin {
        number: String,
        name: Option<String>,
        electrical_type: ElectricalType,
        #[serde(default, skip_serializing_if = "Vec::is_empty", deserialize_with = "library::authored_alternates")]
        alternates: Vec<crate::schematic::AlternatePinFunction>,
        at: Point,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        appearance: Option<crate::schematic::PinAppearance>,
    },
    Graphic {
        graphic: crate::library_graphics::Graphic,
    },
    Artwork {
        artwork: footprint::Artwork,
    },
    Line {
        from: Point,
        to: Point,
    },
    Rectangle {
        min: Point,
        max: Point,
    },
    Circle {
        center: Point,
        radius_nm: i64,
    },
    Text {
        at: Point,
        text: String,
        align: TextAlign,
    },
    Polyline {
        points: Vec<Point>,
    },
    Pad {
        pad: footprint::RectPad,
    },
    NonPlatedHole {
        hole: footprint::CircularNpth,
    },
    Outline {
        graphic: footprint::GraphicRect,
    },
    Silkscreen {
        path: footprint::SilkPath,
    },
}

impl Primitive {
    /// The grab anchor is independent of electrical numbering and graphical list order.
    pub fn anchor(&self) -> Point {
        match self {
            Self::Graphic { graphic } => graphic.anchor(),
            Self::Artwork { artwork } => artwork.graphic.anchor(),
            Self::Pin { at, .. } | Self::Text { at, .. } => *at,
            Self::Line { from, .. } => *from,
            Self::Rectangle { min, max } => Point {
                x_nm: ((i128::from(min.x_nm) + i128::from(max.x_nm)) / 2) as i64,
                y_nm: ((i128::from(min.y_nm) + i128::from(max.y_nm)) / 2) as i64,
            },
            Self::Circle { center, .. } => *center,
            Self::Polyline { points } => points.first().copied().unwrap_or(Point { x_nm: 0, y_nm: 0 }),
            Self::Pad { pad } => pad.center,
            Self::NonPlatedHole { hole } => hole.center,
            Self::Outline { graphic } => graphic.center,
            Self::Silkscreen { path } => path.points.first().copied().unwrap_or(Point { x_nm: 0, y_nm: 0 }),
        }
    }

    /// Translate one primitive without changing its number or orientation. Arithmetic overflow
    /// is rejected before a command reaches native geometry validation.
    pub fn translated(&self, delta: Point) -> Result<Self, String> {
        let at = |p: Point| {
            Ok::<_, String>(Point {
                x_nm: p.x_nm.checked_add(delta.x_nm).ok_or("X coordinate overflow.")?,
                y_nm: p.y_nm.checked_add(delta.y_nm).ok_or("Y coordinate overflow.")?,
            })
        };
        Ok(match self {
            Self::Graphic { graphic } => Self::Graphic {
                graphic: graphic.transformed(crate::library_graphics::Transform {
                    pivot: Point { x_nm: 0, y_nm: 0 },
                    offset: delta,
                    quarter_turns: 0,
                    mirror_x: false,
                    mirror_y: false,
                })?,
            },
            Self::Artwork { artwork } => Self::Artwork {
                artwork: footprint::Artwork {
                    graphic: artwork.graphic.transformed(crate::library_graphics::Transform {
                        pivot: Point { x_nm: 0, y_nm: 0 },
                        offset: delta,
                        quarter_turns: 0,
                        mirror_x: false,
                        mirror_y: false,
                    })?,
                    purpose: artwork.purpose,
                },
            },
            Self::Pin { number, name, electrical_type, alternates, at: p, appearance } => Self::Pin {
                number: number.clone(),
                name: name.clone(),
                electrical_type: *electrical_type,
                alternates: alternates.clone(),
                at: at(*p)?,
                appearance: appearance.clone(),
            },
            Self::Line { from, to } => Self::Line { from: at(*from)?, to: at(*to)? },
            Self::Rectangle { min, max } => Self::Rectangle { min: at(*min)?, max: at(*max)? },
            Self::Circle { center, radius_nm } => Self::Circle { center: at(*center)?, radius_nm: *radius_nm },
            Self::Text { at: p, text, align } => Self::Text { at: at(*p)?, text: text.clone(), align: *align },
            Self::Polyline { points } => Self::Polyline { points: points.iter().map(|p| at(*p)).collect::<Result<_, _>>()? },
            Self::Pad { pad } => {
                let mut pad = pad.clone();
                pad.center = at(pad.center)?;
                Self::Pad { pad }
            }
            Self::NonPlatedHole { hole } => Self::NonPlatedHole { hole: footprint::CircularNpth { center: at(hole.center)?, ..*hole } },
            Self::Outline { graphic } => {
                let mut graphic = graphic.clone();
                graphic.center = at(graphic.center)?;
                Self::Outline { graphic }
            }
            Self::Silkscreen { path } => {
                let mut path = path.clone();
                path.points = path.points.iter().map(|p| at(*p)).collect::<Result<_, _>>()?;
                Self::Silkscreen { path }
            }
        })
    }
}

/// An original item or explicit fork. Descriptive provenance never grants source write permission.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    format: String,
    pub kind: Kind,
    pub library: String,
    pub id: String,
    pub revision: u32,
    pub name: String,
    pub license: String,
    pub source: String,
    pub reference_prefix: String,
    pub default_value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behavior: Option<library::Behavior>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defaults: Option<library::Defaults>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<library::parts::CategoryBinding>,
    pub unit: String,
    /// Native drawing order, whose first entry remains the default independently of the active editor unit.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unit_order: Vec<String>,
    /// Inactive symbol units retain their stable primitive IDs and complete editable geometry.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty", deserialize_with = "unique_units")]
    pub other_units: BTreeMap<String, BTreeMap<u64, Primitive>>,
    pub reference_font: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_offset: Option<Point>,
    /// Read-only provenance from a forked footprint, independent of its editable source summary.
    pub original_source: Option<footprint::Source>,
    allocation: Allocation,
    #[serde(deserialize_with = "unique_objects")]
    pub objects: BTreeMap<u64, Primitive>,
}

/// Persist allocation watermarks outside undo so save/reopen cannot reuse deleted object IDs
/// or content revisions that were already observed by an exported definition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Allocation {
    object: u64,
    revision: u32,
}

/// Reject duplicate or aliased object keys before a decoder could silently discard geometry.
fn unique_objects<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<BTreeMap<u64, Primitive>, D::Error> {
    struct Objects;
    impl<'de> serde::de::Visitor<'de> for Objects {
        type Value = BTreeMap<u64, Primitive>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("unique canonical positive primitive IDs")
        }
        /// Validate every object key before accepting its associated primitive.
        fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            use serde::de::Error as _;
            let mut objects = BTreeMap::new();
            while let Some(key) = map.next_key::<String>()? {
                let id = key.parse::<u64>().map_err(A::Error::custom)?;
                if id == 0 || id.to_string() != key || objects.len() >= MAX_FOOTPRINT_OBJECTS || objects.contains_key(&id) {
                    return Err(A::Error::custom("Duplicate, noncanonical or excessive primitive ID."));
                }
                objects.insert(id, map.next_value()?);
            }
            Ok(objects)
        }
    }
    deserializer.deserialize_map(Objects)
}

/// Decode inactive drawings without discarding duplicate unit names or primitive identities.
fn unique_units<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<BTreeMap<String, BTreeMap<u64, Primitive>>, D::Error> {
    #[derive(Deserialize)]
    #[serde(transparent)]
    struct Objects(#[serde(deserialize_with = "unique_objects")] BTreeMap<u64, Primitive>);
    struct Units;
    impl<'de> serde::de::Visitor<'de> for Units {
        type Value = BTreeMap<String, BTreeMap<u64, Primitive>>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("unique bounded inactive symbol units")
        }
        /// Validate names and the bounded primitive maps before retaining each drawing.
        fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            use serde::de::Error as _;
            let mut units = BTreeMap::new();
            while let Some(key) = map.next_key::<String>()? {
                if !footprint::valid_id(&key) || units.len() >= 31 || units.contains_key(&key) {
                    return Err(A::Error::custom("Duplicate, invalid or excessive unit identity."));
                }
                units.insert(key, map.next_value::<Objects>()?.0);
            }
            Ok(units)
        }
    }
    deserializer.deserialize_map(Units)
}

/// Default JEDEC skip-letter flag for ball-grid array numbering.
fn default_true() -> bool {
    true
}

/// Numbering policy for electrical primitives created by [`Action::Array`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArrayNumbering {
    /// Increment trailing digits while keeping any existing prefix (`next_number`).
    Sequential,
    /// Ball-grid array numbering: row letters (`A..Z`, then `AA..`) and 1-based column numbers.
    BallGrid {
        /// Omit JEDEC skip letters (`I`, `O`, `Q`, `S`, `X`, `Z`) from row labels.
        #[serde(default = "default_true")]
        skip_jedec: bool,
    },
}

/// Readable summary of one authoring transaction, returned for both dry-run previews and applied edits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeRecord {
    pub operation: String,
    pub dry_run: bool,
    pub changed: bool,
    pub summary: String,
    pub before_revision: u64,
    pub after_revision: u64,
    pub before_object_count: usize,
    pub after_object_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added_pads: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub renamed_pads: Vec<String>,
}

/// Validated transactions shared by forms, canvas tools and live MCP.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    /// Select a drawing without changing the identities of its primitives or logical terminals.
    SelectUnit {
        unit: String,
    },
    /// Add an empty draft drawing under a new stable identity and select it.
    AddUnit {
        unit: String,
    },
    Insert {
        primitive: Primitive,
    },
    Replace {
        object: u64,
        primitive: Primitive,
    },
    ReplaceMany {
        primitives: BTreeMap<u64, Primitive>,
    },
    /// Renumber explicitly selected logical pins simultaneously, including shared unit appearances.
    RemapPins {
        pins: Vec<remap::PinRemap>,
    },
    Delete {
        objects: Vec<u64>,
    },
    Transform {
        objects: Vec<u64>,
        transform: crate::library_graphics::Transform,
    },
    Duplicate {
        objects: Vec<u64>,
        offset: Point,
    },
    Paste {
        primitives: Vec<Primitive>,
        offset: Point,
    },
    Array {
        objects: Vec<u64>,
        columns: u16,
        rows: u16,
        column_step: Point,
        row_step: Point,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        numbering: Option<ArrayNumbering>,
    },
    Metadata {
        name: String,
        license: String,
        source: String,
        reference_prefix: String,
        default_value: String,
        #[serde(default)]
        symbol_settings: Option<SymbolSettings>,
        #[serde(default)]
        footprint_reference: Option<FootprintReferenceSettings>,
    },
    /// Generate a closed courtyard polygon enclosing pads and optional body graphics.
    GenerateCourtyard {
        clearance_nm: i64,
        #[serde(default)]
        source: courtyard::CourtyardSource,
        #[serde(default = "courtyard::default_stroke_nm")]
        stroke_nm: i64,
        #[serde(default = "courtyard::default_replace_existing")]
        replace_existing: bool,
    },
    Undo,
    Redo,
}

impl Item {
    /// Resolve an authored symbol through the same native parser used for pinned copies.
    /// Footprint documents return an error rather than producing a placeholder symbol.
    pub fn symbol(&self) -> Result<Arc<SymbolDefinition>, String> {
        if self.kind != Kind::Symbol {
            return Err("This document is a footprint.".into());
        }
        library::read_saved_copy(self.native()?)
    }

    /// Resolve physical authoring data for the shared board painter and native export.
    pub fn footprint(&self) -> Result<footprint::FootprintDefinition, String> {
        if self.kind != Kind::Footprint {
            return Err("This document is a symbol.".into());
        }
        serde_json::from_value(self.native()?).map_err(|e| e.to_string())
    }

    /// Create an original starting definition with a fresh portable identity.
    pub fn new(kind: Kind) -> Result<Self, String> {
        let mut bytes = [0_u8; 16];
        getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
        let id = format!("item-{}", bytes.iter().map(|byte| format!("{byte:02x}")).collect::<String>());
        let mut item = Self {
            format: FORMAT.into(),
            kind,
            library: "user".into(),
            id,
            revision: 1,
            name: if kind == Kind::Symbol { "Custom symbol" } else { "Custom footprint" }.into(),
            license: "MIT OR Apache-2.0".into(),
            source: "Original user-authored definition".into(),
            reference_prefix: "U".into(),
            default_value: "Custom part".into(),
            behavior: None,
            defaults: None,
            category: None,
            unit: "main".into(),
            unit_order: Vec::new(),
            other_units: BTreeMap::new(),
            reference_font: None,
            reference_offset: None,
            original_source: None,
            allocation: Allocation { object: 3, revision: 1 },
            objects: BTreeMap::new(),
        };
        let point = |x_nm, y_nm| Point { x_nm, y_nm };
        if kind == Kind::Symbol {
            item.objects.insert(
                1,
                Primitive::Pin {
                    number: "1".into(),
                    name: Some("IN".into()),
                    electrical_type: ElectricalType::Passive,
                    alternates: Vec::new(),
                    at: point(-5_080_000, 0),
                    appearance: Some(crate::schematic::PinAppearance::default()),
                },
            );
            item.objects.insert(2, Primitive::Rectangle { min: point(-2_540_000, -2_540_000), max: point(2_540_000, 2_540_000) });
            item.objects.insert(
                3,
                Primitive::Pin {
                    number: "2".into(),
                    name: Some("OUT".into()),
                    electrical_type: ElectricalType::Passive,
                    alternates: Vec::new(),
                    at: point(5_080_000, 0),
                    appearance: Some(crate::schematic::PinAppearance { orientation: crate::schematic::PinOrientation::Left, ..Default::default() }),
                },
            );
        } else {
            for (id, x_nm) in [(1, -500_000), (2, 500_000)] {
                item.objects.insert(
                    id,
                    Primitive::Pad {
                        pad: footprint::RectPad {
                            number: id.to_string(),
                            center: point(x_nm, 0),
                            width_nm: 500_000,
                            height_nm: 600_000,
                            rotation: 0,
                            shape: footprint::PadShape::RoundedRectangle,
                            copper: footprint::CopperBinding::ComponentSide,
                            corner_radius_nm: 125_000,
                            drill_nm: None,
                            plated_slot: None,
                        },
                    },
                );
            }
            item.objects.insert(
                3,
                Primitive::Outline {
                    graphic: footprint::GraphicRect {
                        purpose: footprint::GraphicPurpose::Fabrication,
                        center: point(0, 0),
                        width_nm: 1_600_000,
                        height_nm: 800_000,
                        rotation: 0,
                    },
                },
            );
        }
        item.allocation.object = item.objects.keys().next_back().copied().unwrap_or(0);
        item.validate()?;
        Ok(item)
    }

    /// Fork an exact symbol under a new identity, leaving its source and placed copies unchanged.
    pub fn from_symbol(definition: &SymbolDefinition) -> Result<Self, String> {
        Self::read_symbol(definition, true)
    }

    /// Reconstruct a published symbol without migrating its drawing or changing terminal/unit IDs.
    /// Primitive IDs are allocated in native unit, pin and drawing order for this session.
    pub fn edit_symbol(definition: &SymbolDefinition) -> Result<Self, String> {
        Self::read_symbol(definition, false)
    }

    /// Share reconstruction with explicit forks, limiting legacy lead integration to the copy path.
    fn read_symbol(definition: &SymbolDefinition, fork: bool) -> Result<Self, String> {
        let mut item = Self::new(Kind::Symbol)?;
        item.objects.clear();
        item.name = copy_name(&definition.name);
        item.source = format!("Fork of {}/{} at revision {}", definition.library, definition.id, definition.revision());
        let original = if definition.library == "starter" { library::compatibility::starter() } else { library::basic_components() }
            .symbol(&definition.id)
            .cloned();
        let known_original = original.as_ref().is_some_and(|original| {
            serde_json::to_value(library::saved_copy(definition))
                .ok()
                .is_some_and(|value| serde_json::to_value(library::saved_copy(original)).ok() == Some(value))
        });
        item.license = definition
            .provenance
            .as_ref()
            .map(|p| p.license.clone())
            .unwrap_or_else(|| if known_original { item.license.clone() } else { "Not recorded".into() });
        item.reference_prefix = definition.reference_prefix().into();
        item.default_value = definition.default_value().into();
        item.unit = definition.unit_id().into();
        item.unit_order =
            if definition.units.len() > 1 { definition.units.iter().map(|unit| unit.unit_id().to_owned()).collect() } else { Vec::new() };
        item.behavior = definition.behavior.clone();
        item.defaults = definition.defaults.clone();
        item.category = definition.category.clone();
        item.allocation.object = 0;
        for unit in &definition.units {
            let mut consumed = std::collections::BTreeSet::new();
            for pin in unit.pins() {
                item.objects.insert(
                    item.allocation.object + item.objects.len() as u64 + 1,
                    Primitive::Pin {
                        number: pin.number.clone(),
                        name: pin.name.clone(),
                        electrical_type: pin.electrical_type,
                        alternates: pin.alternates.clone(),
                        at: pin.offset,
                        appearance: pin.appearance.clone().or_else(|| fork.then(|| operations::legacy_pin(unit, pin, &mut consumed)).flatten()),
                    },
                );
            }
            for (index, shape) in unit.drawing.iter().enumerate() {
                if consumed.contains(&index) {
                    continue;
                }
                let primitive = match shape {
                    Shape::Graphic(graphic) => Primitive::Graphic { graphic: graphic.clone() },
                    Shape::Line(a, b) => Primitive::Line { from: *a, to: *b },
                    Shape::Rect(a, b) => Primitive::Rectangle { min: *a, max: *b },
                    Shape::Circle(at, r) => Primitive::Circle { center: *at, radius_nm: *r },
                    Shape::Polyline(points) => Primitive::Polyline { points: points.clone() },
                    Shape::Text(at, text, align) => Primitive::Text { at: *at, text: text.clone(), align: *align },
                };
                item.objects.insert(item.allocation.object + item.objects.len() as u64 + 1, primitive);
            }
            item.allocation.object = item.objects.keys().next_back().copied().unwrap_or(item.allocation.object);
            item.other_units.insert(unit.unit_id().to_owned(), std::mem::take(&mut item.objects));
        }
        item.objects = item.other_units.remove(&item.unit).ok_or("Missing default symbol unit.")?;
        if !item.other_units.is_empty() {
            item.format = MULTI_UNIT_FORMAT.into();
        }
        if definition.format() == library::SYMBOL_FORMAT_V5 {
            item.format = PIN_FUNCTION_FORMAT.into();
        }
        if !fork {
            item.library = definition.library.clone();
            item.id = definition.id.clone();
            item.name = definition.name.clone();
            item.revision = definition.revision();
            item.allocation.revision = item.revision;
            item.source = format!("Published {}/{} r{}", item.library, item.id, item.revision);
        }
        item.validate()?;
        Ok(item)
    }

    /// Fork physical data while retaining its license and source; the original receives no writes.
    pub fn from_footprint(definition: &footprint::FootprintDefinition) -> Result<Self, String> {
        let mut item = Self::read_footprint(definition)?;
        item.name = copy_name(&definition.name);
        item.source =
            bounded_text(format!("Fork of {}/{} r{}; {}", definition.library, definition.id, definition.revision, definition.source.reference), 2048);
        item.validate()?;
        Ok(item)
    }

    /// Reconstruct geometry before choosing fork or published metadata. Intermediate copy labels
    /// cannot invalidate an otherwise editable published definition at its source-text limit.
    /// Refuses a definition with an orientation statement, which the editor cannot keep yet.
    fn read_footprint(definition: &footprint::FootprintDefinition) -> Result<Self, String> {
        // The editor cannot carry an orientation statement yet; refuse rather than drop it.
        if definition.orientation.is_some() {
            return Err("This footprint states an assembly orientation, which the editor cannot keep yet.".into());
        }
        let mut item = Self::new(Kind::Footprint)?;
        item.objects.clear();
        item.license = definition.license.clone();
        item.reference_font = definition.reference.clone();
        item.reference_offset = definition.reference_offset;
        item.original_source = Some(definition.source.clone());
        for primitive in definition
            .pads
            .iter()
            .cloned()
            .map(|pad| Primitive::Pad { pad })
            .chain(definition.non_plated_holes.iter().copied().map(|hole| Primitive::NonPlatedHole { hole }))
            .chain(definition.graphics.iter().cloned().map(|graphic| Primitive::Outline { graphic }))
            .chain(definition.artwork.iter().cloned().map(|artwork| Primitive::Artwork { artwork }))
            .chain(definition.silkscreen.iter().cloned().map(|path| Primitive::Silkscreen { path }))
        {
            item.objects.insert(item.objects.len() as u64 + 1, primitive);
        }
        item.allocation.object = item.objects.keys().next_back().copied().unwrap_or(0);
        item.format = item.footprint_format().into();
        Ok(item)
    }

    /// Re-derive the editor format after a direct test mutation of footprint primitives.
    #[cfg(test)]
    pub(crate) fn refresh_footprint_format(&mut self) {
        if self.kind == Kind::Footprint {
            self.format = self.footprint_format().into();
        }
    }

    /// Select the lowest editor format preserving all hole primitives, including mixed drafts.
    fn footprint_format(&self) -> &'static str {
        let mut format = FORMAT;
        for object in self.objects.values() {
            match object {
                Primitive::Pad { pad } if pad.plated_slot.is_some() => return SLOT_FORMAT,
                Primitive::NonPlatedHole { .. } => format = NPTH_FORMAT,
                _ => {}
            }
        }
        format
    }

    /// Reopen an exact footprint, numbering primitives deterministically and retaining pad IDs.
    pub fn edit_footprint(definition: &footprint::FootprintDefinition) -> Result<Self, String> {
        let mut item = Self::read_footprint(definition)?;
        item.library = definition.library.clone();
        item.id = definition.id.clone();
        item.name = definition.name.clone();
        item.source = definition.source.reference.clone();
        item.revision = definition.revision;
        item.allocation.revision = item.revision;
        item.validate()?;
        Ok(item)
    }

    /// Derive a native definition through existing validators. Cross-kind primitives, duplicated
    /// electrical identities and unsupported geometry fail rather than being silently omitted.
    pub fn native(&self) -> Result<Value, String> {
        self.native_mode(false)
    }
    /// Check whether the draft requires the alternate-function editor format, including inactive units.
    fn has_alternate_functions(&self) -> bool {
        self.objects
            .values()
            .chain(self.other_units.values().flat_map(BTreeMap::values))
            .any(|p| matches!(p, Primitive::Pin { alternates, .. } if !alternates.is_empty()))
    }

    /// Validate draft and native definitions through the same path. Newly authored symbols
    /// and footprints use their current native formats regardless of optional records.
    fn native_mode(&self, draft: bool) -> Result<Value, String> {
        let at = |point: Point| json!([point.x_nm, point.y_nm]);
        if self.kind == Kind::Symbol {
            let mut terminal_map = BTreeMap::new();
            let mut terminal_order = Vec::new();
            let mut units = Vec::new();
            let drawings: BTreeMap<_, _> = std::iter::once((self.unit.as_str(), &self.objects))
                .chain(self.other_units.iter().map(|(id, objects)| (id.as_str(), objects)))
                .collect();
            let order: Vec<_> =
                if self.unit_order.is_empty() { drawings.keys().copied().collect() } else { self.unit_order.iter().map(String::as_str).collect() };
            for id in order {
                let objects = drawings.get(id).ok_or("Missing ordered symbol unit.")?;
                let mut pins = Vec::new();
                let mut drawing = Vec::new();
                for object in objects.values() {
                    drawing.extend(match object {
                        Primitive::Pin { number, name, electrical_type, alternates, at: point, appearance } => {
                            let mut terminal = json!({"id":number,"electrical_type":electrical_type});
                            if let Some(name) = name {
                                terminal["name"] = json!(name);
                            }
                            if !alternates.is_empty() {
                                terminal["alternates"] = json!(alternates);
                            }
                            if let Some(previous) = terminal_map.insert(number.clone(), terminal.clone()) {
                                if previous != terminal {
                                    return Err("Shared terminal metadata must agree across units.".into());
                                }
                            } else {
                                terminal_order.push(number.clone());
                            }
                            let mut pin = json!({"terminal":number,"at_nm":at(*point)});
                            if let Some(appearance) = appearance {
                                pin["appearance"] = serde_json::to_value(appearance).map_err(|e| e.to_string())?;
                            }
                            pins.push(pin);
                            None
                        }
                        Primitive::Graphic { graphic } => Some(json!({"type":"graphic","graphic":graphic})),
                        Primitive::Line { from, to } => Some(json!({"type":"line","from_nm":at(*from),"to_nm":at(*to)})),
                        Primitive::Rectangle { min, max } => Some(json!({"type":"rect","min_nm":at(*min),"max_nm":at(*max)})),
                        Primitive::Circle { center, radius_nm } => Some(json!({"type":"circle","center_nm":at(*center),"radius_nm":radius_nm})),
                        Primitive::Text { at: point, text, align } => Some(json!({"type":"text","at_nm":at(*point),"text":text,"align":align})),
                        Primitive::Polyline { points } => {
                            Some(json!({"type":"polyline","points_nm":points.iter().map(|point|at(*point)).collect::<Vec<_>>()}))
                        }
                        _ => return Err("A symbol cannot contain footprint primitives.".into()),
                    });
                }
                units.push(json!({"id":id,"drawing":drawing,"pins":pins}));
            }
            let terminals: Vec<_> = terminal_order.iter().map(|id| &terminal_map[id]).collect();
            let mut value = json!({"format":library::SYMBOL_FORMAT_V5,"library":self.library,"id":self.id,"revision":self.revision,"name":self.name,
                "reference_prefix":self.reference_prefix,"default_value":self.default_value,"terminals":terminals,"units":units});
            if let Some(behavior) = &self.behavior {
                value["behavior"] = json!(behavior);
            }
            if let Some(defaults) = &self.defaults {
                value["defaults"] = json!(defaults);
            }
            if let Some(category) = &self.category {
                value["category"] = json!(category);
            }
            if draft {
                library::read_saved_draft(value.clone())?;
            } else {
                library::read_saved_copy(value.clone())?;
            }
            Ok(value)
        } else {
            let mut pads = Vec::new();
            let mut non_plated_holes = Vec::new();
            let mut artwork = Vec::new();
            let mut graphics = Vec::new();
            let mut silkscreen = Vec::new();
            for object in self.objects.values() {
                match object {
                    Primitive::Artwork { artwork: a } => artwork.push(a.clone()),
                    Primitive::Pad { pad } => pads.push(pad.clone()),
                    Primitive::NonPlatedHole { hole } => non_plated_holes.push(*hole),
                    Primitive::Outline { graphic } => graphics.push(graphic.clone()),
                    Primitive::Silkscreen { path } => silkscreen.push(path.clone()),
                    _ => return Err("A footprint cannot contain schematic primitives.".into()),
                }
            }
            let definition = footprint::FootprintDefinition {
                format: footprint::CURRENT_FORMAT.into(),
                library: self.library.clone(),
                id: self.id.clone(),
                revision: self.revision,
                name: self.name.clone(),
                license: self.license.clone(),
                source: footprint::Source { kind: "native-editor".into(), reference: self.source.clone(), locator: self.id.clone() },
                pads,
                non_plated_holes,
                graphics,
                artwork,
                silkscreen,
                reference: self.reference_font.clone(),
                reference_offset: self.reference_offset,
                orientation: None,
            };
            if draft {
                definition.validate_draft()?;
            } else {
                definition.validate()?;
            }
            serde_json::to_value(definition).map_err(|e| e.to_string())
        }
    }

    /// Maximum primitives permitted in the active drawing for this document kind.
    pub(super) fn max_objects(&self) -> usize {
        match self.kind {
            Kind::Symbol => MAX_SYMBOL_OBJECTS,
            Kind::Footprint => MAX_FOOTPRINT_OBJECTS,
        }
    }
    /// Bound the editor envelope as well as the derived native geometry and electrical metadata.
    pub fn validate(&self) -> Result<(), String> {
        let all_objects: Vec<_> = self.objects.iter().chain(self.other_units.values().flat_map(|objects| objects.iter())).collect();
        let unique: std::collections::BTreeSet<_> = all_objects.iter().map(|(id, _)| *id).collect();
        if !matches!(self.format.as_str(), FORMAT | MULTI_UNIT_FORMAT | NPTH_FORMAT | SLOT_FORMAT | PIN_FUNCTION_FORMAT)
            || (self.kind == Kind::Footprint && self.format != self.footprint_format())
            || (self.kind == Kind::Symbol && matches!(self.format.as_str(), NPTH_FORMAT | SLOT_FORMAT))
            || (self.format != PIN_FUNCTION_FORMAT && self.has_alternate_functions())
            || (!self.other_units.is_empty() && (self.format == FORMAT || self.kind != Kind::Symbol))
            || (!self.unit_order.is_empty()
                && (self.unit_order.len() != self.other_units.len() + 1
                    || self.unit_order.iter().collect::<std::collections::BTreeSet<_>>()
                        != std::iter::once(&self.unit).chain(self.other_units.keys()).collect()))
            || self.other_units.contains_key(&self.unit)
            || self.other_units.len() >= 32
            || unique.len() != all_objects.len()
            || all_objects.iter().any(|(id, _)| **id == 0 || **id > self.allocation.object)
            || self.other_units.values().any(|objects| objects.len() > MAX_SYMBOL_OBJECTS)
            || self.objects.len() > self.max_objects()
            || self.objects.contains_key(&0)
            || self.objects.keys().any(|id| *id > self.allocation.object)
            || self.revision > self.allocation.revision
            || self.source.len() > if self.kind == Kind::Footprint { 2048 } else { 1024 }
            || self.license.trim().is_empty()
            || self.license.len() > 128
            || self.original_source.as_ref().is_some_and(|source| serde_json::to_vec(source).map_or(true, |bytes| bytes.len() > 16_384))
            || self.reference_prefix.len() > 4
            || self.default_value.len() > 256
            || self.unit.len() > 64
            || (self.kind == Kind::Symbol && self.reference_offset.is_some())
            || serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?.len() >= MAX_BYTES
        {
            return Err("Invalid or oversized native library item.".into());
        }
        self.native_mode(true)?;
        Ok(())
    }

    /// Decode a bounded editor document before opening it; no project or source file is changed.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_BYTES {
            return Err("Library item exceeds the 1 MiB limit.".into());
        }
        let mut item: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if item.unit_order.is_empty() && !item.other_units.is_empty() {
            item.unit_order = std::iter::once(item.unit.clone()).chain(item.other_units.keys().cloned()).collect();
            item.unit_order.sort();
        }
        item.validate()?;
        Ok(item)
    }

    /// Encode deterministic bytes with stable object IDs and recorded provenance.
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        if self.license == "Not recorded" {
            return Err("Record the source license before saving this library item.".into());
        }
        let mut bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        if bytes.len() > MAX_BYTES {
            return Err("Formatted library item exceeds the 1 MiB limit.".into());
        }
        Ok(bytes)
    }

    /// Export a new standalone native definition. Editor identities, primitive IDs and
    /// symbol provenance remain in the editable item; export never confirms an editor save.
    pub fn export_native(&self) -> Result<Value, String> {
        self.encode()?;
        let mut value = self.native()?;
        if self.kind == Kind::Symbol {
            value.as_object_mut().ok_or("Invalid native symbol.")?.remove("library");
        }
        Ok(value)
    }
}

/// One definition's history has monotonic runtime revisions and allocation counters outside undo.
#[derive(Debug)]
pub struct Session {
    item: Arc<Item>,
    saved: Option<Arc<Item>>,
    undo: Vec<Arc<Item>>,
    redo: Vec<Arc<Item>>,
    revision: u64,
    next_id: u64,
    next_content_revision: u32,
    published: Option<publishing::Address>,
    publication_saved: Option<Arc<Item>>,
    published_symbol: Option<Arc<SymbolDefinition>>,
}

impl Session {
    /// Start a validated document; a successfully imported file can begin in a saved state.
    pub fn new(item: Item, saved: bool) -> Result<Self, String> {
        item.validate()?;
        let next_id = item.allocation.object.checked_add(1).ok_or("Primitive ID limit reached.")?;
        let next_content_revision = item.allocation.revision.checked_add(1).ok_or("Content revision limit reached.")?;
        let item = Arc::new(item);
        Ok(Self {
            saved: saved.then(|| item.clone()),
            item,
            undo: Vec::new(),
            redo: Vec::new(),
            revision: 0,
            next_id,
            next_content_revision,
            published: None,
            publication_saved: None,
            published_symbol: None,
        })
    }
    /// Retain immutable data for rendering or an in-flight save.
    pub fn snapshot(&self) -> Arc<Item> {
        self.item.clone()
    }
    /// Runtime authority advances for edits, undo and redo, and never rewinds with history.
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// A download or an older save cannot clear newer unsaved changes.
    pub fn dirty(&self) -> bool {
        self.saved.as_ref() != Some(&self.item)
    }
    /// Whether an authored transaction can be undone without restoring project history.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    /// Whether an undone library transaction can be replayed.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    /// Adopt exactly the snapshot reported by a successful durable write.
    pub fn mark_saved(&mut self, snapshot: Arc<Item>) {
        self.saved = Some(snapshot);
    }

    /// Validate and preview one transaction without mutating data, revision, history or allocation counters.
    pub fn preview(&self, expected_revision: u64, action: Action) -> Result<ChangeRecord, String> {
        self.plan(expected_revision, action, true).map(|(_, record)| record)
    }

    /// Apply one transaction and return its readable change record.
    /// Rejections preserve data, history and allocation counters.
    pub fn execute_change(&mut self, expected_revision: u64, action: Action) -> Result<ChangeRecord, String> {
        let (planned, record) = self.plan(expected_revision, action, false)?;
        match planned {
            Planned::NoChange => return Ok(record),
            Planned::Undo(previous, revision) => {
                self.undo.pop();
                self.redo.push(std::mem::replace(&mut self.item, previous));
                self.revision = revision;
            }
            Planned::Redo(next, revision) => {
                self.redo.pop();
                self.undo.push(std::mem::replace(&mut self.item, next));
                self.revision = revision;
            }
            Planned::Edit { next, next_id, next_content_revision, revision } => {
                self.undo.push(std::mem::replace(&mut self.item, next));
                self.redo.clear();
                if self.undo.len() > HISTORY {
                    self.undo.remove(0);
                }
                self.next_id = next_id;
                self.next_content_revision = next_content_revision;
                self.revision = revision;
            }
        }
        Ok(record)
    }

    /// Apply one transaction. Rejections preserve data, history and allocation counters.
    pub fn execute(&mut self, expected_revision: u64, action: Action) -> Result<u64, String> {
        self.execute_change(expected_revision, action).map(|record| record.after_revision)
    }

    /// Validate a candidate transaction against the current snapshot and build its change record.
    fn plan(&self, expected_revision: u64, action: Action, dry_run: bool) -> Result<(Planned, ChangeRecord), String> {
        if expected_revision != self.revision {
            return Err("Library document changed; inspect it before editing.".into());
        }
        let operation = action_name(&action).to_owned();
        let revision = self.revision.checked_add(1).ok_or("Runtime revision limit reached.")?;
        let allocation = Allocation { object: self.next_id - 1, revision: self.next_content_revision - 1 };
        let before_count = self.item.objects.len();
        match action {
            Action::Undo => {
                let previous = restored(self.undo.last().ok_or("Nothing to undo.")?, allocation)?;
                let after_count = previous.objects.len();
                let record = ChangeRecord {
                    operation,
                    dry_run,
                    changed: true,
                    summary: format!("Undo to revision {revision}: {before_count} -> {after_count} primitives."),
                    before_revision: self.revision,
                    after_revision: revision,
                    before_object_count: before_count,
                    after_object_count: after_count,
                    added_pads: Vec::new(),
                    renamed_pads: Vec::new(),
                };
                Ok((Planned::Undo(previous, revision), record))
            }
            Action::Redo => {
                let next = restored(self.redo.last().ok_or("Nothing to redo.")?, allocation)?;
                let after_count = next.objects.len();
                let record = ChangeRecord {
                    operation,
                    dry_run,
                    changed: true,
                    summary: format!("Redo to revision {revision}: {before_count} -> {after_count} primitives."),
                    before_revision: self.revision,
                    after_revision: revision,
                    before_object_count: before_count,
                    after_object_count: after_count,
                    added_pads: Vec::new(),
                    renamed_pads: Vec::new(),
                };
                Ok((Planned::Redo(next, revision), record))
            }
            action => {
                let mut next = (*self.item).clone();
                let mut cursor = self.next_id;
                let mut ball_grid_spec: Option<(u16, u16)> = None;
                match action {
                    Action::SelectUnit { unit } => {
                        let objects = next.other_units.remove(&unit).ok_or("Unknown symbol unit.")?;
                        next.other_units.insert(std::mem::replace(&mut next.unit, unit), std::mem::replace(&mut next.objects, objects));
                    }
                    Action::AddUnit { unit } => {
                        if next.kind != Kind::Symbol || !footprint::valid_id(&unit) || unit == next.unit || next.other_units.contains_key(&unit) {
                            return Err("A new symbol unit needs an unused portable identity.".into());
                        }
                        if next.format != PIN_FUNCTION_FORMAT {
                            next.format = MULTI_UNIT_FORMAT.into();
                        }
                        if next.unit_order.is_empty() {
                            next.unit_order.push(next.unit.clone());
                        }
                        next.unit_order.push(unit.clone());
                        next.other_units.insert(std::mem::replace(&mut next.unit, unit), std::mem::take(&mut next.objects));
                    }
                    Action::Insert { primitive } => {
                        next.objects.insert(cursor, primitive);
                        cursor = cursor.checked_add(1).ok_or("Primitive ID limit reached.")?;
                    }
                    Action::Replace { object, primitive } => {
                        let slot = next.objects.get_mut(&object).ok_or("Unknown primitive ID.")?;
                        *slot = primitive;
                    }
                    Action::ReplaceMany { primitives } => {
                        let limit = next.max_objects();
                        if primitives.is_empty() || primitives.len() > limit || primitives.keys().any(|id| !next.objects.contains_key(id)) {
                            return Err("Unknown or empty primitive replacement batch.".into());
                        }
                        next.objects.extend(primitives);
                    }
                    Action::RemapPins { pins } => remap::apply(&mut next, &pins)?,
                    Action::Delete { objects } => {
                        if objects.is_empty() {
                            return Err("Choose at least one primitive.".into());
                        }
                        for id in objects {
                            if next.objects.remove(&id).is_none() {
                                return Err("Unknown or duplicate primitive ID.".into());
                            }
                        }
                    }
                    Action::Transform { objects, transform } => {
                        let originals = operations::selected(&next, &objects)?;
                        for (id, primitive) in objects.into_iter().zip(originals) {
                            next.objects.insert(id, primitive.transformed(transform)?);
                        }
                    }
                    Action::Duplicate { objects, offset } => {
                        let originals = operations::selected(&next, &objects)?;
                        operations::paste(&mut next, &originals, offset, &mut cursor)?;
                    }
                    Action::Paste { primitives, offset } => operations::paste(&mut next, &primitives, offset, &mut cursor)?,
                    Action::Array { objects, columns, rows, column_step, row_step, numbering } => {
                        if let Some(ArrayNumbering::BallGrid { skip_jedec }) = numbering {
                            operations::ball_grid_array(
                                &mut next,
                                &objects,
                                operations::BallGridSpec { columns, rows, column_step, row_step, skip_jedec },
                                &mut cursor,
                            )?;
                            ball_grid_spec = Some((rows, columns));
                        } else {
                            let originals = operations::selected(&next, &objects)?;
                            let limit = next.max_objects();
                            let count = usize::from(columns) * usize::from(rows);
                            if columns == 0
                                || rows == 0
                                || count > limit
                                || next.objects.len().saturating_add(count.saturating_sub(1).saturating_mul(originals.len())) > limit
                            {
                                return Err(format!("Array exceeds the definition's {limit} primitive limit."));
                            }
                            for row in 0..rows {
                                for column in 0..columns {
                                    if row == 0 && column == 0 {
                                        continue;
                                    }
                                    let axis = |a: i64, b: i64| {
                                        i64::try_from(i128::from(a) * i128::from(column) + i128::from(b) * i128::from(row))
                                            .map_err(|_| "Array offset overflow.")
                                    };
                                    let offset = Point { x_nm: axis(column_step.x_nm, row_step.x_nm)?, y_nm: axis(column_step.y_nm, row_step.y_nm)? };
                                    operations::paste(&mut next, &originals, offset, &mut cursor)?;
                                }
                            }
                        }
                    }
                    Action::Metadata { name, license, source, reference_prefix, default_value, symbol_settings, footprint_reference } => {
                        next.name = name;
                        next.license = license;
                        next.source = source;
                        next.reference_prefix = reference_prefix;
                        next.default_value = default_value;
                        if let Some(settings) = symbol_settings {
                            if next.kind != Kind::Symbol {
                                return Err("Symbol settings require a symbol document.".into());
                            }
                            next.behavior = settings.behavior;
                            next.defaults = settings.defaults;
                            if let Some(category) = settings.category {
                                next.category = category;
                            }
                        }
                        if let Some(settings) = footprint_reference {
                            if next.kind != Kind::Footprint {
                                return Err("Reference settings require a footprint document.".into());
                            }
                            if settings.enabled {
                                next.reference_font.get_or_insert_with(|| "pcbcraft-technical-1".into());
                                next.reference_offset = (settings.offset != crate::board::DEFAULT_REFERENCE_OFFSET).then_some(settings.offset);
                            } else {
                                next.reference_font = None;
                                next.reference_offset = None;
                            }
                        }
                    }
                    Action::GenerateCourtyard { clearance_nm, source, stroke_nm, replace_existing } => {
                        if next.kind != Kind::Footprint {
                            return Err("Courtyard generation is only available for footprint definitions.".into());
                        }
                        let primitive = courtyard::generate(&next, clearance_nm, source, stroke_nm)?;
                        if replace_existing {
                            next.objects.retain(|_, existing| !courtyard::is_courtyard_primitive(existing));
                        }
                        next.objects.insert(cursor, primitive);
                        cursor = cursor.checked_add(1).ok_or("Primitive ID limit reached.")?;
                    }
                    Action::Undo | Action::Redo => return Err("History operation cannot be nested.".into()),
                }
                if next.kind == Kind::Footprint {
                    next.format = next.footprint_format().into();
                }
                if next == *self.item {
                    let record = ChangeRecord {
                        operation,
                        dry_run,
                        changed: false,
                        summary: "No changes.".into(),
                        before_revision: self.revision,
                        after_revision: self.revision,
                        before_object_count: before_count,
                        after_object_count: before_count,
                        added_pads: Vec::new(),
                        renamed_pads: Vec::new(),
                    };
                    return Ok((Planned::NoChange, record));
                }
                let metadata: BTreeMap<_, _> = next
                    .objects
                    .values()
                    .filter_map(|object| match object {
                        Primitive::Pin { number, name, electrical_type, alternates, .. } => {
                            Some((number.clone(), (name.clone(), *electrical_type, alternates.clone())))
                        }
                        _ => None,
                    })
                    .collect();
                for object in next.other_units.values_mut().flat_map(|objects| objects.values_mut()) {
                    if let Primitive::Pin { number, name, electrical_type, alternates, .. } = object
                        && let Some((shared_name, shared_type, shared_alternates)) = metadata.get(number)
                    {
                        *name = shared_name.clone();
                        *electrical_type = *shared_type;
                        alternates.clone_from(shared_alternates);
                    }
                }
                if next.format != PIN_FUNCTION_FORMAT && next.has_alternate_functions() {
                    next.format = PIN_FUNCTION_FORMAT.into();
                }
                next.revision = self.next_content_revision;
                next.allocation = Allocation { object: cursor - 1, revision: self.next_content_revision };
                next.validate()?;
                // Every action that adds courtyard artwork, changes a primitive into courtyard
                // artwork, or changes a courtyard's stroke must satisfy the minimum courtyard
                // stroke; geometry-only edits preserve a stroke that was valid when saved. Any
                // added or changed closed courtyard polyline must also stay a simple polygon.
                // This runs after the bounds check above, so the polygon arithmetic only sees
                // bounded coordinates. Unchanged outlines are not rechecked, so a loaded
                // definition stays editable.
                for (id, primitive) in &next.objects {
                    let previous = self.item.objects.get(id);
                    if previous != Some(primitive) {
                        if let Primitive::Artwork { artwork } = primitive
                            && courtyard::is_courtyard_primitive(primitive)
                        {
                            let preserved_stroke = matches!(
                                previous,
                                Some(prev @ Primitive::Artwork { artwork: prev_art })
                                    if courtyard::is_courtyard_primitive(prev)
                                        && prev_art.graphic.stroke_nm == artwork.graphic.stroke_nm
                            );
                            if !preserved_stroke {
                                courtyard::validate_stroke(artwork.graphic.stroke_nm)?;
                            }
                        }
                        if let Some((points, _)) = courtyard::courtyard_polygon(primitive) {
                            courtyard::validate_polygon(points)?;
                        }
                    }
                }
                let next_content_revision = self.next_content_revision.checked_add(1).ok_or("Content revision limit reached.")?;
                let record = summarize_change(&self.item, &next, operation, dry_run, self.revision, revision, ball_grid_spec);
                Ok((Planned::Edit { next: Arc::new(next), next_id: cursor, next_content_revision, revision }, record))
            }
        }
    }
    /// Report document and primitive identities used by the GUI and live tools.
    pub fn inspect(&self) -> Value {
        json!({"revision":self.revision,"dirty":self.dirty(),"published":self.published,"publication_dirty":self.publication_dirty(),"saved_content_revision":self.saved.as_ref().map(|item|item.revision),"can_undo":!self.undo.is_empty(),"can_redo":!self.redo.is_empty(),"item":self.item.as_ref()})
    }
}

/// Validated state transition ready to commit or discard after a dry-run preview.
enum Planned {
    NoChange,
    Undo(Arc<Item>, u64),
    Redo(Arc<Item>, u64),
    Edit { next: Arc<Item>, next_id: u64, next_content_revision: u32, revision: u64 },
}

/// Stable snake-case operation name matching the serialized [`Action`] tag.
fn action_name(action: &Action) -> &'static str {
    match action {
        Action::SelectUnit { .. } => "select_unit",
        Action::AddUnit { .. } => "add_unit",
        Action::Insert { .. } => "insert",
        Action::Replace { .. } => "replace",
        Action::ReplaceMany { .. } => "replace_many",
        Action::RemapPins { .. } => "remap_pins",
        Action::Delete { .. } => "delete",
        Action::Transform { .. } => "transform",
        Action::Duplicate { .. } => "duplicate",
        Action::Paste { .. } => "paste",
        Action::Array { .. } => "array",
        Action::Metadata { .. } => "metadata",
        Action::GenerateCourtyard { .. } => "generate_courtyard",
        Action::Undo => "undo",
        Action::Redo => "redo",
    }
}

/// Build a readable summary of the pad and primitive differences between `before` and `after`.
fn summarize_change(
    before: &Item,
    after: &Item,
    operation: String,
    dry_run: bool,
    before_revision: u64,
    after_revision: u64,
    ball_grid_spec: Option<(u16, u16)>,
) -> ChangeRecord {
    let mut added_pads = Vec::new();
    let mut renamed: Vec<(String, String)> = Vec::new();
    for (id, primitive) in &after.objects {
        if let Primitive::Pad { pad } = primitive {
            match before.objects.get(id) {
                None => added_pads.push(pad.number.clone()),
                Some(Primitive::Pad { pad: old }) if old.number != pad.number => {
                    renamed.push((old.number.clone(), pad.number.clone()));
                }
                _ => {}
            }
        }
    }
    ball_grid::natural_sort(&mut added_pads);
    renamed.sort_by(|a, b| ball_grid::natural_cmp(&a.1, &b.1));
    let renamed_pads: Vec<String> = renamed.into_iter().map(|(old, new)| format!("{old} -> {new}")).collect();
    let summary = if let Some((rows, columns)) = ball_grid_spec {
        let mut all_pads: Vec<&str> =
            after.objects.values().filter_map(|p| if let Primitive::Pad { pad } = p { Some(pad.number.as_str()) } else { None }).collect();
        all_pads.sort_by(|a, b| ball_grid::natural_cmp(a, b));
        let span = match (all_pads.first(), all_pads.last()) {
            (Some(first), Some(last)) => format!(" ({first}..{last})"),
            _ => String::new(),
        };
        format!(
            "Ball-grid array {rows}x{columns}: {} pads{span}, {} added, {} renamed.",
            usize::from(rows) * usize::from(columns),
            added_pads.len(),
            renamed_pads.len(),
        )
    } else {
        format!("{operation}: {} -> {} primitives.", before.objects.len(), after.objects.len())
    };
    ChangeRecord {
        operation,
        dry_run,
        changed: true,
        summary,
        before_revision,
        after_revision,
        before_object_count: before.objects.len(),
        after_object_count: after.objects.len(),
        added_pads,
        renamed_pads,
    }
}

/// Reapply non-rewinding allocation records before publishing a history snapshot. A size or
/// schema failure leaves both history stacks and the current document untouched.
fn restored(item: &Arc<Item>, allocation: Allocation) -> Result<Arc<Item>, String> {
    if item.allocation == allocation {
        return Ok(item.clone());
    }
    let mut item = (**item).clone();
    item.allocation = allocation;
    item.validate()?;
    Ok(Arc::new(item))
}

/// Keep a bounded, unpadded copy label with the complete suffix and valid UTF-8 in both editors.
fn copy_name(name: &str) -> String {
    let prefix = bounded_text(name.to_owned(), 64 - " copy".len());
    format!("{} copy", prefix.trim_end())
}

/// Bound UTF-8 text by bytes while retaining complete code points.
fn bounded_text(mut text: String, limit: usize) -> String {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Stable IDs and source preservation survive geometry edits, undo, redo and reopening.
    #[test]
    fn footprint_authoring_preserves_source_and_history() {
        let original = footprint::basic_components("resistor-0402").unwrap();
        let before = (*original).clone();
        let mut s = Session::new(Item::from_footprint(&original).unwrap(), false).unwrap();
        let first = s.snapshot();
        let id = *first.objects.keys().next().unwrap();
        let Primitive::Pad { mut pad } = first.objects[&id].clone() else { panic!("pad expected") };
        pad.width_nm += 50_000;
        s.execute(0, Action::Replace { object: id, primitive: Primitive::Pad { pad } }).unwrap();
        assert_ne!(s.snapshot(), first);
        assert_eq!(*original, before);
        assert_eq!(s.snapshot().original_source.as_ref(), Some(&before.source));
        let changed = s.snapshot();
        assert_eq!(Item::decode(&changed.encode().unwrap()).unwrap(), *changed);
        s.execute(1, Action::Undo).unwrap();
        assert_eq!(s.snapshot().native().unwrap(), first.native().unwrap());
        assert!(s.execute(1, Action::Redo).is_err());
        s.execute(2, Action::Redo).unwrap();
        assert_eq!(s.snapshot(), changed);
    }
    /// Invalid identities cannot partially alter a draft or consume an object ID.
    #[test]
    fn invalid_and_duplicate_geometry_is_atomic() {
        let mut s = Session::new(Item::new(Kind::Footprint).unwrap(), false).unwrap();
        let before = s.snapshot();
        assert!(s.execute(0, Action::Insert { primitive: before.objects[&1].clone() }).is_err());
        assert_eq!(s.snapshot(), before);
        assert_eq!(s.revision(), 0);
        assert!(s.execute(0, Action::Delete { objects: vec![1, 999] }).is_err());
        assert_eq!(s.snapshot(), before);
        let Primitive::Pad { mut pad } = before.objects[&1].clone() else { panic!("pad expected") };
        pad.number = "3".into();
        s.execute(0, Action::Insert { primitive: Primitive::Pad { pad } }).unwrap();
        assert!(s.snapshot().objects.contains_key(&4));
        s.mark_saved(before);
        assert!(s.dirty());
    }
    /// Native terminal checks and deterministic encoding apply to symbol authoring too.
    #[test]
    fn symbol_terminals_validate_and_save_deterministically() {
        let definition = library::basic_components().symbol("connector").unwrap();
        let item = Item::from_symbol(definition).unwrap();
        let bytes = item.encode().unwrap();
        assert_eq!(Item::decode(&bytes).unwrap().encode().unwrap(), bytes);
        let mut s = Session::new(item, true).unwrap();
        assert!(!s.dirty());
        let Primitive::Pin { number, name, electrical_type, .. } = s.snapshot().objects[&1].clone() else { panic!("pin expected") };
        assert!(
            s.execute(
                0,
                Action::Replace {
                    object: 1,
                    primitive: Primitive::Pin {
                        number,
                        name,
                        electrical_type,
                        alternates: Vec::new(),
                        at: Point { x_nm: 1, y_nm: 0 },
                        appearance: None
                    }
                }
            )
            .is_err()
        );
        assert!(!s.dirty());
    }
    /// Standalone exports use existing schemas and cannot bypass an unrecorded source license.
    #[test]
    fn native_exports_validate_and_unknown_sources_require_a_license() {
        let item = Item::new(Kind::Symbol).unwrap();
        let native = item.export_native().unwrap();
        let parsed = library::parse_symbol(&native.to_string()).unwrap();
        assert_eq!(parsed.id, item.id);
        assert_eq!(parsed.pins().len(), 2);
        let footprint = Item::new(Kind::Footprint).unwrap();
        let parsed: footprint::FootprintDefinition = serde_json::from_value(footprint.export_native().unwrap()).unwrap();
        parsed.validate().unwrap();
        let unknown = library::parse_symbol(&native.to_string()).unwrap();
        let fork = Item::from_symbol(&unknown).unwrap();
        assert_eq!(fork.license, "Not recorded");
        assert!(fork.encode().is_err());
        assert!(fork.export_native().is_err());
    }
    /// Save/reopen retains allocations beyond undo and rejects ambiguous primitive records.
    #[test]
    fn allocation_survives_history_save_and_reopen() {
        let mut s = Session::new(Item::new(Kind::Footprint).unwrap(), false).unwrap();
        let outline = s.snapshot().objects[&3].clone();
        s.execute(0, Action::Delete { objects: vec![3] }).unwrap();
        s.execute(1, Action::Insert { primitive: outline.clone() }).unwrap();
        assert!(s.snapshot().objects.contains_key(&4));
        s.execute(2, Action::Undo).unwrap();
        let item = Item::decode(&s.snapshot().encode().unwrap()).unwrap();
        let mut reopened = Session::new(item, true).unwrap();
        reopened.execute(0, Action::Insert { primitive: outline }).unwrap();
        assert!(reopened.snapshot().objects.contains_key(&5));
        assert_eq!(reopened.snapshot().revision, 4);
        let original = Item::new(Kind::Footprint).unwrap();
        let bytes = serde_json::to_string(&original).unwrap();
        let primitive = serde_json::to_string(&original.objects[&1]).unwrap();
        let duplicate = bytes.replace("\"objects\":{", &format!("\"objects\":{{\"1\":{primitive},"));
        assert!(Item::decode(duplicate.as_bytes()).is_err());
        let mut alias = serde_json::to_value(&original).unwrap();
        alias["objects"] = json!({"01":original.objects[&1]});
        assert!(Item::decode(&serde_json::to_vec(&alias).unwrap()).is_err());
    }
}
