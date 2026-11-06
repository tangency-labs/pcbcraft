//! Named project arrangements coordinate presentation and user-local restoration.
//! The engine remains the sole owner of electrical data and undo history.

mod edit;
mod model;
pub(super) use edit::Edit;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
use crate::{Dialog, PcbApp};
use model::{Arrangement, ProjectArrangements, valid_name};
pub(super) use model::{PanelState, SavedHost, SavedLayout, Snapshot, WindowBounds};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) const STORAGE_KEY: &str = "pcbcraft.project_arrangements";
const MAX_STORAGE: usize = 8 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Store {
    version: u32,
    projects: BTreeMap<String, Value>,
}

impl Default for Store {
    fn default() -> Self {
        Self { version: 1, projects: BTreeMap::new() }
    }
}

/// Actions are deferred until every native host has returned its render slot.
#[derive(Clone, Debug)]
pub(super) enum Action {
    Select(u64),
    Create { name: String, single_view: bool },
    Rename(String),
    Remove,
}

#[derive(Debug, Default)]
pub(super) struct Arrangements {
    store: Store,
    current: Option<ProjectArrangements>,
    key: Option<String>,
    project: Option<String>,
    opening: u64,
    frozen: bool,
    protected_record: bool,
    snapshot_limited: bool,
    pub(super) dirty: bool,
    pub(super) last_flush: f64,
    pending: Option<(String, u64, u64, Action)>,
    pub(super) notice: Option<String>,
}

impl Arrangements {
    /// Preserve malformed/newer storage instead of replacing it with startup defaults.
    pub(super) fn from_storage(text: Option<String>) -> Self {
        let Some(text) = text else {
            return Self::default();
        };
        let store = (text.len() <= MAX_STORAGE).then(|| serde_json::from_str::<Store>(&text).ok()).flatten();
        match store.filter(|store| store.version == 1 && store.projects.len() <= 128) {
            Some(store) => Self { store, ..Self::default() },
            None => Self {
                frozen: true,
                notice: Some("Saved arrangements could not be read; the stored record has been preserved.".into()),
                ..Self::default()
            },
        }
    }

    /// Supply a bounded store for eframe's native/browser persistence backend.
    pub(super) fn storage_text(&self) -> Option<String> {
        if self.frozen {
            return None;
        }
        serde_json::to_string(&self.store).ok().filter(|text| text.len() <= MAX_STORAGE)
    }

    /// Labels and identities for the title bar; a project name is not an arrangement identity.
    pub(super) fn tabs(&self) -> Vec<(u64, String)> {
        self.current.as_ref().map_or_else(Vec::new, |record| record.arrangements.iter().map(|a| (a.id, a.name.clone())).collect())
    }

    /// Return the selected arrangement, or zero while no project has been bound.
    pub(super) fn active(&self) -> u64 {
        self.current.as_ref().map_or(0, |record| record.active)
    }

    /// Publish only valid records, retaining unrelated projects and unsupported records.
    fn remember(&mut self) {
        if self.frozen || self.protected_record {
            return;
        }
        let (Some(key), Some(record)) = (&self.key, &self.current) else {
            return;
        };
        if !record.valid() {
            return;
        }
        if !self.store.projects.contains_key(key) && self.store.projects.len() >= 128 {
            self.notice = Some("Arrangement storage is full; this project's layout is retained only for this session.".into());
            return;
        }
        if let Ok(value) = serde_json::to_value(record)
            && self.store.projects.get(key) != Some(&value)
        {
            let previous = self.store.projects.insert(key.clone(), value);
            if self.storage_text().is_none() {
                match previous {
                    Some(value) => {
                        self.store.projects.insert(key.clone(), value);
                    }
                    None => {
                        self.store.projects.remove(key);
                    }
                }
                self.notice = Some("Arrangement storage is full; recent layout changes could not be retained.".into());
            } else {
                self.dirty = true;
            }
        }
    }
}

