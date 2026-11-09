//! The session and command set behind PCBCraft.
//!
//! One rule shapes this crate: anything a person can do, an agent can do through the same command.
//! The editor and MCP server use the same catalog through [`Session::apply`].
//! [`Session::execute`] also includes an inspection result for existing callers.
//! The native codec snapshots authored data independently of editing history and transport.

#[cfg(test)]
mod allocation_tests;
pub mod authoring;
pub mod board;
pub mod hierarchy;
pub mod library;
pub mod library_graphics;
pub mod net_labels;
pub mod persistence;
pub mod schematic;
pub mod selection;
pub mod sheets;
pub mod wire;
pub use board::drawing::{ADD_DRAWING, DELETE_DRAWING, SET_DRAWING};

use board::{Board, FOOTPRINT_DEFINITION, Footprint, Pad};

use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};

use schematic::{
    Component, MAX_CENTER_NM, MAX_SYMBOLS, Point, SymbolDefinition, SymbolPlacement, Terminal, Wire, WireEndpoint, library_symbol,
    wiring::{Connectivity, MAX_WIRES},
};
use serde_json::{Map, Value, json};

/// Undo steps kept. The oldest is dropped past this.
const MAX_HISTORY: usize = 1000;
const MAX_REFERENCE_NUMBER: usize = 1_000_000;
const MAX_OBJECT_ID: u64 = i64::MAX as u64;
/// Longest project name, in characters.
const MAX_NAME: usize = 200;
const NET_NAME: Param =
    Param { name: "name", kind: ParamKind::Text, required: true, summary: "Case-sensitive electrical net name (1–200 characters)." };
const NET_STYLE: Param = Param {
    name: "style",
    kind: ParamKind::NetLabelStyle,
    required: true,
    summary: "Local sheet label, global label, power arrow or ground symbol; power and ground have global scope.",
};
const NET_OBJECT: Param = Param { name: "id", kind: ParamKind::NetLabelId, required: true, summary: "Stable label ID from inspection." };
const NET_ROTATION: Param =
    Param { name: "rotation", kind: ParamKind::Rotation, required: false, summary: "Counterclockwise quarter-turn orientation, default 0." };

/// The name `project.new` gives a project when none is supplied. A desktop project still bearing
/// it takes its folder or archive name when first saved (see [`Session::adopt_saved_name`]).
pub const UNTITLED: &str = "Untitled";

#[derive(Clone, Copy, Debug)]
enum ParamKind {
    CreateBoard,
    NetLabelStyle,
    NetLabelId,
    LibrarySource,
    LibraryDocuments,
    LibraryQuery,
    LibraryKind,
    LibraryDigest,
    DefinitionRevision,
    AutoHeal,
    Disconnect,
    Junctions,
    Name,
    /// Free text for a symbol's reference or value. Its length and characters are checked by the command.
    Text,
    SymbolDefinition,
    SymbolId,
    FootprintDefinition,
    FootprintMapping,
    FootprintId,
    FootprintIds,
    ResolveExistingFootprints,
    BoardRotation,
    PadNumber,
    Coordinate,
    Displacement,
    Revision,
    Terminal,
    Rotation,
    WireAnchor,
    WireEndpoint,
    WireBends,
    WireId,
    JunctionId,
    /// A bounded point in nanometers.
    Point,
    Selection,
    BoardSetup,
    SilkscreenSettings,
    DrawingPoints,
    DrawingId,
    Closed,
    BoardId,
    SheetId,
    CopperLayers,
    HierarchyId,
    Direction,
    SheetExtent,
}

impl ParamKind {
    /// Publish the parameter's JSON type and bounds to command clients.
    fn schema(self) -> Value {
        match self {
            Self::CreateBoard => json!({"type":"boolean"}),
            Self::DefinitionRevision => json!({"type":"integer","minimum":1,"maximum":u32::MAX}),
            Self::LibraryDigest => json!({"type":"string","pattern":"^[0-9a-f]{64}$"}),
            Self::LibrarySource => json!({"type":"string","minLength":1,"maxLength":64}),
            Self::LibraryDocuments => {
                json!({"type":"object","maxProperties":library::sources::MAX_FILES,"additionalProperties":{"type":"string","maxLength":library::sources::MAX_FILE_BYTES}})
            }
            Self::LibraryQuery => json!({"type":"string","maxLength":128}),
            Self::LibraryKind => json!({"type":"string","enum":["symbol","footprint"]}),
            Self::DrawingPoints => board::drawing::geometry_schema(),
            Self::DrawingId => json!({"type":"string","format":"uuid"}),
            Self::Closed => json!({"type":"boolean"}),
            Self::SheetExtent => json!({"type":"integer","minimum":1,"maximum":2*MAX_CENTER_NM}),
            Self::HierarchyId => json!({"type":"string", "pattern":"^(sheet-symbol|label)-[1-9][0-9]*$"}),
            Self::Direction => json!({"type":"string", "enum":hierarchy::Direction::ALL.map(hierarchy::Direction::as_str)}),
            Self::BoardSetup => board::setup::BoardSetup::schema(),
            Self::SilkscreenSettings => json!({"type":"object","properties": {
                "text_height_nm":{"type":"integer","minimum":1,"maximum":10_000_000},
                "stroke_width_nm":{"type":"integer","minimum":1,"maximum":5_000_000},
                "pad_clearance_nm":{"type":"integer","minimum":0,"maximum":10_000_000}
            },"required":["text_height_nm","stroke_width_nm","pad_clearance_nm"],"additionalProperties":false}),
            Self::BoardId | Self::SheetId => json!({"type":"string", "format":"uuid"}),
            Self::CopperLayers => json!({"type":"integer", "minimum":2,"maximum":64,"multipleOf":2}),
            Self::AutoHeal => json!({"type":"boolean","default":true}),
            Self::Disconnect => json!({"type":"boolean","default":false}),
            Self::Junctions => json!({"type":"array","maxItems":2,"uniqueItems":true,"items":Self::JunctionId.schema()}),
            Self::Selection => json!({"type":"object", "properties": {
                "symbols": {"type":"array", "maxItems":MAX_SYMBOLS, "uniqueItems":true, "items":Self::SymbolId.schema()},
                "wires": {"type":"array", "maxItems":MAX_WIRES, "uniqueItems":true, "items":Self::WireId.schema()},
                "net_labels": {"type":"array", "maxItems":net_labels::MAX_LABELS, "uniqueItems":true, "items":Self::NetLabelId.schema()}
            }, "required":["symbols","wires"], "additionalProperties":false}),
            Self::NetLabelStyle => json!({"type":"string","enum":["local","global","power","ground"]}),
            Self::NetLabelId => json!({"type":"string","pattern":"^net-label-[1-9][0-9]*$"}),
            Self::Name | Self::Text => json!({"type": "string"}),
            Self::BoardRotation => json!({"type":"integer", "minimum":0, "maximum":359}),
            Self::FootprintId => json!({"type":"string", "pattern":"^footprint-[1-9][0-9]*$"}),
            Self::FootprintIds => json!({"type":"array","minItems":1,"maxItems":MAX_SYMBOLS,"uniqueItems":true,"items":Self::FootprintId.schema()}),
            Self::ResolveExistingFootprints => json!({"type":"boolean"}),
            Self::SymbolId => json!({"type": "string", "pattern": "^symbol-[1-9][0-9]*$"}),
            Self::FootprintDefinition => {
                json!({"type":"string", "enum":std::iter::once(FOOTPRINT_DEFINITION).chain(board::definition::STARTER_IDS).collect::<Vec<_>>()})
            }
            Self::FootprintMapping => {
                json!({"type":"object","minProperties":1,"maxProperties":128,"additionalProperties":{"type":"string","pattern":"^[A-Za-z0-9+_-]{1,8}$"}})
            }
            Self::PadNumber => json!({"type":"integer", "enum":[1, 2]}),
            Self::SymbolDefinition => json!({"type":"string","minLength":1,"maxLength":64}),
            Self::Displacement => json!({"type": "integer", "minimum": -2 * MAX_CENTER_NM, "maximum": 2 * MAX_CENTER_NM}),
            Self::Coordinate => json!({"type": "integer", "minimum": -MAX_CENTER_NM, "maximum": MAX_CENTER_NM}),
            Self::WireId => json!({"type": "string", "pattern": "^wire-[1-9][0-9]*$"}),
            Self::JunctionId => json!({"type": "string", "pattern": "^junction-[1-9][0-9]*$"}),
            Self::WireAnchor => {
                json!({"oneOf": [ParamKind::Terminal.schema(), {
                "type": "object", "properties": {"junction": {"type": "string", "pattern": "^junction-[1-9][0-9]*$"}}, "required": ["junction"], "additionalProperties": false
            }, point_schema(), {"type":"object","properties":{"wire":ParamKind::WireId.schema(),"at":point_schema()},"required":["wire","at"],"additionalProperties":false}, hierarchy_endpoint_schema("label"), hierarchy_endpoint_schema("sheet_pin"), hierarchy_endpoint_schema("net_label")]})
            }
            Self::WireEndpoint => {
                json!({"oneOf": [ParamKind::Terminal.schema(), {
                "type": "object", "properties": {"junction": ParamKind::JunctionId.schema()}, "required": ["junction"], "additionalProperties": false
            }, {"type":"object","properties":{"wire":ParamKind::WireId.schema(),"at":point_schema()},"required":["wire","at"],"additionalProperties":false}, hierarchy_endpoint_schema("label"), hierarchy_endpoint_schema("sheet_pin"), hierarchy_endpoint_schema("net_label")]})
            }
            Self::WireBends => json!({"type": "array", "maxItems": wire::MAX_BENDS, "items": point_schema()}),
            Self::Point => point_schema(),
            Self::Rotation => json!({"type": "integer", "enum": [0, 90, 180, 270]}),
            Self::Revision => json!({"type": "integer", "minimum": 0}),
            Self::Terminal => json!({
                "type": "object",
                "properties": {
                    "symbol": {"type": "string", "pattern": "^symbol-[1-9][0-9]*$"},
                    "terminal": {"type": "string", "pattern": "^[A-Za-z0-9+_-]{1,8}$"},
                },
                "required": ["symbol", "terminal"],
                "additionalProperties": false,
            }),
        }
    }

    /// Check the raw JSON value before a command reads it, without coercing strings or fractions.
    fn validate(self, value: &Value) -> Result<(), &'static str> {
        match self {
            Self::CreateBoard if value.is_boolean() => Ok(()),
            Self::CreateBoard => Err("must be a boolean"),
            Self::DefinitionRevision if value.as_u64().is_some_and(|n| n > 0 && n <= u32::MAX as u64) => Ok(()),
            Self::DefinitionRevision => Err("must be a positive 32-bit native definition revision"),
            Self::LibraryDigest
                if value.as_str().is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))) =>
            {
                Ok(())
            }
            Self::LibraryDigest => Err("must contain a lowercase SHA-256 content digest"),
            Self::LibrarySource if value.as_str().is_some_and(board::definition::valid_id) => Ok(()),
            Self::LibraryDocuments if library::sources::valid_documents_value(value) => Ok(()),
            Self::LibraryQuery if value.as_str().is_some_and(|s| s.len() <= 128) => Ok(()),
            Self::LibraryKind if matches!(value.as_str(), Some("symbol" | "footprint")) => Ok(()),
            Self::LibrarySource => Err("must be a portable source or library ID"),
            Self::LibraryDocuments => Err("must contain bounded native path-to-text entries"),
            Self::LibraryQuery => Err("must contain at most 128 UTF-8 bytes"),
            Self::LibraryKind => Err("must be symbol or footprint"),
            Self::DrawingPoints
                if value.as_array().is_some_and(|v| (2..=256).contains(&v.len()) && v.iter().all(|p| Self::Point.validate(p).is_ok())) =>
            {
                Ok(())
            }
            Self::DrawingId if value.as_str().is_some_and(|v| uuid::Uuid::parse_str(v).is_ok_and(|id| !id.is_nil())) => Ok(()),
            Self::Closed if value.is_boolean() => Ok(()),
            Self::DrawingPoints => Err("must contain 2–256 bounded vertices"),
            Self::DrawingId => Err("must identify a drawing or layer by non-nil UUID"),
            Self::Closed => Err("must be a boolean"),
            Self::HierarchyId
                if value.as_str().is_some_and(|s| {
                    schematic::wiring::parse_id(s, "label-").is_some() || schematic::wiring::parse_id(s, "sheet-symbol-").is_some()
                }) =>
            {
                Ok(())
            }
            Self::SheetExtent if value.as_i64().is_some_and(|n| (1..=2 * MAX_CENTER_NM).contains(&n)) => Ok(()),
            Self::Direction if serde_json::from_value::<hierarchy::Direction>(value.clone()).is_ok() => Ok(()),
            Self::BoardId | Self::SheetId if value.as_str().is_some_and(|id| uuid::Uuid::parse_str(id).is_ok()) => Ok(()),
            Self::BoardSetup | Self::SilkscreenSettings if value.is_object() => Ok(()),
            Self::CopperLayers if value.as_u64().is_some_and(|n| (2..=64).contains(&n) && n.is_multiple_of(2)) => Ok(()),
            Self::AutoHeal | Self::Disconnect if value.is_boolean() => Ok(()),
            Self::Junctions
                if value.as_array().is_some_and(|ends| {
                    ends.len() <= 2 && ends.iter().all(|end| Self::JunctionId.validate(end).is_ok()) && (ends.len() < 2 || ends[0] != ends[1])
                }) =>
            {
                Ok(())
            }
            Self::Selection if selection::Selection::parse(value).is_some() => Ok(()),
            Self::FootprintDefinition
                if value.as_str().is_some_and(|id| id == FOOTPRINT_DEFINITION || board::definition::STARTER_IDS.contains(&id)) =>
            {
                Ok(())
            }
            Self::FootprintMapping if board::parse_mapping(value).is_some() => Ok(()),
            Self::BoardRotation if value.as_u64().is_some_and(|degrees| degrees < 360) => Ok(()),
            Self::FootprintId if value.as_str().and_then(|id| schematic::wiring::parse_id(id, "footprint-")).is_some() => Ok(()),
            Self::FootprintIds
                if value.as_array().is_some_and(|ids| {
                    !ids.is_empty()
                        && ids.len() <= MAX_SYMBOLS
                        && ids.iter().all(|id| Self::FootprintId.validate(id).is_ok())
                        && ids.iter().filter_map(Value::as_str).collect::<std::collections::BTreeSet<_>>().len() == ids.len()
                }) =>
            {
                Ok(())
            }
            Self::ResolveExistingFootprints if value.is_boolean() => Ok(()),
            Self::PadNumber if matches!(value.as_u64(), Some(1 | 2)) => Ok(()),
            Self::NetLabelStyle if serde_json::from_value::<net_labels::Style>(value.clone()).is_ok() => Ok(()),
            Self::NetLabelId if value.as_str().and_then(|id| schematic::wiring::parse_id(id, "net-label-")).is_some() => Ok(()),
            Self::Name | Self::Text if value.is_string() => Ok(()),
            Self::SymbolId if value.as_str().and_then(schematic::parse_symbol_id).is_some() => Ok(()),
            Self::SymbolDefinition if value.as_str().is_some_and(board::definition::valid_id) => Ok(()),
            Self::Coordinate if value.as_i64().is_some_and(|n| (-MAX_CENTER_NM..=MAX_CENTER_NM).contains(&n)) => Ok(()),
            Self::Displacement if value.as_i64().is_some_and(|n| (-2 * MAX_CENTER_NM..=2 * MAX_CENTER_NM).contains(&n)) => Ok(()),
            Self::Revision if value.as_u64().is_some() => Ok(()),
            Self::WireId if value.as_str().and_then(|id| schematic::wiring::parse_id(id, "wire-")).is_some() => Ok(()),
            Self::JunctionId if value.as_str().and_then(|id| schematic::wiring::parse_id(id, "junction-")).is_some() => Ok(()),
            Self::WireAnchor if wire::Anchor::parse(value).is_some() => Ok(()),
            Self::WireEndpoint if matches!(wire::Anchor::parse(value), Some(wire::Anchor::Existing(_) | wire::Anchor::Split { .. })) => Ok(()),
            Self::WireBends if wire::parse_bends(value).is_some() => Ok(()),
            Self::Point if serde_json::from_value::<schematic::Point>(value.clone()).ok().is_some_and(|point| wire::valid_point(&point)) => Ok(()),
            Self::Rotation if matches!(value.as_u64(), Some(0 | 90 | 180 | 270)) => Ok(()),
            Self::Terminal if Terminal::parse(value).is_some() => Ok(()),
            Self::Selection => Err("must contain bounded unique symbol and wire ID lists"),
            Self::HierarchyId => Err("must identify a sheet symbol or hierarchical label"),
            Self::SheetExtent => Err("must be a positive extent within schematic bounds"),
            Self::Direction => Err("must be input, output, bidirectional, tri_state or passive"),
            Self::SheetId => Err("must identify a sheet by UUID"),
            Self::BoardId => Err("must identify a board by UUID"),
            Self::BoardSetup | Self::SilkscreenSettings => Err("must contain a settings object"),
            Self::CopperLayers => Err("must select an even copper-layer count from 2 through 64"),
            Self::AutoHeal | Self::Disconnect => Err("must be a boolean"),
            Self::Junctions => Err("must contain at most two distinct junction IDs"),
            Self::FootprintDefinition => Err("must identify an available built-in footprint"),
            Self::FootprintMapping => Err("must map terminal IDs to pad IDs of 1 to 8 letters, digits, +, - or _"),
            Self::BoardRotation => Err("must be a whole-degree orientation from 0 through 359"),
            Self::FootprintId => Err("must identify a placed footprint"),
            Self::FootprintIds => Err("must contain unique placed footprint IDs within the symbol limit"),
            Self::ResolveExistingFootprints => Err("must be a boolean"),
            Self::PadNumber => Err("must be physical pad 1 or 2"),
            Self::NetLabelStyle => Err("must be local, global, power or ground"),
            Self::NetLabelId => Err("must identify a named net label"),
            Self::Name | Self::Text => Err("must be a string"),
            Self::SymbolId => Err("must identify a placed symbol"),
            Self::SymbolDefinition => Err("must identify a native symbol"),
            Self::Coordinate => Err("must be a whole number of nanometers within the schematic bounds"),
            Self::Displacement => Err("must be a whole displacement within twice the schematic coordinate bounds"),
            Self::Revision => Err("must be a nonnegative integer revision"),
            Self::WireId => Err("must identify a wire"),
            Self::JunctionId => Err("must identify a junction or free endpoint"),
            Self::WireAnchor => Err("must identify a terminal, junction or bounded free point"),
            Self::WireEndpoint => Err("must identify an existing terminal, junction or point on a wire"),
            Self::WireBends => Err("must contain at most 64 bounded points"),
            Self::Point => Err("must be a point with whole-nanometer x_nm and y_nm within the schematic bounds"),
            Self::Rotation => Err("must be 0, 90, 180 or 270 degrees"),
            Self::Terminal => Err("must identify a symbol and one of its terminal numbers"),
        }
    }
}

