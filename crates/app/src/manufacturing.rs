//! Read-only package review, with independent presentation in each editor pane.
mod files;
mod pane;

use crate::{Dialog, PcbApp, ViewId, canvas_camera::Camera, editor_windows};
pub(super) use files::Files;
use pcbcraft_chrome::workspace::{self, Placement, RibbonTool, ToolIcon};
use pcbcraft_engine::board::fabrication::review::{LayerRole, Package};
use std::sync::Arc;

#[derive(Debug)]
struct Opened {
    name: String,
    package: Package,
}

#[derive(Debug, Default)]
pub(super) struct State {
    binding: Arc<()>,
    opened: Option<Arc<Opened>>,
    failure: Option<String>,
    visible: Vec<bool>,
    camera: Camera,
    highlighted: Option<pcbcraft_engine::schematic::Point>,
    tab: usize,
}

impl State {
    /// Duplicate only the completed package and its presentation, with a fresh file owner.
    pub(super) fn fork(&mut self) -> Self {
        Self { opened: self.opened.clone(), visible: self.visible.clone(), camera: self.camera.fork_navigation(), tab: self.tab, ..Self::default() }
    }

    /// Fit the current package on the next paint using the shared editor camera.
    pub(super) fn fit(&mut self) {
        self.camera.request_fit();
    }

    /// Replace the package atomically after validation and seed the top-side overlay.
    fn install(&mut self, opened: Arc<Opened>) {
        self.visible = opened
            .package
            .layers
            .iter()
            .map(|layer| {
                matches!(
                    layer.role,
                    LayerRole::TopCopper | LayerRole::TopSilkscreen | LayerRole::Profile | LayerRole::PlatedDrill | LayerRole::NonPlatedDrill
                )
            })
            .collect();
        self.opened = Some(opened);
        self.highlighted = None;
        self.failure = None;
        self.fit();
    }

    /// Set visibility by exact archive member name, independently in each pane.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn set_layer(&mut self, name: &str, visible: bool) -> Result<(), String> {
        let index = self
            .opened
            .as_ref()
            .and_then(|opened| opened.package.layers.iter().position(|layer| layer.name == name))
            .ok_or("No fabrication layer with that filename is open in this pane.")?;
        self.visible[index] = visible;
        Ok(())
    }

    /// Expose the same verified drill tool counts, findings and layer visibility as the pane.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn inspect(&self, files: &Files, id: ViewId) -> serde_json::Value {
        let package = self.opened.as_ref().map(|opened| {
            serde_json::json!({
                "filename": opened.name,
                "generator_version": opened.package.generator_version,
                "layers": opened.package.layers.iter().zip(&self.visible).map(|(layer, visible)| serde_json::json!({
                    "filename": layer.name, "role": layer.role, "visible": visible, "drill_tools": layer.tools,
                })).collect::<Vec<_>>(),
                "findings": opened.package.findings,
            })
        });
        serde_json::json!({"package": package, "error": self.failure, "reading": files.reading(id, &self.binding)})
    }
}

impl PcbApp {
    /// Place a service view through the same workspace choices as Parts Library.
    pub(super) fn place_manufacturing(&mut self, ctx: &egui::Context, placement: Placement) -> Result<(), String> {
        let source = self.views.active.id();
        let (id, host) = match placement {
            Placement::Here => (source, self.views.host),
            Placement::Beside => (self.views.open_beside(source).ok_or("Could not create a Manufacturing pane.")?, self.views.host),
            Placement::Window if !crate::IN_BROWSER => {
                let host = self.views.open_window(source).ok_or("Could not create a Manufacturing window.")?;
                (self.views.hosts[&host].focused, host)
            }
            Placement::Window => return Err("Native windows are unavailable in the browser.".into()),
        };
        let view = self.views.view_mut(id).ok_or("Manufacturing pane is unavailable.")?;
        view.release_input();
        view.definition = None;
        view.library = None;
        view.render = None;
        view.manufacturing.get_or_insert_with(State::default);
        self.library_editors.focus(None)?;
        self.home = false;
        if host == self.views.host {
            self.activate_editor(ctx, id);
        }
        ctx.send_viewport_cmd_to(editor_windows::viewport(host), egui::ViewportCommand::Focus);
        self.repaint_editors(ctx);
        Ok(())
    }

    /// Open a ZIP for the requested review pane, leaving its existing package visible until success.
    pub(super) fn open_fabrication_package(&mut self, ctx: &egui::Context, id: ViewId, path: Option<std::path::PathBuf>) -> Result<(), String> {
        if self.home || self.project_action_pending() || !matches!(self.dialog, Dialog::None) {
            return Err("Finish the current project operation before opening a fabrication ZIP.".into());
        }
        let state = self.views.view_mut(id).and_then(|view| view.manufacturing.as_mut()).ok_or("Choose a Manufacturing pane.")?;
        let directory = self.files.project_directory(&self.session).map(std::path::Path::to_owned);
        self.manufacturing_files.open(ctx, id, &state.binding, path, directory)?;
        state.failure = None;
        Ok(())
    }

