//! Native library management presentation, independent of source I/O and electrical models.
use crate::{Tokens, fonts, menu};
use egui::{Align, Frame, Label, Layout, Rect, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use solidcraft_ui_kit::{self as kit, Container, Surface};

/// Present the manager on the same header, body and footer surfaces as Preferences.
/// The body has a fixed viewport; each pane scrolls without moving the closing actions.
pub fn dialog(ctx: &egui::Context, add: impl FnOnce(&mut Ui)) -> bool {
    let t = Tokens::get();
    let id = egui::Id::unique("library_sources");
    let frame = kit::navigation_popup_frame().inner_margin(0);
    let mut close = false;
    let modal = Container::new(ctx, Surface::Window).modal(id).frame(frame).show(ctx, |ui| {
        let width = (t.form_card_width * 3.4).min((ctx.content_rect().width() - t.space_16 * 2.0).max(0.0));
        ui.set_width(width);
        let field_style = ui.style().clone();
        kit::scope(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let (header, closed) = crate::catalog::form_header(ui, "Library sources");
            close = closed;
            let height =
                (ctx.content_rect().height() - t.space_24 * 2.0 - header.height() - t.form_footer_height).clamp(0.0, t.control_height * 17.0);
            let (body, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
            let mut content = ui.new_child(UiBuilder::new().id_salt("library_sources_body").max_rect(body));
            content.set_clip_rect(ui.clip_rect().intersect(body));
            content.set_style(field_style);
            add(&mut content);
            ui.ctx().input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
            kit::footer(ui, "Close", "", true, kit::Footer::default())
        })
    });
    close || modal.inner == kit::Action::Ok || modal.should_close()
}

/// Place sources beside the catalog, or in tabs when two useful columns no longer fit.
/// Each closure gets a clipped, bounded pane and may allocate its own scrolling content.
pub fn panes(ui: &mut Ui, mut add: impl FnMut(&mut Ui, usize)) {
    let t = Tokens::get();
    let body = ui.max_rect();
    let wide = body.width() >= t.form_card_width * 2.5;
    if !wide {
        let id = ui.scope_id().with("compact_source_pane");
        let mut pane = ui.ctx().data_mut(|data| *data.get_temp_mut_or_default::<usize>(id));
        let tabs = Rect::from_min_size(body.min, vec2(body.width(), t.form_header_height + t.space_8));
        ui.painter().rect_filled(tabs, 0, t.dialog_footer);
        let mut tab_ui =
            ui.new_child(UiBuilder::new().max_rect(tabs.shrink2(vec2(t.space_16, t.space_8))).layout(Layout::left_to_right(Align::Center)));
        tab_ui.spacing_mut().item_spacing.x = t.space_8;
        for (index, title) in ["Sources", "Catalog"].into_iter().enumerate() {
            if kit::button(&mut tab_ui, title, if pane == index { kit::Button::SmallSelected } else { kit::Button::Small }).clicked() {
                pane = index;
            }
        }
        ui.ctx().data_mut(|data| data.insert_temp(id, pane));
        rule(ui, tabs.bottom(), tabs.x_range());
        let content = Rect::from_min_max(pos2(body.left(), tabs.bottom()), body.max).shrink(t.space_16);
        let mut child = ui.new_child(UiBuilder::new().id_salt(pane).max_rect(content).layout(Layout::top_down(Align::Min)));
        child.set_clip_rect(ui.clip_rect().intersect(content));
        child.spacing_mut().item_spacing = vec2(t.space_6, t.space_8);
        add(&mut child, pane);
        return;
    }
    let x = body.left() + t.form_card_width;
    let sources = Rect::from_min_max(body.min, pos2(x, body.bottom()));
    let catalog = Rect::from_min_max(pos2(x, body.top()), body.max);
    ui.painter().rect_filled(sources, 0, t.dialog_footer);
    ui.painter().vline(sources.right(), sources.y_range(), Stroke::new(1.0, t.dialog_rule));
    for (index, rect) in [sources, catalog].into_iter().enumerate() {
        let inner = rect.shrink(t.space_16);
        ui.scope_builder(UiBuilder::new().id_salt(index).max_rect(inner).layout(Layout::top_down(Align::Min)), |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(inner));
            ui.spacing_mut().item_spacing = vec2(t.space_6, t.space_8);
            add(ui, index);
        });
    }
}