/// Publish the strict bounded point representation used by free wire endpoints and bends.
fn point_schema() -> Value {
    json!({"type": "object", "properties": {"x_nm": ParamKind::Coordinate.schema(), "y_nm": ParamKind::Coordinate.schema()}, "required": ["x_nm", "y_nm"], "additionalProperties": false})
}

/// Publish a strict fixed interface endpoint alongside ordinary terminals and junctions.
fn hierarchy_endpoint_schema(kind: &str) -> Value {
    let mut properties = Map::new();
    properties
        .insert(kind.into(), json!({"type":"string", "pattern":if kind == "net_label" { "^net-label-[1-9][0-9]*$" } else { "^label-[1-9][0-9]*$" }}));
    json!({"type":"object", "properties":properties, "required":[kind], "additionalProperties":false})
}

/// A parameter of a command.
#[derive(Clone, Copy, Debug)]
pub struct Param {
    /// Whether omission is rejected before the command executes.
    pub required: bool,
    pub name: &'static str,
    pub summary: &'static str,
    kind: ParamKind,
}

/// A command as people and agents see it.
#[derive(Clone, Copy, Debug)]
pub struct Command {
    pub id: &'static str,
    /// What a menu calls it.
    pub title: &'static str,
    pub summary: &'static str,
    /// Required and optional inputs published by command discovery.
    pub params: &'static [Param],
}

impl Command {
    /// Describe the command and its required inputs for discovery by MCP clients.
    pub fn to_json(&self) -> Value {
        let mut params: Map<String, Value> = self
            .params
            .iter()
            .map(|p| {
                let mut schema = p.kind.schema();
                schema["description"] = json!(p.summary);
                (p.name.to_owned(), schema)
            })
            .collect();
        if sheets::scoped_command(self.id) {
            params.insert("sheet".to_owned(), json!({"type":"string", "format":"uuid", "description":"Sheet UUID from inspection. Required when the project has multiple sheets; omission is compatible with one-sheet projects."}));
        }
        let required: Vec<&str> = self.params.iter().filter(|p| p.required).map(|p| p.name).collect();
        json!({
            "id": self.id,
            "title": self.title,
            "summary": self.summary,
            "params": {"type": "object", "properties": params, "required": required, "additionalProperties": false},
        })
    }
}

const NAME: Param = Param { required: true, name: "name", summary: "The project's name", kind: ParamKind::Name };
const NEW_NAME: Param = Param { required: false, name: "name", summary: "The project's name (default Untitled)", kind: ParamKind::Name };
const NEW_BOARD_NAME: Param = Param { required: false, name: "name", summary: "The board's name (default Board)", kind: ParamKind::Name };
/// Compatibility command for callers that placed resistors before the symbol picker.
pub const PLACE_RESISTOR: &str = "schematic.place_resistor";
/// Place the specified starter library symbol on the schematic.
pub const PLACE_SYMBOL: &str = "schematic.place_symbol";
/// The command that joins two inspected symbol terminals with an undoable wire.
pub const CONNECT_PINS: &str = "schematic.connect_pins";
/// Draw a complete orthogonal wire between terminals, junctions or new free endpoints.
pub const DRAW_WIRE: &str = "schematic.draw_wire";
/// Remove one wire while preserving all other edges in its net.
pub const DELETE_WIRE: &str = "schematic.delete_wire";
/// Split a wire into two at a point on it, still connected through a new junction.
pub const SLICE_WIRE: &str = "schematic.slice_wire";
pub const MOVE_WIRE_SEGMENT: &str = "schematic.move_wire_segment";
pub const MOVE_WIRE_CORNER: &str = "schematic.move_wire_corner";
pub const MOVE_JUNCTION: &str = "schematic.move_junction";
/// Attach a free endpoint or junction and all its routes to an existing terminal or junction.
pub const ATTACH_JUNCTION: &str = "schematic.attach_junction";
/// Normalize selected wires and connect exact pin contacts as one undoable edit.
pub const HEAL_WIRES: &str = "schematic.heal_wires";
/// Detach one straight leg from its neighboring legs, pins and shared junctions.
pub const BREAK_WIRE: &str = "schematic.break_wire";
/// Remove one straight leg of a wire, leaving the rest of its route with free ends.
pub const DELETE_WIRE_SEGMENT: &str = "schematic.delete_wire_segment";
/// Move a symbol, optionally disconnecting it while preserving its old wire paths.
pub const MOVE_SYMBOL: &str = "schematic.move_symbol";
/// Turn a symbol a quarter turn counterclockwise.
pub const ROTATE_SYMBOL: &str = "schematic.rotate_symbol";
/// Remove a symbol and the wires attached to it.
pub const DELETE_SYMBOL: &str = "schematic.delete_symbol";
/// Set component fields and optional placed-symbol coordinates/orientation in one undo step.
pub const SET_SYMBOL_FIELDS: &str = "schematic.set_symbol_fields";
/// Assign a pinned generic footprint and record its physical pad mapping.
pub const PLACE_FOOTPRINT: &str = "pcb.place_footprint";
pub use board::assignment::{ASSIGN_FOOTPRINT, PLACE_ASSIGNED_FOOTPRINT};
/// Set a footprint's board-local position and counterclockwise whole-degree orientation atomically.
pub const SET_FOOTPRINT_TRANSFORM: &str = "pcb.set_footprint_transform";
/// Remove board placements while retaining components and physical package assignments.
pub const DELETE_FOOTPRINTS: &str = "pcb.delete_footprints";
/// Initialize stable layers and a rigid stackup, explicitly opting in to existing pad resolution.
pub const INITIALIZE_BOARD_SETUP: &str = "pcb.initialize_board_setup";
/// Atomically replace a validated board setup while retaining its profile identity.
pub const SET_BOARD_SETUP: &str = "pcb.set_board_setup";
/// Set board-local physical silkscreen text height, stroke width and pad clearance.
pub const SET_SILKSCREEN: &str = "pcb.set_silkscreen";
const BOARD: Param = Param {
    required: false,
    name: "board",
    summary: "Board UUID from inspection; omission uses the single-board compatibility default",
    kind: ParamKind::BoardId,
};
const EXPECTED_REVISION: Param =
    Param { required: true, name: "expected_revision", summary: "Revision returned by inspect_project", kind: ParamKind::Revision };
const AUTO_HEAL: Param = Param {
    required: false,
    name: "auto_heal",
    summary: "Clean up touched wire overlaps, returns and redundant loops; defaults to true. False preserves authored routes; explicit attachments still connect.",
    kind: ParamKind::AutoHeal,
};

/// Move a group of symbols and wire routes as one revision and undo step.
pub const MOVE_ITEMS: &str = "schematic.move_items";
/// Delete a group and its incident connections as one revision and undo step.
pub const DELETE_ITEMS: &str = "schematic.delete_items";
const ITEMS: Param = Param { required: true, name: "items", summary: "Symbol and wire IDs from project inspection", kind: ParamKind::Selection };

const WIRE: Param = Param { required: true, name: "wire", summary: "Wire ID from inspection", kind: ParamKind::WireId };
const SYMBOL: Param = Param { required: true, name: "symbol", summary: "Placed symbol ID from project inspection", kind: ParamKind::SymbolId };
const SYMBOL_X: Param =
    Param { required: true, name: "x_nm", summary: "Symbol center X in nanometers, increasing right", kind: ParamKind::Coordinate };
const SYMBOL_Y: Param =
    Param { required: true, name: "y_nm", summary: "Symbol center Y in nanometers, increasing down", kind: ParamKind::Coordinate };

const HIERARCHY_OBJECT: Param =
    Param { required: true, name: "id", summary: "Sheet-symbol or label ID from inspection", kind: ParamKind::HierarchyId };
