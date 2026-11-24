//! Transient identity-relevant endpoint groups; never a persisted second connectivity graph.

use super::{Anchor, AnchorKind, Record, Scope, ScopeKind, State};
use crate::{CommandError, Project, SheetTarget, WireEndpoint};
use std::collections::{BTreeMap, BTreeSet};

/// A derived network's supported anchors and names; anonymous wire geometry is intentionally absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Network {
    /// Logical members used only as transient continuation evidence.
    pub anchors: BTreeSet<Anchor>,
    /// Authored names that make a singleton a meaningful electrical network.
    pub names: BTreeSet<String>,
}

impl Network {
    /// Bare terminals and one-pin unnamed stubs are not competing split/merge branches.
    /// Any name or a second anchor decides it without a scan; only a single anchor is inspected.
    pub fn meaningful(&self) -> bool {
        if !self.names.is_empty() || self.anchors.len() >= 2 {
            return true;
        }
        self.anchors.first().is_some_and(|a| a.kind != AnchorKind::Terminal)
    }

    /// Create one bounded identity using an explicit primary or canonical kind/key priority.
    /// The supplied primary must belong to this network; orphan geometry has no eligible key.
    pub fn record(&self, id: uuid::Uuid, selected: Option<&Anchor>) -> Result<Record, String> {
        let primary = selected.or_else(|| self.anchors.first()).ok_or("Network has no supported identity anchor.")?;
        if !self.anchors.contains(primary) || !self.meaningful() {
            return Err("A persistent net requires a meaningful network and a current anchor.".into());
        }
        Ok(Record {
            id,
            generated_number: None,
            primary: primary.clone(),
            fallbacks: self.anchors.iter().filter(|a| *a != primary).take(3).cloned().collect(),
            lineage: Vec::new(),
            class_bindings: Vec::new(),
            appearance: None,
            state: State::Active {},
        })
    }

    /// Preserve primary/fallback order while pruning lost/foreign-network keys. Refill only when
    /// no stored anchor survives and the unique continuation needs a canonical replacement.
    pub fn retained(&self, record: &Record) -> Result<Record, String> {
        let surviving: Vec<_> = record.anchors().filter(|anchor| self.anchors.contains(*anchor)).cloned().collect();
        if let Some(primary) = surviving.first() {
            Ok(Record { primary: primary.clone(), fallbacks: surviving.into_iter().skip(1).collect(), ..record.clone() })
        } else {
            let next = self.record(record.id, None)?;
            Ok(Record {
                generated_number: record.generated_number,
                lineage: record.lineage.clone(),
                class_bindings: record.class_bindings.clone(),
                appearance: record.appearance,
                ..next
            })
        }
    }
}

/// All electrical identity candidates, with a current command endpoint-to-anchor projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Graph {
    /// Groups ordered by their canonical first anchor, independent of traversal.
    pub networks: Vec<Network>,
    /// Current logical anchor membership; never serialized in the registry.
    pub by_anchor: BTreeMap<Anchor, usize>,
    /// Connected appearance and named-interface addresses used to verify exact net continuation.
    pub(super) endpoints: BTreeMap<(uuid::Uuid, WireEndpoint), Anchor>,
}

/// Construction keys distinguish actual graph groups from isolated logical terminals/interfaces.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Group {
    Connected(usize),
    Isolated(Anchor),
}

impl Project {
    /// Return the sole current electrical owner without allocating a fake schematic board.
    pub fn net_scope(&self) -> Scope {
        match self.board_target() {
            Some(board) => Scope { kind: ScopeKind::Board, id: board.board },
            None => Scope { kind: ScopeKind::Schematic, id: self.id },
        }
    }
}

