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

/// Context storage is shared by native viewports; collection ownership must include the host.
fn collection_key(ctx: &Context) -> egui::Id {
    egui::Id::unique(("pcbcraft.named_targets", ctx.viewport_id()))
}

/// Mark the provenance of this viewport's current raw-input pass, including retained or cleanup
/// batches. Only the host's raw-input adapter sets this flag; it is not client-controlled.
pub fn mark_synthetic_input(ctx: &Context, synthetic: bool) {
    let key = egui::Id::unique(("pcbcraft.synthetic_input", ctx.viewport_id()));
    ctx.data_mut(|data| data.insert_temp(key, synthetic));
}

/// Whether UI handlers may treat this pass as human consent. Coordinate input and keyboard
/// activation obey the same rule as named clicks; another viewport has independent provenance.
pub fn human_input(ctx: &Context) -> bool {
    let key = egui::Id::unique(("pcbcraft.synthetic_input", ctx.viewport_id()));
    !ctx.data(|data| data.get_temp::<bool>(key)).unwrap_or(false)
}

/// Enable this viewport's collection; nested viewports must opt in independently. Ordinary
/// drawing retains no target snapshots, and shared background layers never establish ownership.
pub fn begin(ctx: &Context, enabled: bool) {
    solidcraft_ui_registry::clear_handles();
    let key = collection_key(ctx);
    ctx.data_mut(|data| {
        data.insert_temp(key, enabled.then(Vec::<Target>::new));
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
    let key = collection_key(ui.ctx());
    ui.ctx().data_mut(|data| {
        if let Some(targets) = data.get_temp_mut_or_default::<Option<Vec<Target>>>(key) {
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
    let key = collection_key(ui.ctx());
    ui.ctx().data_mut(|data| {
        if let Some(targets) = data.get_temp_mut_or_default::<Option<Vec<Target>>>(key) {
            solidcraft_ui_registry::publish_target(&name, rect);
            targets.push(Target { name, rect, enabled: enabled && ui.is_enabled(), layer: ui.layer_id() });
        }
    });
}

/// Snapshot this host after drawing. Exclude controls covered by a popup or modal at their
/// click point, so a named click cannot accidentally hit the overlying control instead.
pub fn targets(ctx: &Context) -> Vec<Target> {
    let key = collection_key(ctx);
    ctx.data(|data| data.get_temp::<Option<Vec<Target>>>(key))
        .flatten()
        .unwrap_or_default()
        .into_iter()
        .filter(|target| ctx.layer_id_at(target.rect.center()) == Some(target.layer))
        .collect()
}
