//! Preferences pages and form rows inside a shared Craft modal.
//!
//! The caller owns the settings and what Cancel, Apply and the page defaults mean; this module
//! draws the window, reports which footer action was chosen, and filters rows while searching.
//! A row is found by being drawn: nothing registers for search.

use egui::{Align, FontId, Id, Key, Label, Layout, Rect, Response, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use solidcraft_ui_kit::{self as kit, Container, Surface, Window};

use crate::{Tokens, fonts, menu::MenuIcon};

/// Below this window width the sidebar narrows, as in SolidCraft.
const NARROW_AT: f32 = 700.0;
const NARROW_SIDEBAR: f32 = 150.0;
/// Space between a sidebar icon and its label.
const NAV_ROW_GAP: f32 = 9.0;
/// How long an opened search result stays outlined, in seconds.
const REVEAL_SECONDS: f64 = 0.9;
const SEARCH_STATE: &str = "settings_search";

/// One page: its sidebar glyph and label, and the function that draws its rows.
pub struct Page<S> {
    pub icon: MenuIcon,
    pub label: &'static str,
    pub draw: fn(&mut Ui, &mut S),
}

/// The footer button chosen this frame. Closing the window (×, Esc) reports `Cancel`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Action {
    #[default]
    None,
    Ok,
    Apply,
    Cancel,
    Defaults,
}

/// What a paged window keeps between frames: the open page and the search text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WindowState {
    pub page: usize,
    pub query: String,
}

/// The footer's state: whether Apply has anything to keep and whether the open page has
/// defaults to restore.
#[derive(Clone, Copy, Debug)]
pub struct Footer {
    pub apply_enabled: bool,
    pub defaults_enabled: bool,
}

/// One row found by the last search pass, for the keyboard cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Hit {
    page: usize,
    label: String,
}

/// Search state shared by the rows a page draws. Stored in egui memory for the window's frame.
#[derive(Clone, Debug, Default)]
struct Search {
    /// The folded query, or empty when not searching.
    query: String,
    page: usize,
    page_hit: bool,
    section_hit: bool,
    /// Rows shown in this pass, counted for probes.
    hits: u32,
    /// Nested invisible passes that only count hits.
    probing: u32,
    /// Rows drawn for real this frame, in order, and the previous frame's list.
    found: Vec<Hit>,
    last_found: Vec<Hit>,
    /// The keyboard cursor into `last_found`.
    cursor: usize,
    /// The row to outline after a result was opened, and until when.
    reveal: Option<(Hit, f64)>,
}

/// Where the shared search state lives in egui memory.
fn search_id() -> Id {
    Id::unique(SEARCH_STATE)
}

/// Run `f` on the shared search state. `f` runs under egui's memory lock, so it must not read
/// input or memory through the context.
fn with_search<R>(ui: &Ui, f: impl FnOnce(&mut Search) -> R) -> R {
    ui.ctx().data_mut(|data| f(data.get_temp_mut_or_default::<Search>(search_id())))
}

/// Show the window and return the footer action. Esc first clears a search, then cancels.
/// `pages` must not be empty; `state.page` is clamped to them.
pub fn paged_window<S>(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    state: &mut WindowState,
    pages: &[Page<S>],
    data: &mut S,
    footer: Footer,
) -> Action {
    let t = Tokens::get();
    if pages.is_empty() {
        return Action::Cancel;
    }
    state.page = state.page.min(pages.len() - 1);
    let id = Id::unique(id);
    let frame = kit::navigation_popup_frame().inner_margin(0);
    let window = Window::new(id, "settings", title).fields().actions("OK", "Cancel", Some("Apply")).apply_enabled(footer.apply_enabled);
    let mut defaults = false;
    let modal = Container::new(ctx, Surface::Window).modal(id).frame(frame).show(ctx, |ui| {
        let width = (t.form_card_width * 10.0 / 3.0).min(ui.ctx().content_rect().width() - 2.0 * t.space_16).max(0.0);
        ui.set_width(width);
        let page_style = ui.style().clone();
        window.content(
            ui,
            true,
            |ui| {
                // Page controls still use the host's field styling, including slider tracks.
                ui.set_style(page_style);
                body(ui, id, state, pages, data);
            },
            |ui| {
                defaults =
                    kit::text_button(ui, "Restore page defaults", footer.defaults_enabled).on_hover_text("Reset the settings on this page").clicked();
            },
        )
    });
    let action = match modal.inner {
        (_, kit::Action::Ok) => Action::Ok,
        (_, kit::Action::Apply) => Action::Apply,
        (closed, action) if closed || action == kit::Action::Cancel || modal.should_close() => Action::Cancel,
        _ if defaults => Action::Defaults,
        _ => Action::None,
    };
    if !matches!(action, Action::None) {
        ctx.data_mut(|data| data.remove::<Search>(search_id()));
    }
    action
}

