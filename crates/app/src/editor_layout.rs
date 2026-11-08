//! Runtime window layouts reference view IDs; they never own project data or cameras.

use egui::{Rect, Ui};
use pcbcraft_chrome::editor;

use crate::ViewId;

/// Stable host identity. Zero is the original native window; IDs are never reused.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct HostId(pub(super) u64);

impl HostId {
    pub(super) const ROOT: Self = Self(0);
}

/// A window can contain any number of panes, with independent divider positions. `Tabs`
/// retains the saved-format name for a pane's cached documents; it has no visible tab strip.
#[derive(Clone, Debug)]
pub(super) enum Layout {
    View(ViewId),
    Tabs { views: Vec<ViewId>, active: ViewId },
    Split { first: Box<Self>, second: Box<Self>, ratio: f32 },
}

impl Layout {
    /// Replace runtime identities with snapshot-local references for persistence.
    pub(super) fn saved_state(&self, indices: &std::collections::BTreeMap<ViewId, usize>) -> Option<crate::arrangements::SavedLayout> {
        use crate::arrangements::SavedLayout;
        Some(match self {
            Self::View(id) => SavedLayout::View { view: *indices.get(id)? },
            Self::Tabs { views, active } => SavedLayout::Tabs {
                views: views.iter().map(|id| indices.get(id).copied()).collect::<Option<Vec<_>>>()?,
                active: *indices.get(active)?,
            },
            Self::Split { first, second, ratio } => {
                SavedLayout::Split { first: Box::new(first.saved_state(indices)?), second: Box::new(second.saved_state(indices)?), ratio: *ratio }
            }
        })
    }

    /// Rebuild a validated tree with fresh runtime identities.
    pub(super) fn from_state(state: &crate::arrangements::SavedLayout, ids: &[ViewId]) -> Option<Self> {
        use crate::arrangements::SavedLayout;
        Some(match state {
            SavedLayout::View { view } => Self::View(*ids.get(*view)?),
            SavedLayout::Tabs { views, active } => {
                Self::Tabs { views: views.iter().map(|index| ids.get(*index).copied()).collect::<Option<Vec<_>>>()?, active: *ids.get(*active)? }
            }
            SavedLayout::Split { first, second, ratio } => {
                Self::Split { first: Box::new(Self::from_state(first, ids)?), second: Box::new(Self::from_state(second, ids)?), ratio: *ratio }
            }
        })
    }

    /// Enumerate leaves in stable screen order, without depending on the focused view.
    pub(super) fn views(&self) -> Vec<ViewId> {
        match self {
            Self::View(id) => vec![*id],
            Self::Tabs { views, .. } => views.clone(),
            Self::Split { first, second, .. } => first.views().into_iter().chain(second.views()).collect(),
        }
    }

    /// Enumerate the active document in each pane, excluding inactive document tabs.
    pub(super) fn visible_views(&self) -> Vec<ViewId> {
        match self {
            Self::View(id) => vec![*id],
            Self::Tabs { active, .. } => vec![*active],
            Self::Split { first, second, .. } => first.visible_views().into_iter().chain(second.visible_views()).collect(),
        }
    }

    /// Find the whole pane containing a view, preserving tab order and its active document.
    pub(super) fn group(&self, id: ViewId) -> Option<&Self> {
        match self {
            Self::View(current) => (*current == id).then_some(self),
            Self::Tabs { views, .. } => views.contains(&id).then_some(self),
            Self::Split { first, second, .. } => first.group(id).or_else(|| second.group(id)),
        }
    }

    /// Move a whole pane to another pane's slot, shifting intervening panes in screen order.
    /// Split structure/ratios, cached documents and active documents are retained. Unknown IDs
    /// and two IDs in the same pane do nothing; two visible panes exchange places.
    pub(super) fn reorder(&mut self, source: ViewId, target: ViewId) -> bool {
        let mut groups = Vec::new();
        self.collect_groups(&mut groups);
        let Some(from) = groups.iter().position(|group| group.views().contains(&source)) else { return false };
        let Some(to) = groups.iter().position(|group| group.views().contains(&target)) else { return false };
        if from == to {
            return false;
        }
        let moved = groups.remove(from);
        groups.insert(to, moved);
        self.replace_groups(&mut groups.into_iter());
        true
    }

    /// Collect pane leaves in the same traversal order used by layout and hit testing.
    fn collect_groups(&self, groups: &mut Vec<Self>) {
        match self {
            Self::Split { first, second, .. } => {
                first.collect_groups(groups);
                second.collect_groups(groups);
            }
            _ => groups.push(self.clone()),
        }
    }

