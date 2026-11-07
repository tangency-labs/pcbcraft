//! Versioned, bounded presentation records; none of these records owns electrical data.

use crate::{editor_view::ViewState, parts_browser::PartsState};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum SavedLayout {
    View { view: usize },
    Tabs { views: Vec<usize>, active: usize },
    Split { first: Box<Self>, second: Box<Self>, ratio: f32 },
}

impl SavedLayout {
    /// Validate tree depth and geometry while enumerating document-view references.
    pub(crate) fn members(&self, depth: usize, result: &mut Vec<usize>) -> bool {
        if depth > 12 || result.len() >= 128 {
            return false;
        }
        match self {
            Self::View { view } => {
                result.push(*view);
                true
            }
            Self::Tabs { views, active } => {
                if views.len() < 2 || result.len() + views.len() > 128 || !views.contains(active) {
                    return false;
                }
                result.extend(views);
                true
            }
            Self::Split { first, second, ratio } => {
                (0.25..=0.75).contains(ratio) && first.members(depth + 1, result) && second.members(depth + 1, result)
            }
        }
    }

    /// Focus must address the active document of a pane, never one of its hidden tabs.
    pub(crate) fn visible(&self, view: usize) -> bool {
        match self {
            Self::View { view: current } => *current == view,
            Self::Tabs { active, .. } => *active == view,
            Self::Split { first, second, .. } => first.visible(view) || second.visible(view),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PanelState {
    #[serde(default = "properties_default")]
    pub properties: bool,
    pub browser: [bool; 2],
    pub parts: PartsState,
    pub nets: bool,
    /// Stable presentation slots: Capture/Layout=0, legacy View=1, Analyze/Libraries/Manage=2..4.
    pub ribbon: usize,
}

impl Default for PanelState {
    fn default() -> Self {
        Self { properties: properties_default(), browser: [true, false], parts: PartsState::default(), nets: false, ribbon: 0 }
    }
}

/// Arrangements saved before Properties existed show it, as a fresh editor does.
fn properties_default() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WindowBounds {
    pub position: [f32; 2],
    pub size: [f32; 2],
    pub maximized: bool,
}

impl WindowBounds {
    /// Reject impossible geometry before native window commands are issued.
    pub(crate) fn valid(&self) -> bool {
        self.position.iter().all(|n| n.is_finite() && n.abs() <= 100_000.0) && self.size.iter().all(|n| (100.0..=32_768.0).contains(n))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SavedHost {
    pub layout: SavedLayout,
    pub focused: usize,
    pub parked: Vec<usize>,
    pub collapsed: Option<SavedLayout>,
    pub panels: PanelState,
    pub bounds: Option<WindowBounds>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Snapshot {
    pub views: Vec<ViewState>,
    /// The first host is the project-controls owner. Persistent references are local indices,
    /// never runtime IDs, which are freshly allocated on every activation.
    pub hosts: Vec<SavedHost>,
    pub primary: usize,
}

impl Snapshot {
    /// Reject duplicate placements, orphan views, invalid focus and excessive state atomically.
    pub(crate) fn valid(&self) -> bool {
        if self.hosts.is_empty()
            || self.hosts.len() > 16
            || self.views.is_empty()
            || self.views.len() > 128
            || self.primary >= self.views.len()
            || !self.views.iter().all(ViewState::valid)
        {
            return false;
        }
        let mut all = BTreeSet::new();
        for host in &self.hosts {
            let mut placed = Vec::new();
            if !host.layout.members(0, &mut placed)
                || !host.layout.visible(host.focused)
                || !host.panels.parts.valid()
                || host.panels.ribbon > 4
                || host.bounds.as_ref().is_some_and(|bounds| !bounds.valid())
            {
                return false;
            }
            placed.extend(&host.parked);
            if placed.iter().any(|id| *id >= self.views.len() || !all.insert(*id)) {
                return false;
            }
            if let Some(collapsed) = &host.collapsed {
                let mut members = Vec::new();
                if !collapsed.members(0, &mut members)
                    || members.len() != placed.len()
                    || members.into_iter().collect::<BTreeSet<_>>() != placed.into_iter().collect()
                {
                    return false;
                }
            }
        }
        all.len() == self.views.len()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Arrangement {
    pub id: u64,
    pub name: String,
    pub snapshot: Snapshot,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectArrangements {
    pub version: u32,
    pub active: u64,
    pub arrangements: Vec<Arrangement>,
}

impl ProjectArrangements {
    /// New projects start with a Views arrangement; restored records retain their saved names.
    pub(crate) fn new(snapshot: Snapshot) -> Self {
        Self { version: 1, active: 1, arrangements: vec![Arrangement { id: 1, name: "Views".into(), snapshot }] }
    }

    /// Validate the complete record before restoring any of its windows.
    pub(crate) fn valid(&self) -> bool {
        let mut ids = BTreeSet::new();
        self.version == 1
            && !self.arrangements.is_empty()
            && self.arrangements.len() <= 32
            && self
                .arrangements
                .iter()
                .all(|arrangement| arrangement.id > 0 && ids.insert(arrangement.id) && valid_name(&arrangement.name) && arrangement.snapshot.valid())
            && ids.contains(&self.active)
    }
}

/// Names label arrangements without acting as identity or accepting multiline/control text.
pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.trim() == name && name.chars().count() <= 80 && !name.chars().any(char::is_control)
}
