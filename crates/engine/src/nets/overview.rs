//! Read-only inter-sheet net overview: which sheets each net reaches, and how it gets there.
//!
//! The overview is derived from the same whole-project connectivity as the netlist and the Net
//! Inspector. It never stores a second graph, allocates a net identity or changes history.

use super::{Anchor, Registry, Scope, State, graph::Graph};
use crate::{CommandError, Project, Session, SheetTarget, WireEndpoint, schematic::Connectivity};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use uuid::Uuid;

/// List every net that reaches more than one sheet, with those sheets and how it reaches them.
pub const SHEET_OVERVIEW: &str = "nets.sheet_overview";

/// How a net arrives on one sheet. Sorted in this order within a sheet entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Via {
    /// A global text label on this sheet joins the matching name on other sheets.
    GlobalLabel,
    /// A power symbol on this sheet; its value names a rail shared by every sheet.
    PowerSymbol,
    /// A hierarchical label: this sheet is the child side of a sheet pin.
    HierarchicalLabel,
    /// A sheet pin on the symbol of a child placed on this sheet.
    SheetPin,
    /// A terminal of a part whose units are placed on more than one sheet.
    SharedTerminal,
}

impl Via {
    /// Plain wording for presentation; commands use the snake_case serialization instead.
    pub fn label(self) -> &'static str {
        match self {
            Self::GlobalLabel => "global label",
            Self::PowerSymbol => "power symbol",
            Self::HierarchicalLabel => "hierarchical label",
            Self::SheetPin => "sheet pin",
            Self::SharedTerminal => "shared part terminal",
        }
    }
}

/// One sheet occurrence a net reaches.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SheetReach {
    /// Sheet UUID, as other sheet-scoped commands take it.
    pub sheet: Uuid,
    /// The sheet's occurrence. Each sheet has one occurrence while repeated sheet use is refused;
    /// a repeated sheet would be reported once per occurrence.
    pub occurrence: Uuid,
    /// Current sheet name.
    pub name: String,
    /// Page number in Browser order, as `@instance.Page` shows it.
    pub page: usize,
    /// How the net reaches this sheet; empty when it only arrives through an ordinary wire
    /// to a part terminal, local label or junction that another listed mechanism carries.
    pub via: Vec<Via>,
    /// Logical part terminals on this net drawn on this sheet, not counting power symbols.
    pub terminals: usize,
}

/// One net that reaches more than one sheet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CrossSheetNet {
    /// Persistent net UUID, or null for a net the registry does not yet identify.
    pub id: Option<Uuid>,
    /// Canonical derived address of the net, accepted as a derived target by
    /// `nets.inspect_classes` and `nets.set_classes`. It can change when the net is edited.
    pub anchor: Anchor,
    /// Readable name, the same text the Browser and Net Inspector show.
    pub name: String,
    /// Every authored name on the net, sorted.
    pub names: Vec<String>,
    /// Sheets reached, in page order.
    pub sheets: Vec<SheetReach>,
}

/// The nets that reach more than one sheet, sorted by name and then anchor.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SheetOverview {
    /// Cross-sheet nets; empty when every net stays on one sheet.
    pub nets: Vec<CrossSheetNet>,
}

/// What one sheet contributes to one net while the overview is collected.
#[derive(Debug, Default, PartialEq, Eq)]
struct Reach {
    via: BTreeSet<Via>,
    terminals: BTreeSet<(u64, String)>,
}

/// A shared value compared by identity: equal only when both hold the same allocation.
#[derive(Debug)]
struct Same<T>(Arc<T>);

impl<T> PartialEq for Same<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl<T> Eq for Same<T> {}

/// One sheet as the overview reports it.
#[derive(Debug, PartialEq, Eq)]
struct SheetInfo {
    target: SheetTarget,
    occurrence: Uuid,
    name: String,
    page: usize,
}

/// Everything the overview reports about one cross-sheet net except its anchor and identity.
#[derive(Debug, PartialEq, Eq)]
struct NetBasis {
    name: String,
    names: Vec<String>,
    reached: BTreeMap<Uuid, Reach>,
    /// Every endpoint of the net with its sheet and, for part terminals, the owning part, which
    /// together fix the net's anchors.
    endpoints: BTreeSet<(Uuid, WireEndpoint, u64)>,
}

/// What a project's [`SheetOverview`] is derived from, read from the cached whole-project
/// connectivity without resolving net anchors. Two snapshots with equal bases have equal
/// overviews, so a viewer reads the overview again only when the basis changes: moving parts,
/// wires, labels or sheet symbols without changing what they connect keeps it.
#[derive(Debug, PartialEq, Eq)]
pub struct SheetOverviewBasis {
    project: Uuid,
    scope: Scope,
    registry: Same<Registry>,
    sheets: Vec<SheetInfo>,
    nets: Vec<NetBasis>,
}

impl Project {
    /// List every net with endpoints on more than one sheet. With `only`, keep the nets that
    /// reach that sheet. It reads the project's cached whole-project graph and stores nothing,
    /// and the result holds at most one entry per net and sheet. Matching local names alone do
    /// not join sheets. The overview omits nets that stay on one sheet. Fails only when the net
    /// anchors cannot be resolved.
    pub fn sheet_overview(&self, only: Option<SheetTarget>) -> Result<SheetOverview, String> {
        self.sheet_overview_basis()?.overview(self, only)
    }