/// Incremental builder for [`Graph`] so native Save validation yields between sheets,
/// symbols, labels, ports, sheet symbols and network anchor indexing. Phase 0 warms
/// `project.borrowed_electrical_connectivity()` without cloning it (building the snapshot's
/// `OnceLock` connectivity once if not yet cached) and indexes label/port endpoints to their
/// connectivity group; phase 3 sorts the derived networks vector in one step; all other phases
/// advance one sheet item or one network anchor entry per step. A new connected group starts
/// without names; the following steps copy its connectivity names one per step before any
/// other work.
#[derive(Default)]
pub(super) struct GraphBuilder {
    phase: u8,
    substep: u8,
    endpoint_groups: BTreeMap<WireEndpoint, usize>,
    groups: BTreeMap<Group, Network>,
    /// Connected groups whose connectivity names are still being copied: (net index, next name).
    pending_names: std::collections::VecDeque<(usize, usize)>,
    endpoints: BTreeMap<(uuid::Uuid, WireEndpoint), Anchor>,
    sheet_index: usize,
    current_occurrence: Option<uuid::Uuid>,
    item_index: usize,
    networks: Vec<Network>,
    current_anchor: Option<Anchor>,
    by_anchor: BTreeMap<Anchor, usize>,
}

impl GraphBuilder {
    fn add_anchor(
        &mut self,
        graph: &crate::schematic::Connectivity,
        sheet_id: uuid::Uuid,
        endpoint: WireEndpoint,
        anchor: Anchor,
        name: Option<&str>,
    ) -> Result<(), String> {
        anchor.validate_form()?;
        let index = match endpoint {
            WireEndpoint::Terminal(pin) => graph.net_index_for(pin),
            WireEndpoint::NetLabel(_) | WireEndpoint::Label(_) => self.endpoint_groups.get(&endpoint).copied(),
            _ => graph.net_index_for_endpoint(endpoint),
        };
        let group = index.map_or_else(|| Group::Isolated(anchor.clone()), Group::Connected);
        let network = match self.groups.entry(group) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => {
                if let Some(index) = index
                    && !graph.net_names(index).is_empty()
                {
                    self.pending_names.push_back((index, 0));
                }
                entry.insert(Network { anchors: BTreeSet::new(), names: BTreeSet::new() })
            }
        };
        network.anchors.insert(anchor.clone());
        if let Some(name) = name {
            network.names.insert(name.to_owned());
        }
        self.endpoints.insert((sheet_id, endpoint), anchor);
        Ok(())
    }

    /// Advance one graph-construction step, returning `Some(graph)` when complete.
    pub(super) fn step(&mut self, project: &Project) -> Result<Option<Graph>, String> {
        if let Some(&(index, next)) = self.pending_names.front() {
            let names = project.borrowed_electrical_connectivity().net_names(index);
            if let (Some(name), Some(network)) = (names.get(next), self.groups.get_mut(&Group::Connected(index))) {
                network.names.insert(name.clone());
            }
            if next + 1 < names.len() {
                self.pending_names[0].1 = next + 1;
            } else {
                self.pending_names.pop_front();
            }
            return Ok(None);
        }
        match self.phase {
            0 => {
                let graph = project.borrowed_electrical_connectivity();
                self.endpoint_groups.clear();
                for index in 0..graph.nets.len() {
                    for endpoint in graph.endpoints(index) {
                        if matches!(endpoint, WireEndpoint::NetLabel(_) | WireEndpoint::Label(_)) {
                            self.endpoint_groups.entry(endpoint).or_insert(index);
                        }
                    }
                }
                self.phase = 1;
            }
            1 => {
                let Some(sheet) = project.sheets.get(self.sheet_index) else {
                    self.sheet_index = 0;
                    self.item_index = 0;
                    self.current_occurrence = None;
                    self.endpoint_groups.clear();
                    self.phase = 2;
                    return Ok(None);
                };
                let occurrence = match self.current_occurrence {
                    Some(id) => id,
                    None => {
                        let target = SheetTarget { project: project.id, sheet: sheet.sheet_id };
                        let id = project.sheet_occurrence(target).ok_or("Missing occurrence while resolving net anchors.")?.id;
                        self.current_occurrence = Some(id);
                        id
                    }
                };
                let graph = project.borrowed_electrical_connectivity();
                let scope = project.net_scope();
                let sheet_id = sheet.sheet_id;
                match self.substep {
                    0 => {
                        if let Some(symbol) = sheet.symbols.get(self.item_index) {
                            for terminal in symbol.terminals() {
                                // Bare pins are not networks. Avoid allocating persistent-address groups for
                                // every unused pin on large devices; a connected appearance includes aliases.
                                if graph.net_index_for(terminal).is_none() {
                                    continue;
                                }
                                let anchor = Anchor {
                                    kind: AnchorKind::Terminal,
                                    project: project.id,
                                    scope,
                                    occurrence: uuid::Uuid::nil(),
                                    object: symbol.component,
                                    terminal: terminal.number().into(),
                                };
                                self.add_anchor(graph, sheet_id, WireEndpoint::Terminal(terminal), anchor, None)?;
                            }
                            self.item_index += 1;
                        } else {
                            self.item_index = 0;
                            self.substep = 1;
                        }
                    }
                    1 => {
                        if let Some(label) = sheet.net_labels.get(self.item_index) {
                            let anchor =
                                Anchor { kind: AnchorKind::Label, project: project.id, scope, occurrence, object: label.id, terminal: String::new() };
                            self.add_anchor(graph, sheet_id, WireEndpoint::NetLabel(label.id), anchor, Some(&label.name))?;
                            self.item_index += 1;
                        } else {
                            self.item_index = 0;
                            self.substep = 2;
                        }
                    }
                    2 => {
                        if let Some(port) = sheet.labels.get(self.item_index) {
                            let anchor =
                                Anchor { kind: AnchorKind::Port, project: project.id, scope, occurrence, object: port.id, terminal: String::new() };
                            self.add_anchor(graph, sheet_id, WireEndpoint::Label(port.id), anchor, Some(&port.name))?;
                            self.item_index += 1;
                        } else {
                            self.item_index = 0;
                            self.substep = 0;
                            self.current_occurrence = None;
                            self.sheet_index += 1;
                        }
                    }
                    _ => return Err("invalid net graph build substep".into()),
                }
            }
            2 => {
                // A parent pin and its child port have one sheet-owned logical identity.
                let Some(sheet) = project.sheets.get(self.sheet_index) else {
                    self.sheet_index = 0;
                    self.item_index = 0;
                    self.phase = 3;
                    return Ok(None);
                };
                if let Some(block) = sheet.subsheets.get(self.item_index) {
                    for pin in project.sheet_pins(block) {
                        let anchor = self.endpoints.get(&(block.child, WireEndpoint::Label(pin.label.id))).cloned().ok_or("Missing port anchor.")?;
                        self.endpoints.insert((sheet.sheet_id, WireEndpoint::SheetPin(pin.label.id)), anchor);
                    }
                    self.item_index += 1;
                } else {
                    self.item_index = 0;
                    self.sheet_index += 1;
                }
            }
            3 => {
                let mut networks: Vec<_> = std::mem::take(&mut self.groups).into_values().collect();
                networks.sort_by(|a, b| a.anchors.first().cmp(&b.anchors.first()));
                self.networks = networks;
                self.item_index = 0;
                self.current_anchor = None;
                self.phase = 4;
            }
            4 => {
                if let Some(network) = self.networks.get(self.item_index) {
                    let next_anchor = match &self.current_anchor {
                        None => network.anchors.first(),
                        Some(prev) => network.anchors.range((std::ops::Bound::Excluded(prev), std::ops::Bound::Unbounded)).next(),
                    };
                    if let Some(anchor) = next_anchor {
                        if self.by_anchor.insert(anchor.clone(), self.item_index).is_some() {
                            return Err("A physical-terminal identity appears in different electrical networks.".into());
                        }
                        if network.anchors.last() == Some(anchor) {
                            self.current_anchor = None;
                            self.item_index += 1;
                        } else {
                            self.current_anchor = Some(anchor.clone());
                        }
                    } else {
                        self.current_anchor = None;
                        self.item_index += 1;
                    }
                } else {
                    let graph = Graph {
                        networks: std::mem::take(&mut self.networks),
                        by_anchor: std::mem::take(&mut self.by_anchor),
                        endpoints: std::mem::take(&mut self.endpoints),
                    };
                    *self = Self::default();
                    return Ok(Some(graph));
                }
            }
            _ => return Err("invalid net graph build phase".into()),
        }
        Ok(None)
    }
}

