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
    pub fn meaningful(&self) -> bool {
        !self.names.is_empty()
            || self.anchors.iter().any(|a| a.kind != AnchorKind::Terminal)
            || self.anchors.iter().filter(|a| a.kind == AnchorKind::Terminal).count() >= 2
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
    endpoints: BTreeMap<(uuid::Uuid, WireEndpoint), Anchor>,
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

/// Incremental builder for [`Graph`] so native Save validation can yield between sheets,
/// symbols, labels, sheet symbols and network indexing.
#[derive(Default)]
pub(super) struct GraphBuilder {
    phase: u8,
    substep: u8,
    connectivity: Option<crate::schematic::Connectivity>,
    groups: BTreeMap<Group, Network>,
    endpoints: BTreeMap<(uuid::Uuid, WireEndpoint), Anchor>,
    sheet_index: usize,
    current_occurrence: Option<uuid::Uuid>,
    item_index: usize,
    networks: Vec<Network>,
    by_anchor: BTreeMap<Anchor, usize>,
}

impl GraphBuilder {
    fn add_anchor(&mut self, sheet_id: uuid::Uuid, endpoint: WireEndpoint, anchor: Anchor, name: Option<&str>) -> Result<(), String> {
        anchor.validate_form()?;
        let graph = self.connectivity.as_ref().ok_or("missing electrical connectivity during net graph build")?;
        let index = graph.net_index_for_endpoint(endpoint);
        let group = index.map_or_else(|| Group::Isolated(anchor.clone()), Group::Connected);
        let network = self.groups.entry(group).or_insert_with(|| Network { anchors: BTreeSet::new(), names: BTreeSet::new() });
        network.anchors.insert(anchor.clone());
        if let Some(index) = index {
            network.names.extend(graph.net_names(index).iter().cloned());
        }
        if let Some(name) = name {
            network.names.insert(name.to_owned());
        }
        self.endpoints.insert((sheet_id, endpoint), anchor);
        Ok(())
    }

    /// Advance one bounded graph-construction unit, returning `Some(graph)` when complete.
    pub(super) fn step(&mut self, project: &Project) -> Result<Option<Graph>, String> {
        match self.phase {
            0 => {
                self.connectivity = Some(project.electrical_connectivity());
                self.phase = 1;
            }
            1 => {
                let Some(sheet) = project.sheets.get(self.sheet_index) else {
                    self.sheet_index = 0;
                    self.item_index = 0;
                    self.current_occurrence = None;
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
                let scope = project.net_scope();
                let sheet_id = sheet.sheet_id;
                match self.substep {
                    0 => {
                        if let Some(symbol) = sheet.symbols.get(self.item_index) {
                            for terminal in symbol.terminals() {
                                // Bare pins are not networks. Avoid allocating persistent-address groups for
                                // every unused pin on large devices; a connected appearance includes aliases.
                                if self.connectivity.as_ref().is_some_and(|graph| graph.net_index_for(terminal).is_none()) {
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
                                self.add_anchor(sheet_id, WireEndpoint::Terminal(terminal), anchor, None)?;
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
                            self.add_anchor(sheet_id, WireEndpoint::NetLabel(label.id), anchor, Some(&label.name))?;
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
                            self.add_anchor(sheet_id, WireEndpoint::Label(port.id), anchor, Some(&port.name))?;
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
                    self.connectivity = None;
                    self.phase = 3;
                    return Ok(None);
                };
                if let Some(block) = sheet.subsheets.get(self.item_index) {
                    for pin in project.sheet_pins(block) {
                        let child = project
                            .sheets
                            .iter()
                            .find(|candidate| candidate.labels.iter().any(|label| label.id == pin.label.id))
                            .ok_or("Missing child port.")?;
                        let anchor =
                            self.endpoints.get(&(child.sheet_id, WireEndpoint::Label(pin.label.id))).cloned().ok_or("Missing port anchor.")?;
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
                self.phase = 4;
            }
            4 => {
                if let Some(network) = self.networks.get(self.item_index) {
                    for anchor in &network.anchors {
                        if self.by_anchor.insert(anchor.clone(), self.item_index).is_some() {
                            return Err("A physical-terminal identity appears in different electrical networks.".into());
                        }
                    }
                    self.item_index += 1;
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
