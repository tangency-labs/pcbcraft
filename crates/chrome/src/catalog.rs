//! Craft catalog and property forms share the source manager's header, body and footer rhythm.
use crate::{Tokens, fonts, menu::MenuIcon};
use egui::{Align, Frame, Layout, Rect, RichText, Sense, Stroke, Ui, UiBuilder, pos2, vec2};
use solidcraft_ui_kit::{self as kit, Container, Surface, Window};

/// A shared explicit placement choice keeps compact and expanded catalogs in agreement.
pub fn repeat_placement(ui: &mut Ui, repeat: &mut bool) -> egui::Response {
    ui.checkbox(repeat, "Repeat placement")
}

/// Decisions from a form remain presentation events; callers own documents and validation.
#[derive(Default)]
pub struct FormAction {
    pub close: bool,
    pub primary: bool,
    /// A separate host action, such as placing the saved footprint assignment.
    pub secondary: bool,
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
    form_body(ctx, id, title, width, primary, enabled, libraries, active, if libraries { 18. } else { 8. }, None, "Cancel", add)
}

/// Keep the shared property actions visible while fields scroll. The caller handles field
/// submission and validation; Escape and backdrop clicks dismiss the dialog.
pub fn property_form(ctx: &egui::Context, id: &'static str, title: &str, width: f32, primary: &str, add: impl FnOnce(&mut Ui)) -> FormAction {
    let t = Tokens::get();
    let id = egui::Id::unique(id);
    let window = Window::new(id, "settings", title).fields().actions(primary, "Cancel", None);
    let frame = kit::navigation_popup_frame().inner_margin(0);
    let modal = Container::new(ctx, Surface::Window).modal(id).frame(frame).show(ctx, |ui| {
        ui.set_width(width.min((ctx.content_rect().width() - 2.0 * t.space_16).max(0.0)));
        let field_style = ui.style().clone();
        window.content(
            ui,
            true,
            |ui| {
                ui.set_style(field_style);
                add(ui);
            },
            |_| {},
        )
    });
    let (closed, action) = modal.inner;
    // Let the modal route Escape so an open dropdown dismisses before the draft does.
    let cancelled = action == kit::Action::Cancel && !ctx.input(|input| input.key_pressed(egui::Key::Escape));
    FormAction { close: closed || cancelled || modal.should_close(), primary: action == kit::Action::Ok, ..Default::default() }
}

/// The footprint browser shares the property shell and keeps assignment/placement actions fixed.
pub fn footprint_form(ctx: &egui::Context, enabled: bool, place: Option<bool>, add: impl FnOnce(&mut Ui)) -> FormAction {
    form_body(
        ctx,
        "footprint_assignment",
        "Assign Footprint",
        1160.,
        "Assign",
        enabled,
        false,
        true,
        24.,
        place.map(|enabled| ("Place on board", enabled)),
        "Close",
        add,
    )
}

/// Keep shared window actions fixed around a bounded page editor. Navigation, validation
/// and page scrolling fit inside the body while the caller controls primary action availability.
pub fn settings_form(ctx: &egui::Context, title: &str, primary: &str, enabled: bool, add: impl FnOnce(&mut Ui)) -> FormAction {
    let t = Tokens::get();
    let id = egui::Id::unique("board_setup_form");
    let frame = kit::navigation_popup_frame().inner_margin(0);
    let mut closed = false;
    let modal = Container::new(ctx, Surface::Window).modal(id).frame(frame).show(ctx, |ui| {
        let width = (t.form_card_width * 4.0).min((ctx.content_rect().width() - 2.0 * t.space_16).max(0.0));
        ui.set_width(width);
        let field_style = ui.style().clone();
        kit::scope(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            kit::panel::header(ui, "settings", title, || closed = true);
            let height =
                (ctx.content_rect().height() - kit::header_height() - t.form_footer_height - t.space_24 * 2.0).clamp(0.0, t.control_height * 20.0);
            let (body, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
            let mut content = ui.new_child(UiBuilder::new().id_salt("setup_body").max_rect(body));
            content.set_clip_rect(ui.clip_rect().intersect(body));
            content.set_style(field_style);
            add(&mut content);
            ui.ctx().input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
            kit::footer(ui, primary, "Cancel", enabled, kit::Footer::default())
        })
    });
    let cancelled = modal.inner == kit::Action::Cancel && !ctx.input(|input| input.key_pressed(egui::Key::Escape));
    FormAction { close: closed || cancelled || modal.should_close(), primary: modal.inner == kit::Action::Ok, ..Default::default() }
}