    /// Install a result only in the review that started it, including when that pane is inactive.
    pub(super) fn poll_manufacturing(&mut self, ctx: &egui::Context) {
        let Some((id, binding, completion)) = self.manufacturing_files.poll() else { return };
        let Some(state) = self.views.view_mut(id).and_then(|view| view.manufacturing.as_mut()).filter(|state| Arc::ptr_eq(&state.binding, &binding))
        else {
            return;
        };
        match completion {
            Ok(Some(opened)) => state.install(opened),
            Ok(None) => {}
            Err(error) => state.failure = Some(error),
        }
        self.repaint_editors(ctx);
    }

    /// Paint the focused review with the standard shell and pane host, without project edit keys.
    pub(super) fn manufacturing_frame(&mut self, ui: &mut egui::Ui, integrated: bool) {
        let ctx = ui.ctx().clone();
        let enabled = matches!(self.dialog, Dialog::None) && !self.files.busy() && !self.project_action_pending();
        let previous = std::mem::replace(&mut self.integrated_titlebar, integrated);
        self.app_bar(ui, enabled);
        self.integrated_titlebar = previous;
        ui.add_enabled_ui(enabled, |ui| {
            if self.home && self.views.host == self.views.main {
                self.status(ui);
                self.home_page(ui);
                return;
            }
            self.manufacturing_ribbon(ui);
            let area = ui.available_rect_before_wrap();
            self.editor_workspace(ui, area);
            #[cfg(not(target_arch = "wasm32"))]
            if self.views.host == self.views.main {
                self.host_assistant(&ctx, Some(area), enabled, self.session.project().is_some());
            }
        });
        let dialog_host = self.arrangement_dialog_host().or_else(|| self.rule_dialog_host()).unwrap_or(self.views.main);
        if self.views.host == dialog_host {
            self.project_dialogs(&ctx);
        }
    }

    /// Use the standard picker and layout tools in the read-only workspace ribbon.
    fn manufacturing_ribbon(&mut self, ui: &mut egui::Ui) {
        let entries = self.workspace_entries();
        let id = self.views.active.id();
        let Some(state) = self.views.active.manufacturing.as_mut() else { return };
        let loaded = state.opened.is_some();
        let mirrored = state.camera.mirrored();
        let mut tab = state.tab;
        let docked = self.views.host == self.views.main;
        let ready = !self.manufacturing_files.busy();
        let latest = self.fabrication_exports.latest(&self.session).map(std::path::Path::to_owned);
        let mut open = vec![tool(ToolIcon::Shared("folder"), "Open ZIP", "Open a PCBcraft fabrication ZIP", ready)];
        if latest.is_some() {
            open.push(tool(ToolIcon::Outputs, "Last export", "Open the last saved fabrication ZIP", ready));
        }
        let view = [
            tool(ToolIcon::Fit, "Fit", "Fit fabrication artwork", loaded),
            RibbonTool {
                active: mirrored,
                menu_check: Some(pcbcraft_chrome::menu::CheckKind::Toggle),
                ..tool(ToolIcon::FlipView, "Flip view", "View from the bottom", loaded)
            },
        ];
        let layout = [
            tool(ToolIcon::SplitView, "Split", "Split Manufacturing workspace", true),
            tool(ToolIcon::EditorWindow, if docked { "Detach" } else { "Dock" }, "Detach or dock Manufacturing pane", !crate::IN_BROWSER),
            tool(ToolIcon::EditorWindow, "New window", "Open Manufacturing in a new window", !crate::IN_BROWSER),
        ];
        let (mut package_action, mut view_action, mut layout_action) = (None, None, None);
        let response = workspace::ribbon(
            ui,
            "Manufacturing",
            &["Review", "View"],
            &mut tab,
            workspace::Workspaces { entries: &entries, ..crate::WORKSPACES },
            &[],
            |ui, shown| {
                if shown == 0 {
                    package_action = workspace::ribbon_group(ui, "Package", &open);
                }
                view_action = workspace::ribbon_group(ui, "Canvas", &view);
                if shown == 1 {
                    layout_action = workspace::ribbon_group(ui, "Layout", &layout);
                }
            },
        );
        state.tab = tab;
        match view_action {
            Some(0) => state.fit(),
            Some(1) => state.camera.set_mirrored(!mirrored),
            _ => {}
        }
        if let Some(action) = layout_action {
            match action {
                0 => self.split_editors(ui.ctx(), !self.views.is_split()),
                1 => self.toggle_editor_window(ui.ctx()),
                _ => self.open_editor_window(ui.ctx()),
            }
        }
        if let Some(action) = package_action
            && let Err(error) = self.open_fabrication_package(ui.ctx(), id, if action == 0 { None } else { latest })
            && let Some(state) = self.views.active.manufacturing.as_mut()
        {
            state.failure = Some(error);
        }
        if let Some(pick) = response.workspace {
            self.pick_workspace(ui.ctx(), pick);
        }
    }
}

fn tool<'a>(icon: ToolIcon, label: &'a str, description: &'static str, enabled: bool) -> RibbonTool<'a> {
    RibbonTool { icon, label, description, shortcut: "", enabled, active: false, menu_check: None }
}
