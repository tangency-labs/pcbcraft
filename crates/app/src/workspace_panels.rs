//! Per-view card placement and edge stacking stay outside the electrical session.
use crate::PcbApp;
#[cfg(not(target_arch = "wasm32"))]
use crate::editor_view::{ViewId, Workspace};
use egui::Rect;
use pcbcraft_chrome::workspace::panels::{Panel, PanelState, Position};
#[cfg(not(target_arch = "wasm32"))]
use serde_json::{Value, json};

pub(super) const LAYERS: usize = 0;
pub(super) const PROPERTIES: usize = 1;
pub(super) const FILTERS: usize = 2;

/// Fresh contextual cards reproduce the FigCraft left/right arrangement.
pub(super) fn defaults() -> [Panel; 3] {
    [Panel::new(Position::Left), Panel::new(Position::Right), Panel::new(Position::Right)]
}

/// Older arrangement records have the same edge defaults as new views.
pub(super) fn saved_defaults() -> SavedPanels {
    SavedPanels::capture(&defaults())
}

/// Named card identities keep local arrangement records independent of runtime slot ordering.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SavedPanels {
    layers: PanelState,
    properties: PanelState,
    selection_filters: PanelState,
}

impl Default for SavedPanels {
    fn default() -> Self {
        saved_defaults()
    }
}

impl SavedPanels {
    /// Capture positions without serializing any active pointer gesture.
    pub(super) fn capture(panels: &[Panel; 3]) -> Self {
        Self { layers: panels[LAYERS].saved_state(), properties: panels[PROPERTIES].saved_state(), selection_filters: panels[FILTERS].saved_state() }
    }
    /// Refuse invalid saved coordinates before adopting any part of the arrangement.
    pub(super) fn valid(&self) -> bool {
        self.layers.valid() && self.properties.valid() && self.selection_filters.valid()
    }
    /// Recreate runtime cards in their initial slot order with no drag ownership.
    pub(super) fn restore(&self) -> [Panel; 3] {
        [Panel::from_state(self.layers), Panel::from_state(self.properties), Panel::from_state(self.selection_filters)]
    }
}

/// Split edge slots between visible cards, keeping filters at the bottom and Browser above
/// left-docked cards. Floating cards consume no dock slot and clamp against the full host instead.
pub(super) fn areas(canvas: Rect, browser: Rect, panels: &[Panel; 3], visible: [bool; 3], schematic: bool) -> [Rect; 3] {
    let mut result = [canvas; 3];
    for side in [Position::Left, Position::Right] {
        let members: Vec<_> = (0..3).filter(|i| visible[*i] && panels[*i].position() == side).collect();
        if members.is_empty() {
            continue;
        }
        let top = if browser.is_positive()
            && (side == Position::Left || (schematic && canvas.width() < pcbcraft_chrome::workspace::panels::COMPACT_HOST_WIDTH))
        {
            browser.bottom()
        } else {
            canvas.top()
        };
        let height = (canvas.bottom() - top).max(0.0);
        let filters = members.contains(&FILTERS);
        let ordinary = members.len() - usize::from(filters);
        let filter_height = if filters {
            pcbcraft_chrome::workspace::panels::FILTER_DOCK_HEIGHT.min(if ordinary == 0 { height } else { height * 0.45 })
        } else {
            0.0
        };
        let each = if ordinary > 0 { (height - filter_height) / ordinary as f32 } else { 0.0 };
        let mut y = top;
        for id in members.into_iter().filter(|i| *i != FILTERS) {
            result[id] = Rect::from_min_max(egui::pos2(canvas.left(), y), egui::pos2(canvas.right(), y + each));
            y += each;
        }
        if filters {
            result[FILTERS] = Rect::from_min_max(egui::pos2(canvas.left(), canvas.bottom() - filter_height), canvas.max);
        }
    }
    result
}

impl PcbApp {
    /// Apply a card position atomically through the GUI/live shared placement state. Layers
    /// requires a PCB view; Properties also supports definition panes. Unknown panels, bad
    /// coordinates and unavailable views change nothing.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn set_panel_position(&mut self, id: ViewId, params: &Value) -> Result<(), String> {
        let view = self
            .views
            .view(id)
            .filter(|view| view.library.is_some() || view.definition.is_some() || !view.unavailable())
            .ok_or("Panel placement requires an available editor view")?;
        let panel = match params["panel"].as_str() {
            Some("layers") if view.library.is_none() && view.definition.is_none() && view.workspace == Workspace::Board => LAYERS,
            Some("properties") => PROPERTIES,
            Some("preview") if view.library.is_some() => PROPERTIES,
            Some("selection_filters") if view.library.is_none() && view.definition.is_none() => FILTERS,
            _ => return Err("That panel is unavailable in this workspace".into()),
        };
        let position = match params["position"].as_str() {
            Some("left") => Position::Left,
            Some("right") => Position::Right,
            Some("floating") => Position::Floating,
            _ => return Err("Use left, right or floating for panel position".into()),
        };
        let at = match (params.get("x"), params.get("y")) {
            (None, None) => None,
            (Some(x), Some(y)) if position == Position::Floating => {
                let (x, y) = (x.as_f64().ok_or("x must be a number")?, y.as_f64().ok_or("y must be a number")?);
                Some([x as f32, y as f32])
            }
            _ => return Err("Floating coordinates require both x and y".into()),
        };
        let size = match (params.get("width"), params.get("height")) {
            (None, None) => view.panels[panel].saved_state().size,
            (Some(width), Some(height)) if view.library.is_some() && panel == PROPERTIES && position == Position::Floating => {
                Some([width.as_f64().ok_or("width must be a number")? as f32, height.as_f64().ok_or("height must be a number")? as f32])
            }
            _ => return Err("Dimensions require a floating Library Preview and both width and height".into()),
        };
        let state = PanelState { position, at, size };
        if !state.valid() {
            return Err("Panel coordinates are out of range".into());
        }
        self.views.view_mut(id).ok_or("The addressed view closed")?.panels[panel] = Panel::from_state(state);
        Ok(())
    }

    /// Cancel a card drag before Escape reaches a canvas tool, restoring its previous anchor.
    pub(super) fn take_panel_escape(&mut self, ctx: &egui::Context) -> bool {
        if !ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            return false;
        }
        let mut cancelled = false;
        for panel in &mut self.views.active.panels {
            cancelled |= panel.cancel_drag();
        }
        if cancelled {
            ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            ctx.request_repaint();
        }
        cancelled
    }
}

/// Inspection uses stable panel identities, with placement expressed in logical host coordinates.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn inspect(panels: &[Panel; 3]) -> Value {
    json!({"layers":panels[LAYERS].saved_state(),"properties":panels[PROPERTIES].saved_state(),"selection_filters":panels[FILTERS].saved_state()})
}