/// A pane title with a quiet, aligned count rather than an additional caption row.
pub fn heading(ui: &mut Ui, title: &str, count: usize) {
    let t = Tokens::get();
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).font(fonts::interface_heading(ui, t.font_intro)).color(t.text));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(count.to_string()).size(t.font_label).color(t.text_dim));
        });
    });
}

/// Full-height scroll areas use independent IDs so one pane never displaces the other.
pub fn list(ui: &mut Ui, id: &str, add: impl FnOnce(&mut Ui)) {
    scrollbar(ui);
    egui::ScrollArea::vertical().id_salt(id).max_height(ui.available_height()).auto_shrink([false, false]).show(ui, add);
}

/// Source import actions retain readable control height and can wrap on compact hosts.
pub fn imports(ui: &mut Ui, enabled: bool, desktop: bool) -> (bool, bool) {
    let mut add = false;
    let mut upload = false;
    ui.horizontal_wrapped(|ui| {
        if desktop {
            add = ui.add_enabled_ui(enabled, |ui| kit::primary_button(ui, "Add folder…")).inner.clicked();
        }
        upload = ui
            .add_enabled_ui(enabled, |ui| kit::button(ui, "Upload bundle…", if desktop { kit::Button::Normal } else { kit::Button::Primary }))
            .inner
            .clicked();
    });
    (add, upload)
}

/// Display-only source information; paths and diagnostics are never interpreted by chrome.
pub struct Source<'a> {
    pub name: &'a str,
    pub library: &'a str,
    pub license: &'a str,
    pub location: Option<&'a str>,
    pub status: Option<&'a str>,
    pub problems: &'a [&'a str],
    pub symbols: u64,
    pub footprints: u64,
    pub parts: u64,
    pub categories: u64,
    pub builtin: bool,
    pub available: bool,
    pub loading: bool,
}

/// The host carries out the selected operation after the presentation pass.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum SourceAction {
    #[default]
    None,
    Refresh,
    Relocate,
    Remove,
}

