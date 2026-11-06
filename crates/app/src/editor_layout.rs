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

/// A window can contain any number of views, with independent divider positions.
#[derive(Debug)]
pub(super) enum Layout {
    View(ViewId),
    Split { first: Box<Self>, second: Box<Self>, ratio: f32 },
}

impl Layout {
    /// Enumerate leaves in stable screen order, without depending on the focused view.
    pub(super) fn views(&self) -> Vec<ViewId> {
        match self {
            Self::View(id) => vec![*id],
            Self::Split { first, second, .. } => first.views().into_iter().chain(second.views()).collect(),
        }
    }

    /// Insert a view beside the addressed leaf, preserving all other placements and dividers.
    pub(super) fn insert(&mut self, beside: ViewId, id: ViewId) -> bool {
        match self {
            Self::View(current) if *current == beside => {
                *self = Self::Split { first: Box::new(Self::View(beside)), second: Box::new(Self::View(id)), ratio: 0.5 };
                true
            }
            Self::View(_) => false,
            Self::Split { first, second, .. } => first.insert(beside, id) || second.insert(beside, id),
        }
    }

    /// Remove one leaf and collapse its empty branch. An empty window has no layout.
    pub(super) fn remove(self, id: ViewId) -> Option<Self> {
        match self {
            Self::View(current) => (current != id).then_some(Self::View(current)),
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
