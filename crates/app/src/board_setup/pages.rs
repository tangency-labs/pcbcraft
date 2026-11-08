//! Layer catalog and physical-stackup presentation over one disposable candidate.
use super::*;

/// The schema and engine use this same bounded even range; presets do not limit the editor to 2/4.
fn copper_count(ui: &mut Ui, count: &mut u64) -> bool {
    let previous = *count;
    form::field(ui, "Copper layers", |ui| {
        egui::ComboBox::from_id_salt("copper_count")
            .selected_text(format!("{count} copper layers"))
            .show_ui(ui, |ui| {
                for n in (2..=64).step_by(2) {
                    ui.selectable_value(count, n, format!("{n} copper layers"));
                }
            })
            .response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Copper layers"));
    });
    *count != previous
}

/// Material identities remain pinned by ID/revision; None never invents an FR-4 specification.
fn material_choice(ui: &mut Ui, id: impl egui::AsIdSalt, reference: &mut Option<MaterialRef>, materials: &[Material]) {
    let text = reference
        .and_then(|r| materials.iter().find(|m| m.reference() == r))
        .map(|m| format!("{} · r{}", m.name, m.revision))
        .unwrap_or_else(|| "Unspecified".into());
    egui::ComboBox::from_id_salt(id)
        .selected_text(text)
        .show_ui(ui, |ui| {
            ui.selectable_value(reference, None, "Unspecified");
            for material in materials {
                ui.selectable_value(reference, Some(material.reference()), format!("{} · r{}", material.name, material.revision));
            }
        })
        .response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Material specification"));
}

impl Edit {
    /// Board-wide print dimensions stay staged with layers and stackup until one atomic Apply.
    /// Invalid fields and preview failures are visible immediately beside the form controls.
    pub(super) fn silkscreen(&mut self, ui: &mut Ui) {
        form::heading(ui, "Silkscreen");
        properties::split(ui, |ui, section| {
            if section == 0 {
                let [height, stroke, clearance] = &mut self.silkscreen;
                form::silkscreen(ui, [&mut height.text, &mut stroke.text, &mut clearance.text]);
            } else {
                let (style, preview) =
                    match self.silk_style().and_then(|style| pcbcraft_engine::board::silkscreen::preview(style).map(|p| (style, p))) {
                        Ok(value) => value,
                        Err(error) => {
                            properties::section(ui, "Preview");
                            ui.label(pcbcraft_chrome::shell::error(&error));
                            return;
                        }
                    };
                let paths = preview
                    .paths
                    .iter()
                    .map(|path| path.iter().map(|p| [p.x_nm as f32 / 1_000_000., p.y_nm as f32 / 1_000_000.]).collect())
                    .collect::<Vec<_>>();
                let pads = preview
                    .pads
                    .iter()
                    .map(|p| {
                        [
                            p.center.x_nm as f32 / 1_000_000.,
                            p.center.y_nm as f32 / 1_000_000.,
                            p.width_nm as f32 / 1_000_000.,
                            p.height_nm as f32 / 1_000_000.,
                        ]
                    })
                    .collect::<Vec<_>>();
                form::silkscreen_preview(ui, &paths, &pads, style.stroke_width_nm as f32 / 1_000_000., style.pad_clearance_nm as f32 / 1_000_000.);
            }
        });
    }