/// Arrow keys move through the found rows and Enter opens the one under the cursor.
/// Esc clears an active search before it can close the window. Returns the row to open.
fn keyboard(ui: &mut Ui, state: &mut WindowState, field: &Response) -> Option<Hit> {
    if !searching(&state.query) {
        with_search(ui, |search| search.cursor = 0);
        return None;
    }
    if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        state.query.clear();
        field.request_focus();
        return None;
    }
    // Ask about focus before taking the input lock: the context can't be read inside it.
    let in_field = field.has_focus() || field.lost_focus();
    let (down, up, enter) = ui.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
            i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
            i.key_pressed(Key::Enter) && in_field,
        )
    });
    with_search(ui, |search| {
        let count = search.last_found.len();
        if count == 0 {
            search.cursor = 0;
            return None;
        }
        if down {
            search.cursor = (search.cursor + 1).min(count - 1);
        }
        if up {
            search.cursor = search.cursor.saturating_sub(1);
        }
        search.cursor = search.cursor.min(count - 1);
        enter.then(|| search.last_found[search.cursor].clone())
    })
}

/// Whether `query` is long enough to search with: one letter would match nearly everything.
/// While it is, the window shows results from every page instead of the selected one.
pub fn searching(query: &str) -> bool {
    query.trim().chars().count() >= 2
}

