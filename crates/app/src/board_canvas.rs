//! Board selection and transform previews over resolved engine geometry; commands own edits.
use crate::{
    canvas_camera::Camera,
    editor_view::{BoardState, ViewInput},
    preferences::Navigation,
};
use egui::{PointerButton, Sense, Ui};
use pcbcraft_chrome::board;
use pcbcraft_engine::{
    board::{
        setup::{LayerPurpose, Side},
        view::{BoardView, FootprintView},
    },
    schematic::{Point, rotate},
};
use std::collections::BTreeSet;

/// A local transform draft retains its originating input authority until completion or cancellation.
#[derive(Clone, Debug)]
pub(super) struct Draft {
    pub(super) source: ViewInput,
    pub(super) footprint: String,
    pub(super) position: Point,
    pub(super) rotation: u16,
    origin: Point,
    anchor: Point,
    drag: bool,
}

/// Selection and authored intent are handled after the renderer releases its view/session borrows.
#[derive(Default)]
pub(super) struct Interaction {
    pub(super) drawing_edit: Option<crate::board_drawing::Command>,
    pub(super) selection: Option<BTreeSet<String>>,
    pub(super) edit: Option<Draft>,
}

/// Select or preview one footprint at a time. Drags retain their grab offset; M moves the origin
/// to the pointer. R rotates counterclockwise, including during a move. Only completion on the
/// canvas emits an edit; secondary click, disabled input or stale authority discards the draft.
/// Footprint moves retain placement clicks over edges and win overlapping drag starts unless
/// an explicit drawing tool or an existing drawing edit already owns the gesture.
/// Physical pad color/visibility follow engine-resolved copper membership; the active copper
/// layer changes display priority for spanning lands without rebinding any geometry.
/// Selection filters suppress picking and transform shortcuts, without hiding rendered geometry.
#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    ui: &mut Ui,
    project: BoardView<'_>,
    camera: &mut Camera,
    state: &mut BoardState,
    source: Option<ViewInput>,
    highlighted: &BTreeSet<String>,
    filters: crate::selection_filters::Filters,
    appearance: &crate::board_appearance::Appearance,
    enabled: bool,
    navigation: Navigation,
    opacity: u8,
    grid_style: pcbcraft_chrome::grid::Style,
    label: &str,
) -> Interaction {
    let rect = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(rect, Sense::CLICK | Sense::DRAG);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Panel, true, label));
    camera.navigate_board(&response, rect, project, enabled, navigation);
    let copper = project.setup().map(|setup| {
        setup
            .layers()
            .iter()
            .filter(|layer| layer.purpose == LayerPurpose::Copper && !state.hidden_layers.contains(&layer.id.to_string()))
            .min_by_key(|layer| state.active_layer.as_ref() != Some(&layer.id.to_string()))
    });
    let visible = copper.is_none_or(|layer| layer.is_some());
    let color = board::copper_color(copper.flatten().and_then(|layer| layer.side.map(|side| side == Side::Front)));
    let footprints: Vec<_> = project.footprints().collect();
    let hit_bounds: Vec<_> = footprints
        .iter()
        .map(|footprint| {
            let (pads, graphics) = drawing(footprint, camera, rect, project, state, visible, appearance);
            if pads.is_empty() && graphics.is_empty() {
                egui::Rect::NOTHING
            } else {
                board::footprint_bounds(&pads, &graphics, camera.screen(footprint.position, rect))
            }
        })
        .collect();
    let hit = |pointer: egui::Pos2| {
        footprints.iter().zip(&hit_bounds).rev().find_map(|(footprint, bounds)| (filters.footprints && bounds.contains(pointer)).then_some(footprint))
    };
    let hovered = response.hover_pos().filter(|_| visible).and_then(hit);
    let modifiers = ui.input(|i| i.modifiers);
    let drag_start = response.drag_started_by(PointerButton::Primary) && !modifiers.shift && !modifiers.ctrl && !modifiers.command;
    // A footprint gesture owns its completion and overlapping drag origin. Explicit drawing
    // tools and existing drawing edits keep their own input even when crossing a footprint.
    let footprint_drag = drag_start
        && visible
        && state.drawing.tool.is_none()
        && state.drawing.source().is_none()
        && ui.input(|i| i.pointer.press_origin()).and_then(hit).is_some();
    let edge = crate::board_drawing::interact(
        ui,
        &response,
        camera,
        project,
        &mut state.drawing,
        state.active_layer.as_deref(),
        &state.hidden_layers,
        source,
        enabled && state.draft.is_none() && !footprint_drag,
        state.move_requested,
        filters.graphics,
    );
    let enabled = enabled && !edge.consumed;
    let mut out = Interaction { drawing_edit: edge.command, ..Interaction::default() };
    if edge.consumed && !state.selection.is_empty() {
        out.selection = Some(BTreeSet::new());
    }
    if !enabled || state.draft.as_ref().is_some_and(|draft| Some(draft.source) != source) {
        state.draft = None;
    }
    let moving = std::mem::take(&mut state.move_requested);
    let rotation = std::mem::take(&mut state.rotate_requested);
    let rotating = rotation != 0;
    let cancelled = enabled && ui.input(|i| i.pointer.button_pressed(PointerButton::Secondary));
    if cancelled {
        state.draft = None;
    }
    if enabled && !cancelled {
        if state.draft.is_none()
            && visible
            && filters.footprints
            && let Some(source) = source
        {
            let picked = if drag_start {
                ui.input(|i| i.pointer.press_origin()).and_then(hit)
            } else if moving || rotating {
                hovered.or_else(|| (state.selection.len() == 1).then(|| footprints.iter().find(|f| state.selection.contains(&f.id))).flatten())
            } else {
                None
            };
            if let Some(footprint) = picked {
                state.drawing.selected = None;
                out.selection = Some(BTreeSet::from([footprint.id.clone()]));
                let anchor = if drag_start {
                    ui.input(|i| i.pointer.press_origin()).map(|p| camera.unsnapped(p, rect)).unwrap_or(footprint.position)
                } else {
                    footprint.position
                };
                let draft = Draft {
                    source,
                    footprint: footprint.id.clone(),
                    position: footprint.position,
                    origin: footprint.position,
                    anchor,
                    rotation: footprint.rotation,
                    drag: drag_start,
                };
                if moving || drag_start {
                    state.draft = Some(draft);
                } else if rotating {
                    out.edit = Some(Draft { rotation: (draft.rotation + rotation) % 360, ..draft });
                }
            }
        }
        if let Some(draft) = &mut state.draft {
            if rotating {
                draft.rotation = (draft.rotation + rotation) % 360;
            }
            if let Some(pointer) = ui.input(|i| i.pointer.hover_pos()) {
                let pointer = camera.unsnapped(pointer, rect);
                let candidate = Point {
                    x_nm: draft.origin.x_nm.saturating_add(pointer.x_nm.saturating_sub(draft.anchor.x_nm)),
                    y_nm: draft.origin.y_nm.saturating_add(pointer.y_nm.saturating_sub(draft.anchor.y_nm)),
                };
                draft.position = camera.snap(camera.screen(candidate, rect), rect);
            }
            let completed = if draft.drag { response.drag_stopped_by(PointerButton::Primary) } else { response.clicked() && !moving };
            if completed {
                if response.contains_pointer() {
                    out.edit = state.draft.take();
                } else {
                    state.draft = None;
                }
            } else if draft.drag && !ui.input(|i| i.pointer.primary_down()) {
                state.draft = None;
            }
        } else if !moving && !rotating && response.clicked() {
            let mut selection = if modifiers.shift || modifiers.ctrl || modifiers.command { state.selection.clone() } else { BTreeSet::new() };
            if let Some(footprint) = hovered {
                if (modifiers.ctrl || modifiers.command) && selection.contains(&footprint.id) {
                    selection.remove(&footprint.id);
                } else {
                    selection.insert(footprint.id.clone());
                }
            }
            out.selection = Some(selection);
        }
    }
    let p = ui.painter().with_clip_rect(rect);
    board::canvas(&p, rect, camera.screen(Point { x_nm: 0, y_nm: 0 }, rect), camera.snap_step(), opacity, grid_style);
    crate::board_drawing::paint(ui, rect, camera, project, &state.drawing, &state.hidden_layers, appearance);
    for mut footprint in footprints {
        if let Some(draft) = state.draft.as_ref().filter(|d| d.footprint == footprint.id) {
            preview(&mut footprint, draft);
        }
        let center = camera.screen(footprint.position, rect);
        let (pads, graphics) = drawing(&footprint, camera, rect, project, state, visible, appearance);
        if (pads.is_empty() && graphics.is_empty()) || !rect.intersects(board::footprint_bounds(&pads, &graphics, center)) {
            continue;
        }
        board::footprint(
            &p,
            &pads,
            &graphics,
            if footprint.physical_reference { "" } else { footprint.reference },
            center,
            color,
            state.selection.contains(&footprint.id) || state.draft.as_ref().is_some_and(|d| d.footprint == footprint.id),
            highlighted.contains(&footprint.id),
        );
    }
    if project.footprints().next().is_none() && project.drawings().is_empty() && state.drawing.tool.is_none() {
        board::empty(&p, rect);
    }
    if state.draft.is_some() {
        response.on_hover_cursor(egui::CursorIcon::Grabbing);
    }
    out
}