const INTERFACE_NAME: Param = Param { required: true, name: "name", summary: "Interface name, unique within the child sheet", kind: ParamKind::Name };
const ANCHOR_X: Param = Param { required: true, name: "x_nm", summary: "Anchor X in nanometers, increasing right", kind: ParamKind::Coordinate };
const ANCHOR_Y: Param = Param { required: true, name: "y_nm", summary: "Anchor Y in nanometers, increasing down", kind: ParamKind::Coordinate };
const DIRECTION: Param = Param { required: true, name: "direction", summary: "Hierarchical interface direction", kind: ParamKind::Direction };
const CATALOG: &[Command] = &[
    Command {
        id: hierarchy::RESIZE_SHEET,
        title: "Resize Sheet Symbol",
        summary: "Resize a sheet rectangle and its fixed interface anchors in one undo step; refuse sizes that cut through pins.",
        params: &[
            HIERARCHY_OBJECT,
            ANCHOR_X,
            ANCHOR_Y,
            Param { required: true, name: "width_nm", summary: "Sheet width in nanometers", kind: ParamKind::SheetExtent },
            Param { required: true, name: "height_nm", summary: "Sheet height in nanometers", kind: ParamKind::SheetExtent },
            EXPECTED_REVISION,
        ],
    },
    Command {
        id: hierarchy::CREATE_CHILD,
        title: "New Hierarchical Sheet",
        summary: "Create a child sheet and place its parent symbol as one undo step.",
        params: &[sheets::SHEET_NAME, ANCHOR_X, ANCHOR_Y, EXPECTED_REVISION],
    },
    Command {
        id: hierarchy::PLACE_SHEET,
        title: "Place Hierarchical Sheet",
        summary: "Place an existing unparented sheet as a child; cycles and reuse are rejected. Undoable.",
        params: &[
            Param { required: true, name: "child", summary: "Child sheet UUID", kind: ParamKind::SheetId },
            ANCHOR_X,
            ANCHOR_Y,
            EXPECTED_REVISION,
        ],
    },
    Command {
        id: hierarchy::MOVE_SHEET,
        title: "Move Sheet Symbol",
        summary: "Move a sheet symbol and its connected pins. Undoable.",
        params: &[HIERARCHY_OBJECT, ANCHOR_X, ANCHOR_Y, EXPECTED_REVISION],
    },
    Command {
        id: hierarchy::REMOVE_SHEET,
        title: "Remove Sheet Symbol",
        summary: "Detach a child to the project root; remove parent wires attached to its pins. Child contents are retained. Undoable.",
        params: &[HIERARCHY_OBJECT, EXPECTED_REVISION],
    },
    Command {
        id: net_labels::ADD,
        title: "Place Net Label",
        summary: "Place a local/global label or power/ground anchor. Matching names connect by scope; a label on a pin or wire connects at its anchor. Undoable.",
        params: &[NET_NAME, NET_STYLE, ANCHOR_X, ANCHOR_Y, NET_ROTATION, EXPECTED_REVISION],
    },
    Command {
        id: net_labels::SET,
        title: "Edit Net Label",
        summary: "Rename, move, rotate or change the scope/style of an existing named anchor. Undoable.",
        params: &[NET_OBJECT, NET_NAME, NET_STYLE, ANCHOR_X, ANCHOR_Y, NET_ROTATION, EXPECTED_REVISION],
    },
    Command {
        id: net_labels::DELETE,
        title: "Delete Net Label",
        summary: "Remove a named anchor and its directly attached wires. Undoable.",
        params: &[NET_OBJECT, EXPECTED_REVISION],
    },
    Command {
        id: hierarchy::ADD_LABEL,
        title: "Add Hierarchical Label",
        summary: "Add a child interface label and synchronized parent pin. Undoable.",
        params: &[INTERFACE_NAME, DIRECTION, ANCHOR_X, ANCHOR_Y, EXPECTED_REVISION],
    },
    Command {
        id: hierarchy::EDIT_LABEL,
        title: "Edit Hierarchical Label",
        summary: "Rename, change direction or move a hierarchical label and its parent pin. Undoable.",
        params: &[HIERARCHY_OBJECT, INTERFACE_NAME, DIRECTION, ANCHOR_X, ANCHOR_Y, EXPECTED_REVISION],
    },
    Command {
        id: hierarchy::DELETE_LABEL,
        title: "Delete Hierarchical Label",
        summary: "Remove a label, parent pin and wires directly attached to either. Undoable.",
        params: &[HIERARCHY_OBJECT, EXPECTED_REVISION],
    },
    Command {
        id: "project.new",
        title: "New Project",
        summary: "Start a new empty project, named Untitled unless a name is given, replacing the open one. Undoable.",
        params: &[
            NEW_NAME,
            Param {
                name: "create_board",
                kind: ParamKind::CreateBoard,
                required: false,
                summary: "Explicitly create the first PCB along with this project; defaults to schematic-only.",
            },
        ],
    },
    Command {
        id: board::identity::CREATE_BOARD,
        title: "Create Board",
        summary: "Create the first physical board in a schematic-only project. Undoable.",
        params: &[NEW_BOARD_NAME, EXPECTED_REVISION],
    },
    Command {
        id: sheets::CREATE_SHEET,
        title: "New Sheet",
        summary: "Create an empty flat sheet. Undoable.",
        params: &[sheets::SHEET_NAME, EXPECTED_REVISION],
    },
    Command {
        id: sheets::RENAME_SHEET,
        title: "Rename Sheet",
        summary: "Rename an explicitly addressed sheet. Undoable.",
        params: &[sheets::SHEET, sheets::SHEET_NAME, EXPECTED_REVISION],
    },
    Command {
        id: sheets::DELETE_SHEET,
        title: "Delete Sheet",
        summary: "Delete a sheet and its placements, wires and associated board footprints. Retain at least one sheet. Undoable.",
        params: &[sheets::SHEET, EXPECTED_REVISION],
    },
    Command { id: "project.rename", title: "Rename Project", summary: "Change the open project's name.", params: &[NAME] },
    Command {
        id: PLACE_RESISTOR,
        title: "Place Resistor",
        summary: "Compatibility command for the starter library's 1k resistor. Use schematic.place_symbol for new calls. Undoable.",
        params: &[SYMBOL_X, SYMBOL_Y, EXPECTED_REVISION, AUTO_HEAL],
    },
    Command {
        id: PLACE_SYMBOL,
        title: "Place Symbol",
        summary: "Place a native symbol on the open schematic. External definitions require library, definition_revision and definition_digest from library.search. Undoable.",
        params: &[
            Param { required: true, name: "definition", summary: "Native symbol item ID", kind: ParamKind::SymbolDefinition },
            Param {
                required: false,
                name: "definition_digest",
                summary: "SHA-256 digest from library.search; required for external symbols",
                kind: ParamKind::LibraryDigest,
            },
            Param { required: false, name: "library", summary: "Native library ID; omit for the embedded starter", kind: ParamKind::LibrarySource },
            Param {
                required: false,
                name: "definition_revision",
                summary: "Exact loaded symbol revision; required for external libraries",
                kind: ParamKind::DefinitionRevision,
            },
            Param { required: false, name: "rotation", summary: "Counterclockwise degrees; defaults to 0", kind: ParamKind::Rotation },
            SYMBOL_X,
            SYMBOL_Y,
            EXPECTED_REVISION,
            AUTO_HEAL,
        ],
    },
    Command {
        id: DRAW_WIRE,
        title: "Draw Wire",
        summary: "Draw an orthogonal wire with explicit endpoint identities and bends. One undoable edit.",
        params: &[
            Param {
                required: true,
                name: "from",
                summary: "Existing endpoint, explicit wire/at target, or new free point",
                kind: ParamKind::WireAnchor,
            },
            Param {
                required: true,
                name: "to",
                summary: "Existing endpoint, explicit wire/at target, or new free point",
                kind: ParamKind::WireAnchor,
            },
            Param { required: true, name: "bends", summary: "Intermediate orthogonal route points, excluding endpoints", kind: ParamKind::WireBends },
            EXPECTED_REVISION,
            AUTO_HEAL,
        ],
    },
    Command {
        id: DELETE_WIRE,
        title: "Delete Wire",
        summary: "Delete one wire and unused free endpoints. Undoable.",
        params: &[WIRE, EXPECTED_REVISION],
    },
    Command {
        id: SLICE_WIRE,
        title: "Slice Wire",
        summary: "Split a wire at a point on its route into two wires joined by a new junction. Connections are unchanged. Undoable.",
        params: &[
            WIRE,
            Param { required: true, name: "at", summary: "Point on the wire's drawn route, not at either end", kind: ParamKind::Point },
            EXPECTED_REVISION,
        ],
    },
    Command {
        id: BREAK_WIRE,
        title: "Break Wire",
        summary: "Detach the selected straight leg from neighboring legs, pins and shared junctions at either end. Already-free ends stay free and all geometry stays put. Undoable.",
        params: &[
            WIRE,
            Param { required: true, name: "at", summary: "Point inside one leg of the wire's drawn route, not on a corner", kind: ParamKind::Point },
            EXPECTED_REVISION,
        ],
    },
    Command {
        id: DELETE_WIRE_SEGMENT,
        title: "Delete Wire Segment",
        summary: "Delete the straight leg of a wire that contains a point, keeping the rest of the route; the cut ends become free endpoints. Undoable.",
        params: &[
            WIRE,
            Param { required: true, name: "at", summary: "Point inside one leg of the wire's drawn route, not on a corner", kind: ParamKind::Point },
            EXPECTED_REVISION,
        ],
    },
    Command {
        id: MOVE_WIRE_SEGMENT,
        title: "Move Wire Segment",
        summary: "Move a detached straight wire in both axes, or slide a connected leg across its direction. Pins and shared junctions stay anchored. Positive overlaps heal; crossings stay separate. Undoable.",
        params: &[
            WIRE,
            Param { required: true, name: "at", summary: "Point inside one leg of the wire's drawn route, not on a corner", kind: ParamKind::Point },
            Param {
                required: true,
                name: "to",
                summary: "Where that point goes; connected legs move only across the leg, detached straight wires move in both axes",
                kind: ParamKind::Point,
            },
            EXPECTED_REVISION,
            AUTO_HEAL,
            Param {
                required: false,
                name: "connect_ends",
                summary: "Free ends of this wire to join to exact stationary pin or wire contacts on drop",
                kind: ParamKind::Junctions,
            },
        ],
    },
    Command {
        id: MOVE_WIRE_CORNER,
        title: "Move Wire Corner",
        summary: "Move an interior corner of a wire; the two legs meeting there slide with it and the wire's endpoints stay put. Positive overlaps heal; crossings stay separate. Undoable.",
        params: &[
            WIRE,
            Param { required: true, name: "at", summary: "An interior corner of the wire's drawn route", kind: ParamKind::Point },
            Param { required: true, name: "to", summary: "The corner's new position", kind: ParamKind::Point },
            EXPECTED_REVISION,
            AUTO_HEAL,
        ],
    },
    Command {
        id: MOVE_JUNCTION,
        title: "Move Junction",
        summary: "Move a free endpoint or junction; the end leg of each attached wire slides to keep its direction. Exact pin contacts connect and positive overlaps heal. Undoable.",
        params: &[
            Param { required: true, name: "junction", summary: "Junction or free endpoint ID from inspection", kind: ParamKind::JunctionId },
            Param { required: true, name: "to", summary: "The new position", kind: ParamKind::Point },
            EXPECTED_REVISION,
            AUTO_HEAL,
        ],
    },
    Command {
        id: HEAL_WIRES,
        title: "Heal Wires",
        summary: "Heal selected wire routes, compatible overlaps and redundant junctions; connect exact pin contacts. Crossings stay separate. Undoable.",
        params: &[ITEMS, EXPECTED_REVISION],
    },
    Command {
        id: ATTACH_JUNCTION,
        title: "Attach Junction",
        summary: "Move a free endpoint or junction onto a pin, junction or explicit wire/at target, connecting every attached route and healing overlaps. Undoable.",
        params: &[
            Param { required: true, name: "junction", summary: "Junction or free endpoint ID from inspection", kind: ParamKind::JunctionId },
            Param {
                required: true,
                name: "to",
                summary: "Existing terminal, junction or wire/at point to connect to",
                kind: ParamKind::WireEndpoint,
            },
            EXPECTED_REVISION,
            AUTO_HEAL,
        ],
    },
    Command {
        id: CONNECT_PINS,
        title: "Connect Pins",
        summary: "Join two existing schematic terminals. Undoable.",
        params: &[
            Param { required: true, name: "from", summary: "First symbol ID and terminal number from project inspection", kind: ParamKind::Terminal },
            Param { required: true, name: "to", summary: "Second symbol ID and terminal number from project inspection", kind: ParamKind::Terminal },
            EXPECTED_REVISION,
        ],
    },
    Command {
        id: MOVE_SYMBOL,
        title: "Move Symbol",
        summary: "Move a symbol, retaining attached wires by default. With disconnect, leave its wiring in place. Exact destination contacts still connect. Undoable.",
        params: &[
            Param { required: true, name: "symbol", summary: "Placed symbol ID from project inspection", kind: ParamKind::SymbolId },
            SYMBOL_X,
            SYMBOL_Y,
            EXPECTED_REVISION,
            AUTO_HEAL,
            Param {
                required: false,
                name: "disconnect",
                summary: "Leave the old wires in place with free ends before moving the symbol",
                kind: ParamKind::Disconnect,
            },
        ],
    },
    Command {
        id: ROTATE_SYMBOL,
        title: "Rotate Symbol",
        summary: "Turn a symbol 90 degrees counterclockwise. Its wires follow its pins. Undoable.",
        params: &[SYMBOL, EXPECTED_REVISION],
    },
    Command {
        id: DELETE_SYMBOL,
        title: "Delete Symbol",
        summary: "Remove a symbol with the wires attached to it and its footprint. Undoable.",
        params: &[SYMBOL, EXPECTED_REVISION],
    },
    Command {
        id: SET_SYMBOL_FIELDS,
        title: "Edit Symbol Fields",
        summary: "Set reference, value and optional placement together. Attached wires follow position changes. References are unique and have no spaces. Undoable.",
        params: &[
            SYMBOL,
            Param { required: true, name: "reference", summary: "Reference designator, such as R7", kind: ParamKind::Text },
            Param { required: true, name: "value", summary: "Value text, such as 4.7k", kind: ParamKind::Text },
            Param {
                required: false,
                name: "rotation",
                summary: "Counterclockwise degrees; omission preserves orientation",
                kind: ParamKind::Rotation,
            },
            Param { required: false, ..SYMBOL_X },
            Param { required: false, ..SYMBOL_Y },
            EXPECTED_REVISION,
            AUTO_HEAL,
        ],
    },
    Command {
        id: PLACE_FOOTPRINT,
        title: "Place Footprint",
        summary: "Place a built-in footprint for a schematic component with validated terminal-to-pad mapping. Undoable.",
        params: &[
            BOARD,
            Param { required: true, name: "symbol", summary: "Placed symbol ID from project inspection", kind: ParamKind::SymbolId },
            Param { required: true, name: "definition", summary: "Pinned built-in footprint definition", kind: ParamKind::FootprintDefinition },
            Param { required: false, name: "pin_1_pad", summary: "Two-pin shorthand; mutually exclusive with mapping", kind: ParamKind::PadNumber },
            Param {
                required: false,
                name: "mapping",
                summary: "Explicit terminal-to-pad bijection; required for other terminal sets",
                kind: ParamKind::FootprintMapping,
            },
            Param { required: true, name: "x_nm", summary: "Footprint center X in nanometers", kind: ParamKind::Coordinate },
            Param { required: true, name: "y_nm", summary: "Footprint center Y in nanometers", kind: ParamKind::Coordinate },
            EXPECTED_REVISION,
        ],
    },
    Command {
        id: ASSIGN_FOOTPRINT,
        title: "Assign Footprint",
        summary: "Pin an exact physical package and explicit terminal mapping without placing it. Changing a placed package requires a reviewed update.",
        params: &[
            BOARD,
            Param { required: true, name: "component", summary: "Stable component ID from project inspection", kind: ParamKind::Text },
            Param { required: true, name: "library", summary: "Exact footprint library identity", kind: ParamKind::Text },
            Param { required: true, name: "definition", summary: "Exact physical footprint identity", kind: ParamKind::Text },
            Param { required: true, name: "definition_revision", summary: "Pinned content revision", kind: ParamKind::Revision },
            Param {
                required: true,
                name: "mapping",
                summary: "Explicit bijection from logical terminals to pad numbers",
                kind: ParamKind::FootprintMapping,
            },
            EXPECTED_REVISION,
        ],
    },
    Command {
        id: PLACE_ASSIGNED_FOOTPRINT,
        title: "Place Assigned Footprint",
        summary: "Place an assigned component using only its project-pinned package and mapping. Undoable.",
        params: &[
            BOARD,
            Param { required: true, name: "component", summary: "Assigned component ID", kind: ParamKind::Text },
            Param { required: true, name: "x_nm", summary: "Footprint origin X in nanometers", kind: ParamKind::Coordinate },
            Param { required: true, name: "y_nm", summary: "Footprint origin Y in nanometers", kind: ParamKind::Coordinate },
            EXPECTED_REVISION,
        ],
    },
    Command {
        id: SET_FOOTPRINT_TRANSFORM,
        title: "Move or Rotate Footprint",
        summary: "Set one footprint's center and counterclockwise orientation without changing its pad mapping. One undoable edit; unchanged values are a no-op.",
        params: &[
            BOARD,
            Param { required: true, name: "footprint", summary: "Placed footprint ID from board inspection", kind: ParamKind::FootprintId },
            Param { required: true, name: "x_nm", summary: "Footprint center X in nanometers", kind: ParamKind::Coordinate },
            Param { required: true, name: "y_nm", summary: "Footprint center Y in nanometers", kind: ParamKind::Coordinate },
            Param {
                required: true,
                name: "rotation",
                summary: "Counterclockwise whole degrees from 0 through 359 around the footprint center",
                kind: ParamKind::BoardRotation,
            },
            EXPECTED_REVISION,
        ],
    },
    Command {
        id: DELETE_FOOTPRINTS,
        title: "Delete Footprints",
        summary: "Remove selected board placements in one undo step. Retain schematic components and pinned physical package assignments.",
        params: &[
            BOARD,
            Param { required: true, name: "footprints", summary: "Unique placed footprint IDs from board inspection", kind: ParamKind::FootprintIds },
            EXPECTED_REVISION,
        ],
    },
    Command {
        id: MOVE_ITEMS,
        title: "Move Selected Items",
        summary: "Move symbols, named-net anchors and wire routes together; retain attached pins, connect exact pin contacts and heal touched routes. One undoable edit.",
        params: &[
            ITEMS,
            Param { required: true, name: "dx_nm", summary: "Horizontal displacement in nanometers", kind: ParamKind::Displacement },
            Param { required: true, name: "dy_nm", summary: "Vertical displacement in nanometers", kind: ParamKind::Displacement },
            EXPECTED_REVISION,
            AUTO_HEAL,
        ],
    },
    Command {
        id: DELETE_ITEMS,
        title: "Delete Selected Items",
        summary: "Remove symbols, named-net anchors, selected and incident wires, and unused components/footprints. One undoable edit.",
        params: &[ITEMS, EXPECTED_REVISION],
    },
    Command {
        id: INITIALIZE_BOARD_SETUP,
        title: "Initialize Board Setup",
        summary: "Create stable layers and a rigid stackup. Existing placements require explicit binding resolution; thickness and materials remain unspecified. Undoable.",
        params: &[
            BOARD,
            Param { required: true, name: "copper_layers", summary: "Even copper-layer count from 2 through 64", kind: ParamKind::CopperLayers },
            Param {
                required: false,
                name: "resolve_existing_footprints",
                summary: "Explicitly resolve existing component-side pads to front copper and all-copper pads across the stack; retain marker-only pads",
                kind: ParamKind::ResolveExistingFootprints,
            },
            Param {
                required: false,
                name: "silkscreen",
                summary: "Board print settings to apply atomically with setup",
                kind: ParamKind::SilkscreenSettings,
            },
            Param {
                required: false,
                name: "setup",
                summary: "Complete staged setup; its copper count must match copper_layers and occupied drawings must retain valid layers",
                kind: ParamKind::BoardSetup,
            },
            EXPECTED_REVISION,
        ],
    },
    Command {
        id: SET_BOARD_SETUP,
        title: "Set Board Setup",
        summary: "Validate and replace layers and physical stackup in one undo step. Populated topology changes require explicit compatible-binding consent.",
        params: &[
            BOARD,
            Param {
                required: true,
                name: "setup",
                summary: "Complete setup from inspection; lengths are exact integer nanometers",
                kind: ParamKind::BoardSetup,
            },
            Param {
                required: false,
                name: "resolve_existing_footprints",
                summary: "Explicitly adopt changed front/all-copper spans while retaining surviving layer roles and identities",
                kind: ParamKind::ResolveExistingFootprints,
            },
            Param {
                required: false,
                name: "silkscreen",
                summary: "Board print settings to apply atomically with setup",
                kind: ParamKind::SilkscreenSettings,
            },
            EXPECTED_REVISION,
        ],
    },
    Command {
        id: SET_SILKSCREEN,
        title: "Set Silkscreen Settings",
        summary: "Set physical text height, stroke width and copper-pad clearance for this board. Undoable.",
        params: &[
            BOARD,
            Param {
                required: true,
                name: "settings",
                summary: "Physical silkscreen settings in integer nanometers",
                kind: ParamKind::SilkscreenSettings,
            },
            EXPECTED_REVISION,
        ],
    },
    library::sources::COMMANDS[0],
    library::sources::COMMANDS[1],
    library::sources::COMMANDS[2],
    library::sources::COMMANDS[3],
    Command {
        id: ADD_DRAWING,
        title: "Draw Board Edge",
        summary: "Add one open or closed graphic on a Board Edge layer, with one undo step.",
        params: &[
            BOARD,
            EXPECTED_REVISION,
            Param { name: "layer", required: true, summary: "Board Edge layer UUID from inspection", kind: ParamKind::DrawingId },
            Param { name: "points", required: true, summary: "Ordered vertices in integer nanometers", kind: ParamKind::DrawingPoints },
            Param { name: "closed", required: true, summary: "Join the last vertex to the first", kind: ParamKind::Closed },
        ],
    },
    Command {
        id: SET_DRAWING,
        title: "Edit Board Drawing",
        summary: "Replace one layer graphic's vertices without changing its identity or layer.",
        params: &[
            BOARD,
            EXPECTED_REVISION,
            Param { name: "drawing", required: true, summary: "Drawing UUID from inspection", kind: ParamKind::DrawingId },
            Param { name: "points", required: true, summary: "Replacement vertices in integer nanometers", kind: ParamKind::DrawingPoints },
            Param { name: "closed", required: true, summary: "Join the last vertex to the first", kind: ParamKind::Closed },
        ],
    },
    Command {
        id: DELETE_DRAWING,
        title: "Delete Board Drawing",
        summary: "Remove one addressed graphic, retaining its layer. One undo step.",
        params: &[
            BOARD,
            EXPECTED_REVISION,
            Param { name: "drawing", required: true, summary: "Drawing UUID from inspection", kind: ParamKind::DrawingId },
        ],
    },
    Command { id: "edit.undo", title: "Undo", summary: "Undo the last change.", params: &[] },
    Command { id: "edit.redo", title: "Redo", summary: "Redo the last undone change.", params: &[] },
];

