//! Board setup presentation uses the same surfaces and form controls as Craft settings.
use crate::{Tokens, fonts};
use egui::{Align, Layout, Rect, RichText, Sense, Stroke, Ui, UiBuilder, pos2, vec2};

/// Pages carry presentation labels only; their model and staged changes stay in the app.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Layers,
    Stackup,
    Materials,
}
impl Page {
    /// Keep the familiar board-editor/physical-stackup distinction explicit in navigation.
    pub fn title(self) -> &'static str {
        match self {
            Self::Layers => "Board editor layers",
            Self::Stackup => "Physical stackup",
            Self::Materials => "Materials",
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
        heading(&mut nav, "Board stackup");
        for value in [Page::Layers, Page::Stackup, Page::Materials] {
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
                for value in [Page::Layers, Page::Stackup, Page::Materials] {
                    ui.selectable_value(page, value, value.title());
                }
            })
            .response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Board setup page"));
        body.min.y = bar.bottom();
    }
    if *page != previous {
        ui.ctx().request_repaint();
    }
    let inner = body.shrink(t.space_16);
    let mut content = ui.new_child(UiBuilder::new().id_salt("setup_content").max_rect(inner).layout(Layout::top_down(Align::Min)));
    content.set_clip_rect(ui.clip_rect().intersect(inner));
    content.spacing_mut().item_spacing = vec2(t.space_8, t.space_8);
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

/// A physical cross-section keeps copper thin and dielectric gaps readable, with unknown
/// dimensions represented by equal display bands rather than invented physical proportions.
pub fn cross_section(ui: &mut Ui, bands: &[(String, Option<i64>, bool)]) {
    let t = Tokens::get();
    let width = t.form_card_width * 0.5;
    let height = (bands.len() as f32 * t.control_height).min(t.control_height * 14.);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    for (i, (name, thickness, copper)) in bands.iter().enumerate() {
        let h = height / bands.len().max(1) as f32;
        let row = Rect::from_min_size(pos2(rect.left(), rect.top() + i as f32 * h), vec2(width, h));
        let fill = if *copper { crate::board::copper_color(Some(true)) } else { t.selected_bg };
        let band = Rect::from_center_size(row.center(), vec2(width - t.space_8, if *copper { t.space_4 } else { (h - t.space_8).max(t.space_4) }));
        ui.painter().rect_filled(band, t.radius_sm, fill);
        ui.interact(row, ui.id().with(("stratum", i)), Sense::hover()).on_hover_text(format!(
            "{name}: {}",
            thickness.map(|n| crate::units::label(n, crate::units::Unit::Mm)).unwrap_or_else(|| "Unspecified".into())
        ));
    }
    ui.label(RichText::new("Not to scale").size(t.font_small).color(t.text_dim));
}
