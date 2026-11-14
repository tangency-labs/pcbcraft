//! Library workspace adapters over the pinned Craft tree, table and panel primitives.
mod browser;
mod column;
use crate::{Tokens, controls};
pub use browser::Browser;
pub use column::ColumnWidth;
use egui::{Align, Layout, Rect, RichText, Ui, UiBuilder, pos2, vec2};
use solidcraft_ui_kit as kit;

/// Independently scrolling regions share their host's theme and bounded pane rectangle.
#[derive(Clone, Copy)]
pub enum Region {
    Browser,
    Items,
    Preview,
}

/// Tree actions change only host-owned navigation, never model or source state.
#[derive(Clone, Copy)]
pub enum TreeAction {
    ExpandAll,
    CollapseAll,
}

/// Keep three useful columns on wide hosts and expose the same regions through shared choices
/// on compact hosts. Every region clips and scrolls independently of the ribbon and pane header.
pub fn workspace(
    ui: &mut Ui,
    compact: &mut usize,
    preview: &mut crate::workspace::panels::Panel,
    collapsed: &mut bool,
    browser: &mut Browser,
    preview_width: &mut ColumnWidth,
    mut draw: impl FnMut(&mut Ui, Region),
) -> Option<TreeAction> {
    let mut action = None;
    let t = Tokens::get();
    let mut area = ui.available_rect_before_wrap().shrink(t.space_8);
    if area.width() < 1050.0 {
        browser.cancel_resize();
        preview_width.cancel_resize();
        kit::segmented(ui, compact, &["Browser", "Items", "Preview"], kit::Segments { small: true, ..Default::default() });
        area.min.y = ui.cursor().min.y + t.space_8;
        let region = [Region::Browser, Region::Items, Region::Preview][(*compact).min(2)];
        if !matches!(region, Region::Preview) {
            preview.cancel_drag();
        }
        if matches!(region, Region::Preview) {
            let (_, toggle) =
                preview.show_in_slot(ui, ui.scope_id().with("library_preview"), "Preview", area, area, *collapsed, |ui| draw(ui, region));
            *collapsed ^= toggle;
        } else {
            action = card(ui, area, region, if matches!(region, Region::Browser) { Some(&mut browser.collapsed) } else { None }, &mut draw);
        }
    } else {
        let gap = t.space_12;
        let default_right = (area.width() * 0.29).clamp(300.0, 440.0);
        let floating = preview.position() == crate::workspace::panels::Position::Floating;
        let left = browser.resize(ui, area, area.width() - if floating { 0.0 } else { 240.0 + gap } - gap - 360.0);
        let max_preview = (area.width() - left - gap * 2.0 - 360.0).max(240.0);
        let mut right = preview_width.width(default_right).clamp(240.0, max_preview);
        let browser_rect = Rect::from_min_max(area.min, pos2(area.left() + left, area.bottom()));
        let on_left = preview.position() == crate::workspace::panels::Position::Left;
        if floating {
            preview_width.cancel_resize();
        } else {
            let x = if on_left { browser_rect.right() + gap + right + gap / 2.0 } else { area.right() - right - gap / 2.0 };
            let bounds = Rect::from_min_max(pos2(x - t.space_6 / 2.0, area.top()), pos2(x + t.space_6 / 2.0, area.bottom()));
            right = preview_width.resize(
                ui,
                ui.make_persistent_id("library_preview_width"),
                default_right,
                column::Sash { bounds, name: "Resize Preview column width", min: 240.0, max: max_preview, leading: !on_left },
            );
        }
        let preview_rect = if on_left {
            Rect::from_min_max(pos2(browser_rect.right() + gap, area.top()), pos2(browser_rect.right() + gap + right, area.bottom()))
        } else {
            Rect::from_min_max(pos2(area.right() - right, area.top()), area.max)
        };
        let items = Rect::from_min_max(
            pos2(if on_left && !floating { preview_rect.right() + gap } else { browser_rect.right() + gap }, area.top()),
            pos2(if !on_left && !floating { preview_rect.left() - gap } else { area.right() }, area.bottom()),
        );
        for (rect, region) in [(browser_rect, Region::Browser), (items, Region::Items)] {
            action =
                card(ui, rect, region, if matches!(region, Region::Browser) { Some(&mut browser.collapsed) } else { None }, &mut draw).or(action);
        }
        let (_, toggle) =
            preview.show_in_slot(ui, ui.scope_id().with("library_preview"), "Preview", area, preview_rect.shrink(1.0), *collapsed, |ui| {
                draw(ui, Region::Preview)
            });
        *collapsed ^= toggle;
    }
    ui.advance_cursor_after_rect(area);
    action
}