/// The sidebar of pages and the scrolling page body, or every page's matching rows while searching.
fn body<S>(ui: &mut Ui, id: Id, state: &mut WindowState, pages: &[Page<S>], data: &mut S) {
    let t = Tokens::get();
    let width = ui.available_width();
    // The shared window adds 12 points of padding around this body. Keep the page scroll
    // within its budget so the sidebar and footer stay in reach on short viewports.
    let shared = solidcraft_theme::Tokens::get();
    let height = (ui.ctx().content_rect().height() - shared.form_header_height - shared.form_footer_height - shared.space_24 * 3.0)
        .clamp(0.0, t.control_height * 18.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let nav_width = if width < NARROW_AT { NARROW_SIDEBAR } else { t.form_label_width * 2.0 + t.space_24 * 2.0 }.min(width);
    let nav = Rect::from_min_size(rect.min, vec2(nav_width, height));
    ui.painter().rect_filled(nav, 0, t.dialog_footer);
    ui.painter().vline(nav.right() - 0.5, nav.y_range(), Stroke::new(1.0, t.dialog_rule));
    let list = nav.shrink2(vec2(t.space_8, t.space_6));
    let mut sidebar = ui.new_child(UiBuilder::new().max_rect(list).layout(Layout::top_down(Align::Min)));
    sidebar.set_clip_rect(sidebar.clip_rect().intersect(nav));
    egui::ScrollArea::vertical().id_salt("settings_pages").max_height(list.height().max(0.0)).show(&mut sidebar, |ui| {
        ui.spacing_mut().item_spacing.y = t.space_2;
        for (index, page) in pages.iter().enumerate() {
            if nav_row(ui, page, index == state.page).clicked() {
                state.page = index;
                state.query.clear();
            }
        }
    });

    let content = Rect::from_min_max(pos2(nav.right(), rect.top()), rect.max);
    let padding = vec2(t.space_24 - t.space_2, t.space_16 + t.space_2);
    let content = Rect::from_min_max(content.min + padding, content.max - vec2(t.space_8, t.space_8));
    if !content.is_positive() {
        return;
    }
    // Both slots were allocated in the body above. A long control may scroll or clip inside
    // its slot, but must not grow the window past its chosen viewport width.
    let mut contents = ui.new_child(UiBuilder::new().max_rect(content).layout(Layout::top_down(Align::Min)));
    contents.set_clip_rect(contents.clip_rect().intersect(content));
    contents.scope(|ui| {
        let field = kit::preferences_search_field(ui, id.with("search"), &mut state.query, "Search settings", "preferences");
        if ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, Key::F)) {
            field.request_focus();
        }
        if let Some(hit) = keyboard(ui, state, &field) {
            state.page = hit.page;
            state.query.clear();
            let until = ui.input(|i| i.time) + REVEAL_SECONDS;
            with_search(ui, |search| search.reveal = Some((hit, until)));
            ui.ctx().request_repaint();
        }
        let query = fold(state.query.trim());
        let active = searching(&state.query);
        // Read the clock before taking the memory lock.
        let now = ui.input(|i| i.time);
        with_search(ui, |search| {
            search.last_found = std::mem::take(&mut search.found);
            search.query = if active { query } else { String::new() };
            search.probing = 0;
            search.hits = 0;
            if search.reveal.as_ref().is_some_and(|(_, until)| now > *until) {
                search.reveal = None;
            }
        });
        let salt = if active { usize::MAX } else { state.page };
        egui::ScrollArea::vertical().id_salt(("settings_page", salt)).auto_shrink([false, false]).show(ui, |ui| {
            crate::workspace::describe_scroll_viewport(ui);
            ui.set_width(ui.available_width() - t.space_12);
            ui.spacing_mut().item_spacing.y = t.form_gap_y;
            if !active {
                begin_page(ui, state.page, "");
                if let Some(page) = pages.get(state.page) {
                    (page.draw)(ui, data);
                }
                return;
            }
            let mut any = false;
            for (index, page) in pages.iter().enumerate() {
                begin_page(ui, index, page.label);
                if !probe(ui, ("settings_probe_page", index), |ui| (page.draw)(ui, data)) {
                    continue;
                }
                if any {
                    ui.add_space(t.space_16);
                }
                any = true;
                begin_page(ui, index, page.label);
                ui.label(RichText::new(page.label).font(fonts::semibold(t.font_intro)).color(t.text));
                (page.draw)(ui, data);
            }
            if !any {
                ui.label(RichText::new(format!("No settings match “{}”", state.query.trim())).size(t.font_body).color(t.form_dim));
            }
        });
    });
}

/// One sidebar page row: glyph and label, filled when selected or hovered.
fn nav_row<S>(ui: &mut Ui, page: &Page<S>, selected: bool) -> Response {
    let t = Tokens::get();
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), t.control_height + t.space_4), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, page.label));
    if selected || response.hovered() || response.has_focus() {
        ui.painter().rect_filled(rect, t.control_radius, if selected { t.selected_bg } else { t.hover });
    }
    let ink = if selected { t.form_primary } else { t.text };
    let icon = pos2(rect.left() + t.space_8 + t.space_8 + t.space_2, rect.center().y);
    let mut painter = ui.painter().clone();
    if !selected {
        painter.multiply_opacity(0.8);
    }
    page.icon.paint_sized(&painter, icon, t.space_16, t, selected);
    let font = if selected { fonts::semibold(t.font_label) } else { FontId::proportional(t.font_label) };
    let galley = ui.painter().layout_no_wrap(page.label.to_owned(), font, ink);
    let left = icon.x + t.space_8 + NAV_ROW_GAP;
    ui.painter().with_clip_rect(rect).galley(pos2(left, rect.center().y - galley.size().y / 2.0), galley, ink);
    response
}

