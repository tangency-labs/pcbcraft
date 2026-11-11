//! Board setup presentation uses the same surfaces and form controls as Craft settings.
use crate::{Tokens, fonts};
use egui::{Align, Layout, Rect, RichText, Sense, Stroke, Ui, UiBuilder, pos2, vec2};

/// Open board-wide settings with a rounded action sized to its label.
pub fn open_button(ui: &mut Ui, enabled: bool) -> egui::Response {
    ui.add_enabled(enabled, crate::catalog::button("Board setup…", false))
}

/// Reserve the setup action and compact visibility row before the layer list starts scrolling.
pub fn layer_actions_height() -> f32 {
    Tokens::get().control_height * 2.
}

/// Pages carry presentation labels only; their model and staged changes stay in the app.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Layers,
    Stackup,
    Materials,
    Silkscreen,
    Profiles,
}
impl Page {
    /// Keep the familiar board-editor/physical-stackup distinction explicit in navigation.
    pub fn title(self) -> &'static str {
        match self {
            Self::Layers => "Board editor layers",
            Self::Stackup => "Physical stackup",
            Self::Materials => "Materials",
            Self::Silkscreen => "Silkscreen",
            Self::Profiles => "Profiles",
        }
    }
}

/// Wide editors have a stable navigation rail; compact windows keep full labels in a selector.
/// Validation stays below the scrolling page so it remains visible beside the fixed form actions.
pub fn pages(ui: &mut Ui, page: &mut Page, error: Option<&str>, add: impl FnOnce(&mut Ui, Page)) {
    let previous = *page;
    let t = Tokens::get();
    let mut body = ui.available_rect_before_wrap();
    if let Some(error) = error {
        let height = t.control_height * 2.;
        let status = Rect::from_min_max(pos2(body.left(), body.bottom() - height), body.max);
        let mut child = ui.new_child(UiBuilder::new().max_rect(status.shrink(t.space_8)));
        child.label(crate::shell::error(error));
        body.max.y = status.top();
    }
    if body.width() >= t.form_card_width * 3. {
        let rail = Rect::from_min_size(body.min, vec2(t.form_card_width * 0.75, body.height()));
        ui.painter().rect_filled(rail, 0, t.dialog_footer);
        ui.painter().vline(rail.right(), rail.y_range(), Stroke::new(1., t.dialog_rule));
        let mut nav = ui.new_child(UiBuilder::new().max_rect(rail.shrink(t.space_12)).layout(Layout::top_down(Align::Min)));
        heading(&mut nav, "Board setup");
        for value in [Page::Layers, Page::Stackup, Page::Materials, Page::Silkscreen, Page::Profiles] {
            if nav
                .add_sized(
                    vec2(nav.available_width(), t.control_height + t.space_8),
                    egui::Button::new(value.title()).selected(*page == value).frame(*page == value),
                )
                .clicked()
            {
                *page = value;
            }
        }
        body.min.x = rail.right();
    } else {
        let bar = Rect::from_min_size(body.min, vec2(body.width(), t.form_header_height));
        let mut nav = ui.new_child(UiBuilder::new().max_rect(bar.shrink2(vec2(t.space_16, t.space_6))));
        egui::ComboBox::from_id_salt("setup_page")
            .selected_text(page.title())
            .width(nav.available_width())
            .show_ui(&mut nav, |ui| {
                for value in [Page::Layers, Page::Stackup, Page::Materials, Page::Silkscreen, Page::Profiles] {
                    ui.selectable_value(page, value, value.title());
                }
            })
            .response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Board setup page"));
        body.min.y = bar.bottom();
    }
    if *page != previous {
        ui.ctx().request_repaint();
    }
    let inner = body.shrink(t.space_16);
    let mut content = ui.new_child(UiBuilder::new().id_salt("setup_content").max_rect(inner).layout(Layout::top_down(Align::Min)));
    content.set_clip_rect(ui.clip_rect().intersect(inner));
    content.spacing_mut().item_spacing = vec2(t.space_8, t.space_8);
    content.visuals_mut().widgets.inactive.bg_stroke = Stroke::new(1., t.border);
    egui::ScrollArea::vertical()
        .id_salt(("setup_page", *page as u8))
        .auto_shrink([false, false])
        .max_height(inner.height())
        .show(&mut content, |ui| add(ui, *page));
}

/// Page headings use the interface face and weight shared with Preferences and the catalog.
pub fn heading(ui: &mut Ui, title: &str) {
    let t = Tokens::get();
    ui.label(RichText::new(title).font(fonts::interface_heading(ui, t.font_intro)));
    ui.add_space(t.space_8);
}

/// A bounded section card groups related physical fields without the old striped-grid chrome.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let t = Tokens::get();
    egui::Frame::new()
        .fill(t.panel)
        .stroke(Stroke::new(1., t.dialog_rule))
        .corner_radius(t.radius_md)
        .inner_margin(t.space_12)
        .show(ui, |ui| {
            ui.set_width((ui.available_width()).max(0.));
            add(ui)
        })
        .inner
}

