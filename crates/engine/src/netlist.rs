//! Netlist export: every component with its logical terminals, and every net with the terminals
//! it joins, written as one JSON document from the project's complete electrical connectivity.
//! Terminals of placed units that are on no net are listed separately as unconnected.
//!
//! The document follows `docs/schemas/netlist-v1.schema.json`. Terminals are logical part
//! terminals, so a terminal drawn on several units of one part appears once. Nets carry their
//! persistent registry UUID when they have one and the name the Net Inspector shows. Rows are
//! ordered by the number in the component ID (`component-9` before `component-10`) and then by
//! terminal-table order, never by connectivity traversal, so the same design always gives the
//! same bytes.
//!
//! A board's netlist ([`EXPORT_BOARD_NETLIST`]) uses format 2 with the `accepted` basis:
//! the components and connections that board has accepted through reviewed updates, with each
//! terminal's pads. Its terminal order is native while the live symbol matches the accepted
//! revision and terminal table, otherwise lexical from the accepted baseline.

use crate::{
    CommandError, Project, Session,
    board::fabrication::{Output, file_part},
    schematic::ElectricalType,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

mod accepted;
mod integer;
pub use accepted::EXPORT_BOARD_NETLIST;

/// Generate the project's netlist file; edits nothing.
pub const EXPORT_NETLIST: &str = "project.export_netlist";
/// The format tag of a schematic netlist, the shape `docs/schemas/netlist-v1.schema.json`
/// describes.
pub const FORMAT: &str = "pcbcraft-netlist/1";
/// The format tag of a board's accepted netlist. Format 2 widens format 1 with the board,
/// packages, pads, unknown component kinds and null electrical types;
/// `docs/schemas/netlist-v2.schema.json` describes it.
pub const FORMAT_V2: &str = "pcbcraft-netlist/2";

/// A complete netlist document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Netlist {
    /// [`FORMAT`] for a `schematic` netlist, [`FORMAT_V2`] for an `accepted` one.
    pub format: String,
    pub project: ProjectRef,
    /// Which connectivity the nets describe.
    pub basis: Basis,
    /// The board whose accepted state an `accepted` netlist describes; absent otherwise.
    pub board: Option<BoardRef>,
    /// Components in numeric component ID order: `component-9` before `component-10`.
    pub components: Vec<Component>,
    /// Nets with at least one terminal, ordered by their first terminal.
    pub nets: Vec<Net>,
    /// Terminals on no net, in numeric component ID and then exported terminal order.
    pub unconnected: Vec<TerminalRef>,
}

/// The document's field order and field-level rules, shared by both serde entry points.
#[derive(Serialize, Deserialize)]
#[serde(remote = "Netlist", rename = "Netlist", deny_unknown_fields)]
struct NetlistText {
    format: String,
    #[serde(deserialize_with = "object")]
    project: ProjectRef,
    basis: Basis,
    #[serde(default, deserialize_with = "present_object", skip_serializing_if = "Option::is_none")]
    board: Option<BoardRef>,
    #[serde(deserialize_with = "objects")]
    components: Vec<Component>,
    #[serde(deserialize_with = "objects")]
    nets: Vec<Net>,
    #[serde(deserialize_with = "objects")]
    unconnected: Vec<TerminalRef>,
}

impl Serialize for Netlist {
    /// Check the tagged document before writing any bytes.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.validate_document().map_err(serde::ser::Error::custom)?;
        NetlistText::serialize(self, serializer)
    }
}

impl<'de> Deserialize<'de> for Netlist {
    /// Read one closed document object, then validate its tag and basis-specific fields.
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        struct Document;
        impl<'de> serde::de::Visitor<'de> for Document {
            type Value = Netlist;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a netlist object")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(self, map: A) -> Result<Netlist, A::Error> {
                let netlist = NetlistText::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
                netlist.validate_document().map_err(serde::de::Error::custom)?;
                Ok(netlist)
            }
        }
        de.deserialize_map(Document)
    }
}

/// The exported project's identity and display name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectRef {
    #[serde(deserialize_with = "hyphenated")]
    pub id: Uuid,
    pub name: String,
}

/// The source of the exported connections.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    /// The schematic's current wiring, labels and hierarchy across all sheets.
    Schematic,
    /// The board's accepted connectivity and packages. Reference and value follow the live
    /// component; unapplied wiring changes stay out of the accepted connections.
    Accepted,
}

