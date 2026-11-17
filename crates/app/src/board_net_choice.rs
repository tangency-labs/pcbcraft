//! Shared typed net choices for board intent forms; display names never grant authority.
use pcbcraft_engine::nets::ClassTarget;
use serde_json::Value;
/// A revision-qualified derived/persistent target and display label; names never grant authority.
#[derive(Clone, Debug)]
pub(super) struct NetChoice {
    pub(super) label: String,
    pub(super) target: ClassTarget,
    pub(super) id: Option<uuid::Uuid>,
}
/// Build picker presentation once from an engine inspection; unnamed nets remain distinguishable.
pub(super) fn net_choices(rows: &Value) -> Vec<NetChoice> {
    rows["nets"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, row)| {
            let anchor: pcbcraft_engine::nets::Anchor = serde_json::from_value(row["anchor"].clone()).ok()?;
            let label = row["name"].as_str().filter(|name| !name.is_empty()).map_or_else(|| format!("Net {}", index + 1), str::to_owned);
            let id = serde_json::from_value(row["id"].clone()).ok();
            let target = if let Some(net) = id {
                ClassTarget::Persistent { project: anchor.project, scope: anchor.scope, net }
            } else {
                ClassTarget::Derived { anchor }
            };
            Some(NetChoice { label, target, id })
        })
        .collect()
}

/// Render the existing Craft dropdown with automatic mode and an inspectable current selection.
/// Return only a newly selected policy; closing or dismissing the menu makes no edit.
pub(super) fn picker(
    ui: &mut egui::Ui,
    label: &str,
    id: egui::Id,
    choices: &[NetChoice],
    current: &str,
) -> (egui::Response, Option<Option<ClassTarget>>) {
    let mut selected = "current".to_owned();
    let response = pcbcraft_chrome::controls::Choice::from_id_salt(id).selected_text(current).width(ui.available_width()).show_ui(ui, |ui| {
        if pcbcraft_chrome::MenuItem::new("Automatic").choice(current == "Automatic").show(ui) {
            selected = "automatic".into();
        }
        for (i, choice) in choices.iter().enumerate() {
            if pcbcraft_chrome::MenuItem::new(&choice.label).choice(current == choice.label).show(ui) {
                selected = i.to_string();
            }
        }
    });
    response.response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, ui.is_enabled(), label));
    pcbcraft_chrome::automation::publish(ui, &response.response, format!("field:{label}"));
    let choice = if selected == "automatic" {
        Some(None)
    } else {
        selected.parse::<usize>().ok().and_then(|i| choices.get(i)).map(|c| Some(c.target.clone()))
    };
    (response.response, choice)
}
