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

use crate::{
    CommandError, Project, Session,
    board::fabrication::{Output, file_part},
    schematic::ElectricalType,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Generate the project's netlist file; edits nothing.
pub const EXPORT_NETLIST: &str = "project.export_netlist";
/// The format tag written into, and required from, every netlist document.
pub const FORMAT: &str = "pcbcraft-netlist/1";

/// A complete netlist document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Netlist {
    /// Always [`FORMAT`].
    pub format: String,
    pub project: ProjectRef,
    /// Which connectivity the nets describe.
    pub basis: Basis,
    /// Components in numeric component ID order: `component-9` before `component-10`.
    pub components: Vec<Component>,
    /// Nets with at least one terminal, ordered by their first terminal.
    pub nets: Vec<Net>,
    /// Terminals of placed units that are on no net, in numeric component ID and then
    /// terminal-table order.
    pub unconnected: Vec<TerminalRef>,
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
}

/// Whether a component is an ordinary part or a power port that names its net.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentKind {
    Part,
    Power,
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
    pub definition: DefinitionRef,
    /// Placed units in unit ID order. Units not placed on any sheet are absent.
    pub units: Vec<Unit>,
    /// The definition's logical terminals in native terminal order, each listed once.
    pub terminals: Vec<TerminalInfo>,
}

/// The pinned library definition of a component.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefinitionRef {
    pub library: String,
    pub id: String,
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
    pub electrical_type: ElectricalType,
}

/// One electrical net and the logical terminals it joins.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Net {
    /// Persistent registry UUID, or explicit null for a net the registry does not identify.
    #[serde(deserialize_with = "nullable_hyphenated")]
    pub id: Option<Uuid>,
    /// The name the Net Inspector shows.
    pub name: String,
    /// Every authored name on the net, sorted.
    pub names: Vec<String>,
    /// Joined terminals in numeric component ID and then terminal-table order.
    pub terminals: Vec<TerminalRef>,
}

/// A logical part terminal address, in the shape commands accept.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalRef {
    pub part: String,
    pub terminal: String,
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

impl Netlist {
    /// Read a netlist document, refusing other formats, unknown fields and malformed values.
    pub fn parse(text: &str) -> Result<Self, String> {
        let netlist: Self = serde_json::from_str(text).map_err(|error| format!("invalid netlist: {error}"))?;
        if netlist.format != FORMAT {
            return Err(format!("unsupported netlist format `{}`", netlist.format));
        }
        Ok(netlist)
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
                        .map(|pin| TerminalInfo { terminal: pin.number.clone(), name: pin.name.clone(), electrical_type: pin.electrical_type })
                        .collect(),
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
        let file = if params["dry_run"].as_bool() == Some(true) { json!({"name": file.name}) } else { json!(file) };
        Ok(json!({"files": [file], "findings": findings}))
    }
}

#[cfg(test)]
mod tests;
