//! A board's netlist: the components, pads and connections the board has accepted through
//! reviewed updates. Unapplied connectivity changes are reported as findings; reference and
//! value follow the live component. Comparing with the schematic netlist exposes pending
//! connection changes, apart from the board-only nets joining pads of one unwired terminal.

use super::{
    Basis, BoardRef, Component, ComponentKind, DefinitionRef, Export, FORMAT_V2, Net, Netlist, PackageRef, ProjectRef, TerminalInfo, TerminalRef,
};
use crate::{
    CommandError, Session,
    board::{
        fabrication::{Output, file_part},
        update::{Baseline, NetOrigin, TerminalKey, capture_source, preview},
        view::BoardView,
    },
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// Generate the addressed board's netlist file; edits nothing.
pub const EXPORT_BOARD_NETLIST: &str = "pcb.export_netlist";

impl BoardView<'_> {
    /// Build the netlist this board has accepted. A board with no placements and no accepted
    /// update has an empty accepted state. Fails when placements exist without an accepted
    /// baseline, whose accepted state is unknown, or when the net registry does not resolve.
    ///
    /// A net keeps the persistent ID of the current schematic net with exactly the same board
    /// terminals; a net the schematic has since changed has none and is reported. Reference
    /// and value follow the live component, as on the board itself. Role and terminal metadata
    /// require a live definition matching the accepted library, symbol, revision and terminal
    /// table. A matching terminal uses its effective function; without that match the role is
    /// unknown and terminal names and types are null.
    pub fn netlist(self) -> Result<Export, String> {
        let project = self.project();
        let accepted = match &project.board.baseline {
            Some(baseline) => Arc::clone(baseline),
            None if project.board.footprints().is_empty() => {
                let source = capture_source(project).map_err(|error| error.to_string())?;
                Arc::new(Baseline::empty(self.target.project, self.target.board, source)?)
            }
            None => return Err("Initialize the board's accepted baseline before exporting its netlist; its accepted state is unknown.".into()),
        };
        let mut findings = Vec::new();
        let placed: BTreeMap<_, _> = project.board.footprints().iter().map(|footprint| (footprint.component, footprint.id)).collect();
        let mut order = BTreeMap::new();
        let mut components = Vec::new();
        for (&id, state) in &accepted.components {
            let live = project.components.get(&id);
            let mut native: Vec<_> = live.map(|part| part.definition.pins().iter().map(|pin| pin.number.as_str()).collect()).unwrap_or_default();
            native.sort_unstable();
            // The live table describes these terminals only while it is the accepted symbol revision.
            let current = live.filter(|part| {
                (part.definition.library.as_str(), part.definition.id.as_str(), part.definition.revision())
                    == (state.library.as_str(), state.definition.as_str(), state.revision)
                    && native == state.terminals
            });
            let pads = |terminal: &str| {
                Some(state.package.as_ref().and_then(|package| package.mapping.get(terminal)).map_or_else(Vec::new, |pads| pads.pads().to_vec()))
            };
            let terminals: Vec<_> = match current {
                Some(part) => part
                    .definition
                    .pins()
                    .iter()
                    .map(|pin| {
                        let (name, electrical_type) = part.effective_pin(pin);
                        TerminalInfo {
                            terminal: pin.number.clone(),
                            name: name.map(str::to_owned),
                            electrical_type: Some(electrical_type),
                            pads: pads(&pin.number),
                        }
                    })
                    .collect(),
                None => state
                    .terminals
                    .iter()
                    .map(|terminal| TerminalInfo { terminal: terminal.clone(), name: None, electrical_type: None, pads: pads(terminal) })
                    .collect(),
            };
            for (index, terminal) in terminals.iter().enumerate() {
                order.insert((id, terminal.terminal.clone()), (id, index));
            }
            if !project.component_is_board_eligible(id) {
                findings.push(json!({"kind":"pending_removal","part":format!("component-{id}"),
                    "message":format!("{} was deleted or excluded from the board in the schematic; the board keeps it until the next update.", state.reference)}));
            }
            components.push(Component {
                id: format!("component-{id}"),
                kind: match current {
                    Some(part) if part.definition.power_terminal().is_some() => ComponentKind::Power,
                    Some(_) => ComponentKind::Part,
                    None => ComponentKind::Unknown,
                },
                reference: live.map_or(state.reference.as_str(), |part| part.reference()).to_owned(),
                value: live.map_or(state.value.as_str(), |part| part.value()).to_owned(),
                definition: DefinitionRef { library: state.library.clone(), id: state.definition.clone(), revision: state.revision },
                units: Vec::new(),
                terminals,
                package: state.package.as_ref().map(|package| PackageRef {
                    library: package.library.clone(),
                    id: package.definition.clone(),
                    revision: package.revision,
                    footprint: placed.get(&id).map(|footprint| format!("footprint-{footprint}")),
                    unconnected_pads: package.unconnected_pads.clone(),
                }),
            });
        }
        let key =
            |terminal: &TerminalKey| order.get(&(terminal.component, terminal.terminal.clone())).copied().unwrap_or((terminal.component, usize::MAX));
        let reference =
            |terminal: &TerminalKey| TerminalRef { part: format!("component-{}", terminal.component), terminal: terminal.terminal.clone() };

        // Current schematic nets by their board terminals, the same membership a board update compares.
        let electrical = project.electrical_connectivity();
        let ids = project.persistent_net_ids()?;
        let owners: BTreeMap<_, _> = project.all_symbols().map(|symbol| (symbol.ordinal, symbol.component)).collect();
        let current: BTreeMap<Vec<TerminalKey>, usize> = electrical
            .nets
            .iter()
            .enumerate()
            .filter_map(|(index, group)| {
                let mut members: Vec<_> = group
                    .iter()
                    .filter_map(|terminal| {
                        let component = *owners.get(&terminal.symbol)?;
                        project.component_is_board_eligible(component).then(|| TerminalKey { component, terminal: terminal.number().to_owned() })
                    })
                    .collect();
                members.sort();
                members.dedup();
                (!members.is_empty()).then_some((members, index))
            })
            .collect();
        let mut nets = Vec::new();
        for (index, net) in accepted.nets.iter().enumerate() {
            let name = accepted.net_name(index).ok_or("accepted net index out of range")?;
            let id = match net.accepted_origin() {
                Some(NetOrigin::PadOnly) => {
                    findings.push(json!({"kind":"pad_net","net":name,
                        "message":format!("At the accepted update, net {name} joined only pads of one unwired terminal; its id is null.")}));
                    None
                }
                None => {
                    findings.push(json!({"kind":"net_origin_unknown","net":name,
                        "message":format!("The accepted baseline does not record whether net {name} came from the schematic; its id is null.")}));
                    None
                }
                Some(NetOrigin::Schematic) => match current.get(&net.terminals).copied() {
                    None => {
                        findings.push(json!({"kind":"pending_net","net":name,
                            "message":format!("Net {name} differs from the schematic until the board is updated; its id is null.")}));
                        None
                    }
                    Some(row) => {
                        let id = ids.get(row).copied().flatten();
                        if id.is_none() {
                            findings.push(json!({"kind":"unregistered_net","net":name,
                                "message":format!("Net {name} has no persistent ID; its id is null.")}));
                        }
                        id
                    }
                },
            };
            let mut terminals = net.terminals.clone();
            terminals.sort_by_key(key);
            let Some(first) = terminals.first().map(key) else { continue };
            nets.push((first, Net { id, name, names: net.names.clone(), terminals: terminals.iter().map(reference).collect() }));
        }
        nets.sort_by_key(|(first, _)| *first);
        let joined: BTreeSet<_> = accepted.nets.iter().flat_map(|net| &net.terminals).collect();
        let mut unconnected: Vec<_> = accepted
            .components
            .iter()
            .flat_map(|(&component, state)| state.terminals.iter().map(move |terminal| TerminalKey { component, terminal: terminal.clone() }))
            .filter(|terminal| !joined.contains(terminal))
            .collect();
        unconnected.sort_by_key(key);
        findings.extend(pending_update(project, &accepted));
        let netlist = Netlist {
            format: FORMAT_V2.to_owned(),
            project: ProjectRef { id: project.id, name: project.name.clone() },
            basis: Basis::Accepted,
            board: Some(BoardRef {
                id: self.target.board,
                name: self.name().to_owned(),
                source: project.board.baseline.as_ref().map(|b| b.source.clone()),
            }),
            components,
            nets: nets.into_iter().map(|(_, net)| net).collect(),
            unconnected: unconnected.iter().map(reference).collect(),
        };
        Ok(Export { netlist, findings })
    }
}

