//! The Home page: open from the house button in the app bar. It replaces the ribbon and canvas
//! while it is shown and leaves the open project, its history and any unsaved edits untouched.
//! Each action reuses the same dialogs, file jobs and engine commands as the File menu.

use egui::Ui;
use pcbcraft_chrome::{home, menu::MenuIcon};

use super::{COMMAND_COMMA, COMMAND_N, COMMAND_O, COMMAND_S, Dialog, IN_BROWSER, PcbApp, SAVE_AS};

/// What a Home row or tile asked for, applied after the page is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Back,
    New,
    Open,
    Save,
    SaveAs,
    Preferences,
    About,
    #[cfg(not(target_arch = "wasm32"))]
    OpenRecent(usize),
    #[cfg(not(target_arch = "wasm32"))]
    RemoveRecent(usize),
    #[cfg(not(target_arch = "wasm32"))]
    ClearRecent,
    /// Connect AI assistants, the Help row the narrow app bar gives way to. Desktop only.
    #[cfg(not(target_arch = "wasm32"))]
    Connect,
}

impl PcbApp {
    /// Show Home and drop any canvas tool preview, which belongs to the hidden schematic.
    pub(super) fn show_home(&mut self) {
        self.home = true;
        self.views.suspend();
    }

    /// Draw the Home page in the space below the app bar and apply the chosen action.
    pub(super) fn home_page(&mut self, ui: &mut Ui) {
        let (mut navigation, mut start) = (None, None);
        home::page(ui, |ui| self.home_navigation(ui, &mut navigation), |ui| self.home_start(ui, &mut start));
        if let Some(action) = navigation.or(start) {
            self.home_action(ui.ctx(), action);
        }
    }

    /// The rail: back to the workspace, then the project and app actions with their shortcuts.
    /// Save rows are disabled without a project or while a file job runs, like the File menu.
    fn home_navigation(&self, ui: &mut Ui, action: &mut Option<Action>) {
        let back = match self.session.project() {
            Some(project) => format!("Back to {}", project.name),
            None => "Back to the workspace".to_owned(),
        };
        if home::back_to(ui, "home:back", &back, "Esc").clicked() {
            *action = Some(Action::Back);
        }
        let key = |shortcut| if IN_BROWSER { String::new() } else { ui.ctx().format_shortcut(shortcut) };
        let file_ready = !self.files.busy();
        let can_save = self.can_save_project();
        let mut rows = vec![
            ("home:start", "Start", String::new(), true, None),
            ("home:new", "New Project", key(&COMMAND_N), self.session.available("project.new"), Some(Action::New)),
            ("home:open", "Open…", key(&COMMAND_O), file_ready, Some(Action::Open)),
        ];
        if IN_BROWSER {
            rows.push(("home:save", "Download Project…", String::new(), can_save, Some(Action::Save)));
        } else {
            rows.push(("home:save", "Save", key(&COMMAND_S), can_save, Some(Action::Save)));
            rows.push(("home:save_as", "Save As…", key(&SAVE_AS), can_save, Some(Action::SaveAs)));
        }
        let first_save = 3;
        let first_app = rows.len();
        rows.push(("home:preferences", "Preferences", ui.ctx().format_shortcut(&COMMAND_COMMA), true, Some(Action::Preferences)));
        rows.push(("home:about", "About PCBCraft", String::new(), true, Some(Action::About)));
        #[cfg(not(target_arch = "wasm32"))]
        rows.push(("home:connect", "Connect AI assistants…", String::new(), true, Some(Action::Connect)));
        for (index, (id, label, shortcut, enabled, picked)) in rows.into_iter().enumerate() {
            if index == first_save || index == first_app {
                home::separator(ui);
            }
            if home::nav_row(ui, id, label, &shortcut, picked.is_none(), enabled).clicked() {
                *action = picked;
            }
        }
    }

    /// The Start section: project actions, followed by desktop history from successful file jobs.
    fn home_start(&self, ui: &mut Ui, action: &mut Option<Action>) {
        home::heading(ui, "Start", "Create a schematic project, or open a .pc project you saved.");
        let open_caption = if IN_BROWSER {
            "Choose a .pc project file".to_owned()
        } else {
            format!("Choose a .pc project file ({})", ui.ctx().format_shortcut(&COMMAND_O))
        };
        let tiles = [
            (
                "home:new_tile",
                MenuIcon::New,
                "New Project",
                "Start an empty schematic".to_owned(),
                true,
                self.session.available("project.new"),
                Action::New,
            ),
            ("home:open_tile", MenuIcon::Open, "Open Project", open_caption, false, !self.files.busy(), Action::Open),
        ];
        home::section(ui, "New", |ui| {
            home::grid(ui, "home_new", tiles.len(), |ui, index, width| {
                let Some((id, icon, label, caption, primary, enabled, picked)) = tiles.get(index) else { return };
                let tile = home::Tile { icon: *icon, label, caption, primary: *primary, enabled: *enabled };
                if home::action_tile(ui, id, width, tile).clicked() {
                    *action = Some(*picked);
                }
            });
        });
        #[cfg(not(target_arch = "wasm32"))]
        home::section(ui, "Recent projects", |ui| {
            let entries = self.files.recent.entries();
            if entries.is_empty() {
                home::empty_recents(ui);
                return;
            }
            home::recent_list(ui, |ui| {
                for (index, entry) in entries.iter().enumerate() {
                    let path = entry.path.to_string_lossy();
                    let picked = home::recent_row(ui, &path, &entry.name, &path, index > 0, !self.files.busy());
                    if picked.open {
                        *action = Some(Action::OpenRecent(index));
                    } else if picked.remove {
                        *action = Some(Action::RemoveRecent(index));
                    }
                }
            });
            if home::clear_recents(ui).clicked() {
                *action = Some(Action::ClearRecent);
            }
        });
    }

    /// Apply a Home action. New opens an Untitled project in the workspace at once. Open keeps Home
    /// up until the project is opened, so cancelling returns to it; saving returns to the workspace
    /// like SolidCraft's start page.
    fn home_action(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Back => self.home = false,
            Action::New => self.new_project(ctx),
            Action::Open => self.open_project_file(ctx),
            Action::Save => {
                self.home = false;
                self.primary_save(ctx);
            }
            Action::SaveAs => {
                self.home = false;
                self.dialog = Dialog::SaveEncoding;
            }
            Action::Preferences => self.open_preferences(),
            Action::About => self.dialog = Dialog::About,
            #[cfg(not(target_arch = "wasm32"))]
            Action::OpenRecent(index) => {
                if !self.files.busy()
                    && let Some(entry) = self.files.recent.entries().get(index)
                {
                    self.unless_unsaved(super::unsaved::Then::OpenRecent(entry.path.clone()), ctx);
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            Action::RemoveRecent(index) => {
                if let Some(entry) = self.files.recent.entries().get(index) {
                    let path = entry.path.clone();
                    self.files.recent.remove(&path);
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            Action::ClearRecent => self.files.recent.clear(),
            #[cfg(not(target_arch = "wasm32"))]
            Action::Connect => self.dialog = Dialog::LocalMcp { error: None, copied: false },
        }
    }
}
