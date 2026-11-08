//! Craft catalog and property forms share the source manager's header, body and footer rhythm.
use crate::{Tokens, fonts, menu::MenuIcon};
use egui::{Align, Frame, Layout, Rect, RichText, Sense, Stroke, Ui, UiBuilder, pos2, vec2};

/// Decisions from a form remain presentation events; callers own documents and validation.
#[derive(Default)]
pub struct FormAction {
    pub close: bool,
    pub primary: bool,
    pub libraries: bool,
    pub compact: bool,
}

/// A wide catalog or bounded properties form with a fixed action footer and clipped body.
/// The supplied primary label distinguishes Place from Apply; only catalogs expose library management.
/// `active` inherits input blocking from the owning editor, including another native host's dialog.
#[allow(clippy::too_many_arguments)]
pub fn form(
    ctx: &egui::Context,
    id: &'static str,
    title: &str,
    width: f32,
    primary: &str,
    enabled: bool,
    libraries: bool,
    active: bool,
    add: impl FnOnce(&mut Ui),
) -> FormAction {
    let t = Tokens::get();
    let mut action = FormAction::default();
    let frame = Frame::new().fill(t.dialog_body).stroke(Stroke::new(1.0, t.border)).corner_radius(t.radius_lg);
    let modal = egui::Modal::new(id.into()).frame(frame).show(ctx, |ui| {
        if !active {
            ui.disable();
        }
        let width = width.min((ctx.content_rect().width() - 2. * t.space_16).max(0.));
        ui.set_width(width);
        ui.spacing_mut().item_spacing = vec2(0., 0.);
        let (header, _) = ui.allocate_exact_size(vec2(width, t.form_header_height + t.space_8), Sense::hover());
        let icon = if libraries { MenuIcon::Open } else { MenuIcon::Settings };
        icon.paint_sized(ui.painter(), header.left_center() + vec2(t.space_24, 0.), t.space_16, t, true);
        ui.painter().text(
            header.left_center() + vec2(2. * t.space_24, 0.),
            egui::Align2::LEFT_CENTER,
            title,
            fonts::interface_heading(ui, t.font_heading),
            t.text,
        );
        let close = Rect::from_center_size(header.right_center() - vec2(t.space_24, 0.), vec2(t.control_height, t.control_height));
        let response = ui.interact(close, ui.id().with("close"), Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Close {title}")));
        if response.hovered() {
            ui.painter().rect_filled(close, t.radius_sm, t.hover);
        }
        MenuIcon::Close.paint_sized(ui.painter(), close.center(), t.space_16, t, true);
        if libraries {
            let dock = Rect::from_min_max(close.left_top() - vec2(56., 0.), close.left_bottom() - vec2(t.space_8, 0.));
            let response = ui.put(dock, egui::Button::new("Dock").small()).on_hover_text("Show compact catalog");
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Dock Parts Catalog"));
            action.compact = response.clicked();
        }
        action.close = response.clicked();
        rule(ui, header.bottom(), header.x_range());
        let height = if libraries { 18. * t.control_height } else { 8. * t.control_height };
        let height = height.min((ctx.content_rect().height() - 2. * t.space_24 - header.height() - t.form_footer_height).max(0.));
        let (body, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        let mut child = ui.new_child(UiBuilder::new().max_rect(body));
        child.set_clip_rect(ui.clip_rect().intersect(body));
        child.spacing_mut().item_spacing = vec2(t.space_8, t.space_8);
        add(&mut child);
        let (footer, _) = ui.allocate_exact_size(vec2(width, t.form_footer_height), Sense::hover());
        ui.painter().rect_filled(footer, egui::CornerRadius { nw: 0, ne: 0, sw: t.radius_lg, se: t.radius_lg }, t.dialog_footer);
        rule(ui, footer.top(), footer.x_range());
        let inner = footer.shrink2(vec2(t.space_16, t.space_8));
        ui.scope_builder(UiBuilder::new().max_rect(inner).layout(Layout::right_to_left(Align::Center)), |ui| {
            action.primary = ui.add_enabled(enabled, button(primary, true)).clicked();
            action.close |= ui.add(button("Cancel", false)).clicked();
            if libraries {
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    action.libraries = ui.add(button("Manage libraries…", false)).clicked();
                });
            }
        });
    });
    action.close |= active && modal.should_close();
    if active { action } else { FormAction::default() }
}