impl Basis {
    /// The basis as the document spells it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Schematic => "schematic",
            Self::Accepted => "accepted",
        }
    }
}

/// The board an accepted netlist describes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardRef {
    #[serde(deserialize_with = "hyphenated")]
    pub id: Uuid,
    pub name: String,
    /// The source capture stored in the board's accepted baseline, including one initialized
    /// from the current state; null if there is no accepted baseline.
    #[serde(deserialize_with = "nullable_capture")]
    pub source: Option<crate::board::update::SourceCapture>,
}

/// The component's role when known from the definition matching the exported state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentKind {
    Part,
    Power,
    /// An accepted component with no matching live definition to supply its role.
    Unknown,
}

/// One logical component and every terminal of its definition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Component {
    /// Stable component ID, such as `component-3`.
    pub id: String,
    pub kind: ComponentKind,
    pub reference: String,
    pub value: String,
    #[serde(deserialize_with = "object")]
    pub definition: DefinitionRef,
    /// Placed units in unit ID order. Units not placed on any sheet are absent.
    #[serde(deserialize_with = "objects")]
    pub units: Vec<Unit>,
    /// Logical terminals, each listed once. Native order for the schematic or a matching
    /// accepted symbol revision and terminal table; lexical order for an accepted fallback.
    #[serde(deserialize_with = "objects")]
    pub terminals: Vec<TerminalInfo>,
    /// The accepted physical package; present only in an `accepted` netlist.
    #[serde(default, deserialize_with = "present_object", skip_serializing_if = "Option::is_none")]
    pub package: Option<PackageRef>,
}

/// The pinned footprint a board accepted for a component, and where it is placed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageRef {
    /// Null for a legacy two-pad marker, which has no library.
    #[serde(deserialize_with = "nullable")]
    pub library: Option<String>,
    pub id: String,
    #[serde(deserialize_with = "integer::revision")]
    pub revision: u32,
    /// The placement ID, such as `footprint-2`, or null while the footprint is not placed.
    #[serde(deserialize_with = "nullable")]
    pub footprint: Option<String>,
    /// Pads of the footprint that no terminal uses, sorted.
    pub unconnected_pads: Vec<String>,
}

/// The pinned library definition of a component.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefinitionRef {
    pub library: String,
    pub id: String,
    #[serde(deserialize_with = "integer::revision")]
    pub revision: u32,
}

/// Where one unit of a component is placed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Unit {
    /// The unit's ID within its definition, such as `a`.
    pub unit: String,
    /// The placement's symbol ID, such as `symbol-4`.
    pub symbol: String,
    /// The sheet the unit is drawn on.
    #[serde(deserialize_with = "hyphenated")]
    pub sheet: Uuid,
    /// Sheet occurrence IDs from the root sheet down to the unit's own sheet.
    #[serde(deserialize_with = "hyphenated_list")]
    pub occurrence: Vec<Uuid>,
}

/// One logical terminal of a component.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalInfo {
    pub terminal: String,
    /// The terminal's name, or explicit null when it has none.
    #[serde(deserialize_with = "nullable")]
    pub name: Option<String>,
    /// Null only in an `accepted` netlist, for a component whose current symbol is not the
    /// accepted revision with the same terminals.
    #[serde(deserialize_with = "nullable")]
    pub electrical_type: Option<ElectricalType>,
    /// The footprint pads the terminal maps to; present only in an `accepted` netlist.
    #[serde(default, deserialize_with = "present_nonnull", skip_serializing_if = "Option::is_none")]
    pub pads: Option<Vec<String>>,
}

/// One electrical net and the logical terminals it joins.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Net {
    /// Persistent registry UUID, or explicit null for a net the registry does not identify.
    #[serde(deserialize_with = "nullable_hyphenated")]
    pub id: Option<Uuid>,
    /// The current Net Inspector display name for the schematic basis, or the display name
    /// captured in the board's accepted baseline for the accepted basis.
    pub name: String,
    /// Authored names on the net, sorted; captured at the accepted update for the accepted basis.
    pub names: Vec<String>,
    /// Joined terminals in numeric component ID and then exported terminal order.
    #[serde(deserialize_with = "objects")]
    pub terminals: Vec<TerminalRef>,
}

/// A logical part terminal address, in the shape commands accept.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalRef {
    pub part: String,
    pub terminal: String,
}

