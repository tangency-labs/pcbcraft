//! Paint and pick component fields using the same anchors used by independent text dragging.
use super::*;
use pcbcraft_engine::schematic::fields::{Layout, Layouts};

/// A field hit retains its owner, stable field ID and pointer-to-anchor offset.
#[derive(Clone, Debug)]
pub(crate) struct Drag {
    pub symbol: String,
    pub field: String,
    pub offset: Vec2,
}

/// A drawn field owns its exact glyph bounds; hidden/empty text cannot steal body or wire input.
struct Ink {
    id: String,
    anchor: Pos2,
    shape: egui::epaint::TextShape,
    bounds: Rect,
}

/// Borrowed component text can be painted by canvases and saved-project thumbnails alike.
pub(super) struct Content<'a> {
    pub reference: &'a str,
    pub value: &'a str,
    pub metadata: &'a pcbcraft_engine::schematic::fields::Metadata,
}

/// Paint read-only component text with the same layout as interactive canvas fields.
#[cfg(any(test, not(target_arch = "wasm32")))]
pub(super) fn paint_labels(painter: &Painter, symbol: &SymbolPlacement, content: Content<'_>, center: Pos2, step: f32, color: egui::Color32) {
    for ink in ink(painter, symbol, content, center, step, color) {
        painter.add(ink.shape);
    }
}

/// Bind a sheet's component content and camera to the common field painter/picker.
fn instance_ink(ui: &Ui, project: &Project, symbol: &SymbolPlacement, camera: &Camera, rect: Rect, color: egui::Color32) -> Vec<Ink> {
    let Some(component) = project.component(symbol) else { return Vec::new() };
    ink(
        ui.painter(),
        symbol,
        Content { reference: component.reference(), value: component.value(), metadata: &component.metadata },
        camera.screen(symbol.position, rect),
        camera.unit_step(),
        color,
    )
}

/// Resolve automatic anchors exactly as legacy labels, then apply explicit physical offsets.
fn ink(painter: &Painter, symbol: &SymbolPlacement, content: Content<'_>, center: Pos2, step: f32, color: egui::Color32) -> Vec<Ink> {
    if step < theme::SCHEMATIC_MIN_GRID_SPACING {
        return Vec::new();
    }
    let drawing = drawing_bounds(&symbol.definition);
    let bounds = if drawing.is_positive() { turn_rect(drawing, symbol.turns()) } else { Rect::from_center_size(Pos2::ZERO, vec2(2., 2.)) };
    let t = Tokens::get();
    let metadata = content.metadata;
    let mut content = vec![("reference", "Reference", content.reference), ("value", "Value", content.value)];
    content.extend(metadata.fields.iter().map(|(id, f)| (id.as_str(), f.name.as_str(), f.value.as_str())));
    content
        .into_iter()
        .enumerate()
        .filter_map(|(index, (id, name, value))| {
            let default = Layout { visible: index < 2, ..Layout::default() };
            let layout = symbol.field_layout.get(id).unwrap_or(&default);
            if !layout.visible || value.is_empty() {
                return None;
            }
            let (automatic, automatic_align) = if index < 2 && symbol.turns() % 2 == 1 {
                let side = center + vec2((step * (bounds.max.x + 0.4)).max(t.space_12), step * bounds.center().y);
                if index == 0 { (side - vec2(0., t.space_2), Align2::LEFT_BOTTOM) } else { (side + vec2(0., t.space_2), Align2::LEFT_TOP) }
            } else if index == 0 {
                (center - vec2(0., (step * (0.4 - bounds.min.y)).max(t.space_12)), Align2::CENTER_BOTTOM)
            } else {
                (center + vec2(0., (step * (bounds.max.y + 0.4)).max(t.space_12) + (index - 1) as f32 * step * 1.5), Align2::CENTER_TOP)
            };
            let anchor = layout.offset.map_or(automatic, |p| {
                let (x, y) = turn(p.x_nm as f32 / GRID_NM as f32, p.y_nm as f32 / GRID_NM as f32, symbol.turns());
                center + vec2(x, y) * step
            });
            let horizontal = match layout.align {
                TextAlign::Start => egui::Align::Min,
                TextAlign::Middle => egui::Align::Center,
                TextAlign::End => egui::Align::Max,
            };
            let align = if !symbol.field_layout.contains_key(id) {
                automatic_align
            } else if layout.offset.is_none() {
                Align2([horizontal, automatic_align.y()])
            } else {
                Align2([horizontal, egui::Align::Center])
            };
            let text = if layout.show_name { format!("{name}: {value}") } else { value.to_owned() };
            let height = (step * layout.height_nm as f32 / GRID_NM as f32).max(1.);
            let galley = painter.layout_no_wrap(text, FontId::proportional(height), color);
            let pos = align.anchor_size(anchor, galley.size()).min;
            let shape = egui::epaint::TextShape::new(pos, galley, color).with_angle_and_anchor(-(layout.rotation as f32).to_radians(), align);
            let bounds = shape.visual_bounding_rect();
            Some(Ink { id: id.to_owned(), anchor, shape, bounds })
        })
        .collect()
}

