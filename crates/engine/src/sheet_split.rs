//! Moving selected circuitry from one sheet onto a new child sheet, with a read-only preview.
//!
//! The preview names the objects that would move with their identities unchanged, the nets
//! that would cross the new sheet boundary and need a hierarchical port, and the nets that
//! keep connecting by a global or power name without a port. It never edits the project.
//! The split applies exactly that plan as one undo step and refuses any result whose
//! netlist differs from the one it started from.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use serde::Serialize;
use serde_json::{Value, json};

use crate::{
    CommandError, Project, Session,
    clipboard::allocate,
    hierarchy::{self, Direction, MAX_LABELS, SheetSymbol},
    no_connect::Attachment,
    schematic::{Connectivity, ElectricalType, GRID_NM, Point, Wire, WireEndpoint, wiring::Terminal},
    selection::Selection,
    sheets::MAX_SHEETS,
};

/// Preview splitting selected schematic objects onto a new child sheet without editing.
pub const PREVIEW: &str = "schematic.preview_split_sheet";

/// Move selected schematic objects onto a new child sheet as one undo step.
pub const SPLIT: &str = "schematic.split_sheet";

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
    direction: Direction,
    inside: Vec<Value>,
    outside: Vec<Value>,
    #[serde(skip)]
    net: usize,
    /// Inside and outside members, as addressed endpoints.
    #[serde(skip)]
    members: [Vec<WireEndpoint>; 2],
}

/// A boundary-crossing net that keeps connecting through the same global or power name on
/// both sides, so it needs no port.
#[derive(Serialize)]
struct GlobalNet {
    name: String,
    inside: Vec<Value>,
    outside: Vec<Value>,
    /// Inside and outside members, as addressed endpoints.
    #[serde(skip)]
    members: [Vec<WireEndpoint>; 2],
    /// A member on each side that carries the shared name, where one exists.
    #[serde(skip)]
    anchors: [Option<WireEndpoint>; 2],
}

/// A wire with one end inside the selection and the other outside it.
#[derive(Serialize)]
struct CutWire {
    wire: String,
    inside: Value,
    outside: Value,
    #[serde(skip)]
    ordinal: u64,
    #[serde(skip)]
    outside_end: WireEndpoint,
    /// The net of the inside end, before the split.
    #[serde(skip)]
    net: Option<usize>,
}