    /// Collect the [`SheetOverviewBasis`] of this snapshot. It costs one pass over the
    /// project's endpoints, like reading its connectivity, and skips the anchor resolution
    /// that [`SheetOverviewBasis::overview`] performs.
    pub fn sheet_overview_basis(&self) -> Result<SheetOverviewBasis, String> {
        let electrical = self.borrowed_electrical_connectivity();
        let pages: BTreeMap<Uuid, usize> = self.page_order().into_iter().enumerate().map(|(index, target)| (target.id(), index + 1)).collect();
        let mut sheets = Vec::new();
        let mut rows = Rows::default();
        let mut global_markers = BTreeMap::<(usize, String), BTreeMap<Uuid, (bool, bool)>>::new();
        let mut hierarchical_labels = BTreeMap::<u64, (usize, Uuid)>::new();
        let mut sheet_pins = Vec::<(u64, usize, Uuid)>::new();
        let mut units = BTreeMap::<(u64, String), (usize, BTreeSet<Uuid>)>::new();
        for sheet in self.sheets() {
            sheets.push(SheetInfo {
                target: sheet.target,
                occurrence: self.sheet_occurrence(sheet.target).ok_or("Missing sheet occurrence while building the sheet overview.")?.id,
                name: sheet.name.to_owned(),
                page: pages.get(&sheet.target.id()).copied().ok_or("Missing page while building the sheet overview.")?,
            });
            let own = self.for_sheet(sheet.target).ok_or("Unknown sheet while building the sheet overview.")?;
            let power: BTreeMap<_, _> = own.power_terminals().map(|(end, name, _)| (end, name.to_owned())).collect();
            for symbol in own.symbols() {
                for terminal in symbol.terminals() {
                    let end = WireEndpoint::Terminal(terminal);
                    if let Some(name) = power.get(&end) {
                        if let Some(row) = rows.mark(electrical, sheet.target, end, symbol.component) {
                            global_markers.entry((row, name.clone())).or_default().entry(sheet.target.id()).or_default().1 = true;
                        }
                        continue;
                    }
                    let key = (symbol.component, terminal.number().to_owned());
                    if let Some(row) = rows.mark(electrical, sheet.target, end, symbol.component) {
                        if let Some(reach) = rows.reached.get_mut(&row).and_then(|reached| reached.get_mut(&sheet.target.id())) {
                            reach.terminals.insert(key.clone());
                        }
                        units.entry(key).or_insert_with(|| (row, BTreeSet::new())).1.insert(sheet.target.id());
                    }
                }
            }
            for label in own.net_labels() {
                if let Some(row) = rows.mark(electrical, sheet.target, WireEndpoint::NetLabel(label.id), 0)
                    && label.style.global()
                {
                    global_markers.entry((row, label.name.clone())).or_default().entry(sheet.target.id()).or_default().0 = true;
                }
            }
            for label in own.hierarchical_labels() {
                if let Some(row) = rows.mark(electrical, sheet.target, WireEndpoint::Label(label.id), 0) {
                    hierarchical_labels.insert(label.id, (row, sheet.target.id()));
                }
            }
            for block in own.sheet_symbols() {
                for pin in own.sheet_pins(block) {
                    if let Some(row) = rows.mark(electrical, sheet.target, WireEndpoint::SheetPin(pin.label.id), 0) {
                        sheet_pins.push((pin.label.id, row, sheet.target.id()));
                    }
                }
            }
        }
        // A global label or power symbol counts as a crossing only when the same name appears on
        // another sheet occurrence.
        for ((row, _), by_sheet) in global_markers.into_iter().filter(|(_, by_sheet)| by_sheet.len() > 1) {
            for (sheet, (has_global, has_power)) in by_sheet {
                if let Some(reach) = rows.reached.get_mut(&row).and_then(|reached| reached.get_mut(&sheet)) {
                    if has_global {
                        reach.via.insert(Via::GlobalLabel);
                    }
                    if has_power {
                        reach.via.insert(Via::PowerSymbol);
                    }
                }
            }
        }
        // A sheet pin and its child hierarchical label count as a crossing only when joined on the
        // same net across distinct sheets.
        for (label_id, pin_row, parent_sheet) in sheet_pins {
            if let Some(&(child_row, child_sheet)) = hierarchical_labels.get(&label_id)
                && pin_row == child_row
                && parent_sheet != child_sheet
            {
                if let Some(reach) = rows.reached.get_mut(&pin_row).and_then(|reached| reached.get_mut(&parent_sheet)) {
                    reach.via.insert(Via::SheetPin);
                }
                if let Some(reach) = rows.reached.get_mut(&child_row).and_then(|reached| reached.get_mut(&child_sheet)) {
                    reach.via.insert(Via::HierarchicalLabel);
                }
            }
        }
        // A part terminal drawn on units placed on several sheets carries its net to each of them.
        for (row, reached) in units.into_values().filter(|(_, reached)| reached.len() > 1) {
            for sheet in reached {
                if let Some(reach) = rows.reached.get_mut(&row).and_then(|reached| reached.get_mut(&sheet)) {
                    reach.via.insert(Via::SharedTerminal);
                }
            }
        }
        let mut nets = Vec::new();
        for (row, reached) in rows.reached {
            if reached.len() < 2 {
                continue;
            }
            let endpoints = rows.endpoints.remove(&row).unwrap_or_default();
            nets.push(NetBasis { name: electrical.net_name(row), names: electrical.net_names(row).to_vec(), reached, endpoints });
        }
        nets.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.endpoints.first().cmp(&b.endpoints.first())));
        Ok(SheetOverviewBasis { project: self.id, scope: self.net_scope(), registry: Same(self.net_registry.clone()), sheets, nets })
    }

    /// Whether two snapshots, or two sheet projections of one snapshot, share every collection
    /// the overview reads, so [`Self::sheet_overview`] is the same without comparing bases. A
    /// false answer only means the basis must be read again.
    pub fn same_sheet_overview(&self, other: &Self) -> bool {
        self.id == other.id
            && Arc::ptr_eq(&self.sheets, &other.sheets)
            && Arc::ptr_eq(&self.components, &other.components)
            && Arc::ptr_eq(&self.named_graph, &other.named_graph)
            && Arc::ptr_eq(&self.net_registry, &other.net_registry)
    }
}