impl PcbApp {
    /// Capture the main host and detached hosts after their contextual panels have been returned.
    pub(super) fn arrangement_snapshot(&self) -> Option<Snapshot> {
        let mut state = self.views.saved_state(&self.session)?;
        for (host, saved) in self.views.host_order().into_iter().zip(&mut state.hosts) {
            saved.panels = self.saved_editor_panels(host);
            saved.bounds = self.windows.bounds.get(&host).cloned();
        }
        state.valid().then_some(state)
    }

    /// Keep the current arrangement's latest navigation without changing electrical revision.
    pub(super) fn checkpoint_arrangements(&mut self) {
        let project = self.session.project().map(|project| project.board_target().project.to_string());
        if self.arrangements.project != project || self.arrangements.opening != self.session.history_root_revision() {
            return;
        }
        if self.arrangements.current.is_none() {
            return;
        }
        let Some(snapshot) = self.arrangement_snapshot() else {
            if !self.arrangements.snapshot_limited {
                self.arrangements.notice = Some("This arrangement exceeds the supported layout limits; its last saved layout has been kept.".into());
            }
            self.arrangements.snapshot_limited = true;
            return;
        };
        self.arrangements.snapshot_limited = false;
        if let Some(record) = &mut self.arrangements.current
            && let Some(current) = record.arrangements.iter_mut().find(|a| a.id == record.active)
        {
            if current.snapshot == snapshot {
                return;
            }
            current.snapshot = snapshot;
        }
        self.arrangements.remember();
    }

    /// Adopt saved UI state only after a successful project load. Save As copies the current
    /// arrangement collection to its new location while retaining the original record.
    pub(super) fn sync_arrangements(&mut self, ctx: &egui::Context) {
        let Some(project) = self.session.project().map(|project| project.board_target().project.to_string()) else {
            return;
        };
        let key = self.files.arrangement_location(&self.session).map(|location| format!("{project}:{location}"));
        let opening = self.session.history_root_revision();
        if self.arrangements.project.as_ref() == Some(&project) && self.arrangements.opening == opening {
            if self.arrangements.key != key {
                self.checkpoint_arrangements();
                self.arrangements.key = key;
                self.arrangements.protected_record = false;
                self.arrangements.remember();
            }
            return;
        }
        let Some(fallback) = self.arrangement_snapshot() else {
            return;
        };
        let stored = key.as_ref().and_then(|key| self.arrangements.store.projects.get(key));
        let mut record =
            stored.and_then(|value| serde_json::from_value::<ProjectArrangements>(value.clone()).ok()).filter(ProjectArrangements::valid);
        self.arrangements.protected_record = stored.is_some() && record.is_none();
        if self.arrangements.protected_record {
            self.arrangements.notice =
                Some("This project's saved arrangements are unsupported or invalid; the original record has been preserved.".into());
        }
        if let Some(saved) = &record
            && let Some(active) = saved.arrangements.iter().find(|a| a.id == saved.active)
            && !self.restore_arrangement(&active.snapshot, ctx)
        {
            self.arrangements.protected_record = true;
            self.arrangements.notice =
                Some("The saved arrangement could not be restored on this host; its original record has been preserved.".into());
            record = None;
        }
        self.arrangements.current = Some(record.unwrap_or_else(|| ProjectArrangements::new(fallback)));
        self.arrangements.project = Some(project);
        self.arrangements.opening = opening;
        self.arrangements.key = key;
        self.arrangements.pending = None;
        if matches!(self.dialog, Dialog::Arrangement(_)) {
            self.dialog = Dialog::None;
        }
        self.arrangements.remember();
    }

    /// Queue UI or future live-MCP intents against the current project opening and arrangement.
    pub(super) fn queue_arrangement(&mut self, ctx: &egui::Context, action: Action) {
        if self.home || self.files.busy() || self.project_action_pending() || !matches!(self.dialog, Dialog::None) {
            return;
        }
        if let Some(project) = &self.arrangements.project {
            self.arrangements.pending = Some((project.clone(), self.arrangements.opening, self.arrangements.active(), action));
            ctx.request_repaint_of(egui::ViewportId::ROOT);
        }
    }