/// What moves, by identity, for the apply step.
#[derive(Default)]
struct Moved {
    symbols: BTreeSet<u64>,
    wires: BTreeSet<u64>,
    junctions: BTreeSet<u64>,
    net_labels: BTreeSet<u64>,
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
    no_connects: Vec<Attachment>,
    /// Selected wires that stay on this sheet, because neither of their ends moves.
    staying_wires: Vec<String>,
    /// Selected no-connect markers whose pin or junction stays on this sheet.
    staying_no_connects: Vec<Attachment>,
    ports: Vec<Port>,
    global_nets: Vec<GlobalNet>,
    cut_wires: Vec<CutWire>,
    #[serde(skip)]
    moved: Moved,
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

/// The effective electrical type of every terminal in the project, keyed by its project-unique
/// address. A selected alternate pin function replaces the primary type, as in the netlist.
fn terminal_types(project: &Project) -> BTreeMap<Terminal, ElectricalType> {
    project
        .all_symbols()
        .flat_map(|symbol| {
            let choices = project.pin_functions.owner(symbol.component);
            let types = symbol.unit_definition().pins().iter().map(move |pin| crate::pin_functions::effective(choices, pin).1);
            symbol.terminals().into_iter().zip(types)
        })
        .collect()
}

/// Global net labels and power terminals, with the name and owning sheet index of each.
fn global_anchors(project: &Project) -> BTreeMap<WireEndpoint, (String, usize)> {
    let mut anchors = BTreeMap::new();
    for sheet in project.sheets() {
        let Some(view) = project.for_sheet(sheet.target) else {
            continue;
        };
        for label in view.net_labels().iter().filter(|label| label.style.global()) {
            anchors.insert(WireEndpoint::NetLabel(label.id), (label.name.clone(), view.sheet_index));
        }
        for (end, name, _) in view.power_terminals() {
            anchors.insert(end, (name.to_owned(), view.sheet_index));
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
    let mut moved = Moved::default();
    let mut parts = BTreeSet::new();
    let mut inside: BTreeSet<WireEndpoint> = BTreeSet::new();
    for symbol in project.symbols().iter().filter(|symbol| items.symbols.contains(&symbol.id())) {
        symbols.push(MovedSymbol { symbol: symbol.id(), part_instance: symbol.component_id(), reference: project.reference(symbol) });
        parts.insert(symbol.component);
        moved.symbols.insert(symbol.ordinal);
        inside.extend(symbol.terminals().into_iter().map(WireEndpoint::Terminal));
    }
    for label in project.net_labels().iter().filter(|label| items.net_labels.contains(&label.object_id())) {
        moved.net_labels.insert(label.id);
        inside.insert(WireEndpoint::NetLabel(label.id));
    }
    // Wires and markers follow what they attach to, so they cannot be split off on their own.
    if moved.symbols.is_empty() && moved.net_labels.is_empty() {
        return Err(CommandError::InvalidParams("Select at least one symbol or net label to move.".into()));
    }

    let mut units_of: BTreeMap<u64, Vec<_>> = BTreeMap::new();
    for symbol in project.all_symbols().filter(|symbol| parts.contains(&symbol.component)) {
        units_of.entry(symbol.component).or_default().push(symbol);
    }
    let mut split = Vec::new();
    for units in units_of.values() {
        let staying: Vec<_> = units.iter().filter(|symbol| !items.symbols.contains(&symbol.id())).map(|symbol| symbol.id()).collect();
        if let (false, Some(unit)) = (staying.is_empty(), units.first()) {
            split.push(SplitPart {
                part_instance: unit.component_id(),
                reference: project.reference(unit),
                moved: units.iter().filter(|symbol| items.symbols.contains(&symbol.id())).map(|symbol| symbol.id()).collect(),
                staying,
            });
        }
    }

    // A junction travels when every wire meeting it leads to a selected anchor or another
    // travelling junction, and its routes reach the selection. A wire travels when both of
    // its ends travel and is cut when only one does, so no route is stranded on either side.
    let mut branches: BTreeMap<u64, Vec<WireEndpoint>> = BTreeMap::new();
    for wire in project.wires() {
        let [from, to] = wire.endpoints();
        for (end, other) in [(from, to), (to, from)] {
            if let WireEndpoint::Junction(id) = end {
                branches.entry(id).or_default().push(other);
            }
        }
    }
    let neighbours = |id: u64| branches.get(&id).map_or(&[][..], Vec::as_slice);
    let mut travelling: BTreeSet<u64> = branches.keys().copied().collect();
    let mut pending: Vec<u64> = travelling.iter().copied().collect();
    while let Some(id) = pending.pop() {
        let leaves = neighbours(id).iter().any(|end| match end {
            WireEndpoint::Junction(other) => !travelling.contains(other),
            _ => !inside.contains(end),
        });
        if leaves && travelling.remove(&id) {
            pending.extend(neighbours(id).iter().filter_map(|end| match end {
                WireEndpoint::Junction(other) if travelling.contains(other) => Some(*other),
                _ => None,
            }));
        }
    }
    let mut junctions = BTreeSet::new();
    let mut pending: Vec<u64> = travelling.iter().copied().filter(|id| neighbours(*id).iter().any(|end| inside.contains(end))).collect();
    while let Some(id) = pending.pop() {
        if junctions.insert(id) {
            pending.extend(neighbours(id).iter().filter_map(|end| match end {
                WireEndpoint::Junction(other) if travelling.contains(other) => Some(*other),
                _ => None,
            }));
        }
    }
    let within = |end: &WireEndpoint| inside.contains(end) || matches!(end, WireEndpoint::Junction(id) if junctions.contains(id));
    let graph = project.borrowed_electrical_connectivity();
    let cut = |wire: &Wire, inside_end: WireEndpoint, outside_end: WireEndpoint| CutWire {
        wire: wire.id(),
        inside: inside_end.inspect(),
        outside: outside_end.inspect(),
        ordinal: wire.ordinal(),
        outside_end,
        net: graph.net_index_for_endpoint(inside_end),
    };
    let mut wires = Vec::new();
    let mut cut_wires = Vec::new();
    let mut staying_wires = Vec::new();
    for wire in project.wires() {
        let [from, to] = wire.endpoints();
        match (within(&from), within(&to)) {
            (true, true) => {
                moved.wires.insert(wire.ordinal());
                wires.push(wire.id());
            }
            (true, false) => cut_wires.push(cut(wire, from, to)),
            (false, true) => cut_wires.push(cut(wire, to, from)),
            // A cut wire moves with its inside end; one with neither end inside stays.
            (false, false) if items.wires.contains(&wire.id()) => staying_wires.push(wire.id()),
            (false, false) => {}
        }
    }
    moved.junctions = junctions.clone();
    let moved_terminals: BTreeSet<(String, String)> = project
        .symbols()
        .iter()
        .filter(|symbol| moved.symbols.contains(&symbol.ordinal))
        .flat_map(|symbol| symbol.terminals().into_iter().map(move |terminal| (symbol.component_id(), terminal.number().to_owned())))
        .collect();
    let (travelling_markers, staying_markers): (Vec<_>, Vec<_>) = items.no_connects.iter().cloned().partition(|marker| match marker {
        Attachment::Pin(target) => moved_terminals.contains(&(target.part.clone(), target.terminal.clone())),
        Attachment::WireEnd(target) => target.ordinal().is_some_and(|id| junctions.contains(&id)),
    });

    let types = terminal_types(project);
    let globals = global_anchors(project);
    let cut_nets: BTreeSet<_> = cut_wires.iter().filter_map(|wire| wire.net).collect();
    let nets: BTreeSet<_> = inside.iter().filter_map(|end| graph.net_index_for_endpoint(*end)).collect();
    let mut ports = Vec::new();
    let mut global_nets = Vec::new();
    for index in nets {
        // Junctions decide whether a net crosses, as a junction that stays is reached by a cut
        // wire, but only anchors are listed.
        let (members_in, members_out): (Vec<_>, Vec<_>) = graph.endpoints(index).partition(|end| within(end));
        if members_in.is_empty() || members_out.is_empty() {
            continue;
        }
        let anchors = |members: Vec<WireEndpoint>| -> Vec<WireEndpoint> {
            members.into_iter().filter(|end| !matches!(end, WireEndpoint::Junction(_))).collect()
        };
        let (members_in, members_out) = (anchors(members_in), anchors(members_out));
        let name_at = |end: &WireEndpoint, parent_only: bool| {
            globals.get(end).filter(|(_, sheet)| !parent_only || *sheet == project.sheet_index).map(|(name, _)| name.as_str())
        };
        let names = |members: &[WireEndpoint], parent_only| -> BTreeSet<_> { members.iter().filter_map(|end| name_at(end, parent_only)).collect() };
        let inside_names = names(&members_in, false);
        // A branch can reach the selection through a cut wire or through named contacts
        // with no wire at all. A remote anchor cannot serve as the parent's wiring hub.
        let needs_parent_hub =
            cut_nets.contains(&index) || (!inside_names.is_empty() && members_out.iter().any(|end| on_sheet(project, project.sheet_index, *end)));
        let shared = inside_names.intersection(&names(&members_out, needs_parent_hub)).next().map(|name| (*name).to_owned());
        let listed = |members: &[WireEndpoint]| -> Vec<Value> { members.iter().map(|end| end.inspect()).collect() };
        if let Some(name) = shared {
            let anchor = |members: &[WireEndpoint], parent_only| members.iter().copied().find(|end| name_at(end, parent_only) == Some(name.as_str()));
            global_nets.push(GlobalNet {
                inside: listed(members_in.as_slice()),
                outside: listed(members_out.as_slice()),
                anchors: [anchor(&members_in, false), anchor(&members_out, needs_parent_hub)],
                members: [members_in, members_out],
                name,
            });
            continue;
        }
        // The authored name, not its project-qualified display form, names the port; the
        // generated number is used only for a net nobody named.
        let name = match graph.net_names(index).iter().min() {
            Some(name) => name.clone(),
            None => graph
                .borrowed_net_name(index)
                .ok_or_else(|| CommandError::Unavailable("A boundary net has neither a name nor a generated number.".into()))?
                .to_owned(),
        };
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
            direction: direction(&kinds(members_in.as_slice()), &kinds(members_out.as_slice())),
            inside: listed(members_in.as_slice()),
            outside: listed(members_out.as_slice()),
            net: index,
            members: [members_in, members_out],
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
        no_connects: travelling_markers,
        staying_wires,
        staying_no_connects: staying_markers,
        ports,
        global_nets,
        cut_wires,
        moved,
    })
}

/// Whether an addressed endpoint is drawn on the sheet at `index`.
fn on_sheet(project: &Project, index: usize, end: WireEndpoint) -> bool {
    let Some(sheet) = project.sheets.get(index) else {
        return false;
    };
    match end {
        WireEndpoint::Terminal(terminal) => sheet.symbols.iter().any(|symbol| symbol.ordinal == terminal.symbol),
        WireEndpoint::Junction(id) => sheet.junctions.contains_key(&id),
        WireEndpoint::NetLabel(id) => sheet.net_labels.iter().any(|label| label.id == id),
        WireEndpoint::Label(id) => sheet.labels.iter().any(|label| label.id == id),
        WireEndpoint::SheetPin(id) => sheet
            .subsheets
            .iter()
            .any(|block| project.sheets.iter().any(|child| child.sheet_id == block.child && child.labels.iter().any(|label| label.id == id))),
    }
}

/// Drop every cached graph after a structural edit that spans sheets.
fn forget_connectivity(project: &mut Project) {
    project.named_graph = Arc::default();
    for sheet in Arc::make_mut(&mut project.sheets) {
        sheet.connectivity = Arc::default();
    }
}

/// Whether two graphs group exactly the same terminals into nets, and every net keeps at
/// least the names it had. Names may only be added: a port adds its label name.
fn same_netlist(before: &Connectivity, after: &Connectivity) -> bool {
    let populated = |graph: &Connectivity| graph.nets.iter().filter(|net| !net.is_empty()).count();
    populated(before) == populated(after)
        && before.nets.iter().enumerate().all(|(index, net)| {
            let Some(first) = net.first() else {
                return true;
            };
            let Some(other) = after.net_index_for(*first) else {
                return false;
            };
            let members: BTreeSet<_> = net.iter().collect();
            after.nets.get(other).is_some_and(|other| other.iter().collect::<BTreeSet<_>>() == members)
                && before.net_names(index).iter().all(|name| after.net_names(other).contains(name))
        })
}

/// The members of `members` on the sheet at `index` that do not already share a net with
/// `hub`, one per net, so a single wire from the hub reaches each separate group.
fn unreached(project: &Project, graph: &Connectivity, index: usize, hub: WireEndpoint, members: &[WireEndpoint]) -> Vec<WireEndpoint> {
    let mut reached: BTreeSet<_> = graph.net_index_for_endpoint(hub).into_iter().collect();
    members
        .iter()
        .copied()
        .filter(|end| on_sheet(project, index, *end))
        .filter(|end| graph.net_index_for_endpoint(*end).is_none_or(|net| reached.insert(net)))
        .collect()
}

/// Apply a plan to the sheet projection it was built on and return the edited project.
/// Moved objects keep their identities and coordinates. Each port becomes a hierarchical
/// label on the child, placed where its first cut wire left the selection; the cut wires
/// of a port end on that label and the parent reaches the matching sheet pin by new wires.
/// Cut wires of global nets are removed, and each side rejoins through its global anchor.
fn split(project: &Project, plan: &Preview, anchor: Option<Point>, cursor: &mut u64) -> Result<Project, CommandError> {
    let moved = &plan.moved;
    let parent = project.sheet_index;
    let child_id = crate::fresh_id()?;
    let mut child = project.new_sheet_record(child_id, plan.name.clone())?;
    child.symbols = Arc::new(project.symbols.iter().filter(|symbol| moved.symbols.contains(&symbol.ordinal)).cloned().collect());
    child.junctions = Arc::new(project.junctions.iter().filter(|(id, _)| moved.junctions.contains(id)).map(|(id, at)| (*id, *at)).collect());
    child.net_labels = Arc::new(project.net_labels.iter().filter(|label| moved.net_labels.contains(&label.id)).cloned().collect());
    child.generated_legs = Arc::new(project.generated_legs.iter().filter(|leg| moved.wires.contains(&leg.wire)).cloned().collect());
    child.no_connect_ends = Arc::new(project.no_connect_ends.iter().filter(|id| moved.junctions.contains(id)).copied().collect());

    let mut labels = Vec::new();
    let mut slots = [0_u8; 2];
    for port in &plan.ports {
        let side = usize::from(port.direction.right());
        let slot = slots.get_mut(side).ok_or_else(|| CommandError::Unavailable("no free sheet pin slot".into()))?;
        let free = *slot;
        *slot = slot.checked_add(1).ok_or_else(|| CommandError::CapExceeded("no free sheet pin slot".into()))?;
        let at = plan
            .cut_wires
            .iter()
            .find(|cut| cut.net == Some(port.net))
            .and_then(|cut| project.endpoint_position(cut.outside_end))
            .or_else(|| {
                let [inside, _] = &port.members;
                let first = inside.iter().find_map(|end| project.endpoint_position(*end))?;
                Some(Point { x_nm: first.x_nm.saturating_sub(2 * GRID_NM), y_nm: first.y_nm })
            })
            .ok_or_else(|| CommandError::Unavailable(format!("port {} has no position on this sheet", port.name)))?;
        labels.push(hierarchy::Label { id: allocate(cursor)?, name: port.name.clone(), direction: port.direction, at, slot: free });
    }
    let by_net: BTreeMap<usize, &hierarchy::Label> = plan.ports.iter().map(|port| port.net).zip(&labels).collect();

    let cut: BTreeMap<u64, &CutWire> = plan.cut_wires.iter().map(|wire| (wire.ordinal, wire)).collect();
    let mut wires = Vec::new();
    for wire in project.wires.iter() {
        if moved.wires.contains(&wire.ordinal) {
            wires.push(wire.clone());
        } else if let Some(boundary) = cut.get(&wire.ordinal)
            && let Some(label) = boundary.net.and_then(|net| by_net.get(&net))
        {
            let mut copy = wire.clone();
            copy.endpoints = wire.endpoints.map(|end| if end == boundary.outside_end { WireEndpoint::Label(label.id) } else { end });
            if project.endpoint_position(boundary.outside_end) != Some(label.at) {
                copy.bends = None;
            }
            wires.push(copy);
        }
    }
    child.wires = Arc::new(wires);
    child.labels = Arc::new(labels);

    let at = anchor
        .or_else(|| {
            let corners = child.symbols.iter().map(|symbol| symbol.position).chain(child.net_labels.iter().map(|label| label.at));
            corners.reduce(|a, b| Point { x_nm: a.x_nm.min(b.x_nm), y_nm: a.y_nm.min(b.y_nm) })
        })
        .ok_or_else(|| CommandError::InvalidParams("the selection has nothing to place on a new sheet".into()))?;
    let block = SheetSymbol { id: allocate(cursor)?, child: child_id, at, size: None };

    let mut next = project.clone();
    let sheet = next.sheet_mut();
    Arc::make_mut(&mut sheet.symbols).retain(|symbol| !moved.symbols.contains(&symbol.ordinal));
    Arc::make_mut(&mut sheet.wires).retain(|wire| !moved.wires.contains(&wire.ordinal) && !cut.contains_key(&wire.ordinal));
    Arc::make_mut(&mut sheet.junctions).retain(|id, _| !moved.junctions.contains(id));
    Arc::make_mut(&mut sheet.net_labels).retain(|label| !moved.net_labels.contains(&label.id));
    Arc::make_mut(&mut sheet.generated_legs).retain(|leg| !moved.wires.contains(&leg.wire) && !cut.contains_key(&leg.wire));
    Arc::make_mut(&mut sheet.no_connect_ends).retain(|id| !moved.junctions.contains(id));
    Arc::make_mut(&mut sheet.subsheets).push(block);
    Arc::make_mut(&mut next.sheets).push(child);
    let inner = next.sheets.len().saturating_sub(1);
    forget_connectivity(&mut next);

    // Rejoin each boundary net on both sides through its hub: the new label and sheet pin
    // for a port, or the members carrying the shared name for a global net.
    let graph = next.electrical_connectivity();
    let mut joins = Vec::new();
    for (port, label) in plan.ports.iter().zip(next.sheets.get(inner).map(|sheet| sheet.labels.to_vec()).unwrap_or_default()) {
        let [inside, outside] = &port.members;
        for (index, hub, members) in [(inner, WireEndpoint::Label(label.id), inside), (parent, WireEndpoint::SheetPin(label.id), outside)] {
            joins.extend(unreached(&next, &graph, index, hub, members).into_iter().map(|end| (index, hub, end)));
        }
    }
    for global in &plan.global_nets {
        let [inside, outside] = &global.members;
        let [inside_anchor, outside_anchor] = global.anchors;
        for (index, hub, members) in [(inner, inside_anchor, inside), (parent, outside_anchor, outside)] {
            if let Some(hub) = hub.filter(|hub| on_sheet(&next, index, *hub)) {
                joins.extend(unreached(&next, &graph, index, hub, members).into_iter().map(|end| (index, hub, end)));
            }
        }
    }
    for (index, hub, end) in joins {
        next.sheet_index = index;
        let wire = Wire { ordinal: allocate(cursor)?, endpoints: [hub, end], bends: None };
        Arc::make_mut(&mut next.sheet_mut().wires).push(wire);
    }
    next.sheet_index = parent;
    next.prune_junctions();
    forget_connectivity(&mut next);

    if !same_netlist(project.borrowed_electrical_connectivity(), &next.electrical_connectivity()) {
        return Err(CommandError::InvalidParams(
            "Moving this selection would change the netlist; select whole connected groups or the markers they share.".into(),
        ));
    }
    next.validate_hierarchy().map_err(CommandError::InvalidParams)?;
    next.numeric_id_high_water = Some(next.allocation_high_water().max(cursor.saturating_sub(1)));
    crate::clipboard::validate_candidate(next)
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

    /// Move the selection onto a new child sheet exactly as [`PREVIEW`] describes it, as one
    /// undo step. Refusals leave the project, revision and history unchanged.
    pub(super) fn split_sheet(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        let target = project.resolve_sheet(params.get("sheet").and_then(Value::as_str))?;
        let view = project.for_sheet(target).ok_or_else(|| CommandError::NotFound("unknown sheet ID".into()))?;
        let name = check_new_sheet(&view, params)?;
        let items = Selection::parse(&params["items"]).ok_or_else(|| CommandError::InvalidParams("invalid selection".into()))?;
        items.validate(&view)?;
        items.validate_marker_owners(&view)?;
        let anchor = match (params.get("x_nm").and_then(Value::as_i64), params.get("y_nm").and_then(Value::as_i64)) {
            (Some(x_nm), Some(y_nm)) => Some(Point { x_nm, y_nm }),
            (None, None) => None,
            _ => return Err(CommandError::InvalidParams("`x_nm` and `y_nm` must be given together".into())),
        };
        let plan = plan(&view, &items, name)?;
        let mut cursor = self.next_object_id()?;
        let next = split(&view, &plan, anchor, &mut cursor)?;
        self.change(next)
    }
}