/// Every command there is.
pub fn catalog() -> &'static [Command] {
    CATALOG
}

/// The command with this id.
pub fn command(id: &str) -> Option<&'static Command> {
    CATALOG.iter().find(|c| c.id == id)
}

/// Why a command did not run. The project is unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandError {
    Unknown(String),
    InvalidParams(String),
    /// The command is real, but can't run in this state, like undo with nothing to undo.
    Unavailable(String),
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(id) => write!(f, "unknown command `{id}`"),
            Self::InvalidParams(m) | Self::Unavailable(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for CommandError {}

/// The persistent address of a schematic sheet within a project. Names and file paths are not
/// identities; this address survives edits and saves, but does not identify a runtime opening.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SheetTarget {
    project: uuid::Uuid,
    sheet: uuid::Uuid,
}

/// A typed project snapshot. History and background saves share immutable collections.
#[derive(Clone, Debug)]
pub struct Project {
    pub name: String,
    id: uuid::Uuid,
    /// Allocation metadata is retained across undo and saved once the watermark format is adopted.
    numeric_id_high_water: Option<u64>,
    /// Ordered authored sheets. A projection chooses a sheet without changing this collection.
    sheets: Arc<Vec<sheets::SchematicSheet>>,
    /// Transient immutable projection; canonical session snapshots always use index zero.
    sheet_index: usize,
    /// Logical components own their definition, reference and value.
    components: Arc<BTreeMap<u64, Component>>,
    /// Derived index of each placed component unit to its symbol ID.
    units: Arc<BTreeMap<(u64, String), u64>>,
    board: Arc<Board>,
    /// Project-wide name resolution shared by all immutable sheet projections.
    named_graph: Arc<OnceLock<Connectivity>>,
    /// The older format this snapshot was migrated from when it was opened, if any.
    upgraded_from: Option<u32>,
}

impl Project {
    /// Stable project identity, available even when no physical board exists.
    pub fn id(&self) -> uuid::Uuid {
        self.id
    }
    /// The board's validated layer catalog and uniform rigid stackup, if explicitly initialized.
    /// Legacy projects retain an unspecified setup until a supported initialization is requested.
    pub fn board_setup(&self) -> Option<&board::setup::BoardSetup> {
        self.board.setup.as_ref()
    }

    /// Address this snapshot’s schematic projection. Session snapshots default to the first sheet.
    pub fn sheet_target(&self) -> SheetTarget {
        SheetTarget { project: self.id, sheet: self.sheet_id }
    }

    /// Return this snapshot's addressed sheet symbols in placement order.
    pub fn symbols(&self) -> &[SymbolPlacement] {
        &self.symbols
    }

    /// Return the logical components in stable component ID order.
    pub fn components(&self) -> impl Iterator<Item = &Component> {
        self.components.values()
    }

    /// Resolve a placement's owner in this snapshot. Foreign placements may have no component.
    pub fn component(&self, symbol: &SymbolPlacement) -> Option<&Component> {
        self.components.get(&symbol.component)
    }

    /// Return the owning component's reference, or empty text for a foreign placement.
    pub fn reference(&self, symbol: &SymbolPlacement) -> String {
        self.component(symbol).map_or_else(String::new, |component| component.reference.clone())
    }

    /// Return the owning component's value, or empty text for a foreign placement.
    pub fn value(&self, symbol: &SymbolPlacement) -> &str {
        self.component(symbol).map_or("", Component::value)
    }

    /// Resolve a component pin through the unit-placement index. Only one unit is supported today.
    pub(crate) fn component_terminal(&self, component: u64, pin: &str) -> Option<Terminal> {
        let definition = &self.components.get(&component)?.definition;
        let symbol = self.units.get(&(component, definition.unit_id().to_owned()))?;
        Terminal::from_number(*symbol, definition, pin)
    }

    /// The older single-document format migrated when this snapshot was opened, such as 1.
    /// Saving does not mutate snapshots; reopening a split project returns `None`.
    pub fn upgraded_from(&self) -> Option<u32> {
        self.upgraded_from
    }

    /// Return committed wires on this snapshot's addressed sheet in placement order.
    pub fn wires(&self) -> &[Wire] {
        &self.wires
    }

    /// Look up a terminal's symbol by its stable ID, without scanning the sheet.
    pub fn terminal_symbol(&self, terminal: Terminal) -> Option<&SymbolPlacement> {
        self.symbol_index(terminal.symbol).map(|index| &self.symbols[index])
    }

    /// Resolve the current coordinates of a terminal in this snapshot.
    pub fn terminal_position(&self, terminal: Terminal) -> Option<Point> {
        let symbol = self.terminal_symbol(terminal)?;
        let index = symbol.definition.pin_index(terminal.number())?;
        symbol.pins().get(index).copied()
    }

    /// Return the cached sheet projection of connectivity. Named anchors resolve project-wide
    /// before projection; geometry edits invalidate their contact graph. Unnamed geometry edits
    /// can continue sharing a topology-only cache.
    pub fn connectivity(&self) -> &Connectivity {
        self.connectivity
            .get_or_init(|| if self.has_named_nets() { self.named_connectivity(false) } else { Connectivity::build(&self.symbols, &self.wires) })
    }

    /// Inspect the schematic in the shared command format, without including the board.
    pub fn inspect_schematic(&self) -> Value {
        json!({
            "symbols": self.symbols.iter().map(|symbol| symbol.inspect(&self.reference(symbol), self.value(symbol))).collect::<Vec<_>>(),
            "wires": self.wires.iter().map(|wire| {
                let mut inspected = wire.inspect();
                inspected["path"] = json!(self.wire_path(wire));
                inspected
            }).collect::<Vec<_>>(),
            "junctions": self.junctions().map(|(id, point)| json!({"id": format!("junction-{id}"), "x_nm": point.x_nm, "y_nm": point.y_nm})).collect::<Vec<_>>(),
            "net_labels": self.net_labels.iter().map(|label| label.inspect()).collect::<Vec<_>>(),
            "netlist": self.connectivity().inspect(),
            "sheet_symbols": self.subsheets.iter().map(|block| json!({"id":format!("sheet-symbol-{}",block.id), "child":block.child,"name":self.child_name(block),"at":block.at,"size":self.sheet_symbol_size(block),"pins":self.sheet_pins(block).map(|pin| json!({"endpoint":WireEndpoint::SheetPin(pin.label.id).inspect(),"name":pin.label.name,"direction":pin.label.direction,"at":pin.at})).collect::<Vec<_>>()})).collect::<Vec<_>>(),
            "hierarchical_labels":self.labels.iter().map(|label|json!({"id":format!("label-{}",label.id),"name":label.name,"direction":label.direction,"at":label.at,"endpoint":WireEndpoint::Label(label.id).inspect()})).collect::<Vec<_>>(),
        })
    }

    /// Whether these snapshots belong to the same project, even across revisions.
    pub fn same_document(&self, other: &Self) -> bool {
        self.id == other.id
    }

    /// Copy this snapshot under another name, for a save that names the project after its file.
    /// The name is trimmed and checked like `project.rename`'s; an invalid one is an error and
    /// the copy shares every other collection with this snapshot.
    pub fn renamed(&self, name: &str) -> Result<Self, CommandError> {
        Ok(Self { name: name_param(&json!({"name": name}))?, ..self.clone() })
    }

    /// Whether two snapshots share the same terminal groups and pin references.
    pub fn same_connectivity(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.connectivity, &other.connectivity)
    }

    /// Find the placed board pad for a terminal on any sheet, if a footprint is assigned.
    pub fn pad_for_terminal(&self, terminal: Terminal) -> Option<Pad> {
        let board = self.board_target()?;
        self.board
            .pad_for(self.all_symbols().find(|symbol| symbol.ordinal == terminal.symbol)?.component, terminal.number())
            .map(|pad| pad.on_board(board.board))
    }

    /// Locate a mapped physical pad in board coordinates.
    pub fn pad_position(&self, pad: Pad) -> Option<Point> {
        let board = self.board_target()?;
        if pad.board.is_some_and(|id| id != board.board) {
            return None;
        }
        self.board.pad_position(pad)
    }

    /// Find the schematic terminal corresponding to a placed board pad.
    pub fn terminal_for_pad(&self, pad: Pad) -> Option<Terminal> {
        let board = self.board_target()?;
        if pad.board.is_some_and(|id| id != board.board) {
            return None;
        }
        let (component, pin) = self.board.component_pin_for(pad)?;
        self.component_terminal(component, pin)
    }

    /// Find the stable symbol ID in the ID-ordered placement records.
    fn symbol_index(&self, id: u64) -> Option<usize> {
        self.symbols.binary_search_by_key(&id, |symbol| symbol.ordinal).ok()
    }
}

/// The open project and its history. Editor views share this authority; headless MCP owns a
/// separate session. Window placement and navigation are not part of the project history.
#[derive(Debug, Default)]
pub struct Session {
    libraries: Arc<library::sources::Catalog>,
    project: Option<Arc<Project>>,
    undo: Vec<Option<Arc<Project>>>,
    redo: Vec<Option<Arc<Project>>>,
    /// Goes up with every change, undo and redo included, so it never repeats.
    revision: u64,
    /// Revision at the latest file adoption. View bindings use it to distinguish reopening the
    /// same persistent project from an edit or undo within the current history.
    history_root_revision: u64,
}

impl Session {
    pub fn project(&self) -> Option<&Project> {
        self.project.as_deref()
    }

    /// Share the exact current snapshot with a background save without copying model data.
    pub fn snapshot(&self) -> Option<Arc<Project>> {
        self.project.clone()
    }