/// Start a page for the rows that follow, so a search for the page name shows all of them.
fn begin_page(ui: &Ui, index: usize, label: &str) {
    with_search(ui, |search| {
        search.page = index;
        search.page_hit = !search.query.is_empty() && matches(&search.query, label);
        search.section_hit = false;
    });
}

/// Run `add` as a counting pass, in which rows draw nothing, and report whether any row matched.
/// The child is invisible and disabled too, so anything else it draws can't be seen or clicked.
fn probe(ui: &mut Ui, salt: impl egui::AsIdSalt, add: impl FnOnce(&mut Ui)) -> bool {
    let before = with_search(ui, |search| {
        search.probing += 1;
        search.hits
    });
    let mut child = ui.new_child(UiBuilder::new().id_salt(salt).max_rect(ui.max_rect()).layout(*ui.layout()).invisible());
    add(&mut child);
    with_search(ui, |search| {
        search.probing -= 1;
        search.hits > before
    })
}

/// Whether the window is showing search results. A page that draws its own controls rather than
/// [`row`]s uses this to offer a searchable row in their place.
pub fn is_searching(ui: &Ui) -> bool {
    with_search(ui, |search| !search.query.is_empty())
}

/// A page's title and the line under it. Hidden while searching, where results are grouped by page.
pub fn heading(ui: &mut Ui, title: &str, description: &str) {
    if with_search(ui, |search| !search.query.is_empty()) {
        return;
    }
    let t = Tokens::get();
    ui.label(RichText::new(title).font(fonts::semibold(t.font_page_heading)).color(t.text));
    if !description.is_empty() {
        ui.label(RichText::new(description).size(t.font_label).color(t.form_dim));
    }
}

/// A titled group of rows under a rule. While searching it shows only when its title, its page
/// or one of its rows matches, and a title match shows every row in it.
pub fn section(ui: &mut Ui, title: &str, mut add: impl FnMut(&mut Ui)) {
    let active = with_search(ui, |search| !search.query.is_empty());
    if active {
        let section_hit = with_search(ui, |search| search.page_hit || matches(&search.query, title));
        with_search(ui, |search| search.section_hit = section_hit);
        if !probe(ui, ("settings_probe_section", title), &mut add) {
            return;
        }
        with_search(ui, |search| search.section_hit = section_hit);
    }
    form_section(ui, title, |ui| add(ui));
    with_search(ui, |search| search.section_hit = search.page_hit);
}

/// Draw the Preferences section treatment without search state or repeated probe calls.
/// Object-property forms share its heading, rule and row rhythm while owning their own draft.
pub fn form_section<R>(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    let t = Tokens::get();
    ui.add_space(t.space_16);
    ui.label(RichText::new(title.to_uppercase()).font(fonts::semibold(t.font_small)).color(t.form_dim));
    let (rule, _) = ui.allocate_exact_size(vec2(ui.available_width(), t.space_6), Sense::hover());
    ui.painter().hline(rule.x_range(), rule.center().y, Stroke::new(1., t.dialog_rule));
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = t.space_8;
        add(ui)
    })
    .inner
}

/// Dialog actions share the Preferences palette, corner radius and minimum control size.
pub fn action_button(label: &str, primary: bool) -> egui::Button<'_> {
    let t = Tokens::get();
    egui::Button::new(RichText::new(label).size(t.font_label).color(if primary { t.form_primary } else { t.text }))
        .fill(if primary { t.selected_bg } else { t.panel_header })
        .stroke(Stroke::new(1., if primary { t.form_primary } else { t.border }))
        .corner_radius(t.radius_sm)
        .min_size(vec2(t.space_24 * 3., t.control_height))
}

/// A label and its control. See [`row_with_keywords`].
pub fn row<R>(ui: &mut Ui, label: &str, control: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    row_with_keywords(ui, label, &[], control)
}

