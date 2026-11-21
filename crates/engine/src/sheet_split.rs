//! Read-only preview of moving selected circuitry from one sheet onto a new child sheet.
//!
//! The preview names the objects that would move with their identities unchanged, the nets
//! that would cross the new sheet boundary and need a hierarchical port, and the nets that
//! keep connecting by a global or power name without a port. It never edits the project.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::{Value, json};

use crate::{
    CommandError, Project, Session,
    hierarchy::{Direction, MAX_LABELS},
    schematic::{ElectricalType, WireEndpoint, wiring::Terminal},
    selection::Selection,
    sheets::MAX_SHEETS,
};

/// Preview splitting selected schematic objects onto a new child sheet without editing.
pub const PREVIEW: &str = "schematic.preview_split_sheet";

#[cfg(test)]
mod tests;

/// One selected placement and the part instance it belongs to.
#[derive(Serialize)]
struct MovedSymbol {
    symbol: String,
    part_instance: String,
    reference: String,
}

/// A part instance with units both inside and outside the selection. Units of one part may
/// sit on different sheets, so this is reported rather than refused.
#[derive(Serialize)]
struct SplitPart {
    part_instance: String,
    reference: String,
    moved: Vec<String>,
    staying: Vec<String>,
}

/// A net with members on both sides of the boundary that would need one hierarchical label
/// in the new sheet and one pin on its sheet symbol.
#[derive(Serialize)]
struct Port {
    name: String,
    /// True when the name is the net's generated number rather than an authored name.
    generated: bool,
    direction: &'static str,
    inside: Vec<Value>,
    outside: Vec<Value>,
}

/// A boundary-crossing net that keeps connecting through the same global or power name on
/// both sides, so it needs no port.
#[derive(Serialize)]
struct GlobalNet {
    name: String,
    inside: Vec<Value>,
    outside: Vec<Value>,
}

/// A wire with one end inside the selection and the other outside it.
#[derive(Serialize)]
struct CutWire {
    wire: String,
    inside: Value,
    outside: Value,
}

#[derive(Serialize)]
struct Preview {
    sheet: uuid::Uuid,
    name: String,
    symbols: Vec<MovedSymbol>,
    part_instances: Vec<String>,
    split_part_instances: Vec<SplitPart>,
    wires: Vec<String>,
    junctions: Vec<String>,
    net_labels: Vec<String>,
    no_connects: Value,
    ports: Vec<Port>,
    global_nets: Vec<GlobalNet>,
    cut_wires: Vec<CutWire>,
}

/// Electrical types that drive a net, including tri-state and open drivers.
fn drives(kind: ElectricalType) -> bool {
    matches!(
        kind,
        ElectricalType::Output | ElectricalType::PowerOut | ElectricalType::TriState | ElectricalType::OpenCollector | ElectricalType::OpenEmitter
    )
}

/// Propose the port direction from the child's point of view. A net driven only from inside
/// is an output (tri-state when every inside driver is tri-state), one driven only from
/// outside is an input, and one driven from both sides or carrying a bidirectional pin is
/// bidirectional. Anything else, such as a net of passive pins, is passive.
fn direction(inside: &[ElectricalType], outside: &[ElectricalType]) -> Direction {
    if inside.iter().chain(outside).any(|kind| *kind == ElectricalType::Bidirectional) {
        return Direction::Bidirectional;
    }
    let inside_drivers: Vec<_> = inside.iter().copied().filter(|kind| drives(*kind)).collect();
    let outside_drives = outside.iter().any(|kind| drives(*kind));
    match (inside_drivers.is_empty(), outside_drives) {
        (false, true) => Direction::Bidirectional,
        (false, false) if inside_drivers.iter().all(|kind| *kind == ElectricalType::TriState) => Direction::TriState,
        (false, false) => Direction::Output,
        (true, true) => Direction::Input,
        (true, false) => Direction::Passive,
    }
}

/// The electrical type of every terminal in the project, keyed by its project-unique address.
fn terminal_types(project: &Project) -> BTreeMap<Terminal, ElectricalType> {
    project
        .all_symbols()
        .flat_map(|symbol| symbol.terminals().into_iter().zip(symbol.unit_definition().pins().iter().map(|pin| pin.electrical_type)))
        .collect()
}