/// One finding when the schematic differs from what the board accepted, counted the way
/// `pcb.preview_update` reports it, or when that comparison cannot be made.
fn pending_update(project: &crate::Project, accepted: &Baseline) -> Option<Value> {
    let changes =
        Baseline::proposed(project, &Baseline::current_choices(project), accepted.source.clone()).and_then(|proposed| preview(accepted, &proposed));
    match changes {
        Ok(changes) if changes.is_noop() => None,
        Ok(changes) => Some(json!({"kind":"pending_update","component_changes":changes.components.len(),
            "removed_nets":changes.removed_nets.len(),"added_nets":changes.added_nets.len(),"blockers":changes.blockers.len(),
            "message":"The schematic has changes the board has not accepted; pcb.preview_update lists them."})),
        Err(error) => Some(json!({"kind":"update_unavailable",
            "message":format!("The schematic could not be compared with the board's accepted state: {error}")})),
    }
}

impl Session {
    /// Resolve the file name and project the accepted rows and findings without rendering JSON.
    fn prepare_board_netlist(&self, params: &Value) -> Result<(String, Export), CommandError> {
        let project = self.open_project()?;
        let target = project.resolve_board(params.get("board").and_then(Value::as_str))?;
        let view = project.board_view(target).ok_or_else(|| CommandError::Unavailable("board unavailable".into()))?;
        let export = view.netlist().map_err(CommandError::Unavailable)?;
        let name = format!("{}-{}-netlist.json", file_part(&project.name), file_part(view.name()));
        Ok((name, export))
    }

    /// Build the addressed board's netlist file without editing anything. Fails when no
    /// project is open, the board is unknown, or [`BoardView::netlist`] fails.
    pub fn export_board_netlist(&self, params: &Value) -> Result<(Output, Vec<Value>), CommandError> {
        let (name, Export { netlist, findings }) = self.prepare_board_netlist(params)?;
        let contents = netlist.to_text().map_err(CommandError::Unavailable)?;
        Ok((Output { name, contents }, findings))
    }

    /// A dry run still projects findings but does not render the file text.
    pub(crate) fn board_netlist_report(&self, params: &Value) -> Result<Value, CommandError> {
        if params["dry_run"].as_bool() == Some(true) {
            let (name, export) = self.prepare_board_netlist(params)?;
            return Ok(json!({"files":[{"name":name}],"findings":export.findings}));
        }
        let (file, findings) = self.export_board_netlist(params)?;
        Ok(json!({"files":[file],"findings":findings}))
    }
}

#[cfg(test)]
mod tests;
