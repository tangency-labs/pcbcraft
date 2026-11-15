//! Craft catalog and property forms share the source manager's header, body and footer rhythm.
use crate::{Tokens, fonts};
use egui::{Align, Layout, Rect, RichText, Sense, Stroke, Ui, UiBuilder, pos2, vec2};
use solidcraft_ui_kit::{self as kit, Container, Surface, Window};

/// A shared explicit placement choice keeps compact and expanded catalogs in agreement.
pub fn repeat_placement(ui: &mut Ui, repeat: &mut bool) -> egui::Response {
    kit::checkbox(ui, "Repeat placement", repeat)
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

/// Open the expanded Parts Catalog at 70% of the viewport with a remembered resize corner. Dock switches
/// presentation; Manage libraries remains available independently of placement validity.
/// Inactive hosts paint disabled controls and cannot dispatch actions.
pub fn parts_form(ctx: &egui::Context, enabled: bool, active: bool, repeat: &mut bool, add: impl FnOnce(&mut Ui)) -> FormAction {
    let t = Tokens::get();
    let id = egui::Id::unique("parts_catalog");
    let frame = kit::navigation_popup_frame().inner_margin(0);
    let mut closed = false;
    let mut compact = false;
    let mut libraries = false;
    let modal = Container::new(ctx, Surface::Window).modal(id).frame(frame).show(ctx, |ui| {
        if !active {
            ui.disable();
        }
        let maximum = (ctx.content_rect().size() - vec2(t.space_16, t.space_16) * 2.0).max(egui::Vec2::ZERO);
        let field_style = ui.style().clone();
        // ModalHost supplies the qualified surface and input boundary but has no sizing API.
        // Compose egui's persisted resize region inside it, keeping all form chrome shared.
        egui::Resize::default()
            .id(id.with("size"))
            .default_size(ctx.content_rect().size() * 0.7)
            .min_size(vec2(600.0, 400.0).min(maximum))
            .max_size(maximum)
            .with_stroke(false)
            .resizable(active)
            .show(ui, |ui| {
                kit::scope(ui, |ui| {
                    let size = ui.available_size();
                    let width = size.x;
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let (header, close) = form_header(ui, "Parts Catalog");
                    closed = close;
                    let dock = Rect::from_center_size(
                        header.right_center() - vec2(t.space_24 + t.control_height / 2.0 + t.space_8 + t.space_24, 0.0),
                        vec2(t.space_24 * 2.0, t.control_height),
                    );
                    compact = kit::button_at(ui, "dock_parts_catalog", dock, "Dock", kit::Button::Normal)
                        .accessible_name("Dock Parts Catalog")
                        .on_hover_text("Show compact catalog")
                        .clicked();
                    let footer_height = t.form_footer_height;
                    let height = (size.y - header.height() - footer_height).max(0.0);
                    let (body, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
                    let mut content = ui.new_child(UiBuilder::new().id_salt("parts_catalog_body").max_rect(body));
                    content.set_clip_rect(ui.clip_rect().intersect(body));
                    content.set_style(field_style);
                    content.spacing_mut().item_spacing = vec2(t.space_8, t.space_8);
                    add(&mut content);
                    // The catalog owns Enter confirmation, including its search and popup focus rules.
                    ui.ctx().input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
                    let footer = Rect::from_min_size(ui.cursor().min, vec2(width, footer_height));
                    let action = kit::footer(ui, "Place", "Cancel", enabled, kit::Footer::default());
                    // The standalone footer has no public leading-content slot. Keep its
                    // surface/actions and compose shared controls in the unused leading band.
                    let place_width =
                        ui.painter().layout_no_wrap("Place".into(), fonts::interface_heading(ui, t.font_label), t.text).size().x + 2. * t.space_16;
                    let cancel_width =
                        ui.painter().layout_no_wrap("Cancel".into(), egui::FontId::proportional(t.font_label), t.text).size().x + 2. * t.space_12;
                    let mut left = footer.shrink2(vec2(t.space_12, t.space_8 + t.space_2));
                    left.max.x -= (place_width.max(t.space_24 * 3.) + cancel_width + t.space_6 + t.space_8).min(left.width());
                    ui.scope_builder(
                        UiBuilder::new().id_salt("catalog_footer_options").max_rect(left).layout(Layout::left_to_right(Align::Center)),
                        |ui| {
                            libraries = crate::controls::button(ui, "Manage libraries…").clicked();
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                repeat_placement(ui, repeat);
                            });
                        },
                    );
                    action
                })
            })
    });
    let cancelled = modal.inner == kit::Action::Cancel && !ctx.input(|input| input.key_pressed(egui::Key::Escape));
    if active {
        FormAction {
            close: closed || cancelled || modal.should_close(),
            primary: modal.inner == kit::Action::Ok,
            libraries,
            compact,
            ..Default::default()
        }
    } else {
        FormAction::default()
    }
}