    /// Adopt a validated project as a new history root. Advance past its highest stored ID
    /// so old revision checks fail and newly created objects cannot reuse loaded IDs. Record the
    /// new root revision even when reopening the same snapshot, so views can reset local state.
    /// A sheet projection is normalized to the first sheet without changing authored documents.
    /// Restoring this live project's older content retains its adopted allocation watermark.
    pub fn open_snapshot(&mut self, project: Arc<Project>) {
        let project = self.retain_allocation_watermark(project);
        let highest = project.highest_id();
        self.revision = self.revision.max(highest) + 1;
        self.history_root_revision = self.revision;
        self.project = Some(if project.sheet_index == 0 { project } else { Arc::new(Project { sheet_index: 0, ..Project::clone(&project) }) });
        self.undo.clear();
        self.redo.clear();
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Give the project the name a completed save chose for it, as part of that save rather than
    /// as an edit. `original` is the snapshot the save started from and `saved` its renamed copy.
    /// Every snapshot of that document still bearing `original`'s name, in the open project and in
    /// undo and redo, takes the new name, so undo doesn't bring the old name back; a name the person
    /// set meanwhile is kept. `original` itself is replaced by `saved`, so a caller comparing
    /// snapshots by pointer still sees the saved one. The revision and history depth don't change,
    /// so edits based on the current revision stay valid. Returns whether anything was renamed.
    pub fn adopt_saved_name(&mut self, original: &Arc<Project>, saved: &Arc<Project>) -> bool {
        if !saved.same_document(original) {
            return false;
        }
        let mut renamed = false;
        for slot in std::iter::once(&mut self.project).chain(self.undo.iter_mut()).chain(self.redo.iter_mut()) {
            let Some(project) = slot.as_mut() else { continue };
            if !project.same_document(original) || project.name != original.name {
                continue;
            }
            *project = if Arc::ptr_eq(project, original) {
                Arc::clone(saved)
            } else {
                Arc::new(Project { name: saved.name.clone(), ..Project::clone(project) })
            };
            renamed = true;
        }
        renamed
    }

    /// Return the runtime revision at which a file replaced the history root, or zero before
    /// the first open. Reopening even the same snapshot changes this value; edits and undo do not.
    pub fn history_root_revision(&self) -> u64 {
        self.history_root_revision
    }

    /// Whether the command can run now, for greying out a menu item. Setup initialization
    /// requires an unconfigured board; populated boards require explicit pad-resolution consent
    /// at execution. Setup edits require existing setup. Sheet creation stops at the sheet limit
    /// and deletion retains one sheet. Unknown IDs are unavailable.
    pub fn available(&self, id: &str) -> bool {
        if id == board::identity::CREATE_BOARD {
            return self.project.as_ref().is_some_and(|project| project.board_target().is_none());
        }
        if id.starts_with("pcb.") && !self.project.as_ref().is_some_and(|project| project.board_target().is_some()) {
            return false;
        }
        match id {
            library::sources::LOAD | library::sources::REMOVE | library::sources::LIST | library::sources::SEARCH => true,
            "project.new" => true,
            sheets::CREATE_SHEET => self.project.as_ref().is_some_and(|p| p.sheets.len() < sheets::MAX_SHEETS),
            sheets::DELETE_SHEET => self.project.as_ref().is_some_and(|p| p.sheets.len() > 1),
            net_labels::ADD
            | net_labels::SET
            | net_labels::DELETE
            | hierarchy::CREATE_CHILD
            | hierarchy::PLACE_SHEET
            | hierarchy::RESIZE_SHEET
            | hierarchy::MOVE_SHEET
            | hierarchy::REMOVE_SHEET
            | hierarchy::ADD_LABEL
            | hierarchy::EDIT_LABEL
            | hierarchy::DELETE_LABEL
            | sheets::RENAME_SHEET
            | "project.rename"
            | PLACE_RESISTOR
            | PLACE_SYMBOL
            | CONNECT_PINS
            | MOVE_SYMBOL
            | ROTATE_SYMBOL
            | DELETE_SYMBOL
            | SET_SYMBOL_FIELDS
            | PLACE_FOOTPRINT
            | ASSIGN_FOOTPRINT
            | PLACE_ASSIGNED_FOOTPRINT
            | SET_FOOTPRINT_TRANSFORM
            | DELETE_FOOTPRINTS
            | SET_SILKSCREEN
            | ADD_DRAWING
            | SET_DRAWING
            | DELETE_DRAWING
            | DRAW_WIRE
            | DELETE_WIRE
            | SLICE_WIRE
            | BREAK_WIRE
            | DELETE_WIRE_SEGMENT
            | MOVE_WIRE_SEGMENT
            | MOVE_WIRE_CORNER
            | MOVE_JUNCTION
            | ATTACH_JUNCTION
            | HEAL_WIRES
            | MOVE_ITEMS
            | DELETE_ITEMS => self.project.is_some(),
            INITIALIZE_BOARD_SETUP => self.project.as_ref().is_some_and(|p| p.board.setup.is_none()),
            SET_BOARD_SETUP => self.project.as_ref().is_some_and(|p| p.board.setup.is_some()),
            "edit.undo" => !self.undo.is_empty(),
            "edit.redo" => !self.redo.is_empty(),
            _ => false,
        }
    }

    /// Return the current project, revision, and undo and redo availability as JSON.
    /// Each schematic wire reports explicit component/junction/interface endpoints, authored `bends`, and
    /// its drawn `path`: every corner from end to end in nanometres, so a point on any leg can be
    /// chosen for the commands that take one.
    /// The schematic's `junctions` list provides endpoint IDs and positions; its `netlist`
    /// groups connected terminals and junctions, including junction-only nets, and lists
    /// terminals without a wire as `unconnected`. `sheets` reports ordered sheet identities,
    /// display names, relative document paths and per-sheet schematic inspection. The legacy
    /// top-level schematic is populated only for a single-sheet project. Sheet entries include
    /// their parent UUID; project `netlist` follows the complete parent/child electrical graph.
    /// Physical assignments report their placed/unplaced status; `footprint_catalog` lists exact
    /// available revisions with pad identities and provenance, without duplicating drawing data.
    /// A configured board also reports setup and laminate thickness (null when unspecified).
    pub fn inspect(&self) -> Value {
        let project = self.project.as_ref().map(|p| {
            let mut inspected = json!({
                "name": p.name,
                "boards": p.boards().map(|board| json!({"id":board.target.board, "name":board.name(), "file":"board.json", "layers":board.layers(), "drawings":board.drawings()})).collect::<Vec<_>>(),
                "sheets": p.sheets().map(|sheet| json!({"id":sheet.target.sheet, "name":sheet.name, "parent":p.parent_sheet(sheet.target).map(|s|s.id()), "file":format!("sheets/{}.json",sheet.target.sheet), "schematic":p.for_sheet(sheet.target).map(|sheet|sheet.inspect_schematic())})).collect::<Vec<_>>(),
                "netlist":p.electrical_connectivity().inspect(),
                "components": p.components.values().map(Component::inspect).collect::<Vec<_>>(),
                "schematic": if p.sheets.len()==1 {p.inspect_schematic()} else {Value::Null},
            });
            if !p.board.assignments.is_empty() || p.board.setup.is_some() || p.board.silkscreen.is_some() || p.board.outline.is_some() || p.board.drawings.is_some() || !p.board.footprints().is_empty() {
                inspected["board"] = json!({"id":p.board_target().map(|target|target.board), "footprints": p.board.footprints().iter().map(|footprint| footprint.inspect(p)).collect::<Vec<_>>()});
                inspected["board"]["assignments"] = json!(p.footprint_assignments());
                inspected["board"]["silkscreen"] = json!(p.board.silkscreen.unwrap_or_default());
                inspected["board"]["outline"] = json!(p.board.outline);
                if !p.board.definitions.is_empty() {
                    inspected["board"]["definitions"] = json!(p.board.definitions.values().map(Arc::as_ref).collect::<Vec<_>>());
                }
                if let Some(setup) = &p.board.setup {
                    inspected["board"]["setup"] = json!(setup);
                    inspected["board"]["laminate_thickness_nm"] = json!(setup.laminate_thickness_nm());
                }
            }
            inspected
        });
        json!({
            "project": project,
            "footprint_catalog": self.project.as_ref().map(|p| p.inspect_footprint_catalog()),
            "revision": self.revision,
            "can_undo": !self.undo.is_empty(),
            "can_redo": !self.redo.is_empty(),
        })
    }

    /// Run a shared command. Document commands answer [`Self::inspect`]; library commands return
    /// catalog inspection/search and never change the project or its history. Failed commands
    /// leave the current state intact.
    pub fn execute(&mut self, id: &str, params: &Value) -> Result<Value, CommandError> {
        if id.starts_with("library.") {
            return self.library_command(id, params);
        }
        self.apply(id, params)?;
        Ok(self.inspect())
    }

    /// Run the shared command without building an inspection snapshot. Returns the current
    /// revision, which remains unchanged on a no-op. Failed commands leave the project intact.
    /// Setup handlers retain their revision checks and specific state diagnostics even when
    /// availability disables the command on an open project.
    pub fn apply(&mut self, id: &str, params: &Value) -> Result<u64, CommandError> {
        if id.starts_with("library.") {
            self.library_command(id, params)?;
            return Ok(self.revision);
        }
        if sheets::scoped_command(id) {
            return self.apply_to_sheet(id, params);
        }
        self.apply_inner(id, params)
    }

    /// Dispatch validated commands inside one atomic sheet transaction or the project session.
    /// External symbol placement names missing revision/digest guards before catalog resolution.
    fn apply_inner(&mut self, id: &str, params: &Value) -> Result<u64, CommandError> {
        let command = command(id).ok_or_else(|| CommandError::Unknown(id.to_owned()))?;
        check_params(command, params)?;
        if matches!(
            id,
            ASSIGN_FOOTPRINT
                | PLACE_ASSIGNED_FOOTPRINT
                | PLACE_FOOTPRINT
                | SET_FOOTPRINT_TRANSFORM
                | DELETE_FOOTPRINTS
                | INITIALIZE_BOARD_SETUP
                | SET_BOARD_SETUP
                | SET_SILKSCREEN
                | ADD_DRAWING
                | SET_DRAWING
                | DELETE_DRAWING
        ) {
            self.open_project()?.resolve_board(params.get("board").and_then(Value::as_str))?;
        }
        let handler_checks_state = self.project.is_some()
            && matches!(id, INITIALIZE_BOARD_SETUP | SET_BOARD_SETUP | SET_SILKSCREEN | sheets::CREATE_SHEET | sheets::DELETE_SHEET);
        if !handler_checks_state && !self.available(id) {
            return Err(CommandError::Unavailable(match id {
                sheets::CREATE_SHEET
                | sheets::RENAME_SHEET
                | sheets::DELETE_SHEET
                | "project.rename"
                | PLACE_RESISTOR
                | PLACE_SYMBOL
                | CONNECT_PINS
                | MOVE_SYMBOL
                | ROTATE_SYMBOL
                | DELETE_SYMBOL
                | SET_SYMBOL_FIELDS
                | PLACE_FOOTPRINT
                | ASSIGN_FOOTPRINT
                | PLACE_ASSIGNED_FOOTPRINT
                | SET_FOOTPRINT_TRANSFORM
                | DELETE_FOOTPRINTS
                | SET_SILKSCREEN
                | DRAW_WIRE
                | DELETE_WIRE
                | SLICE_WIRE
                | BREAK_WIRE
                | DELETE_WIRE_SEGMENT
                | MOVE_WIRE_SEGMENT
                | MOVE_WIRE_CORNER
                | MOVE_JUNCTION
                | ATTACH_JUNCTION
                | HEAL_WIRES
                | MOVE_ITEMS
                | DELETE_ITEMS
                | INITIALIZE_BOARD_SETUP
                | SET_BOARD_SETUP => "no project is open".to_owned(),
                "edit.undo" => "nothing to undo".to_owned(),
                "edit.redo" => "nothing to redo".to_owned(),
                _ => format!("`{id}` can't run now"),
            }));
        }
        match id {
            "project.new" => self.change(Project {
                name: if params.get("name").is_some() { name_param(params)? } else { UNTITLED.to_owned() },
                id: fresh_id()?,
                numeric_id_high_water: Some(0),
                sheets: Arc::new(vec![sheets::SchematicSheet::empty(fresh_id()?, "Main".into())]),
                sheet_index: 0,
                components: Arc::default(),
                units: Arc::default(),
                board: Arc::new(Board::for_new_project(params["create_board"].as_bool().unwrap_or(false))?),
                named_graph: Arc::default(),
                upgraded_from: None,
            }),
            "project.rename" => {
                let name = name_param(params)?;
                let mut project = self.open_project()?.clone();
                if project.name != name {
                    project.name = name;
                    self.change(project);
                }
            }
            board::identity::CREATE_BOARD => self.create_board(params)?,
            sheets::CREATE_SHEET | sheets::RENAME_SHEET | sheets::DELETE_SHEET => self.edit_sheet(id, params)?,
            net_labels::ADD | net_labels::SET | net_labels::DELETE => self.edit_net_label(id, params)?,
            hierarchy::CREATE_CHILD
            | hierarchy::PLACE_SHEET
            | hierarchy::RESIZE_SHEET
            | hierarchy::MOVE_SHEET
            | hierarchy::REMOVE_SHEET
            | hierarchy::ADD_LABEL
            | hierarchy::EDIT_LABEL
            | hierarchy::DELETE_LABEL => self.edit_hierarchy(id, params)?,
            PLACE_RESISTOR => {
                let resistor =
                    library_symbol("resistor").ok_or_else(|| CommandError::Unavailable("the starter library has no resistor symbol".to_owned()))?;
                self.place_symbol(resistor, params)?;
            }
            PLACE_SYMBOL => {
                let library = params["library"].as_str().unwrap_or("starter");
                let id = params["definition"].as_str().unwrap_or_default();
                let revision = params.get("definition_revision").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok());
                let definition = if library == "starter" {
                    match revision {
                        Some(revision) => library::starter().symbol_revision(id, revision).cloned(),
                        None => library_symbol(id),
                    }
                } else {
                    if params.get("definition_revision").is_none() {
                        return Err(CommandError::InvalidParams("`definition_revision` is required for external libraries".into()));
                    }
                    if params.get("definition_digest").is_none() {
                        return Err(CommandError::InvalidParams("`definition_digest` is required for external libraries".into()));
                    }
                    revision.and_then(|revision| self.libraries.symbol(library, id, revision))
                }
                .ok_or_else(|| CommandError::InvalidParams("unknown symbol definition".to_owned()))?;
                if (library != "starter" || params.get("definition_digest").is_some())
                    && params["definition_digest"].as_str()
                        != Some(library::sources::symbol_digest(&definition).map_err(CommandError::InvalidParams)?.as_str())
                {
                    return Err(CommandError::InvalidParams("external symbol content changed; search the library again".into()));
                }
                self.place_symbol(definition, params)?;
            }
            CONNECT_PINS => self.connect_pins(params)?,
            DRAW_WIRE => self.draw_wire(params)?,
            DELETE_WIRE => self.delete_wire(params)?,
            SLICE_WIRE => self.slice_wire(params)?,
            MOVE_WIRE_SEGMENT => self.move_wire_segment(params)?,
            MOVE_WIRE_CORNER => self.move_wire_corner(params)?,
            MOVE_JUNCTION => self.move_junction(params)?,
            ATTACH_JUNCTION => self.attach_junction(params)?,
            HEAL_WIRES => self.heal_wires(params)?,
            BREAK_WIRE => self.break_wire(params)?,
            DELETE_WIRE_SEGMENT => self.delete_wire_segment(params)?,
            MOVE_SYMBOL => self.move_symbol(params)?,
            MOVE_ITEMS => self.move_items(params)?,
            DELETE_ITEMS => self.delete_items(params)?,
            ROTATE_SYMBOL => self.rotate_symbol(params)?,
            DELETE_SYMBOL => self.delete_symbol(params)?,
            SET_SYMBOL_FIELDS => self.set_symbol_fields(params)?,
            ASSIGN_FOOTPRINT => self.assign_footprint(params)?,
            PLACE_ASSIGNED_FOOTPRINT => self.place_assigned_footprint(params)?,
            PLACE_FOOTPRINT => self.place_footprint(params)?,
            SET_FOOTPRINT_TRANSFORM => self.set_footprint_transform(params)?,
            DELETE_FOOTPRINTS => self.delete_footprints(params)?,
            INITIALIZE_BOARD_SETUP => self.initialize_board_setup(params)?,
            SET_BOARD_SETUP => self.set_board_setup(params)?,
            SET_SILKSCREEN => self.set_silkscreen(params)?,
            ADD_DRAWING | SET_DRAWING | DELETE_DRAWING => self.edit_board_drawing(id, params)?,
            "edit.undo" => self.step(false),
            "edit.redo" => self.step(true),
            _ => return Err(CommandError::Unknown(id.to_owned())),
        }
        Ok(self.revision)
    }

