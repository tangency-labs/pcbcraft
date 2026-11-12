//! Transient identity-relevant endpoint groups; never a persisted second connectivity graph.

use super::{Anchor, AnchorKind, Record, Scope, ScopeKind, State};
use crate::{CommandError, Project, SheetTarget, WireEndpoint};
use std::collections::{BTreeMap, BTreeSet};

/// A derived network's supported anchors and names; anonymous wire geometry is intentionally absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Network {
    /// Logical members used only as transient continuation evidence.
    pub anchors: BTreeSet<Anchor>,
    /// Authored names that make a singleton eligible for sparse promotion.
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
            primary: primary.clone(),
            fallbacks: self.anchors.iter().filter(|a| *a != primary).take(3).cloned().collect(),
            lineage: Vec::new(),
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
            Ok(Record { lineage: record.lineage.clone(), ..next })
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

impl Graph {
    /// Build from the existing complete project graph, deduplicating repeated physical-terminal
    /// appearances and including standalone authored labels/ports. Does not allocate net IDs.
    pub fn build(project: &Project) -> Result<Self, String> {
        let graph = project.electrical_connectivity();
        let scope = project.net_scope();
        let mut groups = BTreeMap::<Group, Network>::new();
        let mut endpoints = BTreeMap::new();
        for sheet in project.sheets() {
            let own = project.for_sheet(sheet.target).ok_or("Unknown sheet while resolving net anchors.")?;
            let occurrence = project.sheet_occurrence(sheet.target).ok_or("Missing occurrence while resolving net anchors.")?.id;
            let mut add = |endpoint: WireEndpoint, kind: AnchorKind, object: u64, terminal: String, name: Option<&str>| -> Result<(), String> {
                let anchor = Anchor {
                    kind,
                    project: project.id,
                    scope,
                    occurrence: if kind == AnchorKind::Terminal { uuid::Uuid::nil() } else { occurrence },
                    object,
                    terminal,
                };
                anchor.validate_form()?;
                let index = graph.net_index_for_endpoint(endpoint);
                let group = index.map_or_else(|| Group::Isolated(anchor.clone()), Group::Connected);
                let network = groups.entry(group).or_insert_with(|| Network { anchors: BTreeSet::new(), names: BTreeSet::new() });
                network.anchors.insert(anchor.clone());
                if let Some(index) = index {
                    network.names.extend(graph.net_names(index).iter().cloned());
                }
                if let Some(name) = name {
                    network.names.insert(name.to_owned());
                }
                endpoints.insert((sheet.target.id(), endpoint), anchor);
                Ok(())
            };
            for symbol in own.symbols() {
                for terminal in symbol.terminals() {
                    // Bare pins are not networks. Avoid allocating persistent-address groups for
                    // every unused pin on large devices; a connected appearance includes aliases.
                    if graph.net_index_for(terminal).is_none() {
                        continue;
                    }
                    add(WireEndpoint::Terminal(terminal), AnchorKind::Terminal, symbol.component, terminal.number().into(), None)?;
                }
            }
            for label in own.net_labels() {
                add(WireEndpoint::NetLabel(label.id), AnchorKind::Label, label.id, String::new(), Some(&label.name))?;
            }
            for port in own.hierarchical_labels() {
                add(WireEndpoint::Label(port.id), AnchorKind::Port, port.id, String::new(), Some(&port.name))?;
            }
        }
        // A parent pin and its child port have one sheet-owned logical identity.
        for sheet in project.sheets() {
            let own = project.for_sheet(sheet.target).ok_or("Unknown parent sheet while resolving net anchors.")?;
            for block in own.sheet_symbols() {
                for pin in own.sheet_pins(block) {
                    let child =
                        project.sheets.iter().find(|sheet| sheet.labels.iter().any(|label| label.id == pin.label.id)).ok_or("Missing child port.")?;
                    let anchor = endpoints.get(&(child.sheet_id, WireEndpoint::Label(pin.label.id))).cloned().ok_or("Missing port anchor.")?;
                    endpoints.insert((sheet.target.id(), WireEndpoint::SheetPin(pin.label.id)), anchor);
                }
            }
        }
        let mut networks: Vec<_> = groups.into_values().collect();
        networks.sort_by(|a, b| a.anchors.first().cmp(&b.anchors.first()));
        let mut by_anchor = BTreeMap::new();
        for (index, network) in networks.iter().enumerate() {
            for anchor in &network.anchors {
                if by_anchor.insert(anchor.clone(), index).is_some() {
                    return Err("A physical-terminal identity appears in different electrical networks.".into());
                }
            }
        }
        Ok(Self { networks, by_anchor, endpoints })
    }

    /// Resolve a validated terminal/label/port command target in its explicit sheet context.
    /// Junctions and foreign/missing endpoints are refused rather than chosen by display order.
    pub fn target(&self, sheet: SheetTarget, value: &serde_json::Value) -> Result<Anchor, CommandError> {
        if let Ok(anchor) = serde_json::from_value::<Anchor>(value.clone()) {
            anchor.validate_form().map_err(CommandError::InvalidParams)?;
            if self.endpoints.iter().any(|((id, _), current)| *id == sheet.id() && current == &anchor) {
                return Ok(anchor);
            }
            return Err(CommandError::InvalidParams("Unknown or foreign typed net anchor in the addressed sheet.".into()));
        }
        let endpoint = WireEndpoint::parse(value)
            .filter(|end| !matches!(end, WireEndpoint::Junction(_)))
            .ok_or_else(|| CommandError::InvalidParams("Net targets require a terminal, net label or hierarchical port.".into()))?;
        self.endpoints
            .get(&(sheet.id(), endpoint))
            .cloned()
            .ok_or_else(|| CommandError::InvalidParams("Unknown or foreign net anchor in the addressed sheet.".into()))
    }

    /// Find the current network for a typed anchor without inferring one from names or geometry.
    pub fn network(&self, anchor: &Anchor) -> Option<usize> {
        self.by_anchor.get(anchor).copied()
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