/// Keep the shared property actions visible while fields scroll. The caller handles field
/// submission and validation; Escape and backdrop clicks dismiss the dialog.
pub fn property_form(ctx: &egui::Context, id: &'static str, title: &str, width: f32, primary: &str, add: impl FnOnce(&mut Ui)) -> FormAction {
    property_form_enabled(ctx, id, title, width, primary, true, add)
}

/// The shared property form can disable its fixed primary action while retaining editable fields
/// and cancellation. Validation stays in the host; no separate modal/footer presentation is drawn.
pub fn property_form_enabled(
    ctx: &egui::Context,
    id: &'static str,
    title: &str,
    width: f32,
    primary: &str,
    enabled: bool,
    add: impl FnOnce(&mut Ui),
) -> FormAction {
    let t = Tokens::get();
    let id = egui::Id::unique(id);
    let window = Window::new(id, "settings", title).fields().actions(primary, "Cancel", None);
    let frame = kit::navigation_popup_frame().inner_margin(0);
    let modal = Container::new(ctx, Surface::Window).modal(id).frame(frame).show(ctx, |ui| {
        ui.set_width(width.min((ctx.content_rect().width() - 2.0 * t.space_16).max(0.0)));
        let field_style = ui.style().clone();
        window.content(
            ui,
            enabled,
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

/// Keep footprint browsing bounded by shared modal chrome. Assignment and placement have
/// independent enabled states, with optional placement in the footer's left action slot.
pub fn footprint_form(ctx: &egui::Context, enabled: bool, place: Option<bool>, add: impl FnOnce(&mut Ui)) -> FormAction {
    let t = Tokens::get();
    let id = egui::Id::unique("footprint_assignment");
    let frame = kit::navigation_popup_frame().inner_margin(0);
    let mut closed = false;
    let modal = Container::new(ctx, Surface::Window).modal(id).frame(frame).show(ctx, |ui| {
        let width = 1160.0_f32.min((ctx.content_rect().width() - 2.0 * t.space_16).max(0.0));
        ui.set_width(width);
        let field_style = ui.style().clone();
        kit::scope(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let (header, close) = form_header(ui, "Assign Footprint");
            closed = close;
            let footer_height = t.form_footer_height + t.space_4;
            let height = (ctx.content_rect().height() - header.height() - footer_height - t.space_24 * 2.0).clamp(0.0, t.control_height * 24.0);
            let (body, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
            let mut content = ui.new_child(UiBuilder::new().id_salt("assignment_body").max_rect(body));
            content.set_clip_rect(ui.clip_rect().intersect(body));
            content.set_style(field_style);
            add(&mut content);
            ui.ctx().input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
            kit::footer(
                ui,
                "Assign",
                "Close",
                enabled,
                kit::Footer {
                    apply: place.map(|_| "Place on board"),
                    nothing_to_apply: place != Some(true),
                    confirm: true,
                    secondary_left: true,
                    ..Default::default()
                },
            )
        })
    });
    let cancelled = modal.inner == kit::Action::Cancel && !ctx.input(|input| input.key_pressed(egui::Key::Escape));
    FormAction {
        close: closed || cancelled || modal.should_close(),
        primary: modal.inner == kit::Action::Ok,
        secondary: modal.inner == kit::Action::Apply,
        ..Default::default()
    }
}

/// Draw shared title text and a title-specific close control, keeping the header target
/// distinct from a form's Close footer action. Return the allocated band and dismissal request.
pub(crate) fn form_header(ui: &mut Ui, title: &str) -> (Rect, bool) {
    let t = Tokens::get();
    let (header, _) = ui.allocate_exact_size(vec2(ui.available_width(), kit::header_height()), Sense::hover());
    let close = Rect::from_center_size(header.right_center() - vec2(t.space_24, 0.0), vec2(t.control_height, t.control_height));
    let close_response = kit::icon_button_at(ui, close, "close", &format!("Close {title}"));
    crate::automation::publish(ui, &close_response, format!("button:Close {title}"));
    let closed = close_response.clicked();
    let label = Rect::from_min_max(header.min + vec2(t.space_24, 0.0), pos2(close.left() - t.space_8, header.bottom()));
    kit::paint_text(ui, label, title, fonts::interface_heading(ui, t.font_button), t.text, false);
    ui.interact(label, ui.make_persistent_id("form_title"), Sense::hover())
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, ui.is_enabled(), title));
    rule(ui, header.bottom(), header.x_range());
    (header, closed)
}

/// Keep shared window actions fixed around a bounded page editor. Navigation, validation
/// and page scrolling fit inside the body while the caller controls primary action availability.
pub fn settings_form(ctx: &egui::Context, title: &str, primary: &str, enabled: bool, add: impl FnOnce(&mut Ui)) -> FormAction {
    settings_form_width(ctx, title, Tokens::get().form_card_width * 4., primary, enabled, add)
}

/// Use shared settings chrome with a content-appropriate maximum width; the footer and
/// resize-settling behavior remain identical to the ordinary board setup form.
pub fn settings_form_width(ctx: &egui::Context, title: &str, width: f32, primary: &str, enabled: bool, add: impl FnOnce(&mut Ui)) -> FormAction {
    let t = Tokens::get();
    let id = egui::Id::unique("board_setup_form");
    // The backing Area positions using its previous size. Schedule one settling frame when
    // viewport bounds or requested form width change, keeping transitions between forms centered.
    let width = width.max(0.);
    let metrics = (ctx.content_rect(), ctx.pixels_per_point(), width);
    let key = id.with(("layout_metrics", ctx.viewport_id()));
    let changed = ctx.data_mut(|data| {
        let changed = data.get_temp::<(Rect, f32, f32)>(key) != Some(metrics);
        data.insert_temp(key, metrics);
        changed
    });
    if changed {
        ctx.request_repaint();
    }
    let frame = kit::navigation_popup_frame().inner_margin(0);
    let mut closed = false;
    let modal = Container::new(ctx, Surface::Window).modal(id).frame(frame).show(ctx, |ui| {
        let width = width.max(0.).min((ctx.content_rect().width() - 2.0 * t.space_16).max(0.0));
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

/// Consistent action surfaces match the library source manager and Preferences.
pub fn button(label: &str, primary: bool) -> impl egui::Widget + '_ {
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
pub fn panes(ui: &mut Ui, preview: &mut crate::library_workspace::ColumnWidth, mut add: impl FnMut(&mut Ui, Region)) {
    let t = Tokens::get();
    let area = ui.available_rect_before_wrap();
    let library = if area.width() >= 900. { 176. } else { 0. };
    let content = Rect::from_min_max(pos2(area.left() + library, area.top()), area.max);
    if library > 0. {
        let rect = Rect::from_min_max(area.min, pos2(content.left(), area.bottom()));
        pane(ui, rect, true, |ui| add(ui, Region::Libraries));
    }
    if content.width() < 560. {
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
        let width = preview.width(280.).clamp(240., content.width() - 320.);
        let edge = content.right() - width;
        let width = preview.resize(
            ui,
            ui.scope_id().with("catalog_preview_width"),
            280.,
            crate::library_workspace::Sash {
                bounds: Rect::from_min_max(pos2(edge - t.space_4, content.top()), pos2(edge + t.space_4, content.bottom())),
                name: "Resize catalog preview",
                min: 240.,
                max: content.width() - 320.,
                leading: true,
            },
        );
        let x = content.right() - width;
        pane(ui, Rect::from_min_max(content.min, pos2(x - t.space_4, content.bottom())), false, |ui| add(ui, Region::Results));
        pane(ui, Rect::from_min_max(pos2(x + t.space_4, content.top()), content.max), true, |ui| add(ui, Region::Preview));
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

/// The expanded chooser uses the toolkit table with visible-row virtualization and fixed headers.
/// Compact dock density remains independent, and explicit selection reveal scrolls only once.
pub fn part_list<T>(ui: &mut Ui, items: &[T], reveal: Option<usize>, mut add: impl FnMut(&mut Ui, &mut kit::Table<'_>, &T)) {
    let columns = [kit::Column::new("Part", 3.).flex(), kit::Column::new("Default value", 1.5).flex(), kit::Column::new("Library source", 2.).flex()];
    kit::table_custom(ui, "catalog_parts", &columns, "No matching parts", |ui, table| {
        table.headers(ui, |_, _| false);
        let mut scroll = egui::ScrollArea::vertical().id_salt("catalog_part_rows").auto_shrink([false, false]);
        ui.spacing_mut().scroll.floating = false;
        if let Some(index) = reveal.filter(|i| *i < items.len()) {
            scroll = scroll.vertical_scroll_offset(index as f32 * kit::table_row_height());
        }
        if items.len() <= 64 {
            scroll.show(ui, |ui| {
                for item in items {
                    add(ui, table, item);
                }
            });
        } else {
            scroll.show_rows(ui, kit::table_row_height(), items.len(), |ui, rows| {
                for item in &items[rows] {
                    add(ui, table, item);
                }
            });
        }
    });
}

/// Full-width selectable table rows keep drag, focus and double-click authority with the caller.
pub fn part_row(
    ui: &mut Ui,
    table: &mut kit::Table<'_>,
    name: &str,
    value: &str,
    library: &str,
    identity: Option<&str>,
    selected: bool,
) -> egui::Response {
    let accessible = identity.map_or_else(|| name.to_owned(), |identity| format!("{name} · {identity}"));
    table
        .row(ui, &accessible, selected, |row| {
            for (text, primary) in [(name, true), (value, false), (library, false)] {
                row.cell(|ui| {
                    let t = Tokens::get();
                    kit::paint_text(
                        ui,
                        ui.max_rect(),
                        text,
                        egui::FontId::proportional(t.font_body),
                        if primary { t.text } else { t.text_dim },
                        false,
                    );
                });
            }
        })
        .interact(Sense::click_and_drag())
        .on_hover_text(format!("{name}\n{value} · {library}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Resizing a visible settings modal requests a settling paint even without pointer input.
    #[test]
    fn settings_resize_requests_layout_settling() {
        let ctx = egui::Context::default();
        crate::install(&ctx);
        let frame = |size| {
            let mut output =
                ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)), ..Default::default() }, |ui| {
                    settings_form(ui.ctx(), "Rules", "Save class", false, |ui| {
                        ui.label("Class name");
                    });
                });
            output.textures_delta.clear();
            output
        };
        for _ in 0..4 {
            let _ = frame(vec2(1440., 1000.));
        }
        let output = frame(vec2(760., 640.));
        assert_eq!(output.viewport_output[&egui::ViewportId::ROOT].repaint_delay, std::time::Duration::ZERO);
        let _ = frame(vec2(760., 640.));
        let rect = egui::AreaState::load(&ctx, egui::Id::unique("board_setup_form")).unwrap().rect();
        assert!(rect.left() >= 0. && rect.right() <= 760., "{rect:?}");
        assert!((rect.center().x - 380.).abs() < 2., "{rect:?}");
    }

    /// Twenty thousand results still prepare only visible, single-line table rows.
    #[test]
    fn expanded_list_virtualizes_dense_rows() {
        let ctx = egui::Context::default();
        let items: Vec<_> = (0..20_000).collect();
        let mut count = 0;
        let output =
            ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(700., 600.))), ..Default::default() }, |ui| {
                part_list(ui, &items, None, |ui, table, n| {
                    count += 1;
                    let _ = part_row(ui, table, &format!("Part {n}"), "10k", "demo", None, false);
                });
            });
        output.drop_without_applying_deltas();
        assert!(count > 10 && count < 40, "painted {count} rows");
    }
}
