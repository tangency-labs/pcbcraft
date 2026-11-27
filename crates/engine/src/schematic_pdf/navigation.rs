//! Page destinations derived from authored hierarchy and one immutable electrical graph.

use std::collections::{BTreeMap, BTreeSet};

use crate::{Project, schematic::WireEndpoint};

use super::occurrences::Visit;

pub(super) struct Navigation {
    children: BTreeMap<(usize, u64), usize>,
    next_pages: BTreeMap<WireEndpoint, usize>,
}

impl Navigation {
    /// Resolve each symbol slot to a child page. Resolve linked labels by actual membership
    /// in the current graph; neither display names nor a saved net registry are required.
    pub(super) fn new(project: &Project, visits: &[Visit<'_>], sheets: &[Project]) -> Self {
        let children = visits.iter().enumerate().filter_map(|(page, visit)| Some((visit.parent?, page))).collect();
        let next_pages = net_destinations(project, sheets);
        Self { children, next_pages }
    }

    /// Look up an authored sheet symbol on this page, not its display name or definition ID.
    pub(super) fn child(&self, page: usize, slot: u64) -> Option<usize> {
        self.children.get(&(page, slot)).copied()
    }

    /// Return the next other page carrying the connected anchor's net, if there is one.
    pub(super) fn net(&self, anchor: WireEndpoint) -> Option<usize> {
        self.next_pages.get(&anchor).copied()
    }
}

/// Index endpoint ownership once, then walk the complete graph once. Row numbers stay
/// local to that graph; page order, not row or persistent net ID order, decides destinations.
fn net_destinations(project: &Project, sheets: &[Project]) -> BTreeMap<WireEndpoint, usize> {
    let sources: BTreeSet<_> = sheets
        .iter()
        .flat_map(|sheet| {
            sheet
                .net_labels()
                .iter()
                .filter(|label| label.style.global())
                .map(|label| WireEndpoint::NetLabel(label.id))
                .chain(sheet.hierarchical_labels().iter().map(|label| WireEndpoint::Label(label.id)))
        })
        .collect();
    if sources.is_empty() || sheets.len() < 2 {
        return BTreeMap::new();
    }
    let mut owners = BTreeMap::new();
    let mut definitions = BTreeSet::new();
    for (page, sheet) in sheets.iter().enumerate() {
        // The graph has no occurrence-qualified endpoints for repeated definitions yet.
        // Never invent a cross-page electrical connection for such input.
        if !definitions.insert(sheet.sheet_target().id()) {
            return BTreeMap::new();
        }
        for endpoint in sheet
            .hierarchy_endpoints()
            .map(|(end, _)| end)
            .chain(sheet.symbols().iter().flat_map(|s| s.terminals().into_iter().map(WireEndpoint::Terminal)))
            .chain(sheet.wires().iter().flat_map(|wire| wire.endpoints()))
        {
            owners.insert(endpoint, page);
        }
    }
    let graph = project.electrical_connectivity();
    let mut links = BTreeMap::new();
    for row in 0..graph.nets.len() {
        let mut anchors = Vec::new();
        let mut pages = Vec::new();
        for endpoint in graph.endpoints(row) {
            if let Some(&page) = owners.get(&endpoint) {
                pages.push(page);
                if sources.contains(&endpoint) {
                    anchors.push((endpoint, page));
                }
            }
        }
        if anchors.is_empty() {
            continue;
        }
        pages.sort_unstable();
        pages.dedup();
        if pages.len() > 1 {
            for (endpoint, page) in anchors {
                let next = pages.partition_point(|&other| other <= page) % pages.len();
                if let Some(&destination) = pages.get(next) {
                    links.insert(endpoint, destination);
                }
            }
        }
    }
    links
}