/// A card uses the same toolkit surface and font roles as the existing Browser and Properties.
fn card(ui: &mut Ui, rect: Rect, region: Region, mut collapsed: Option<&mut bool>, draw: &mut impl FnMut(&mut Ui, Region)) -> Option<TreeAction> {
    let mut action = None;
    let id = match region {
        Region::Browser => "library_browser",
        Region::Items => "library_items",
        Region::Preview => "library_preview",
    };
    let rect = rect.shrink(1.0);
    let mut child = ui.new_child(UiBuilder::new().id_salt(id).max_rect(rect).layout(Layout::top_down(Align::Min)));
    child.set_clip_rect(ui.clip_rect().intersect(rect));
    let frame = kit::panel_host_frame(ui.ctx(), true).inner_margin(Tokens::get().space_12);
    let margin = frame.total_margin().sum();
    frame.show(&mut child, |ui| {
        let height = if collapsed.as_deref().copied().unwrap_or(false) { Tokens::get().control_height } else { rect.height() - margin.y };
        ui.set_min_size(vec2((rect.width() - margin.x).max(0.0), height.max(0.0)));
        ui.set_max_size(ui.min_size());
        if let Some(collapsed) = collapsed.as_deref_mut() {
            ui.horizontal(|ui| {
                heading(ui, "Browser");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let (_, rect) = ui.allocate_space(vec2(Tokens::get().space_24, Tokens::get().space_24));
                    if kit::panel_disclosure_button_at(ui, rect, !*collapsed, if *collapsed { "Expand Browser" } else { "Collapse Browser" })
                        .clicked()
                    {
                        *collapsed = !*collapsed;
                    }
                    let (_, rect) = ui.allocate_space(vec2(Tokens::get().space_24, Tokens::get().space_24));
                    let more = kit::button_at(ui, "library_browser_actions", rect, "…", kit::Button::Ghost).accessible_name("Browser tree actions");
                    egui::Popup::menu(&more).frame(crate::menu::frame()).show(|ui| {
                        for (label, choice) in [("Expand all", TreeAction::ExpandAll), ("Collapse all", TreeAction::CollapseAll)] {
                            if crate::MenuItem::new(label).show(ui) {
                                action = Some(choice);
                                ui.close();
                            }
                        }
                    });
                });
            });
        } else if matches!(region, Region::Preview) {
            heading(ui, "Preview");
        }
        if !collapsed.as_deref().copied().unwrap_or(false) {
            ui.add_space(Tokens::get().space_8);
            draw(ui, region);
        }
    });
    action
}

/// Host-owned Browser metadata, with a qualified icon and an optional accessible path.
pub struct Node<'a> {
    pub label: &'a str,
    pub depth: usize,
    pub fold: Option<bool>,
    pub selected: bool,
    pub count: &'a str,
    pub readonly: bool,
    pub problem: Option<&'a str>,
    pub icon: crate::workspace::ToolIcon,
    pub accessible: &'a str,
}

/// Reuse the pinned tree's fold/count/status slots; electronics glyphs occupy its reserved icon slot.
pub fn tree(ui: &mut Ui, id: egui::Id, node: Node<'_>) -> (bool, bool) {
    let Node { label, depth, fold, selected, count, readonly, problem, icon, accessible } = node;
    let response = kit::tree_row(
        ui,
        id,
        &kit::Row {
            depth,
            label,
            name: true,
            fold,
            selected,
            count,
            compact: true,
            icon: crate::ribbon_icons::HOST_PAINTED,
            badges: if readonly { &["lock"] } else { &[] },
            status: problem.map(|_| kit::Status::Warning),
            tip: problem,
            ..Default::default()
        },
    );
    if let Some(rect) = response.rect {
        let t = Tokens::get();
        crate::workspace::paint_icon(
            ui.painter(),
            icon,
            pos2(rect.left() + t.space_8 + depth as f32 * solidcraft_theme::Tokens::get().space_14 + t.space_16 + t.space_8, rect.center().y),
            t.space_16,
            t,
            true,
        );
    }
    if let Some(target) = response.resp.as_ref() {
        target.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), selected, accessible));
    }
    let mut folded = response.fold;
    if let Some(target) = response.resp.as_ref()
        && let Some(open) = fold
    {
        ui.ctx().accesskit_node_builder(target.id, |node| node.set_expanded(open));
        if target.has_focus() {
            folded |= ui.input(|i| if open { i.key_pressed(egui::Key::ArrowLeft) } else { i.key_pressed(egui::Key::ArrowRight) });
        }
    }
    (response.clicked, folded)
}

/// A presentation-only table entry keeps stable identity separate from displayed cells.
pub struct TableEntry {
    pub key: String,
    pub cells: Vec<String>,
    pub selected: bool,
    pub branch: Option<bool>,
    pub nested: bool,
}
/// Branch expansion and row selection are separate actions, as in the Browser tree.
pub enum ItemAction {
    Select(usize),
    Toggle(usize),
}