/// Transform resolved pad offsets only for painting; pad identities and authored snapshots stay intact.
fn preview(footprint: &mut FootprintView<'_>, draft: &Draft) {
    let turns = ((draft.rotation + 360 - footprint.rotation) % 360 / 90) as u8;
    let transform = |point: Point| {
        let (x, y) = rotate(point.x_nm - footprint.position.x_nm, point.y_nm - footprint.position.y_nm, turns);
        Point { x_nm: draft.position.x_nm.saturating_add(x), y_nm: draft.position.y_nm.saturating_add(y) }
    };
    for pad in &mut footprint.pads {
        pad.center = transform(pad.center);
        pad.copper = pad.copper.map(|points| points.map(transform));
    }
    for graphic in &mut footprint.graphics {
        graphic.points = graphic.points.iter().copied().map(transform).collect();
    }
    footprint.position = draft.position;
    footprint.rotation = draft.rotation;
}

/// Project physical geometry without deriving dimensions or copper membership from UI state.
/// Engine memberships drive pad color/visibility; technical graphics retain their layer-role visibility.
fn drawing(
    footprint: &FootprintView<'_>,
    camera: &Camera,
    rect: egui::Rect,
    project: BoardView<'_>,
    state: &BoardState,
    legacy_visible: bool,
    appearance: &crate::board_appearance::Appearance,
) -> (Vec<board::PadDrawing>, Vec<board::GraphicDrawing>) {
    let pads = footprint
        .pads
        .iter()
        .filter_map(|pad| {
            let color = pad
                .copper_layers
                .as_ref()
                .and_then(|membership| appearance.copper(project, membership, state.active_layer.as_deref(), &state.hidden_layers));
            if (pad.copper.is_some() && color.is_none()) || (pad.copper.is_none() && !legacy_visible) {
                return None;
            }
            Some(board::PadDrawing {
                number: pad.number.clone(),
                center: camera.screen(pad.center, rect),
                copper: pad.copper.map(|points| points.map(|p| camera.screen(p, rect))),
                color,
                contour: match pad.shape {
                    pcbcraft_engine::board::definition::PadShape::Rectangle => board::PadContour::Rectangle,
                    pcbcraft_engine::board::definition::PadShape::RoundedRectangle => board::PadContour::RoundedRectangle {
                        radius: (camera.screen(Point { x_nm: pad.center.x_nm + pad.corner_radius_nm, y_nm: pad.center.y_nm }, rect)
                            - camera.screen(pad.center, rect))
                        .length(),
                    },
                    pcbcraft_engine::board::definition::PadShape::Circle => board::PadContour::Circle,
                    pcbcraft_engine::board::definition::PadShape::Oval => board::PadContour::Oval,
                },
                drill_radius: pad.drill_nm.map(|diameter| {
                    (camera.screen(Point { x_nm: pad.center.x_nm + diameter, y_nm: pad.center.y_nm }, rect) - camera.screen(pad.center, rect))
                        .length()
                        / 2.
                }),
                through_hole: pad.through_hole,
                net: pad.net.clone(),
            })
        })
        .collect();
    let graphics = footprint
        .graphics
        .iter()
        .filter_map(|graphic| {
            use pcbcraft_engine::board::definition::GraphicPurpose;
            let (purpose, style) = match graphic.purpose {
                GraphicPurpose::Fabrication => (LayerPurpose::Fabrication, board::GraphicStyle::Fabrication),
                GraphicPurpose::Silkscreen => (LayerPurpose::Silkscreen, board::GraphicStyle::Silkscreen),
                GraphicPurpose::Courtyard => (LayerPurpose::Courtyard, board::GraphicStyle::Courtyard),
            };
            appearance.technical(project, purpose, state.active_layer.as_deref(), &state.hidden_layers).map(|color| board::GraphicDrawing {
                points: graphic.points.iter().map(|p| camera.screen(*p, rect)).collect(),
                style,
                color: Some(color),
                closed: graphic.closed,
                stroke_width: graphic
                    .stroke_nm
                    .map(|width| (camera.screen(Point { x_nm: width, y_nm: 0 }, rect) - camera.screen(Point { x_nm: 0, y_nm: 0 }, rect)).length()),
            })
        })
        .collect();
    (pads, graphics)
}
