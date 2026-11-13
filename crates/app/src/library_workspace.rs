//! Library browsing owns per-pane presentation; source changes never transact project data.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
use crate::{Dialog, PcbApp, ViewId};
use pcbcraft_chrome::{
    controls, library_workspace as chrome, menu, shell,
    workspace::{self, Placement, RibbonTool, ToolIcon},
};
use pcbcraft_engine::library::{
    parts::{PartDefinition, Reference},
    sources::{Item, Key, Kind},
};
use serde_json::Value;
#[cfg(not(target_arch = "wasm32"))]
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

/// Disposable navigation is copied when opening beside and retained by its own pane.
#[derive(Clone, Debug, Default)]
pub(super) struct State {
    source: Option<String>,
    kind: usize,
    query: String,
    selected: Option<Key>,
    expanded: BTreeSet<Key>,
    closed: BTreeSet<String>,
    compact: usize,
    tab: usize,
    preview_collapsed: bool,
    browser_collapsed: bool,
    preview_key: Option<Key>,
    symbol_camera: crate::library_preview::Camera,
    footprint_camera: crate::library_preview::Camera,
}
impl State {
    /// Selection changes reset both independent cameras exactly once, including when
    /// a command selects an item and configures its camera in the same request.
    fn sync_preview(&mut self) {
        if self.preview_key != self.selected {
            self.preview_key = self.selected.clone();
            self.symbol_camera = Default::default();
            self.footprint_camera = Default::default();
        }
    }

    /// Inspect stable source/item identities without granting project edit authority.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn inspect(&self) -> Value {
        json!({"source":self.source,"type":self.kind,"query":self.query,"selected":self.selected,"preview_collapsed":self.preview_collapsed,"previews":{"symbol":self.symbol_camera.inspect(),"footprint":self.footprint_camera.inspect()},"ribbon_tab":if self.tab==0 {"manage"}else{"view"}})
    }
}

/// All source actions are shared by ribbon and future host command clients.
#[derive(Clone, Copy, Debug)]
enum Action {
    Add,
    Refresh,
    Relocate,
    Remove,
    Delete,
    Split,
    Detach,
    Window,
}

