//! Read-only inter-sheet net overview: which sheets each net reaches, and how it gets there.
//!
//! The overview is derived from the same whole-project connectivity as the netlist and the Net
//! Inspector. It never stores a second graph, allocates a net identity or changes history.

use super::{Anchor, State, graph::Graph};
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
#[derive(Default)]
struct Reach {
    via: BTreeSet<Via>,
    terminals: BTreeSet<(u64, String)>,
}

impl Project {
    /// List every net with endpoints on more than one sheet. With `only`, keep the nets that
    /// reach that sheet. It reads the project's cached whole-project graph and stores nothing,
    /// and the result holds at most one entry per net and sheet. Local labels never join
    /// sheets, so equal local names on two sheets are two nets. Fails only when the net
    /// anchors cannot be resolved.
    pub fn sheet_overview(&self, only: Option<SheetTarget>) -> Result<SheetOverview, String> {
        let electrical = self.borrowed_electrical_connectivity();
        let graph = Graph::build(self)?;
        let pages: BTreeMap<Uuid, usize> = self.page_order().into_iter().enumerate().map(|(index, target)| (target.id(), index + 1)).collect();
        let mut rows = Rows::default();
        let mut units = BTreeMap::<(u64, String), (usize, BTreeSet<Uuid>)>::new();
        for sheet in self.sheets() {
            let own = self.for_sheet(sheet.target).ok_or("Unknown sheet while building the sheet overview.")?;
            let power: BTreeSet<_> = own.power_terminals().map(|(end, _, _)| end).collect();
            for symbol in own.symbols() {
                for terminal in symbol.terminals() {
                    let end = WireEndpoint::Terminal(terminal);
                    if power.contains(&end) {
                        rows.mark(electrical, sheet.target, end, Some(Via::PowerSymbol));
                        continue;
                    }
                    let key = (symbol.component, terminal.number().to_owned());
                    if let Some(reach) = rows.mark(electrical, sheet.target, end, None) {
                        reach.terminals.insert(key.clone());
                    }
                    if let Some(row) = electrical.net_index_for(terminal) {
                        units.entry(key).or_insert_with(|| (row, BTreeSet::new())).1.insert(sheet.target.id());
                    }
                }
            }
            for label in own.net_labels() {
                rows.mark(electrical, sheet.target, WireEndpoint::NetLabel(label.id), label.style.global().then_some(Via::GlobalLabel));
            }
            for label in own.hierarchical_labels() {
                rows.mark(electrical, sheet.target, WireEndpoint::Label(label.id), Some(Via::HierarchicalLabel));
            }
            for block in own.sheet_symbols() {
                for pin in own.sheet_pins(block) {
                    rows.mark(electrical, sheet.target, WireEndpoint::SheetPin(pin.label.id), Some(Via::SheetPin));
                }
            }
        }
        // A part terminal drawn on units placed on several sheets carries its net to each of them.
        for (row, sheets) in units.into_values().filter(|(_, sheets)| sheets.len() > 1) {
            for sheet in sheets {
                if let Some(reach) = rows.reached.get_mut(&row).and_then(|reached| reached.get_mut(&sheet)) {
                    reach.via.insert(Via::SharedTerminal);
                }
            }
        }
        let mut active = BTreeMap::new();
        for record in self.net_registry.records.iter().filter(|record| matches!(record.state, State::Active {})) {
            if let Some(network) = graph.network(&record.primary) {
                active.entry(network).or_insert(record.id);
            }
        }
        let mut nets = Vec::new();
        for (row, reached) in rows.reached {
            if reached.len() < 2 || only.is_some_and(|sheet| !reached.contains_key(&sheet.id())) {
                continue;
            }
            let (sheet, end) = rows.first.get(&row).copied().ok_or("Missing endpoint while building the sheet overview.")?;
            let network = graph.endpoint_anchor(sheet, end).and_then(|anchor| graph.network(anchor));
            let anchor = network
                .and_then(|index| graph.networks.get(index))
                .and_then(|network| network.anchors.first())
                .cloned()
                .ok_or("Missing net anchor while building the sheet overview.")?;
            let mut sheets = Vec::with_capacity(reached.len());
            for (id, reach) in reached {
                let info = self.sheets().find(|sheet| sheet.target.id() == id).ok_or("Unknown sheet while building the sheet overview.")?;
                sheets.push(SheetReach {
                    sheet: id,
                    occurrence: self.sheet_occurrence(info.target).ok_or("Missing sheet occurrence while building the sheet overview.")?.id,
                    name: info.name.to_owned(),
                    page: pages.get(&id).copied().ok_or("Missing page while building the sheet overview.")?,
                    via: reach.via.into_iter().collect(),
                    terminals: reach.terminals.len(),
                });
            }
            sheets.sort_by_key(|reach| reach.page);
            nets.push(CrossSheetNet {
                id: network.and_then(|index| active.get(&index).copied()),
                anchor,
                name: electrical.net_name(row),
                names: electrical.net_names(row).to_vec(),
                sheets,
            });
        }
        nets.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.anchor.cmp(&b.anchor)));
        Ok(SheetOverview { nets })
    }

    /// Whether two snapshots, or two sheet projections of one snapshot, give the same
    /// [`Self::sheet_overview`]. Shared collections mean no sheet, part value, connection or net
    /// identity changed; a false answer only means the overview must be read again.
    pub fn same_sheet_overview(&self, other: &Self) -> bool {
        self.id == other.id
            && Arc::ptr_eq(&self.sheets, &other.sheets)
            && Arc::ptr_eq(&self.components, &other.components)
            && Arc::ptr_eq(&self.named_graph, &other.named_graph)
            && Arc::ptr_eq(&self.net_registry, &other.net_registry)
    }
}

/// Per-net, per-sheet contributions, plus the first endpoint seen on each net for its anchor.
#[derive(Default)]
struct Rows {
    reached: BTreeMap<usize, BTreeMap<Uuid, Reach>>,
    first: BTreeMap<usize, (SheetTarget, WireEndpoint)>,
}

impl Rows {
    /// Record that the net holding `end`, when it has one, reaches `sheet`.
    fn mark(&mut self, electrical: &Connectivity, sheet: SheetTarget, end: WireEndpoint, via: Option<Via>) -> Option<&mut Reach> {
        let row = electrical.net_index_for_endpoint(end)?;
        self.first.entry(row).or_insert((sheet, end));
        let reach = self.reached.entry(row).or_default().entry(sheet.id()).or_default();
        reach.via.extend(via);
        Some(reach)
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