/// Field text draws after bodies and wires, and highlights the exact movable text under the pointer.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint(
    ui: &Ui,
    project: &Project,
    camera: &Camera,
    rect: Rect,
    selected: &Selection,
    highlighted: &Selection,
    tool: Option<&Tool>,
    enabled: bool,
) {
    for symbol in project.symbols() {
        let color = if selected.symbols.contains(&symbol.id()) {
            Tokens::get().accent
        } else if highlighted.symbols.contains(&symbol.id()) {
            Tokens::get().schematic_highlight
        } else {
            Tokens::get().text
        };
        for field in instance_ink(ui, project, symbol, camera, rect, color) {
            if rect.intersects(field.bounds) {
                let hovered = enabled && tool.is_none() && ui.rect_contains_pointer(field.bounds);
                ui.painter_at(rect).add(field.shape);
                if hovered {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                    ui.painter_at(rect).rect_stroke(field.bounds.expand(2.), 2., Stroke::new(1., Tokens::get().accent), egui::StrokeKind::Outside);
                }
            }
        }
    }
}

/// Text is picked before bodies, but only in the symbol selection category.
pub(super) fn at(ui: &Ui, project: &Project, camera: &Camera, rect: Rect, pointer: Pos2) -> Option<Drag> {
    project.symbols().iter().rev().find_map(|symbol| {
        instance_ink(ui, project, symbol, camera, rect, Tokens::get().text)
            .into_iter()
            .rev()
            .find(|field| field.bounds.contains(pointer))
            .map(|field| Drag { symbol: symbol.id(), field: field.id, offset: field.anchor - pointer })
    })
}

/// Snap the field anchor, convert into local coordinates, and preserve all other field appearances.
pub(super) fn destination(project: &Project, camera: &Camera, rect: Rect, drag: &Drag, pointer: Pos2) -> Option<Layouts> {
    let symbol = project.symbols().iter().find(|symbol| symbol.id() == drag.symbol)?;
    let point = camera.snap(pointer + drag.offset, rect);
    let dx = point.x_nm.checked_sub(symbol.position.x_nm)?;
    let dy = point.y_nm.checked_sub(symbol.position.y_nm)?;
    let (x_nm, y_nm) = match symbol.turns() % 4 {
        0 => (dx, dy),
        1 => (-dy, dx),
        2 => (-dx, -dy),
        _ => (dy, -dx),
    };
    let mut layouts = symbol.field_layout.clone();
    layouts.entry(drag.field.clone()).or_default().offset = Some(Point { x_nm, y_nm });
    pcbcraft_engine::schematic::fields::validate_layouts(&layouts, &project.component(symbol)?.metadata).ok()?;
    Some(layouts)
}

/// Preview edits only the temporary sheet snapshot; commit uses the revision-checked command.
pub(super) fn preview(project: &Project, symbol: &str, layouts: Layouts) -> Option<Project> {
    project.with_field_layout(symbol, layouts).ok()
}
