//! Saved placement discovery settings and their shared Preferences form; no project policy.
use crate::requirement_input::Draft;
use pcbcraft_chrome::{controls, settings};
use pcbcraft_engine::library::{
    filter::{Filter, Parameter},
    parts::Category,
    sources::{Catalog, Kind},
};
use serde::{Deserialize, Serialize};

const MAX_SETTINGS_BYTES: usize = 64 * 1024;

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
    /// Bound the complete persisted record so accepted GUI/live edits survive restoration.
    fn validate(&self) -> Result<(), String> {
        self.filter.validate()?;
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > MAX_SETTINGS_BYTES {
            return Err("Library preferences exceed 64 KiB.".into());
        }
        Ok(())
    }
    /// Restore only bounded, semantically valid criteria; malformed settings use inclusive defaults.
    pub(crate) fn parse(text: Option<String>) -> Self {
        text.filter(|s| s.len() <= MAX_SETTINGS_BYTES)
            .and_then(|s| serde_json::from_str::<Self>(&s).ok())
            .filter(|s| s.validate().is_ok())
            .unwrap_or_default()
    }
}
/// Parse the same bounded typed filter for live settings commands and saved preferences.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn filter_from_value(value: &serde_json::Value) -> Result<Filter, String> {
    if serde_json::to_vec(value).map_err(|e| e.to_string())?.len() > MAX_SETTINGS_BYTES {
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
        next.validate()?;
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
        settings.validate()?;
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
            settings::row(ui, "Category", |ui| {
                choice(ui, "filter_category", &mut self.category, &options);
            });
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
                let keywords: Vec<_> = category.fields.values().map(|f| f.name.as_str()).collect();
                settings::row_with_keywords(ui, "Parameter", &keywords, |ui| {
                    ui.vertical(|ui| {
                        choice(ui, "filter_field", &mut self.field, &choices);
                        if controls::enabled_button(ui, !self.field.is_empty() && self.drafts.len() < 32, "Add parameter").clicked() {
                            self.settings.filter.parameters.push(Parameter { field: self.field.clone(), requirement: None });
                            self.drafts.push(Draft::new(None));
                            self.field.clear();
                        }
                    });
                });
                let mut remove = None;
                for (i, (parameter, draft)) in self.settings.filter.parameters.iter().zip(&mut self.drafts).enumerate() {
                    ui.push_id(i, |ui| {
                        let name = &category.fields[&parameter.field].name;
                        settings::row(ui, name, |ui| {
                            ui.vertical(|ui| {
                                draft.show_presence_controls(ui, name, "Present");
                                if controls::button(ui, "Remove parameter").clicked() {
                                    remove = Some(i);
                                }
                            });
                        });
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
/// A Preferences row supplies the field label and search registration around the shared choice.
fn choice(ui: &mut egui::Ui, id: &str, value: &mut String, options: &[(&str, String)]) {
    let selected = options.iter().find(|(key, _)| *key == value.as_str()).map(|(_, text)| text.as_str()).unwrap_or(value.as_str()).to_owned();
    controls::Choice::from_id_salt(id).selected_text(selected).width(ui.available_width()).show_ui(ui, |ui| {
        for (key, text) in options {
            ui.selectable_value(value, (*key).into(), text);
        }
    });
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

#[cfg(test)]
mod tests {
    use super::*;
    use pcbcraft_engine::library::parts::{Field, FieldType};

    /// Construct a semantically valid schema large enough to exercise the complete saved boundary.
    fn large_settings(target: usize) -> Settings {
        let mut category = Category::read(include_str!("../../../examples/native-parts/categories/resistor.json")).unwrap();
        category.fields = (0..16)
            .map(|i| {
                (
                    format!("test:field-{i}"),
                    Field {
                        name: format!("Field {i}"),
                        value_type: FieldType::Choice { choices: (0..64).map(|j| format!("{j:03}{}", "x".repeat(61))).collect() },
                    },
                )
            })
            .collect();
        let mut settings = Settings { filter: Filter { enabled: true, category: Some(category), ..Default::default() }, ..Default::default() };
        let mut excess = serde_json::to_vec(&settings).unwrap().len().saturating_sub(target);
        for field in settings.filter.category.as_mut().unwrap().fields.values_mut() {
            if let FieldType::Choice { choices } = &mut field.value_type {
                for choice in choices {
                    let trim = excess.min(choice.len() - 3);
                    choice.truncate(choice.len() - trim);
                    excess -= trim;
                }
            }
        }
        assert_eq!(excess, 0);
        settings.filter.validate().unwrap();
        settings
    }

    /// GUI Apply rejects valid large schemas, including a filter fitting alone but not its wrapper.
    #[test]
    fn complete_settings_size_is_shared_by_apply_restore_and_live_updates() {
        for size in [MAX_SETTINGS_BYTES, MAX_SETTINGS_BYTES + 1, MAX_SETTINGS_BYTES + 4096] {
            let candidate = large_settings(size);
            let saved = serde_json::to_string(&candidate).unwrap();
            assert_eq!(saved.len(), size);
            let editor = Editor::new(candidate.clone(), &Catalog::default());
            if size <= MAX_SETTINGS_BYTES {
                assert_eq!(editor.value().unwrap(), candidate);
                assert_eq!(Settings::parse(Some(saved)), candidate);
            } else {
                assert!(editor.value().unwrap_err().contains("64 KiB"));
                assert_eq!(Settings::parse(Some(saved)), Settings::default());
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let kept = Settings::default();
                let params = serde_json::json!({"placement_filter": candidate.filter});
                let result = kept.updated(&params);
                assert_eq!(result.is_ok(), size <= MAX_SETTINGS_BYTES);
                assert_eq!(kept, Settings::default());
                if size == MAX_SETTINGS_BYTES + 1 {
                    // The filter alone fits: only validating the wrapper catches this rejection.
                    assert!(filter_from_value(&params["placement_filter"]).is_ok());
                    let mut app = crate::PcbApp::default();
                    app.session.apply("project.new", &serde_json::json!({})).unwrap();
                    let before = app.session.snapshot().unwrap();
                    let theme = app.preferences.theme;
                    let mut update = params.clone();
                    update["theme"] = serde_json::json!("light");
                    update["builtin_power"] = serde_json::json!(false);
                    assert!(app.run_window_command(&egui::Context::default(), "app.set_preferences", &update).is_err());
                    assert_eq!(app.library_preferences, kept);
                    assert_eq!(app.preferences.theme, theme);
                    assert!(app.session.libraries().builtin_enabled("power"));
                    assert!(std::sync::Arc::ptr_eq(&before, &app.session.snapshot().unwrap()));
                }
            }
        }
    }
}