/// A record must be an object, not the positional sequence a derived struct can also read.
struct Object<T>(T);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Object<T> {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        struct Record<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Record<T> {
            type Value = Object<T>;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an object")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                T::deserialize(serde::de::value::MapAccessDeserializer::new(map)).map(Object)
            }
        }
        de.deserialize_map(Record(std::marker::PhantomData))
    }
}

fn object<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(de: D) -> Result<T, D::Error> {
    Object::deserialize(de).map(|Object(value)| value)
}

fn objects<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(de: D) -> Result<Vec<T>, D::Error> {
    Vec::<Object<T>>::deserialize(de).map(|records| records.into_iter().map(|Object(value)| value).collect())
}

fn present_object<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(de: D) -> Result<Option<T>, D::Error> {
    object(de).map(Some)
}

/// Read a value only when the field is present, refusing explicit null. Paired with
/// `serde(default)`, omission stays `None` without confusing it with a null value.
fn present_nonnull<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(de: D) -> Result<Option<T>, D::Error> {
    T::deserialize(de).map(Some)
}

/// Read an optional value that must still be present: the format writes null rather than
/// omitting the field, so a missing field is an error instead of a silent `None`.
fn nullable<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(de: D) -> Result<Option<T>, D::Error> {
    Option::deserialize(de)
}

/// A UUID in the schema's `uuid` format: the 36-character hyphenated form only. The simple,
/// braced and `urn:uuid:` spellings other UUID readers accept are refused.
struct Hyphenated(Uuid);

impl<'de> Deserialize<'de> for Hyphenated {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let text = String::deserialize(de)?;
        parse_hyphenated(&text).map(Hyphenated).ok_or_else(|| serde::de::Error::custom(format!("`{text}` is not a hyphenated UUID")))
    }
}

/// Parse the 36-character hyphenated UUID form, returning `None` for any other text.
pub(crate) fn parse_hyphenated(text: &str) -> Option<Uuid> {
    if text.len() == 36 { Uuid::try_parse(text).ok() } else { None }
}

/// Read a required UUID in hyphenated form.
fn hyphenated<'de, D: serde::Deserializer<'de>>(de: D) -> Result<Uuid, D::Error> {
    Hyphenated::deserialize(de).map(|id| id.0)
}

/// Read a list of UUIDs, each in hyphenated form.
fn hyphenated_list<'de, D: serde::Deserializer<'de>>(de: D) -> Result<Vec<Uuid>, D::Error> {
    Vec::<Hyphenated>::deserialize(de).map(|ids| ids.into_iter().map(|id| id.0).collect())
}

/// Read a UUID that must be present but may be null, in hyphenated form when given.
fn nullable_hyphenated<'de, D: serde::Deserializer<'de>>(de: D) -> Result<Option<Uuid>, D::Error> {
    Option::<Hyphenated>::deserialize(de).map(|id| id.map(|id| id.0))
}

/// A source capture as the netlist format writes it, so its `capture_id` is read in the
/// hyphenated form like every other UUID in the document.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureText {
    capture_id: Hyphenated,
    content_digest: String,
}

/// Read a source capture that must be present but may be null, with a hyphenated capture ID.
fn nullable_capture<'de, D: serde::Deserializer<'de>>(de: D) -> Result<Option<crate::board::update::SourceCapture>, D::Error> {
    Option::<Object<CaptureText>>::deserialize(de).map(|capture| {
        capture
            .map(|Object(capture)| crate::board::update::SourceCapture { capture_id: capture.capture_id.0, content_digest: capture.content_digest })
    })
}

impl Netlist {
    /// Read a netlist document, refusing other formats, unknown fields and malformed values.
    /// The format tag is read first, so a document of a later format is refused by its tag
    /// rather than by the first field this reader does not know. Omitted fields are refused
    /// even where null is allowed, and so are fields that do not belong to the document's
    /// basis: see [`Netlist::validate_document`]. The tag must be the one the content needs: format 1
    /// for a `schematic` netlist and format 2 for an `accepted` one, so a format 1 reader never
    /// meets format 2 fields under its own tag.
    pub fn parse(text: &str) -> Result<Self, String> {
        #[derive(Deserialize)]
        struct Header {
            format: String,
        }
        let Object(header): Object<Header> = serde_json::from_str(text).map_err(|error| format!("invalid netlist: {error}"))?;
        if header.format != FORMAT && header.format != FORMAT_V2 {
            return Err(format!("unsupported netlist format `{}`", header.format));
        }
        serde_json::from_str(text).map_err(|error| format!("invalid netlist: {error}"))
    }

