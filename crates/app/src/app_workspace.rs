//! Shell workspace navigation keeps library documents independent of project view targets.
use crate::{Dialog, PcbApp, WORKSPACES, Workspace};
use pcbcraft_chrome::workspace::{self, Placement};
use pcbcraft_engine::authoring::Kind;

impl PcbApp {
    /// Identify the visible workspace in the bound host, including definition authoring.
    pub(super) fn workspace_key(&self) -> &'static str {
        if self.views.active.library.is_some() {
            return "library";
        }
        if let Some(kind) = self.views.active.definition.and_then(|id| self.library_kind(id)) {
            return if kind == Kind::Symbol { "symbol_editor" } else { "footprint_editor" };
        }
        if self.views.active.workspace == Workspace::Board { "board" } else { "schematic" }
    }

    /// Every workspace uses the same contributions and current badge; library documents do
    /// not masquerade as project sheets or physical boards.
    pub(super) fn workspace_entries(&self) -> Vec<workspace::Workspace<'static>> {
        let current = self.workspace_key();
        WORKSPACES
            .entries
            .iter()
            .copied()
            .map(|mut entry| {
                entry.current = entry.id == current && (!matches!(entry.id, "schematic" | "board") || !self.views.active.unavailable());
                entry
            })
            .collect()
    }

    /// Resolve a presentation index through its stable key before invoking shared navigation.
    pub(super) fn pick_workspace(&mut self, ctx: &egui::Context, pick: workspace::WorkspacePick) {
        if let Some(entry) = WORKSPACES.entries.get(pick.index) {
            self.message = self.enter_workspace(ctx, entry.id, pick.placement).err();
        }
    }

    /// GUI and live MCP switch through this boundary. Entering an authoring workspace resumes
    /// its last document, creating one only on first entry. Rejected navigation retains focus.
    /// Both document kinds use the same view/window lifecycle without sharing edit histories.
    pub(super) fn enter_workspace(&mut self, ctx: &egui::Context, key: &str, placement: Placement) -> Result<(), String> {
        if !matches!(self.dialog, Dialog::None) || self.files.busy() || self.project_action_pending() {
            return Err("Finish the current project operation before switching workspaces.".into());
        }
        match key {
            "library" => self.place_library_workspace(ctx, placement)?,
            "symbol_editor" | "footprint_editor" => {
                if placement == Placement::Window && crate::IN_BROWSER {
                    return Err("Native windows are unavailable in the browser.".into());
                }
                let kind = if key == "symbol_editor" { Kind::Symbol } else { Kind::Footprint };
                let preferred = self.preferred_library_source();
                self.library_editors.enter(kind)?;
                let id = self.library_editors.active.ok_or("No library document is open.")?;
                self.library_editors.suggest_destination(id, preferred);
                self.place_library_document(ctx, id, placement)?;
            }
            "schematic" | "board" => {
                if self.session.project().is_none() {
                    return Err("Open or create a project to use this workspace.".into());
                }
                self.library_editors.focus(None)?;
                self.home = false;
                let workspace = if key == "board" { Workspace::Board } else { Workspace::Schematic };
                self.open_workspace(workspace, placement, ctx);
            }
            _ => return Err("Unknown workspace.".into()),
        }
        ctx.request_repaint();
        Ok(())
    }
}
