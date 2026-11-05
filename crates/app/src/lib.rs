//! The PCBCraft window, the same on the desktop and in the browser.
//!
//! Every change to the project goes through [`Session::execute`], with commands from the engine's
//! catalog, the same ones MCP runs. Preferences belong to the person, not the project, so they live
//! here and stay out of the command set.

mod net_inspector;
mod preferences;
mod project_files;
mod schematic_canvas;

use schematic_canvas::{Click, Tool};

use eframe::Frame;
use egui::{Key, KeyboardShortcut, Modifiers, RichText, Ui};
use pcbcraft_chrome::{
    MenuItem, Tokens, fonts, menu,
    shell::{self, dim},
    theme, workspace,
};
use pcbcraft_engine::{
    CONNECT_PINS, MOVE_SYMBOL, PLACE_SYMBOL, Session, command,
    schematic::{BUILTIN_SYMBOLS, SymbolDefinition},
};
use serde_json::{Value, json};

pub use preferences::{Preferences, ThemeChoice};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const PREFERENCES_KEY: &str = "pcbcraft.preferences";

const COMMAND_Z: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
const COMMAND_Y: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);
const COMMAND_O: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
const COMMAND_S: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
const SAVE_AS: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::S);
const COMMAND_N: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
const COMMAND_Q: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Q);
const ADD_SYMBOL: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::A);
const ADD_WIRE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::W);
const COMMAND_COMMA: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Comma);

/// A browser keeps Ctrl+N and Ctrl+Q for itself, and has no window of ours to quit.
const IN_BROWSER: bool = cfg!(target_arch = "wasm32");

#[derive(Debug, Default)]
enum Dialog {
    #[default]
    None,
    /// Asks for the name that `project.new` or `project.rename` takes.
    Name {
        command: &'static str,
        text: String,
        error: Option<String>,
        focused: bool,
    },
    ChooseSymbol,
    ConfirmOpen,
    Preferences,
    About,
}

#[derive(Debug, Default)]
pub struct PcbApp {
    session: Session,
    files: project_files::ProjectFiles,
    file_notice: Option<String>,
    camera: schematic_canvas::Camera,
    preferences: Preferences,
    net_inspector: net_inspector::NetInspector,
    dialog: Dialog,
    tool: Option<Tool>,
    /// Why the last command failed, until the next one runs.
    message: Option<String>,
    /// The palette and zoom last applied, so they are set only when they change.
    applied: Option<(bool, f32)>,
}

impl PcbApp {
    /// The app as the desktop and browser builds start it.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let preferences = cc.storage.and_then(|s| s.get_string(PREFERENCES_KEY)).map(|text| Preferences::parse(&text)).unwrap_or_default();
        Self { preferences, ..Self::default() }
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn preferences(&self) -> &Preferences {
        &self.preferences
    }

    /// Run a command for the person at the keyboard. A failure shows in the status bar.
    fn run(&mut self, id: &str, params: &Value) {
        self.file_notice = None;
        self.message = self.session.apply(id, params).err().map(|e| e.to_string());
    }

    /// Start a symbol preview at the current project revision.
    fn begin_placement(&mut self, definition: &'static SymbolDefinition) {
        self.message = None;
        self.tool = Some(Tool::PlaceSymbol { definition, revision: self.session.revision() });
    }

    /// Wait for the first pin at the current project revision.
    fn begin_wire(&mut self) {
        self.message = None;
        self.tool = Some(Tool::Wire { revision: self.session.revision(), start: None });
    }

    fn ask_name(&mut self, command: &'static str) {
        let text = match command {
            "project.rename" => self.session.project().map(|p| p.name.clone()).unwrap_or_default(),
            _ => String::new(),
        };
        self.dialog = Dialog::Name { command, text, error: None, focused: false };
    }

    /// Protect unsaved edits before opening a file that replaces the current session.
    fn open_project_file(&mut self, ctx: &egui::Context) {
        if self.files.dirty(&self.session) {
            self.dialog = Dialog::ConfirmOpen;
        } else {
            self.message = self.files.open(self.session.revision(), ctx).err();
            self.file_notice = None;
        }
    }