impl PcbApp {
    /// Existing Sources/Manage libraries navigation replaces the former management modal.
    pub(super) fn open_library_workspace(&mut self, ctx: &egui::Context) {
        self.dialog = Dialog::None;
        self.message = self.enter_workspace(ctx, "library", Placement::Here).err();
    }
    /// Open Library through the normal pane/window ownership path without requiring a project.
    pub(super) fn place_library_workspace(&mut self, ctx: &egui::Context, placement: Placement) -> Result<(), String> {
        if placement == Placement::Window && crate::IN_BROWSER {
            return Err("Native windows are unavailable in the browser.".into());
        }
        let source = self.views.active.id();
        let (view, host) = match placement {
            Placement::Here => (source, self.views.host),
            Placement::Beside => (self.views.open_beside(source).ok_or("Could not create a Library pane")?, self.views.host),
            Placement::Window => {
                let host = self.views.open_window(source).ok_or("Could not create a Library window")?;
                (self.views.hosts[&host].focused, host)
            }
        };
        if let Some(view) = self.views.view_mut(view) {
            view.release_input();
            view.definition = None;
            view.library.get_or_insert_with(State::default);
        }
        self.library_editors.focus(None)?;
        self.home = false;
        if host == self.views.host {
            self.activate_editor(ctx, view);
        }
        ctx.send_viewport_cmd_to(crate::editor_windows::viewport(host), egui::ViewportCommand::Focus);
        self.repaint_editors(ctx);
        Ok(())
    }
    /// A service workspace shares the application shell but does not consume project edit keys.
    pub(super) fn library_workspace_frame(&mut self, ui: &mut egui::Ui, integrated: bool) {
        ui.painter().rect_filled(ui.max_rect(), 0, pcbcraft_chrome::Tokens::get().canvas);
        let ctx = ui.ctx().clone();
        let enabled = matches!(self.dialog, Dialog::None) && !self.files.busy() && !self.project_action_pending();
        let home = std::cell::Cell::new(false);
        let mut preferences = false;
        let mut help = None;
        shell::app_bar(
            ui,
            Some(("Parts Library", false)),
            integrated,
            enabled,
            |ui| {
                home.set(menu::quick_action(ui, menu::MenuIcon::Home, "Home", "", true).clicked());
            },
            |ui| {
                menu::file(ui, |ui| {
                    if pcbcraft_chrome::MenuItem::new("Home").show(ui) {
                        home.set(true);
                    }
                });
            },
            |_| {},
            |ui, _| {
                help = menu::help(ui, crate::help_menu).flatten();
                preferences = menu::quick_action(ui, menu::MenuIcon::Settings, "Preferences", "", true).clicked();
            },
        );
        if home.get() {
            self.show_home();
        }
        if preferences {
            self.open_preferences();
        }
        if let Some(help) = help {
            self.help_action(&ctx, help);
        }
        ui.add_enabled_ui(enabled, |ui| {
            self.library_workspace_ribbon(ui);
            let loaded = self.session.libraries().inspect();
            let mut sources: BTreeSet<String> =
                loaded.as_array().into_iter().flatten().filter_map(|s| s["source"].as_str().map(str::to_owned)).collect();
            sources.extend(self.library_sources.registered.iter().map(|r| r.source.clone()));
            sources.extend(self.library_sources.statuses.keys().cloned());
            shell::status_bar(
                ui,
                |ui| {
                    ui.label(format!("Libraries · {} {}", sources.len(), if sources.len() == 1 { "library" } else { "libraries" }));
                },
                |_| {},
            );
            if let Some(message) = self.message.as_ref() {
                ui.label(egui::RichText::new(message).color(pcbcraft_chrome::Tokens::get().error));
            }
            let area = ui.available_rect_before_wrap();
            self.editor_workspace(ui, area);
        });
        let dialog_host = self.arrangement_dialog_host().unwrap_or(self.views.main);
        if self.views.host == dialog_host {
            self.project_dialogs(&ctx);
        }
    }
    /// Shared ribbon metrics and source-qualified availability match the existing workspaces.
    fn library_workspace_ribbon(&mut self, ui: &mut egui::Ui) {
        let entries = self.workspace_entries();
        let mut tab = self.views.active.library.as_ref().map_or(0, |s| s.tab);
        let source = self.views.active.library.as_ref().and_then(|s| s.source.as_deref());
        let loaded = self.session.libraries().inspect();
        let builtin = source.is_some_and(|id| loaded.as_array().into_iter().flatten().any(|s| s["source"] == id && s["builtin"] == true));
        let folder = source.is_some_and(|id| self.library_sources.registered.iter().any(|r| r.source == id && !r.path.is_empty()));
        let writable = source.is_some_and(|id| self.library_sources.registered.iter().any(|r| r.source == id && r.writable));
        let external = source.is_some() && !builtin;
        let idle = !self.library_sources.busy() && !self.library_mutation_busy();
        let tools = [
            (
                Action::Add,
                ToolIcon::LibraryFolder(workspace::LibraryFolderAction::Add),
                "Add library",
                "Add library",
                idle && self.library_sources.can_add(&self.session),
            ),
            (
                Action::Refresh,
                ToolIcon::LibraryRefresh,
                "Refresh",
                if crate::IN_BROWSER { "Reopen library" } else { "Refresh library" },
                external && idle,
            ),
            (
                Action::Relocate,
                ToolIcon::LibraryFolder(workspace::LibraryFolderAction::Relocate),
                "Relocate",
                "Relocate library",
                external && folder && idle && !crate::IN_BROWSER,
            ),
            (
                Action::Remove,
                ToolIcon::LibraryFolder(workspace::LibraryFolderAction::Remove),
                "Remove",
                "Remove library registration",
                external && idle,
            ),
            (Action::Delete, ToolIcon::Delete, "Delete", "Delete library folder", external && folder && writable && idle && !crate::IN_BROWSER),
        ];
        let layouts = [
            (Action::Split, ToolIcon::SplitView, "Split", "Split Library workspace", true),
            (
                Action::Detach,
                ToolIcon::EditorWindow,
                if self.views.host == self.views.main { "Detach" } else { "Dock" },
                "Detach or dock Library pane",
                !crate::IN_BROWSER,
            ),
            (Action::Window, ToolIcon::EditorWindow, "New window", "New Library window", !crate::IN_BROWSER),
        ];
        let mut action = None;
        let mut acquisition = None;
        let response = workspace::ribbon(
            ui,
            "Parts Library",
            &["Manage", "View"],
            &mut tab,
            workspace::Workspaces { entries: &entries, ..crate::WORKSPACES },
            &[],
            |ui, shown| {
                let rows = if shown == 0 { &tools[..] } else { &layouts[..] };
                let entries: Vec<_> = rows
                    .iter()
                    .map(|(_, icon, label, description, enabled)| RibbonTool {
                        icon: *icon,
                        label,
                        description,
                        shortcut: "",
                        enabled: *enabled,
                        active: false,
                        menu_check: None,
                    })
                    .collect();
                let mut add_triggered = false;
                if let Some(index) =
                    workspace::ribbon_group_with_popups(ui, if shown == 0 { "Sources" } else { "Layout" }, &entries, &[], |_ui, index, trigger| {
                        if shown == 0 && index == 0 {
                            add_triggered = trigger.clicked();
                            egui::Popup::menu(trigger).frame(menu::frame()).show(|ui| {
                                let (folder, bundle) = chrome::add_choices(ui);
                                if folder || bundle {
                                    acquisition = Some(folder);
                                }
                            });
                        }
                    })
                {
                    // Add opens its anchored menu directly; caption-menu activation uses
                    // the command's chooser fallback below.
                    if shown != 0 || index != 0 || !add_triggered {
                        action = Some(rows[index].0);
                    }
                }
                if shown == 0 {
                    for (title, labels) in [
                        (
                            "New",
                            &[
                                ("Library", ToolIcon::Shared("library"), "New library · Planned"),
                                ("Symbol", ToolIcon::Symbol, "New symbol · Planned"),
                                ("Footprint", ToolIcon::Footprint, "New footprint · Planned"),
                                ("Generic part", ToolIcon::Bom, "New generic part · Planned"),
                            ][..],
                        ),
                        (
                            "Item",
                            &[
                                ("Edit", ToolIcon::Shared("edit"), "Edit item · Planned"),
                                ("Edit copy", ToolIcon::Shared("copy"), "Edit copy · Planned"),
                                ("Compare", ToolIcon::Shared("compare"), "Compare items · Planned"),
                            ][..],
                        ),
                        ("Share", &[("Export", ToolIcon::Shared("export"), "Export library item · Planned")][..]),
                    ] {
                        workspace::ribbon_separator(ui);
                        let planned: Vec<_> = labels
                            .iter()
                            .map(|(label, icon, description)| RibbonTool {
                                icon: *icon,
                                label,
                                description,
                                shortcut: "",
                                enabled: false,
                                active: false,
                                menu_check: None,
                            })
                            .collect();
                        workspace::ribbon_group(ui, title, &planned);
                    }
                }
            },
        );
        if let Some(state) = self.views.active.library.as_mut() {
            state.tab = tab;
        }
        if let Some(pick) = response.workspace {
            self.pick_workspace(ui.ctx(), pick);
        }
        if let Some(folder) = acquisition {
            self.message = self.acquire_library(ui.ctx(), None, folder).err();
        }
        if let Some(action) = action {
            let source = self.views.active.library.as_ref().and_then(|s| s.source.clone());
            self.message = self.library_source_action(ui.ctx(), action, source).err();
        }
    }
    /// Execute a source action after the presentation pass, with availability checked again.
    fn library_source_action(&mut self, ctx: &egui::Context, action: Action, source: Option<String>) -> Result<(), String> {
        use crate::editor_windows::ViewAction;
        if matches!(action, Action::Add | Action::Refresh | Action::Relocate | Action::Remove | Action::Delete) {
            if self.library_mutation_busy() || self.library_sources.installing() {
                return Err("A library folder operation is running".into());
            }
            if !matches!(action, Action::Add) {
                let id = source.as_deref().ok_or("Choose a library first")?;
                let loaded = self.session.libraries().inspect();
                if loaded.as_array().into_iter().flatten().any(|s| s["source"] == id && s["builtin"] == true) {
                    return Err("Built-in libraries are read-only".into());
                }
                if !self.library_sources.registered.iter().any(|r| r.source == id)
                    && !self.library_sources.statuses.contains_key(id)
                    && !loaded.as_array().into_iter().flatten().any(|s| s["source"] == id)
                {
                    return Err("Unknown library source".into());
                }
            }
        }
        self.repaint_editors(ctx);
        match action {
            Action::Add => {
                self.dialog = Dialog::AddLibrary(None);
                Ok(())
            }
            Action::Split => {
                self.split_editors(ctx, !self.views.is_split());
                Ok(())
            }
            Action::Detach => {
                self.queue_view_action(
                    ctx,
                    self.views.active.id(),
                    if self.views.host == self.views.main { ViewAction::Detach } else { ViewAction::Dock },
                );
                Ok(())
            }
            Action::Window => {
                self.queue_view_action(ctx, self.views.active.id(), ViewAction::OpenWindow);
                Ok(())
            }
            Action::Remove => {
                if let Some(source) = source {
                    self.library_sources.remove(&source, &mut self.session);
                }
                Ok(())
            }
            Action::Refresh | Action::Relocate => {
                #[cfg(target_arch = "wasm32")]
                if matches!(action, Action::Refresh) {
                    self.dialog = Dialog::AddLibrary(source);
                    return Ok(());
                }
                if let Some(source) = source {
                    let registration = self.library_sources.registered.iter().find(|r| r.source == source).cloned();
                    #[cfg(not(target_arch = "wasm32"))]
                    if matches!(action, Action::Refresh)
                        && let Some(registration) = registration
                    {
                        return self.library_sources.load_path(registration.source, registration.path, ctx, self.session.libraries().generation());
                    }
                    #[cfg(target_arch = "wasm32")]
                    let _ = registration;
                    self.library_sources.pick_folder(source, ctx, self.session.libraries().generation())
                } else {
                    Ok(())
                }
            }
            Action::Delete => self.begin_library_delete(ctx, source.as_deref()),
        }
    }