/// Consistent action surfaces match the library source manager and Preferences.
fn button(label: &str, primary: bool) -> egui::Button<'_> {
    let t = Tokens::get();
    egui::Button::new(RichText::new(label).size(t.font_label).color(if primary { t.form_primary } else { t.text }))
        .fill(if primary { t.selected_bg } else { t.dialog_body })
        .stroke(Stroke::new(1., if primary { t.form_primary } else { t.border }))
        .corner_radius(t.control_radius)
        .min_size(vec2(80., t.control_height))
}
/// Draw a shared divider without introducing a second border palette.
fn rule(ui: &Ui, y: f32, x: egui::emath::Rangef) {
    ui.painter().hline(x, y, Stroke::new(1., Tokens::get().dialog_rule));
}

/// Regions of a catalog: collection/library navigation, selectable results and a large preview.
#[derive(Clone, Copy)]
pub enum Region {
    Libraries,
    Results,
    Preview,
}

/// Keep desktop catalogs in three panes. Compact hosts move library selection into the caller's
/// toolbar, retaining results beside preview or in two tabs when neither pane would remain useful.
pub fn panes(ui: &mut Ui, mut add: impl FnMut(&mut Ui, Region)) {
    let t = Tokens::get();
    let area = ui.available_rect_before_wrap();
    let library = if area.width() >= 900. { t.form_card_width - 32. } else { 0. };
    let content = Rect::from_min_max(pos2(area.left() + library, area.top()), area.max);
    if library > 0. {
        let rect = Rect::from_min_max(area.min, pos2(content.left(), area.bottom()));
        pane(ui, rect, true, |ui| add(ui, Region::Libraries));
    }
    if content.width() < 600. {
        let id = ui.id().with("catalog_pane");
        let mut tab = ui.ctx().data_mut(|d| *d.get_temp_mut_or_default::<usize>(id));
        let tabs = Rect::from_min_size(content.min, vec2(content.width(), t.form_header_height));
        ui.scope_builder(UiBuilder::new().max_rect(tabs.shrink(t.space_8)).layout(Layout::left_to_right(Align::Center)), |ui| {
            ui.selectable_value(&mut tab, 0, "Parts");
            ui.selectable_value(&mut tab, 1, "Preview");
        });
        ui.ctx().data_mut(|d| d.insert_temp(id, tab));
        let rect = Rect::from_min_max(pos2(content.left(), tabs.bottom()), content.max);
        pane(ui, rect, false, |ui| add(ui, if tab == 0 { Region::Results } else { Region::Preview }));
    } else {
        let x = content.left() + content.width() * 0.54;
        pane(ui, Rect::from_min_max(content.min, pos2(x, content.bottom())), false, |ui| add(ui, Region::Results));
        pane(ui, Rect::from_min_max(pos2(x, content.top()), content.max), true, |ui| add(ui, Region::Preview));
    }
}
/// Each pane owns its clipping/scrolling and receives standard Craft spacing.
fn pane(ui: &mut Ui, rect: Rect, tinted: bool, add: impl FnOnce(&mut Ui)) {
    let t = Tokens::get();
    if tinted {
        ui.painter().rect_filled(rect, 0, t.dialog_footer);
    }
    ui.painter().vline(rect.right(), rect.y_range(), Stroke::new(1., t.dialog_rule));
    ui.scope_builder(UiBuilder::new().max_rect(rect.shrink(t.space_16)).layout(Layout::top_down(Align::Min)), |ui| {
        ui.set_clip_rect(ui.clip_rect().intersect(rect));
        ui.spacing_mut().item_spacing = vec2(t.space_8, t.space_8);
        add(ui);
    });
}

/// A contained geometry preview uses the schematic palette, shared rounded corners and margins.
pub fn preview(ui: &mut Ui, paint: impl FnOnce(&egui::Painter, Rect)) {
    let t = Tokens::get();
    let height = 180.0_f32.min((ui.available_height() * 0.6).max(40.));
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    ui.painter().rect_filled(rect, t.radius_md, t.canvas);
    paint(&ui.painter().with_clip_rect(rect), rect.shrink(t.space_16));
}

/// A selected catalog item's heading uses the same heading family as the source manager.
pub fn title(ui: &mut Ui, text: &str) {
    let response = ui.label(RichText::new(text).font(fonts::interface_heading(ui, Tokens::get().font_intro)));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, format!("Preview {text}")));
}

/// Inset search/filter controls above the catalog panes using the source manager's spacing.
pub fn toolbar(ui: &mut Ui, rows: usize, add: impl FnOnce(&mut Ui)) {
    let t = Tokens::get();
    let height = 2. * t.space_16 + rows as f32 * t.control_height + rows.saturating_sub(1) as f32 * t.space_8;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    ui.scope_builder(UiBuilder::new().max_rect(rect.shrink(t.space_16)), |ui| {
        ui.spacing_mut().item_spacing = vec2(t.space_8, t.space_8);
        add(ui);
    });
    rule(ui, rect.bottom(), rect.x_range());
}