/// Render the common shell with a bounded body, preserving modal input and dismissal behavior.
#[allow(clippy::too_many_arguments)]
pub(crate) fn form_body(
    ctx: &egui::Context,
    id: &'static str,
    title: &str,
    width: f32,
    primary: &str,
    enabled: bool,
    libraries: bool,
    active: bool,
    rows: f32,
    secondary: Option<(&str, bool)>,
    dismiss: &str,
    add: impl FnOnce(&mut Ui),
) -> FormAction {
    let t = Tokens::get();
    let mut action = FormAction::default();
    let frame = Frame::new().fill(t.dialog_body).stroke(Stroke::new(1.0, t.border)).corner_radius(if libraries { t.radius_lg } else { t.radius_md });
    let modal = egui::Modal::new(egui::Id::unique(id)).frame(frame).show(ctx, |ui| {
        if !active {
            ui.disable();
        }
        let width = width.min((ctx.content_rect().width() - 2. * t.space_16).max(0.));
        ui.set_width(width);
        ui.spacing_mut().item_spacing = vec2(0., 0.);
        let (header, header_response) =
            ui.allocate_exact_size(vec2(width, t.form_header_height + if libraries { t.space_8 } else { 0. }), Sense::hover());
        header_response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, title));
        if libraries {
            MenuIcon::Open.paint_sized(ui.painter(), header.left_center() + vec2(t.space_24, 0.), t.space_16, t, true);
        }
        ui.painter().text(
            header.left_center() + vec2(if libraries { 2. * t.space_24 } else { t.space_24 }, 0.),
            egui::Align2::LEFT_CENTER,
            title,
            fonts::interface_heading(ui, if libraries { t.font_intro } else { t.font_button }),
            t.text,
        );
        let close = Rect::from_center_size(header.right_center() - vec2(t.space_24, 0.), vec2(t.control_height, t.control_height));
        let response = ui.interact(close, ui.scope_id().with("close"), Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Close {title}")));
        if response.hovered() {
            ui.painter().rect_filled(close, t.radius_sm, t.hover);
        }
        MenuIcon::Close.paint_sized(ui.painter(), close.center(), t.space_16, t, true);
        if libraries {
            let dock = Rect::from_min_max(close.left_top() - vec2(56., 0.), close.left_bottom() - vec2(t.space_8, 0.));
            let response = ui.put(dock, egui::Button::new("Dock").small()).on_hover_text("Show compact catalog");
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Dock Parts Catalog"));
            action.compact = response.clicked();
        }
        action.close = response.clicked();
        rule(ui, header.bottom(), header.x_range());
        let height = rows * t.control_height;
        let height = height.min((ctx.content_rect().height() - 2. * t.space_24 - header.height() - t.form_footer_height).max(0.));
        let (body, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        let mut child = ui.new_child(UiBuilder::new().max_rect(body));
        child.set_clip_rect(ui.clip_rect().intersect(body));
        child.spacing_mut().item_spacing = vec2(t.space_8, t.space_8);
        add(&mut child);
        let (footer, _) = ui.allocate_exact_size(vec2(width, t.form_footer_height), Sense::hover());
        ui.painter().rect_filled(
            footer,
            egui::CornerRadius {
                nw: 0,
                ne: 0,
                sw: if libraries { t.radius_lg } else { t.radius_md },
                se: if libraries { t.radius_lg } else { t.radius_md },
            },
            t.dialog_footer,
        );
        rule(ui, footer.top(), footer.x_range());
        let inner = footer.shrink2(vec2(t.space_16, t.space_8));
        ui.scope_builder(UiBuilder::new().max_rect(inner).layout(Layout::right_to_left(Align::Center)), |ui| {
            ui.spacing_mut().item_spacing.x = t.space_8;
            action.primary = ui.add_enabled(enabled, button(primary, true)).clicked();
            if let Some((label, enabled)) = secondary {
                action.secondary = ui.add_enabled(enabled, button(label, false)).clicked();
            }
            action.close |= ui.add(button(dismiss, false)).clicked();
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
pub fn button(label: &str, primary: bool) -> egui::Button<'_> {
    crate::settings::action_button(label, primary)
}
/// Draw a shared divider without introducing a second border palette.
fn rule(ui: &Ui, y: f32, x: egui::emath::Rangef) {
    ui.painter().hline(x, y, Stroke::new(1., Tokens::get().dialog_rule));
}

/// Regions of a catalog: collection/library navigation, selectable results and a large preview.
#[derive(Clone, Copy, PartialEq, Eq)]
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
        let id = ui.scope_id().with("catalog_pane");
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
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, format!("Preview {text}")));
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

/// Responsive footprint browsing keeps three independent panes on desktop and stacks them in
/// a scrollable body on narrow hosts, with the same selection state in either layout.
pub fn footprint_panes(ui: &mut Ui, mut add: impl FnMut(&mut Ui, Region)) {
    let t = Tokens::get();
    if ui.available_width() >= 900. {
        let rect = ui.available_rect_before_wrap();
        let library = 176.;
        let results = (rect.width() * 0.34).clamp(280., 380.);
        let libraries = Rect::from_min_max(rect.min, pos2(rect.left() + library, rect.bottom()));
        let list = Rect::from_min_max(pos2(libraries.right(), rect.top()), pos2(libraries.right() + results, rect.bottom()));
        let preview = Rect::from_min_max(pos2(list.right(), rect.top()), rect.max);
        for (rect, region) in [(libraries, Region::Libraries), (list, Region::Results), (preview, Region::Preview)] {
            let mut child = ui.new_child(
                UiBuilder::new()
                    .id_salt(match region {
                        Region::Libraries => "footprint_libraries",
                        Region::Results => "footprint_results",
                        Region::Preview => "footprint_preview",
                    })
                    .max_rect(rect.shrink(t.space_12)),
            );
            child.set_clip_rect(ui.clip_rect().intersect(rect));
            add(&mut child, region);
            if region != Region::Preview {
                ui.painter().vline(rect.right(), rect.y_range(), Stroke::new(1., t.dialog_rule));
            }
        }
        ui.advance_cursor_after_rect(rect);
    } else {
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Frame::new().inner_margin(t.space_12).show(ui, |ui| {
                add(ui, Region::Libraries);
                ui.add_space(t.space_8);
                add(ui, Region::Results);
                ui.add_space(t.space_12);
                add(ui, Region::Preview);
            });
        });
    }
}

/// A footprint result uses distinct name and library/pad metadata while keeping one accessible
/// selection identity, independent of its visual line wrapping.
pub fn footprint_result(ui: &mut Ui, label: &str, name: &str, detail: &str, selected: bool) -> egui::Response {
    let t = Tokens::get();
    let mut text = egui::text::LayoutJob::default();
    text.append(
        &format!("{name}\n"),
        0.,
        egui::TextFormat { font_id: egui::FontId::proportional(t.font_label), color: t.text, ..Default::default() },
    );
    text.append(detail, 0., egui::TextFormat { font_id: egui::FontId::proportional(t.font_label - 1.), color: t.text_dim, ..Default::default() });
    let response = ui.add_sized(vec2(ui.available_width(), t.control_height * 2.), egui::Button::selectable(selected, text).right_text(()).wrap());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), selected, label));
    response
}
