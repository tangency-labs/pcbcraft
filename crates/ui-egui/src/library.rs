use super::*;
use pcbcraft_library::{FootprintDefinition, SymbolDefinition};

#[derive(Default)]
pub(super) struct Browser {
    pub(super) catalog: Option<Value>,
    pub(super) kind: bool,
    pub(super) path: String,
    pub(super) query: String,
    pub(super) results: Option<Value>,
    pub(super) preview: Option<Value>,
    pub(super) pending: Option<std::sync::mpsc::Receiver<(String, Result<Value, String>)>>,
    error: String,
}
impl Browser {
    fn request(&mut self, ctx: &egui::Context, name: &str, args: Value) {
        if self.pending.is_some() {
            return;
        }
        self.error.clear();
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (tx, rx) = std::sync::mpsc::channel();
            let ctx = ctx.clone();
            let name = name.to_owned();
            match std::thread::Builder::new()
                .name("pcbcraft-library".into())
                .spawn(move || {
                    let result = pcbcraft_engine::library::read_tool(&name, &args);
                    let _ = tx.send((name, result));
                    ctx.request_repaint();
                }) {
                Ok(_) => self.pending = Some(rx),
                Err(e) => self.error = e.to_string(),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (ctx, name, args);
            self.error = "Installed library browsing is available in the desktop build.".into();
            self.catalog = Some(json!({}));
        }
    }
    fn search(&mut self, ctx: &egui::Context) {
        self.preview = None;
        self.request(ctx,"library.search",json!({"path":self.path,"kind":if self.kind {"footprint"} else {"symbol"},"query":self.query,"limit":100}));
    }
    fn poll(&mut self) {
        let response = self.pending.as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some((name, result)) = response {
            self.pending = None;
            match result {
                Ok(v) => match name.as_str() {
                    "library.catalog" => self.catalog = Some(v),
                    "library.search" => self.results = Some(v),
                    _ => self.preview = Some(v),
                },
                Err(e) => {
                    self.error = e;
                    if name == "library.catalog" {
                        self.catalog = Some(json!({}));
                    }
                }
            }
        }
    }
}
impl App {
    pub(super) fn kicad_library(&mut self, ui: &mut egui::Ui) {
        self.library.poll();
        if self.library.catalog.is_none() && self.library.pending.is_none() {
            self.library.request(ui.ctx(), "library.catalog", json!({}));
        }
        ui.heading("KiCad libraries");
        ui.label(
            "Browse installed libraries · definitions used by a project are embedded on placement.",
        );
        let busy = self.library.pending.is_some();
        let before = self.library.kind;
        ui.add_enabled_ui(!busy, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.library.kind, false, "Symbols");
                ui.selectable_value(&mut self.library.kind, true, "Footprints");
                if ui.button("Refresh folders").clicked() {
                    self.library.catalog = None;
                    self.library.path.clear();
                    self.library.results = None;
                    self.library.preview = None;
                }
            });
        });
        if before != self.library.kind {
            self.library.path.clear();
            self.library.results = None;
            self.library.preview = None;
            self.library.query.clear();
        }
        let items = self
            .library
            .catalog
            .as_ref()
            .and_then(|v| {
                v.get(if self.library.kind {
                    "footprint_libraries"
                } else {
                    "symbol_libraries"
                })
            })
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut changed = false;
        if self.library.path.is_empty() && !items.is_empty() {
            let preferred = if self.library.kind {
                "Resistor_SMD"
            } else {
                "Device"
            };
            let item = items
                .iter()
                .find(|v| v.get("name").and_then(Value::as_str) == Some(preferred))
                .or_else(|| items.first());
            if let Some(path) = item.and_then(|v| v.get("path")).and_then(Value::as_str) {
                self.library.path = path.into();
                changed = true;
            }
        }
        ui.add_enabled_ui(!busy, |ui| {
            let label = std::path::Path::new(&self.library.path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Choose library");
            egui::ComboBox::from_id_salt("kicad-library")
                .selected_text(label)
                .width(300.0)
                .show_ui(ui, |ui| {
                    for item in &items {
                        if let (Some(name), Some(path)) = (
                            item.get("name").and_then(Value::as_str),
                            item.get("path").and_then(Value::as_str),
                        ) {
                            changed |= ui
                                .selectable_value(&mut self.library.path, path.to_owned(), name)
                                .changed();
                        }
                    }
                });
            ui.horizontal(|ui| {
                ui.label("Library path");
                ui.add(egui::TextEdit::singleline(&mut self.library.path).desired_width(480.0));
            });
            ui.horizontal(|ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.library.query)
                        .hint_text("Search names, descriptions, keywords")
                        .desired_width(360.0),
                );
                if ui.button("Search").clicked()
                    || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                {
                    changed = true;
                }
            });
        });
        if changed && !busy {
            self.library.search(ui.ctx());
        }
        if busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Reading library…");
            });
        }
        if !self.library.error.is_empty() {
            ui.colored_label(Color32::from_rgb(237, 135, 113), &self.library.error);
        }
        let results = self.library.results.clone().unwrap_or(json!({}));
        if self
            .library
            .catalog
            .as_ref()
            .and_then(|v| v.get("scan_truncated"))
            .and_then(Value::as_bool)
            == Some(true)
        {
            ui.label("Some library folders exceeded the scan limit; use a specific library path.");
        }
        ui.columns(2,|cols| {
            let left=&mut cols[0];
            if let Some(total)=results.get("total").and_then(Value::as_u64) {left.label(format!("{total} matches in scanned entries · showing up to 100"));}
            if results.get("scan_truncated").and_then(Value::as_bool)==Some(true) {left.label("Directory scan reached its limit; more entries may exist.");}
            egui::ScrollArea::vertical().id_salt("library-results").max_height(360.0).show(left,|ui| {
                for item in results.get("entries").and_then(Value::as_array).into_iter().flatten() {
                    let name=item.get("name").and_then(Value::as_str).unwrap_or("");
                    if ui.add_enabled(!busy,egui::Button::new(name).selected(self.library.preview.as_ref().and_then(|v|v.pointer("/definition/name")).and_then(Value::as_str)==Some(name))).clicked() {
                        self.library.preview=None;
                        let path=item.get("path").and_then(Value::as_str).unwrap_or(&self.library.path);
                        let (tool,args)=if self.library.kind {("library.inspect_footprint",json!({"path":path}))} else {("library.inspect_symbol",json!({"path":path,"name":name}))};
                        self.library.request(ui.ctx(),tool,args);
                    }
                }
            });
            let right=&mut cols[1];
            if let Some(preview)=self.library.preview.clone() {
                let path=preview.get("path").and_then(Value::as_str).unwrap_or("").to_owned();
                if self.library.kind {
                    if let Some(def)=preview.get("definition").and_then(|v|serde_json::from_value::<FootprintDefinition>(v.clone()).ok()) {
                        right.heading(&def.name);right.label(&def.description);
                        self.library_preview(right,None,Some(&def));
                        right.label(format!("{} pads · {} graphics",def.pads.len(),def.graphics.len()));
                        let supported=pcbcraft_doc::validate_footprint_support(&def);
                        if let Err(e)=&supported {right.label(e);}
                        let component=self.selected.and_then(|id|self.project.component(id)).map(|c|(c.id,c.reference.clone()));
                        if let Some((id,reference))=component {
                            let matches=self.project.component(id).is_some_and(|c|c.pins.iter().map(|p|p.number.as_str()).collect::<std::collections::BTreeSet<_>>()==def.pads.iter().map(|p|p.number.as_str()).collect::<std::collections::BTreeSet<_>>());
                            if !matches {right.label("Pad numbers must match every pin of the selected symbol.");}
                            if right.add_enabled(supported.is_ok() && matches,egui::Button::new(format!("Assign to {reference}"))).clicked() {self.run("footprint.assign",json!({"id":id,"path":path}));}
                        } else {right.label("Select a schematic component before assigning its footprint.");}
                    }
                } else if let Some(def)=preview.get("definition").and_then(|v|serde_json::from_value::<SymbolDefinition>(v.clone()).ok()) {
                    right.heading(&def.name);right.label(&def.description);
                    self.library_preview(right,Some(&def),None);
                    let supported=def.unsupported.is_empty() && !def.pins.iter().any(|p|p.unit>1) && !def.pins.is_empty() && def.pins.len()<=64;
                    right.label(format!("{} pins · {} units",def.pins.len(),def.pins.iter().map(|p|p.unit).max().unwrap_or(1)));
                    right.label(format!("Suggested footprint: {}",def.footprint));
                    if right.add_enabled(supported,egui::Button::new("Place in schematic →")).clicked() {
                        self.change_tab(Tab::Schematic);self.tool=Tool::LibraryPlace(path,def.name.clone(),Box::new(def));
                    }
                    if !supported {right.label("Inspection available; multi-unit or unsupported pin/graphic placement is planned.");}
                }
            } else {right.label("Choose an entry to preview its real geometry.");}
        });
        ui.separator();
        ui.hyperlink_to(
            "KiCad libraries: CC-BY-SA 4.0 + design exception",
            "https://www.kicad.org/libraries/license/",
        );
        ui.label("Initial placement: single-unit symbols and matching front SMD footprints. Library editing and through-hole placement are planned.");
    }
    fn library_preview(
        &self,
        ui: &mut egui::Ui,
        symbol: Option<&SymbolDefinition>,
        footprint: Option<&FootprintDefinition>,
    ) {
        let (response, painter) =
            ui.allocate_painter(Vec2::new(ui.available_width(), 180.0), egui::Sense::hover());
        let mut paths: Vec<Vec<Point>> = vec![];
        if let Some(s) = symbol {
            paths.extend(
                s.graphics
                    .iter()
                    .filter(|g| g.unit <= 1 && g.body_style <= 1)
                    .map(|g| g.primitive.path_points()),
            );
            paths.extend(
                s.pins
                    .iter()
                    .filter(|p| p.unit <= 1 && p.body_style <= 1)
                    .map(|p| {
                        vec![
                            p.position,
                            p.position.offset(Point::new(
                                p.length * p.angle.to_radians().cos(),
                                -p.length * p.angle.to_radians().sin(),
                            )),
                        ]
                    }),
            );
        }
        if let Some(f) = footprint {
            paths.extend(f.graphics.iter().map(|g| g.path_points()));
            paths.extend(f.pads.iter().map(|p| {
                pcbcraft_doc::Pad {
                    number: p.number.clone(),
                    offset: p.position,
                    diameter: p.size.x.min(p.size.y),
                    net: None,
                    geometry: Some(p.clone()),
                }
                .outline_points()
            }));
        }
        let extent = paths.iter().flatten().fold(Point::new(1.0, 1.0), |m, p| {
            Point::new(m.x.max(p.x.abs()), m.y.max(p.y.abs()))
        });
        let camera = Camera {
            center: Point::default(),
            zoom: ((response.rect.width() - 24.0) / (extent.x as f32 * 2.0))
                .min(150.0 / (extent.y as f32 * 2.0))
                .clamp(0.01, 40.0),
        };
        painter.rect_filled(response.rect, 6.0, self.surface());
        for path in paths.into_iter().filter(|p| p.len() > 1) {
            painter.add(egui::Shape::line(
                path.into_iter()
                    .map(|p| camera.screen(p, response.rect))
                    .collect(),
                Stroke::new(1.5, self.accent()),
            ));
        }
    }
}
