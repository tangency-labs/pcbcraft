//! One window, shared project, tabbed workspaces. Edits use the engine exclusively.
mod canvas;
use egui::{Align2, Color32, FontId, Rect, Stroke, Vec2};
use pcbcraft_doc::{Id, PinRef, Project, TEMPLATES};
use pcbcraft_geom::Point;
use pcbcraft_mcp::SharedSession;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Schematic,
    Pcb,
    Libraries,
    Checks,
}
#[derive(Clone, Serialize, Deserialize)]
struct Preferences {
    dark: bool,
    grid: bool,
    accent: [u8; 3],
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            dark: true,
            grid: true,
            accent: [91, 224, 188],
        }
    }
}
#[derive(Clone)]
enum Tool {
    Select,
    Place(String),
    Wire(Option<PinRef>),
    Route(Option<(Id, String)>),
    Move,
    Cutout,
}
#[derive(Clone, Copy)]
struct Camera {
    center: Point,
    zoom: f32,
}
impl Camera {
    fn screen(self, p: Point, rect: Rect) -> egui::Pos2 {
        rect.center()
            + Vec2::new(
                (p.x - self.center.x) as f32 * self.zoom,
                (p.y - self.center.y) as f32 * self.zoom,
            )
    }
    fn world(self, p: egui::Pos2, rect: Rect) -> Point {
        Point::new(
            self.center.x + (p.x - rect.center().x) as f64 / self.zoom as f64,
            self.center.y + (p.y - rect.center().y) as f64 / self.zoom as f64,
        )
    }
}
pub struct App {
    session: SharedSession,
    project: Project,
    revision: u64,
    checks: Value,
    tab: Tab,
    tool: Tool,
    selected: Option<Id>,
    preferences: Preferences,
    appearance: bool,
    palette: bool,
    schematic: Camera,
    pcb: Camera,
    drag: Option<(Id, Point)>,
    drag_offset: Point,
    cursor: Option<Point>,
    message: String,
    value: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    cutout_radius: f64,
    path: String,
    saved_revision: Option<u64>,
    command: String,
    params: String,
    command_open: bool,
    capture: Option<String>,
    frames: u32,
}
impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        session: SharedSession,
        capture: Option<String>,
        tab: Option<&str>,
        light: bool,
    ) -> Self {
        let mut preferences: Preferences = cc
            .storage
            .and_then(|s| eframe::get_value(s, "preferences"))
            .unwrap_or_default();
        if light {
            preferences.dark = false;
        }
        if let Some(text) = cc
            .storage
            .and_then(|storage| storage.get_string("recovery"))
            && text.len() <= pcbcraft_io_limit()
            && let Ok(project) = serde_json::from_str::<Project>(&text)
            && project.validate().is_ok()
            && let Ok(mut current) = session.lock()
            && current.revision == 0
            && current.project.components.is_empty()
        {
            let _ = current.replace(project);
        }
        Self::create(session, preferences, capture, tab)
    }
    fn create(
        session: SharedSession,
        preferences: Preferences,
        capture: Option<String>,
        tab: Option<&str>,
    ) -> Self {
        let project = session
            .lock()
            .map(|s| s.project.clone())
            .unwrap_or_default();
        let width = project.board.width;
        let height = project.board.height;
        Self {
            session,
            project,
            revision: u64::MAX,
            checks: json!({}),
            tab: match tab {
                Some("pcb") => Tab::Pcb,
                Some("checks") => Tab::Checks,
                _ => Tab::Schematic,
            },
            tool: Tool::Select,
            selected: None,
            preferences,
            appearance: false,
            palette: false,
            schematic: Camera {
                center: Point::new(40.0, 24.0),
                zoom: 11.0,
            },
            pcb: Camera {
                center: Point::new(30.0, 20.0),
                zoom: 15.0,
            },
            drag: None,
            drag_offset: Point::default(),
            cursor: None,
            message: "Ready · A add · W wire · X route · M move · R rotate · E properties".into(),
            value: String::new(),
            x: 0.0,
            y: 0.0,
            width,
            height,
            cutout_radius: 1.6,
            path: "design.pcbcraft".into(),
            saved_revision: None,
            command: "component.add".into(),
            params: "{\"kind\":\"resistor\",\"position\":{\"x\":30,\"y\":20}}".into(),
            command_open: false,
            capture,
            frames: 0,
        }
    }
    fn refresh(&mut self) {
        let snapshot = self.session.lock().ok().and_then(|s| {
            if s.revision != self.revision {
                Some((s.project.clone(), s.revision, s.checks()))
            } else {
                None
            }
        });
        if let Some((p, r, checks)) = snapshot {
            self.project = p;
            self.revision = r;
            self.checks = checks;
            if self
                .selected
                .is_some_and(|id| !self.project.ids().any(|present| present == id))
            {
                self.selected = None;
            }
            self.width = self.project.board.width;
            self.height = self.project.board.height;
            self.load_properties();
        }
    }
    fn run(&mut self, command: &str, params: Value) {
        let result = self
            .session
            .lock()
            .map_err(|_| "Session unavailable".to_string())
            .and_then(|mut s| {
                s.execute(command, params, Some(self.revision))
                    .map_err(|e| e.to_string())
            });
        match result {
            Ok(v) => {
                self.message = format!("{command} · committed");
                if let Some(id) = v.pointer("/result/id").and_then(Value::as_u64) {
                    self.selected = Some(id);
                }
            }
            Err(e) => self.message = e,
        }
        self.refresh();
    }
    fn tool_call(&mut self, name: &str, args: Value) -> bool {
        let result = pcbcraft_mcp::call_tool(&self.session, name, &args);
        let success = result.get("isError").and_then(Value::as_bool) != Some(true);
        self.message = if success {
            format!("{name} · complete")
        } else {
            result
                .pointer("/content/0/text")
                .and_then(Value::as_str)
                .unwrap_or("operation failed")
                .into()
        };
        self.refresh();
        if success && (name == "save" || name == "open") {
            self.saved_revision = Some(self.revision);
        }
        success
    }
    fn accent(&self) -> Color32 {
        if !self.preferences.dark && self.preferences.accent == [91, 224, 188] {
            return Color32::from_rgb(18, 135, 110);
        }
        Color32::from_rgb(
            self.preferences.accent[0],
            self.preferences.accent[1],
            self.preferences.accent[2],
        )
    }
    fn surface(&self) -> Color32 {
        if self.preferences.dark {
            Color32::from_rgb(20, 29, 43)
        } else {
            Color32::from_rgb(245, 247, 251)
        }
    }
    fn muted(&self) -> Color32 {
        if self.preferences.dark {
            Color32::from_rgb(140, 157, 182)
        } else {
            Color32::from_rgb(88, 104, 128)
        }
    }
    fn ink(&self) -> Color32 {
        if self.preferences.dark {
            Color32::from_rgb(225, 234, 247)
        } else {
            Color32::from_rgb(34, 48, 68)
        }
    }
    fn select(&mut self, id: Option<Id>) {
        self.selected = id;
        self.load_properties();
    }
    fn load_properties(&mut self) {
        if let Some(id) = self.selected {
            if let Some(c) = self.project.component(id) {
                self.value = c.value.clone();
                self.x = c.position.x;
                self.y = c.position.y;
            } else if let Some(f) = self.project.footprint(id) {
                self.x = f.position.x;
                self.y = f.position.y;
                if let Some(c) = self.project.component(f.component) {
                    self.value = c.value.clone();
                }
            }
        }
    }
    fn change_tab(&mut self, tab: Tab) {
        if tab == Tab::Pcb
            && let Some(c) = self.selected.and_then(|id| self.project.component(id))
        {
            self.selected = self
                .project
                .board
                .footprints
                .iter()
                .find(|f| f.component == c.id)
                .map(|f| f.id);
        }
        if tab == Tab::Schematic
            && let Some(f) = self.selected.and_then(|id| self.project.footprint(id))
        {
            self.selected = Some(f.component);
        }
        self.tab = tab;
        self.tool = Tool::Select;
        self.drag = None;
        self.load_properties();
    }
    fn rotate(&mut self) {
        if let Some(id) = self.selected {
            if let Some(c) = self.project.component(id) {
                self.run(
                    "component.rotate",
                    json!({"id":id,"degrees":c.rotation+90.0}),
                );
            } else if let Some(f) = self.project.footprint(id) {
                self.run(
                    "footprint.rotate",
                    json!({"id":id,"degrees":f.rotation+90.0}),
                );
            }
        }
    }
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || ctx.current_pass_index() > 0 {
            return;
        }
        ctx.input(|i| {
            if i.modifiers.command {
                if i.key_pressed(egui::Key::Z) {
                    self.tool_call(if i.modifiers.shift { "redo" } else { "undo" }, json!({}));
                }
                if i.key_pressed(egui::Key::Y) {
                    self.tool_call("redo", json!({}));
                }
                if i.key_pressed(egui::Key::S) {
                    self.tool_call("save", json!({"path":self.path}));
                }
                if i.key_pressed(egui::Key::K) {
                    self.command_open = !self.command_open;
                }
                return;
            }
            if i.key_pressed(egui::Key::A) && self.tab == Tab::Schematic {
                self.palette = true;
            }
            if i.key_pressed(egui::Key::W) && self.tab == Tab::Schematic {
                self.tool = Tool::Wire(None);
            }
            if i.key_pressed(egui::Key::X) && self.tab == Tab::Pcb {
                self.tool = Tool::Route(None);
            }
            if i.key_pressed(egui::Key::M) {
                self.select_hover();
                self.tool = Tool::Move;
            }
            if i.key_pressed(egui::Key::R) {
                self.select_hover();
                self.rotate();
            }
            if i.key_pressed(egui::Key::E) {
                self.select_hover();
                self.appearance = false;
                self.load_properties();
            }
            if i.key_pressed(egui::Key::Home) {
                self.fit();
            }
            if i.key_pressed(egui::Key::Escape) {
                self.tool = Tool::Select;
                self.drag = None;
                self.palette = false;
            }
            if i.key_pressed(egui::Key::Delete)
                && let Some(id) = self.selected
            {
                self.run("object.delete", json!({"id":id}));
                self.select(None);
            }
        });
    }
    fn style(&self, ctx: &egui::Context) {
        let mut visuals = if self.preferences.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        visuals.panel_fill = if self.preferences.dark {
            Color32::from_rgb(17, 24, 36)
        } else {
            Color32::from_rgb(235, 239, 245)
        };
        visuals.window_fill = self.surface();
        visuals.selection.bg_fill = self.accent().gamma_multiply(0.25);
        visuals.selection.stroke = Stroke::new(1.0, self.accent());
        craft_ui::theme::WidgetPalette {
            noninteractive: self.surface(),
            inactive: self.surface(),
            hovered: if self.preferences.dark {
                Color32::from_rgb(37, 53, 73)
            } else {
                Color32::from_rgb(215, 225, 236)
            },
            active: self.accent().gamma_multiply(0.25),
            open: self.surface(),
        }
        .apply_to(
            &mut visuals.widgets,
            egui::CornerRadius::same(6),
            Stroke::new(1.0, self.ink()),
        );
        ctx.set_visuals(visuals);
        ctx.style_mut_of(ctx.theme(), |s| {
            s.spacing.item_spacing = Vec2::new(8.0, 8.0);
            s.spacing.button_padding = Vec2::new(10.0, 6.0);
        });
    }
    fn header(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("header").exact_size(118.0).show(ui, |ui| {
            ui.add_space(7.0);
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.label(
                    egui::RichText::new("PCBCraft")
                        .size(22.0)
                        .strong()
                        .color(self.accent()),
                );
                ui.label(egui::RichText::new("/").color(self.muted()));
                ui.label(egui::RichText::new(&self.project.name).size(16.0));
                if self.saved_revision != Some(self.revision) {
                    ui.label(egui::RichText::new("*").color(self.accent()));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Appearance").clicked() {
                        self.appearance = !self.appearance;
                    }
                    if ui.button("Save  Ctrl+S").clicked() {
                        self.tool_call("save", json!({"path":self.path}));
                    }
                    if ui.button("Redo").clicked() {
                        self.tool_call("redo", json!({}));
                    }
                    if ui.button("Undo").clicked() {
                        self.tool_call("undo", json!({}));
                    }
                    ui.label(
                        egui::RichText::new("MCP FIRST")
                            .size(10.0)
                            .color(self.muted()),
                    );
                });
            });
            ui.add_space(5.0);
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                for (tab, label) in [
                    (Tab::Schematic, "Schematic"),
                    (Tab::Pcb, "PCB Layout"),
                    (Tab::Libraries, "Libraries"),
                    (Tab::Checks, "Checks"),
                ] {
                    let (_, rect) = ui.allocate_space(Vec2::new(132.0, 30.0));
                    let selected = self.tab == tab;
                    let accent = self.accent();
                    let ink = self.ink();
                    let muted = self.muted();
                    let response = craft_ui::tabs::Tab::new(ui.id().with(label), label, selected)
                        .show_at(ui, rect, |ui, response| {
                            if selected || response.hovered() {
                                ui.painter().rect_filled(
                                    rect,
                                    6.0,
                                    accent.gamma_multiply(if selected { 0.16 } else { 0.08 }),
                                );
                            }
                            ui.painter().text(
                                rect.center(),
                                Align2::CENTER_CENTER,
                                label,
                                FontId::proportional(13.0),
                                if selected { ink } else { muted },
                            );
                            if selected {
                                ui.painter().line_segment(
                                    [rect.left_bottom(), rect.right_bottom()],
                                    Stroke::new(2.0, accent),
                                );
                            }
                        });
                    if response.clicked() {
                        self.change_tab(tab);
                    }
                }
                ui.add_enabled(false, egui::Button::new("3D Board"))
                    .on_hover_text("A real Rust 3D viewer is planned.");
            });
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                if ui
                    .selectable_label(matches!(self.tool, Tool::Select), "Select  Esc")
                    .clicked()
                {
                    self.tool = Tool::Select;
                }
                match self.tab {
                    Tab::Schematic => {
                        if ui.button("Add symbol  A").clicked() {
                            self.palette = !self.palette;
                        }
                        if ui
                            .selectable_label(matches!(self.tool, Tool::Wire(_)), "Wire  W")
                            .clicked()
                        {
                            self.tool = Tool::Wire(None);
                        }
                        if ui.button("Sync to PCB >").clicked() {
                            self.run("board.sync", json!({}));
                            self.change_tab(Tab::Pcb);
                        }
                    }
                    Tab::Pcb => {
                        if ui
                            .selectable_label(matches!(self.tool, Tool::Route(_)), "Route  X")
                            .clicked()
                        {
                            self.tool = Tool::Route(None);
                        }
                        if ui.button("Sync schematic").clicked() {
                            self.run("board.sync", json!({}));
                        }
                        if ui
                            .selectable_label(matches!(self.tool, Tool::Cutout), "Circular cutout")
                            .clicked()
                        {
                            self.tool = Tool::Cutout;
                        }
                    }
                    _ => {}
                }
                if ui.button("Move  M").clicked() {
                    self.tool = Tool::Move;
                }
                if ui.button("Rotate  R").clicked() {
                    self.rotate();
                }
                if ui.button("Fit").clicked() {
                    self.fit();
                }
                if ui.button("Commands  Ctrl+K").clicked() {
                    self.command_open = !self.command_open;
                }
            });
        });
    }
    fn fit(&mut self) {
        if self.tab == Tab::Pcb {
            self.pcb.center = Point::new(
                self.project.board.width / 2.0,
                self.project.board.height / 2.0,
            );
            self.pcb.zoom = (700.0 / self.project.board.width.max(self.project.board.height))
                .clamp(1.0, 60.0) as f32;
        } else {
            let (mut xmin, mut ymin, mut xmax, mut ymax) = (
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            );
            for c in &self.project.components {
                xmin = xmin.min(c.position.x - 8.0);
                xmax = xmax.max(c.position.x + 8.0);
                ymin = ymin.min(c.position.y - 6.0);
                ymax = ymax.max(c.position.y + 6.0);
            }
            if xmin.is_finite() {
                self.schematic.center = Point::new((xmin + xmax) / 2.0, (ymin + ymax) / 2.0);
                self.schematic.zoom = (650.0 / (xmax - xmin).max((ymax - ymin) * 1.5).max(10.0))
                    .clamp(2.0, 35.0) as f32;
            }
        }
    }
    fn browser(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("browser")
            .default_size(210.0)
            .min_size(170.0)
            .show(ui, |ui| {
                ui.add_space(12.0);
                ui.label(
                    egui::RichText::new("PROJECT")
                        .size(10.0)
                        .color(self.muted()),
                );
                ui.label(egui::RichText::new(&self.project.name).strong());
                ui.add_space(12.0);
                if self.palette {
                    ui.label(egui::RichText::new("Add symbol").strong());
                    for t in TEMPLATES {
                        if ui.button(t.label).clicked() {
                            self.change_tab(Tab::Schematic);
                            self.tool = Tool::Place(t.kind.into());
                            self.palette = false;
                        }
                    }
                    ui.separator();
                }
                ui.label(
                    egui::RichText::new("COMPONENTS")
                        .size(10.0)
                        .color(self.muted()),
                );
                for c in self.project.components.clone() {
                    let id = if self.tab == Tab::Pcb {
                        self.project
                            .board
                            .footprints
                            .iter()
                            .find(|f| f.component == c.id)
                            .map(|f| f.id)
                            .unwrap_or(c.id)
                    } else {
                        c.id
                    };
                    if ui
                        .selectable_label(
                            self.selected == Some(id),
                            format!("{}   {}", c.reference, c.value),
                        )
                        .clicked()
                    {
                        self.select(Some(id));
                    }
                }
                if self.project.components.is_empty() {
                    ui.label(
                        egui::RichText::new("Press A to place your first symbol")
                            .small()
                            .color(self.muted()),
                    );
                }
                ui.add_space(20.0);
                ui.label(egui::RichText::new("NETS").size(10.0).color(self.muted()));
                for net in &self.project.nets {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("*").color(self.accent()));
                        ui.label(&net.name);
                    });
                }
                ui.add_space(20.0);
                ui.label(egui::RichText::new("BOARD").size(10.0).color(self.muted()));
                ui.label(format!(
                    "{} × {} mm",
                    self.project.board.width, self.project.board.height
                ));
                ui.label("2 copper layers · 1.6 mm");
                ui.label(format!(
                    "{} footprints · {} segments",
                    self.project.board.footprints.len(),
                    self.project.board.tracks.len()
                ));
                ui.add_space(20.0);
                ui.separator();
                ui.label(
                    egui::RichText::new("One project. One window.")
                        .small()
                        .color(self.muted()),
                );
                ui.label(
                    egui::RichText::new("Scroll to zoom · middle-drag to pan")
                        .small()
                        .color(self.muted()),
                );
            });
    }
    fn inspector(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("inspector")
            .default_size(250.0)
            .min_size(220.0)
            .show(ui, |ui| {
                ui.add_space(12.0);
                if self.appearance {
                    ui.heading("Appearance");
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.preferences.dark, true, "Dark");
                        ui.selectable_value(&mut self.preferences.dark, false, "Light");
                    });
                    ui.checkbox(&mut self.preferences.grid, "Show grid");
                    ui.label("Accent color");
                    ui.color_edit_button_srgb(&mut self.preferences.accent);
                    ui.add_space(16.0);
                    ui.label("Appearance is saved automatically.");
                    return;
                }
                ui.label(
                    egui::RichText::new("PROPERTIES")
                        .size(10.0)
                        .color(self.muted()),
                );
                if let Some(id) = self.selected {
                    let c = self.project.component(id).cloned().or_else(|| {
                        self.project
                            .footprint(id)
                            .and_then(|f| self.project.component(f.component))
                            .cloned()
                    });
                    if let Some(c) = c {
                        ui.heading(&c.reference);
                        ui.label(&c.kind);
                        ui.label(
                            egui::RichText::new(format!("Stable ID {id}"))
                                .small()
                                .color(self.muted()),
                        );
                        ui.add_space(10.0);
                        ui.label("Value");
                        ui.text_edit_singleline(&mut self.value);
                        if ui.button("Apply value").clicked() {
                            self.run("component.value", json!({"id":c.id,"value":self.value}));
                        }
                        ui.add_space(12.0);
                        ui.label("Position · mm");
                        ui.horizontal(|ui| {
                            ui.label("X");
                            ui.add(egui::DragValue::new(&mut self.x).speed(0.1));
                            ui.label("Y");
                            ui.add(egui::DragValue::new(&mut self.y).speed(0.1));
                        });
                        if ui.button("Apply position").clicked() {
                            self.run(
                                if self.project.footprint(id).is_some() {
                                    "footprint.move"
                                } else {
                                    "component.move"
                                },
                                json!({"id":id,"position":{"x":self.x,"y":self.y}}),
                            );
                        }
                        if ui.button("Rotate 90°  R").clicked() {
                            self.rotate();
                        }
                        ui.add_space(12.0);
                        ui.label("Pin / net");
                        for pin in &c.pins {
                            ui.label(format!(
                                "{} · {}",
                                pin.number,
                                pin.net
                                    .map(|n| self.project.net_name(n))
                                    .unwrap_or("unassigned")
                            ));
                        }
                    } else {
                        ui.heading("Selected object");
                        ui.label(format!("Stable ID {id}"));
                    }
                    ui.add_space(10.0);
                    if ui.button("Delete  Del").clicked() {
                        self.run("object.delete", json!({"id":id}));
                        self.select(None);
                    }
                    ui.separator();
                } else {
                    ui.heading("Project settings");
                    ui.label("Select an item to inspect and edit it.");
                    ui.add_space(12.0);
                }
                ui.label(
                    egui::RichText::new("BOARD GEOMETRY")
                        .size(10.0)
                        .color(self.muted()),
                );
                ui.horizontal(|ui| {
                    ui.label("W");
                    ui.add(
                        egui::DragValue::new(&mut self.width)
                            .speed(0.5)
                            .suffix(" mm"),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("H");
                    ui.add(
                        egui::DragValue::new(&mut self.height)
                            .speed(0.5)
                            .suffix(" mm"),
                    );
                });
                if ui.button("Apply board dimensions").clicked() {
                    self.run(
                        "board.resize",
                        json!({"width":self.width,"height":self.height}),
                    );
                }
                ui.horizontal(|ui| {
                    ui.label("Cutout radius");
                    ui.add(
                        egui::DragValue::new(&mut self.cutout_radius)
                            .speed(0.1)
                            .suffix(" mm"),
                    );
                });
                ui.add_space(12.0);
                ui.label(egui::RichText::new("CHECKS").size(10.0).color(self.muted()));
                let erc = self
                    .checks
                    .get("erc")
                    .and_then(Value::as_array)
                    .map(Vec::len)
                    .unwrap_or(0);
                let drc = self
                    .checks
                    .get("drc")
                    .and_then(Value::as_array)
                    .map(Vec::len)
                    .unwrap_or(0);
                if ui
                    .button(format!("{erc} electrical · {drc} board >"))
                    .clicked()
                {
                    self.change_tab(Tab::Checks);
                }
                ui.label(
                    egui::RichText::new("Initial checks; full ERC/DRC is in development.")
                        .small()
                        .color(self.muted()),
                );
            });
    }
    fn footer(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(&self.message)
                        .small()
                        .color(self.muted()),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(format!("rev {}", self.revision))
                            .small()
                            .color(self.muted()),
                    );
                    if let Some(p) = self.cursor {
                        ui.label(
                            egui::RichText::new(format!("{:.2}, {:.2} mm", p.x, p.y))
                                .small()
                                .color(self.muted()),
                        );
                    }
                });
            });
        });
        egui::Panel::bottom("files").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Project file");
                ui.add(egui::TextEdit::singleline(&mut self.path).desired_width(260.0));
                if ui.button("Open").clicked() {
                    self.tool_call("open", json!({"path":self.path}));
                }
                if ui.button("Save").clicked() {
                    self.tool_call("save", json!({"path":self.path}));
                }
                if ui.button("Export KiCad board").clicked() {
                    let path = std::path::Path::new(&self.path)
                        .with_extension("kicad_pcb")
                        .to_string_lossy()
                        .into_owned();
                    self.tool_call("export", json!({"path":path,"format":"kicad_pcb"}));
                }
                if ui.button("LED example").clicked() {
                    self.tool_call("sample_design", json!({}));
                    self.fit();
                }
                if ui.button("New project").clicked() {
                    self.tool_call("new_design", json!({}));
                    self.select(None);
                }
            });
        });
        if self.command_open {
            egui::Panel::bottom("commands")
                .resizable(true)
                .default_size(130.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("ENGINE COMMAND")
                                .small()
                                .color(self.accent()),
                        );
                        ui.text_edit_singleline(&mut self.command);
                        ui.add(egui::TextEdit::singleline(&mut self.params).desired_width(500.0));
                        if ui.button("Execute").clicked() {
                            match serde_json::from_str(&self.params) {
                                Ok(v) => self.run(&self.command.clone(), v),
                                Err(e) => self.message = e.to_string(),
                            }
                        }
                    });
                    egui::ScrollArea::vertical()
                        .max_height(85.0)
                        .show(ui, |ui| {
                            for spec in pcbcraft_engine::commands() {
                                if ui
                                    .selectable_label(
                                        self.command == spec.id,
                                        format!("{}   {}", spec.id, spec.description),
                                    )
                                    .clicked()
                                {
                                    self.command = spec.id.into();
                                    self.params = "{}".into();
                                }
                            }
                        });
                });
        }
    }
    fn library(&mut self, ui: &mut egui::Ui) {
        ui.add_space(30.0);
        ui.heading("Component library");
        ui.label(
            "Original symbols and generic circular SMD footprints for the first working circuit.",
        );
        ui.add_space(20.0);
        for t in TEMPLATES {
            egui::Frame::group(ui.style())
                .inner_margin(18.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(t.label).size(18.0).strong());
                            ui.label(format!("{} · {} · 2 pins", t.prefix, t.value));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Place in schematic >").clicked() {
                                self.change_tab(Tab::Schematic);
                                self.tool = Tool::Place(t.kind.into());
                            }
                        });
                    });
                });
            ui.add_space(10.0);
        }
        ui.label(
            egui::RichText::new("KiCad library import and symbol/footprint editing are planned.")
                .color(self.muted()),
        );
    }
    fn checks_view(&mut self, ui: &mut egui::Ui) {
        ui.add_space(24.0);
        ui.heading("Design checks");
        ui.label("Select a finding to inspect its affected object in the editor.");
        ui.add_space(16.0);
        let checks = self.checks.clone();
        for (key, label) in [("erc", "Electrical"), ("drc", "Board")] {
            ui.label(egui::RichText::new(label).size(18.0).strong());
            if let Some(findings) = checks.get(key).and_then(Value::as_array) {
                if findings.is_empty() {
                    ui.label(
                        egui::RichText::new("No findings in the implemented checks")
                            .color(self.accent()),
                    );
                }
                for f in findings {
                    let text = format!(
                        "{}   {}",
                        f.get("code").and_then(Value::as_str).unwrap_or("finding"),
                        f.get("message").and_then(Value::as_str).unwrap_or("")
                    );
                    if ui.button(text).clicked()
                        && let Some(id) = f.pointer("/objects/0").and_then(Value::as_u64)
                    {
                        self.change_tab(if key == "erc" {
                            Tab::Schematic
                        } else {
                            Tab::Pcb
                        });
                        self.select(Some(id));
                    }
                }
            }
            ui.add_space(20.0);
        }
        ui.separator();
        ui.label(
            egui::RichText::new(checks.get("scope").and_then(Value::as_str).unwrap_or(""))
                .small()
                .color(self.muted()),
        );
    }
    fn capture(&mut self, ctx: &egui::Context) {
        self.frames = self.frames.saturating_add(1);
        if self.capture.is_some() && self.frames == 5 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        if let Some(path) = self.capture.clone() {
            let images = ctx.input(|i| {
                i.events
                    .iter()
                    .filter_map(|e| {
                        if let egui::Event::Screenshot { image, .. } = e {
                            Some(image.clone())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            });
            for image in images {
                let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                if let Err(e) = image::save_buffer(
                    &path,
                    &bytes,
                    image.size[0] as u32,
                    image.size[1] as u32,
                    image::ColorType::Rgba8,
                ) {
                    self.message = format!("capture failed: {e}");
                    eprintln!("{}", self.message);
                }
                self.capture = None;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ctx.request_repaint();
        }
    }
}
impl App {
    fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.refresh();
        self.style(&ctx);
        self.shortcuts(&ctx);
        self.header(ui);
        self.footer(ui);
        self.browser(ui);
        self.inspector(ui);
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(self.surface()))
            .show(ui, |ui| match self.tab {
                Tab::Schematic | Tab::Pcb => self.canvas(ui),
                Tab::Libraries => {
                    egui::ScrollArea::vertical()
                        .show(ui, |ui| self.library(ui))
                        .inner
                }
                Tab::Checks => {
                    egui::ScrollArea::vertical()
                        .show(ui, |ui| self.checks_view(ui))
                        .inner
                }
            });
        self.capture(&ctx);
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    }
}
impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        self.show(ui);
    }
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "preferences", &self.preferences);
        if let Ok(session) = self.session.lock()
            && let Ok(text) = serde_json::to_string(&session.project)
            && text.len() <= pcbcraft_io_limit()
        {
            storage.set_string("recovery", text);
        }
    }
}
fn pcbcraft_io_limit() -> usize {
    8 * 1024 * 1024
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(
        ctx: &egui::Context,
        app: &mut App,
        mut events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) -> egui::FullOutput {
        events.insert(0, egui::Event::ModifiersChanged(modifiers));
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(1440.0, 940.0),
                )),
                events,
                ..Default::default()
            },
            |ui| app.show(ui),
        );
        // This input test has no GPU renderer; intentionally discard texture uploads.
        output.textures_delta.clear();
        output
    }
    #[test]
    fn tab_click_preserves_component_identity_and_rotation_undo_works() -> Result<(), String> {
        let session =
            pcbcraft_mcp::shared(pcbcraft_engine::Session::sample().map_err(|e| e.to_string())?);
        let mut app = App::create(session, Preferences::default(), None, None);
        app.refresh();
        app.select(Some(2));
        let ctx = egui::Context::default();
        let output = frame(&ctx, &mut app, vec![], egui::Modifiers::NONE);
        assert_eq!(
            output.viewport_output.len(),
            1,
            "editor must have only its root window"
        );
        let pos = egui::Pos2::new(220.0, 64.0);
        for pressed in [true, false] {
            frame(
                &ctx,
                &mut app,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                egui::Modifiers::NONE,
            );
        }
        assert!(matches!(app.tab, Tab::Pcb));
        assert_eq!(app.selected, Some(11));
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        frame(
            &ctx,
            &mut app,
            vec![key(egui::Key::R, egui::Modifiers::NONE)],
            egui::Modifiers::NONE,
        );
        assert_eq!(app.project.footprint(11).map(|f| f.rotation), Some(90.0));
        let ctrl = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        frame(&ctx, &mut app, vec![key(egui::Key::Z, ctrl)], ctrl);
        assert_eq!(app.project.footprint(11).map(|f| f.rotation), Some(0.0));
        Ok(())
    }
    #[test]
    fn agent_changes_refresh_the_displayed_document() -> Result<(), String> {
        let shared =
            pcbcraft_mcp::shared(pcbcraft_engine::Session::sample().map_err(|e| e.to_string())?);
        let mut app = App::create(shared.clone(), Preferences::default(), None, None);
        app.refresh();
        let result =
            pcbcraft_mcp::call_tool(&shared, "component.value", &json!({"id":2,"value":"470 Ω"}));
        assert_eq!(result.get("isError"), Some(&json!(false)));
        app.refresh();
        assert_eq!(
            app.project.component(2).map(|c| c.value.as_str()),
            Some("470 Ω")
        );
        Ok(())
    }
}
