//! Power-name interaction changes only its saved appearance, never its electrical anchor.
use super::*;
use egui::Vec2;

/// Power names are independent fields; ordinary local/global labels retain whole-label movement.
pub(super) fn ink(ui: &Ui, camera: &Camera, rect: Rect, label: &Label) -> Option<chrome::NameInk> {
    if !matches!(label.style, Style::Ground | Style::Power | Style::PowerBar) {
        return None;
    }
    chrome::name_ink(
        ui.painter(),
        camera.screen(label.at, rect),
        &label.name,
        visual(label.style),
        label.rotation,
        camera.unit_step(),
        visual_text(label, camera.unit_step()).as_ref(),
    )
}

/// Convert a snapped screen name anchor into the marker's unrotated local offset. Its name,
/// symbol orientation, appearance and electrical contact are otherwise retained.
pub(super) fn destination(camera: &Camera, rect: Rect, label: &Label, offset: Vec2, pointer: Pos2) -> Option<Label> {
    let at = camera.snap(pointer + offset, rect);
    let dx = at.x_nm.checked_sub(label.at.x_nm)?;
    let dy = at.y_nm.checked_sub(label.at.y_nm)?;
    let (x_nm, y_nm) = match label.rotation {
        90 => (-dy, dx),
        180 => (-dx, -dy),
        270 => (dy, -dx),
        _ => (dx, dy),
    };
    let mut label = label.clone();
    label.text.get_or_insert_default().offset = Some(Point { x_nm, y_nm });
    // Reuse the native appearance bounds before painting or dispatching the complete label.
    pcbcraft_engine::schematic::fields::validate_layouts(
        &std::collections::BTreeMap::from([("value".into(), label.text.clone()?)]),
        &Default::default(),
    )
    .ok()?;
    Some(label)
}

/// A held name drag finishes on release; keyboard pickup finishes on a click. Cancel, disabled
/// picking, lost focus or release outside the owning canvas discard the entire preview.
#[allow(clippy::too_many_arguments)]
pub(super) fn input(
    ui: &mut Ui,
    response: &Response,
    rect: Rect,
    camera: &Camera,
    label: &Label,
    drag: Option<Vec2>,
    enabled: bool,
    filters: crate::selection_filters::Filters,
) -> Option<Click> {
    if !enabled
        || !filters.allows_net_label(label.style)
        || ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(false))))
        || response.secondary_clicked()
    {
        return Some(Click::NetLabelCancel);
    }
    if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::R)) {
        return Some(Click::NetLabelTextRotate);
    }
    let released = drag.is_some() && ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary));
    if released || (drag.is_none() && response.clicked()) {
        return Some(if response.contains_pointer() {
            ui.input(|i| i.pointer.hover_pos())
                .and_then(|p| destination(camera, rect, label, drag.unwrap_or_default(), p))
                .map(Click::NetLabelCommit)
                .unwrap_or(Click::NetLabelCancel)
        } else {
            Click::NetLabelCancel
        });
    }
    None
}