impl Graph {
    /// Build from the existing complete project graph, deduplicating repeated physical-terminal
    /// appearances and including standalone authored labels/ports. Does not allocate net IDs.
    pub fn build(project: &Project) -> Result<Self, String> {
        let mut builder = GraphBuilder::default();
        loop {
            if let Some(graph) = builder.step(project)? {
                return Ok(graph);
            }
        }
    }

    /// Resolve a validated terminal/label/port command target in its explicit sheet context.
    /// Junctions and foreign/missing endpoints are refused rather than chosen by display order.
    pub fn target(&self, sheet: SheetTarget, value: &serde_json::Value) -> Result<Anchor, CommandError> {
        if let Ok(anchor) = serde_json::from_value::<Anchor>(value.clone()) {
            anchor.validate_form().map_err(CommandError::InvalidParams)?;
            if self.endpoints.iter().any(|((id, _), current)| *id == sheet.id() && current == &anchor) {
                return Ok(anchor);
            }
            return Err(CommandError::NotFound("Unknown or foreign typed net anchor in the addressed sheet.".into()));
        }
        let endpoint = WireEndpoint::parse(value)
            .filter(|end| !matches!(end, WireEndpoint::Junction(_)))
            .ok_or_else(|| CommandError::InvalidParams("Net targets require a terminal, net label or hierarchical port.".into()))?;
        self.endpoints
            .get(&(sheet.id(), endpoint))
            .cloned()
            .ok_or_else(|| CommandError::NotFound("Unknown or foreign net anchor in the addressed sheet.".into()))
    }

