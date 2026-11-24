//! Native part relationships are edited here; geometry remains in the drawing editors.
use super::*;
use pcbcraft_engine::library::parts::{
    Category, FieldType, MAX_DESCRIPTION_CHARS, MAX_KEYWORD_CHARS, MAX_KEYWORDS, MAX_LINK_LABEL_CHARS, MAX_LINK_URL_CHARS, MAX_LINKS, PartDefinition,
    PartLink, Value as TypedValue,
};
use std::{collections::BTreeSet, sync::Arc};

impl Document {
    /// Whether pending text still needs applying. Focused metadata fields can retain raw text
    /// after the normalized value is committed, without blocking publication or history.
    pub(crate) fn has_unapplied(&self) -> bool {
        let Record::Part(part) = self.session.record() else {
            return !self.pending.is_empty();
        };
        self.pending.iter().any(|(key, text)| match key.as_str() {
            DESCRIPTION_KEY => !description_applied(text, part.description.as_deref()),
            KEYWORDS_KEY => !keyword_applied(text, &part.keywords),
            _ => !key
                .strip_prefix("metadata/link/")
                .and_then(|key| key.strip_suffix("/label"))
                .and_then(|index| index.parse::<usize>().ok())
                .and_then(|index| part.links.get(index))
                .is_some_and(|link| text.trim() == link.label),
        })
    }
}

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
        let mut applied = Vec::new();
        pcbcraft_chrome::part_form::document(ui, |ui, footer| {
            if footer {
                let valid = record.validate(catalog);
                let changed = doc.dirty() || &record != doc.session.record();
                let status = if crate::IN_BROWSER {
                    "Desktop required".into()
                } else if busy {
                    "Library operation in progress".into()
                } else if doc.has_unapplied() {
                    "Unapplied values".into()
                } else if valid.is_err() {
                    "Cannot publish".into()
                } else if changed {
                    format!("Draft · {}", destination.as_deref().unwrap_or("No library selected"))
                } else {
                    format!("Published r{} · {}", record.reference().revision, destination.as_deref().unwrap_or("Library unavailable"))
                };
                publish = pcbcraft_chrome::part_form::publication(ui, &status, !crate::IN_BROWSER && !busy && valid.is_ok() && !doc.has_unapplied());
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
                        if controls::enabled_button(ui, !doc.has_unapplied(), label).clicked() {
                            doc.error = doc.change_history(revision, redo).err();
                            record = doc.session.record().clone();
                        }
                    }
                });
                match &mut record {
                    Record::Part(part) => {
                        properties::text(ui, "Name", &mut part.name);
                        metadata_form(ui, part, &mut doc.pending, &mut applied, &mut doc.error);
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
                        let symbol = part.symbol.as_ref().and_then(|r| catalog.symbol(&r.library, &r.id, r.revision));
                        if let Some(symbol) = &symbol {
                            properties::field(ui, "Units", |ui| {
                                ui.label(symbol.units().iter().map(|unit| unit.unit_id()).collect::<Vec<_>>().join(", "));
                            });
                        }
                        supply_ranges(ui, part, symbol.as_deref(), &mut doc.pending, &mut applied, &mut doc.error);
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
                    if !doc.has_unapplied() {
                        match record.select_reference(&field, None, catalog) {
                            Ok(candidate) => {
                                applied.extend(doc.pending.keys().cloned());
                                record = candidate;
                            }
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
        // Applied supply and metadata text is cleared only once the draft accepts the
        // edit, so a refused replacement (for example at the size limit) keeps what was typed.
        let accepted = record == *doc.session.record() || {
            let result = doc.session.replace(doc.session.revision(), record);
            doc.error = result.as_ref().err().cloned();
            result.is_ok()
        };
        if accepted {
            for key in applied {
                doc.pending.remove(&key);
            }
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
/// Description and keywords occupy one pending key each; link rows use positional keys
/// because an uncommitted row can sit beside authored links. The form never writes rejected
/// text to the record, so the engine's bounded replacement stays the only commit path.
const DESCRIPTION_KEY: &str = "metadata/description";
const KEYWORDS_KEY: &str = "metadata/keywords";
/// A link row whose text is valid but waits for earlier rows to be authored before it is
/// committed. The marker keeps the row identity stable instead of appending the link early.
const LINK_READY_FIELD: &str = "ready";

fn link_key(index: usize, field: &str) -> String {
    format!("metadata/link/{index}/{field}")
}

/// Pending key marking a valid link row that waits for earlier rows before being committed.
fn link_ready_key(index: usize) -> String {
    link_key(index, LINK_READY_FIELD)
}

/// Whether a raw description buffer still normalizes to the stored value. Such text is an
/// active edit: the trimmed text is already the draft's value, so it must not block publishing.
fn description_applied(raw: &str, stored: Option<&str>) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return stored.is_none();
    }
    trimmed.chars().count() <= MAX_DESCRIPTION_CHARS && !trimmed.chars().any(char::is_control) && stored == Some(trimmed)
}

/// Whether a raw keywords buffer normalizes to the stored list, so it is an active edit rather
/// than unapplied input (the normalized value has already reached the draft).
fn keyword_applied(raw: &str, stored: &[String]) -> bool {
    keyword_lines(raw).eq(stored.iter().map(String::as_str))
}

/// Omit blank lines from the stored keywords while leaving them in the focused editor.
fn keyword_lines(raw: &str) -> impl Iterator<Item = &str> {
    raw.lines().map(str::trim).filter(|line| !line.is_empty())
}

/// Number of displayed link rows, including pending rows that have no authored link yet.
fn pending_link_rows(pending: &BTreeMap<String, String>) -> usize {
    pending
        .keys()
        .filter_map(|key| key.strip_prefix("metadata/link/"))
        .filter_map(|rest| rest.split('/').next())
        .filter_map(|index| index.parse::<usize>().ok())
        .map(|index| index + 1)
        .max()
        .unwrap_or(0)
}

/// Drop one link row and compact the pending keys of later rows so their text stays attached
/// to the same visible row after the list shifts. A blank pending row never removes an
/// authored link: the accepted link in a later row can sit at a lower index in `part.links`.
fn remove_link(part: &mut PartDefinition, pending: &mut BTreeMap<String, String>, index: usize) {
    let displayed = |field: &str| {
        pending.get(&link_key(index, field)).cloned().unwrap_or_else(|| {
            part.links.get(index).map_or_else(String::new, |link| if field == "label" { link.label.clone() } else { link.url.clone() })
        })
    };
    let blank = displayed("label").trim().is_empty() && displayed("url").trim().is_empty();
    if !blank && index < part.links.len() {
        part.links.remove(index);
    }
    for field in ["label", "url", LINK_READY_FIELD] {
        pending.remove(&link_key(index, field));
    }
    let later: Vec<_> = pending
        .keys()
        .filter_map(|key| {
            let rest = key.strip_prefix("metadata/link/")?;
            let (number, field) = rest.split_once('/')?;
            Some((number.parse::<usize>().ok()?, field.to_owned(), key.clone()))
        })
        .filter(|(number, _, _)| *number > index)
        .collect();
    for (number, field, key) in later {
        if let Some(text) = pending.remove(&key) {
            pending.insert(link_key(number - 1, &field), text);
        }
    }
}

/// Mirror the engine's one-line text bounds so rejected text stays editable instead of
/// reaching the bounded session replacement. The engine remains authoritative on commit.
fn metadata_text(field: &str, text: &str, limit: usize) -> Result<String, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.chars().count() > limit || trimmed.chars().any(char::is_control) {
        Err(format!("{field} must be 1 to {limit} characters on one line"))
    } else {
        Ok(trimmed.to_owned())
    }
}

/// Accept only bounded HTTP(S) references with a host, matching the native part validator.
fn metadata_url(text: &str) -> Result<String, String> {
    let url = metadata_text("Link URL", text, MAX_LINK_URL_CHARS)?;
    let host = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).unwrap_or("").split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() || url.chars().any(char::is_whitespace) {
        return Err("Link URL must be an HTTP(S) URL; loading performs no fetch.".into());
    }
    Ok(url)
}

/// Validate the one-keyword-per-line draft before it replaces the authored list.
fn metadata_keywords(lines: &[&str]) -> Result<Vec<String>, String> {
    if lines.len() > MAX_KEYWORDS {
        return Err(format!("A part supports at most {MAX_KEYWORDS} keywords, not {}.", lines.len()));
    }
    let mut seen = BTreeSet::new();
    let mut keywords = Vec::with_capacity(lines.len());
    for line in lines {
        let keyword = metadata_text("Keyword", line, MAX_KEYWORD_CHARS)?;
        if !seen.insert(keyword.to_lowercase()) {
            return Err(format!("Duplicate keyword {keyword:?} after trimming and case folding."));
        }
        keywords.push(keyword);
    }
    Ok(keywords)
}

/// Build one link from its row, checking it against the labels visible in every other row
/// so a duplicate cannot be committed by editing either side of the pair.
fn metadata_link(
    index: usize,
    label: &str,
    url: &str,
    part: &PartDefinition,
    pending: &BTreeMap<String, String>,
    rows: usize,
) -> Result<PartLink, String> {
    let label = metadata_text("Link label", label, MAX_LINK_LABEL_CHARS)?;
    let url = metadata_url(url)?;
    let duplicate = (0..rows).filter(|other| *other != index).any(|other| {
        let other = pending
            .get(&link_key(other, "label"))
            .cloned()
            .unwrap_or_else(|| part.links.get(other).map_or_else(String::new, |link| link.label.clone()));
        other.trim().to_lowercase() == label.to_lowercase()
    });
    if duplicate {
        return Err(format!("Duplicate link label {label:?} after trimming and case folding."));
    }
    Ok(PartLink { label, url })
}

/// Commit rows that were typed ahead of an unauthored row, in row order. A ready row keeps its
/// text and identity in `pending` until every earlier row has an authored link, so an earlier
/// completion inserts at its own index instead of overwriting the later link.
fn flush_staged_links(
    part: &mut PartDefinition,
    pending: &mut BTreeMap<String, String>,
    applied: &mut Vec<String>,
    rows: usize,
    error: &mut Option<String>,
) {
    for index in 0..rows {
        let ready_key = link_ready_key(index);
        if !pending.contains_key(&ready_key) || index != part.links.len() {
            continue;
        }
        let label = pending.get(&link_key(index, "label")).cloned().unwrap_or_default();
        let url = pending.get(&link_key(index, "url")).cloned().unwrap_or_default();
        match metadata_link(index, &label, &url, part, pending, rows) {
            Ok(link) => {
                part.links.insert(index, link);
                applied.extend([link_key(index, "label"), link_key(index, "url"), ready_key]);
                *error = None;
            }
            Err(reason) => {
                pending.remove(&ready_key);
                *error = Some(reason);
            }
        }
    }
}

/// Keep metadata near the part name behind one disclosure so compact hosts retain the
/// existing Category row on screen while the same implicit replacement commits valid input.
fn metadata_form(
    ui: &mut egui::Ui,
    part: &mut PartDefinition,
    pending: &mut BTreeMap<String, String>,
    applied: &mut Vec<String>,
    error: &mut Option<String>,
) {
    controls::collapsing(ui, "part_metadata", "Search metadata", false, None, |ui| metadata_fields(ui, part, pending, applied, error));
}

/// Edit description, keywords and labeled links. Valid input reaches the draft immediately
/// through the existing implicit replacement; invalid input stays in `pending` and visible
/// in the field while publishing remains unavailable. Applied keys go to `applied` so the
/// caller clears them only after the draft accepts the replacement.
fn metadata_fields(
    ui: &mut egui::Ui,
    part: &mut PartDefinition,
    pending: &mut BTreeMap<String, String>,
    applied: &mut Vec<String>,
    error: &mut Option<String>,
) {
    let mut description = pending.get(DESCRIPTION_KEY).cloned().unwrap_or_else(|| part.description.clone().unwrap_or_default());
    let response = properties::text(ui, "Description", &mut description);
    if response.changed() {
        if description.trim().is_empty() {
            part.description = None;
            if description.is_empty() {
                applied.push(DESCRIPTION_KEY.into());
            } else {
                pending.insert(DESCRIPTION_KEY.into(), description);
                if !response.has_focus() {
                    applied.push(DESCRIPTION_KEY.into());
                }
            }
            *error = None;
        } else {
            match metadata_text("Description", &description, MAX_DESCRIPTION_CHARS) {
                Ok(value) => {
                    part.description = Some(value.clone());
                    // Keep the raw text in `pending` until the bounded session accepts the draft:
                    // a refused replacement must not discard what was typed. A buffer that already
                    // normalizes to the stored value is an active edit (not unapplied input), and
                    // is only queued for clearing once the field loses focus or needs no normalization.
                    pending.insert(DESCRIPTION_KEY.into(), description.clone());
                    if !response.has_focus() || description == value {
                        applied.push(DESCRIPTION_KEY.into());
                    }
                    *error = None;
                }
                Err(reason) => {
                    pending.insert(DESCRIPTION_KEY.into(), description);
                    *error = Some(reason);
                }
            }
        }
    } else if !response.has_focus() && pending.get(DESCRIPTION_KEY).is_some_and(|raw| description_applied(raw, part.description.as_deref())) {
        applied.push(DESCRIPTION_KEY.into());
    }

    let mut keywords = pending.get(KEYWORDS_KEY).cloned().unwrap_or_else(|| part.keywords.join("\n"));
    let t = pcbcraft_chrome::Tokens::get();
    let response = ui
        .horizontal(|ui| {
            let label = ui.add_sized(egui::vec2(72.0, t.control_height), egui::Label::new("Keywords").halign(egui::Align::Min));
            ui.add(
                egui::TextEdit::multiline(&mut keywords)
                    .margin(egui::Margin::symmetric(t.space_8 as i8, t.space_4 as i8))
                    .min_size(egui::vec2(ui.available_width(), t.control_height * 3.0))
                    .hint_text("One keyword per line"),
            )
            .labelled_by(label.id)
        })
        .inner;
    pcbcraft_chrome::automation::publish(ui, &response, "field:Keywords");
    if response.changed() {
        let lines: Vec<_> = keyword_lines(&keywords).collect();
        match metadata_keywords(&lines) {
            Ok(value) => {
                if !response.has_focus() || keywords == value.join("\n") {
                    applied.push(KEYWORDS_KEY.into());
                }
                part.keywords = value;
                *error = None;
            }
            Err(reason) => *error = Some(reason),
        }
        // Keep raw input across frames, including if Session::replace rejects it.
        pending.insert(KEYWORDS_KEY.into(), keywords);
    } else if !response.has_focus() && pending.get(KEYWORDS_KEY).is_some_and(|raw| keyword_applied(raw, &part.keywords)) {
        applied.push(KEYWORDS_KEY.into());
    }

    properties::section(ui, "Links");
    let rows = part.links.len().max(pending_link_rows(pending));
    for index in 0..rows {
        ui.push_id(("part_link", index), |ui| {
            let label_key = link_key(index, "label");
            let url_key = link_key(index, "url");
            let ready_key = link_ready_key(index);
            let mut label =
                pending.get(&label_key).cloned().unwrap_or_else(|| part.links.get(index).map_or_else(String::new, |link| link.label.clone()));
            let mut url = pending.get(&url_key).cloned().unwrap_or_else(|| part.links.get(index).map_or_else(String::new, |link| link.url.clone()));
            let label_response = properties::text(ui, &format!("Link label {}", index + 1), &mut label);
            let label_changed = label_response.changed();
            let url_changed = properties::text(ui, &format!("Link URL {}", index + 1), &mut url).changed();
            let remove = controls::button(ui, format!("Remove link {}", index + 1)).clicked();
            if remove || (label_changed || url_changed) && label.trim().is_empty() && url.trim().is_empty() {
                remove_link(part, pending, index);
                *error = None;
                return;
            }
            if label_changed || url_changed {
                match metadata_link(index, &label, &url, part, pending, rows) {
                    Ok(link) => {
                        if index <= part.links.len() {
                            let retain_label = label_response.has_focus() && label != link.label;
                            // Every earlier row is authored: edit in place or insert at this
                            // row's index, never append a link typed in a later row at index 0.
                            if index < part.links.len() {
                                part.links[index] = link;
                            } else {
                                part.links.insert(index, link);
                            }
                            // Keep the typed text in `pending` until the bounded session accepts
                            // the draft; the caller removes these keys only when replace is Ok.
                            pending.insert(label_key.clone(), label);
                            pending.insert(url_key.clone(), url);
                            applied.extend([url_key, ready_key]);
                            if !retain_label {
                                applied.push(label_key);
                            }
                            *error = None;
                        } else {
                            // Earlier rows are still blank or invalid. Stage the valid text so
                            // it stays with this row until those rows are completed.
                            pending.insert(label_key, label);
                            pending.insert(url_key, url);
                            pending.insert(ready_key, "1".into());
                            *error = None;
                        }
                    }
                    Err(reason) => {
                        pending.insert(label_key, label);
                        pending.insert(url_key, url);
                        pending.remove(&ready_key);
                        *error = Some(reason);
                    }
                }
            } else if !label_response.has_focus()
                && !pending.contains_key(&url_key)
                && pending.get(&label_key).is_some_and(|raw| part.links.get(index).is_some_and(|link| raw.trim() == link.label))
            {
                applied.push(label_key);
            }
        });
    }
    flush_staged_links(part, pending, applied, rows, error);
    // One unauthored row at a time keeps row indices aligned with the authored links.
    if controls::enabled_button(ui, rows < MAX_LINKS && pending_link_rows(pending) <= part.links.len(), "Add link").clicked() {
        pending.entry(link_key(rows, "label")).or_default();
        pending.entry(link_key(rows, "url")).or_default();
    }
}

/// Edit both bounds of one power input in one Apply/undo. Pending text is session UI,
/// never a native value; errors leave the previous authored range untouched. Applied keys
/// go to `applied` so the caller clears them only after the draft accepts the edit.
/// Unavailable inputs expose a text-only discard even when the exact symbol no longer resolves.
fn supply_ranges(
    ui: &mut egui::Ui,
    part: &mut PartDefinition,
    symbol: Option<&pcbcraft_engine::schematic::SymbolDefinition>,
    pending: &mut BTreeMap<String, String>,
    applied: &mut Vec<String>,
    error: &mut Option<String>,
) {
    use pcbcraft_engine::schematic::ElectricalType::PowerIn;
    let power_input =
        |id: &str| symbol.is_some_and(|symbol| symbol.pin_index(id).is_some_and(|index| symbol.pins()[index].electrical_type == PowerIn));
    let mut unavailable = Vec::new();
    for (key, text) in pending.iter() {
        if let Some((terminal, bound)) = key.strip_prefix("supply/").and_then(|rest| rest.rsplit_once('/'))
            && !power_input(terminal)
        {
            if unavailable.is_empty() {
                properties::section(ui, "Unapplied supply ranges");
            }
            let label = if bound == "min" { "minimum" } else { "maximum" };
            ui.label(format!("Terminal {terminal} {label}: {text:?}"));
            unavailable.push(key.clone());
        }
    }
    if !unavailable.is_empty() {
        ui.label("The exact power input is unavailable. Restore it to apply these edits, or discard the unapplied text.");
        if controls::button(ui, "Discard unavailable supply edits").clicked() {
            for key in unavailable {
                pending.remove(&key);
            }
        }
    }
    let pins = symbol.map_or(&[][..], |symbol| symbol.pins());
    if part.terminal_ratings.is_empty() && !pins.iter().any(|pin| pin.electrical_type == PowerIn) {
        return;
    }
    controls::collapsing(ui, "supply_ranges", "Device supply ranges", false, None, |ui| {
        ui.label("Enter volts or SI-prefixed voltages. Leave either bound blank when unknown.");
        for pin in pins.iter().filter(|pin| pin.electrical_type == PowerIn) {
            let id = &pin.number;
            ui.push_id(id, |ui| {
                let name = pin.name.as_ref().map_or_else(|| id.clone(), |name| format!("{name} ({id})"));
                ui.strong(name);
                let range = part.supply_range(id);
                let min = supply_input(ui, pending, id, "min", "Minimum (V)", range.and_then(|range| range.supply_min.as_ref()));
                let max = supply_input(ui, pending, id, "max", "Maximum (V)", range.and_then(|range| range.supply_max.as_ref()));
                if pending.contains_key(&min.0) || pending.contains_key(&max.0) {
                    ui.horizontal(|ui| {
                        if controls::button(ui, "Apply range").clicked() {
                            match part.set_supply_range(id, &min.1, &max.1) {
                                Ok(()) => {
                                    applied.extend([min.0.clone(), max.0.clone()]);
                                    *error = None;
                                }
                                Err(reason) => *error = Some(reason),
                            }
                        }
                        if controls::button(ui, "Reset range").clicked() {
                            pending.remove(&min.0);
                            pending.remove(&max.0);
                            *error = None;
                        }
                    });
                }
            });
        }
        // Authored ranges are independent of pending text and never removed on source loss.
        let unavailable: Vec<_> = part.terminal_ratings.keys().filter(|id| !power_input(id)).cloned().collect();
        for id in unavailable {
            ui.push_id(&id, |ui| {
                ui.label(format!("Exact power input {id} unavailable."));
                if controls::button(ui, "Remove authored range").clicked() {
                    *error = part.set_supply_range(&id, "", "").err();
                }
            });
        }
    });
}

/// Keep uncommitted bound text distinct from category field IDs and from a stored zero.
fn supply_input(
    ui: &mut egui::Ui,
    pending: &mut BTreeMap<String, String>,
    id: &str,
    bound: &str,
    label: &str,
    current: Option<&TypedValue>,
) -> (String, String) {
    let key = format!("supply/{id}/{bound}");
    let current = current.map(TypedValue::display).unwrap_or_default();
    let mut text = pending.get(&key).cloned().unwrap_or_else(|| current.clone());
    if properties::text(ui, label, &mut text).changed() {
        if text == current {
            pending.remove(&key);
        } else {
            pending.insert(key.clone(), text.clone());
        }
    }
    (key, text)
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
/// Retained assignments to removed pads remain visible until explicitly removed during review.
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
    let mut pads: Vec<_> = footprint.pads.iter().map(|pad| pad.number.clone()).collect();
    let removed: std::collections::BTreeSet<_> =
        package.mapping.values().flatten().chain(&package.unconnected_pads).filter(|pad| !pads.contains(pad)).cloned().collect();
    pads.extend(removed.iter().cloned());
    if doc.selected_pad.as_ref().is_some_and(|pad| !pads.contains(pad)) {
        doc.selected_pad = None;
    }
    pcbcraft_chrome::part_form::mapping(ui, &pads, &mut doc.selected_pad, |ui, pad| {
        if removed.contains(pad) {
            ui.horizontal(|ui| {
                ui.label("Removed pad");
                let response = controls::button(ui, "Remove assignment");
                response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), format!("Remove assignment for pad {pad}")));
                if response.clicked() {
                    for pads in package.mapping.values_mut() {
                        pads.retain(|p| p != pad);
                    }
                    package.mapping.retain(|_, pads| !pads.is_empty());
                    package.unconnected_pads.retain(|p| p != pad);
                }
            });
            return;
        }
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
