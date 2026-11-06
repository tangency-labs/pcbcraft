//! Board inspection and hit testing over resolved engine geometry. Authored edits remain in commands.
use crate::{canvas_camera::Camera, editor_view::BoardState, preferences::Navigation};
use egui::{Sense, Ui};
use pcbcraft_chrome::board;
use pcbcraft_engine::{
    board::setup::{LayerPurpose, Side},
    board::view::BoardView,
    schematic::Point,
};
use std::collections::BTreeSet;

/// Select visible placements without granting input to another workspace or mutating the project.
/// Grid preferences affect drawing only; camera navigation and snapping remain independent.
#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    ui: &mut Ui,
    project: BoardView<'_>,
    camera: &mut Camera,
    state: &BoardState,
    highlighted: &BTreeSet<String>,
    enabled: bool,
    navigation: Navigation,
    opacity: u8,
    grid_style: pcbcraft_chrome::grid::Style,
    label: &str,
) -> Option<BTreeSet<String>> {
    let rect = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(rect, Sense::CLICK | Sense::DRAG);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Panel, true, label));
    camera.navigate_board(&response, rect, project, enabled, navigation);
    let p = ui.painter().with_clip_rect(rect);
    board::canvas(&p, rect, camera.screen(Point { x_nm: 0, y_nm: 0 }, rect), camera.snap_step(), opacity, grid_style);
    let copper = project.setup().map(|setup| {
        setup
            .layers()
            .iter()
            .filter(|layer| layer.purpose == LayerPurpose::Copper && !state.hidden_layers.contains(&layer.id.to_string()))
            .min_by_key(|layer| state.active_layer.as_ref() != Some(&layer.id.to_string()))
    });
    let visible = copper.is_none_or(|layer| layer.is_some());
    let color = board::copper_color(copper.flatten().and_then(|layer| layer.side.map(|side| side == Side::Front)));
    let mut hovered = None;
    let mut count = 0;
    if visible {
        for footprint in project.footprints() {
            count += 1;
            let center = camera.screen(footprint.position, rect);
            let pads: Vec<_> = footprint.pads.iter().map(|pad| (pad.number.clone(), camera.screen(pad.center, rect))).collect();
            let points: Vec<_> = pads.iter().map(|(_, point)| *point).collect();
            let bounds = board::marker_bounds(&points, center);
            if !rect.intersects(bounds) {
                continue;
            }
            board::footprint(
                &p,
                &pads,
                footprint.reference,
                center,
                color,
                state.selection.contains(&footprint.id),
                highlighted.contains(&footprint.id),
            );
            if response.hover_pos().is_some_and(|pointer| bounds.contains(pointer)) {
                hovered = Some(footprint.id);
            }
        }
    }
    if count == 0 && project.footprints().next().is_none() {
        board::empty(&p, rect);
    }
    if enabled && response.clicked() {
        let (add, toggle) = ui.input(|input| (input.modifiers.shift, input.modifiers.ctrl || input.modifiers.command));
        let mut selection = if add || toggle { state.selection.clone() } else { BTreeSet::new() };
        if let Some(id) = hovered {
            if toggle && selection.contains(&id) {
                selection.remove(&id);
            } else {
                selection.insert(id);
            }
        }
        return Some(selection);
    }
    None
}