    /// Fill existing leaf slots from an equally sized pane sequence, retaining every divider.
    fn replace_groups(&mut self, groups: &mut std::vec::IntoIter<Self>) {
        match self {
            Self::Split { first, second, .. } => {
                first.replace_groups(groups);
                second.replace_groups(groups);
            }
            _ => {
                if let Some(group) = groups.next() {
                    *self = group;
                }
            }
        }
    }

    /// Activate a document only within its existing pane; unrelated panes retain their tabs.
    pub(super) fn activate(&mut self, id: ViewId) {
        match self {
            Self::Tabs { views, active } if views.contains(&id) => *active = id,
            Self::Split { first, second, .. } => {
                first.activate(id);
                second.activate(id);
            }
            _ => {}
        }
    }

    /// Append and activate a document tab in the addressed pane without making another split.
    pub(super) fn insert_tab(&mut self, source: ViewId, id: ViewId) -> bool {
        match self {
            Self::View(current) if *current == source => {
                *self = Self::Tabs { views: vec![source, id], active: id };
                true
            }
            Self::Tabs { views, active } if views.contains(&source) => {
                views.push(id);
                *active = id;
                true
            }
            Self::Split { first, second, .. } => first.insert_tab(source, id) || second.insert_tab(source, id),
            _ => false,
        }
    }

    /// Insert a new single-document pane beside the addressed pane.
    pub(super) fn insert(&mut self, beside: ViewId, id: ViewId) -> bool {
        self.insert_group(beside, &Self::View(id))
    }

    /// Insert a whole pane, including its cached document navigation, beside the addressed pane.
    /// Existing placements and divider ratios stay intact; an unknown source changes nothing.
    pub(super) fn insert_group(&mut self, beside: ViewId, group: &Self) -> bool {
        match self {
            Self::View(current) if *current == beside => {
                *self = Self::Split { first: Box::new(self.clone()), second: Box::new(group.clone()), ratio: 0.5 };
                true
            }
            Self::Tabs { views, .. } if views.contains(&beside) => {
                *self = Self::Split { first: Box::new(self.clone()), second: Box::new(group.clone()), ratio: 0.5 };
                true
            }
            Self::Split { first, second, .. } => first.insert_group(beside, group) || second.insert_group(beside, group),
            _ => false,
        }
    }

    /// Remove one leaf and collapse its empty branch. An empty window has no layout.
    pub(super) fn remove(self, id: ViewId) -> Option<Self> {
        match self {
            Self::View(current) => (current != id).then_some(Self::View(current)),
            Self::Tabs { mut views, mut active } => {
                let index = views.iter().position(|current| *current == id).unwrap_or(0);
                views.retain(|current| *current != id);
                if active == id {
                    active = *views.get(index.min(views.len().saturating_sub(1)))?;
                }
                match views.as_slice() {
                    [] => None,
                    [only] => Some(Self::View(*only)),
                    _ => Some(Self::Tabs { views, active }),
                }
            }
            Self::Split { first, second, ratio } => match (first.remove(id), second.remove(id)) {
                (Some(first), Some(second)) => Some(Self::Split { first: Box::new(first), second: Box::new(second), ratio }),
                (remaining, None) | (None, remaining) => remaining,
            },
        }
    }

    /// Compute each pane using shared chrome dividers, scoped to the stable leaf identities.
    pub(super) fn panes(&mut self, ui: &mut Ui) -> Vec<(ViewId, Rect)> {
        match self {
            Self::View(id) => vec![(*id, ui.available_rect_before_wrap())],
            Self::Tabs { active, .. } => vec![(*active, ui.available_rect_before_wrap())],
            Self::Split { first, second, ratio } => {
                let rects = editor::split(ui, ratio);
                let mut panes = Vec::new();
                for (index, (layout, rect)) in [(first, rects[0]), (second, rects[1])].into_iter().enumerate() {
                    ui.scope_builder(egui::UiBuilder::new().id_salt(index).max_rect(rect), |ui| {
                        ui.set_clip_rect(rect);
                        panes.extend(layout.panes(ui));
                    });
                }
                panes
            }
        }
    }
}

/// Focus and hidden split views belong to their host, independently of other native windows.
#[derive(Debug)]
pub(super) struct ViewHost {
    pub(super) layout: Layout,
    pub(super) focused: ViewId,
    pub(super) parked: Vec<ViewId>,
    pub(super) collapsed: Option<Layout>,
    pub(super) rects: Vec<(ViewId, Rect)>,
}

impl ViewHost {
    /// Start a host with one visible view and no cached pane geometry.
    pub(super) fn new(id: ViewId) -> Self {
        Self { layout: Layout::View(id), focused: id, parked: Vec::new(), collapsed: None, rects: Vec::new() }
    }

    /// Whether this host currently displays more than one pane.
    pub(super) fn split(&self) -> bool {
        matches!(self.layout, Layout::Split { .. })
    }
}
