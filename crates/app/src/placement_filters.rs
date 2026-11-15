//! Saved placement discovery settings and their shared Preferences form; no project policy.
use crate::requirement_input::Draft;
use pcbcraft_chrome::{properties, settings};
use pcbcraft_engine::library::{
    filter::{Filter, Parameter},
    parts::Category,
    sources::{Catalog, Kind},
};
use serde::{Deserialize, Serialize};

pub(crate) const STORAGE_KEY: &str = "pcbcraft.library-preferences.v1";
/// Library availability and placement filtering are independent personal preferences.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Settings {
    pub basic_components: bool,
    pub power: bool,
    pub filter: Filter,
}
impl Default for Settings {
    fn default() -> Self {
        Self { basic_components: true, power: true, filter: Filter::default() }
    }
}
impl Settings {
    /// Restore only bounded, semantically valid criteria; malformed settings use inclusive defaults.
    pub(crate) fn parse(text: Option<String>) -> Self {
        text.filter(|s| s.len() <= 64 * 1024)
            .and_then(|s| serde_json::from_str::<Self>(&s).ok())
            .filter(|s| s.filter.validate().is_ok())
            .unwrap_or_default()
    }
}
/// Parse the same bounded typed filter for live settings commands and saved preferences.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn filter_from_value(value: &serde_json::Value) -> Result<Filter, String> {
    if serde_json::to_vec(value).map_err(|e| e.to_string())?.len() > 64 * 1024 {
        return Err("Placement filters exceed 64 KiB.".into());
    }
    let filter: Filter = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    filter.validate()?;
    Ok(filter)
}
#[cfg(not(target_arch = "wasm32"))]
impl Settings {
    /// Build a complete candidate before applying partial live preference updates atomically.
    pub(crate) fn updated(&self, params: &serde_json::Value) -> Result<Self, String> {
        let mut next = self.clone();
        if let Some(enabled) = params["builtin_basic_components"].as_bool() {
            next.basic_components = enabled;
        }
        if let Some(enabled) = params["builtin_power"].as_bool() {
            next.power = enabled;
        }
        if let Some(filter) = params.get("placement_filter") {
            next.filter = filter_from_value(filter)?;
        }
        Ok(next)
    }
}
/// Unconfirmed text stays in this modal draft. Cancel never applies library/filter changes.
#[derive(Clone, Debug)]
pub(crate) struct Editor {
    settings: Settings,
    kept: Settings,
    drafts: Vec<Draft>,
    categories: Vec<Category>,
    category: String,
    field: String,
    pub(crate) error: Option<String>,
}
impl Editor {
    /// Preserve category meanings with saved filters even when their source is unavailable.
    pub(crate) fn new(settings: Settings, catalog: &Catalog) -> Self {
        let mut categories = std::collections::BTreeMap::new();
        for (_, item) in catalog.search("", Some(Kind::Category)) {
            if let Some(c) = item.category() {
                categories.insert(c.reference(), (*c).clone());
            }
        }
        if let Some(c) = &settings.filter.category {
            categories.insert(c.reference(), c.clone());
        }
        let category = settings.filter.category.as_ref().map(key).unwrap_or_default();
        let drafts = settings.filter.parameters.iter().map(|p| Draft::new(p.requirement.as_ref())).collect();
        Self { kept: settings.clone(), settings, drafts, categories: categories.into_values().collect(), category, field: String::new(), error: None }
    }
    /// Convert all typed text together; an invalid comparison leaves the current settings intact.
    pub(crate) fn value(&self) -> Result<Settings, String> {
        let mut settings = self.settings.clone();
        for (p, draft) in settings.filter.parameters.iter_mut().zip(&self.drafts) {
            let field = settings.filter.category.as_ref().and_then(|c| c.fields.get(&p.field)).ok_or("Parameter category is unavailable.")?;
            p.requirement = draft.requirement(field)?;
        }
        settings.filter.validate()?;
        Ok(settings)
    }
    /// Incomplete input enables Apply so the form can explain why it cannot yet be saved.
    pub(crate) fn dirty(&self) -> bool {
        self.value().map_or(true, |value| value != self.kept)
    }
    /// Keep the confirmed state for further edits in the same Preferences window.
    pub(crate) fn applied(&mut self, value: Settings) {
        self.settings = value.clone();
        self.kept = value;
        self.error = None;
    }
    /// Restore inclusive page defaults while retaining the last applied Cancel boundary.
    pub(crate) fn defaults(&mut self) {
        self.settings = Settings::default();
        self.category.clear();
        self.field.clear();
        self.drafts.clear();
        self.error = None;
    }
    /// Compose library switches and category-typed placement filters from existing Craft fields.
    pub(crate) fn show(&mut self, ui: &mut egui::Ui) {
        settings::section(ui, "Built-in libraries", |ui| {
            settings::row(ui, "Basic components", |ui| {
                settings::checkbox(ui, "Enabled", "Include Basic components", &mut self.settings.basic_components);
            });
            settings::row(ui, "Power", |ui| {
                settings::checkbox(ui, "Enabled", "Include Power", &mut self.settings.power);
            });
        });
        settings::section(ui, "Parts Catalog placement filters", |ui| {
            settings::row_with_keywords(ui, "Placement filters", &["specified", "MPN", "manufacturer", "voltage", "parameters"], |ui| {
                settings::checkbox(ui, "Enabled", "Use placement filters", &mut self.settings.filter.enabled);
            });
            settings::row(ui, "Manufacturer and MPN", |ui| {
                settings::checkbox(ui, "Required", "Require manufacturer and MPN", &mut self.settings.filter.manufacturer);
            });
            settings::row(ui, "Footprint and pin map", |ui| {
                settings::checkbox(ui, "Required", "Require footprint and pin map", &mut self.settings.filter.footprint);
            });
            let category_options: Vec<_> = std::iter::once((String::new(), "All categories".into()))
                .chain(self.categories.iter().map(|c| (key(c), format!("{} · {}/{} r{}", c.name, c.library, c.id, c.revision))))
                .collect();
            let options: Vec<_> = category_options.iter().map(|(id, name)| (id.as_str(), name.clone())).collect();
            let before = self.category.clone();
            properties::choice(ui, "Category", ui.scope_id().with("filter_category"), &mut self.category, &options);
            if before != self.category {
                self.settings.filter.category = self.categories.iter().find(|c| key(c) == self.category).cloned();
                self.settings.filter.parameters.clear();
                self.drafts.clear();
                self.field.clear();
                self.error = None;
            }
            if let Some(category) = self.settings.filter.category.clone() {
                let choices: Vec<_> = category
                    .fields
                    .iter()
                    .filter(|(id, _)| !self.settings.filter.parameters.iter().any(|p| p.field == **id))
                    .map(|(id, f)| (id.as_str(), f.name.clone()))
                    .collect();
                if !choices.iter().any(|(id, _)| *id == self.field) {
                    self.field = choices.first().map(|(id, _)| (*id).to_owned()).unwrap_or_default();
                }
                properties::choice(ui, "Parameter", ui.scope_id().with("filter_field"), &mut self.field, &choices);
                if pcbcraft_chrome::controls::enabled_button(ui, !self.field.is_empty() && self.drafts.len() < 32, "Add parameter").clicked() {
                    self.settings.filter.parameters.push(Parameter { field: self.field.clone(), requirement: None });
                    self.drafts.push(Draft::new(None));
                    self.field.clear();
                }
                let mut remove = None;
                for (i, (parameter, draft)) in self.settings.filter.parameters.iter().zip(&mut self.drafts).enumerate() {
                    ui.push_id(i, |ui| {
                        let name = &category.fields[&parameter.field].name;
                        draft.show_presence(ui, name, "Present");
                        if pcbcraft_chrome::controls::button(ui, "Remove parameter").clicked() {
                            remove = Some(i);
                        }
                    });
                }
                if let Some(i) = remove {
                    self.settings.filter.parameters.remove(i);
                    self.drafts.remove(i);
                }
            }
            if let Some(error) = &self.error {
                ui.colored_label(pcbcraft_chrome::Tokens::get().error, error);
            }
        });
    }
}
/// A dropdown key is explicit native identity, never a display-name inference.
fn key(c: &Category) -> String {
    format!("{}/{}@{}", c.library, c.id, c.revision)
}

impl crate::PcbApp {
    /// Synchronize saved personal discovery choices without changing project bytes or undo.
    pub(crate) fn sync_library_preferences(&mut self) {
        for (library, enabled) in [("basic-components", self.library_preferences.basic_components), ("power", self.library_preferences.power)] {
            if self.session.libraries().builtin_enabled(library) != enabled
                && let Err(error) = self.session.set_builtin_library_enabled(library, enabled)
            {
                self.message = Some(error.to_string());
            }
        }
        self.parts.set_filter(self.library_preferences.filter.clone());
    }
}
