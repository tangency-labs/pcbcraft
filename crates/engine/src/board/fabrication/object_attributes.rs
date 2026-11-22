//! Copper object metadata. Net identity comes from board intent; names are only presentation.

use super::BoardView;
use crate::{
    Project,
    board::{
        Pad,
        track::{IslandBuilder, Land, lands},
        via_nets::{Memberships, node_membership},
    },
};
use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    fmt::Write as _,
};
use uuid::Uuid;

/// Attributes captured by a single pad, track or via's drawing operations.
pub(super) struct Object<'a> {
    /// `Some("")` means no net; `None` leaves unresolved or conflicting intent unspecified.
    pub(super) net: Option<&'a str>,
    /// A component reference and (on outer copper only) its physical pin number.
    pub(super) component: Option<(&'a str, Option<&'a str>)>,
}

impl Object<'_> {
    /// Set attributes before the flash, draw or region takes a copy (Ucamco 2026.05 §§5.4, 5.6.13–15).
    pub(super) fn set(&self, text: &mut String) {
        if let Some(name) = self.net {
            text.push_str("%TO.N,");
            // Reserve the N/C: namespace to represent the spec's reserved name injectively.
            if name == "N/C" || name.starts_with("N/C:") {
                text.push_str("N/C:");
            }
            field(text, name);
            text.push_str("*%\n");
        }
        if let Some((reference, pin)) = self.component {
            text.push_str("%TO.C,");
            field(text, reference);
            text.push_str("*%\n");
            if let Some(pin) = pin {
                text.push_str("%TO.P,");
                field(text, reference);
                text.push(',');
                field(text, pin);
                text.push_str("*%\n");
            }
        }
    }

    /// Clear only the attributes set for this copper object, leaving aperture metadata alone (§5.5).
    pub(super) fn clear(&self, text: &mut String) {
        if self.net.is_some() {
            text.push_str("%TD.N*%\n");
        }
        if let Some((_, pin)) = self.component {
            if pin.is_some() {
                text.push_str("%TD.P*%\n");
            }
            text.push_str("%TD.C*%\n");
        }
    }
}

/// Escape Gerber delimiters, the escape character and line/control characters without
/// losing the original text (Ucamco Gerber 2026.05 §§3.4.3–4).
fn field(text: &mut String, value: &str) {
    for c in value.chars() {
        if matches!(c, ',' | '%' | '*' | '\\') || c.is_control() {
            let _ = write!(text, "\\u{:04X}", u32::from(c));
        } else {
            text.push(c);
        }
    }
}

#[derive(Clone, Copy, Default)]
enum Intent {
    #[default]
    Unnamed,
    Named(usize),
    Unresolved,
}

impl Intent {
    /// A single continuous copper island cannot safely select one of several distinct
    /// intended nets. Keep that uncertainty even when later members agree.
    fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Self::Unnamed, intent) | (intent, Self::Unnamed) => intent,
            (Self::Named(a), Self::Named(b)) if a == b => self,
            _ => Self::Unresolved,
        }
    }
}

/// Export-local island intent. Keep one name per intended net, not a name or a set per member.
pub(super) struct RoutedNets {
    objects: BTreeMap<Uuid, Intent>,
    pads: BTreeMap<String, BTreeMap<String, usize>>,
    names: BTreeMap<usize, String>,
}

/// Allocate one Gerber name per identity in membership order. Reserve every original name
/// before adding suffixes, so generated names cannot shadow another net's original name.
fn distinct_names(names: BTreeMap<usize, String>) -> BTreeMap<usize, String> {
    let mut reserved: BTreeSet<_> = names.values().cloned().collect();
    let mut next_suffix = BTreeMap::new();
    names
        .into_iter()
        .map(|(id, name)| match next_suffix.entry(name.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(2usize);
                (id, name)
            }
            Entry::Occupied(mut entry) => loop {
                let suffix = entry.get_mut();
                let candidate = format!("{name} ({suffix})");
                *suffix += 1;
                if reserved.insert(candidate.clone()) {
                    break (id, candidate);
                }
            },
        })
        .collect()
}