/// Render one source as a full-width card, keeping diagnostics with their owner and actions
/// in an accessible overflow menu. Long names and paths truncate with full hover text.
pub fn source(ui: &mut Ui, source: Source<'_>, can_refresh: bool) -> SourceAction {
    let t = Tokens::get();
    let mut action = SourceAction::None;
    let width = ui.available_width();
    Frame::new().fill(t.dialog_body).stroke(Stroke::new(1.0, t.border)).corner_radius(t.radius_md).inner_margin(t.space_12).show(ui, |ui| {
        ui.set_width((width - t.space_24 - 2.0).max(0.0));
        ui.spacing_mut().item_spacing.y = t.space_6;
        let (header, _) = ui.allocate_exact_size(vec2(ui.available_width(), t.space_24), Sense::hover());
        let mut title_rect = header;
        if !source.builtin {
            let menu_rect = Rect::from_min_max(pos2(header.right() - t.space_24, header.top()), header.max);
            title_rect.max.x = menu_rect.left() - t.space_4;
            let response = ui.interact(menu_rect, ui.scope_id().with(source.name), Sense::click());
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Actions for {}", source.name)));
            if response.hovered() || response.has_focus() {
                ui.painter().rect_filled(menu_rect, t.radius_sm, t.hover);
            }
            for offset in [-t.space_4, 0.0, t.space_4] {
                ui.painter().circle_filled(menu_rect.center() + vec2(offset, 0.0), 1.0, t.text_dim);
            }
            egui::Popup::menu(&response).frame(menu::frame()).show(|ui| {
                if source.location.is_some() {
                    if source_action(ui, "Refresh", source.name, can_refresh) {
                        action = SourceAction::Refresh;
                    }
                    if source_action(ui, "Relocate…", source.name, can_refresh) {
                        action = SourceAction::Relocate;
                    }
                    menu::separator(ui);
                }
                if source_action(ui, "Remove", source.name, true) {
                    action = SourceAction::Remove;
                }
            });
        }
        let mut title_ui = ui.new_child(UiBuilder::new().max_rect(title_rect).layout(Layout::left_to_right(Align::Center)));
        title_ui.add(Label::new(RichText::new(source.name).font(fonts::interface_heading(ui, t.font_button))).truncate()).on_hover_text(source.name);
        ui.label(RichText::new(format!("{} symbols · {} footprints", source.symbols, source.footprints)).size(t.font_small).color(t.text_dim));
        if source.parts != 0 || source.categories != 0 {
            ui.label(RichText::new(format!("Parts: {} · Categories: {}", source.parts, source.categories)).size(t.font_small).color(t.text_dim));
        }
        ui.horizontal(|ui| {
            let state = if source.loading {
                "Loading…"
            } else if !source.available {
                "Unavailable"
            } else if !source.problems.is_empty() {
                "Needs attention"
            } else if source.builtin {
                "Built-in"
            } else {
                "Available"
            };
            let color = if !source.loading && (!source.available || !source.problems.is_empty()) { t.error } else { t.text_dim };
            if source.loading {
                ui.add(egui::Spinner::new().size(t.space_12));
            }
            ui.label(RichText::new(state).size(t.font_small).color(color));
        });
        if let Some(status) = source.status.filter(|s| !matches!(*s, "Available" | "Loading…")) {
            ui.label(RichText::new(status).size(t.font_small).color(t.error));
        }
        for problem in source.problems {
            ui.label(RichText::new(*problem).size(t.font_small).color(t.error));
        }
        let detail = source.location.unwrap_or(source.library);
        ui.add(Label::new(RichText::new(detail).size(t.font_small).color(t.text_dim)).truncate()).on_hover_text(detail);
        if !source.license.is_empty() {
            ui.add(Label::new(RichText::new(source.license).size(t.font_small).color(t.text_dim)).truncate()).on_hover_text(source.license);
        }
    });
    action
}

/// Keep menu labels short while exposing their source-qualified names to assistive technology.
fn source_action(ui: &mut Ui, label: &str, name: &str, enabled: bool) -> bool {
    let response = menu::MenuItem::new(label).enabled(enabled).show_response(ui);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, format!("{label} {name}")));
    response.clicked()
}

/// Full-width search with the same inset glyph and field surface as settings search.
pub fn search(ui: &mut Ui, query: &mut String) {
    kit::search_field(ui, ui.make_persistent_id("library_search"), query, "Search library items…").accessible_name("Search libraries");
}

/// View-only catalog categories, mapped to domain kinds by the host.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    Symbols,
    Footprints,
    Parts,
    Categories,
    ManufacturerParts,
}

/// A compact segmented filter, with the selection exposed to keyboard and accessibility users.
pub fn filters(ui: &mut Ui, kind: &mut Filter) {
    let t = Tokens::get();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = t.space_4;
        for (value, label) in [
            (Filter::All, "All items"),
            (Filter::Symbols, "Symbols"),
            (Filter::Footprints, "Footprints"),
            (Filter::Parts, "Parts"),
            (Filter::Categories, "Categories"),
            (Filter::ManufacturerParts, "Manufacturer parts"),
        ] {
            let selected = *kind == value;
            let response = kit::button(ui, label, if selected { kit::Button::SmallSelected } else { kit::Button::Small });
            response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, format!("Show {}", label.to_lowercase())));
            if response.clicked() {
                *kind = value;
            }
        }
    });
}

/// The catalog needs only item identity and attribution, without depending on engine models.
pub struct Item<'a> {
    pub name: &'a str,
    pub library: &'a str,
    pub id: &'a str,
    pub revision: u64,
    pub footprint: bool,
    pub kind: &'a str,
    pub problem: Option<&'a str>,
    pub license: &'a str,
}

