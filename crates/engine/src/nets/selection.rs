//! Selection resolves current logical identity without promoting nets or caching a second graph.
use super::{Anchor, AnchorKind, ClassTarget, graph::Graph};
use crate::{CommandError, Project, SheetTarget, WireEndpoint};
use std::collections::{BTreeMap, BTreeSet};

/// Current physical component terminals, labels and interfaces retain their exact endpoint
/// identities; codec-only junction anchors never become editable class targets.
fn endpoint(project: &Project, anchor: &Anchor) -> Option<WireEndpoint> {
    match anchor.kind {
        AnchorKind::Terminal => project.component_terminal(anchor.object, &anchor.terminal).map(WireEndpoint::Terminal),
        AnchorKind::Label => Some(WireEndpoint::NetLabel(anchor.object)),
        AnchorKind::Port => Some(WireEndpoint::Label(anchor.object)),
        AnchorKind::Junction => None,
    }
}

/// A selection must prove exactly one current meaningful network. Names and traversal order
/// never choose a winner; empty/bare geometry is unavailable and competing groups are explicit.
fn target(graph: &Graph, indices: BTreeSet<usize>) -> Result<Option<ClassTarget>, CommandError> {
    if indices.len() > 1 {
        return Err(CommandError::Unavailable(
            "Selection spans distinct intended nets or a pending connectivity split. Update the board or select one current net.".into(),
        ));
    }
    Ok(indices
        .first()
        .and_then(|index| graph.networks.get(*index))
        .filter(|net| net.meaningful())
        .and_then(|net| net.anchors.first())
        .cloned()
        .map(|anchor| ClassTarget::Derived { anchor }))
}

impl Project {
    /// Resolve a selected schematic endpoint (including an attached junction) to one supported
    /// current class target. Foreign/missing endpoints fail; bare pins and free wire groups
    /// return None. The read neither allocates identities nor changes history or saved bytes.
    pub fn net_class_target_for_endpoint(&self, sheet: SheetTarget, selected: WireEndpoint) -> Result<Option<ClassTarget>, CommandError> {
        let own = self.for_sheet(sheet).ok_or_else(|| CommandError::InvalidParams("Unknown selection sheet.".into()))?;
        if own.endpoint_position(selected).is_none() {
            return Err(CommandError::InvalidParams("Unknown or foreign selected endpoint.".into()));
        }
        let graph = Graph::build(self).map_err(CommandError::InvalidParams)?;
        if let Some(anchor) = graph.endpoint_anchor(sheet, selected) {
            return target(&graph, graph.network(anchor).into_iter().collect());
        }
        let electrical = self.electrical_connectivity();
        let Some(net) = electrical.net_index_for_endpoint(selected) else { return Ok(None) };
        let indices = graph
            .by_anchor
            .iter()
            .filter_map(|(anchor, index)| (electrical.net_index_for_endpoint(endpoint(self, anchor)?) == Some(net)).then_some(*index))
            .collect();
        target(&graph, indices)
    }

    /// Convert one accepted physical membership into current logical authority. Pending splits
    /// can leave multiple current networks on accepted copper; refuse rather than assigning one
    /// branch by name. The membership is snapshot-local and must come from this board geometry.
    pub(crate) fn net_class_target_for_board_membership(&self, membership: usize) -> Result<Option<ClassTarget>, CommandError> {
        let graph = Graph::build(self).map_err(CommandError::InvalidParams)?;
        let accepted = self.board.baseline.as_ref().map(|baseline| baseline.terminal_net_index());
        let electrical = accepted.is_none().then(|| self.electrical_connectivity());
        // Identity projection omits bare terminals. Retain accepted physical terminals in
        // split evidence so a detached bare branch cannot silently disappear from authority.
        let mut indices = BTreeSet::new();
        let mut unsupported = false;
        if let Some(accepted) = accepted {
            let mut current = BTreeMap::<_, BTreeSet<_>>::new();
            for (anchor, index) in &graph.by_anchor {
                if anchor.kind == AnchorKind::Terminal {
                    current.entry((anchor.object, anchor.terminal.as_str())).or_default().insert(*index);
                }
            }
            for ((component, terminal), net) in accepted {
                if net != membership {
                    continue;
                }
                match current.get(&(component, terminal)) {
                    Some(found) => indices.extend(found),
                    None => unsupported = true,
                }
            }
        } else {
            for (anchor, index) in &graph.by_anchor {
                if anchor.kind == AnchorKind::Terminal
                    && electrical.as_ref().and_then(|graph| endpoint(self, anchor).and_then(|pin| graph.net_index_for_endpoint(pin)))
                        == Some(membership)
                {
                    indices.insert(*index);
                }
            }
        }
        if unsupported && !indices.is_empty() {
            return Err(CommandError::Unavailable(
                "Accepted copper includes terminals outside its current supported net. Update the board before assigning classes.".into(),
            ));
        }
        target(&graph, indices)
    }
}
