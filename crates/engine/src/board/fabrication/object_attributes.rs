//! Copper object metadata. Net identity comes from board intent; names are only presentation.

use super::BoardView;
use crate::board::{
    track::{IslandBuilder, lands},
    via_nets::node_membership,
};
use std::{collections::BTreeMap, fmt::Write as _};
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
#[derive(Default)]
pub(super) struct RoutedNets {
    objects: BTreeMap<Uuid, Intent>,
    names: BTreeMap<usize, String>,
}

impl RoutedNets {
    /// Derive track and via intent once for the export, including plated transitions,
    /// direct pad contacts and authored via assignments. Pad intent remains independent.
    pub(super) fn new(view: BoardView<'_>) -> Self {
        let (tracks, vias) = (view.tracks(), view.vias());
        if tracks.is_empty() && vias.is_empty() {
            return Self::default();
        }
        let lands = lands(view);
        let project = view.project();
        let memberships = project.board_intent_memberships();
        let intrinsic = node_membership(project, &lands, tracks.len(), vias, &memberships);
        let roots = IslandBuilder::new(&lands, tracks, vias).fabrication_roots();
        let mut intents = vec![Intent::Unnamed; roots.len()];
        let mut names = memberships.names;
        for (node, (&root, member)) in roots.iter().zip(intrinsic).enumerate() {
            let intent = match member {
                Some(net) => {
                    if let Some(name) = lands.get(node).and_then(|land| land.net.as_ref()) {
                        names.entry(net).or_insert_with(|| name.clone());
                    }
                    Intent::Named(net)
                }
                None if node.checked_sub(lands.len() + tracks.len()).and_then(|i| vias.get(i)).is_some_and(|via| via.net_assignment.is_some()) => {
                    Intent::Unresolved
                }
                None => Intent::Unnamed,
            };
            intents[root] = intents[root].combine(intent);
        }
        let objects = tracks.iter().map(|t| t.id).chain(vias.iter().map(|v| v.id)).zip(roots[lands.len()..].iter().map(|&i| intents[i])).collect();
        Self { objects, names }
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