/// Board text/graphics style and copper clearance are separate groups, with bounded inputs
/// instead of stretching three generic fields across the entire settings page.
pub fn silkscreen(ui: &mut Ui, values: [&mut String; 3]) {
    let [height, stroke, clearance] = values;
    crate::properties::section(ui, "Text and graphics");
    card(ui, |ui| {
        text(ui, "Text height", height);
        text(ui, "Stroke width", stroke);
    });
    crate::properties::section(ui, "Copper clearance");
    card(ui, |ui| {
        text(ui, "Pad clearance", clearance);
    });
}

/// Render a physical sample from the engine's actual stroke font and clearance clipping.
/// Dimensions are millimeters; the fixed viewport makes style changes visible at a stable scale.
pub fn silkscreen_preview(ui: &mut Ui, paths: &[Vec<[f32; 2]>], pads: &[[f32; 4]], stroke_mm: f32, clearance_mm: f32) {
    let t = Tokens::get();
    crate::properties::section(ui, "Preview");
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), t.control_height * 6.), Sense::hover());
    ui.painter().rect_filled(rect, t.radius_md, t.board_canvas);
    let painter = ui.painter().with_clip_rect(rect.shrink(t.space_12));
    let scale = ((rect.width() - t.space_24) / 8.).min((rect.height() - t.space_24) / 5.);
    let origin = rect.center() + vec2(0., scale * 0.5);
    let at = |p: [f32; 2]| origin + vec2(p[0], p[1]) * scale;
    for &[x, y, w, h] in pads {
        let pad = Rect::from_center_size(at([x, y]), vec2(w, h) * scale);
        painter.rect_filled(pad, t.radius_sm, t.stack_copper_highlight);
        painter.rect_stroke(pad.expand(clearance_mm * scale), t.radius_sm, Stroke::new(0.8, t.text_dim), egui::StrokeKind::Inside);
    }
    for path in paths {
        painter.add(egui::Shape::line(path.iter().copied().map(at).collect(), Stroke::new((stroke_mm * scale).max(0.5), t.board_silkscreen)));
    }
}

/// Left-align a table caption within a stable cell without letting long text widen the column.
pub fn table_label(ui: &mut Ui, label: &str, width: f32) {
    ui.allocate_ui_with_layout(vec2(width, Tokens::get().control_height), Layout::left_to_right(Align::Center), |ui| {
        ui.set_width(width);
        ui.add(egui::Label::new(label).truncate());
    });
}

/// Material details use the same aligned label column as the stackup summary.
pub fn text(ui: &mut Ui, label: &str, value: &mut String) -> egui::Response {
    field(ui, label, |ui| {
        let response = ui.add(crate::properties::input(value).desired_width(ui.available_width()));
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, label));
        response
    })
}

/// A material's routine card gives its family and usage before optional technical records.
pub fn material_heading(ui: &mut Ui, name: &str, family: Option<&str>, used: bool) {
    let t = Tokens::get();
    ui.horizontal(|ui| {
        ui.label(RichText::new(name).font(fonts::interface_heading(ui, t.font_heading)));
        if used {
            ui.label(RichText::new("In use").color(t.text_dim));
        }
    });
    if let Some(family) = family.filter(|family| *family != name) {
        ui.label(RichText::new(family).color(t.text_dim));
    }
}

/// A compact material choice exposes its family without a characterization form.
pub fn material_choice(ui: &mut Ui, name: &str, assign: bool) -> bool {
    let t = Tokens::get();
    ui.add(
        crate::catalog::button(&format!("{} {name}", if assign { "Use" } else { "Add" }), false)
            .min_size(vec2(ui.available_width(), t.control_height + t.space_8)),
    )
    .clicked()
}

/// Align summary labels with their controls independently of each caption's text width.
pub fn field<R>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        table_label(ui, label, Tokens::get().form_label_width * 2.);
        ui.with_layout(Layout::left_to_right(Align::Center), add).inner
    })
    .inner
}

/// Stack table cells share label padding and a fixed row height instead of intrinsic text widths.
pub fn stack_label(ui: &mut Ui, label: &str, width: f32) {
    ui.allocate_ui_with_layout(vec2(width, Tokens::get().control_height), Layout::left_to_right(Align::Center), |ui| {
        ui.set_width(width);
        ui.add_space(Tokens::get().space_6);
        ui.add(egui::Label::new(label).truncate());
    });
}

/// Keep selected and unselected layer names on the same text baseline and left inset.
/// Pointer and keyboard focus share a visible row surface; long names clip within the cell.
pub fn stratum_row(ui: &mut Ui, name: &str, width: f32, selected: bool) -> egui::Response {
    let t = Tokens::get();
    let (rect, response) = ui.allocate_exact_size(vec2(width, t.control_height), Sense::click());
    let fill = if selected {
        t.selected_bg
    } else if response.hovered() || response.has_focus() {
        t.hover
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, t.control_radius, fill);
    if response.has_focus() {
        ui.painter().rect_stroke(rect, t.control_radius, Stroke::new(1., t.accent), egui::StrokeKind::Inside);
    }
    ui.painter().with_clip_rect(rect).text(
        rect.left_center() + vec2(t.space_6, 0.),
        egui::Align2::LEFT_CENTER,
        name,
        egui::FontId::proportional(t.font_label),
        t.text,
    );
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, name));
    response.on_hover_text(name)
}