    /// Find the current network for a typed anchor without inferring one from names or geometry.
    pub fn network(&self, anchor: &Anchor) -> Option<usize> {
        self.by_anchor.get(anchor).copied()
    }

    /// Read exact current endpoint identity for selection without accepting junctions as saved
    /// registry anchors. A junction may instead resolve through its current electrical group.
    pub(super) fn endpoint_anchor(&self, sheet: SheetTarget, endpoint: WireEndpoint) -> Option<&Anchor> {
        self.endpoints.get(&(sheet.id(), endpoint))
    }

    /// Requalify transient keys for the first board's whole-scope transfer, preserving memberships.
    pub fn requalify(&mut self, scope: Scope) {
        for network in &mut self.networks {
            network.anchors = std::mem::take(&mut network.anchors)
                .into_iter()
                .map(|mut anchor| {
                    anchor.scope = scope;
                    anchor
                })
                .collect();
        }
        for anchor in self.endpoints.values_mut() {
            anchor.scope = scope;
        }
        self.by_anchor = self
            .networks
            .iter()
            .enumerate()
            .flat_map(|(index, network)| network.anchors.iter().cloned().map(move |anchor| (anchor, index)))
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CONNECT_PINS, DRAW_WIRE, PLACE_RESISTOR, Session};
    use serde_json::{Value, json};

    fn apply(session: &mut Session, command: &str, mut params: Value) {
        params["expected_revision"] = json!(session.revision());
        session.apply(command, &params).unwrap();
    }