/// A label in the form's label column and a control beside it. While searching, the row is
/// drawn only when its label, one of `keywords`, its section or its page matches, and `None` is
/// returned when it is skipped. A row opened from search results is outlined for a moment.
pub fn row_with_keywords<R>(ui: &mut Ui, label: &str, keywords: &[&str], control: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    let t = Tokens::get();
    let (show, real, cursor, reveal) = with_search(ui, |search| {
        let active = !search.query.is_empty();
        let show = !active
            || search.section_hit
            || matches(&search.query, label)
            || keywords.iter().chain(theme_keywords(label)).any(|word| matches(&search.query, word));
        let real = search.probing == 0;
        if show {
            search.hits += 1;
        }
        let hit = Hit { page: search.page, label: label.to_owned() };
        let cursor = active && real && show && search.last_found.get(search.cursor) == Some(&hit);
        if active && real && show {
            search.found.push(hit.clone());
        }
        let reveal = !active && search.reveal.as_ref().is_some_and(|(revealed, _)| *revealed == hit);
        (show, real, cursor, reveal)
    });
    // A probe only counts matches. Drawing nothing keeps its rows out of the accessibility tree.
    if !show || !real {
        return None;
    }
    let label_width = label_column(ui);
    let text = RichText::new(label).size(t.font_label).color(t.text);
    let controls = |ui: &mut Ui| {
        let room = ui.available_width().max(0.0);
        ui.allocate_ui_with_layout(vec2(room, t.control_height), Layout::left_to_right(Align::Center).with_main_wrap(true), |ui| {
            solidcraft_ui_kit::named_control(ui, label, control)
        })
        .inner
    };
    let inner = if label_width == 0.0 {
        ui.vertical(|ui| {
            ui.add(Label::new(text).wrap());
            controls(ui)
        })
    } else {
        ui.horizontal(|ui| {
            ui.set_min_height(t.control_height);
            ui.spacing_mut().item_spacing.x = t.space_8;
            ui.allocate_ui_with_layout(vec2(label_width, t.control_height), Layout::left_to_right(Align::Center), |ui| {
                ui.set_width(label_width);
                ui.add(Label::new(text).truncate());
            });
            controls(ui)
        })
    };
    let rect = inner.response.rect.expand2(vec2(t.space_6, t.space_2));
    if cursor || reveal {
        ui.painter().rect_stroke(rect, t.radius_sm, Stroke::new(1.5, t.form_primary), StrokeKind::Outside);
        if reveal {
            ui.scroll_to_rect(rect, Some(Align::Center));
            ui.ctx().request_repaint();
        }
        if cursor {
            ui.scroll_to_rect(rect, None);
        }
    }
    Some(inner.inner)
}

/// Reserve a label column only when there is room left for a slider and its value.
/// Zero means the label and control stack within the page width.
fn label_column(ui: &Ui) -> f32 {
    let t = Tokens::get();
    let width = t.form_label_width * 2.0 + t.space_8;
    if ui.available_width() >= width + t.space_24 * 7.0 { width } else { 0.0 }
}

/// A line of explanation under a row, aligned with its control.
pub fn hint(ui: &mut Ui, text: &str) {
    if with_search(ui, |search| search.probing > 0 || (!search.query.is_empty() && !search.section_hit)) {
        return;
    }
    let t = Tokens::get();
    let column = label_column(ui);
    ui.horizontal(|ui| {
        if column > 0.0 {
            ui.add_space(column + t.space_8);
        }
        ui.add(Label::new(RichText::new(text).size(t.font_small).color(t.form_dim)).wrap());
    });
}

/// Choose a single option in the shared dropdown.
/// The label supplies a stable accessible name; returns whether the value changed.
pub fn choice<T: Copy + PartialEq>(ui: &mut Ui, label: &str, value: &mut T, options: &[(T, &str)]) -> bool {
    choose_option(value, options, |selected, labels| {
        solidcraft_ui_kit::named_control(ui, label, |ui| solidcraft_ui_kit::dropdown(ui, label, selected, labels));
    })
}