    /// Borrow the current project or report that the command has no target.
    fn open_project(&self) -> Result<&Project, CommandError> {
        self.project.as_deref().ok_or_else(|| CommandError::Unavailable("no project is open".to_owned()))
    }

    /// Add a symbol from validated parameters if the caller's revision is still current.
    /// Reserve above the allocation watermark and runtime revision so undo cannot recycle it.
    /// Exact wire contacts connect; a two-pin placement on a straight run inserts the part.
    /// Explicit native selections reject conflicting pinned content. Legacy starter-only calls
    /// retain their compatibility behavior of reusing the project's existing saved definition.
    fn place_symbol(&mut self, definition: Arc<SymbolDefinition>, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        if project.components.len() >= MAX_SYMBOLS {
            return Err(CommandError::Unavailable(format!("the schematic already has {MAX_SYMBOLS} symbols")));
        }
        // A project keeps one copy per library, ID and revision: reuse the one it already saves.
        let same = |saved: &&Arc<SymbolDefinition>| {
            (&saved.library, &saved.id, saved.revision()) == (&definition.library, &definition.id, definition.revision())
        };
        let definition = match project.components.values().map(|component| &component.definition).find(same) {
            Some(saved) if (definition.library != "starter" || params.get("definition_digest").is_some()) && **saved != *definition => {
                return Err(CommandError::InvalidParams("symbol identity/revision conflicts with its pinned project copy".into()));
            }
            Some(saved) => saved.clone(),
            None => definition,
        };
        let mut reference_number = project
            .components
            .values()
            .filter(|component| component.definition.reference_prefix == definition.reference_prefix)
            .map(|component| component.reference_number)
            .max()
            .unwrap_or_default()
            + 1;
        // An edited reference may already use the next automatic one, such as R1 renamed to R2.
        let shown: std::collections::BTreeSet<String> = project.components.values().map(|component| component.reference.clone()).collect();
        while shown.contains(&format!("{}{reference_number}", definition.reference_prefix)) {
            reference_number += 1;
        }
        if reference_number > MAX_REFERENCE_NUMBER {
            return Err(CommandError::Unavailable("no more reference numbers are available for this symbol type".to_owned()));
        }
        let ordinal = self.next_object_id()?;
        let component = Component {
            ordinal,
            definition: definition.clone(),
            reference_number,
            reference: format!("{}{reference_number}", definition.reference_prefix),
            value: definition.default_value().to_owned(),
        };
        let mut project = project.clone();
        Arc::make_mut(&mut project.components).insert(ordinal, component);
        Arc::make_mut(&mut project.units).insert((ordinal, definition.unit_id().to_owned()), ordinal);
        Arc::make_mut(&mut project.sheet_mut().symbols).push(SymbolPlacement {
            unit: definition.unit_id().to_owned(),
            definition,
            ordinal,
            component: ordinal,
            position: Point { x_nm: params["x_nm"].as_i64().unwrap_or_default(), y_nm: params["y_nm"].as_i64().unwrap_or_default() },
            rotation: (params["rotation"].as_u64().unwrap_or_default() / 90) as u8,
        });
        project.sheet_mut().connectivity = Arc::default();
        let project = project.healed(self.open_project()?, self.revision(), wire::auto_heal(params))?;
        self.change(project);
        Ok(())
    }

    /// Move a placement as one undo step, retaining connections by default. With `disconnect`,
    /// leave old routes at their original coordinates; destination contacts still connect.
    fn move_symbol(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        let index = params["symbol"]
            .as_str()
            .and_then(schematic::parse_symbol_id)
            .and_then(|id| project.symbol_index(id))
            .ok_or_else(|| CommandError::InvalidParams("symbol does not exist".to_owned()))?;
        let position = Point { x_nm: params["x_nm"].as_i64().unwrap_or_default(), y_nm: params["y_nm"].as_i64().unwrap_or_default() };
        if project.symbols[index].position != position {
            let next = if params["disconnect"].as_bool().unwrap_or(false) {
                project.moved_symbol_disconnected(&project.symbols[index].id(), position, self.revision(), wire::auto_heal(params))?
            } else {
                project.moved_symbol(&project.symbols[index].id(), position, self.revision(), wire::auto_heal(params))?
            };
            self.change(next);
        }
        Ok(())
    }

    /// Find the placed symbol named by the `symbol` parameter.
    fn symbol_param(project: &Project, params: &Value) -> Result<usize, CommandError> {
        params["symbol"]
            .as_str()
            .and_then(schematic::parse_symbol_id)
            .and_then(|id| project.symbol_index(id))
            .ok_or_else(|| CommandError::InvalidParams("symbol does not exist".to_owned()))
    }

    /// Turn a symbol a quarter turn counterclockwise, as KiCad's R does. Its wires follow its pins.
    fn rotate_symbol(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        let index = Self::symbol_param(project, params)?;
        let mut project = project.clone();
        let symbol = &mut Arc::make_mut(&mut project.sheet_mut().symbols)[index];
        symbol.rotation = (symbol.rotation + 1) % 4;
        self.change(project);
        Ok(())
    }

    /// Remove a unit and its wires. Removing the last placed unit also removes its component
    /// and footprint; undo restores the entire snapshot, including component fields.
    fn delete_symbol(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        let index = Self::symbol_param(project, params)?;
        let symbol = project.symbols[index].clone();
        let ordinal = symbol.ordinal;
        let mut project = project.clone();
        Arc::make_mut(&mut project.sheet_mut().symbols).remove(index);
        Arc::make_mut(&mut project.sheet_mut().wires).retain(|wire| wire.terminals().iter().all(|terminal| terminal.symbol != ordinal));
        project.prune_junctions();
        Arc::make_mut(&mut project.units).remove(&(symbol.component, symbol.unit.clone()));
        if !project.symbols.iter().any(|other| other.component == symbol.component) {
            Arc::make_mut(&mut project.components).remove(&symbol.component);
            Arc::make_mut(&mut project.board).remove_component(symbol.component);
        }
        project.sheet_mut().connectivity = Arc::default();
        self.change(project);
        Ok(())
    }

    /// Set component-owned fields through a placed unit. References must be unique across
    /// components; both fields are bounded single-line text. Optional orientation and center axes
    /// change in the same transaction; omitted placement fields retain their values. Center changes
    /// share Move Symbol's exact contacts and optional healing, preserving attached wires. Any
    /// validation/contact failure rejects the entire edit; unchanged edits are no-ops.
    fn set_symbol_fields(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        let index = Self::symbol_param(project, params)?;
        let component =
            project.component(&project.symbols[index]).ok_or_else(|| CommandError::InvalidParams("symbol has no component".to_owned()))?;
        let reference = params["reference"].as_str().unwrap_or_default().trim().to_owned();
        let value = params["value"].as_str().unwrap_or_default().trim().to_owned();
        schematic::check_reference(&reference).map_err(CommandError::InvalidParams)?;
        schematic::check_value(&value).map_err(CommandError::InvalidParams)?;
        if project.components.values().any(|other| other.ordinal != component.ordinal && other.reference == reference) {
            return Err(CommandError::InvalidParams(format!("another component is already {reference}")));
        }
        let rotation = params["rotation"].as_u64().map_or(project.symbols[index].rotation, |degrees| (degrees / 90) as u8);
        let original_position = project.symbols[index].position;
        let position = Point {
            x_nm: params["x_nm"].as_i64().unwrap_or(original_position.x_nm),
            y_nm: params["y_nm"].as_i64().unwrap_or(original_position.y_nm),
        };
        if component.reference == reference
            && component.value == value
            && project.symbols[index].rotation == rotation
            && position == original_position
        {
            return Ok(());
        }
        let mut component = component.clone();
        component.reference = reference;
        component.value = value;
        let mut next = project.clone();
        Arc::make_mut(&mut next.components).insert(component.ordinal, component);
        let symbol = &mut Arc::make_mut(&mut next.sheet_mut().symbols)[index];
        symbol.rotation = rotation;
        symbol.position = position;
        if position != original_position {
            next = next.healed(project, self.revision(), wire::auto_heal(params))?;
        }
        self.change(next);
        Ok(())
    }

    /// Place one footprint per component, pinning its exact physical definition and validating
    /// the terminal-to-pad bijection before changing state. Legacy two-pad callers retain their
    /// original encoding. Supplying both mapping forms, an incomplete mapping, or a placement
    /// that disagrees with an existing independent assignment is refused.
    fn place_footprint(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        let index = Self::symbol_param(project, params)?;
        let component = project.symbols[index].component;
        let symbol = Arc::clone(&project.symbols[index].definition);
        let position = Point { x_nm: params["x_nm"].as_i64().unwrap_or_default(), y_nm: params["y_nm"].as_i64().unwrap_or_default() };
        let id = params["definition"].as_str().unwrap_or_default();
        if params.get("mapping").is_some() && params.get("pin_1_pad").is_some() {
            return Err(CommandError::InvalidParams("supply mapping or pin_1_pad, not both".into()));
        }
        let mapping = if let Some(value) = params.get("mapping") {
            board::parse_mapping(value).ok_or_else(|| CommandError::InvalidParams("invalid terminal-to-pad mapping".into()))?
        } else {
            let first = params["pin_1_pad"].as_u64().ok_or_else(|| CommandError::InvalidParams("supply mapping or pin_1_pad".into()))? as u8;
            BTreeMap::from([("1".into(), first.to_string()), ("2".into(), (3 - first).to_string())])
        };
        let mut project = project.clone();
        let mut footprint = Footprint::new(self.next_object_id()?, component, 0, position);
        if id == FOOTPRINT_DEFINITION {
            if !symbol.fits_two_pad_footprint() {
                return Err(CommandError::InvalidParams("the two-pad footprint only fits symbols with exactly pins 1 and 2".into()));
            }
            let first = mapping.get("1").and_then(|pad| pad.parse::<u8>().ok()).filter(|pad| (1..=2).contains(pad));
            let Some(first) = first.filter(|pad| mapping.len() == 2 && mapping.get("2") == Some(&(3 - pad).to_string())) else {
                return Err(CommandError::InvalidParams("legacy footprint requires an exact two-pad bijection".into()));
            };
            footprint.pin_1_pad = first;
        } else {
            let definition = board::definition::starter(id).ok_or_else(|| CommandError::InvalidParams("unavailable footprint definition".into()))?;
            let key = definition.key();
            if project.board.definitions.get(&key).is_some_and(|pinned| pinned.as_ref() != definition.as_ref()) {
                return Err(CommandError::InvalidParams("footprint identity/revision conflicts with its pinned copy".into()));
            }
            footprint.library = Some(definition.library.clone());
            footprint.definition.clone_from(&definition.id);
            footprint.definition_revision = definition.revision;
            footprint.mapping = Some(mapping);
            Arc::make_mut(&mut project.board).definitions.insert(key, definition);
        }
        if !project.board.mapping_valid(&footprint, &symbol) {
            return Err(CommandError::InvalidParams("footprint pads must map exactly once to every symbol terminal".into()));
        }
        if project
            .board
            .assignments
            .get(&component)
            .is_some_and(|a| footprint.key() != Some(a.key()) || footprint.mapping.as_ref() != Some(&a.mapping))
        {
            return Err(CommandError::InvalidParams("placement conflicts with the pinned assignment; use pcb.place_assigned_footprint".into()));
        }
        Arc::make_mut(&mut project.board).append(footprint).map_err(|error| CommandError::InvalidParams(error.into()))?;
        self.change(project);
        Ok(())
    }

    /// Add one wire and undo step after validating both terminals against the current project.
    fn connect_pins(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        let endpoint = |name: &str| {
            Terminal::parse(&params[name])
                .filter(|terminal| project.terminal_position(*terminal).is_some())
                .ok_or_else(|| CommandError::InvalidParams(format!("`{name}` must reference an existing symbol terminal")))
        };
        let mut terminals = [endpoint("from")?, endpoint("to")?];
        terminals.sort_unstable();
        if terminals[0] == terminals[1] {
            return Err(CommandError::InvalidParams("a wire needs two distinct terminals".to_owned()));
        }
        if project.wires.iter().any(|wire| {
            wire.endpoints == terminals.map(WireEndpoint::Terminal)
                || wire.endpoints == [WireEndpoint::Terminal(terminals[1]), WireEndpoint::Terminal(terminals[0])]
        }) {
            return Err(CommandError::InvalidParams("these terminals are already joined by a wire".to_owned()));
        }
        if project.wires.len() >= MAX_WIRES {
            return Err(CommandError::Unavailable(format!("the schematic already has {MAX_WIRES} wires")));
        }
        let mut project = project.clone();
        Arc::make_mut(&mut project.sheet_mut().wires).push(Wire {
            ordinal: self.next_object_id()?,
            endpoints: terminals.map(WireEndpoint::Terminal),
            bends: None,
        });
        project.sheet_mut().connectivity = Arc::default();
        self.change(project);
        Ok(())
    }

    /// Reserve above both the monotonic allocation watermark and runtime revision. The candidate
    /// transaction records the reservation only on success; stale/invalid requests allocate nothing.
    fn next_object_id(&self) -> Result<u64, CommandError> {
        self.revision
            .max(self.project.as_ref().map_or(0, |project| project.allocation_high_water()))
            .checked_add(1)
            .filter(|id| *id <= MAX_OBJECT_ID)
            .ok_or_else(|| CommandError::Unavailable("project has exhausted its object IDs".to_owned()))
    }

    /// Reject an edit prepared against a project state that has since changed.
    fn check_revision(&self, params: &Value) -> Result<(), CommandError> {
        if params["expected_revision"].as_u64() != Some(self.revision) {
            return Err(CommandError::Unavailable(format!("project changed, inspect the current revision ({}) and retry", self.revision)));
        }
        Ok(())
    }

