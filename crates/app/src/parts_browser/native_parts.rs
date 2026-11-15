//! Native part presentation captures immutable engine selections; validation stays in the engine.
use super::*;
use pcbcraft_engine::{
    library::sources::{Catalog, Kind, ResolvedPart},
    part_instances::Selection,
};

/// Rows keep their symbol preview and optional central part record under separate immutable identities.
#[derive(Clone, Debug)]
pub(super) struct Entry(pub(super) Arc<SymbolDefinition>, pub(super) Option<Arc<ResolvedPart>>, String);
impl Entry {
    /// A symbol-only row retains existing placement defaults and key semantics.
    pub(super) fn symbol(symbol: Arc<SymbolDefinition>) -> Self {
        let value = symbol.default_value().to_owned();
        Self(symbol, None, value)
    }
    /// A native part row derives its visible Value only from explicit primary requirements.
    fn part(part: Arc<ResolvedPart>) -> Option<Self> {
        let symbol = part.symbol.clone()?;
        let selection = initial(part.clone());
        let value = selection.display_primary_value().unwrap_or_else(|| symbol.default_value().to_owned());
        Some(Self(symbol, Some(part), value))
    }
    /// The native part identity stays distinct from its reusable symbol's identity.
    pub(super) fn key(&self) -> Key {
        match &self.1 {
            Some(part) => Key::Part(part.definition.library.clone(), part.definition.id.clone(), part.definition.revision),
            None => Key::Symbol(self.0.library.clone(), self.0.id.clone(), self.0.revision()),
        }
    }
    /// Show the part's name when present; a backing symbol does not become a second catalog identity.
    pub(super) fn name(&self) -> &str {
        self.1.as_ref().map_or(self.0.name.as_str(), |part| part.definition.name.as_str())
    }
    /// Preserve the canonical primary default captured with the immutable catalog row.
    pub(super) fn default_value(&self) -> &str {
        &self.2
    }
    /// Scope by the selected record's library, including cross-library symbol dependencies.
    pub(super) fn in_collection(&self, collection: &Collection) -> bool {
        match collection {
            Collection::All => true,
            Collection::Library(id) => id == self.1.as_ref().map_or(&self.0.library, |part| &part.definition.library),
        }
    }
}
/// A native selection starts generic and may remain without a physical package during schematic work.
fn initial(part: Arc<ResolvedPart>) -> Selection {
    let package = (part.definition.packages.len() == 1).then(|| part.definition.packages.keys().next().cloned()).flatten();
    Selection { resolved: part, package, manufacturer: None, requirements: Default::default() }
}
/// List actual reusable parts while preserving symbol-only sources and avoiding duplicate backing-symbol rows.
pub(super) fn entries(query: &str, catalog: &Catalog) -> Vec<Entry> {
    let mut entries: Vec<_> = catalog
        .search(query, Some(Kind::Part))
        .into_iter()
        .filter_map(|(_, item)| {
            let part = item.part()?;
            catalog.resolve_part(&part.reference()).ok().and_then(|part| Entry::part(Arc::new(part.clone())))
        })
        .collect();
    let represented: std::collections::BTreeSet<_> =
        entries.iter().map(|entry| (entry.0.library.clone(), entry.0.id.clone(), entry.0.revision())).collect();
    entries.extend(
        super::matches_sources(query, catalog)
            .into_iter()
            .filter(|symbol| !represented.contains(&(symbol.library.clone(), symbol.id.clone(), symbol.revision())))
            .map(Entry::symbol),
    );
    entries
}
impl PartsBrowser {
    /// Selecting a row discards only the previous uncommitted catalog selection, never project state.
    pub(super) fn select_entry(&mut self, entry: &Entry) {
        self.selected = Some(entry.key());
        self.value = entry.default_value().to_owned();
        self.part = entry.1.clone().map(initial);
    }
    /// Package and manufacturer choices capture the same typed selection used by placement commands.
    pub(super) fn part_controls(&mut self, ui: &mut Ui, catalog: &Catalog) {
        let Some(part) = &mut self.part else {
            return;
        };
        properties::field(ui, "Package", |ui| {
            let label =
                part.package.as_ref().and_then(|id| part.resolved.definition.packages.get(id)).map_or("Unassigned", |package| package.name.as_str());
            let response =
                egui::ComboBox::from_id_salt("part_package").width(ui.available_width()).truncate().selected_text(label).show_ui(ui, |ui| {
                    ui.selectable_value(&mut part.package, None, "Unassigned");
                    for (id, package) in &part.resolved.definition.packages {
                        ui.selectable_value(&mut part.package, Some(id.clone()), &package.name);
                    }
                });
            response.response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Package"));
        });
        properties::field(ui, "Manufacturer part", |ui| {
            let label =
                part.manufacturer.as_ref().map(|record| format!("{} · {}", record.manufacturer, record.mpn)).unwrap_or_else(|| "Generic".into());
            let response =
                egui::ComboBox::from_id_salt("part_manufacturer").width(ui.available_width()).truncate().selected_text(label).show_ui(ui, |ui| {
                    if ui.selectable_label(part.manufacturer.is_none(), "Generic").clicked() {
                        part.manufacturer = None;
                    }
                    for record in catalog.manufacturers_for(&part.resolved.definition.reference()) {
                        let selected = part.manufacturer.as_ref().is_some_and(|current| current.reference() == record.reference());
                        if ui.selectable_label(selected, format!("{} · {}", record.manufacturer, record.mpn)).clicked() {
                            part.package = Some(record.package.clone());
                            part.manufacturer = Some(record);
                            part.manufacturer_primary_default();
                            if let Some(value) = part.display_primary_value() {
                                self.value = value;
                            }
                        }
                    }
                });
            response.response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Manufacturer part"));
        });
        match self.capture_part() {
            Err(error) => {
                ui.label(pcbcraft_chrome::shell::error(error));
            }
            Ok(Some(selection)) => {
                let compatibility = selection.compatibility();
                if compatibility["status"] == "does_not_meet_requirements" {
                    ui.label(pcbcraft_chrome::shell::error("Manufacturer specifications do not meet the selected requirements."));
                }
                if let Some(manufacturer) = &selection.manufacturer {
                    for (id, value) in &manufacturer.specifications {
                        let name = selection
                            .resolved
                            .category
                            .as_ref()
                            .and_then(|category| category.fields.get(id))
                            .map_or(id.as_str(), |field| field.name.as_str());
                        properties::field(ui, name, |ui| {
                            ui.label(value.display());
                        });
                    }
                }
            }
            _ => {}
        }
    }
    /// Normalization and fixed-value checks use the engine's explicit category binding.
    pub(super) fn capture_part(&self) -> Result<Option<Selection>, String> {
        self.part.as_ref().map(|part| part.with_primary_value(&self.value)).transpose()
    }
    /// Click, Enter and drag all retain the same package, MPN and canonical primary requirement.
    pub(super) fn capture_placement(&self, entry: &Entry, value: &str) -> Option<Placement> {
        if !valid_value(value) {
            return None;
        }
        let part = if self.selected == Some(entry.key()) {
            self.capture_part().ok()?
        } else {
            entry.1.clone().map(initial).map(|part| part.with_primary_value(value)).transpose().ok()?
        };
        if part.as_ref().is_some_and(|part| part.manufacturer.is_some() && part.compatibility()["status"] != "meets_requirements") {
            return None;
        }
        Some(Placement { definition: entry.0.clone(), value: value.to_owned(), part })
    }
}
