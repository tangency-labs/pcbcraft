//! Native part relationships are edited here; geometry remains in the drawing editors.
use super::*;
use pcbcraft_engine::library::parts::{Category, FieldType, PartDefinition, Value as TypedValue};
use std::sync::Arc;

impl PcbApp {
    /// Render a retained document in the Items region, preserving invalid typed input until Apply/Reset.
    pub(crate) fn part_document_form(&mut self, ui: &mut egui::Ui, id: u64) {
        let Some(mut doc) = self.library_editors.parts.remove(&id) else { return };
        let mut record = doc.session.record().clone();
        let revision = doc.session.revision();
        let catalog = self.session.libraries();
        let busy = self.library_editors.busy() || self.library_sources.busy() || self.library_mutation_busy();
        let destination = self.writable_libraries().into_iter().find(|(source, _)| Some(source) == doc.source.as_ref()).map(|(_, name)| name);
        let mut publish = false;
        let mut open_orderable = None;
        let mut edit_orderable = None;
        let mut browse = None;
        let mut clear_field = None;
        pcbcraft_chrome::part_form::document(ui, |ui, footer| {
            if footer {
                let valid = record.validate(catalog);
                let changed = doc.dirty() || &record != doc.session.record();
                let status = if crate::IN_BROWSER {
                    "Desktop required".into()
                } else if busy {
                    "Library operation in progress".into()
                } else if !doc.pending.is_empty() {
                    "Unapplied values".into()
                } else if valid.is_err() {
                    "Cannot publish".into()
                } else if changed {
                    format!("Draft · {}", destination.as_deref().unwrap_or("No library selected"))
                } else {
                    format!("Published r{} · {}", record.reference().revision, destination.as_deref().unwrap_or("Library unavailable"))
                };
                publish =
                    pcbcraft_chrome::part_form::publication(ui, &status, !crate::IN_BROWSER && !busy && valid.is_ok() && doc.pending.is_empty());
                return;
            }
            ui.push_id(("part_form", id), |ui| {
                properties::section(
                    ui,
                    match record {
                        Record::Part(_) => "Generic part",
                        Record::Orderable(_) => "Orderable part",
                    },
                );
                ui.horizontal(|ui| {
                    for (label, redo) in [("Undo", false), ("Redo", true)] {
                        if controls::enabled_button(ui, doc.pending.is_empty(), label).clicked() {
                            doc.error = doc.session.history(revision, redo).err();
                            record = doc.session.record().clone();
                        }
                    }
                });
                match &mut record {
                    Record::Part(part) => {
                        properties::text(ui, "Name", &mut part.name);
                        match pcbcraft_chrome::part_form::reference(
                            ui,
                            "Symbol",
                            &reference_name(catalog, part.symbol.as_ref(), Kind::Symbol),
                            part.symbol.is_some(),
                        ) {
                            pcbcraft_chrome::part_form::ReferenceAction::Browse => browse = Some(ReferenceField::Symbol),
                            pcbcraft_chrome::part_form::ReferenceAction::Clear => clear_field = Some(ReferenceField::Symbol),
                            _ => {}
                        }
                        if let Some(symbol) = part.symbol.as_ref().and_then(|r| catalog.symbol(&r.library, &r.id, r.revision)) {
                            properties::field(ui, "Units", |ui| {
                                ui.label(symbol.units().iter().map(|unit| unit.unit_id()).collect::<Vec<_>>().join(", "));
                            });
                        }
                        match pcbcraft_chrome::part_form::reference(
                            ui,
                            "Category",
                            &reference_name(catalog, part.category.as_ref().map(|c| &c.definition), Kind::Category),
                            part.category.is_some(),
                        ) {
                            pcbcraft_chrome::part_form::ReferenceAction::Browse => browse = Some(ReferenceField::Category),
                            pcbcraft_chrome::part_form::ReferenceAction::Clear => clear_field = Some(ReferenceField::Category),
                            _ => {}
                        }
                        primary_value_form(ui, part, catalog);
                        if let Some(category) = part.category.as_ref().and_then(|binding| catalog.category(&binding.definition).ok()) {
                            typed_fields(ui, &category, &mut part.fixed_parameters, &mut doc.pending, &mut doc.error, "Fixed parameters");
                        }
                        package_form(ui, part, &mut doc, catalog, &mut browse);
                        properties::section(ui, "Orderable parts");
                        for orderable in catalog.manufacturers_for(&part.reference()) {
                            if controls::button(ui, format!("{} · {} · r{}", orderable.manufacturer, orderable.mpn, orderable.revision)).clicked() {
                                edit_orderable = Some(Record::Orderable((*orderable).clone()));
                            }
                        }
                        if controls::enabled_button(ui, !doc.session.dirty() && !part.packages.is_empty(), "Add orderable part").clicked() {
                            open_orderable = Some(part.reference());
                        }
                    }
                    Record::Orderable(part) => {
                        properties::text(ui, "Name", &mut part.name);
                        properties::text(ui, "Manufacturer", &mut part.manufacturer);
                        properties::text(ui, "Part number", &mut part.mpn);
                        if pcbcraft_chrome::part_form::reference(ui, "Generic part", &reference_name(catalog, Some(&part.part), Kind::Part), false)
                            == pcbcraft_chrome::part_form::ReferenceAction::Browse
                        {
                            browse = Some(ReferenceField::Part);
                        }
                        let mut datasheet = part.datasheet.clone().unwrap_or_default();
                        properties::text(ui, "Datasheet", &mut datasheet);
                        part.datasheet = (!datasheet.is_empty()).then_some(datasheet);
                        if let Ok(resolved) = catalog.resolve_part(&part.part) {
                            let options: Vec<_> =
                                resolved.definition.packages.iter().map(|(id, package)| (id.as_str(), package.name.clone())).collect();
                            properties::choice(ui, "Package", ui.scope_id().with("mpn_package"), &mut part.package, &options);
                            if let Some(category) = &resolved.category {
                                typed_fields(ui, category, &mut part.specifications, &mut doc.pending, &mut doc.error, "Specifications");
                            }
                        }
                    }
                }
                if let Some(field) = clear_field.take() {
                    if doc.pending.is_empty() {
                        match record.select_reference(&field, None, catalog) {
                            Ok(candidate) => record = candidate,
                            Err(error) => doc.error = Some(error),
                        }
                    } else {
                        doc.error = Some("Apply or reset typed fields before clearing an association.".into());
                    }
                }
                controls::collapsing(ui, "part_details", "Details", false, None, |ui| {
                    let reference = record.reference();
                    let (license, source) = match &mut record {
                        Record::Part(p) => (&mut p.license, &p.source),
                        Record::Orderable(p) => (&mut p.license, &p.source),
                    };
                    properties::text(ui, "License", license);
                    properties::field(ui, "Identity", |ui| {
                        ui.label(format!("{}/{} · r{}", reference.library, reference.id, reference.revision));
                    });
                    properties::field(ui, "Source", |ui| {
                        ui.label(&source.reference);
                    });
                });
                let valid = record.validate(catalog);
                if let Err(error) = &valid {
                    ui.colored_label(pcbcraft_chrome::Tokens::get().error, error);
                }
                if let Some(error) = &doc.error {
                    ui.colored_label(pcbcraft_chrome::Tokens::get().error, error);
                }
            });
        });
        if record != *doc.session.record() {
            doc.error = doc.session.replace(doc.session.revision(), record).err();
        }
        self.library_editors.parts.insert(id, doc);
        if let Some(field) = browse {
            self.message = self.begin_part_reference(ui.ctx(), id, field).err();
        }
        if publish {
            self.message = self.begin_library_publish(id).err();
        }
        let next = if let Some(part) = open_orderable {
            Record::new_orderable(self.session.libraries(), part).and_then(|record| self.library_editors.open_part(record, false, None)).map(Some)
        } else if let Some(record) = edit_orderable {
            self.library_editors.open_part(record, true, None).map(Some)
        } else {
            Ok(None)
        };
        match next {
            Ok(Some(next)) => self.pending_part_navigation = Some(next),
            Err(error) => self.message = Some(error),
            _ => {}
        }
    }
    /// Preview the selected package, or the first package before Items has been visited, with
    /// optional schematic geometry and explicit pad/terminal correspondence.
    pub(crate) fn part_document_preview(&mut self, ui: &mut egui::Ui, id: u64) {
        let Some(doc) = self.library_editors.parts.get_mut(&id) else { return };
        let catalog = self.session.libraries();
        let part = match doc.session.record() {
            Record::Part(part) => Arc::new(part.clone()),
            Record::Orderable(record) => match catalog.resolve_part(&record.part) {
                Ok(part) => part.definition.clone(),
                Err(error) => {
                    ui.label(error);
                    return;
                }
            },
        };
        let package = match doc.session.record() {
            Record::Orderable(p) => part.packages.get(&p.package),
            _ => doc.package.as_ref().map_or_else(|| part.packages.values().next(), |id| part.packages.get(id)),
        };
        let terminal = package.and_then(|package| {
            doc.selected_pad.as_ref().and_then(|pad| package.mapping.iter().find(|(_, pads)| pads.contains(pad)).map(|(id, _)| id.as_str()))
        });
        if let Some(symbol) = part.symbol.as_ref().and_then(|r| catalog.symbol(&r.library, &r.id, r.revision)) {
            crate::library_preview::part_symbol(ui, &symbol, terminal, &mut doc.symbol_camera);
        }
        if let Some(package) = package
            && let Some(footprint) = catalog.footprint(&package.footprint.library, &package.footprint.id, package.footprint.revision)
        {
            crate::library_preview::part_footprint(ui, &footprint, doc.selected_pad.as_deref(), &mut doc.footprint_camera);
        }
    }
}
/// The primary quantity is chosen only among the exact selected category's bounded fields.
fn primary_value_form(ui: &mut egui::Ui, part: &mut PartDefinition, catalog: &Catalog) {
    if let Some(binding) = &mut part.category
        && let Ok(category) = catalog.category(&binding.definition)
    {
        let mut primary = binding.primary_value.clone().unwrap_or_default();
        let mut options = vec![("", "None".into())];
        options.extend(
            category
                .fields
                .iter()
                .filter(|(_, field)| matches!(field.value_type, FieldType::Quantity { .. }))
                .map(|(id, field)| (id.as_str(), field.name.clone())),
        );
        properties::choice(ui, "Value field", ui.scope_id().with("primary_value"), &mut primary, &options);
        binding.primary_value = (!primary.is_empty()).then_some(primary);
    }
}
/// Typed values are applied through the existing category parser; missing values remain absent.
fn typed_fields(
    ui: &mut egui::Ui,
    category: &Category,
    values: &mut BTreeMap<String, TypedValue>,
    pending: &mut BTreeMap<String, String>,
    error: &mut Option<String>,
    title: &str,
) {
    controls::collapsing(ui, title, title, false, None, |ui| {
        for (id, field) in &category.fields {
            ui.push_id(id, |ui| {
                let current = values.get(id).map(TypedValue::display).unwrap_or_default();
                let mut text = pending.get(id).cloned().unwrap_or_else(|| current.clone());
                if properties::text(ui, &field.name, &mut text).changed() {
                    if text == current {
                        pending.remove(id);
                    } else {
                        pending.insert(id.clone(), text.clone());
                    }
                }
                if pending.contains_key(id) {
                    ui.horizontal(|ui| {
                        if controls::button(ui, "Apply value").clicked() {
                            let value = if text.trim().is_empty() { Ok(None) } else { field.parse_input(&text).map(Some) };
                            match value {
                                Ok(value) => {
                                    if let Some(value) = value {
                                        values.insert(id.clone(), value);
                                    } else {
                                        values.remove(id);
                                    }
                                    pending.remove(id);
                                    *error = None;
                                }
                                Err(reason) => *error = Some(format!("{}: {reason}", field.name)),
                            }
                        }
                        if controls::button(ui, "Reset value").clicked() {
                            pending.remove(id);
                            *error = None;
                        }
                    });
                }
            });
        }
    });
}
/// Choose physical packages independently of schematic symbols. When a symbol is present,
/// each pad is explicitly assigned or marked unconnected; geometry never infers a mapping.
fn package_form(ui: &mut egui::Ui, part: &mut PartDefinition, doc: &mut Document, catalog: &Catalog, browse: &mut Option<ReferenceField>) {
    properties::section(ui, "Footprints");
    ui.horizontal(|ui| {
        if controls::enabled_button(ui, part.packages.len() < 64, "Add footprint…").clicked()
            && let Some(package) = (1..=64).map(|n| format!("package-{n}")).find(|id| !part.packages.contains_key(id))
        {
            *browse = Some(ReferenceField::NewPackage { package });
        }
        if controls::enabled_button(ui, doc.package.is_some(), "Remove footprint").clicked() {
            if let Some(id) = doc.package.take() {
                part.packages.remove(&id);
            }
            doc.selected_pad = None;
        }
    });
    if doc.package.as_ref().is_none_or(|id| !part.packages.contains_key(id)) {
        doc.package = part.packages.keys().next().cloned();
    }
    let mut chosen = doc.package.clone().unwrap_or_default();
    let options: Vec<_> = part.packages.iter().map(|(id, p)| (id.as_str(), p.name.clone())).collect();
    if options.len() > 1 {
        properties::choice(ui, "Package", ui.scope_id().with("package_choice"), &mut chosen, &options);
    }
    if doc.package.as_ref() != Some(&chosen) {
        doc.selected_pad = None;
        doc.footprint_camera = Default::default();
    }
    doc.package = (!chosen.is_empty()).then_some(chosen.clone());
    let Some(package) = part.packages.get_mut(&chosen) else { return };
    if pcbcraft_chrome::part_form::reference(ui, "Footprint", &reference_name(catalog, Some(&package.footprint), Kind::Footprint), false)
        == pcbcraft_chrome::part_form::ReferenceAction::Browse
    {
        *browse = Some(ReferenceField::Footprint { package: chosen });
    }
    controls::collapsing(ui, "package_details", "Package details", false, None, |ui| {
        properties::text(ui, "Name", &mut package.name);
        properties::field(ui, "3D model", |ui| {
            ui.add_enabled(false, egui::Label::new("None"));
        });
    });
    let Some(footprint) = catalog.footprint(&package.footprint.library, &package.footprint.id, package.footprint.revision) else { return };
    let Some(symbol) = part.symbol.as_ref().and_then(|r| catalog.symbol(&r.library, &r.id, r.revision)) else { return };
    properties::section(ui, "Pin mapping");
    let pads: Vec<_> = footprint.pads.iter().map(|pad| pad.number.clone()).collect();
    pcbcraft_chrome::part_form::mapping(ui, &pads, &mut doc.selected_pad, |ui, pad| {
        let mut terminal = package
            .mapping
            .iter()
            .find(|(_, pads)| pads.iter().any(|p| p == pad))
            .map(|(id, _)| id.clone())
            .unwrap_or_else(|| if package.unconnected_pads.iter().any(|p| p == pad) { "__nc".into() } else { String::new() });
        let before = terminal.clone();
        let mut options = vec![("", "Unassigned".into()), ("__nc", "Unconnected".into())];
        options.extend(
            symbol.pins().iter().map(|pin| {
                (pin.number.as_str(), format!("{}{}", pin.number, pin.name.as_ref().map(|name| format!(" · {name}")).unwrap_or_default()))
            }),
        );
        let selected = options.iter().find(|(id, _)| *id == terminal).map(|(_, name)| name.as_str()).unwrap_or("Unassigned");
        let response = controls::Choice::from_id_salt(ui.scope_id().with(pad))
            .selected_text(selected)
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for (id, name) in &options {
                    if pcbcraft_chrome::MenuItem::new(name).choice(*id == terminal).show(ui) {
                        terminal = (*id).into();
                    }
                }
            })
            .response;
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, ui.is_enabled(), format!("Terminal for pad {pad}")));
        if terminal != before {
            for pads in package.mapping.values_mut() {
                pads.retain(|p| p != pad);
            }
            package.mapping.retain(|_, pads| !pads.is_empty());
            package.unconnected_pads.retain(|p| p != pad);
            if terminal == "__nc" {
                package.unconnected_pads.push(pad.into());
                package.unconnected_pads.sort();
            } else if !terminal.is_empty() {
                let pads = package.mapping.entry(terminal).or_default();
                pads.push(pad.into());
                pads.sort();
            }
        }
    });
}
