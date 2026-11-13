//! Native library-item documents own validation and history independently of projects.
#[cfg(test)]
mod basics_tests;
mod operations;
pub mod publishing;
#[cfg(test)]
mod publishing_tests;
use crate::{
    board::definition as footprint,
    library,
    schematic::{ElectricalType, Point, Shape, SymbolDefinition, TextAlign},
};
pub use operations::next_number;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

/// Maximum editor document size, including metadata and stable primitive identities.
pub const MAX_BYTES: usize = 1 << 20;
const FORMAT: &str = "pcbcraft-library-item/1";
const MULTI_UNIT_FORMAT: &str = "pcbcraft-library-item/2";
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
            Self::Pin { number, name, electrical_type, at: p, appearance } => Self::Pin {
                number: number.clone(),
                name: name.clone(),
                electrical_type: *electrical_type,
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
                if id == 0 || id.to_string() != key || objects.len() >= 512 || objects.contains_key(&id) {
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
        item.name = format!("{} copy", definition.name).chars().take(64).collect();
        item.source = format!("Fork of {}/{} at revision {}", definition.library, definition.id, definition.revision());
        let original = library::basic_components().symbol(&definition.id).cloned();
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
        let mut item = Self::new(Kind::Footprint)?;
        item.objects.clear();
        item.name = format!("{} copy", definition.name).chars().take(64).collect();
        item.license = definition.license.clone();
        item.source = format!("Fork of {}/{} r{}; {}", definition.library, definition.id, definition.revision, definition.source.reference);
        item.reference_font = definition.reference.clone();
        item.reference_offset = definition.reference_offset;
        item.original_source = Some(definition.source.clone());
        for primitive in definition
            .pads
            .iter()
            .cloned()
            .map(|pad| Primitive::Pad { pad })
            .chain(definition.graphics.iter().cloned().map(|graphic| Primitive::Outline { graphic }))
            .chain(definition.artwork.iter().cloned().map(|artwork| Primitive::Artwork { artwork }))
            .chain(definition.silkscreen.iter().cloned().map(|path| Primitive::Silkscreen { path }))
        {
            item.objects.insert(item.objects.len() as u64 + 1, primitive);
        }
        item.allocation.object = item.objects.keys().next_back().copied().unwrap_or(0);
        item.validate()?;
        Ok(item)
    }

    /// Reopen an exact footprint, numbering primitives deterministically and retaining pad IDs.
    pub fn edit_footprint(definition: &footprint::FootprintDefinition) -> Result<Self, String> {
        let mut item = Self::from_footprint(definition)?;
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
    /// Derive draft/native data through one validator path, preserving unsupported-kind rejection.
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
                        Primitive::Pin { number, name, electrical_type, at: point, appearance } => {
                            let terminal = json!({"id":number,"name":name,"electrical_type":electrical_type});
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
            let mut value = json!({"format":if !self.other_units.is_empty() { library::SYMBOL_FORMAT_V4 } else if self.category.is_some() { library::SYMBOL_FORMAT_V3 } else if self.behavior.is_some() || self.defaults.is_some() { library::SYMBOL_FORMAT_V2 } else { library::SYMBOL_FORMAT },"library":self.library,"id":self.id,"revision":self.revision,"name":self.name,
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
            let mut artwork = Vec::new();
            let mut graphics = Vec::new();
            let mut silkscreen = Vec::new();
            for object in self.objects.values() {
                match object {
                    Primitive::Artwork { artwork: a } => artwork.push(a.clone()),
                    Primitive::Pad { pad } => pads.push(pad.clone()),
                    Primitive::Outline { graphic } => graphics.push(graphic.clone()),
                    Primitive::Silkscreen { path } => silkscreen.push(path.clone()),
                    _ => return Err("A footprint cannot contain schematic primitives.".into()),
                }
            }
            let definition = footprint::FootprintDefinition {
                format: "pcbcraft-footprint/1".into(),
                library: self.library.clone(),
                id: self.id.clone(),
                revision: self.revision,
                name: self.name.clone(),
                license: self.license.clone(),
                source: footprint::Source { kind: "native-editor".into(), reference: self.source.clone(), locator: self.id.clone() },
                pads,
                graphics,
                artwork,
                silkscreen,
                reference: self.reference_font.clone(),
                reference_offset: self.reference_offset,
            };
            if draft {
                definition.validate_draft()?;
            } else {
                definition.validate()?;
            }
            serde_json::to_value(definition).map_err(|e| e.to_string())
        }
    }

    /// Bound the editor envelope as well as the derived native geometry and electrical metadata.
    pub fn validate(&self) -> Result<(), String> {
        let all_objects: Vec<_> = self.objects.iter().chain(self.other_units.values().flat_map(|objects| objects.iter())).collect();
        let unique: std::collections::BTreeSet<_> = all_objects.iter().map(|(id, _)| *id).collect();
        if !matches!(self.format.as_str(), FORMAT | MULTI_UNIT_FORMAT)
            || (!self.other_units.is_empty() && (self.format != MULTI_UNIT_FORMAT || self.kind != Kind::Symbol))
            || (!self.unit_order.is_empty()
                && (self.unit_order.len() != self.other_units.len() + 1
                    || self.unit_order.iter().collect::<std::collections::BTreeSet<_>>()
                        != std::iter::once(&self.unit).chain(self.other_units.keys()).collect()))
            || self.other_units.contains_key(&self.unit)
            || self.other_units.len() >= 32
            || unique.len() != all_objects.len()
            || all_objects.iter().any(|(id, _)| **id == 0 || **id > self.allocation.object)
            || self.other_units.values().any(|objects| objects.len() > 512)
            || self.objects.len() > 512
            || self.objects.contains_key(&0)
            || self.objects.keys().any(|id| *id > self.allocation.object)
            || self.revision > self.allocation.revision
            || self.source.len() > 1024
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

    /// Export the existing standalone native schema. Editor identities, primitive IDs and
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

    /// Apply one transaction. Rejections preserve data, history and allocation counters.
    pub fn execute(&mut self, expected_revision: u64, action: Action) -> Result<u64, String> {
        if expected_revision != self.revision {
            return Err("Library document changed; inspect it before editing.".into());
        }
        let revision = self.revision.checked_add(1).ok_or("Runtime revision limit reached.")?;
        let allocation = Allocation { object: self.next_id - 1, revision: self.next_content_revision - 1 };
        match action {
            Action::Undo => {
                let previous = restored(self.undo.last().ok_or("Nothing to undo.")?, allocation)?;
                self.undo.pop();
                self.redo.push(std::mem::replace(&mut self.item, previous));
            }
            Action::Redo => {
                let next = restored(self.redo.last().ok_or("Nothing to redo.")?, allocation)?;
                self.redo.pop();
                self.undo.push(std::mem::replace(&mut self.item, next));
            }
            action => {
                let mut next = (*self.item).clone();
                let mut cursor = self.next_id;
                match action {
                    Action::SelectUnit { unit } => {
                        let objects = next.other_units.remove(&unit).ok_or("Unknown symbol unit.")?;
                        next.other_units.insert(std::mem::replace(&mut next.unit, unit), std::mem::replace(&mut next.objects, objects));
                    }
                    Action::AddUnit { unit } => {
                        if next.kind != Kind::Symbol || !footprint::valid_id(&unit) || unit == next.unit || next.other_units.contains_key(&unit) {
                            return Err("A new symbol unit needs an unused portable identity.".into());
                        }
                        next.format = MULTI_UNIT_FORMAT.into();
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
                        if primitives.is_empty() || primitives.len() > 512 || primitives.keys().any(|id| !next.objects.contains_key(id)) {
                            return Err("Unknown or empty primitive replacement batch.".into());
                        }
                        next.objects.extend(primitives);
                    }
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
                    Action::Array { objects, columns, rows, column_step, row_step } => {
                        let originals = operations::selected(&next, &objects)?;
                        let count = usize::from(columns) * usize::from(rows);
                        if columns == 0
                            || rows == 0
                            || count > 512
                            || next.objects.len().saturating_add(count.saturating_sub(1).saturating_mul(originals.len())) > 512
                        {
                            return Err("Array exceeds the definition's 512 primitive limit.".into());
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
                    Action::Undo | Action::Redo => return Err("History operation cannot be nested.".into()),
                }
                if next == *self.item {
                    return Ok(self.revision);
                }
                let metadata: BTreeMap<_, _> = next
                    .objects
                    .values()
                    .filter_map(|object| match object {
                        Primitive::Pin { number, name, electrical_type, .. } => Some((number.clone(), (name.clone(), *electrical_type))),
                        _ => None,
                    })
                    .collect();
                for object in next.other_units.values_mut().flat_map(|objects| objects.values_mut()) {
                    if let Primitive::Pin { number, name, electrical_type, .. } = object
                        && let Some((shared_name, shared_type)) = metadata.get(number)
                    {
                        *name = shared_name.clone();
                        *electrical_type = *shared_type;
                    }
                }
                next.revision = self.next_content_revision;
                next.allocation = Allocation { object: cursor - 1, revision: self.next_content_revision };
                next.validate()?;
                let next_content_revision = self.next_content_revision.checked_add(1).ok_or("Content revision limit reached.")?;
                let next_id = cursor;
                self.undo.push(std::mem::replace(&mut self.item, Arc::new(next)));
                self.redo.clear();
                if self.undo.len() > HISTORY {
                    self.undo.remove(0);
                }
                self.next_id = next_id;
                self.next_content_revision = next_content_revision;
            }
        }
        self.revision = revision;
        Ok(revision)
    }
    /// Report document and primitive identities used by the GUI and live tools.
    pub fn inspect(&self) -> Value {
        json!({"revision":self.revision,"dirty":self.dirty(),"published":self.published,"publication_dirty":self.publication_dirty(),"saved_content_revision":self.saved.as_ref().map(|item|item.revision),"can_undo":!self.undo.is_empty(),"can_redo":!self.redo.is_empty(),"item":self.item.as_ref()})
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
                    primitive: Primitive::Pin { number, name, electrical_type, at: Point { x_nm: 1, y_nm: 0 }, appearance: None }
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
