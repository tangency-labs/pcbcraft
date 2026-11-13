//! Named interaction geometry layered over the pinned Craft widget registry.
use egui::{Context, LayerId, Rect, Response, Ui};

/// One visible control, including its clipped hit area and ordinary enabled state.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    /// Host-supplied alias, independent of physical desktop coordinates.
    pub name: String,
    /// Visible, clipped hit area in the current viewport's logical points.
    pub rect: Rect,
    /// The host's action guard combined with the response's ordinary enabled state.
    pub enabled: bool,
    layer: LayerId,
}

/// Enable collection for an observed editor; ordinary drawing retains no target snapshots.
pub fn begin(ctx: &Context, enabled: bool) {
    solidcraft_ui_registry::clear_handles();
    ctx.data_mut(|data| {
        data.insert_temp(egui::Id::unique("pcbcraft.named_targets"), enabled.then(Vec::<Target>::new));
    });
}

/// Publish an explicit interaction name through the shared registry. Invisible sizing passes
/// and fully clipped controls have no target; duplicates are retained for ambiguity checks.
pub fn publish(ui: &Ui, response: &Response, name: impl Into<String>) {
    publish_enabled(ui, response, name, true);
}

/// Retain a host's explicit enabled state when the shared control guards its action separately
/// from the egui response, as the pinned ribbon and menu do.
pub fn publish_enabled(ui: &Ui, response: &Response, name: impl Into<String>, enabled: bool) {
    let rect = response.interact_rect.intersect(ui.clip_rect());
    if ui.is_sizing_pass() || !ui.is_visible() || !rect.is_positive() {
        return;
    }
    let name = name.into();
    ui.ctx().data_mut(|data| {
        if let Some(targets) = data.get_temp_mut_or_default::<Option<Vec<Target>>>(egui::Id::unique("pcbcraft.named_targets")) {
            solidcraft_ui_registry::publish_target(&name, rect);
            targets.push(Target { name, rect, enabled: enabled && response.enabled(), layer: response.layer_id });
        }
    });
}

/// Name a host-owned canvas region without adding a widget or changing its input routing.
pub fn region(ui: &Ui, name: impl Into<String>, rect: Rect, enabled: bool) {
    let rect = rect.intersect(ui.clip_rect());
    if ui.is_sizing_pass() || !ui.is_visible() || !rect.is_positive() {
        return;
    }
    let name = name.into();
    ui.ctx().data_mut(|data| {
        if let Some(targets) = data.get_temp_mut_or_default::<Option<Vec<Target>>>(egui::Id::unique("pcbcraft.named_targets")) {
            solidcraft_ui_registry::publish_target(&name, rect);
            targets.push(Target { name, rect, enabled: enabled && ui.is_enabled(), layer: ui.layer_id() });
        }
    });
}

/// Snapshot this host after drawing. Exclude controls covered by a popup or modal at their
/// click point, so a named click cannot accidentally hit the overlying control instead.
pub fn targets(ctx: &Context) -> Vec<Target> {
    ctx.data(|data| data.get_temp::<Option<Vec<Target>>>(egui::Id::unique("pcbcraft.named_targets")))
        .flatten()
        .unwrap_or_default()
        .into_iter()
        .filter(|target| ctx.layer_id_at(target.rect.center()) == Some(target.layer))
        .collect()
}