    /// Commit a snapshot and bound undo history. Preserve adopted allocation metadata across
    /// content changes, adopting it for legacy allocations/retirements. New edits clear redo;
    /// revisions advance past every live ID allocated by a compound wire edit.
    fn change(&mut self, mut project: Project) {
        // Materialize the legacy board address on the first physical edit, preserving
        // untouched files and keeping the prior representation available to Undo.
        if self
            .project
            .as_ref()
            .is_some_and(|previous| previous.id == project.id && !Arc::ptr_eq(&previous.board, &project.board) && previous.board != project.board)
            && matches!(project.board.ownership, board::identity::Ownership::Legacy)
            && let Some(target) = project.board_target()
        {
            let registration = board::identity::Registration { id: target.board, name: "Board".into(), file: "board.json".into() };
            // Old files allowed the typed board UUID to equal a sheet UUID. Keep
            // their compatibility encoding rather than rename IDs or save an invalid registry.
            if !registration.id_collides(&project) {
                Arc::make_mut(&mut project.board).ownership = board::identity::Ownership::Explicit(Some(registration));
                project.numeric_id_high_water = Some(project.allocation_high_water());
            }
        }
        // Named anchors can attach to moving geometry and join other sheets.
        if project.has_named_nets() || self.project.as_ref().is_some_and(|p| p.has_named_nets()) {
            project.named_graph = Arc::default();
            for sheet in Arc::make_mut(&mut project.sheets) {
                sheet.connectivity = Arc::default();
            }
        }
        let highest = project.highest_id();
        let previous_highest = self.project.as_ref().filter(|old| old.same_document(&project)).map_or(0, |old| old.highest_id());
        if project.numeric_id_high_water.is_some() || highest != previous_highest {
            project.numeric_id_high_water = Some(project.numeric_id_high_water.unwrap_or(0).max(highest).max(previous_highest));
        }
        let project = self.retain_allocation_watermark(Arc::new(project));
        self.undo.push(self.project.replace(project));
        if self.undo.len() > MAX_HISTORY {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.revision = (self.revision + 1).max(highest);
    }

    /// Restore authored content and advance the revision, retaining the allocation watermark
    /// from the state being left when both snapshots belong to the same project.
    fn step(&mut self, forward: bool) {
        let (from, to) = if forward { (&mut self.redo, &mut self.undo) } else { (&mut self.undo, &mut self.redo) };
        if let Some(state) = from.pop() {
            to.push(std::mem::replace(&mut self.project, state));
            self.revision += 1;
        }
        if let Some(project) = self.project.clone() {
            // The state just left is now at the end of the opposite history stack.
            let prior = if forward { self.undo.last() } else { self.redo.last() }.and_then(Option::as_ref);
            if let Some(prior) = prior.filter(|prior| prior.same_document(&project)) {
                self.project = Some(Project::with_allocation_watermark(project, prior.numeric_id_high_water));
            }
        }
    }

    /// Preserve allocation metadata when adopting an earlier snapshot of the current project.
    /// Opening a different project starts that project's independent saved allocation lineage.
    fn retain_allocation_watermark(&self, project: Arc<Project>) -> Arc<Project> {
        let previous = self.project.as_ref().filter(|old| old.same_document(&project)).and_then(|old| old.numeric_id_high_water);
        Project::with_allocation_watermark(project, previous)
    }
}

/// Generate a persistent document or board-record ID and report unavailable system entropy.
fn fresh_id() -> Result<uuid::Uuid, CommandError> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|error| CommandError::Unavailable(format!("could not create a project identity: {error}")))?;
    Ok(uuid::Builder::from_random_bytes(bytes).into_uuid())
}

/// Compare restored authored content while allowing monotonic allocation metadata to advance.
/// Failure/no-op tests should continue comparing exact pointers instead.
#[cfg(test)]
fn assert_same_content(expected: &Arc<Project>, actual: &Arc<Project>) {
    assert!(actual.numeric_id_high_water.unwrap_or(0) >= expected.numeric_id_high_water.unwrap_or(0));
    let expected = Project::with_allocation_watermark(expected.clone(), actual.numeric_id_high_water);
    assert_eq!(persistence::encode_documents(expected).unwrap(), persistence::encode_documents(actual.clone()).unwrap());
}

/// Validate declared inputs, requiring mandatory parameters and rejecting unknown or invalid values.
fn check_params(command: &Command, params: &Value) -> Result<(), CommandError> {
    let empty = Map::new();
    let given = match params {
        Value::Null => &empty,
        Value::Object(o) => o,
        _ => return Err(CommandError::InvalidParams(format!("`{}` takes an object of parameters", command.id))),
    };
    if let Some(extra) =
        given.keys().find(|k| !command.params.iter().any(|p| p.name == k.as_str()) && !(k.as_str() == "sheet" && sheets::scoped_command(command.id)))
    {
        return Err(CommandError::InvalidParams(format!("`{}` has no parameter `{extra}`", command.id)));
    }
    if let Some(sheet) = given.get("sheet") {
        sheets::SHEET.kind.validate(sheet).map_err(|message| CommandError::InvalidParams(message.to_owned()))?;
    }
    for param in command.params {
        if !param.required && !given.contains_key(param.name) {
            continue;
        }
        let value = given.get(param.name).ok_or_else(|| CommandError::InvalidParams(format!("`{}` needs `{}`", command.id, param.name)))?;
        param.kind.validate(value).map_err(|message| CommandError::InvalidParams(format!("`{}` {message}", param.name)))?;
    }
    Ok(())
}