    /// `GraphBuilder` indexes label/port groups once, copies connected group names after creation,
    /// preserves isolated labels, and yields once per anchor during phase 4 indexing.
    #[test]
    fn graph_builder_yields_per_anchor_and_preserves_connected_and_isolated_names() {
        let mut session = Session::default();
        session.apply("project.new", &json!({"create_board": false})).unwrap();
        let mut pins = Vec::new();
        for index in 0..3 {
            apply(&mut session, PLACE_RESISTOR, json!({"x_nm": index as i64 * 25_400_000, "y_nm": 0}));
            pins.push(json!({"symbol": session.project().unwrap().symbols().last().unwrap().id(), "terminal": "1"}));
        }
        apply(&mut session, CONNECT_PINS, json!({"from": pins[0], "to": pins[1]}));
        apply(&mut session, CONNECT_PINS, json!({"from": pins[1], "to": pins[2]}));
        let pin_pos = |session: &Session, idx: usize| {
            let project = session.project().unwrap();
            let terminal = project.symbols()[idx].terminals()[0];
            project.terminal_position(terminal).unwrap()
        };
        let p0 = pin_pos(&session, 0);
        let p2 = pin_pos(&session, 2);
        apply(&mut session, crate::net_labels::ADD, json!({"name": "SIG_A", "style": "local", "x_nm": p0.x_nm, "y_nm": p0.y_nm + 25_400_000}));
        apply(&mut session, crate::net_labels::ADD, json!({"name": "SIG_B", "style": "local", "x_nm": p2.x_nm, "y_nm": p2.y_nm + 25_400_000}));
        apply(
            &mut session,
            crate::net_labels::ADD,
            json!({"name": "ISOLATED", "style": "local", "x_nm": p2.x_nm + 25_400_000, "y_nm": p2.y_nm + 25_400_000}),
        );
        let labels: Vec<u64> = session.project().unwrap().net_labels().iter().map(|label| label.id).collect();
        assert_eq!(labels.len(), 3);
        apply(&mut session, DRAW_WIRE, json!({"from": {"net_label": format!("net-label-{}", labels[0])}, "to": pins[0], "bends": []}));
        apply(&mut session, DRAW_WIRE, json!({"from": {"net_label": format!("net-label-{}", labels[1])}, "to": pins[2], "bends": []}));

        let project = session.project().unwrap();
        let mut builder = GraphBuilder::default();
        let mut phase4_steps = 0usize;
        let graph = loop {
            let in_phase4 = builder.phase == 4;
            let done = builder.step(project).unwrap();
            if in_phase4 {
                phase4_steps += 1;
            }
            if let Some(graph) = done {
                break graph;
            }
        };

        let total_anchors: usize = graph.networks.iter().map(|network| network.anchors.len()).sum();
        assert!(total_anchors >= 6);
        assert!(phase4_steps > graph.networks.len());
        assert_eq!(phase4_steps, total_anchors + 1);

        let root_sheet = project.sheets().next().unwrap().target;
        let label_a = graph.endpoint_anchor(root_sheet, WireEndpoint::NetLabel(labels[0])).unwrap();
        let label_b = graph.endpoint_anchor(root_sheet, WireEndpoint::NetLabel(labels[1])).unwrap();
        let isolated = graph.endpoint_anchor(root_sheet, WireEndpoint::NetLabel(labels[2])).unwrap();
        let connected_idx = graph.network(label_a).unwrap();
        assert_eq!(graph.network(label_b), Some(connected_idx));
        let connected = &graph.networks[connected_idx];
        assert_eq!(connected.anchors.len(), 5);
        assert!(connected.names.contains("SIG_A"));
        assert!(connected.names.contains("SIG_B"));

        let isolated_idx = graph.network(isolated).unwrap();
        assert_ne!(isolated_idx, connected_idx);
        assert_eq!(graph.networks[isolated_idx].anchors.len(), 1);
        assert!(graph.networks[isolated_idx].names.contains("ISOLATED"));

        let built = Graph::build(project).unwrap();
        assert_eq!(graph.networks, built.networks);
        assert_eq!(graph.by_anchor, built.by_anchor);
        assert_eq!(graph.endpoints, built.endpoints);
    }

    /// A new connected group copies its connectivity names one per step: no step adds more
    /// than one name, and the built graph keeps every name.
    #[test]
    fn graph_builder_copies_connected_names_one_per_step() {
        let mut session = Session::default();
        session.apply("project.new", &json!({"create_board": false})).unwrap();
        let mut pins = Vec::new();
        for index in 0..3 {
            apply(&mut session, PLACE_RESISTOR, json!({"x_nm": index as i64 * 25_400_000, "y_nm": 0}));
            pins.push(json!({"symbol": session.project().unwrap().symbols().last().unwrap().id(), "terminal": "1"}));
        }
        apply(&mut session, CONNECT_PINS, json!({"from": pins[0], "to": pins[1]}));
        apply(&mut session, CONNECT_PINS, json!({"from": pins[1], "to": pins[2]}));
        for (index, name) in ["SIG_A", "SIG_B", "SIG_C"].into_iter().enumerate() {
            let project = session.project().unwrap();
            let at = project.terminal_position(project.symbols()[index].terminals()[0]).unwrap();
            apply(&mut session, crate::net_labels::ADD, json!({"name": name, "style": "local", "x_nm": at.x_nm, "y_nm": at.y_nm + 25_400_000}));
            let label = session.project().unwrap().net_labels().last().unwrap().id;
            apply(&mut session, DRAW_WIRE, json!({"from": {"net_label": format!("net-label-{label}")}, "to": pins[index], "bends": []}));
        }

        let project = session.project().unwrap();
        let names = |builder: &GraphBuilder| builder.groups.values().map(|network| network.names.len()).sum::<usize>();
        let mut builder = GraphBuilder::default();
        let mut copy_steps = 0usize;
        let graph = loop {
            let before = names(&builder);
            let copying = !builder.pending_names.is_empty();
            let done = builder.step(project).unwrap();
            if let Some(graph) = done {
                break graph;
            }
            assert!(names(&builder) <= before + 1, "one builder step added more than one net name");
            copy_steps += usize::from(copying);
        };
        assert!(copy_steps >= 3);

        let root = project.sheets().next().unwrap().target;
        let label = project.net_labels()[0].id;
        let index = graph.network(graph.endpoint_anchor(root, WireEndpoint::NetLabel(label)).unwrap()).unwrap();
        let expected: BTreeSet<String> = ["SIG_A", "SIG_B", "SIG_C"].into_iter().map(String::from).collect();
        assert_eq!(graph.networks[index].names, expected);
        assert_eq!(graph.networks, Graph::build(project).unwrap().networks);
    }