/// Read-only tables use the kit's selectable rows, clipping, cell widths and accessibility.
pub fn table(ui: &mut Ui, headers: &[&str], rows: &[TableEntry], mut select: impl FnMut(ItemAction)) {
    ui.spacing_mut().scroll.floating = false;
    ui.spacing_mut().scroll.bar_width = Tokens::get().space_6;
    let columns: Vec<_> = headers
        .iter()
        .enumerate()
        .map(|(i, label)| kit::Column::new(label, if i == 0 { 2.0 } else { 1.0 }).flex().min_content_width(if i == 0 { 160.0 } else { 95.0 }))
        .collect();
    egui::ScrollArea::vertical().id_salt("library_table_vertical").auto_shrink([false, false]).show(ui, |ui| {
        kit::table_custom(ui, "library_items", &columns, "No matching items", |ui, table| {
            table.headers(ui, |_, _| false);
            for (index, entry) in rows.iter().enumerate() {
                let mut fold = false;
                if table
                    .row_visible(ui, &entry.key, entry.selected, false, |row| {
                        for (column, cell) in entry.cells.iter().enumerate() {
                            row.cell(|ui| {
                                if column == 0 {
                                    if entry.nested {
                                        ui.add_space(Tokens::get().space_24);
                                    }
                                    if let Some(open) = entry.branch {
                                        let (_, rect) = ui.allocate_space(vec2(Tokens::get().space_16, Tokens::get().control_height));
                                        fold = kit::panel_disclosure_button_at(
                                            ui,
                                            rect,
                                            open,
                                            &format!("{} {}", if open { "Collapse" } else { "Expand" }, cell),
                                        )
                                        .clicked();
                                    }
                                }
                                ui.add(
                                    egui::Label::new(if entry.selected && column == 0 { RichText::new(cell).strong() } else { RichText::new(cell) })
                                        .selectable(false)
                                        .truncate(),
                                )
                                .on_hover_text(cell);
                            });
                        }
                    })
                    .clicked()
                {
                    select(ItemAction::Select(index));
                }
                if fold {
                    select(ItemAction::Toggle(index));
                }
            }
        });
    });
}

/// Section headings retain the existing app's shared semibold face and type scale.
pub fn heading(ui: &mut Ui, title: &str) {
    let t = Tokens::get();
    ui.label(RichText::new(title).font(crate::fonts::interface_heading(ui, t.font_heading)).color(t.text));
}

/// Compact read-only metadata retains full values on hover without looking editable.
pub fn detail(ui: &mut Ui, label: &str, value: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(label).color(Tokens::get().text_dim));
        ui.label(value);
    });
}

/// Exact revision choices use the shared control's focus and selected state.
pub fn revision(ui: &mut Ui, label: &str, selected: bool) -> bool {
    kit::button(ui, label, if selected { kit::Button::SmallSelected } else { kit::Button::Small }).clicked()
}

/// Type is the existing kind filter; category definitions are only dependency metadata here.
pub fn type_filter(ui: &mut Ui, current: &mut usize) {
    const TYPES: [&str; 5] = ["All", "Symbol", "Footprint", "Generic part", "Orderable part"];
    let mut value = TYPES[(*current).min(4)].to_owned();
    let options: Vec<_> = TYPES.into_iter().map(|s| (s, s.to_owned())).collect();
    crate::properties::choice(ui, "Type", ui.make_persistent_id("library_type"), &mut value, &options);
    *current = TYPES.iter().position(|s| *s == value).unwrap_or(0);
}

/// The single acquisition action leads to folder/bundle selection on either platform.
pub fn add_choices(ui: &mut Ui) -> (bool, bool) {
    ui.set_min_width(Tokens::get().control_height * 8.0);
    ui.spacing_mut().item_spacing.y = 0.0;
    let folder = crate::MenuItem::new("Choose folder…")
        .tool_icon(crate::workspace::ToolIcon::LibraryFolder(crate::workspace::LibraryFolderAction::Add))
        .show(ui);
    let bundle = crate::MenuItem::new("Choose bundle…").tool_icon(crate::workspace::ToolIcon::Shared("library")).show(ui);
    crate::menu::separator(ui);
    crate::MenuItem::new("Connect Git…").enabled(false).show(ui);
    crate::MenuItem::new("Connect remote catalog…").enabled(false).show(ui);
    (folder, bundle)
}

/// Main item titles use the same interface introduction face as the app's catalog forms.
pub fn title(ui: &mut Ui, title: &str) {
    let t = Tokens::get();
    ui.label(RichText::new(title).font(crate::fonts::interface_heading(ui, t.font_intro)).color(t.text));
}

/// Search and Type share one bounded toolbar, wrapping when controls would become cramped.
pub fn toolbar(ui: &mut Ui, query: &mut String, kind: &mut usize) {
    let width = ui.available_width();
    if width < 440.0 {
        controls::search(ui, ui.make_persistent_id("library_search"), query, "Search this library", "Search library items");
        type_filter(ui, kind);
    } else {
        ui.horizontal(|ui| {
            ui.allocate_ui(vec2((width * 0.65).max(220.0), Tokens::get().control_height), |ui| {
                controls::search(ui, ui.make_persistent_id("library_search"), query, "Search this library", "Search library items");
            });
            ui.vertical(|ui| type_filter(ui, kind));
        });
    }
}

/// Read-only preview sections have one shared separation rhythm without instructional captions.
pub fn section(ui: &mut Ui, title: &str) {
    ui.add_space(Tokens::get().space_12);
    heading(ui, title);
}