    /// Stage copper-count changes before catalog/physical edits; failed resizing keeps every field.
    fn count(&mut self, ui: &mut Ui) {
        let candidate = &self.candidate;
        let mut count = candidate.setup.stackup().strata.len().div_ceil(2) as u64;
        if copper_count(ui, &mut count) {
            match candidate.value(self.unit).and_then(|s| s.with_copper_count(count).map_err(|e| e.to_string())) {
                Ok(setup) => {
                    self.candidate = Candidate::new(setup, self.unit);
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
        if self.populated && self.candidate.setup.stackup().strata.len().div_ceil(2) as u64 != self.original_copper {
            ui.label("Changing the copper count updates existing through-hole pad spans. Surface-mount pads retain front copper.");
        }
    }

    /// Catalog edits are staged as membership changes; occupied drawing layers are refused by
    /// shared engine validation. Required physical/outline roles remain present in this editor.
    pub(super) fn layers(&mut self, ui: &mut Ui) {
        form::heading(ui, "Board editor layers");
        self.count(ui);
        let candidate = &mut self.candidate;
        let t = pcbcraft_chrome::Tokens::get();
        form::card(ui, |ui| {
            let room = (ui.available_width() - ui.spacing().item_spacing.x * 3.).max(0.);
            let widths = [room * 0.09, room * 0.49, room * 0.26, room * 0.16];
            egui::Grid::new("catalog_layers").num_columns(4).show(ui, |ui| {
                for (title, width) in ["Use", "Layer name", "Purpose", "Side"].into_iter().zip(widths) {
                    form::table_label(ui, title, width);
                }
                ui.end_row();
                for layer in crate::board_panels::ordered_catalog(candidate.setup.layers().to_vec(), Some(&candidate.setup)) {
                    let Some(index) = candidate.setup.layers().iter().position(|l| l.id == layer.id) else { continue };
                    let required = matches!(layer.purpose, LayerPurpose::Copper | LayerPurpose::Outline | LayerPurpose::Courtyard);
                    ui.add_enabled_ui(!required, |ui| {
                        ui.checkbox(&mut candidate.enabled[index], "")
                            .on_hover_text(if required { "Required layer" } else { "Remove this layer from the board catalog" })
                            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Checkbox, !required, format!("Use layer {}", layer.name)))
                    });
                    ui.add_sized(egui::vec2(widths[1], t.control_height), properties::input(&mut candidate.names[index]).id_salt(layer.id))
                        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, format!("Layer name {}", layer.name)));
                    form::table_label(ui, purpose_name(layer.purpose), widths[2]);
                    form::table_label(
                        ui,
                        match layer.side {
                            Some(Side::Front) => "Front",
                            Some(Side::Back) => "Back",
                            None if layer.purpose == LayerPurpose::Copper => "Internal",
                            None => "—",
                        },
                        widths[3],
                    );
                    ui.end_row();
                }
            });
        });
        properties::section(ui, "Add technical layer");
        let mut added = None;
        form::card(ui, |ui| {
            form::text(ui, "Layer name", &mut self.new_name);
            form::field(ui, "Purpose", |ui| {
                egui::ComboBox::from_id_salt("new_purpose").selected_text(purpose_name(self.new_purpose)).show_ui(ui, |ui| {
                    for purpose in [
                        LayerPurpose::User,
                        LayerPurpose::Mechanical,
                        LayerPurpose::Outline,
                        LayerPurpose::Silkscreen,
                        LayerPurpose::Fabrication,
                        LayerPurpose::Courtyard,
                        LayerPurpose::Mask,
                        LayerPurpose::Paste,
                        LayerPurpose::Adhesive,
                    ] {
                        if ui.selectable_value(&mut self.new_purpose, purpose, purpose_name(purpose)).changed() {
                            self.new_side = if matches!(purpose, LayerPurpose::User | LayerPurpose::Mechanical | LayerPurpose::Outline) {
                                None
                            } else {
                                Some(Side::Front)
                            };
                        }
                    }
                });
            });
            if self.new_purpose != LayerPurpose::Outline {
                form::field(ui, "Side", |ui| {
                    egui::ComboBox::from_id_salt("new_side")
                        .selected_text(match self.new_side {
                            Some(Side::Front) => "Front",
                            Some(Side::Back) => "Back",
                            None => "Neither",
                        })
                        .show_ui(ui, |ui| {
                            if matches!(self.new_purpose, LayerPurpose::User | LayerPurpose::Mechanical) {
                                ui.selectable_value(&mut self.new_side, None, "Neither");
                            }
                            ui.selectable_value(&mut self.new_side, Some(Side::Front), "Front");
                            ui.selectable_value(&mut self.new_side, Some(Side::Back), "Back");
                        });
                });
            }
            if ui.button("Add layer").clicked() {
                let result = candidate
                    .value(self.unit)
                    .and_then(|s| s.with_added_technical_layer(self.new_name.clone(), self.new_purpose, self.new_side).map_err(|e| e.to_string()));
                match result {
                    Ok(s) => {
                        added = Some(Candidate::new(s, self.unit));
                        self.new_name.clear();
                        self.error = None;
                    }
                    Err(e) => self.error = Some(e),
                }
            }
        });
        if let Some(candidate) = added {
            self.candidate = candidate;
        }
    }

    /// Finished thickness is derived from all strata. Fitting changes only unlocked dielectric
    /// gaps and refuses missing copper, nonpositive remaining thickness or contradictory tolerance.
    pub(super) fn fit(&mut self) -> Result<(), String> {
        let candidate = &self.candidate;
        let target = units::parse_length(&self.target_thickness, self.unit).map_err(|e| e.to_string())?;
        let setup = candidate.value(self.unit)?;
        let locked = setup
            .stackup()
            .strata
            .iter()
            .filter_map(|s| if let Stratum::Dielectric { id, .. } = s { self.locked.contains(&id.to_string()).then_some(*id) } else { None })
            .collect();
        let fitted = setup.with_fitted_dielectrics(target, &locked).map_err(|e| e.to_string())?;
        self.candidate = Candidate::new(fitted, self.unit);
        Ok(())
    }

    /// Physical strata use a real cross-section beside per-gap material/construction controls.
    /// Core/prepreg references are selected by pinned identities and preserve unedited reports.
    pub(super) fn stackup(&mut self, ui: &mut Ui) {
        form::heading(ui, "Physical stackup");
        let mut preset = None;
        form::field(ui, "Starting stackup", |ui| {
            ui.add_enabled_ui(self.candidate.setup.stackup().strata.len() == 3, |ui| {
                egui::ComboBox::from_id_salt("starting_stackup")
                    .selected_text("Choose preset…")
                    .show_ui(ui, |ui| {
                        for value in pcbcraft_engine::board::setup::StackupPreset::ALL {
                            if ui.selectable_label(false, value.name()).clicked() {
                                preset = Some(value);
                            }
                        }
                    })
                    .response
                    .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Starting stackup"));
            });
        });
        if let Some(preset) = preset {
            match self.candidate.value(self.unit).and_then(|setup| setup.with_starting_stackup(preset).map_err(|e| e.to_string())) {
                Ok(setup) => {
                    self.candidate = Candidate::new(setup, self.unit);
                    self.locked.clear();
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
        self.count(ui);
        let candidate = &mut self.candidate;
        let total = candidate
            .thickness
            .iter()
            .try_fold(0_i64, |sum, length| length.value(self.unit).ok().flatten().filter(|n| *n > 0).and_then(|n| sum.checked_add(n)))
            .map(|n| units::label(n, self.unit))
            .unwrap_or_else(|| "Unspecified".into());
        form::field(ui, "Laminate thickness", |ui| {
            ui.label(total);
        });
        let bands: Vec<_> = candidate
            .setup
            .stackup()
            .strata
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let name = match s {
                    Stratum::Copper { layer, .. } => {
                        candidate.setup.layers().iter().position(|l| l.id == *layer).map(|n| candidate.names[n].clone()).unwrap_or_default()
                    }
                    _ => format!("Dielectric {}", i.div_ceil(2)),
                };
                (name, candidate.thickness[i].value(self.unit).ok().flatten(), matches!(s, Stratum::Copper { .. }))
            })
            .collect();
        Self::stack_table(ui, candidate, &bands, &mut self.locked, &mut self.selected_stratum, self.unit);
        if let Some(error) = Self::stratum_details(ui, candidate, self.selected_stratum, self.unit) {
            self.error = Some(error);
        }
        properties::section(ui, "Adjust dielectric thickness");
        form::card(ui, |ui| {
            form::text(ui, "Target thickness", &mut self.target_thickness);
            if ui.button("Fit unlocked dielectrics").clicked() {
                self.error = self.fit().err();
            }
        });
    }

    /// Show each stratum and its illustration cell in one grid, so row geometry and scrolling
    /// remain aligned at any layer count. The selected gap's details follow the complete table.
    fn stack_table(
        ui: &mut Ui,
        candidate: &mut Candidate,
        bands: &[(String, Option<i64>, bool)],
        locked: &mut BTreeSet<String>,
        selected: &mut usize,
        unit: units::Unit,
    ) {
        let t = pcbcraft_chrome::Tokens::get();
        let preview_width = form::cross_section_column(ui);
        let columns = if preview_width.is_some() { 5 } else { 4 };
        let width = (ui.available_width() - preview_width.unwrap_or(0.) - ui.spacing().item_spacing.x * (columns - 1) as f32).max(0.);
        let widths = [width * 0.35, width * 0.27, width * 0.25, width * 0.13];
        let mut preview_rows = Vec::new();
        egui::Grid::new("physical_stack").num_columns(columns).show(ui, |ui| {
            if let Some(width) = preview_width {
                form::cross_section_heading(ui, width);
            }
            for (label, width) in
                ["Layer".to_owned(), format!("Thickness ({})", unit.symbol()), "Construction".to_owned(), "Lock".to_owned()].into_iter().zip(widths)
            {
                if label == "Lock" {
                    ui.add_sized(egui::vec2(width, t.control_height), egui::Label::new(label));
                } else {
                    form::stack_label(ui, &label, width);
                }
            }
            ui.end_row();
            for (i, (name, _, copper)) in bands.iter().enumerate() {
                if let Some(width) = preview_width {
                    let (row, _) = ui.allocate_exact_size(egui::vec2(width, t.control_height), egui::Sense::hover());
                    preview_rows.push(row);
                }
                if form::stratum_row(ui, name, widths[0], *selected == i).clicked() {
                    *selected = i;
                }
                let thickness = ui.add_sized(
                    egui::vec2(widths[1], t.control_height),
                    properties::input(&mut candidate.thickness[i].text).id_salt(("thickness", i)).hint_text("Unspecified"),
                );
                thickness.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, format!("{name} thickness")));
                if thickness.gained_focus() {
                    *selected = i;
                }
                form::stack_label(
                    ui,
                    if *copper {
                        match candidate.thickness[i].value(unit).ok().flatten() {
                            Some(35_000) => "Copper · 1 oz",
                            Some(70_000) => "Copper · 2 oz",
                            Some(17_500) => "Copper · ½ oz",
                            _ => "Copper",
                        }
                    } else {
                        match candidate.constructions[i] {
                            Some(Construction::Core { .. }) => "Core",
                            Some(Construction::Prepreg { .. }) => "Prepreg",
                            None => "Unspecified",
                        }
                    },
                    widths[2],
                );
                if let Stratum::Dielectric { id, .. } = &candidate.setup.stackup().strata[i] {
                    let mut is_locked = locked.contains(&id.to_string());
                    let lock = form::thickness_lock(ui, widths[3], &mut is_locked, &format!("Lock {name} thickness"));
                    if lock.changed() {
                        if is_locked {
                            locked.insert(id.to_string());
                        } else {
                            locked.remove(&id.to_string());
                        }
                    }
                } else {
                    ui.add_sized(egui::vec2(widths[3], t.control_height), egui::Label::new("—"));
                }
                ui.end_row();
            }
        });
        if preview_width.is_some() {
            form::cross_section(ui, bands, &preview_rows);
        }
    }

    /// The selected dielectric exposes construction, pinned materials and tolerance without
    /// burying the stack beneath a full form for every gap. Allocation failures remain visible.
    fn stratum_details(ui: &mut Ui, candidate: &mut Candidate, selected: usize, unit: units::Unit) -> Option<String> {
        let mut error = None;
        {
            let i = selected.min(candidate.setup.stackup().strata.len().saturating_sub(1));
            let stratum = &candidate.setup.stackup().strata[i];
            let title = match stratum {
                Stratum::Copper { layer, .. } => {
                    candidate.setup.layers().iter().position(|l| l.id == *layer).map(|n| candidate.names[n].clone()).unwrap_or_default()
                }
                _ => format!("Dielectric {}", i.div_ceil(2)),
            };
            ui.push_id(i, |ui| {
                form::card(ui, |ui| {
                    properties::identity(ui, &title, if matches!(stratum, Stratum::Copper { .. }) { "Copper" } else { "Dielectric" });
                    if let Stratum::Dielectric { .. } = stratum {
                        form::text(ui, "Thickness tolerance", &mut candidate.tolerance[i].text);
                        form::text(ui, "Material label", &mut candidate.material_labels[i]);
                        let mut kind = match candidate.constructions[i] {
                            Some(Construction::Core { .. }) => 1,
                            Some(Construction::Prepreg { .. }) => 2,
                            None => 0,
                        };
                        let before = kind;
                        form::field(ui, "Construction", |ui| {
                            egui::ComboBox::from_id_salt("construction")
                                .selected_text(["Unspecified", "Core", "Prepreg"][kind])
                                .show_ui(ui, |ui| {
                                    for (n, label) in ["Unspecified", "Core", "Prepreg"].iter().enumerate() {
                                        ui.selectable_value(&mut kind, n, *label);
                                    }
                                })
                                .response
                                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Construction"));
                        });
                        if kind != before {
                            candidate.constructions[i] = match kind {
                                1 => Some(Construction::Core { material: None, glass_construction: None }),
                                2 => match PrepregPly::new() {
                                    Ok(p) => Some(Construction::Prepreg { plies: vec![p] }),
                                    Err(e) => {
                                        error = Some(e.to_string());
                                        candidate.constructions[i].clone()
                                    }
                                },
                                _ => None,
                            };
                        }
                        match &mut candidate.constructions[i] {
                            Some(Construction::Core { material, glass_construction }) => {
                                form::field(ui, "Material", |ui| material_choice(ui, "core_material", material, &candidate.materials));
                                let mut glass = glass_construction.clone().unwrap_or_default();
                                if form::text(ui, "Glass construction", &mut glass).changed() {
                                    *glass_construction = (!glass.is_empty()).then_some(glass);
                                }
                            }
                            Some(Construction::Prepreg { plies }) => {
                                let mut remove = None;
                                for (n, ply) in plies.iter_mut().enumerate() {
                                    ui.push_id(ply.id, |ui| {
                                        properties::section(ui, &format!("Ply {}", n + 1));
                                        material_choice(ui, "ply_material", &mut ply.material, &candidate.materials);
                                        let mut glass = ply.glass_style.clone().unwrap_or_default();
                                        if form::text(ui, "Glass style", &mut glass).changed() {
                                            ply.glass_style = (!glass.is_empty()).then_some(glass);
                                        }
                                        let fields = candidate.plies.entry(ply.id.to_string()).or_insert_with(|| PlyFields::new(ply, unit));
                                        form::text(ui, "Resin content (%)", &mut fields.resin);
                                        form::text(ui, "Nominal stock thickness", &mut fields.nominal.text);
                                        if ui.button("Remove ply").clicked() {
                                            remove = Some(n);
                                        }
                                    });
                                }
                                if let Some(n) = remove {
                                    plies.remove(n);
                                }
                                if ui.button("Add ply").clicked() {
                                    match PrepregPly::new() {
                                        Ok(ply) => plies.push(ply),
                                        Err(e) => error = Some(e.to_string()),
                                    }
                                }
                            }
                            None => {}
                        }
                    }
                })
            });
        }
        error
    }
}