    /// Live source management invokes the same host operations and confirmations as the ribbon.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn library_workspace_command(&mut self, ctx: &egui::Context, id: &str, params: &Value) -> Result<(), String> {
        let view = ViewId(params["view"].as_u64().unwrap_or(self.views.active.id().0));
        let host = self.views.owner(view).ok_or("Unknown view ID")?;
        if self.views.view(view).is_none_or(|view| view.library.is_none()) {
            return Err("Choose the Library workspace in this pane first".into());
        }
        if id == "app.library_source" {
            let action = match params["action"].as_str() {
                Some("add") => Action::Add,
                Some("refresh") => Action::Refresh,
                Some("relocate") => Action::Relocate,
                Some("remove") => Action::Remove,
                Some("delete") => Action::Delete,
                _ => return Err("Unknown library action".into()),
            };
            let source = params["source"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| self.views.view(view).and_then(|v| v.library.as_ref()).and_then(|s| s.source.clone()));
            self.bind_editor_host(host);
            self.activate_editor(ctx, view);
            let result = self.library_source_action(ctx, action, source);
            self.bind_editor_host(self.views.main);
            return result;
        }
        let source = params["source"].as_str().map(str::to_owned);
        if let Some(source) = source.as_ref() {
            let loaded = self.session.libraries().inspect();
            if !loaded.as_array().into_iter().flatten().any(|s| s["source"] == *source)
                && !self.library_sources.registered.iter().any(|r| r.source == *source)
            {
                return Err("Unknown library source".into());
            }
        }
        let query = params["query"].as_str();
        if query.is_some_and(|q| q.len() > 512) {
            return Err("Library query exceeds 512 bytes".into());
        }
        let state = self.views.view(view).and_then(|v| v.library.as_ref()).ok_or("Library pane is unavailable")?;
        let source = source.or_else(|| state.source.clone());
        let kind = params["type"].as_u64().map(|n| n as usize).unwrap_or(state.kind);
        let selected = if let Some(item) = params["item"].as_str() {
            Some(
                self.session
                    .libraries()
                    .source_items(source.as_deref().unwrap_or(""))
                    .iter()
                    .filter(|i| i.key.id == item && (kind == 0 || kind_index(i.key.kind) == kind))
                    .max_by_key(|i| i.key.revision)
                    .ok_or("Item is unavailable for this source and Type")?
                    .key
                    .clone(),
            )
        } else {
            None
        };
        let mut state = self.views.view(view).and_then(|v| v.library.clone()).ok_or("Library pane is unavailable")?;
        if state.source != source || state.kind != kind {
            state.selected = None;
        }
        state.source = source;
        state.kind = kind;
        if let Some(query) = query {
            state.query = query.into();
        }
        if let Some(selected) = selected {
            state.selected = Some(selected);
        }
        state.sync_preview();
        let camera = match params["preview"].as_str() {
            Some("symbol") => Some(&mut state.symbol_camera),
            Some("footprint") => Some(&mut state.footprint_camera),
            Some(_) => return Err("Choose symbol or footprint preview".into()),
            None => None,
        };
        let zoom = params.get("zoom_percent").map(|v| v.as_f64().map(|z| z as f32 / 100.0).ok_or("Preview zoom must be a number")).transpose()?;
        let pan = match (params.get("pan_x"), params.get("pan_y")) {
            (None, None) => None,
            (Some(x), Some(y)) => {
                Some(egui::vec2(x.as_f64().ok_or("pan_x must be a number")? as f32, y.as_f64().ok_or("pan_y must be a number")? as f32))
            }
            _ => return Err("Preview pan requires both pan_x and pan_y".into()),
        };
        let fit = params.get("fit").map(|v| v.as_bool().ok_or("fit must be a boolean")).transpose()?.unwrap_or(false);
        if let Some(camera) = camera {
            *camera = camera.configured(fit, zoom, pan)?;
        } else if fit || zoom.is_some() || pan.is_some() {
            return Err("Choose a preview before changing its camera".into());
        }
        self.views.view_mut(view).ok_or("Library pane is unavailable")?.library = Some(state);
        self.repaint_editors(ctx);
        Ok(())
    }

    /// Render a pane from its own navigation state, including inactive mixed editor panes.
    pub(super) fn library_workspace_pane(&mut self, ui: &mut egui::Ui, view: ViewId) {
        ui.painter().rect_filled(ui.max_rect(), 0, pcbcraft_chrome::Tokens::get().canvas);
        let Some(mut state) = self.views.view_mut(view).and_then(|v| v.library.take()) else { return };
        let loaded = self.session.libraries().inspect();
        let mut ids: BTreeSet<String> = loaded.as_array().into_iter().flatten().filter_map(|s| s["source"].as_str().map(str::to_owned)).collect();
        ids.extend(self.library_sources.registered.iter().map(|r| r.source.clone()));
        ids.extend(self.library_sources.statuses.keys().cloned());
        if state.source.as_ref().is_none_or(|id| !ids.contains(id)) {
            state.source = ids
                .iter()
                .min_by_key(|id| {
                    let data = loaded.as_array().into_iter().flatten().find(|s| s["source"].as_str() == Some(id.as_str()));
                    (data.is_none_or(|s| s["builtin"] != true), data.and_then(|s| s["name"].as_str()).unwrap_or(id).to_lowercase())
                })
                .cloned();
            state.selected = None;
        }
        // Move the runtime panel out temporarily: Clone intentionally drops live drags.
        let mut preview = self.views.view_mut(view).map(|v| std::mem::take(&mut v.panels[crate::workspace_panels::PROPERTIES])).unwrap_or_default();
        let mut preview_collapsed = state.preview_collapsed;
        let mut browser_collapsed = state.browser_collapsed;
        let previous_compact = state.compact;
        let mut compact = state.compact;
        chrome::workspace(ui, &mut compact, &mut preview, &mut preview_collapsed, &mut browser_collapsed, |ui, region| match region {
            chrome::Region::Browser => self.library_tree(ui, &mut state, &ids, &loaded),
            chrome::Region::Items => self.library_items(ui, &mut state, &loaded),
            chrome::Region::Preview => self.library_item_preview(ui, &mut state),
        });
        if state.compact == previous_compact {
            state.compact = compact;
        }
        state.preview_collapsed = preview_collapsed;
        state.browser_collapsed = browser_collapsed;
        if let Some(target) = self.views.view_mut(view) {
            target.panels[crate::workspace_panels::PROPERTIES] = preview;
            target.library = Some(state);
        }
    }
    /// Two source groups share the pinned tree's fold, count and status presentation.
    fn library_tree(&self, ui: &mut egui::Ui, state: &mut State, ids: &BTreeSet<String>, loaded: &Value) {
        egui::ScrollArea::vertical().id_salt("library_source_tree").auto_shrink([false, false]).show(ui, |ui| {
            for (builtin, title) in [(true, "Built-in"), (false, "Libraries")] {
                let open = !state.closed.contains(title);
                let (_, fold) = chrome::tree(
                    ui,
                    ui.make_persistent_id(title),
                    chrome::Node {
                        label: title,
                        depth: 0,
                        fold: Some(open),
                        selected: false,
                        count: "",
                        readonly: false,
                        problem: None,
                        icon: ToolIcon::Shared("library"),
                        accessible: title,
                    },
                );
                if fold {
                    toggle(&mut state.closed, title.to_owned());
                }
                if !open {
                    continue;
                }
                for id in ids {
                    let data = loaded.as_array().into_iter().flatten().find(|s| s["source"] == *id);
                    if data.is_some_and(|s| s["builtin"] == true) != builtin {
                        continue;
                    }
                    let registration = self.library_sources.registered.iter().find(|r| r.source == *id);
                    let name = data
                        .and_then(|s| s["name"].as_str())
                        .or_else(|| registration.map(|r| r.name.as_str()).filter(|s| !s.is_empty()))
                        .unwrap_or(id);
                    let status = self.library_sources.statuses.get(id).filter(|s| s.as_str() != "Available").map(String::as_str);
                    let problems: Vec<_> = data.into_iter().flat_map(|s| s["problems"].as_array()).flatten().filter_map(Value::as_str).collect();
                    let problem = status.or_else(|| problems.first().copied());
                    let count = data
                        .map(|s| ["symbols", "footprints", "parts", "manufacturer_parts"].iter().map(|k| s[*k].as_u64().unwrap_or(0)).sum::<u64>())
                        .unwrap_or(0)
                        .to_string();
                    let open = !state.closed.contains(id);
                    let (clicked, fold) = chrome::tree(
                        ui,
                        ui.make_persistent_id(id),
                        chrome::Node {
                            label: name,
                            depth: 1,
                            fold: Some(open),
                            selected: state.source.as_ref() == Some(id) && state.kind == 0,
                            count: &count,
                            readonly: builtin || registration.is_some_and(|r| !r.writable && !crate::IN_BROWSER),
                            problem,
                            icon: ToolIcon::Shared("folder"),
                            accessible: name,
                        },
                    );
                    if fold {
                        toggle(&mut state.closed, id.clone());
                    }
                    if clicked {
                        state.source = Some(id.clone());
                        state.kind = 0;
                        state.selected = None;
                        state.compact = 1;
                    }
                    if open {
                        for (kind, label, field) in [
                            (1, "Symbols", "symbols"),
                            (2, "Footprints", "footprints"),
                            (3, "Generic parts", "parts"),
                            (4, "Orderable parts", "manufacturer_parts"),
                        ] {
                            let count = data.and_then(|s| s[field].as_u64()).unwrap_or(0).to_string();
                            if chrome::tree(
                                ui,
                                ui.make_persistent_id((id, kind)),
                                chrome::Node {
                                    label,
                                    depth: 2,
                                    fold: None,
                                    selected: state.source.as_ref() == Some(id) && state.kind == kind,
                                    count: &count,
                                    readonly: false,
                                    problem: None,
                                    icon: match kind {
                                        1 => ToolIcon::Symbol,
                                        2 => ToolIcon::Footprint,
                                        3 => ToolIcon::Bom,
                                        _ => ToolIcon::Shared("library"),
                                    },
                                    accessible: &format!("{label} in {name}"),
                                },
                            )
                            .0
                            {
                                state.source = Some(id.clone());
                                state.kind = kind;
                                state.selected = None;
                                state.compact = 1;
                            }
                        }
                    }
                    if state.source.as_ref() == Some(id) {
                        if let Some(status) = status {
                            ui.label(egui::RichText::new(status).color(if matches!(status, "Needs reopening" | "Loading…") {
                                pcbcraft_chrome::Tokens::get().text_dim
                            } else {
                                pcbcraft_chrome::Tokens::get().error
                            }));
                        }
                        for problem in problems {
                            ui.label(egui::RichText::new(problem).small().color(pcbcraft_chrome::Tokens::get().error));
                        }
                    }
                }
            }
        });
    }
    /// Latest revisions form the list; an explicit preview revision can still address an older item.
    fn library_items(&self, ui: &mut egui::Ui, state: &mut State, loaded: &Value) {
        let Some(source) = state.source.as_deref() else {
            ui.label("No libraries");
            return;
        };
        let data = loaded.as_array().into_iter().flatten().find(|s| s["source"] == source);
        let registration = self.library_sources.registered.iter().find(|r| r.source == source);
        chrome::title(
            ui,
            data.and_then(|s| s["name"].as_str()).or_else(|| registration.map(|r| r.name.as_str()).filter(|s| !s.is_empty())).unwrap_or(source),
        );
        let builtin = data.is_some_and(|s| s["builtin"] == true);
        let kind = if builtin {
            "Built-in"
        } else if registration.is_some_and(|r| !r.path.is_empty()) {
            "Folder"
        } else {
            "Local files"
        };
        let access = if registration.is_some_and(|r| r.writable) { "Writable" } else { "Read-only" };
        let license = data.and_then(|s| s["license"].as_str()).unwrap_or("");
        ui.label(
            egui::RichText::new(format!("{kind} · {access}{}", if license.is_empty() { String::new() } else { format!(" · {license}") }))
                .small()
                .color(pcbcraft_chrome::Tokens::get().text_dim),
        );
        if let Some(path) = self.library_sources.registered.iter().find(|r| r.source == source).map(|r| r.path.as_str()).filter(|s| !s.is_empty()) {
            ui.add(egui::Label::new(egui::RichText::new(path).small().color(pcbcraft_chrome::Tokens::get().text_dim)).truncate()).on_hover_text(path);
        }
        let previous_kind = state.kind;
        chrome::toolbar(ui, &mut state.query, &mut state.kind);
        if state.kind != previous_kind {
            state.selected = None;
        }
        let catalog = self.session.libraries();
        let all = catalog.source_items(source);
        let mut latest: BTreeMap<(Kind, &str), &Item> = BTreeMap::new();
        for item in all.iter().filter(|i| i.key.kind != Kind::Category) {
            let key = (item.key.kind, item.key.id.as_str());
            if latest.get(&key).is_none_or(|old| old.key.revision < item.key.revision) {
                latest.insert(key, item);
            }
        }
        let query = state.query.trim().to_lowercase();
        let mut items: Vec<_> = latest
            .into_values()
            .filter(|item| {
                let kind = kind_index(item.key.kind);
                (state.kind == 0 || state.kind == kind)
                    && format!(
                        "{} {} {}",
                        item.name,
                        item.key.id,
                        item.manufacturer_part().map(|m| format!("{} {}", m.manufacturer, m.mpn)).unwrap_or_default()
                    )
                    .to_lowercase()
                    .contains(&query)
            })
            .collect();
        items.sort_by_key(|item| (item.key.kind != Kind::Part, item.name.to_lowercase(), item.key.id.clone()));
        let manufacturers = latest_manufacturers(all);
        let mut counts = BTreeMap::<(String, String), usize>::new();
        for item in &manufacturers {
            if let Some(record) = item.manufacturer_part() {
                *counts.entry((record.part.library.clone(), record.part.id.clone())).or_default() += 1;
            }
        }
        // Generic views nest exact MPN records under the generic revision they reference.
        if state.kind == 0 || state.kind == 3 {
            let mut nested = Vec::new();
            for item in &items {
                if item.key.kind == Kind::ManufacturerPart
                    && state.kind == 0
                    && items.iter().any(|parent| {
                        parent.part().is_some_and(|p| item.manufacturer_part().is_some_and(|m| m.part.library == p.library && m.part.id == p.id))
                    })
                {
                    continue;
                }
                nested.push((*item, false));
                if state.expanded.contains(&item.key)
                    && let Some(part) = item.part()
                {
                    for &child in &manufacturers {
                        if child.manufacturer_part().is_some_and(|m| m.part.library == part.library && m.part.id == part.id) {
                            nested.push((child, true));
                        }
                    }
                }
            }
            self.library_table(ui, state, &nested, false, &counts);
        } else {
            let rows: Vec<_> = items.iter().map(|i| (*i, false)).collect();
            self.library_table(ui, state, &rows, state.kind == 4, &counts);
        }
    }
    /// Build domain columns while the chrome adapter owns selectable table rendering.
    fn library_table(
        &self,
        ui: &mut egui::Ui,
        state: &mut State,
        items: &[(&Item, bool)],
        orderable: bool,
        counts: &BTreeMap<(String, String), usize>,
    ) {
        let rows: Vec<_> = items
            .iter()
            .map(|(item, nested)| {
                let cells = if orderable {
                    item.manufacturer_part()
                        .map(|m| {
                            vec![
                                m.manufacturer.clone(),
                                m.mpn.clone(),
                                m.part.id.clone(),
                                m.package.clone(),
                                m.specifications.values().map(|v| v.display()).collect::<Vec<_>>().join(", "),
                                m.datasheet.clone().unwrap_or_else(|| "—".into()),
                                format!("r{}", item.key.revision),
                            ]
                        })
                        .unwrap_or_default()
                } else {
                    let part = item.part();
                    let count = part.as_ref().and_then(|p| counts.get(&(p.library.clone(), p.id.clone()))).copied().unwrap_or(0);
                    vec![
                        item.name.clone(),
                        kind_name(item.key.kind).into(),
                        part.as_ref().and_then(|p| p.category.as_ref()).map(|c| c.definition.id.clone()).unwrap_or_else(|| "—".into()),
                        part.as_ref().map(|p| p.packages.values().map(|p| p.name.clone()).collect::<Vec<_>>().join(", ")).unwrap_or_default(),
                        if part.is_some() { count.to_string() } else { String::new() },
                        format!("r{}{}", item.key.revision, if self.session.libraries().conflicting(&item.key) { " · conflict" } else { "" }),
                    ]
                };
                chrome::TableEntry {
                    key: format!("{} {}/{} r{}", kind_name(item.key.kind), item.key.library, item.key.id, item.key.revision),
                    cells,
                    selected: state.selected.as_ref() == Some(&item.key),
                    branch: item.part().map(|_| state.expanded.contains(&item.key)),
                    nested: *nested,
                }
            })
            .collect();
        ui.label(egui::RichText::new(format!("{} items", items.len())).small().color(pcbcraft_chrome::Tokens::get().text_dim));
        let headers: &[&str] = if orderable {
            &["Manufacturer", "Part number", "Generic part", "Package", "Value", "Datasheet", "Rev"]
        } else {
            &["Name", "Type", "Category", "Packages", "Orderable", "Rev"]
        };
        chrome::table(ui, headers, &rows, |action| match action {
            chrome::ItemAction::Select(index) => state.selected = Some(items[index].0.key.clone()),
            chrome::ItemAction::Toggle(index) => toggle(&mut state.expanded, items[index].0.key.clone()),
        });
    }
    /// Preview metadata and exact dependency references from the selected source revision.
    fn library_item_preview(&self, ui: &mut egui::Ui, state: &mut State) {
        state.sync_preview();
        let all = self.session.libraries().source_items(state.source.as_deref().unwrap_or(""));
        let Some(item) = state.selected.as_ref().and_then(|key| all.iter().find(|i| i.key == *key)) else {
            ui.label("Select an item");
            return;
        };
        egui::ScrollArea::vertical().id_salt("library_preview_content").auto_shrink([false, false]).show(ui, |ui| {
            chrome::title(ui, &item.name);
            ui.label(
                egui::RichText::new(format!("{} · {}/{} · r{}", kind_name(item.key.kind), item.key.library, item.key.id, item.key.revision)).small(),
            );
            if self.session.libraries().conflicting(&item.key) {
                ui.label(egui::RichText::new("Quarantined: conflicting content at this revision").color(pcbcraft_chrome::Tokens::get().error));
            }
            if let Some(symbol) = item.symbol() {
                crate::library_preview::workspace_symbol(ui, &symbol, &mut state.symbol_camera);
                chrome::detail(ui, "Units", &symbol.units().len().to_string());
            }
            if let Some(footprint) = item.footprint() {
                crate::library_preview::workspace_footprint(ui, &footprint, &mut state.footprint_camera);
            }
            chrome::detail(ui, "License", &item.license);
            let mut references = Vec::new();
            if let Some(part) = item.part() {
                self.generic_preview(ui, &part, None, &mut references, state);
            }
            if let Some(record) = item.manufacturer_part() {
                chrome::detail(ui, "Manufacturer", &record.manufacturer);
                chrome::detail(ui, "Part number", &record.mpn);
                chrome::detail(ui, "Generic part", &record.part.id);
                chrome::detail(ui, "Package", &record.package);
                if let Some(url) = record.datasheet.as_ref() {
                    ui.hyperlink_to("Datasheet", url);
                }
                chrome::section(ui, "Specifications");
                let category = self.session.libraries().resolve_part(&record.part).ok().and_then(|p| p.category.as_ref());
                for (name, value) in &record.specifications {
                    let label = category.and_then(|c| c.fields.get(name)).map_or(name.as_str(), |f| f.name.as_str());
                    chrome::detail(ui, label, &value.display());
                }
                chrome::detail(ui, "Source", &record.source.reference);
                references.push(record.part.clone());
                match self.session.libraries().resolve_part(&record.part) {
                    Ok(part) => {
                        self.generic_preview(ui, &part.definition, Some(&record.package), &mut references, state);
                    }
                    Err(error) => {
                        ui.label(egui::RichText::new(error).color(pcbcraft_chrome::Tokens::get().error));
                    }
                }
            }
            if let Some(symbol) = item.symbol()
                && let Some(binding) = &symbol.category
            {
                references.push(binding.definition.clone());
            }
            chrome::section(ui, "Revisions");
            let mut revisions: Vec<_> = all.iter().filter(|other| other.key.kind == item.key.kind && other.key.id == item.key.id).collect();
            revisions.sort_by_key(|i| std::cmp::Reverse(i.key.revision));
            revisions.dedup_by_key(|i| i.key.revision);
            for revision in revisions {
                let label = format!("r{}", revision.key.revision);
                if chrome::revision(ui, &label, revision.key == item.key) {
                    state.selected = Some(revision.key.clone());
                }
            }
            chrome::section(ui, "Depends on");
            if references.is_empty() {
                ui.label("None");
            }
            let mut seen = BTreeSet::new();
            for reference in references.into_iter().filter(|r| seen.insert((r.library.clone(), r.id.clone(), r.revision))) {
                ui.label(format!("{}/{} r{}", reference.library, reference.id, reference.revision));
            }
        });
    }
    /// Resolve geometry without placements. Orderable records choose their exact package;
    /// generic browsing uses the first package. Missing selections never fall back silently.
    fn generic_preview(&self, ui: &mut egui::Ui, part: &PartDefinition, package: Option<&str>, references: &mut Vec<Reference>, state: &mut State) {
        references.push(part.symbol.clone());
        if let Some(category) = &part.category {
            references.push(category.definition.clone());
            chrome::detail(ui, "Category", &category.definition.id);
        }
        chrome::detail(ui, "Source", &part.source.reference);
        match self.session.libraries().resolve_part(&part.reference()) {
            Ok(resolved) => {
                chrome::detail(ui, "Units", &resolved.symbol.units().len().to_string());
                let selected = package.map_or_else(|| part.packages.values().next(), |id| part.packages.get(id));
                if let Some(package) = selected {
                    let footprint = self.session.libraries().footprint(&package.footprint.library, &package.footprint.id, package.footprint.revision);
                    ui.columns(2, |columns| {
                        chrome::detail(&mut columns[0], "Symbol", &resolved.symbol.name);
                        crate::library_preview::workspace_symbol(&mut columns[0], &resolved.symbol, &mut state.symbol_camera);
                        chrome::detail(&mut columns[1], "Footprint", &package.name);
                        if let Some(footprint) = footprint {
                            crate::library_preview::workspace_footprint(&mut columns[1], &footprint, &mut state.footprint_camera);
                        }
                    });
                } else {
                    crate::library_preview::workspace_symbol(ui, &resolved.symbol, &mut state.symbol_camera);
                    if let Some(package) = package {
                        ui.label(egui::RichText::new(format!("Package {package} is unavailable")).color(pcbcraft_chrome::Tokens::get().error));
                    }
                }
            }
            Err(error) => {
                ui.label(egui::RichText::new(error).color(pcbcraft_chrome::Tokens::get().error));
            }
        }
        chrome::section(ui, "Packages");
        if part.packages.is_empty() {
            ui.label("No packages");
        }
        for package in part.packages.values() {
            references.push(package.footprint.clone());
            chrome::detail(ui, &package.name, &format!("{} terminals mapped", package.mapping.len()));
            controls::collapsing(ui, ("package_mapping", &package.name), "Pin map", false, None, |ui| {
                for (terminal, pads) in &package.mapping {
                    chrome::detail(ui, terminal, &pads.join(", "));
                }
                if !package.unconnected_pads.is_empty() {
                    chrome::detail(ui, "Unconnected pads", &package.unconnected_pads.join(", "));
                }
            });
        }
        chrome::section(ui, "Orderable parts");
        for record in self.session.libraries().manufacturers_for(&part.reference()) {
            ui.label(format!("{} · {}", record.manufacturer, record.mpn));
        }
    }
}
/// Stable kind mapping keeps display vocabulary separate from stored schema names.
fn kind_index(kind: Kind) -> usize {
    match kind {
        Kind::Symbol => 1,
        Kind::Footprint => 2,
        Kind::Part => 3,
        Kind::ManufacturerPart => 4,
        Kind::Category => 0,
    }
}
/// User-facing names follow #315 without renaming persisted records or commands.
fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Symbol => "Symbol",
        Kind::Footprint => "Footprint",
        Kind::Part => "Generic part",
        Kind::ManufacturerPart => "Orderable part",
        Kind::Category => "Category",
    }
}
/// Toggle an independent branch without coupling Browser and list expansion state.
fn toggle<T: Ord>(values: &mut BTreeSet<T>, value: T) {
    if !values.remove(&value) {
        values.insert(value);
    }
}

/// Both nested and flat views represent the same latest orderable records, including records
/// that deliberately retain a dependency on an older generic-part revision.
fn latest_manufacturers(items: &[Item]) -> Vec<&Item> {
    let mut latest: BTreeMap<(&str, &str), &Item> = BTreeMap::new();
    for item in items.iter().filter(|i| i.key.kind == Kind::ManufacturerPart) {
        let key = (item.key.library.as_str(), item.key.id.as_str());
        if latest.get(&key).is_none_or(|old| old.key.revision < item.key.revision) {
            latest.insert(key, item);
        }
    }
    latest.into_values().collect()
}