/// Virtualize large catalogs while retaining small catalogs' complete accessibility targets.
/// Each row separates its name from identity, kind and revision, with full attribution on hover.
pub fn results(ui: &mut Ui, items: &[Item<'_>]) {
    let t = Tokens::get();
    if items.is_empty() {
        ui.add_space(t.space_24);
        ui.label(RichText::new("No matching items").font(fonts::interface_heading(ui, t.font_heading)));
        return;
    }
    let height = t.space_24 * 2.0 + t.space_12;
    let scroll = egui::ScrollArea::vertical().id_salt("source_results").auto_shrink([false, false]);
    scrollbar(ui);
    ui.spacing_mut().item_spacing.y = 0.0;
    if items.len() > 32 {
        scroll.show_rows(ui, height, items.len(), |ui, rows| {
            for index in rows {
                item(ui, &items[index], height);
            }
        });
    } else {
        scroll.show(ui, |ui| {
            for entry in items {
                item(ui, entry, height);
            }
        });
    }
}

/// Paint a two-line catalog row and its original symbol/package glyph inside a quiet tile.
fn item(ui: &mut Ui, item: &Item<'_>, height: f32) {
    let t = Tokens::get();
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let tile = Rect::from_min_size(rect.min + vec2(0.0, t.space_8), vec2(t.space_24 + t.space_12, t.space_24 + t.space_12));
    ui.painter().rect_filled(tile, t.radius_md, t.dialog_footer);
    glyph(ui, tile, item.footprint);
    let text = Rect::from_min_max(pos2(tile.right() + t.space_12, rect.top() + t.space_8), pos2(rect.right(), rect.bottom() - t.space_8));
    let mut text_ui = ui.new_child(UiBuilder::new().max_rect(text).layout(Layout::top_down(Align::Min)));
    text_ui.spacing_mut().item_spacing.y = t.space_4;
    text_ui.add(Label::new(RichText::new(item.name).font(fonts::interface_heading(ui, t.font_button)).color(t.text)).truncate());
    let kind = if item.problem.is_some() { format!("{} · Unresolved", item.kind) } else { item.kind.to_owned() };
    text_ui.add(
        Label::new(RichText::new(format!("{kind}  ·  {}/{}  ·  r{}", item.library, item.id, item.revision)).size(t.font_small).color(t.text_dim))
            .truncate(),
    );
    response.on_hover_text(format!(
        "{}\n{}/{} r{}\n{}{}",
        item.name,
        item.library,
        item.id,
        item.revision,
        item.license,
        item.problem.map(|problem| format!("\n{problem}")).unwrap_or_default()
    ));
    rule(ui, rect.bottom(), rect.x_range());
}

/// Original schematic and package silhouettes share the current palette's icon roles.
fn glyph(ui: &Ui, rect: Rect, footprint: bool) {
    let t = Tokens::get();
    let painter = ui.painter();
    let center = rect.center();
    let body = Rect::from_center_size(center, vec2(t.space_12, t.space_16));
    painter.rect_stroke(body, t.space_2, Stroke::new(1.3, t.catalog_symbol), StrokeKind::Inside);
    if footprint {
        for x in [-1.0, 1.0] {
            for y in [-1.0, 1.0] {
                painter.rect_filled(
                    Rect::from_center_size(center + vec2(x * t.space_8, y * t.space_4), vec2(t.space_6, t.space_4)),
                    1,
                    t.catalog_symbol,
                );
            }
        }
    } else {
        for direction in [-1.0, 1.0] {
            painter.line_segment(
                [center + vec2(0.0, direction * t.space_8), center + vec2(0.0, direction * t.space_12)],
                Stroke::new(1.3, t.catalog_symbol),
            );
        }
    }
}

/// Shared low-contrast rules align dialog bands, panes and catalog rows.
fn rule(ui: &Ui, y: f32, x: egui::Rangef) {
    ui.painter().hline(x, y, Stroke::new(1.0, Tokens::get().dialog_rule));
}

/// Reserve a narrow, visible scrollbar so clipped rows have a discoverable scroll affordance.
fn scrollbar(ui: &mut Ui) {
    let t = Tokens::get();
    ui.spacing_mut().scroll.floating = false;
    ui.spacing_mut().scroll.bar_width = t.space_6;
    ui.visuals_mut().widgets.inactive.fg_stroke.color = t.text_dim;
}
