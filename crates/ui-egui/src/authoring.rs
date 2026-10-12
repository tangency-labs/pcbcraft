use super::*;
use pcbcraft_library::{
    Drill, FootprintDefinition, FootprintPad, Graphic, SymbolDefinition, SymbolPin, UnitGraphic,
};

pub(super) enum Draft {
    Symbol(SymbolDefinition),
    Footprint(FootprintDefinition),
}

pub(super) struct Editor {
    pub mode: u8,
    pub draft: Option<Draft>,
    pub path: String,
    pub notice: Option<String>,
}
impl Default for Editor {
    fn default() -> Self {
        Self {
            mode: 0,
            draft: None,
            path: "parts.pcbcraft-lib".into(),
            notice: None,
        }
    }
}
pub(super) fn initial_editor(tab: Option<&str>, project: &Project) -> Editor {
    match tab {
        Some("project-library") => Editor {
            mode: 1,
            ..Default::default()
        },
        Some("symbol-editor") => Editor {
            mode: 2,
            draft: Some(Draft::Symbol(
                project
                    .library
                    .symbols
                    .first()
                    .cloned()
                    .unwrap_or_else(new_symbol),
            )),
            ..Default::default()
        },
        Some("footprint-editor") => Editor {
            mode: 2,
            draft: Some(Draft::Footprint(
                project
                    .library
                    .footprints
                    .first()
                    .cloned()
                    .unwrap_or_else(new_footprint),
            )),
            ..Default::default()
        },
        _ => Editor::default(),
    }
}
fn rectangle(layer: &str) -> Graphic {
    Graphic::Rectangle {
        start: Point::new(-3.0, -2.0),
        end: Point::new(3.0, 2.0),
        width: 0.15,
        fill: "none".into(),
        layer: layer.into(),
    }
}
fn new_symbol() -> SymbolDefinition {
    SymbolDefinition {
        name: "NewSymbol".into(),
        reference: "U".into(),
        value: "NewSymbol".into(),
        description: "Custom symbol".into(),
        keywords: String::new(),
        footprint: String::new(),
        extends: None,
        raw_source: String::new(),
        unsupported: vec![],
        pins: vec![
            new_pin("1", 1, Point::new(-5.0, 0.0), 0.0),
            new_pin("2", 1, Point::new(5.0, 0.0), 180.0),
        ],
        graphics: vec![UnitGraphic {
            unit: 0,
            body_style: 1,
            primitive: rectangle(""),
        }],
    }
}
fn new_pin(number: &str, unit: u32, position: Point, angle: f64) -> SymbolPin {
    SymbolPin {
        number: number.into(),
        name: String::new(),
        position,
        angle,
        length: 2.0,
        electrical_type: "passive".into(),
        unit,
        body_style: 1,
    }
}
fn new_pad(number: &str, x: f64) -> FootprintPad {
    FootprintPad {
        number: number.into(),
        pad_type: "thru_hole".into(),
        shape: "circle".into(),
        position: Point::new(x, 0.0),
        rotation: 0.0,
        size: Point::new(1.8, 1.8),
        layers: vec!["*.Cu".into(), "*.Mask".into()],
        drill: Some(Drill {
            size: Point::new(0.8, 0.8),
            offset: Point::default(),
        }),
        roundrect_ratio: None,
        raw_source: String::new(),
    }
}
fn new_footprint() -> FootprintDefinition {
    FootprintDefinition {
        name: "NewFootprint".into(),
        description: "Custom through-hole package".into(),
        keywords: String::new(),
        pads: vec![new_pad("1", -2.5), new_pad("2", 2.5)],
        graphics: vec![rectangle("F.SilkS")],
        raw_source: String::new(),
        unsupported: vec![],
    }
}
fn point(ui: &mut egui::Ui, label: &str, p: &mut Point) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::DragValue::new(&mut p.x).speed(0.1).prefix("X "));
        ui.add(egui::DragValue::new(&mut p.y).speed(0.1).prefix("Y "));
    });
}
fn text(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.text_edit_singleline(value);
    });
}
fn number(ui: &mut egui::Ui, label: &str, value: &mut f64) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::DragValue::new(value).speed(0.1));
    });
}
fn graphic(ui: &mut egui::Ui, g: &mut Graphic) {
    match g {
        Graphic::Line {
            start,
            end,
            width,
            layer,
        } => {
            point(ui, "Start", start);
            point(ui, "End", end);
            number(ui, "Stroke", width);
            text(ui, "Layer", layer);
        }
        Graphic::Rectangle {
            start,
            end,
            width,
            fill,
            layer,
        } => {
            point(ui, "Start", start);
            point(ui, "End", end);
            number(ui, "Stroke", width);
            text(ui, "Fill", fill);
            text(ui, "Layer", layer);
        }
        Graphic::Circle {
            center,
            radius,
            width,
            fill,
            layer,
        } => {
            point(ui, "Center", center);
            number(ui, "Radius", radius);
            number(ui, "Stroke", width);
            text(ui, "Fill", fill);
            text(ui, "Layer", layer);
        }
        Graphic::Arc {
            start,
            mid,
            end,
            width,
            layer,
        } => {
            point(ui, "Start", start);
            point(ui, "Through", mid);
            point(ui, "End", end);
            number(ui, "Stroke", width);
            text(ui, "Layer", layer);
        }
        Graphic::Polyline {
            points,
            width,
            fill,
            layer,
        } => {
            for (i, p) in points.iter_mut().enumerate().take(128) {
                point(ui, &format!("Vertex {}", i + 1), p);
            }
            if points.len() > 128 {
                ui.label("Remaining vertices are preserved.");
            }
            if ui.button("Add vertex").clicked() && points.len() < 128 {
                points.push(Point::default());
            }
            if ui.button("Remove last vertex").clicked() && points.len() > 2 {
                points.pop();
            }
            number(ui, "Stroke", width);
            text(ui, "Fill", fill);
            text(ui, "Layer", layer);
        }
        Graphic::Text {
            position,
            angle,
            text: content,
            size,
            layer,
        } => {
            point(ui, "Position", position);
            text(ui, "Text", content);
            point(ui, "Text size", size);
            number(ui, "Angle", angle);
            text(ui, "Layer", layer);
        }
    }
}
fn add_graphic(ui: &mut egui::Ui, layer: &str) -> Option<Graphic> {
    let mut result = None;
    ui.horizontal_wrapped(|ui| {
        ui.label("Add drawing:");
        if ui.button("Line").clicked() {
            result = Some(Graphic::Line {
                start: Point::new(-3.0, 0.0),
                end: Point::new(3.0, 0.0),
                width: 0.15,
                layer: layer.into(),
            });
        }
        if ui.button("Rectangle").clicked() {
            result = Some(rectangle(layer));
        }
        if ui.button("Circle").clicked() {
            result = Some(Graphic::Circle {
                center: Point::default(),
                radius: 2.0,
                width: 0.15,
                fill: "none".into(),
                layer: layer.into(),
            });
        }
        if ui.button("Arc").clicked() {
            result = Some(Graphic::Arc {
                start: Point::new(-3.0, 0.0),
                mid: Point::new(0.0, -3.0),
                end: Point::new(3.0, 0.0),
                width: 0.15,
                layer: layer.into(),
            });
        }
        if ui.button("Polyline").clicked() {
            result = Some(Graphic::Polyline {
                points: vec![
                    Point::new(-2.0, 2.0),
                    Point::new(0.0, -2.0),
                    Point::new(2.0, 2.0),
                ],
                width: 0.15,
                fill: "none".into(),
                layer: layer.into(),
            });
        }
        if ui.button("Text").clicked() {
            result = Some(Graphic::Text {
                position: Point::default(),
                angle: 0.0,
                text: "Label".into(),
                size: Point::new(1.0, 1.0),
                layer: layer.into(),
            });
        }
    });
    result
}
impl App {
    pub(super) fn library_workspaces(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.library.editor.mode, 0, "Installed libraries");
            ui.selectable_value(&mut self.library.editor.mode, 1, "Project library");
            ui.selectable_value(&mut self.library.editor.mode, 2, "Editor");
            if ui.button("New symbol").clicked() {
                self.library.editor.draft = Some(Draft::Symbol(new_symbol()));
                self.library.editor.mode = 2;
                self.library.editor.notice = None;
            }
            if ui.button("New footprint").clicked() {
                self.library.editor.draft = Some(Draft::Footprint(new_footprint()));
                self.library.editor.mode = 2;
                self.library.editor.notice = None;
            }
        });
        ui.separator();
        match self.library.editor.mode {
            0 => self.kicad_library(ui),
            1 => self.project_library(ui),
            _ => self.library_editor(ui),
        }
    }
    fn project_library(&mut self, ui: &mut egui::Ui) {
        ui.heading("Editable project library");
        ui.label("Saved definitions live in this project. Placed components retain their embedded copies.");
        ui.horizontal(|ui| {
            ui.label("Collection file");
            ui.text_edit_singleline(&mut self.library.editor.path);
            if ui.button("Save library").clicked() {
                self.tool_call("library.save", json!({"path":self.library.editor.path}));
            }
            if ui.button("Load library").clicked() {
                self.run("library.load", json!({"path":self.library.editor.path}));
            }
        });
        ui.separator();
        ui.heading("Symbols");
        for def in self.project.library.symbols.clone() {
            ui.horizontal_wrapped(|ui| {
                ui.strong(&def.name);
                ui.label(format!("{} physical pins", def.pins.len()));
                if ui.button("Edit").clicked() {
                    self.library.editor.draft = Some(Draft::Symbol(def.clone()));
                    self.library.editor.mode = 2;
                    self.library.editor.notice = None;
                }
                let supported = pcbcraft_doc::library_component(
                    def.clone(),
                    "project".into(),
                    0,
                    "preview".into(),
                    Point::default(),
                    None,
                )
                .is_ok();
                if ui
                    .add_enabled(supported, egui::Button::new("Place"))
                    .clicked()
                {
                    self.change_tab(Tab::Schematic);
                    self.tool = Tool::CustomPlace(def.name.clone(), Box::new(def.clone()));
                }
                if ui.button("Remove").clicked() {
                    self.run("library.symbol.remove", json!({"name":def.name}));
                }
            });
        }
        ui.separator();
        ui.heading("Footprints");
        for def in self.project.library.footprints.clone() {
            ui.horizontal_wrapped(|ui| {
                ui.strong(&def.name);
                ui.label(format!("{} pads", def.pads.len()));
                if ui.button("Edit").clicked() {
                    self.library.editor.draft = Some(Draft::Footprint(def.clone()));
                    self.library.editor.mode = 2;
                    self.library.editor.notice = None;
                }
                let selected = self.selected.and_then(|id| self.project.component(id));
                let matching = selected.is_some_and(|c| {
                    c.pins
                        .iter()
                        .map(|p| p.number.as_str())
                        .collect::<std::collections::BTreeSet<_>>()
                        == def
                            .pads
                            .iter()
                            .map(|p| p.number.as_str())
                            .collect::<std::collections::BTreeSet<_>>()
                });
                if ui
                    .add_enabled(
                        matching && pcbcraft_doc::validate_footprint_support(&def).is_ok(),
                        egui::Button::new("Assign to selection"),
                    )
                    .clicked()
                    && let Some(id) = selected.map(|c| c.id)
                {
                    self.run("footprint.assign_custom", json!({"id":id,"name":def.name}));
                }
                if ui.button("Remove").clicked() {
                    self.run("library.footprint.remove", json!({"name":def.name}));
                }
            });
        }
    }
    fn library_editor(&mut self, ui: &mut egui::Ui) {
        let Some(mut draft) = self.library.editor.draft.take() else {
            ui.heading("Library editor");
            ui.label(
                "Create a symbol or footprint, or edit a copy from the installed/project library.",
            );
            return;
        };
        let mut save = None;
        ui.label("Dimensions are millimetres; angles are counterclockwise degrees. Save commits one undoable edit.");
        match &mut draft {
            Draft::Symbol(def) => {
                ui.heading("Symbol editor");
                text(ui, "Name", &mut def.name);
                text(ui, "Reference prefix", &mut def.reference);
                text(ui, "Default value", &mut def.value);
                text(ui, "Description", &mut def.description);
                text(ui, "Suggested footprint", &mut def.footprint);
                self.library_preview(ui, Some(def), None);
                if let Err(e) = def.validate() {
                    ui.colored_label(Color32::LIGHT_RED, e.to_string());
                }
                if !def.unsupported.is_empty() {
                    ui.label(format!(
                        "Preserved unsupported features: {}",
                        def.unsupported.join(", ")
                    ));
                }
                if ui.button("Save symbol to project library").clicked() {
                    save = Some(("library.symbol.upsert", json!({"definition":def})));
                }
                ui.separator();
                ui.heading("Pins");
                if ui.button("Duplicate preview unit").clicked() {
                    let unit = def
                        .pins
                        .iter()
                        .map(|p| p.unit)
                        .chain(def.graphics.iter().map(|g| g.unit))
                        .max()
                        .unwrap_or(1)
                        + 1;
                    let source = self.library.preview_unit.max(1);
                    if unit <= 32 {
                        let mut pins: Vec<_> = def
                            .pins
                            .iter()
                            .filter(|p| p.unit == source)
                            .cloned()
                            .collect();
                        let mut numbers: std::collections::BTreeSet<_> =
                            def.pins.iter().map(|p| p.number.clone()).collect();
                        for pin in &mut pins {
                            let mut n = 1;
                            while numbers.contains(&n.to_string()) {
                                n += 1;
                            }
                            pin.number = n.to_string();
                            numbers.insert(pin.number.clone());
                            pin.unit = unit;
                        }
                        def.pins.extend(pins);
                        let graphics: Vec<_> = def
                            .graphics
                            .iter()
                            .filter(|g| g.unit == source)
                            .cloned()
                            .map(|mut g| {
                                g.unit = unit;
                                g
                            })
                            .collect();
                        def.graphics.extend(graphics);
                        self.library.preview_unit = unit;
                    }
                }
                if ui.button("Add pin").clicked() && def.pins.len() < 64 {
                    let mut n = 1;
                    while def.pins.iter().any(|p| p.number == n.to_string()) {
                        n += 1;
                    }
                    def.pins.push(new_pin(
                        &n.to_string(),
                        self.library.preview_unit.max(1),
                        Point::new(-5.0, 0.0),
                        0.0,
                    ));
                }
                let mut remove = None;
                for (i, pin) in def.pins.iter_mut().enumerate().take(64) {
                    ui.push_id(i, |ui| {
                        ui.collapsing(format!("Pin {} · unit {}", pin.number, pin.unit), |ui| {
                            text(ui, "Number", &mut pin.number);
                            text(ui, "Name", &mut pin.name);
                            point(ui, "Connection", &mut pin.position);
                            number(ui, "Angle", &mut pin.angle);
                            number(ui, "Length", &mut pin.length);
                            ui.add(
                                egui::DragValue::new(&mut pin.unit)
                                    .range(1..=32)
                                    .prefix("Unit "),
                            );
                            egui::ComboBox::from_id_salt("pin-type")
                                .selected_text(&pin.electrical_type)
                                .show_ui(ui, |ui| {
                                    for t in [
                                        "passive",
                                        "input",
                                        "output",
                                        "bidirectional",
                                        "tri_state",
                                        "power_in",
                                        "power_out",
                                        "open_collector",
                                        "open_emitter",
                                        "no_connect",
                                    ] {
                                        ui.selectable_value(&mut pin.electrical_type, t.into(), t);
                                    }
                                });
                            if ui.button("Delete pin").clicked() {
                                remove = Some(i);
                            }
                        });
                    });
                }
                if let Some(i) = remove {
                    def.pins.remove(i);
                }
                ui.separator();
                ui.heading("Drawing");
                if let Some(primitive) = add_graphic(ui, "") {
                    def.graphics.push(UnitGraphic {
                        unit: self.library.preview_unit.max(1),
                        body_style: 1,
                        primitive,
                    });
                }
                let mut remove = None;
                for (i, g) in def.graphics.iter_mut().enumerate().take(256) {
                    ui.push_id(("symbol-graphic", i), |ui| {
                        ui.collapsing(format!("Graphic {} · unit {}", i + 1, g.unit), |ui| {
                            ui.add(
                                egui::DragValue::new(&mut g.unit)
                                    .range(0..=32)
                                    .prefix("Unit (0 shared) "),
                            );
                            graphic(ui, &mut g.primitive);
                            if ui.button("Delete graphic").clicked() {
                                remove = Some(i);
                            }
                        });
                    });
                }
                if let Some(i) = remove {
                    def.graphics.remove(i);
                }
            }
            Draft::Footprint(def) => {
                ui.heading("Footprint editor");
                text(ui, "Name", &mut def.name);
                text(ui, "Description", &mut def.description);
                self.library_preview(ui, None, Some(def));
                if let Err(e) = def.validate() {
                    ui.colored_label(Color32::LIGHT_RED, e.to_string());
                }
                if let Err(e) = pcbcraft_doc::validate_footprint_support(def) {
                    ui.label(e);
                }
                if ui.button("Save footprint to project library").clicked() {
                    save = Some(("library.footprint.upsert", json!({"definition":def})));
                }
                ui.separator();
                ui.heading("Pads");
                if ui.button("Add pad").clicked() && def.pads.len() < 64 {
                    let mut n = 1;
                    while def.pads.iter().any(|p| p.number == n.to_string()) {
                        n += 1;
                    }
                    def.pads.push(new_pad(&n.to_string(), 0.0));
                }
                let mut remove = None;
                for (i, pad) in def.pads.iter_mut().enumerate().take(64) {
                    ui.push_id(("pad", i), |ui| {
                        ui.collapsing(format!("Pad {} · {}", pad.number, pad.pad_type), |ui| {
                            text(ui, "Number", &mut pad.number);
                            point(ui, "Position", &mut pad.position);
                            point(ui, "Copper size", &mut pad.size);
                            number(ui, "Angle", &mut pad.rotation);
                            let before = pad.pad_type.clone();
                            egui::ComboBox::from_id_salt("pad-type")
                                .selected_text(&pad.pad_type)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut pad.pad_type,
                                        "smd".into(),
                                        "Front SMD",
                                    );
                                    ui.selectable_value(
                                        &mut pad.pad_type,
                                        "thru_hole".into(),
                                        "Plated through-hole",
                                    );
                                });
                            if before != pad.pad_type {
                                if pad.pad_type == "thru_hole" {
                                    pad.layers = vec!["*.Cu".into(), "*.Mask".into()];
                                    pad.drill = Some(Drill {
                                        size: Point::new(0.8, 0.8),
                                        offset: Point::default(),
                                    });
                                } else {
                                    pad.layers =
                                        vec!["F.Cu".into(), "F.Paste".into(), "F.Mask".into()];
                                    pad.drill = None;
                                }
                            }
                            egui::ComboBox::from_id_salt("pad-shape")
                                .selected_text(&pad.shape)
                                .show_ui(ui, |ui| {
                                    for shape in ["circle", "rect", "oval", "roundrect"] {
                                        ui.selectable_value(&mut pad.shape, shape.into(), shape);
                                    }
                                });
                            if pad.shape == "roundrect" {
                                ui.add(
                                    egui::DragValue::new(pad.roundrect_ratio.get_or_insert(0.25))
                                        .range(0.0..=0.5)
                                        .speed(0.01)
                                        .prefix("Corner ratio "),
                                );
                            } else {
                                pad.roundrect_ratio = None;
                            }
                            if let Some(drill) = &mut pad.drill {
                                point(ui, "Drill size (oval if unequal)", &mut drill.size);
                                point(ui, "Drill offset", &mut drill.offset);
                            }
                            ui.label(format!("Layers: {}", pad.layers.join(", ")));
                            if ui.button("Delete pad").clicked() {
                                remove = Some(i);
                            }
                        });
                    });
                }
                if let Some(i) = remove {
                    def.pads.remove(i);
                }
                ui.separator();
                ui.heading("Drawing");
                if let Some(g) = add_graphic(ui, "F.SilkS") {
                    def.graphics.push(g);
                }
                let mut remove = None;
                for (i, g) in def.graphics.iter_mut().enumerate().take(256) {
                    ui.push_id(("footprint-graphic", i), |ui| {
                        ui.collapsing(format!("Graphic {}", i + 1), |ui| {
                            graphic(ui, g);
                            if ui.button("Delete graphic").clicked() {
                                remove = Some(i);
                            }
                        });
                    });
                }
                if let Some(i) = remove {
                    def.graphics.remove(i);
                }
            }
        }
        self.library.editor.draft = Some(draft);
        if let Some((command, mut params)) = save {
            if let Some(notice) = &self.library.editor.notice {
                params["notice"] = json!(format!(
                    "Derived entry saved as {}.\n{}",
                    params
                        .pointer("/definition/name")
                        .and_then(Value::as_str)
                        .unwrap_or("entry"),
                    notice
                ));
            }
            self.run(command, params);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) -> egui::FullOutput {
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
        output.textures_delta.clear();
        output
    }
    #[test]
    fn editor_button_commits_one_undoable_edit_and_retains_original_nodes() -> Result<(), String> {
        let shared = pcbcraft_mcp::shared(pcbcraft_engine::Session::default());
        let mut app = App::create(
            shared.clone(),
            Preferences::default(),
            None,
            Some("symbol-editor"),
        );
        app.refresh();
        let mut definition = new_symbol();
        definition.raw_source = "(original unknown metadata)".into();
        definition.unsupported = vec!["unknown original expression".into()];
        app.library.editor.draft = Some(Draft::Symbol(definition.clone()));
        app.library.editor.notice = Some("Original source and data license retained".into());
        let ctx = egui::Context::default();
        let output = frame(&ctx, &mut app, vec![]);
        assert_eq!(output.viewport_output.len(), 1);
        let pos = output
            .shapes
            .iter()
            .find_map(|s| {
                if let egui::Shape::Text(t) = &s.shape {
                    if t.galley.text() == "Save symbol to project library" {
                        Some(t.pos + t.galley.size() / 2.0)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .ok_or("save button not rendered")?;
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
            );
        }
        let session = shared.lock().map_err(|_| "session unavailable")?;
        assert_eq!(session.project.library.symbols, vec![definition]);
        assert_eq!(session.project.library.notices.len(), 1);
        assert_eq!(session.revision, 1);
        drop(session);
        app.tool_call("undo", json!({}));
        assert!(app.project.library.symbols.is_empty());
        app.tool_call("redo", json!({}));
        assert_eq!(app.project.library.symbols.len(), 1);
        Ok(())
    }
}