/// Global net labels and power terminals on every sheet, with the name each one carries.
fn global_anchors(project: &Project) -> BTreeMap<WireEndpoint, String> {
    let mut anchors = BTreeMap::new();
    for sheet in project.sheets() {
        let Some(view) = project.for_sheet(sheet.target) else {
            continue;
        };
        for label in view.net_labels().iter().filter(|label| label.style.global()) {
            anchors.insert(WireEndpoint::NetLabel(label.id), label.name.clone());
        }
        for (end, name, _) in view.power_terminals() {
            anchors.insert(end, name.to_owned());
        }
    }
    anchors
}

/// Reject a sheet name the shared child-sheet command would refuse, or a full project.
fn check_new_sheet(project: &Project, params: &Value) -> Result<String, CommandError> {
    let name = crate::name_param(params)?;
    if project.sheets().len() >= MAX_SHEETS {
        return Err(CommandError::CapExceeded(format!("project already has {MAX_SHEETS} sheets")));
    }
    if project.sheets().any(|sheet| sheet.name.to_lowercase() == name.to_lowercase()) {
        return Err(CommandError::InvalidParams("a sheet with that name already exists".into()));
    }
    Ok(name)
}

/// Build the preview for a validated selection on the addressed sheet projection.
fn plan(project: &Project, items: &Selection, name: String) -> Result<Preview, CommandError> {
    let mut symbols = Vec::new();
    let mut parts = BTreeSet::new();
    let mut inside: BTreeSet<WireEndpoint> = BTreeSet::new();
    for symbol in project.symbols().iter().filter(|symbol| items.symbols.contains(&symbol.id())) {
        let reference = project.component(symbol).map(|part| part.reference().to_owned()).unwrap_or_default();
        symbols.push(MovedSymbol { symbol: symbol.id(), part_instance: symbol.component_id(), reference });
        parts.insert(symbol.component);
        inside.extend(symbol.terminals().into_iter().map(WireEndpoint::Terminal));
    }
    inside.extend(
        project.net_labels().iter().filter(|label| items.net_labels.contains(&label.object_id())).map(|label| WireEndpoint::NetLabel(label.id)),
    );

    let mut split = Vec::new();
    for &ordinal in &parts {
        let units: Vec<_> = project.all_symbols().filter(|symbol| symbol.component == ordinal).collect();
        let staying: Vec<_> = units.iter().filter(|symbol| !items.symbols.contains(&symbol.id())).map(|symbol| symbol.id()).collect();
        if let (false, Some(unit)) = (staying.is_empty(), units.first()) {
            split.push(SplitPart {
                part_instance: unit.component_id(),
                reference: project.component(unit).map(|part| part.reference().to_owned()).unwrap_or_default(),
                moved: units.iter().filter(|symbol| items.symbols.contains(&symbol.id())).map(|symbol| symbol.id()).collect(),
                staying,
            });
        }
    }

    // A wire travels when it is selected or both its ends are selected anchors. A junction
    // travels only when every wire meeting it travels, so it never strands an outside route.
    let candidates: BTreeSet<_> = project
        .wires()
        .iter()
        .filter(|wire| items.wires.contains(&wire.id()) || wire.endpoints().iter().all(|end| inside.contains(end)))
        .map(|wire| wire.ordinal())
        .collect();
    let mut incident: BTreeMap<u64, bool> = BTreeMap::new();
    for wire in project.wires() {
        for end in wire.endpoints() {
            if let WireEndpoint::Junction(id) = end {
                *incident.entry(id).or_insert(true) &= candidates.contains(&wire.ordinal());
            }
        }
    }
    let junctions: BTreeSet<_> = incident.into_iter().filter(|(_, all)| *all).map(|(id, _)| id).collect();
    let within = |end: &WireEndpoint| inside.contains(end) || matches!(end, WireEndpoint::Junction(id) if junctions.contains(id));
    let mut wires = Vec::new();
    let mut cut_wires = Vec::new();
    for wire in project.wires() {
        let [from, to] = wire.endpoints();
        match (within(&from), within(&to)) {
            (true, true) if candidates.contains(&wire.ordinal()) => wires.push(wire.id()),
            (true, false) => cut_wires.push(CutWire { wire: wire.id(), inside: from.inspect(), outside: to.inspect() }),
            (false, true) => cut_wires.push(CutWire { wire: wire.id(), inside: to.inspect(), outside: from.inspect() }),
            _ => {}
        }
    }

    let graph = project.borrowed_electrical_connectivity();
    let types = terminal_types(project);
    let globals = global_anchors(project);
    let nets: BTreeSet<_> = inside.iter().filter_map(|end| graph.net_index_for_endpoint(*end)).collect();
    let mut ports = Vec::new();
    let mut global_nets = Vec::new();
    for index in nets {
        let (members_in, members_out): (Vec<_>, Vec<_>) =
            graph.endpoints(index).filter(|end| !matches!(end, WireEndpoint::Junction(_))).partition(|end| inside.contains(end));
        if members_in.is_empty() || members_out.is_empty() {
            continue;
        }
        let names = |members: &[WireEndpoint]| -> BTreeSet<String> { members.iter().filter_map(|end| globals.get(end).cloned()).collect() };
        let shared = names(members_in.as_slice()).intersection(&names(members_out.as_slice())).next().cloned();
        let listed = |members: &[WireEndpoint]| -> Vec<Value> { members.iter().map(|end| end.inspect()).collect() };
        if let Some(name) = shared {
            global_nets.push(GlobalNet { name, inside: listed(members_in.as_slice()), outside: listed(members_out.as_slice()) });
            continue;
        }
        let name = graph
            .borrowed_net_name(index)
            .ok_or_else(|| CommandError::Unavailable("A boundary net has neither a name nor a generated number.".into()))?
            .to_owned();
        let kinds = |members: &[WireEndpoint]| -> Vec<ElectricalType> {
            members
                .iter()
                .filter_map(|end| match end {
                    WireEndpoint::Terminal(terminal) => types.get(terminal).copied(),
                    _ => None,
                })
                .collect()
        };
        ports.push(Port {
            name,
            generated: graph.net_names(index).is_empty(),
            direction: direction(&kinds(members_in.as_slice()), &kinds(members_out.as_slice())).as_str(),
            inside: listed(members_in.as_slice()),
            outside: listed(members_out.as_slice()),
        });
    }
    if ports.len() > MAX_LABELS {
        return Err(CommandError::CapExceeded(format!(
            "The selection crosses {} nets; a sheet interface holds at most {MAX_LABELS} ports.",
            ports.len()
        )));
    }
    ports.sort_by(|a, b| a.name.cmp(&b.name));
    if let Some(pair) = ports.windows(2).find(|pair| pair.first().map(|port| &port.name) == pair.last().map(|port| &port.name)) {
        let name = pair.first().map(|port| port.name.as_str()).unwrap_or_default();
        return Err(CommandError::InvalidParams(format!("Two boundary nets would share the port name {name}.")));
    }
    global_nets.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(Preview {
        sheet: project.sheet_target().id(),
        name,
        symbols,
        part_instances: parts.iter().map(|ordinal| format!("component-{ordinal}")).collect(),
        split_part_instances: split,
        wires,
        junctions: junctions.iter().map(|id| format!("junction-{id}")).collect(),
        net_labels: items.net_labels.iter().cloned().collect(),
        no_connects: serde_json::to_value(&items.no_connects).map_err(|error| CommandError::InvalidParams(error.to_string()))?,
        ports,
        global_nets,
        cut_wires,
    })
}

impl Session {
    /// Report what splitting the selection onto a new child sheet would move and which nets
    /// would become ports, at the current revision. Nothing is cached and nothing changes:
    /// the revision, history and project documents stay as they were.
    pub(super) fn preview_split(&self, params: &Value) -> Result<Value, CommandError> {
        let project = self.open_project()?;
        let target = project.resolve_sheet(params.get("sheet").and_then(Value::as_str))?;
        let view = project.for_sheet(target).ok_or_else(|| CommandError::NotFound("unknown sheet ID".into()))?;
        let name = check_new_sheet(project, params)?;
        let items = Selection::parse(&params["items"]).ok_or_else(|| CommandError::InvalidParams("invalid selection".into()))?;
        items.validate(&view)?;
        items.validate_marker_owners(&view)?;
        let preview = plan(&view, &items, name)?;
        Ok(json!({"revision":self.revision(), "split_preview":preview}))
    }
}
