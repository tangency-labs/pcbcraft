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
        let own = self.for_sheet(sheet).ok_or_else(|| CommandError::NotFound("Unknown selection sheet.".into()))?;
        if own.endpoint_position(selected).is_none() {
            return Err(CommandError::NotFound("Unknown or foreign selected endpoint.".into()));
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
        let intent = self.board_intent_memberships();
        let base = self.board.baseline.as_ref().map_or_else(|| self.electrical_connectivity().nets.len(), |b| b.nets.len());
        if let Some((&net, _)) = intent.ids.iter().find(|(_, member)| **member == membership && membership >= base) {
            return Ok(Some(ClassTarget::Persistent { project: self.id, scope: self.net_scope(), net }));
        }
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

impl Project {
    /// Alias a saved identity to accepted copper only under unique, complete continuation evidence.
    /// Pending splits/merges get distinct transient memberships until reviewed board acceptance.
    pub(crate) fn board_intent_memberships(&self) -> crate::board::via_nets::Memberships {
        if !self.board.vias.iter().any(|v| v.net_assignment.is_some()) {
            return crate::board::via_nets::Memberships::default();
        }
        self.accepted_net_memberships()
    }

    /// Resolve active persistent net identities even on boards without authored via assignments.
    pub(crate) fn accepted_net_memberships(&self) -> crate::board::via_nets::Memberships {
        let mut result = crate::board::via_nets::Memberships::default();
        if self.net_registry.records.is_empty() {
            return result;
        }
        let Ok(graph) = Graph::build(self) else { return result };
        let electrical = self.electrical_connectivity();
        let accepted = self.board.baseline.as_ref().map(|b| b.terminal_net_index());
        let active: BTreeMap<_, _> = self
            .net_registry
            .records
            .iter()
            .filter(|r| matches!(r.state, super::State::Active {}))
            .filter_map(|r| Some((graph.network(&r.primary)?, r.id)))
            .collect();
        let terminals: BTreeMap<_, _> = graph
            .by_anchor
            .iter()
            .filter(|(a, _)| a.kind == AnchorKind::Terminal)
            .map(|(a, n)| ((a.object, a.terminal.as_str()), active.get(n).copied()))
            .collect();
        let mut accepted_ids = BTreeMap::<usize, BTreeSet<Option<uuid::Uuid>>>::new();
        if let Some(accepted) = &accepted {
            for (key, member) in accepted {
                accepted_ids.entry(*member).or_default().insert(terminals.get(key).copied().flatten());
            }
        }
        let base = self.board.baseline.as_ref().map_or(electrical.nets.len(), |b| b.nets.len());
        for (offset, record) in self.net_registry.records.iter().enumerate() {
            if !matches!(record.state, super::State::Active {}) {
                continue;
            }
            let Some(network) = graph.network(&record.primary) else { continue };
            let anchors = &graph.networks[network].anchors;
            let raw: BTreeSet<_> = anchors
                .iter()
                .filter(|a| a.kind == AnchorKind::Terminal)
                .filter_map(|a| {
                    if let Some(accepted) = &accepted {
                        accepted.get(&(a.object, a.terminal.as_str())).copied()
                    } else {
                        electrical.net_index_for_endpoint(endpoint(self, a)?)
                    }
                })
                .collect();
            let alias = raw.first().copied().filter(|member| {
                raw.len() == 1 && accepted.as_ref().is_none_or(|_| accepted_ids.get(member) == Some(&BTreeSet::from([Some(record.id)])))
            });
            let member = alias.unwrap_or(base + offset);
            let name = anchors
                .iter()
                .find_map(|a| electrical.net_index_for_endpoint(endpoint(self, a)?))
                .map(|n| electrical.net_name(n))
                .unwrap_or_else(|| record.generated_number.map(super::numbering::name).unwrap_or_else(|| "Unresolved net".into()));
            let name = alias.and_then(|n| self.board.baseline.as_ref()?.net_name(n)).unwrap_or(name);
            result.ids.insert(record.id, member);
            result.names.insert(member, name);
        }
        result
    }
}