    /// The format tag a netlist of this basis is written with: the lowest one that carries it.
    pub fn format_for(basis: Basis) -> &'static str {
        match basis {
            Basis::Schematic => FORMAT,
            Basis::Accepted => FORMAT_V2,
        }
    }

    /// Check the tag/basis pair and the fields that depend on it, on both sides of the codec.
    /// Accepted documents name a board, give terminals their pads and list no units. Schematic
    /// documents have no board, packages, pads or unknown part kinds, and require electrical types.
    fn validate_document(&self) -> Result<(), String> {
        if (self.basis != Basis::Accepted || self.format != FORMAT_V2)
            && self.components.iter().any(|component| component.kind == ComponentKind::Unknown)
        {
            return Err("unknown component kinds require an accepted format 2 netlist".into());
        }
        let expected = Self::format_for(self.basis);
        if self.format != expected {
            return Err(format!("unsupported netlist format `{}` for the {} basis, which takes `{expected}`", self.format, self.basis.name()));
        }
        let terminals = || self.components.iter().flat_map(|component| &component.terminals);
        match self.basis {
            Basis::Accepted => {
                let board = self.board.as_ref().ok_or("an accepted netlist names its board")?;
                if let Some(source) = &board.source {
                    source.validate()?;
                }
                if self.components.iter().any(|component| !component.units.is_empty()) {
                    return Err("an accepted netlist lists no units".into());
                }
                if terminals().any(|terminal| terminal.pads.is_none()) {
                    return Err("every terminal of an accepted netlist lists its pads".into());
                }
            }
            Basis::Schematic => {
                if self.board.is_some() || self.components.iter().any(|component| component.package.is_some()) {
                    return Err("only an accepted netlist names a board or packages".into());
                }
                if terminals().any(|terminal| terminal.pads.is_some() || terminal.electrical_type.is_none()) {
                    return Err("a schematic netlist terminal has an electrical type and no pads".into());
                }
            }
        }
        Ok(())
    }

    /// The document text: two-space indented JSON with a final newline.
    pub fn to_text(&self) -> Result<String, String> {
        let mut text = serde_json::to_string_pretty(self).map_err(|error| error.to_string())?;
        text.push('\n');
        Ok(text)
    }
}

/// The netlist and what a reader should know about it, such as nets without a persistent ID.
#[derive(Clone, Debug)]
pub struct Export {
    pub netlist: Netlist,
    pub findings: Vec<Value>,
}