/// Choose one of `options` with the shared segments, which fall back to a dropdown in a narrow row.
/// Returns whether the setting changed.
pub fn segmented<T: Copy + PartialEq>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    choose_option(value, options, |selected, labels| {
        solidcraft_ui_kit::segmented(ui, selected, labels, solidcraft_ui_kit::Segments { dense: true, intrinsic: true, ..Default::default() });
    })
}

/// Give an indexed widget the labels and the current selection, keeping the setting's typed value.
/// A value absent from the options stays in place until the widget selects a valid option.
fn choose_option<T: Copy + PartialEq>(value: &mut T, options: &[(T, &str)], draw: impl FnOnce(&mut usize, &[&str])) -> bool {
    let before = *value;
    let mut selected = options.iter().position(|(option, _)| *option == before).unwrap_or(options.len());
    let labels: Vec<_> = options.iter().map(|(_, label)| *label).collect();
    draw(&mut selected, &labels);
    if let Some((option, _)) = options.get(selected) {
        *value = *option;
    }
    *value != before
}

/// Draw a shared checkbox with a visible label and an accessible setting name.
pub fn checkbox(ui: &mut Ui, label: &str, accessible_name: &str, value: &mut bool) -> Response {
    solidcraft_ui_kit::checkbox(ui, label, value).accessible_name(accessible_name)
}

/// A keyboard key or gesture as a bordered chip, for read-only shortcut lists.
pub fn key_chip(ui: &mut Ui, text: &str) -> Response {
    let t = Tokens::get();
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::monospace(t.font_small), t.text);
    let size = galley.size() + vec2(t.space_12, t.space_6);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, text));
    ui.painter().rect(rect, t.radius_sm, t.dialog_footer, Stroke::new(1.0, t.border), StrokeKind::Inside);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, t.text);
    response
}

/// Prefix, the start of a word, or a run of at least three characters, ignoring case and
/// accents, as SolidCraft's Preferences search scores a setting. `query` is already folded.
fn matches(query: &str, text: &str) -> bool {
    if query.chars().count() < 2 || text.is_empty() {
        return false;
    }
    let text = fold(text);
    text.starts_with(query)
        || text.split(|c: char| !c.is_alphanumeric()).any(|word| !word.is_empty() && word.starts_with(query))
        || (query.chars().count() >= 3 && text.contains(query))
}

/// Words that find a setting even though they are not written on it.
fn theme_keywords(label: &str) -> &'static [&'static str] {
    match label {
        "Theme" => &["dark", "light", "appearance"],
        "Interface scale" => &["zoom", "size", "dpi"],
        _ => &[],
    }
}

/// Lower case without the marks that tell "é" from "e".
fn fold(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            let base = match c {
                'à'..='å' | 'À'..='Å' => 'a',
                'ç' | 'Ç' => 'c',
                'è'..='ë' | 'È'..='Ë' => 'e',
                'ì'..='ï' | 'Ì'..='Ï' => 'i',
                'ñ' | 'Ñ' => 'n',
                'ò'..='ö' | 'Ò'..='Ö' => 'o',
                'ù'..='ü' | 'Ù'..='Ü' => 'u',
                c => c,
            };
            base.to_lowercase()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_match_is_a_prefix_a_word_start_or_a_run_of_three() {
        assert!(matches("zo", "Zoom speed"));
        assert!(matches("speed", "Zoom speed"));
        assert!(matches("oom", "Zoom speed"));
        assert!(!matches("zs", "Zoom speed"));
        assert!(!matches("z", "Zoom speed"));
        assert!(matches(&fold("Préf"), "preferences"));
    }

    #[test]
    fn short_queries_do_not_search() {
        assert!(!searching(" a "));
        assert!(searching("ab"));
    }
}