    /// Apply one validated arrangement transition after native callbacks finish. Replacement
    /// never invokes project Close, resets history or transfers stale gesture authority.
    pub(super) fn finish_arrangement_action(&mut self, ctx: &egui::Context) {
        let Some((project, opening, active, action)) = self.arrangements.pending.take() else {
            return;
        };
        if self.home
            || self.files.busy()
            || self.project_action_pending()
            || !matches!(self.dialog, Dialog::None)
            || self.arrangements.project.as_ref() != Some(&project)
            || self.arrangements.opening != opening
            || self.arrangements.active() != active
            || self.session.history_root_revision() != opening
            || self.session.project().is_none_or(|current| current.board_target().project.to_string() != project)
        {
            return;
        }
        self.checkpoint_arrangements();
        let Some(mut record) = self.arrangements.current.clone() else {
            return;
        };
        let mut replacement = None;
        match action {
            Action::Select(id) => {
                if id == record.active {
                    return;
                }
                let Some(arrangement) = record.arrangements.iter().find(|a| a.id == id) else {
                    return;
                };
                replacement = Some(arrangement.snapshot.clone());
                record.active = id;
            }
            Action::Create { name, single_view } => {
                if let Some(error) = self.arrangements.name_error(&name, None) {
                    self.message = Some(error.into());
                    return;
                }
                if record.arrangements.len() >= 32 {
                    self.message = Some("An arrangement collection can contain up to 32 arrangements.".into());
                    return;
                }
                let Some(id) = record.arrangements.iter().map(|a| a.id).max().and_then(|id| id.checked_add(1)) else {
                    return;
                };
                let Some(mut snapshot) = self.arrangement_snapshot() else {
                    self.message =
                        Some("This layout exceeds the arrangement limits; close excess views or windows before creating an arrangement.".into());
                    return;
                };
                if single_view {
                    let focused = snapshot.hosts[0].focused;
                    snapshot.views = vec![snapshot.views[focused].clone()];
                    let mut host = snapshot.hosts[0].clone();
                    host.layout = SavedLayout::View { view: 0 };
                    host.focused = 0;
                    host.parked.clear();
                    host.collapsed = None;
                    snapshot.hosts = vec![host];
                    snapshot.primary = 0;
                    replacement = Some(snapshot.clone());
                }
                // Naming a copy of the current setup does not recreate its native windows
                // or invalidate views. Only an explicitly different layout is restored.
                record.arrangements.push(Arrangement { id, name: name.trim().into(), snapshot });
                record.active = id;
            }
            Action::Rename(name) => {
                let name = name.trim();
                if let Some(error) = self.arrangements.name_error(name, Some(record.active)) {
                    self.message = Some(error.into());
                    return;
                }
                if let Some(current) = record.arrangements.iter_mut().find(|a| a.id == record.active) {
                    current.name = name.into();
                }
            }
            Action::Remove => {
                if record.arrangements.len() <= 1 {
                    return;
                }
                record.arrangements.retain(|a| a.id != record.active);
                record.active = record.arrangements[0].id;
                replacement = Some(record.arrangements[0].snapshot.clone());
            }
        }
        if let Some(snapshot) = replacement
            && !self.restore_arrangement(&snapshot, ctx)
        {
            self.message = Some("The arrangement could not be activated; the current views have been kept.".into());
            return;
        }
        self.arrangements.current = Some(record);
        self.arrangements.remember();
        self.repaint_editors(ctx);
    }

    /// Restore the validated host graph and contextual panels, recreating a retired root window.
    fn restore_arrangement(&mut self, snapshot: &Snapshot, ctx: &egui::Context) -> bool {
        if crate::IN_BROWSER && snapshot.hosts.len() > 1 {
            return false;
        }
        if self.views.restore_state(snapshot, &self.session).is_none() {
            return false;
        }
        self.restore_editor_panels(snapshot);
        egui::DragAndDrop::clear_payload(ctx);
        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Visible(true));
        ctx.request_repaint_of(egui::ViewportId::ROOT);
        true
    }
}