    /// Names or a second anchor make a network meaningful; a lone terminal does not.
    #[test]
    fn meaningful_network_returns_early_and_keeps_single_anchor_rule() {
        let mut session = Session::default();
        session.apply("project.new", &json!({"create_board": false})).unwrap();
        let project = session.project().unwrap();
        let anchor = |kind: AnchorKind, object: u64| Anchor {
            kind,
            project: project.id,
            scope: project.net_scope(),
            occurrence: uuid::Uuid::nil(),
            object,
            terminal: if kind == AnchorKind::Terminal { "1".into() } else { String::new() },
        };
        let network = |anchors: Vec<Anchor>, names: &[&str]| Network {
            anchors: anchors.into_iter().collect(),
            names: names.iter().map(|name| (*name).to_owned()).collect(),
        };
        assert!(!network(vec![], &[]).meaningful());
        assert!(!network(vec![anchor(AnchorKind::Terminal, 1)], &[]).meaningful());
        assert!(network(vec![anchor(AnchorKind::Label, 1)], &[]).meaningful());
        assert!(network(vec![anchor(AnchorKind::Port, 1)], &[]).meaningful());
        assert!(network(vec![anchor(AnchorKind::Terminal, 1)], &["VCC"]).meaningful());
        assert!(network(vec![anchor(AnchorKind::Terminal, 1), anchor(AnchorKind::Terminal, 2)], &[]).meaningful());
        assert!(network(vec![anchor(AnchorKind::Terminal, 1), anchor(AnchorKind::Label, 2)], &[]).meaningful());
    }

    /// Phase 2 resolves parent sheet pins directly via `block.child` and retains the missing-anchor error.
    #[test]
    fn graph_builder_phase2_resolves_sheet_pin_and_rejects_missing_port_anchor() {
        let mut session = Session::default();
        session.apply("project.new", &json!({"create_board": false})).unwrap();
        apply(&mut session, crate::hierarchy::CREATE_CHILD, json!({"name": "Child", "x_nm": 0, "y_nm": 0}));
        let root = session.project().unwrap().sheets().next().unwrap().target;
        let child = session.project().unwrap().sheets().nth(1).unwrap().target;
        apply(&mut session, crate::hierarchy::ADD_LABEL, json!({"sheet": child.id(), "name": "IN", "direction": "input", "x_nm": 0, "y_nm": 0}));

        let project = session.project().unwrap();
        let label_id = project.sheets[1].labels[0].id;
        let graph = Graph::build(project).unwrap();
        let port_anchor = graph.endpoint_anchor(child, WireEndpoint::Label(label_id)).unwrap();
        let pin_anchor = graph.endpoint_anchor(root, WireEndpoint::SheetPin(label_id)).unwrap();
        assert_eq!(pin_anchor, port_anchor);

        let mut builder = GraphBuilder::default();
        while builder.phase < 2 {
            assert!(builder.step(project).unwrap().is_none());
        }
        assert!(builder.endpoints.remove(&(child.id(), WireEndpoint::Label(label_id))).is_some());
        let error = builder.step(project).unwrap_err();
        assert_eq!(error, "Missing port anchor.");
    }
}