    /// Start a save or download for the current snapshot and report setup failures.
    fn save_project_file(&mut self, save_as: bool, ctx: &egui::Context) {
        self.message = self.files.save(&self.session, save_as, ctx).err();
        self.file_notice = None;
    }

    /// A row for an engine command: its title comes from the catalog, its state from the session.
    /// True when it was clicked.
    fn command_row(&self, ui: &mut Ui, id: &str, suffix: &str, shortcut: &str) -> bool {
        let title = command(id).map_or(id, |c| c.title);
        MenuItem::new(&format!("{title}{suffix}")).shortcut(shortcut).enabled(self.session.available(id)).show(ui)
    }

    /// Show engine commands and view controls. Symbol placement starts with the built-in picker.
    fn menus(&mut self, ui: &mut Ui) {
        shell::menu_bar(ui, |ui| {
            menu::top(ui, "File", |ui| {
                if self.command_row(ui, "project.new", "…", if IN_BROWSER { "" } else { "Ctrl+N" }) {
                    self.ask_name("project.new");
                }
                let file_ready = !self.files.busy();
                if MenuItem::new("Open…").shortcut(if IN_BROWSER { "" } else { "Ctrl+O" }).enabled(file_ready).show(ui) {
                    self.open_project_file(ui.ctx());
                }
                let can_save = file_ready && self.session.project().is_some();
                let save_label = if IN_BROWSER { "Download Project" } else { "Save" };
                if MenuItem::new(save_label).shortcut(if IN_BROWSER { "" } else { "Ctrl+S" }).enabled(can_save).show(ui) {
                    self.save_project_file(false, ui.ctx());
                }
                if !IN_BROWSER && MenuItem::new("Save As…").shortcut("Ctrl+Shift+S").enabled(can_save).show(ui) {
                    self.save_project_file(true, ui.ctx());
                }
                if self.command_row(ui, "project.rename", "…", "") {
                    self.ask_name("project.rename");
                }
                if !IN_BROWSER {
                    menu::separator(ui);
                    if MenuItem::new("Quit").shortcut("Ctrl+Q").show(ui) {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            });
            menu::top(ui, "Edit", |ui| {
                if self.command_row(ui, "edit.undo", "", "Ctrl+Z") {
                    self.run("edit.undo", &Value::Null);
                }
                if self.command_row(ui, "edit.redo", "", "Ctrl+Y") {
                    self.run("edit.redo", &Value::Null);
                }
            });
            menu::top(ui, "View", |ui| {
                let has_project = self.session.project().is_some();
                if MenuItem::new("Zoom to Fit").enabled(has_project).show(ui) {
                    self.camera.request_fit();
                }
                let inspector = if has_project && self.net_inspector.open { "Hide Net Inspector" } else { "Show Net Inspector" };
                if MenuItem::new(inspector).enabled(has_project).show(ui) {
                    self.net_inspector.open = !self.net_inspector.open;
                }
            });
            menu::top(ui, "Place", |ui| {
                if self.command_row(ui, PLACE_SYMBOL, "…", "A") {
                    self.dialog = Dialog::ChooseSymbol;
                }
                if MenuItem::new("Wire").shortcut("W").enabled(self.session.available(CONNECT_PINS)).show(ui) {
                    self.begin_wire();
                }
            });
            menu::top(ui, "Preferences", |ui| {
                if MenuItem::new("Preferences…").shortcut("Ctrl+,").show(ui) {
                    self.dialog = Dialog::Preferences;
                }
            });
            menu::top(ui, "Help", |ui| {
                if MenuItem::new("About PCBCraft").show(ui) {
                    self.dialog = Dialog::About;
                }
            });
        });
    }

    /// Expose the same placement, wiring, inspection, and fit actions as the menus.
    fn ribbon(&mut self, ui: &mut Ui) {
        use workspace::ToolIcon;

        let has_project = self.session.project().is_some();
        workspace::ribbon(ui, "Schematic", "Capture", |ui| {
            let placing = matches!(self.tool, Some(Tool::PlaceSymbol { .. }));
            if workspace::tool(ui, ToolIcon::Symbol, "Symbol", "Add symbol (A)", self.session.available(PLACE_SYMBOL), placing).clicked() {
                self.dialog = Dialog::ChooseSymbol;
            }
            let wiring = matches!(self.tool, Some(Tool::Wire { .. }));
            if workspace::tool(ui, ToolIcon::Wire, "Wire", "Draw wire (W)", self.session.available(CONNECT_PINS), wiring).clicked() {
                self.begin_wire();
            }
            ui.separator();
            if workspace::tool(ui, ToolIcon::Nets, "Nets", "Toggle Net Inspector", has_project, has_project && self.net_inspector.open).clicked() {
                self.net_inspector.open = !self.net_inspector.open;
            }
            if workspace::tool(ui, ToolIcon::Fit, "Fit", "Fit schematic", has_project, false).clicked() {
                self.camera.request_fit();
            }
        });
    }

    /// Show the command error or placement hint alongside the project name.
    fn status(&self, ui: &mut Ui) {
        let name = self.session.project().map_or("No project open", |p| p.name.as_str()).to_owned();
        let message = self.message.clone();
        shell::status_bar(
            ui,
            |ui| {
                ui.label(dim(name));
                if let Some(message) = message {
                    ui.label(shell::error(message));
                } else if let Some(tool) = self.tool {
                    ui.label(match tool {
                        Tool::PlaceSymbol { definition, .. } => dim(format!("Place {} · Esc to cancel", definition.name)),
                        Tool::Move { .. } => dim("Move symbol · Esc to cancel"),
                        Tool::Wire { start: None, .. } => dim("Wire · Click a pin"),
                        Tool::Wire { start: Some(_), .. } => dim("Wire · Click the destination pin"),
                    });
                } else if self.files.busy() {
                    ui.label(dim("File operation in progress"));
                } else if let Some(notice) = &self.file_notice {
                    ui.label(dim(notice.as_str()));
                }
            },
            |ui| {
                ui.label(dim(format!("PCBCraft {VERSION}")));
            },
        );
    }

    /// Draw the schematic and send completed edits through the shared command catalog. Cancel only clears the local preview.
    fn canvas(&mut self, ui: &mut Ui) {
        let input_enabled = matches!(self.dialog, Dialog::None) && !egui::Popup::is_any_open(ui.ctx());
        let click = shell::canvas(ui, |ui| {
            if let Some(project) = self.session.project() {
                return schematic_canvas::show(ui, project, &mut self.camera, self.tool, input_enabled);
            }
            let t = Tokens::get();
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("No project open. Choose File, then New Project.").size(t.font_button).color(t.canvas_ink));
            });
            None
        });
        match (self.tool, click) {
            (Some(Tool::PlaceSymbol { definition, revision }), Some(Click::Place(point))) => {
                self.tool = None;
                self.run(PLACE_SYMBOL, &json!({"definition": definition.id, "x_nm": point.x_nm, "y_nm": point.y_nm, "expected_revision": revision}));
            }
            (Some(Tool::Wire { revision, start: None }), Some(Click::Pin(pin))) => {
                self.tool = Some(Tool::Wire { revision, start: Some(pin) });
            }
            (Some(Tool::Wire { revision, start: Some(from) }), Some(Click::Pin(to))) if from != to => {
                self.run(CONNECT_PINS, &json!({"from": from.inspect(), "to": to.inspect(), "expected_revision": revision}));
                if self.message.is_none() {
                    self.tool = None;
                }
            }
            (None, Some(Click::Grab(symbol, offset))) => {
                self.tool = Some(Tool::Move { symbol, offset, revision: self.session.revision() });
            }
            (Some(Tool::Move { symbol, revision, .. }), Some(Click::FinishMove(position))) => {
                self.tool = None;
                if let Some(point) = position {
                    self.run(
                        MOVE_SYMBOL,
                        &json!({
                            "symbol": symbol.id(), "x_nm": point.x_nm, "y_nm": point.y_nm, "expected_revision": revision,
                        }),
                    );
                }
            }
            (Some(Tool::Wire { .. }), Some(Click::CancelWire)) => {
                self.tool = None;
                self.message = None;
            }
            _ => {}
        }
    }

    /// Handle editor keys outside text fields and dialogs. Placement keys apply only while menus are closed.
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || !matches!(self.dialog, Dialog::None) {
            return;
        }
        let pressed = |shortcut: &KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(shortcut));
        if !egui::Popup::is_any_open(ctx) {
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) && self.tool.take().is_some() {
                self.message = None;
            }
            if pressed(&ADD_SYMBOL) && self.session.available(PLACE_SYMBOL) {
                self.dialog = Dialog::ChooseSymbol;
            }
            if pressed(&ADD_WIRE) && self.session.available(CONNECT_PINS) {
                self.begin_wire();
            }
        }
        if pressed(&COMMAND_Z) && self.session.available("edit.undo") {
            self.run("edit.undo", &Value::Null);
        }
        if pressed(&COMMAND_Y) && self.session.available("edit.redo") {
            self.run("edit.redo", &Value::Null);
        }
        if pressed(&COMMAND_COMMA) {
            self.dialog = Dialog::Preferences;
        }
        if !IN_BROWSER {
            if !self.files.busy() {
                if pressed(&COMMAND_O) {
                    self.open_project_file(ctx);
                }
                if self.session.project().is_some() {
                    if pressed(&SAVE_AS) {
                        self.save_project_file(true, ctx);
                    } else if pressed(&COMMAND_S) {
                        self.save_project_file(false, ctx);
                    }
                }
            }
            if pressed(&COMMAND_N) {
                self.ask_name("project.new");
            }
            if pressed(&COMMAND_Q) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// Draw the open dialog and retain it until the person finishes or dismisses it.
    fn dialogs(&mut self, ctx: &egui::Context) {
        match std::mem::take(&mut self.dialog) {
            Dialog::None => {}
            Dialog::Name { command: id, mut text, mut error, mut focused } => {
                let title = command(id).map_or(id, |c| c.title);
                let (mut submit, mut cancel) = (false, false);
                let closing = shell::dialog(ctx, "name_dialog", title, |ui| {
                    ui.label("Name");
                    let field = ui.add(egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY));
                    if !focused {
                        field.request_focus();
                        focused = true;
                    }
                    submit = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    if let Some(error) = &error {
                        ui.label(shell::error(error.as_str()));
                    }
                    ui.horizontal(|ui| {
                        submit |= ui.button("OK").clicked();
                        cancel = ui.button("Cancel").clicked();
                    });
                });
                if submit {
                    // A bad name stays in the dialog to fix, so the person sees why.
                    match self.session.apply(id, &json!({"name": text})) {
                        Ok(_) => {
                            self.file_notice = None;
                            self.message = None;
                            if id == "project.new" {
                                self.camera = schematic_canvas::Camera::default();
                            }
                        }
                        Err(e) => {
                            error = Some(e.to_string());
                            self.dialog = Dialog::Name { command: id, text, error, focused };
                        }
                    }
                } else if !(closing || cancel) {
                    self.dialog = Dialog::Name { command: id, text, error, focused };
                }
            }
            Dialog::ConfirmOpen => {
                let (mut proceed, mut cancel) = (false, false);
                let closing = shell::dialog(ctx, "confirm_open", "Open Project", |ui| {
                    ui.label("Opening another project will discard unsaved changes.");
                    ui.horizontal(|ui| {
                        proceed = ui.button("Discard and open…").clicked();
                        cancel = ui.button("Cancel").clicked();
                    });
                });
                if proceed {
                    self.message = self.files.open(self.session.revision(), ctx).err();
                    self.file_notice = None;
                } else if !(cancel || closing) {
                    self.dialog = Dialog::ConfirmOpen;
                }
            }
            Dialog::ChooseSymbol => {
                let mut selected = None;
                let mut cancel = false;
                let closing = shell::dialog(ctx, "symbol_picker", "Place Symbol", |ui| {
                    let row = [ui.available_width(), Tokens::get().control_height];
                    for &definition in BUILTIN_SYMBOLS {
                        if ui.add_sized(row, egui::Button::new(definition.name)).clicked() {
                            selected = Some(definition);
                        }
                    }
                    cancel = ui.button("Cancel").clicked();
                });
                if let Some(definition) = selected {
                    self.begin_placement(definition);
                } else if !(closing || cancel) {
                    self.dialog = Dialog::ChooseSymbol;
                }
            }
            Dialog::Preferences => {
                let mut close = false;
                let closing = shell::dialog(ctx, "preferences_dialog", "Preferences", |ui| {
                    self.preferences.show(ui);
                    close = ui.button("Close").clicked();
                });
                if !(closing || close) {
                    self.dialog = Dialog::Preferences;
                }
            }
            Dialog::About => {
                let mut close = false;
                let closing = shell::dialog(ctx, "about_dialog", "About PCBCraft", |ui| {
                    ui.label(format!("PCBCraft {VERSION}"));
                    ui.label("Electronics design that feels like KiCad, built for people and agents to work on the same board.");
                    ui.label(dim("MIT OR Apache-2.0. Third-party notices are in ATTRIBUTION.md."));
                    close = ui.button("Close").clicked();
                });
                if !(closing || close) {
                    self.dialog = Dialog::About;
                }
            }
        }
    }

    fn sync_style(&mut self, ctx: &egui::Context) {
        let dark = self.preferences.is_dark(ctx.system_theme());
        let zoom = self.preferences.zoom();
        if self.applied.is_none() {
            fonts::install(ctx);
        }
        if self.applied != Some((dark, zoom)) {
            theme::apply(ctx, dark);
            ctx.set_zoom_factor(zoom);
            self.applied = Some((dark, zoom));
        }
    }

    /// Draw one frame. Drop the preview if another action changed the project or opened a dialog.
    pub fn frame(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.sync_style(&ctx);
        if let Some(update) = self.files.poll(&mut self.session, &ctx) {
            match update {
                project_files::Update::Opened => {
                    self.camera = schematic_canvas::Camera::default();
                    self.camera.request_fit();
                    self.tool = None;
                    self.dialog = Dialog::None;
                    self.message = None;
                    self.file_notice = Some("Project opened".to_owned());
                }
                project_files::Update::Message(text) => self.file_notice = Some(text),
                project_files::Update::Error(error) => self.message = Some(error),
            }
        }
        self.shortcuts(&ctx);
        self.menus(ui);
        self.ribbon(ui);
        if self.tool.is_some_and(|tool| tool.revision() != self.session.revision()) || !matches!(self.dialog, Dialog::None) {
            self.tool = None;
        }
        self.status(ui);
        if let Some(project) = self.session.project() {
            workspace::sheet_header(ui, &format!("{} / Main", project.name));
            self.net_inspector.show(ui, &self.session);
        }
        let revision = self.session.revision();
        self.canvas(ui);
        self.dialogs(&ctx);
        if self.net_inspector.open && revision != self.session.revision() {
            ctx.request_repaint();
        }
    }
}

impl eframe::App for PcbApp {
    fn ui(&mut self, ui: &mut Ui, frame: &mut Frame) {
        let before = self.preferences;
        self.frame(ui);
        // eframe's own save runs every 30 seconds, too late if the page reloads right after a change.
        if self.preferences != before
            && let Some(storage) = frame.storage_mut()
        {
            self.save(storage);
            storage.flush();
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(PREFERENCES_KEY, self.preferences.to_text());
    }
}

#[cfg(test)]
mod tests;