fn name_param(params: &Value) -> Result<String, CommandError> {
    let name = params["name"].as_str().unwrap_or_default().trim();
    if name.is_empty() {
        return Err(CommandError::InvalidParams("the name can't be empty".to_owned()));
    }
    if name.chars().count() > MAX_NAME {
        return Err(CommandError::InvalidParams(format!("the name can't be longer than {MAX_NAME} characters")));
    }
    if name.chars().any(char::is_control) {
        return Err(CommandError::InvalidParams("the name can't contain control characters".to_owned()));
    }
    Ok(name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(session: &Session) -> Option<&str> {
        session.project().map(|p| p.name.as_str())
    }

    #[test]
    fn undo_and_redo_walk_the_history_and_the_revision_never_repeats() {
        let mut s = Session::default();
        s.execute("project.new", &json!({"name": "a"})).unwrap();
        s.execute("project.rename", &json!({"name": "b"})).unwrap();
        s.execute("edit.undo", &Value::Null).unwrap();
        assert_eq!(name(&s), Some("a"));
        s.execute("edit.undo", &Value::Null).unwrap();
        assert_eq!(name(&s), None);
        s.execute("edit.redo", &Value::Null).unwrap();
        assert_eq!(name(&s), Some("a"));
        assert_eq!(s.revision(), 5);
    }

    /// An inspector can change fields and orientation in one history step. Invalid input or a
    /// stale revision cannot partially change either, and older callers retain orientation.
    #[test]
    fn property_fields_and_orientation_are_one_atomic_edit() {
        let mut s = Session::default();
        s.apply("project.new", &json!({})).unwrap();
        s.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":1})).unwrap();
        let before = s.snapshot().unwrap();
        let mut params = json!({"symbol":"symbol-2","reference":"R7","value":"4.7k","rotation":90,"expected_revision":s.revision()});
        let revision = s.revision();
        s.apply(SET_SYMBOL_FIELDS, &params).unwrap();
        assert_eq!(s.revision(), revision + 1);
        let changed = s.snapshot().unwrap();
        assert_eq!(changed.symbols()[0].turns(), 1);
        assert_eq!(changed.reference(&changed.symbols()[0]), "R7");
        params["expected_revision"] = json!(s.revision());
        s.apply(SET_SYMBOL_FIELDS, &params).unwrap();
        assert_eq!(s.revision(), revision + 1);
        params["rotation"] = json!(45);
        params["value"] = json!("invalid turn");
        assert!(s.apply(SET_SYMBOL_FIELDS, &params).is_err());
        assert!(Arc::ptr_eq(&changed, &s.snapshot().unwrap()));
        params["rotation"] = json!(180);
        params["expected_revision"] = json!(revision);
        assert!(s.apply(SET_SYMBOL_FIELDS, &params).is_err());
        assert!(Arc::ptr_eq(&changed, &s.snapshot().unwrap()));
        s.apply("edit.undo", &Value::Null).unwrap();
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    }

    /// Property position edits use the same wire contacts/healing as movement while committing
    /// fields and rotation once. Invalid coordinates and stale revisions preserve all history.
    #[test]
    fn property_position_edits_are_atomic_and_preserve_wires() {
        let mut s = Session::default();
        s.apply("project.new", &json!({})).unwrap();
        for x_nm in [0, 25_400_000] {
            s.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":x_nm,"y_nm":0,"expected_revision":s.revision()})).unwrap();
        }
        s.apply(
            CONNECT_PINS,
            &json!({"from":{"symbol":"symbol-2","terminal":"2"},"to":{"symbol":"symbol-3","terminal":"1"},"expected_revision":s.revision()}),
        )
        .unwrap();
        let before = s.snapshot().unwrap();
        let revision = s.revision();
        let mut params = json!({"symbol":"symbol-2","reference":"R7","value":"4.7k","rotation":90,
            "x_nm":2_540_001,"y_nm":-1_270_001,"expected_revision":revision});
        s.apply(SET_SYMBOL_FIELDS, &params).unwrap();
        assert_eq!(s.revision(), revision + 1);
        let changed = s.snapshot().unwrap();
        assert_eq!(changed.symbols()[0].position, Point { x_nm: 2_540_001, y_nm: -1_270_001 });
        assert_eq!(changed.symbols()[0].turns(), 1);
        assert_eq!(changed.reference(&changed.symbols()[0]), "R7");
        assert!(before.same_connectivity(&changed));
        let documents = persistence::encode_documents(changed.clone()).unwrap();
        assert_eq!(persistence::encode_documents(persistence::decode_documents(&documents).unwrap()).unwrap(), documents);
        let inspected = s.inspect();
        for (key, value) in [("x_nm", json!(MAX_CENTER_NM + 1)), ("y_nm", json!("1 mm")), ("x_nm", Value::Null)] {
            let mut invalid = params.clone();
            invalid["expected_revision"] = json!(s.revision());
            invalid["value"] = json!("must not apply");
            invalid[key] = value;
            assert!(s.apply(SET_SYMBOL_FIELDS, &invalid).is_err());
            assert_eq!(s.inspect(), inspected);
        }
        assert!(s.apply(SET_SYMBOL_FIELDS, &params).is_err(), "stale revision");
        params["expected_revision"] = json!(s.revision());
        params.as_object_mut().unwrap().remove("y_nm");
        s.apply(SET_SYMBOL_FIELDS, &params).unwrap();
        assert_eq!(s.inspect(), inspected, "omitted axes retain exact positions and an unchanged edit is a no-op");
        s.apply("edit.undo", &Value::Null).unwrap();
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
        s.apply("edit.redo", &Value::Null).unwrap();
        assert!(Arc::ptr_eq(&changed, &s.snapshot().unwrap()));
    }

    /// A coordinate edit that would drop a pin on an ambiguous wire crossing rejects its field
    /// changes too, whether automatic wire cleanup is enabled or disabled.
    #[test]
    fn ambiguous_property_moves_reject_the_whole_draft() {
        let mut s = Session::default();
        s.apply("project.new", &json!({})).unwrap();
        let g = schematic::GRID_NM;
        for (from, to) in [
            (Point { x_nm: 0, y_nm: 0 }, Point { x_nm: 20 * g, y_nm: 0 }),
            (Point { x_nm: 10 * g, y_nm: -10 * g }, Point { x_nm: 10 * g, y_nm: 10 * g }),
        ] {
            s.apply(DRAW_WIRE, &json!({"from":from,"to":to,"bends":[],"expected_revision":s.revision()})).unwrap();
        }
        s.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":40*g,"y_nm":40*g,"expected_revision":s.revision()})).unwrap();
        let before = s.inspect();
        let symbol = s.project().unwrap().symbols()[0].id();
        for auto_heal in [true, false] {
            assert!(
                s.apply(
                    SET_SYMBOL_FIELDS,
                    &json!({"symbol":symbol,"reference":"R7","value":"not applied",
                "x_nm":14*g,"y_nm":0,"auto_heal":auto_heal,"expected_revision":s.revision()})
                )
                .is_err()
            );
            assert_eq!(s.inspect(), before);
        }
    }

    /// Rotating turns the pins and the wire follows; deleting removes the symbol with its wires
    /// and footprint as one undo step; field edits are checked and clear back to the defaults.
    #[test]
    fn rotate_delete_and_edit_fields_go_through_the_command_set() {
        let mut s = Session::default();
        s.apply("project.new", &json!({"name": "Edits"})).unwrap();
        for x_nm in [0, 25_400_000] {
            s.apply(PLACE_SYMBOL, &json!({"definition": "resistor", "x_nm": x_nm, "y_nm": 0, "expected_revision": s.revision()})).unwrap();
        }
        let link = json!({"from": {"symbol": "symbol-2", "terminal": "2"}, "to": {"symbol": "symbol-3", "terminal": "1"}, "expected_revision": s.revision()});
        s.apply(CONNECT_PINS, &link).unwrap();
        s.apply(ROTATE_SYMBOL, &json!({"symbol": "symbol-2", "expected_revision": s.revision()})).unwrap();
        let symbol = &s.inspect()["project"]["schematic"]["symbols"][0];
        assert_eq!(symbol["rotation"], 90);
        // A quarter turn counterclockwise takes pin 2 from the right to the top.
        assert_eq!((symbol["pins"][1]["x_nm"].clone(), symbol["pins"][1]["y_nm"].clone()), (json!(0), json!(-(4 * schematic::GRID_NM))));
        let terminal = s.project().unwrap().wires()[0].terminals()[0];
        assert_eq!(s.project().unwrap().terminal_position(terminal), Some(Point { x_nm: 0, y_nm: -(4 * schematic::GRID_NM) }));

        let fields = |s: &mut Session, reference: &str, value: &str| {
            let params = json!({"symbol": "symbol-3", "reference": reference, "value": value, "expected_revision": s.revision()});
            s.apply(SET_SYMBOL_FIELDS, &params)
        };
        assert!(fields(&mut s, "R1", "1k").is_err(), "R1 is taken");
        assert!(fields(&mut s, "R 9", "1k").is_err(), "no spaces in a reference");
        assert!(fields(&mut s, "R9", " ").is_err(), "a value can't be blank");
        fields(&mut s, " R9 ", "4.7k").unwrap();
        let revision = s.revision();
        fields(&mut s, "R9", "4.7k").unwrap();
        assert_eq!(s.revision(), revision, "an unchanged edit is no step");
        s.apply(PLACE_SYMBOL, &json!({"definition": "resistor", "x_nm": 50_800_000, "y_nm": 0, "expected_revision": s.revision()})).unwrap();
        fields(&mut s, "R2", "1k").unwrap();
        let references: Vec<_> =
            s.inspect()["project"]["schematic"]["symbols"].as_array().unwrap().iter().map(|symbol| symbol["reference"].clone()).collect();
        assert_eq!(references, [json!("R1"), json!("R2"), json!("R3")], "a reference back at its default clears the edit");

        let before = s.inspect()["project"].clone();
        s.apply(DELETE_SYMBOL, &json!({"symbol": "symbol-2", "expected_revision": s.revision()})).unwrap();
        let project = s.project().unwrap();
        assert_eq!((project.symbols().len(), project.wires().len()), (2, 0));
        s.apply("edit.undo", &Value::Null).unwrap();
        assert_eq!(s.inspect()["project"], before);
    }

    #[test]
    fn a_new_change_clears_redo() {
        let mut s = Session::default();
        s.execute("project.new", &json!({"name": "a"})).unwrap();
        s.execute("edit.undo", &Value::Null).unwrap();
        s.execute("project.new", &json!({"name": "b"})).unwrap();
        assert!(!s.available("edit.redo"));
    }

    #[test]
    fn a_refused_command_changes_nothing() {
        let mut s = Session::default();
        let before = s.inspect();
        for (id, params) in [
            ("no.such", Value::Null),
            ("project.rename", json!({"name": "x"})),
            ("edit.undo", Value::Null),
            ("project.new", json!({"name": "  "})),
            ("project.new", json!({"name": 3})),
            ("project.new", json!({"name": "x", "extra": "y"})),
            ("project.new", json!({"name": "a\nb"})),
            ("project.new", json!({"name": "n".repeat(MAX_NAME + 1)})),
            ("project.new", json!([])),
        ] {
            assert!(s.execute(id, &params).is_err(), "{id} {params}");
            assert_eq!(s.inspect(), before, "{id} {params}");
        }
    }

    /// New Project needs no name: without one the project is Untitled, with its own identity.
    #[test]
    fn a_new_project_without_a_name_is_untitled() {
        let mut s = Session::default();
        s.apply("project.new", &Value::Null).unwrap();
        let first = s.snapshot().unwrap();
        s.apply("project.new", &json!({})).unwrap();
        let second = s.snapshot().unwrap();
        assert_eq!((first.name.as_str(), second.name.as_str()), (UNTITLED, UNTITLED));
        assert!(!first.same_document(&second), "two untitled projects are different documents");
        let schema = command("project.new").unwrap().to_json();
        assert_eq!(schema["params"]["required"], json!([]));
    }

    /// A save that names an untitled project renames its whole history without an undo step or a
    /// new revision. The saved snapshot is swapped in by pointer, and an edit made while the save
    /// ran keeps its changes under the new name. A name the person chose meanwhile is kept.
    #[test]
    fn a_saved_name_renames_the_untitled_history_in_place() {
        let mut s = Session::default();
        s.apply("project.new", &Value::Null).unwrap();
        s.apply(PLACE_SYMBOL, &json!({"definition": "resistor", "x_nm": 0, "y_nm": 0, "expected_revision": s.revision()})).unwrap();
        let original = s.snapshot().unwrap();
        let saved = Arc::new(original.renamed(" blinky ").unwrap());
        assert_eq!(saved.name, "blinky");
        assert!(original.renamed("").is_err() && original.renamed("a\nb").is_err());
        // An edit lands while the save is running.
        s.apply(PLACE_SYMBOL, &json!({"definition": "resistor", "x_nm": 25_400_000, "y_nm": 0, "expected_revision": s.revision()})).unwrap();
        let revision = s.revision();

        assert!(s.adopt_saved_name(&original, &saved));
        assert_eq!(s.revision(), revision);
        assert_eq!((name(&s), s.project().unwrap().symbols().len()), (Some("blinky"), 2));
        assert!(!Arc::ptr_eq(&s.snapshot().unwrap(), &saved), "the later edit is still unsaved");
        s.apply("edit.undo", &Value::Null).unwrap();
        crate::assert_same_content(&saved, &s.snapshot().unwrap());
        s.apply("edit.undo", &Value::Null).unwrap();
        assert_eq!(name(&s), Some("blinky"), "undo doesn't bring Untitled back");
        s.apply("edit.redo", &Value::Null).unwrap();
        crate::assert_same_content(&saved, &s.snapshot().unwrap());
        assert!(!s.adopt_saved_name(&original, &saved), "nothing is left to rename");

        // Renamed by the person during the save, the project keeps their name.
        let mut s = Session::default();
        s.apply("project.new", &Value::Null).unwrap();
        let original = s.snapshot().unwrap();
        s.apply("project.rename", &json!({"name": "Mine"})).unwrap();
        s.adopt_saved_name(&original, &Arc::new(original.renamed("file").unwrap()));
        assert_eq!(name(&s), Some("Mine"));
        s.apply("edit.undo", &Value::Null).unwrap();
        assert_eq!(name(&s), Some("file"));

        // A save of another document renames nothing here.
        let mut other = Session::default();
        other.apply("project.new", &Value::Null).unwrap();
        let foreign = other.snapshot().unwrap();
        assert!(!s.adopt_saved_name(&foreign, &Arc::new(foreign.renamed("x").unwrap())));
        assert!(!s.adopt_saved_name(&original, &Arc::new(foreign.renamed("x").unwrap())), "the copy must be of the same document");

        // A new project started while the first save ran stays Untitled; the saved one, now
        // behind it in undo, takes the name.
        let mut s = Session::default();
        s.apply("project.new", &Value::Null).unwrap();
        let original = s.snapshot().unwrap();
        s.apply("project.new", &Value::Null).unwrap();
        let saved = Arc::new(original.renamed("first").unwrap());
        assert!(s.adopt_saved_name(&original, &saved));
        assert_eq!(name(&s), Some(UNTITLED));
        s.apply("edit.undo", &Value::Null).unwrap();
        assert!(Arc::ptr_eq(&s.snapshot().unwrap(), &saved));
    }

    #[test]
    fn renaming_to_the_same_name_is_not_a_change() {
        let mut s = Session::default();
        s.execute("project.new", &json!({"name": "a"})).unwrap();
        s.execute("project.rename", &json!({"name": " a "})).unwrap();
        assert_eq!(s.revision(), 1);
    }

    #[test]
    fn placing_a_resistor_preserves_position_and_identity_through_history() {
        let mut s = Session::default();
        s.execute("project.new", &json!({"name": "blinky"})).unwrap();
        let place = |revision| json!({"x_nm": 500, "y_nm": -200, "expected_revision": revision});
        let state = s.execute(PLACE_RESISTOR, &place(1)).unwrap();
        let symbol = &state["project"]["schematic"]["symbols"][0];
        assert_ne!(symbol["id"], symbol["component_id"]);
        assert_eq!(symbol["reference"], "R1");
        assert_eq!(symbol["pins"][0], json!({"number": "1", "electrical_type": "passive", "x_nm": 500 - (4 * schematic::GRID_NM), "y_nm": -200}));
        s.execute("project.rename", &json!({"name": "renamed"})).unwrap();
        assert_eq!(&s.inspect()["project"]["schematic"]["symbols"][0], symbol);
        s.execute("edit.undo", &Value::Null).unwrap();
        s.execute("edit.undo", &Value::Null).unwrap();
        assert!(s.project().unwrap().symbols().is_empty());
        s.execute("edit.redo", &Value::Null).unwrap();
        assert_eq!(&s.inspect()["project"]["schematic"]["symbols"][0], symbol);
        s.execute("edit.undo", &Value::Null).unwrap();
        let state = s.execute(PLACE_RESISTOR, &place(s.revision())).unwrap();
        assert_ne!(state["project"]["schematic"]["symbols"][0]["id"], symbol["id"]);
        assert!(!s.available("edit.redo"));
    }

    /// Unit identities come from the parsed library definition, even when the unit is not
    /// named `main`. The placement command and saved unit record preserve that identity.
    #[test]
    fn placement_preserves_the_library_unit_identity() {
        let mut file: Value = serde_json::from_str(include_str!("../../../libraries/starter/symbols/resistor.json")).unwrap();
        file["units"][0]["id"] = json!("body");
        let definition = Arc::new(library::parse_symbol(&file.to_string()).unwrap());
        let mut session = Session::default();
        session.apply("project.new", &json!({"name": "Unit identity"})).unwrap();
        session.place_symbol(definition, &json!({"x_nm": 0, "y_nm": 0, "expected_revision": session.revision()})).unwrap();
        let snapshot = session.snapshot().unwrap();
        assert_eq!(snapshot.symbols()[0].unit, "body");
        let stored: Value = serde_json::from_slice(&persistence::encode(snapshot).unwrap()).unwrap();
        assert_eq!(stored["sheet"]["symbols"][0]["unit"], "body");
        assert_eq!(session.inspect()["project"]["schematic"]["symbols"][0]["unit"], "body");
    }

    /// Number each kind independently and preserve LED terminal identities across undo and redo.
    #[test]
    fn placing_leds_preserves_resistor_references_and_exposes_polarity() {
        let mut s = Session::default();
        s.execute("project.new", &json!({"name": "blinky"})).unwrap();
        for definition in ["resistor", "led", "resistor", "led"] {
            s.execute(PLACE_SYMBOL, &json!({"definition": definition, "x_nm": 0, "y_nm": 0, "expected_revision": s.revision()})).unwrap();
        }
        let inspected = s.inspect();
        let schematic = &inspected["project"]["schematic"];
        let symbols = schematic["symbols"].as_array().unwrap();
        let references: Vec<_> = symbols.iter().map(|symbol| symbol["reference"].as_str().unwrap()).collect();
        assert_eq!(references, ["R1", "D1", "R2", "D2"]);
        assert_eq!(symbols[1]["kind"], "led");
        assert_eq!(symbols[1]["value"], "LED");
        assert_eq!(
            symbols[1]["pins"],
            json!([
                {"number": "1", "name": "Anode", "electrical_type": "passive", "x_nm": -(3 * schematic::GRID_NM), "y_nm": 0},
                {"number": "2", "name": "Cathode", "electrical_type": "passive", "x_nm": (3 * schematic::GRID_NM), "y_nm": 0},
            ])
        );
        assert_eq!(schematic["netlist"]["unconnected"].as_array().unwrap().len(), 8);
        let undone = s.execute("edit.undo", &Value::Null).unwrap();
        assert_eq!(undone["project"]["schematic"]["symbols"].as_array().unwrap().len(), 3);
        let redone = s.execute("edit.redo", &Value::Null).unwrap();
        assert_eq!(&redone["project"]["schematic"], schematic);
    }

    /// Exercise rejected input against the full inspection snapshot, including the revision and undo state.
    #[test]
    fn invalid_placement_leaves_the_project_and_history_unchanged() {
        let mut s = Session::default();
        assert!(!s.available(PLACE_RESISTOR));
        let valid = json!({"x_nm": 0, "y_nm": 0, "expected_revision": 1});
        assert!(s.execute(PLACE_RESISTOR, &valid).is_err());
        s.execute("project.new", &json!({"name": "blinky"})).unwrap();
        let before = s.inspect();
        for definition in [json!("unknown"), json!(1)] {
            let params = json!({"definition": definition, "x_nm": 0, "y_nm": 0, "expected_revision": 1});
            assert!(matches!(s.execute(PLACE_SYMBOL, &params), Err(CommandError::InvalidParams(_))));
            assert_eq!(s.inspect(), before);
        }
        assert!(matches!(s.execute(PLACE_SYMBOL, &valid), Err(CommandError::InvalidParams(_))));
        for params in [
            json!({"x_nm": 0, "y_nm": 0}),
            json!({"x_nm": "0", "y_nm": 0, "expected_revision": 1}),
            json!({"x_nm": 0.25, "y_nm": 0, "expected_revision": 1}),
            json!({"x_nm": MAX_CENTER_NM + 1, "y_nm": 0, "expected_revision": 1}),
            json!({"x_nm": 0, "y_nm": -MAX_CENTER_NM - 1, "expected_revision": 1}),
            json!({"x_nm": 0, "y_nm": 0, "expected_revision": 0}),
            json!({"x_nm": 0, "y_nm": 0, "expected_revision": -1}),
            json!({"x_nm": 0, "y_nm": 0, "expected_revision": 1, "extra": "pin"}),
        ] {
            assert!(s.execute(PLACE_RESISTOR, &params).is_err(), "{params}");
            assert_eq!(s.inspect(), before);
        }
        s.execute(PLACE_RESISTOR, &valid).unwrap();
    }

    #[test]
    fn history_is_bounded() {
        let mut s = Session::default();
        for i in 0..MAX_HISTORY + 5 {
            s.execute("project.new", &json!({"name": i.to_string()})).unwrap();
        }
        assert_eq!(s.undo.len(), MAX_HISTORY);
    }
}

#[cfg(test)]
mod wiring_tests {
    use super::*;

    /// Build three placements, including two with coincident pins, without connecting any of them.
    fn three_resistors() -> Session {
        let mut session = Session::default();
        session.execute("project.new", &json!({"name": "wiring"})).unwrap();
        for x_nm in [0, 0, 20 * schematic::GRID_NM] {
            session.execute(PLACE_RESISTOR, &json!({"x_nm": x_nm, "y_nm": 0, "expected_revision": session.revision()})).unwrap();
        }
        session
    }

    fn terminal(symbol: &str, number: &str) -> Value {
        json!({"symbol": symbol, "terminal": number})
    }

    #[test]
    fn explicit_wires_form_a_net_and_undo_restores_the_previous_membership() {
        let mut session = three_resistors();
        let a = terminal("symbol-2", "1");
        let b = terminal("symbol-3", "2");
        let c = terminal("symbol-4", "1");
        let initial = session.inspect();
        assert_eq!(initial["project"]["schematic"]["netlist"]["nets"], json!([]));
        assert_eq!(initial["project"]["schematic"]["netlist"]["unconnected"].as_array().unwrap().len(), 6);

        let first = session.execute(CONNECT_PINS, &json!({"from": b, "to": a, "expected_revision": 4})).unwrap();
        let second = session.execute(CONNECT_PINS, &json!({"from": c, "to": b, "expected_revision": 5})).unwrap();
        let schematic = &second["project"]["schematic"];
        let path = json!([{"x_nm": -5_080_000, "y_nm": 0}, {"x_nm": 5_080_000, "y_nm": 0}]);
        assert_eq!(schematic["wires"][0], json!({"id": "wire-5", "from": a, "to": b, "path": path}), "inspection reports the drawn route");
        assert_eq!(schematic["netlist"]["nets"], json!([{"terminals": [a, b, c]}]));
        assert_eq!(schematic["netlist"]["unconnected"].as_array().unwrap().len(), 3);

        let undone = session.execute("edit.undo", &Value::Null).unwrap();
        assert_eq!(undone["project"]["schematic"], first["project"]["schematic"]);
        let redone = session.execute("edit.redo", &Value::Null).unwrap();
        assert_eq!(&redone["project"]["schematic"], schematic);
    }

    /// Invalid endpoints, reverse duplicates and stale requests must leave the full snapshot intact.
    #[test]
    fn rejected_connections_do_not_change_the_project() {
        let mut session = three_resistors();
        let a = terminal("symbol-2", "1");
        let b = terminal("symbol-3", "2");
        session.execute(CONNECT_PINS, &json!({"from": a, "to": b, "expected_revision": 4})).unwrap();
        let before = session.inspect();
        for params in [
            json!({"from": b, "to": a, "expected_revision": 5}),
            json!({"from": a, "to": a, "expected_revision": 5}),
            json!({"from": a, "to": terminal("symbol-999", "1"), "expected_revision": 5}),
            json!({"from": a, "to": terminal("symbol-4", "3"), "expected_revision": 5}),
            json!({"from": a, "to": terminal("symbol-04", "1"), "expected_revision": 5}),
            json!({"from": a, "to": {"symbol": "symbol-4", "terminal": "1", "extra": true}, "expected_revision": 5}),
            json!({"from": a, "to": terminal("symbol-4", "1"), "expected_revision": 4}),
        ] {
            assert!(session.execute(CONNECT_PINS, &params).is_err(), "{params}");
            assert_eq!(session.inspect(), before);
        }
    }
}