impl SheetOverviewBasis {
    /// Resolve each net's anchor and persistent identity in `project`, the snapshot this basis
    /// was collected from or one with an equal basis. With `only`, keep the nets that reach
    /// that sheet.
    pub fn overview(&self, project: &Project, only: Option<SheetTarget>) -> Result<SheetOverview, String> {
        let graph = Graph::build(project)?;
        let mut active = BTreeMap::new();
        for record in self.registry.0.records.iter().filter(|record| matches!(record.state, State::Active {})) {
            if let Some(network) = graph.network(&record.primary) {
                active.entry(network).or_insert(record.id);
            }
        }
        let mut nets = Vec::new();
        for net in self.nets.iter().filter(|net| only.is_none_or(|sheet| net.reached.contains_key(&sheet.id()))) {
            let network = net
                .endpoints
                .iter()
                .find_map(|(sheet, end, _)| {
                    let target = self.sheets.iter().find(|info| info.target.id() == *sheet)?.target;
                    graph.endpoint_anchor(target, *end).and_then(|anchor| graph.network(anchor))
                })
                .ok_or("Missing endpoint while building the sheet overview.")?;
            let anchor = graph
                .networks
                .get(network)
                .and_then(|network| network.anchors.first())
                .cloned()
                .ok_or("Missing net anchor while building the sheet overview.")?;
            let mut sheets = Vec::with_capacity(net.reached.len());
            for (id, reach) in &net.reached {
                let info = self.sheets.iter().find(|info| info.target.id() == *id).ok_or("Unknown sheet while building the sheet overview.")?;
                sheets.push(SheetReach {
                    sheet: *id,
                    occurrence: info.occurrence,
                    name: info.name.clone(),
                    page: info.page,
                    via: reach.via.iter().copied().collect(),
                    terminals: reach.terminals.len(),
                });
            }
            sheets.sort_by_key(|reach| reach.page);
            nets.push(CrossSheetNet { id: active.get(&network).copied(), anchor, name: net.name.clone(), names: net.names.clone(), sheets });
        }
        nets.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.anchor.cmp(&b.anchor)));
        Ok(SheetOverview { nets })
    }
}

/// Per-net, per-sheet contributions, plus every endpoint seen on each net.
#[derive(Default)]
struct Rows {
    reached: BTreeMap<usize, BTreeMap<Uuid, Reach>>,
    endpoints: BTreeMap<usize, BTreeSet<(Uuid, WireEndpoint, u64)>>,
}

impl Rows {
    /// Record that the net holding `end`, when it has one, reaches `sheet`. `part` is the part
    /// owning a terminal endpoint and zero otherwise.
    fn mark(&mut self, electrical: &Connectivity, sheet: SheetTarget, end: WireEndpoint, part: u64) -> Option<usize> {
        let row = electrical.net_index_for_endpoint(end)?;
        self.endpoints.entry(row).or_default().insert((sheet.id(), end, part));
        self.reached.entry(row).or_default().entry(sheet.id()).or_default();
        Some(row)
    }
}

impl Session {
    /// Answer `nets.sheet_overview` for the open project without changing revision or history.
    /// An optional `sheet` must name a sheet of this project.
    pub(crate) fn sheet_overview_query(&self, params: &Value) -> Result<Value, CommandError> {
        let project = self.open_project()?;
        let only = params.get("sheet").map(|sheet| project.resolve_sheet(sheet.as_str())).transpose()?;
        let overview = project.sheet_overview(only).map_err(CommandError::Unavailable)?;
        Ok(json!({"revision":self.revision(),"sheet_overview":overview}))
    }
}

#[cfg(test)]
mod tests;