/// Without an accepted baseline, unwired terminals are absent from the schematic graph.
/// Multiple copper lands mapped to one such terminal still share board intent. Add export-local
/// memberships beyond all schematic/via IDs; leave single lands and unmapped lands unconnected.
fn add_unaccepted_pad_groups(project: &Project, lands: &[Land], intrinsic: &mut [Option<usize>], memberships: &mut Memberships) {
    let mut next = intrinsic.iter().flatten().copied().chain(memberships.names.keys().copied()).max().map_or(0, |id| id + 1);
    let mut terminals = BTreeMap::new();
    for (node, land) in lands.iter().enumerate() {
        if intrinsic[node].is_some() {
            continue;
        }
        let Some(terminal) = Pad::parse(&land.key.0, &land.key.1).and_then(|pad| project.board.component_pin_for(pad)) else { continue };
        match terminals.entry(terminal) {
            Entry::Vacant(entry) => {
                entry.insert((node, None));
            }
            Entry::Occupied(mut entry) => {
                let (first, assigned) = entry.get_mut();
                let id = *assigned.get_or_insert_with(|| {
                    let id = next;
                    next += 1;
                    intrinsic[*first] = Some(id);
                    memberships.names.insert(id, "Unconnected".into());
                    id
                });
                intrinsic[node] = Some(id);
            }
        }
    }
}

impl RoutedNets {
    /// Share one name table across all copper layers, keyed by intended membership rather than
    /// display text. Only routed objects need physical contact traversal; pads keep their own intent.
    pub(super) fn new(view: BoardView<'_>) -> Self {
        let (tracks, vias) = (view.tracks(), view.vias());
        let lands = lands(view);
        let project = view.project();
        let mut memberships = project.board_intent_memberships();
        let mut intrinsic = node_membership(project, &lands, tracks.len(), vias, &memberships);
        if project.board.baseline.is_none() {
            add_unaccepted_pad_groups(project, &lands, &mut intrinsic, &mut memberships);
        }
        let land_count = lands.len();
        let mut names = BTreeMap::new();
        let objects = if tracks.is_empty() && vias.is_empty() {
            BTreeMap::new()
        } else {
            let roots = IslandBuilder::new(&lands, tracks, vias).fabrication_roots();
            let mut intents = vec![Intent::Unnamed; roots.len()];
            for (node, (&root, &member)) in roots.iter().zip(&intrinsic).enumerate() {
                let intent = match member {
                    Some(net) => Intent::Named(net),
                    None if node.checked_sub(land_count + tracks.len()).and_then(|i| vias.get(i)).is_some_and(|via| via.net_assignment.is_some()) => {
                        Intent::Unresolved
                    }
                    None => Intent::Unnamed,
                };
                intents[root] = intents[root].combine(intent);
            }
            tracks.iter().map(|t| t.id).chain(vias.iter().map(|v| v.id)).zip(roots[land_count..].iter().map(|&i| intents[i])).collect()
        };
        let mut pads = BTreeMap::<String, BTreeMap<String, usize>>::new();
        for (land, &member) in lands.into_iter().zip(&intrinsic) {
            if let Some(id) = member {
                let (footprint, pad) = land.key;
                pads.entry(footprint).or_default().insert(pad, id);
                if let Some(name) = land.net.filter(|name| !name.is_empty()) {
                    names.entry(id).or_insert(name);
                }
            }
        }
        for id in intrinsic.into_iter().flatten() {
            names.entry(id).or_insert_with(|| memberships.names.get(&id).filter(|name| !name.is_empty()).cloned().unwrap_or_else(|| "Net".into()));
        }
        Self { objects, pads, names: distinct_names(names) }
    }

    /// Resolve pad intent independently of physical contacts, including on a board without routes.
    pub(super) fn pad_name(&self, footprint: &str, pad: &str) -> &str {
        self.pads.get(footprint).and_then(|pads| pads.get(pad)).and_then(|id| self.names.get(id)).map_or("", String::as_str)
    }

    /// Unnamed copper gets an explicit empty field. An unresolved or shorted island gets
    /// no .N so CAM does not mistake an unknown net for deliberately unconnected copper.
    pub(super) fn object(&self, id: Uuid) -> Object<'_> {
        let net = match self.objects.get(&id) {
            Some(Intent::Named(id)) => self.names.get(id).map(String::as_str),
            Some(Intent::Unnamed) => Some(""),
            Some(Intent::Unresolved) | None => None,
        };
        Object { net, component: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A generated suffix must not overwrite a display name that already uses that spelling.
    #[test]
    fn gerber_names_reserve_existing_suffixes() {
        let names = [(0, "Signal"), (1, "Signal"), (2, "Signal (2)"), (3, "Signal")].map(|(id, name)| (id, name.to_owned()));
        assert_eq!(distinct_names(BTreeMap::from(names)).into_values().collect::<Vec<_>>(), ["Signal", "Signal (3)", "Signal (2)", "Signal (4)"]);
    }
}