/// Center a dielectric lock in the same fixed-width cell as its heading and copper placeholders.
pub fn thickness_lock(ui: &mut Ui, width: f32, value: &mut bool, label: &str) -> egui::Response {
    let h = Tokens::get().control_height;
    let (cell, _) = ui.allocate_exact_size(vec2(width, h), Sense::hover());
    let mut control = ui.new_child(
        UiBuilder::new().max_rect(Rect::from_center_size(cell.center(), vec2(h, h))).layout(Layout::centered_and_justified(egui::Direction::TopDown)),
    );
    let visuals = control.visuals_mut();
    for state in [&mut visuals.widgets.inactive, &mut visuals.widgets.hovered, &mut visuals.widgets.active] {
        state.corner_radius = egui::CornerRadius::same(Tokens::get().radius_sm / 2);
    }
    let response = control.add(egui::Checkbox::without_text(value));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::CheckBox, true, label));
    response
}

/// Reserve a diagram column only when the stack table retains usable field widths.
pub fn cross_section_column(ui: &Ui) -> Option<f32> {
    (ui.available_width() > 700.).then_some(Tokens::get().form_card_width * 0.5)
}

/// Keep the scale note in the diagram's header cell without displacing the first material row.
pub fn cross_section_heading(ui: &mut Ui, width: f32) {
    let t = Tokens::get();
    ui.allocate_ui_with_layout(vec2(width, t.control_height), Layout::left_to_right(Align::Center), |ui| {
        ui.set_width(width);
        ui.add_space(t.space_4);
        ui.label(RichText::new("Not to scale").size(t.font_small).color(t.text_dim));
    });
}

/// Paint joined laminate bands from the grid's actual row rectangles, without allocating
/// independent height. Copper centers on its row; dielectric fills the gap between copper faces.
/// The diagram scrolls with the table and culls off-screen bands while retaining exact hovers.
pub fn cross_section(ui: &mut Ui, bands: &[(String, Option<i64>, bool)], rows: &[Rect]) {
    let t = Tokens::get();
    let depth = vec2(t.space_12, -t.space_8);
    for (i, ((name, thickness, copper), row)) in bands.iter().zip(rows).enumerate() {
        let (top, bottom) = if *copper {
            (row.center().y - row.height() * 0.25, row.center().y + row.height() * 0.25)
        } else {
            let top = i.checked_sub(1).and_then(|index| rows.get(index)).map_or(row.top(), |previous| previous.center().y + previous.height() * 0.25);
            let bottom = rows.get(i + 1).map_or(row.bottom(), |next| next.center().y - next.height() * 0.25);
            (top, bottom)
        };
        let band = Rect::from_min_max(pos2(row.left() + t.space_4, top), pos2(row.right() - t.space_16, bottom));
        if !band.expand(t.space_16).intersects(ui.clip_rect()) {
            continue;
        }
        let fill = if *copper { t.stack_copper } else { t.stack_dielectric };
        if i == 0 {
            ui.painter().add(egui::Shape::convex_polygon(
                vec![band.left_top(), band.right_top(), band.right_top() + depth, band.left_top() + depth],
                t.stack_copper_highlight,
                Stroke::NONE,
            ));
        }
        ui.painter().add(egui::Shape::convex_polygon(
            vec![band.right_top(), band.right_top() + depth, band.right_bottom() + depth, band.right_bottom()],
            fill.lerp_to_gamma(t.field, 0.25),
            Stroke::NONE,
        ));
        if *copper {
            let mut mesh = egui::Mesh::default();
            for (p, color) in [
                (band.left_top(), t.stack_copper_highlight),
                (band.right_top(), t.stack_copper_highlight),
                (band.right_bottom(), fill),
                (band.left_bottom(), fill),
            ] {
                mesh.colored_vertex(p, color);
            }
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            ui.painter().add(egui::Shape::mesh(mesh));
        } else {
            ui.painter().rect_filled(band, 0, fill);
            let painter = ui.painter().with_clip_rect(band);
            let mut x = band.left() - band.height();
            while x < band.right() {
                painter.line_segment([pos2(x, band.bottom()), pos2(x + band.height(), band.top())], Stroke::new(0.7, t.stack_dielectric_weave));
                x += t.space_8;
            }
            let mut line_y = band.top() + t.space_8;
            while line_y < band.bottom() {
                painter.hline(band.x_range(), line_y, Stroke::new(0.5, t.stack_dielectric_weave));
                line_y += t.space_8;
            }
        }
        ui.interact(band, ui.scope_id().with(("stratum", i)), Sense::hover()).on_hover_text(format!(
            "{name}: {}",
            thickness.map(|n| crate::units::label(n, crate::units::Unit::Mm)).unwrap_or_else(|| "Unspecified".into())
        ));
    }
}