impl Project {
    /// Build the netlist from the complete electrical connectivity of every sheet.
    /// Repeated appearances of a terminal on several units collapse to one logical terminal.
    /// Fails when the net registry cannot be resolved against the current hierarchy.
    pub fn netlist(&self) -> Result<Export, String> {
        let electrical = self.electrical_connectivity();
        let ids = self.persistent_net_ids()?;
        let occurrences = self.occurrence_instances().sheet_occurrences;
        // Each sheet has at most one occurrence today, because repeated sheet instances are
        // refused, so the first occurrence of a sheet is its only one.
        let path = |sheet: Uuid| {
            let mut path = Vec::new();
            let mut next = occurrences.iter().find(|record| record.definition == sheet);
            while let Some(record) = next {
                path.push(record.id);
                next = record.parent.and_then(|parent| occurrences.iter().find(|record| record.id == parent));
            }
            path.reverse();
            path
        };
        let mut owners = BTreeMap::new();
        let mut units = BTreeMap::<u64, Vec<Unit>>::new();
        for sheet in self.sheets() {
            let projection = self.for_sheet(sheet.target).ok_or("Unknown sheet while exporting the netlist.")?;
            let occurrence = path(sheet.target.id());
            for symbol in projection.symbols() {
                owners.insert(symbol.ordinal, symbol.component);
                units.entry(symbol.component).or_default().push(Unit {
                    unit: symbol.unit_id().to_owned(),
                    symbol: symbol.id(),
                    sheet: sheet.target.id(),
                    occurrence: occurrence.clone(),
                });
            }
        }
        let mut findings = Vec::new();
        let components = self
            .components
            .values()
            .map(|part| {
                let mut placed = units.remove(&part.ordinal).unwrap_or_default();
                placed.sort_by(|a, b| a.unit.cmp(&b.unit).then_with(|| a.symbol.cmp(&b.symbol)));
                for unit in part.definition.units.iter().filter(|unit| !placed.iter().any(|entry| entry.unit == unit.unit_id())) {
                    findings.push(json!({"kind":"unplaced_unit","part":part.id(),"unit":unit.unit_id(),
                        "message":format!("{} unit {} is not placed; its terminals have no connections.", part.reference(), unit.unit_id())}));
                }
                Component {
                    id: part.id(),
                    kind: if part.definition.power_terminal().is_some() { ComponentKind::Power } else { ComponentKind::Part },
                    reference: part.reference().to_owned(),
                    value: part.value().to_owned(),
                    definition: DefinitionRef {
                        library: part.definition.library.clone(),
                        id: part.definition.id.clone(),
                        revision: part.definition.revision(),
                    },
                    units: placed,
                    terminals: part
                        .definition
                        .pins()
                        .iter()
                        .map(|pin| TerminalInfo {
                            terminal: pin.number.clone(),
                            name: pin.name.clone(),
                            electrical_type: Some(pin.electrical_type),
                            pads: None,
                        })
                        .collect(),
                    package: None,
                }
            })
            .collect();
        // Terminals sort by the component ID's number, then by the definition's terminal order.
        let order = |part: u64, terminal: &str| {
            let index = self.components.get(&part).and_then(|component| component.definition.pin_index(terminal)).unwrap_or(usize::MAX);
            (part, index)
        };
        let logical = |terminals: &[crate::schematic::Terminal]| {
            let mut members: Vec<_> = terminals
                .iter()
                .filter_map(|terminal| owners.get(&terminal.symbol).map(|&part| (order(part, terminal.number()), part, terminal.number().to_owned())))
                .collect();
            members.sort();
            members.dedup();
            members
        };
        let reference = |(_, part, terminal): (_, u64, String)| TerminalRef { part: format!("component-{part}"), terminal };
        let mut joined = BTreeSet::new();
        let mut nets = Vec::new();
        for (index, group) in electrical.nets.iter().enumerate() {
            let members = logical(group);
            joined.extend(members.iter().map(|member| member.0));
            let Some(&(key, _, _)) = members.first() else {
                continue;
            };
            let id = ids.get(index).copied().flatten();
            let name = electrical.net_name(index);
            if id.is_none() {
                findings.push(json!({"kind":"unregistered_net","net":name,"message":format!("Net {name} has no persistent ID; its id is null.")}));
            }
            let terminals = members.into_iter().map(reference).collect();
            nets.push((key, Net { id, name, names: electrical.net_names(index).to_vec(), terminals }));
        }
        nets.sort_by_key(|(key, _)| *key);
        // A terminal drawn on several units can be wired on one and left open on another.
        let unconnected = logical(&electrical.unconnected).into_iter().filter(|member| !joined.contains(&member.0)).map(reference).collect();
        let netlist = Netlist {
            format: FORMAT.to_owned(),
            project: ProjectRef { id: self.id, name: self.name.clone() },
            basis: Basis::Schematic,
            board: None,
            components,
            nets: nets.into_iter().map(|(_, net)| net).collect(),
            unconnected,
        };
        Ok(Export { netlist, findings })
    }
}

impl Session {
    /// Build the open project's netlist file without editing anything. Fails when no project
    /// is open or its net registry does not resolve.
    pub fn export_netlist(&self) -> Result<(Output, Vec<Value>), CommandError> {
        let project = self.open_project()?;
        let export = project.netlist().map_err(CommandError::Unavailable)?;
        let contents = export.netlist.to_text().map_err(CommandError::Unavailable)?;
        Ok((Output { name: format!("{}-netlist.json", file_part(&project.name)), contents }, export.findings))
    }

    /// The execute result: the file's name and text (only its name for a dry run) and the findings.
    pub(crate) fn netlist_report(&self, params: &Value) -> Result<Value, CommandError> {
        let (file, findings) = self.export_netlist()?;
        Ok(report(file, findings, params))
    }
}

/// The shape both netlist commands return; a dry run drops the file text.
fn report(file: Output, findings: Vec<Value>, params: &Value) -> Value {
    let file = if params["dry_run"].as_bool() == Some(true) { json!({"name": file.name}) } else { json!(file) };
    json!({"files": [file], "findings": findings})
}

#[cfg(test)]
mod tests;
