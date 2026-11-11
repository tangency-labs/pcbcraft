//! Shared workspace surfaces and tool buttons. Command state stays with the caller.

use egui::{
    Align, Align2, Color32, FontId, Frame, Label, Layout, Margin, Panel, Pos2, Rect, Response, RichText, Role, ScrollArea, Sense, Shape, Stroke, Ui,
    WidgetInfo, pos2, vec2,
};

use crate::{Tokens, fonts, menu::CheckKind};

pub mod catalog;
pub mod panels;

/// The card is tall enough for the tabs, the groups and PCBcraft Next's 5 point gutter under them.
const RIBBON_HEIGHT: f32 = 135.0;
const WORKSPACE_WIDTH: f32 = 88.0;
const RIBBON_COMPACT_WIDTH: f32 = 1000.0;

/// Compact ribbons promote working tools while retaining planned controls in their group menus.
pub fn ribbon_is_compact(ui: &Ui) -> bool {
    ui.clip_rect().width() < RIBBON_COMPACT_WIDTH
}
/// Shared ribbon height with PCBcraft's qualified command glyphs and inline field widths.
const RIBBON_TOOL_WIDTH: f32 = solidcraft_ui_kit::ribbon::BUTTON_MAX_WIDTH;
const RIBBON_TOOL_HEIGHT: f32 = solidcraft_ui_kit::ribbon::BUTTON_HEIGHT;
const RIBBON_TOOL_ICON: f32 = 24.0;
const RIBBON_GROUP_PADDING: f32 = 6.0;
const RIBBON_SEPARATOR_WIDTH: f32 = 12.0;
const RIBBON_CAPTION_GAP: f32 = 5.0;
const RIBBON_SCROLL_GUTTER: f32 = 5.0;

/// The glyphs used by the schematic ribbon actions and Browser rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolIcon {
    Select,
    Pin,
    Pad,
    Line,
    Rectangle,
    Circle,
    Subsheet,
    Power,
    /// Original three-bar ground marker for direct placement actions.
    Ground,
    Text,
    Bus,
    NetLabel,
    Move,
    Rotate,
    Mirror,
    Rules,
    Erc,
    Annotate,
    Bom,
    Sync,
    Sheet,
    HierarchicalLabel,
    Symbol,
    Wire,
    Slice,
    Break,
    Heal,
    Delete,
    Nets,
    Fit,
    /// Dots for the drawing-grid visibility control.
    Grid,
    Browser,
    SplitView,
    EditorWindow,
    Settings,
    /// PCB layout glyphs, drawn natively: a package, copper routing and board-level actions.
    Footprint,
    Track,
    Via,
    Zone,
    Pair,
    Tune,
    Align,
    Drc,
    Measure,
    FlipView,
    Outputs,
    Refill,
}

/// The glyph at the start of a Browser row: a container or one of the ribbon's own glyphs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeIcon {
    Folder,
    Sheet,
    Board,
    Tool(ToolIcon),
    /// A layer uses the same tree row surface, with an explicit active arrow and color key.
    Layer {
        color: Color32,
        active: bool,
        editable: bool,
    },
}

/// One row of a Browser tree. The caller owns the tree and its fold state; the row only draws.
#[derive(Clone, Copy, Debug)]
pub struct TreeRow<'a> {
    /// Nesting depth; each level indents the row by one step.
    pub depth: usize,
    /// `Some(open)` for a row that folds, drawn with a disclosure arrow.
    pub fold: Option<bool>,
    pub icon: Option<TreeIcon>,
    /// The visible name; also the accessible label unless overridden below.
    pub label: &'a str,
    /// Optional semantic name distinguishes matching symbol/footprint labels for assistive technology.
    pub accessibility_label: Option<&'a str>,
    /// Hover text with more about the row. Empty for none.
    pub tip: &'a str,
    /// Drawn with the selection fill and a semibold label.
    pub selected: bool,
}

/// What happened on a [`tree_row`] this frame. A click on the disclosure arrow only folds.
#[derive(Clone, Debug)]
pub struct TreeResponse {
    /// The stable row response supports contextual actions owned by the navigator.
    pub response: Response,
    pub clicked: bool,
    /// An editable layer swatch has a separate button; this never activates the row.
    pub icon_clicked: bool,
    pub fold: bool,
}

/// PCBcraft Next's Browser metrics: row height, indent per level, the disclosure arrow's hit
/// target, the glyph size and where the glyph and label sit past the arrow.
const TREE_ROW_HEIGHT: f32 = 26.0;
const TREE_INDENT: f32 = 16.0;
const TREE_FOLD_WIDTH: f32 = 18.0;
const TREE_ICON: f32 = 15.0;
const TREE_ICON_CENTER: f32 = 11.0;
const TREE_LABEL_LEFT: f32 = 25.0;
/// The floating card's distance from the canvas edges, inner margin, corner radius and header.
const FLOATING_GAP: f32 = 12.0;
const FLOATING_MARGIN: f32 = 10.0;
const FLOATING_RADIUS: u8 = 12;
const FLOATING_HEADER: f32 = 24.0;

/// The shared surface for cards that overlay the canvas.
fn floating_frame(ctx: &egui::Context) -> Frame {
    solidcraft_ui_kit::panel_host_frame(ctx, true).inner_margin(Margin::same(FLOATING_MARGIN as i8))
}

/// The drawing on a workspace's tile in the picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceIcon {
    Schematic,
    Layout,
    Render,
    Simulate,
}

/// One row of the workspace picker. The app owns the list; the picker only draws it.
#[derive(Clone, Copy, Debug)]
pub struct Workspace<'a> {
    /// Stable application-owned workspace key, independent of its label and visual order.
    pub id: &'a str,
    pub title: &'a str,
    /// Whether this workspace supports split or native-window placements.
    pub multiple_views: bool,
    pub description: &'a str,
    /// The ribbon tabs the workspace offers. `None` marks a planned workspace, which is drawn
    /// dimmed with a Planned badge and can't be picked.
    pub tabs: Option<&'a str>,
    pub icon: WorkspaceIcon,
    /// The workspace on screen, drawn with a Current badge.
    pub current: bool,
}

/// What the workspace tile offers: the workspaces in order and a note under them.
#[derive(Clone, Copy, Debug)]
pub struct Workspaces<'a> {
    pub entries: &'a [Workspace<'a>],
    pub footer: &'a str,
    pub native_windows: bool,
}

/// How a chosen workspace is presented; document state remains owned by the application.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    Here,
    Beside,
    Window,
}

/// A picker action identifies both a workspace entry and its requested placement.
#[derive(Clone, Copy, Debug)]
pub struct WorkspacePick {
    pub index: usize,
    pub placement: Placement,
}

/// Workspace navigation and compact header controls are separate caller-owned intents.
#[derive(Default)]
pub struct RibbonResponse {
    pub workspace: Option<WorkspacePick>,
    pub quick: Option<usize>,
    /// Paint slot the host completes with the current pane grids after drawing the workspace.
    pub background: Option<RibbonBackground>,
}

/// A reserved paint slot below the ribbon cards. The host completes it after the pane cameras
/// have processed this frame's navigation, keeping the exposed grid aligned with each canvas.
pub struct RibbonBackground {
    /// The whole ribbon band, including the exposed margins around its cards.
    pub area: Rect,
    painter: egui::Painter,
    shape: egui::layers::ShapeIdx,
}

impl RibbonBackground {
    /// Complete the backdrop beneath the existing ribbon controls without registering input.
    pub fn paint(self, shapes: Vec<Shape>) {
        self.painter.set(self.shape, Shape::Vec(shapes));
    }
}

/// Put a tabbed command strip above the viewport. `tabs` names the ribbon tabs and `tab` is the
/// index of the one shown; clicking another tab selects it. `add` draws the actions of the tab
/// shown, whose index it receives, including a tab clicked this frame. The workspace tile yields
/// space to longer tab lists at narrow widths; actions scroll with a gutter below the captions.
/// Narrow headers tighten and wrap tabs; quick controls yield when space runs out and on View,
/// which holds their full controls. Extra header rows reserve space above the action strip.
///
/// The workspace tile returns a workspace/placement request. Picking the current row closes
/// the picker; the split and window icons within its card create another view.
pub fn ribbon(
    ui: &mut Ui,
    workspace: &str,
    tabs: &[&str],
    tab: &mut usize,
    workspaces: Workspaces<'_>,
    quick: &[RibbonTool<'_>],
    add: impl FnOnce(&mut Ui, usize),
) -> RibbonResponse {
    let mut add = Some(add);
    ribbon_with_controls(ui, workspace, tabs, tab, RibbonLayout { workspaces, quick, pinned_fields: 0, controls_label: "" }, |ui, section| {
        if let RibbonSection::Commands(shown) = section
            && let Some(add) = add.take()
        {
            add(ui, shown);
        }
    })
}

/// Host-owned options for the command ribbon and its persistent editing fields.
pub struct RibbonLayout<'a> {
    pub workspaces: Workspaces<'a>,
    pub quick: &'a [RibbonTool<'a>],
    /// Number of standard-width fields kept outside the tab's scrolling commands.
    pub pinned_fields: usize,
    /// Noninteractive section caption for the persistent editing fields; empty hides it.
    pub controls_label: &'a str,
}

/// The command strip changes with its tab; editing fields keep their own identity and position.
pub enum RibbonSection {
    Commands(usize),
    /// Narrow windows place compact, wrapping fields below the command strip.
    Controls {
        compact: bool,
    },
}

/// Compose a tab ribbon with persistent fields at the right. At narrow widths fields wrap
/// below the commands, staying visible while only the tab's command strip scrolls horizontally.
pub fn ribbon_with_controls(
    ui: &mut Ui,
    workspace: &str,
    tabs: &[&str],
    tab: &mut usize,
    layout: RibbonLayout<'_>,
    mut add: impl FnMut(&mut Ui, RibbonSection),
) -> RibbonResponse {
    let RibbonLayout { workspaces, quick, pinned_fields, controls_label } = layout;
    let t = Tokens::get();
    let wide = ui.available_width() >= if tabs.len() > 2 { 600.0 } else { 480.0 };
    let tab_gap = if wide { t.space_6 } else { t.space_2 };
    let tab_padding = if wide { t.space_8 } else { t.space_4 };
    let tab_width = tabs
        .iter()
        .map(|title| {
            ui.painter().layout_no_wrap((*title).to_owned(), FontId::proportional(t.font_body), t.text).size().x + 2.0 * tab_padding + tab_gap
        })
        .sum::<f32>();
    let tab_room = (ui.available_width() - if wide { WORKSPACE_WIDTH + t.space_8 * 5.0 } else { t.space_8 * 4.0 }).max(1.0);
    let extra_tab_rows = (tab_width / tab_room).ceil().max(1.0) - 1.0;
    let field_width = t.control_height * 6.0;
    let fixed_width = pinned_fields as f32 * field_width + pinned_fields.saturating_sub(1) as f32 * t.space_8;
    let side_controls = pinned_fields > 0 && tab_room >= fixed_width + 240.0;
    let compact_width = t.control_height * 5.5;
    let columns = ((tab_room + t.space_8) / (compact_width + t.space_8)).floor().max(1.0) as usize;
    let control_rows = pinned_fields.div_ceil(columns);
    let compact_height = t.control_height + t.font_small + t.space_8;
    let controls_height = control_rows as f32 * (compact_height + 1.0);
    let extra_controls = if pinned_fields > 0 && !side_controls { controls_height + t.space_24 } else { 0.0 };
    let height = RIBBON_HEIGHT + if wide { 0.0 } else { t.space_24 } + extra_tab_rows * t.control_height + extra_controls;
    let painter = ui.painter().clone();
    let background = painter.add(Shape::Noop);
    let panel = Panel::top("workspace_ribbon")
        .exact_size(height + 2.0 * t.space_6)
        .resizable(false)
        .show_separator_line(false)
        .frame(Frame::new().fill(Color32::TRANSPARENT).inner_margin(Margin::symmetric(t.space_8 as i8, t.space_6 as i8)))
        .show(ui, |ui| {
            let mut picked = None;
            let mut quick_pick = None;
            ui.horizontal_top(|ui| {
                if wide {
                    // Read before the tile can toggle it, so the picker knows the frame it opens on.
                    let was_open = egui::Popup::is_id_open(ui.ctx(), picker_id(ui.ctx()));
                    let tile = card(ui.ctx(), t.space_8)
                        .show(ui, |ui| {
                            workspace_tile(
                                ui,
                                workspace,
                                workspaces.entries.iter().find(|entry| entry.current).map_or(WorkspaceIcon::Schematic, |entry| entry.icon),
                                height,
                            )
                        })
                        .inner;
                    picked = picker(ui, &tile, workspaces, !was_open);
                }

                card(ui.ctx(), t.space_4).show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width());
                        ui.set_min_height(height - 2.0 * (t.space_4 + 1.0));
                        if !wide {
                            let was_open = egui::Popup::is_id_open(ui.ctx(), picker_id(ui.ctx()));
                            let tile = crate::controls::small_button(ui, format!("{workspace}    "));
                            let c = tile.rect.right_center() - vec2(t.space_6, 0.0);
                            ui.painter()
                                .add(Shape::line(vec![c + vec2(-3.0, -1.5), c + vec2(0.0, 1.5), c + vec2(3.0, -1.5)], Stroke::new(1.0, t.text_dim)));
                            tile.widget_info(|| WidgetInfo::labeled(Role::Button, true, "Choose workspace"));
                            picked = picker(ui, &tile, workspaces, !was_open);
                        }
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = tab_gap;
                            for (index, title) in tabs.iter().enumerate() {
                                if ribbon_tab(ui, title, index == *tab, wide).clicked() {
                                    *tab = index;
                                }
                            }
                            if wide {
                                crate::controls::enabled_button(ui, false, "+").on_disabled_hover_text("Ribbon customization is not available yet.");
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let show_quick = tabs.get(*tab) != Some(&"View")
                                    && ui.available_width() >= quick.len() as f32 * (t.space_24 + ui.spacing().item_spacing.x);
                                for (index, tool) in quick.iter().enumerate().rev().filter(|_| show_quick) {
                                    let (_, rect) = ui.allocate_space(vec2(t.space_24, t.space_24));
                                    let response = crate::controls::glyph_target(
                                        ui,
                                        ui.scope_id().with(("ribbon_quick", tool.description)),
                                        rect,
                                        tool.description,
                                        tool.enabled,
                                        tool.active,
                                    );
                                    response.widget_info(|| {
                                        WidgetInfo::selected(Role::Button, tool.enabled && ui.is_enabled(), tool.active, tool.description)
                                    });
                                    paint_icon(ui.painter(), tool.icon, rect.center(), t.space_16, t, tool.enabled);
                                    if response.on_hover_text(tool.description).clicked() {
                                        quick_pick = Some(index);
                                    }
                                }
                            });
                        });
                        ui.separator();
                        ui.spacing_mut().scroll.floating = false;
                        ui.spacing_mut().scroll.bar_width = RIBBON_SCROLL_GUTTER;
                        ui.spacing_mut().scroll.bar_inner_margin = 0.0;
                        ui.spacing_mut().scroll.bar_outer_margin = 0.0;
                        let width = ui.available_width();
                        let commands_width = if side_controls { (width - fixed_width - t.space_12).max(1.0) } else { width };
                        let (_, row) = ui.allocate_space(vec2(width, solidcraft_ui_kit::ribbon::GROUP_HEIGHT + RIBBON_SCROLL_GUTTER));
                        let commands = Rect::from_min_size(row.min, vec2(commands_width, row.height()));
                        let mut command_ui = ui
                            .new_child(egui::UiBuilder::new().id_salt("ribbon_command_host").max_rect(commands).layout(Layout::top_down(Align::Min)));
                        command_ui.set_clip_rect(ui.clip_rect().intersect(commands));
                        ScrollArea::horizontal().id_salt("ribbon_actions").auto_shrink([false, true]).show(&mut command_ui, |ui| {
                            describe_scroll_viewport(ui);
                            ui.horizontal_top(|ui| add(ui, RibbonSection::Commands(*tab)));
                        });
                        if pinned_fields > 0 {
                            let controls = if side_controls {
                                ui.painter().vline(commands.right() + t.space_6, row.top()..=row.bottom(), Stroke::new(1.0, t.border));
                                Rect::from_min_size(pos2(row.right() - fixed_width, row.top()), vec2(fixed_width, row.height()))
                            } else {
                                ui.add_space(t.space_8);
                                ui.allocate_space(vec2(width, controls_height)).1
                            };
                            let mut controls_ui = ui.new_child(
                                egui::UiBuilder::new()
                                    .id_salt("ribbon_controls")
                                    .max_rect(controls)
                                    .layout(Layout::left_to_right(Align::Min).with_main_wrap(!side_controls)),
                            );
                            controls_ui.set_clip_rect(ui.clip_rect().intersect(controls));
                            controls_ui.spacing_mut().item_spacing.x = t.space_8;
                            add(&mut controls_ui, RibbonSection::Controls { compact: !side_controls });
                            if !controls_label.is_empty() {
                                let fields = controls_ui.min_rect();
                                if side_controls {
                                    use solidcraft_ui_kit::ribbon as shared;
                                    let center = row.top() + shared::GROUP_HEIGHT - shared::SECTION_HEIGHT / 2.0 - (shared::CARD_INSET - 1.0);
                                    shared::text(
                                        ui,
                                        pos2(fields.center().x, center),
                                        controls_label,
                                        solidcraft_theme::Tokens::get().font_caption,
                                        t.text_dim,
                                        false,
                                    );
                                } else {
                                    ui.painter().text(
                                        pos2(fields.center().x, fields.bottom() + t.space_4),
                                        Align2::CENTER_TOP,
                                        controls_label,
                                        FontId::proportional(t.font_small),
                                        t.text_dim,
                                    );
                                }
                            }
                        }
                    });
                });
            });
            (picked, quick_pick)
        });
    RibbonResponse {
        workspace: panel.inner.0,
        quick: panel.inner.1,
        background: Some(RibbonBackground { area: panel.response.rect, painter, shape: background }),
    }
}

/// Draw the workspace tile: the accent rule, icon, workspace name and a "Workspace" caption with
/// a caret that points up while the picker is open. The tile is a button that toggles the picker.
fn workspace_tile(ui: &mut Ui, workspace: &str, icon: WorkspaceIcon, height: f32) -> Response {
    let t = Tokens::get();
    let inset = t.space_8 + 1.0;
    let (rect, response) = ui.allocate_exact_size(
        vec2(if workspace.len() > 10 { WORKSPACE_WIDTH + 32.0 } else { WORKSPACE_WIDTH } - 2.0 * inset, height - 2.0 * inset),
        Sense::click(),
    );
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, "Choose workspace"));
    let open = egui::Popup::is_id_open(ui.ctx(), picker_id(ui.ctx()));
    let center = rect.center();
    let p = ui.painter();
    if open || response.hovered() || response.has_focus() {
        p.rect_filled(rect.expand(inset - 1.0), t.radius_md, t.hover);
    }
    p.hline(rect.x_range(), rect.top() - t.space_6, Stroke::new(t.space_2, t.accent));
    paint_workspace_glyph(p, icon, center - vec2(0.0, t.space_16), t.control_height + t.space_2, t, true);
    p.text(center + vec2(0.0, t.space_12), Align2::CENTER_CENTER, workspace, fonts::interface_heading(ui, t.font_button), t.text);
    let caption = p.layout_no_wrap("Workspace".to_owned(), FontId::proportional(t.font_small), t.text_dim);
    let caret = t.space_4 + t.space_3;
    let caption_center = center + vec2(-caret / 2.0, t.space_24 + t.space_8);
    let tip = caption_center + vec2(caption.size().x / 2.0 + caret, 0.0);
    p.galley(caption_center - caption.size() / 2.0, caption, t.text_dim);
    let direction = if open { -1.0 } else { 1.0 };
    let arm = t.space_3;
    p.add(Shape::line(
        vec![tip + vec2(-arm, -arm / 2.0 * direction), tip + vec2(0.0, arm / 2.0 * direction), tip + vec2(arm, -arm / 2.0 * direction)],
        Stroke::new(1.2, t.text_dim),
    ));
    response.on_hover_text("Choose workspace")
}

/// Keep a picker in its native viewport when several hosts draw the same ribbon.
fn picker_id(ctx: &egui::Context) -> egui::Id {
    egui::Id::unique(("workspace_picker", ctx.viewport_id()))
}

/// Show the workspace picker under the tile while it's open. Clicking an available row, or
/// Enter or Space for the current one, closes it; a planned row never responds. `opening` is
/// true on the frame the tile opens it, when Enter or Space is the key that opened it and must
/// not also pick a row. Returns the requested workspace and placement, including new views of
/// the current workspace.
fn picker(ui: &Ui, tile: &Response, workspaces: Workspaces<'_>, opening: bool) -> Option<WorkspacePick> {
    let t = Tokens::get();
    // SolidCraft's picker width, kept inside narrow windows.
    let width =
        (t.form_card_width + t.form_label_width * 2.0 + t.space_24 + t.space_16 + t.space_6).min(ui.ctx().content_rect().width() - t.space_24);
    let height = (ui.ctx().content_rect().bottom() - tile.rect.bottom() - t.space_4).max(160.0);
    egui::Popup::from_toggle_button_response(tile)
        .id(picker_id(ui.ctx()))
        .kind(egui::PopupKind::Menu)
        .align(egui::RectAlign::BOTTOM_START)
        .align_alternatives(&[])
        .gap(t.space_4)
        .width(width)
        .layout(Layout::top_down(Align::Min))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .frame(crate::menu::frame().inner_margin(Margin::same((t.space_8 + 1.0) as i8)))
        .show(|ui| picker_rows(ui, workspaces, width, height, opening))
        .and_then(|shown| shown.inner)
}

/// The picker's heading, rows and footer. Arrows cycle available workspace rows; Tab can focus
/// placement actions. Enter and Space activate the focused choice (or current row), except on
/// the opening frame. Consumed keys never reach the canvas underneath.
fn picker_rows(ui: &mut Ui, workspaces: Workspaces<'_>, width: f32, height: f32, opening: bool) -> Option<WorkspacePick> {
    let t = Tokens::get();
    ui.set_width(width);
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.add_space(t.space_4);
    ui.label(RichText::new("WORKSPACE").font(fonts::interface_heading(ui, t.font_small)).color(t.text_dim));
    ui.add_space(t.space_8);
    // Keys stay in the picker so they don't reach the canvas underneath.
    let (navigate, activate) = ui.input_mut(|i| {
        let navigate = if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
            Some(true)
        } else if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
            Some(false)
        } else {
            None
        };
        let activate = i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) || i.consume_key(egui::Modifiers::NONE, egui::Key::Space);
        (navigate, activate && !opening)
    });
    let mut picked = None;
    let mut choices = Vec::new();
    ScrollArea::vertical().id_salt("workspace_rows").max_height((height - 116.0).max(60.0)).show(ui, |ui| {
        for (index, entry) in workspaces.entries.iter().enumerate() {
            let (response, actions) = workspace_row(ui, entry, width, workspaces.native_windows);
            if entry.tabs.is_some() && response.clicked() {
                if !entry.current {
                    picked = Some(WorkspacePick { index, placement: Placement::Here });
                }
                ui.close();
            }
            if entry.tabs.is_some() {
                choices.push((WorkspacePick { index, placement: Placement::Here }, response));
                for (placement, button) in actions {
                    if button.clicked() {
                        picked = Some(WorkspacePick { index, placement });
                        ui.close();
                    }
                    choices.push((WorkspacePick { index, placement }, button));
                }
            }
        }
    });
    let current = workspaces.entries.iter().position(|entry| entry.current);
    let focused = choices.iter().find(|(_, response)| response.has_focus());
    if let Some(forward) = navigate {
        // egui computes directional focus before widgets consume keys; this picker owns that move.
        ui.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
        let rows: Vec<_> = choices.iter().filter(|(pick, _)| pick.placement == Placement::Here).collect();
        if !rows.is_empty() {
            let from = focused.map(|(pick, _)| pick.index).or(current);
            let position = rows.iter().position(|(pick, _)| Some(pick.index) == from).unwrap_or(0);
            let next = if forward { (position + 1) % rows.len() } else { (position + rows.len() - 1) % rows.len() };
            rows[next].1.request_focus();
            rows[next].1.scroll_to_me(None);
        }
    }
    if activate
        && let Some((pick, _)) = focused.or_else(|| choices.iter().find(|(pick, _)| Some(pick.index) == current && pick.placement == Placement::Here))
    {
        if pick.placement != Placement::Here || Some(pick.index) != current {
            picked = Some(*pick);
        }
        ui.close();
    }
    ui.add_space(t.space_6);
    ui.separator();
    ui.add_space(t.space_6);
    ui.add(Label::new(RichText::new(workspaces.footer).size(t.font_small).color(t.text_dim)).wrap());
    ui.add_space(t.space_4);
    picked
}

/// Draw one workspace row, as SolidCraft's picker does: an accent bar, the icon tile, the title
/// with its Current or Planned badge, the description, and the tabs for available workspaces.
/// Planned rows are dimmed, only respond to hover, and report themselves disabled. Available
/// rows reserve separate hit targets for in-card split/window icons; those never activate the row.
/// Icon tooltips leave room below the button for the pointer's visible cursor.
fn workspace_row(ui: &mut Ui, entry: &Workspace<'_>, width: f32, native: bool) -> (Response, Vec<(Placement, Response)>) {
    let t = Tokens::get();
    let available = entry.tabs.is_some();
    let accent = if available { t.accent } else { t.text_dim };
    let soft = t.panel.lerp_to_gamma(accent, 0.10);
    let row_height = if available { t.space_24 * 3.0 } else { t.space_24 * 2.0 + t.space_12 };
    let (rect, allocated) = ui.allocate_exact_size(vec2(width, row_height), Sense::hover());
    let action_count = if available && entry.multiple_views { if native { 2 } else { 1 } } else { 0 };
    let action_width = action_count as f32 * (t.space_24 + t.space_4);
    let text_right = rect.right() - t.space_8 - action_width;
    let body = Rect::from_min_max(rect.min, pos2(text_right, rect.bottom()));
    let response = ui.interact(body, allocated.id.with("workspace"), if available { Sense::click() } else { Sense::hover() });
    response.widget_info(|| WidgetInfo::selected(Role::Button, available, entry.current, entry.title));
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    if entry.current || (available && (response.hovered() || response.has_focus())) {
        painter.rect_filled(rect, t.control_radius, soft);
    }
    if available && (response.hovered() || response.has_focus()) {
        painter.rect_stroke(rect.shrink(0.5), t.control_radius, Stroke::new(1.0, accent), egui::StrokeKind::Inside);
    }
    let bar = Rect::from_min_max(rect.min + vec2(t.space_2, t.space_6 + 1.0), pos2(rect.left() + t.space_6, rect.bottom() - t.space_6 - 1.0));
    painter.rect_filled(bar, t.radius_sm / 2, accent);
    let side = t.space_24 + t.space_16 + t.space_4;
    let tile = Rect::from_center_size(pos2(rect.left() + 40.0, rect.center().y), vec2(side, side));
    painter.rect_filled(tile, t.radius_md, if available { soft } else { t.panel_header });
    paint_workspace_glyph(&painter, entry.icon, tile.center(), side - 2.0 * (t.space_6 + 1.0), t, available);

    let ink = if available { t.text } else { t.text_dim };
    let x = rect.left() + 74.0;
    let y = rect.top() + 17.0;
    let title = painter.layout_no_wrap(entry.title.to_owned(), fonts::interface_heading(ui, t.font_heading), ink);
    let badge_x = x + title.size().x + t.space_8 + 1.0;
    painter.galley(pos2(x, y - title.size().y / 2.0), title, ink);
    if entry.current || !available {
        let (fill, ink) = if entry.current { (t.badge, t.badge_ink) } else { (soft, accent) };
        let badge = painter.layout_no_wrap(if entry.current { "Current" } else { "Planned" }.into(), FontId::proportional(t.font_small), ink);
        let badge_rect = Rect::from_min_size(pos2(badge_x, y - t.space_8), badge.size() + vec2(t.space_12, t.space_3));
        painter.rect_filled(badge_rect, t.radius_lg, fill);
        painter.galley(badge_rect.min + vec2(t.space_6, t.space_3 / 2.0), badge, ink);
    }
    let line = |text: &str, y: f32| {
        let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), FontId::proportional(t.font_body), ink);
        job.wrap.max_width = (text_right - x - t.space_6).max(0.0);
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = false;
        painter.galley(pos2(x, y), painter.layout_job(job), ink);
    };
    line(entry.description, rect.top() + 29.0);
    if let Some(tabs) = entry.tabs {
        line(tabs, rect.top() + 47.0);
    }
    if available && response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let mut actions = Vec::new();
    if available {
        for (index, (placement, icon, label)) in [
            (Placement::Beside, ToolIcon::SplitView, format!("Open {} beside current view", entry.title)),
            (Placement::Window, ToolIcon::EditorWindow, format!("Open {} in new window", entry.title)),
        ]
        .into_iter()
        .take(action_count)
        .enumerate()
        {
            let target = Rect::from_min_size(
                pos2(text_right + t.space_4 + index as f32 * (t.space_24 + t.space_4), rect.center().y - t.space_12),
                vec2(t.space_24, t.space_24),
            );
            let action = ui.interact(target, allocated.id.with(index), Sense::click());
            action.widget_info(|| WidgetInfo::labeled(Role::Button, true, &label));
            if action.hovered() || action.has_focus() {
                painter.rect_filled(target, t.control_radius, t.hover);
            }
            paint_icon(&painter, icon, target.center(), t.space_16, t, true);
            egui::Tooltip::for_enabled(&action).gap(t.space_24 + t.space_8).show(|ui| {
                ui.label(&label);
            });
            actions.push((placement, action));
        }
    }
    (response, actions)
}

/// Draw one ribbon tab as PCBcraft Next does: a quiet 22 point button in regular text, with the
/// selection fill on the tab shown.
fn ribbon_tab(ui: &mut Ui, title: &str, selected: bool, wide: bool) -> Response {
    let t = Tokens::get();
    let galley = ui.painter().layout_no_wrap(title.to_owned(), FontId::proportional(t.font_body), t.text);
    let size = vec2(galley.size().x + 2.0 * if wide { t.space_8 } else { t.space_4 }, t.space_24);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| WidgetInfo::selected(Role::Tab, true, selected, title));
    if ui.is_rect_visible(rect) {
        let shared = solidcraft_theme::Tokens::get();
        let style = solidcraft_ui_kit::ribbon::tab_style(
            &shared,
            solidcraft_theme::TabAccent::current(ui.ctx()),
            false,
            selected,
            response.hovered() || response.has_focus(),
        );
        let p = ui.painter();
        p.rect_filled(rect, t.radius_sm, style.fill);
        p.galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, style.ink);
    }
    response
}

/// Draw one Browser row with a stable `id`, so a press and its release stay on the same row
/// while rows above it open or close. The disclosure arrow is its own target, labelled
/// "Expand …" or "Collapse …": clicking it folds the row without clicking the row. Only the
/// part right of the arrow highlights when the row is selected. The kit draws the disclosure,
/// while the row owns the PCBcraft glyph and any independent layer color target.
pub fn tree_row(ui: &mut Ui, id: egui::Id, row: &TreeRow<'_>) -> TreeResponse {
    let t = Tokens::get();
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), TREE_ROW_HEIGHT), Sense::hover());
    let left = rect.left() + row.depth as f32 * TREE_INDENT;
    let fold_rect = Rect::from_min_max(pos2(left, rect.top()), pos2(left + TREE_FOLD_WIDTH, rect.bottom()));
    let body = Rect::from_min_max(pos2(fold_rect.right(), rect.top()), rect.max);
    let editable = matches!(row.icon, Some(TreeIcon::Layer { editable: true, .. }));
    let icon_rect = Rect::from_min_max(body.min, pos2(body.left() + TREE_LABEL_LEFT, body.bottom()));
    let activation = if editable { Rect::from_min_max(pos2(icon_rect.right(), body.top()), body.max) } else { body };
    let response = ui.interact(activation, id, Sense::click());
    let icon_response = editable.then(|| {
        let response = ui.interact(icon_rect, id.with("layer_color"), Sense::click());
        response.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), format!("Change color for layer {}", row.label)));
        response
    });
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, row.selected, row.accessibility_label.unwrap_or(row.label)));
    let fold = row.fold.map(|open| {
        let action = if open { "Collapse" } else { "Expand" };
        let mut arrow = ui.new_child(egui::UiBuilder::new().id_salt(id.with("fold")).max_rect(fold_rect));
        arrow.set_clip_rect(arrow.clip_rect().intersect(fold_rect));
        solidcraft_ui_kit::panel_disclosure_button_at(&mut arrow, fold_rect, open, &format!("{action} {}", row.label))
    });
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        if row.selected {
            p.rect_filled(body, t.radius_sm, t.accent_soft);
        } else if response.hovered() || response.has_focus() {
            p.rect_filled(body, t.radius_sm, t.hover);
        }
        if icon_response.as_ref().is_some_and(|response| response.hovered() || response.has_focus()) {
            p.rect_filled(icon_rect, t.radius_sm, t.hover);
        }
        let cy = rect.center().y;
        if let Some(icon) = row.icon {
            paint_tree_icon(p, icon, pos2(body.left() + TREE_ICON_CENTER, cy), TREE_ICON, t);
        }
        let x = body.left() + TREE_LABEL_LEFT;
        let font = if row.selected { fonts::interface_heading(ui, t.font_body) } else { FontId::proportional(t.font_body) };
        let galley = elided(p, row.label, font, t.text, (body.right() - t.space_6 - x).max(0.0));
        p.galley(pos2(x, cy - galley.size().y / 2.0), galley, t.text);
    }
    let fold_clicked = fold.is_some_and(|fold| fold.clicked());
    let response = if row.tip.is_empty() { response } else { response.on_hover_text(row.tip) };
    let icon_clicked = icon_response.as_ref().is_some_and(|response| response.clicked());
    if let Some(icon) = icon_response {
        icon.on_hover_text(format!("Change color for layer {}", row.label));
    }
    TreeResponse { clicked: response.clicked() && !fold_clicked && !icon_clicked, icon_clicked, fold: fold_clicked, response }
}

/// Lay out one line of text no wider than `width`, ending in an ellipsis when it is cut short.
fn elided(p: &egui::Painter, text: &str, font: FontId, color: Color32, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), font, color, width);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('\u{2026}');
    p.layout_job(job)
}

/// Draw tree glyphs using shared workspace icons, qualified artwork and original layer markers.
fn paint_tree_icon(p: &egui::Painter, icon: TreeIcon, center: Pos2, size: f32, t: Tokens) {
    match icon {
        TreeIcon::Layer { color, active, .. } => {
            p.circle_filled(center, t.space_4, color);
            p.circle_stroke(center, t.space_4, Stroke::new(1.0, t.border));
            if active {
                let c = center - vec2(TREE_LABEL_LEFT - t.space_3, 0.0);
                p.add(Shape::convex_polygon(
                    vec![c + vec2(-t.space_3, -t.space_4), c + vec2(t.space_3, 0.0), c + vec2(-t.space_3, t.space_4)],
                    t.accent,
                    Stroke::NONE,
                ));
            }
        }
        TreeIcon::Board => paint_workspace_glyph(p, WorkspaceIcon::Layout, center, size, t, true),
        TreeIcon::Folder => paint_folder(p, center, size, t.icon_line),
        TreeIcon::Sheet => {
            let at = |x: f32, y: f32| center + vec2(x - 32.0, y - 32.0) * (size / 64.0);
            let stroke = |width: f32, color| Stroke::new(width * size / 64.0, color);
            let outline = [(15.0, 6.0), (38.0, 6.0), (50.0, 18.0), (50.0, 58.0), (15.0, 58.0), (15.0, 6.0)];
            p.add(Shape::line(outline.iter().map(|&(x, y)| at(x, y)).collect(), stroke(4.0, t.icon_line)));
            p.add(Shape::line(vec![at(38.0, 6.0), at(38.0, 20.0), at(50.0, 20.0)], stroke(4.0, t.icon_line)));
            for y in [33.0, 43.0] {
                p.line_segment([at(24.0, y), at(41.0, y)], stroke(3.0, t.icon_active_2));
            }
        }
        TreeIcon::Tool(tool) => paint_icon(p, tool, center, size, t, true),
    }
}

/// PCBcraft Next's filled folder glyph, drawn as two convex parts: the tab and the body.
fn paint_folder(p: &egui::Painter, center: Pos2, size: f32, color: Color32) {
    let at = |x: f32, y: f32| center + vec2(x - 32.0, y - 32.0) * (size / 64.0);
    let tab = [(8.0, 18.5), (8.0, 15.0), (10.5, 12.5), (26.0, 12.5), (32.0, 18.5)];
    p.add(Shape::convex_polygon(tab.iter().map(|&(x, y)| at(x, y)).collect(), color, Stroke::NONE));
    let body = Rect::from_min_max(at(8.0, 18.0), at(56.0, 51.5));
    p.rect_filled(body, (2.5 * size / 64.0) as u8, color);
}

/// One action in a ribbon group: its button, and its row in the group's menu.
#[derive(Clone, Copy, Debug)]
pub struct RibbonTool<'a> {
    pub icon: ToolIcon,
    pub label: &'a str,
    /// The button's accessible name and tooltip, such as "Add symbol (A)".
    pub description: &'a str,
    /// The key shown in the group's menu. Empty for none; the caller handles the key itself.
    pub shortcut: &'a str,
    pub enabled: bool,
    /// Drawn pressed: a tool in use, or a panel that is shown.
    pub active: bool,
    /// A checked menu row declares whether it selects a choice or flips an independent state.
    pub menu_check: Option<CheckKind>,
}

/// Lay out a ribbon group as PCBcraft Next does: the tools' buttons side by side, and under them
/// the group's caption with a caret. The caption is a button that opens a menu listing the
/// same tools with their glyphs, shortcuts and, for toggles, check marks. Returns the index of
/// the tool picked from either this frame.
pub fn ribbon_group(ui: &mut Ui, title: &str, tools: &[RibbonTool<'_>]) -> Option<usize> {
    ribbon_group_with_menu(ui, title, tools, &[])
}

/// Center a final command group in the space left before the persistent editing controls.
/// Narrow or scrolled ribbons retain the natural group width and their ordinary overflow.
pub fn ribbon_group_fill(ui: &mut Ui, title: &str, tools: &[RibbonTool<'_>]) -> Option<usize> {
    ribbon_group_sized(ui, title, tools, &[], true)
}

/// Align an inline control and its non-interactive caption with neighboring ribbon groups.
/// The caller owns control state; chrome owns the group's height and caption metrics.
pub fn ribbon_control(ui: &mut Ui, title: &str, width: f32, add: impl FnOnce(&mut Ui)) {
    let t = Tokens::get();
    let height = RIBBON_TOOL_HEIGHT + RIBBON_CAPTION_GAP + t.font_small + t.space_4;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let body = Rect::from_min_size(rect.min + vec2(t.space_6, t.space_8), vec2(width - t.space_12, RIBBON_TOOL_HEIGHT - t.space_8));
    ui.scope_builder(egui::UiBuilder::new().max_rect(body), add);
    ui.painter().text(
        pos2(rect.center().x, rect.top() + RIBBON_TOOL_HEIGHT + RIBBON_CAPTION_GAP),
        Align2::CENTER_TOP,
        title,
        FontId::proportional(t.font_small),
        t.text_dim,
    );
}

/// Draw a ribbon group with additional menu-only options after a separator. Returned indices
/// address the visible tools first, then the extra entries; toggles use the shared checked row.
/// Buttons share a standard width and wrap labels; a wider caption grows its group evenly.
/// PCBcraft's surrounding layout supplies the separators between these individual groups.
pub fn ribbon_group_with_menu(ui: &mut Ui, title: &str, tools: &[RibbonTool<'_>], menu_tools: &[RibbonTool<'_>]) -> Option<usize> {
    ribbon_group_sized(ui, title, tools, menu_tools, false)
}

/// Reserve the natural tool width or the remaining visible command strip before centering
/// the shared group. The viewport bound prevents a scrolling UI from creating infinite space.
fn ribbon_group_sized(ui: &mut Ui, title: &str, tools: &[RibbonTool<'_>], menu_tools: &[RibbonTool<'_>], fill: bool) -> Option<usize> {
    use solidcraft_ui_kit::ribbon as shared;
    let t = Tokens::get();
    let caption_width = ui.painter().layout_no_wrap(title.to_owned(), FontId::proportional(t.font_small), t.text_dim).size().x + t.space_24;
    let count = tools.len().max(1) as f32;
    let button_width = RIBBON_TOOL_WIDTH.max((caption_width - 8.0 - tools.len().saturating_sub(1) as f32 * shared::BUTTON_GAP) / count);
    let widths = vec![button_width; tools.len()];
    let content_width = shared::panel_width(&widths);
    let natural_width = content_width.max(caption_width) + 5.0;
    let visible_right = ui.available_rect_before_wrap().right().min(ui.clip_rect().right());
    let width = if fill { natural_width.max(visible_right - ui.next_widget_position().x) } else { natural_width };
    let (_, rect) = ui.allocate_space(vec2(width, shared::GROUP_HEIGHT));
    // The shared row reserves four trailing points plus its separator. Keep that fitting
    // room outside the centered content, and clip its separator in this host composition.
    let content_left = rect.left() + (width - content_width) / 2.0;
    let shared_rect = Rect::from_min_size(pos2(content_left, rect.top()), vec2(content_width + 5.0, rect.height()));
    let prefix = format!("pcbcraft:ribbon:{title}:");
    let group = shared::Group {
        key: title,
        name: title,
        label: title,
        tip: title,
        buttons: tools
            .iter()
            .zip(&widths)
            .map(|(tool, &width)| shared::Button {
                id: tool.description,
                run: tool.description,
                label: tool.label,
                name: tool.description,
                icon: crate::ribbon_icons::HOST_PAINTED,
                size: shared::ButtonSize { width, min_width: width },
                enabled: tool.enabled,
                pressed: tool.active,
                exit: false,
                split: None,
                app_paints: false,
                covers: &[],
            })
            .collect(),
        cells: Vec::new(),
        commands: tools.iter().chain(menu_tools).map(|tool| tool.description).collect(),
        icon: None,
        enabled: true,
        open: false,
        running: None,
        more: Vec::new(),
    };
    let row = shared::Row {
        groups: vec![group],
        labels: true,
        accent: solidcraft_theme::TabAccent::current(ui.ctx()),
        exit: false,
        reveal: None,
        more_tip: "More tools",
        handles: shared::Handles {
            button: &[&format!("{prefix}button:")],
            button_rect: &format!("{prefix}button_rect:"),
            split: &format!("{prefix}split:"),
            cell: &[],
            collapsed: &format!("{prefix}collapsed:"),
            section: &format!("{prefix}section:"),
            section_target: &format!("{prefix}section_target:"),
            group: &format!("{prefix}group:"),
            running: &format!("{prefix}running:"),
            reveal: &format!("{prefix}reveal:"),
            more: &format!("{prefix}more:"),
            more_row: &format!("{prefix}more_row:"),
            collapsed_count: &format!("{prefix}collapsed_count:"),
            more_count: &format!("{prefix}more_count:"),
            trimmed_count: &format!("{prefix}trimmed_count:"),
            drag: &format!("{prefix}drag:"),
            drag_remove: &format!("{prefix}drag_remove:"),
        },
    };
    let mut group_ui = ui.new_child(egui::UiBuilder::new().id_salt(("ribbon_group", title)).max_rect(shared_rect));
    let mut clip = ui.clip_rect();
    clip.max.x = clip.max.x.min(content_left + content_width - 0.5);
    group_ui.set_clip_rect(clip);
    let output = shared::show(&mut group_ui, &row, shared_rect, &mut |_, _| false);
    let mut picked = output.actions.iter().find_map(|action| match action {
        shared::Action::Run { id, .. } => tools.iter().position(|tool| tool.description == id),
        _ => None,
    });
    for button in output.buttons {
        if let Some(tool) = tools.get(button.index) {
            paint_icon(ui.painter(), tool.icon, pos2(button.whole.center().x, button.whole.top() + 16.0), RIBBON_TOOL_ICON, t, tool.enabled);
            button.response.widget_info(|| WidgetInfo::selected(Role::Button, tool.enabled && ui.is_enabled(), tool.active, tool.description));
            button.response.on_hover_text(tool.description);
        }
    }
    if let Some(group) = output.groups.first() {
        let response = &group.label_response;
        response.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), format!("{title} menu")));
        egui::Popup::menu(response).frame(crate::menu::frame()).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            ui.set_min_width(t.control_height * 7.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            for (index, tool) in tools.iter().chain(menu_tools).enumerate() {
                if index == tools.len() && index > 0 {
                    ui.separator();
                }
                let mut item = crate::MenuItem::new(tool.label).tool_icon(tool.icon).shortcut(tool.shortcut).enabled(tool.enabled);
                if let Some(kind) = tool.menu_check {
                    item = item.with_check(kind, tool.active);
                }
                if item.show(ui) {
                    picked = Some(index);
                }
            }
        });
    }
    picked
}

/// Standard themed layer dropdown with a color patch in its closed field and every popup row.
/// Labels and color targets stay independent; selecting a row changes only the supplied value.
/// The returned response carries the trigger bounds and reports whether its value changed.
pub fn layer_choice<T: Copy + PartialEq>(ui: &mut Ui, label: &str, value: &mut T, options: &[(T, &str, Color32)]) -> Response {
    let before = *value;
    let selected = options.iter().find(|(option, _, _)| option == value);
    let selected_name = selected.map_or("Layer", |(_, name, _)| *name);
    let mut response = solidcraft_ui_kit::bounded_dropdown_menu(ui, label, "", ui.available_width(), |ui| {
        for (option, name, color) in options {
            let text = layer_choice_label(ui, name);
            let response = ui.selectable_value(value, *option, text);
            layer_choice_patch(ui, response.rect, *color);
        }
    });
    let t = Tokens::get();
    let text_rect = response.rect.shrink2(ui.spacing().button_padding);
    let mut text = layer_choice_label(ui, selected_name);
    text.wrap.max_width = (text_rect.width() - t.space_24).max(0.0);
    text.wrap.max_rows = 1;
    let galley = ui.painter().layout_job(text);
    ui.painter().with_clip_rect(text_rect).galley(pos2(text_rect.left(), text_rect.center().y - galley.size().y / 2.0), galley, t.text);
    if let Some((_, _, color)) = selected {
        layer_choice_patch(ui, response.rect, *color);
    }
    response.widget_info(|| {
        let mut info = WidgetInfo::labeled(Role::ComboBox, ui.is_enabled(), label);
        info.current_text_value = Some(selected_name.to_owned());
        info
    });
    if before != *value {
        response.mark_changed();
    }
    response
}

/// Reserve actual layout space for a swatch rather than padding the visible or accessible text.
fn layer_choice_label(ui: &Ui, name: &str) -> egui::text::LayoutJob {
    let t = Tokens::get();
    let mut job = egui::text::LayoutJob::simple(name.to_owned(), egui::TextStyle::Button.resolve(ui.style()), t.text, f32::INFINITY);
    if let Some(section) = job.sections.first_mut() {
        section.leading_space = t.space_12 + t.space_8;
    }
    job
}

/// Use the same palette and quiet border as the layer rows, with a rectangular toolbar key.
fn layer_choice_patch(ui: &Ui, rect: Rect, color: Color32) {
    let t = Tokens::get();
    let left = rect.left() + ui.spacing().button_padding.x;
    let patch = Rect::from_center_size(pos2(left + t.space_6, rect.center().y), vec2(t.space_12, t.space_12));
    ui.painter().rect_filled(patch, t.space_2 as u8, color);
    ui.painter().rect_stroke(patch, t.space_2 as u8, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
}

/// Place a standard themed field in the ribbon using the existing group height and caption.
/// Its own control opens any menu; the caption adds no second popup or interaction target.
/// Compact fields place the caption above the control for a wrapping persistent settings row.
/// The caption uses the rendered control's center, including any host-imposed size constraints.
pub fn ribbon_field(ui: &mut Ui, title: &str, compact: bool, add: impl FnOnce(&mut Ui) -> Response) -> Response {
    let t = Tokens::get();
    let width = t.control_height * if compact { 5.5 } else { 6.0 };
    let height =
        if compact { t.control_height + t.font_small + t.space_8 } else { RIBBON_TOOL_HEIGHT + RIBBON_CAPTION_GAP + t.font_small + t.space_4 };
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let field = if compact {
        Rect::from_min_max(rect.min + vec2(RIBBON_GROUP_PADDING, t.font_small + t.space_4), rect.max - vec2(RIBBON_GROUP_PADDING, 0.0))
    } else {
        Rect::from_min_max(
            rect.min + vec2(RIBBON_GROUP_PADDING, t.space_8),
            pos2(rect.right() - RIBBON_GROUP_PADDING, rect.top() + RIBBON_TOOL_HEIGHT),
        )
    };
    let mut field_ui = ui.new_child(egui::UiBuilder::new().id_salt(("ribbon_field", title)).max_rect(field).layout(Layout::top_down(Align::Center)));
    let result = add(&mut field_ui);
    ui.painter().text(
        pos2(result.rect.center().x, if compact { rect.top() } else { result.rect.bottom() + RIBBON_CAPTION_GAP }),
        Align2::CENTER_TOP,
        title,
        FontId::proportional(t.font_small),
        t.text_dim,
    );
    result
}

/// Tell assistive technology which part of a scroll area's content is on screen. Call it first
/// thing inside `ScrollArea::show`: the content `ui`'s accessibility node gets the visible
/// viewport, its clip rectangle, as its bounds and is marked as clipping its children. egui
/// leaves these containers without bounds, so without this a control scrolled under the edge
/// of the area still reports its full rectangle as if it were visible.
pub fn describe_scroll_viewport(ui: &Ui) {
    let viewport = ui.clip_rect();
    ui.ctx().accesskit_node_builder(ui.unique_id(), |node| {
        node.set_bounds(egui::accesskit::Rect {
            x0: viewport.min.x.into(),
            y0: viewport.min.y.into(),
            x1: viewport.max.x.into(),
            y1: viewport.max.y.into(),
        });
        node.set_clips_children();
    });
}

/// The quiet rule between two ribbon groups, in a slot as tall as the groups.
pub fn ribbon_separator(ui: &mut Ui) {
    let t = Tokens::get();
    let height = RIBBON_TOOL_HEIGHT + RIBBON_CAPTION_GAP + t.font_small + t.space_4;
    let (rect, _) = ui.allocate_exact_size(vec2(RIBBON_SEPARATOR_WIDTH, height), Sense::hover());
    ui.painter().vline(rect.center().x, rect.top() + t.space_2..=rect.bottom(), Stroke::new(1.0, t.border));
}

/// Paint a ribbon glyph in a menu row's leading column.
pub(crate) fn paint_menu_glyph(p: &egui::Painter, icon: ToolIcon, center: Pos2, size: f32) {
    paint_icon(p, icon, center, size, Tokens::get(), true);
}

/// Paint the sheet path beside the visible Browser, or at the canvas's left inset when the
/// Browser rectangle is not finite. The sheet count sits at the right, as in PCBcraft Next.
/// Paint after the canvas so the text overlays it without reserving space or taking input.
pub fn sheet_heading(p: &egui::Painter, canvas: Rect, browser: Rect, title: &str, count: &str) {
    let t = Tokens::get();
    canvas_heading(p, canvas, browser, Rect::NOTHING, title, count, [t.text, t.text_dim]);
}

/// Keep document labels clear of edge panels, using the displayed workspace's canvas ink.
pub fn canvas_heading(p: &egui::Painter, canvas: Rect, left_panel: Rect, right_panel: Rect, title: &str, count: &str, ink: [Color32; 2]) {
    let t = Tokens::get();
    let left = if left_panel.is_finite() { left_panel.right() + FLOATING_GAP } else { canvas.left() + FLOATING_GAP };
    let right = if right_panel.is_finite() { right_panel.left() - FLOATING_GAP } else { canvas.right() - FLOATING_GAP };
    let y = canvas.top() + FLOATING_GAP + FLOATING_HEADER / 2.0 + FLOATING_MARGIN;
    let count_width = p.layout_no_wrap(count.to_owned(), FontId::proportional(t.font_body), ink[1]).size().x;
    let width = (right - count_width - t.space_24 - left).max(0.0);
    let galley = elided(p, title, FontId::proportional(t.font_button + 1.0), ink[0], width);
    p.galley(pos2(left, y - galley.size().y / 2.0), galley, ink[0]);
    if right - count_width >= left {
        p.text(pos2(right, y), Align2::RIGHT_CENTER, count, FontId::proportional(t.font_body), ink[1]);
    }
}

/// Dock a card at the right edge of the canvas and keep its body scrollable.
/// Reserve at most two fifths of the workspace width and leave room below the
/// card for a floating launcher. Return whether Close was clicked.
pub fn right_card(ui: &mut Ui, id: &'static str, title: &str, bottom_clearance: f32, add: impl FnOnce(&mut Ui)) -> bool {
    docked_card(ui, Panel::right(id), title, bottom_clearance, true, add)
}

/// Dock a persistent card at the left edge of the canvas, with the same frame and width limit
/// as [`right_card`]. It has no Close button; the caller determines when it is visible.
pub fn left_card(ui: &mut Ui, id: &'static str, title: &str, add: impl FnOnce(&mut Ui)) {
    docked_card(ui, Panel::left(id), title, 0.0, false, add);
}

/// The docked card both sides share: a title and optional Close button over a scrolling body.
/// Return whether the button was clicked; persistent cards always return false.
fn docked_card(ui: &mut Ui, panel: Panel, title: &str, bottom_clearance: f32, closable: bool, add: impl FnOnce(&mut Ui)) -> bool {
    let t = Tokens::get();
    let width = (ui.available_width() * 0.4).min(260.0);
    let clearance = bottom_clearance.min(ui.available_height() * 0.4);
    let margin = Margin {
        right: if bottom_clearance > 0.0 { t.space_12 } else { t.space_8 } as i8,
        bottom: (t.space_8 + clearance) as i8,
        ..Margin::same(t.space_8 as i8)
    };
    panel
        .exact_size(width)
        .resizable(false)
        .show_separator_line(false)
        .frame(Frame::new().fill(t.canvas).inner_margin(margin))
        .show(ui, |ui| card(ui.ctx(), t.space_8).show(ui, |ui| card_contents(ui, title, f32::INFINITY, closable, add)).inner)
        .inner
}

/// Float a card over the top-left corner of `area`, normally the canvas, without taking any of
/// its space: the canvas keeps its full size and draws underneath. As in PCBcraft Next, the card
/// sits 12 points in from the corner, is at most 260 points or two fifths of the area wide, and
/// its header is the title with a chevron that collapses the card to that header. Open, it
/// sizes to its rows and scrolls once it would pass the area's bottom edge. It sits above the
/// canvas and below popups, so it takes its own clicks and wheel input. Returns the card's
/// rectangle and whether the header was clicked, which should toggle `collapsed`. Inherits
/// the host's enabled state, including when a project dialog lives in another native window.
pub fn floating_card(ui: &Ui, id: &'static str, title: &str, area: Rect, collapsed: bool, add: impl FnOnce(&mut Ui)) -> (Rect, bool) {
    floating_card_with_id(ui, egui::Id::unique(id), title, area, collapsed, add)
}

/// Render a floating card with an explicit host identity, allowing independent native windows.
pub fn floating_card_with_id(ui: &Ui, id: egui::Id, title: &str, area: Rect, collapsed: bool, add: impl FnOnce(&mut Ui)) -> (Rect, bool) {
    floating_card_aligned(ui, id, title, area, collapsed, false, add)
}

/// Minimum area height needed for a floating card's header, frame and edge clearance.
pub(crate) fn floating_card_min_height() -> f32 {
    let t = solidcraft_theme::Tokens::get();
    FLOATING_HEADER + 2.0 * (FLOATING_MARGIN + t.primary_border(t.border).width) + 2.0 * FLOATING_GAP
}

/// Anchor a workspace card to either canvas edge, retaining shared dimensions and collapse behavior.
/// Short areas omit the body, or the entire card when its header cannot fit, without changing
/// the caller's collapse preference. The body scrolls within the available height.
#[allow(clippy::too_many_arguments)]
pub fn floating_card_aligned(
    ui: &Ui,
    id: egui::Id,
    title: &str,
    area: Rect,
    collapsed: bool,
    right: bool,
    add: impl FnOnce(&mut Ui),
) -> (Rect, bool) {
    let width = (area.width() * 0.4).min(260.0);
    floating_card_width(ui, id, title, area, collapsed, right, width, add)
}

/// Anchor a card with an explicit preferred width while keeping its complete frame inside the
/// host. Uses the same collapse, scroll and pointer ownership behavior as other workspace cards.
#[allow(clippy::too_many_arguments)]
pub fn floating_card_width(
    ui: &Ui,
    id: egui::Id,
    title: &str,
    area: Rect,
    collapsed: bool,
    right: bool,
    width: f32,
    add: impl FnOnce(&mut Ui),
) -> (Rect, bool) {
    let t = Tokens::get();
    let width = width.min((area.width() - 2.0 * FLOATING_GAP).max(1.0));
    if area.height() < floating_card_min_height() {
        return (Rect::NOTHING, false);
    }
    let height = area.height() - 2.0 * FLOATING_GAP;
    let body_height = (height - 2.0 * (FLOATING_MARGIN + 1.0) - FLOATING_HEADER - t.space_8).max(0.0);
    let frame = floating_frame(ui.ctx());
    let shown = egui::Area::new(id)
        .enabled(ui.is_enabled())
        .order(egui::Order::Middle)
        .fixed_pos(pos2(if right { area.right() - width - FLOATING_GAP } else { area.left() + FLOATING_GAP }, area.top() + FLOATING_GAP))
        .constrain(false)
        .show(ui.ctx(), |ui| {
            // An area otherwise offers only last frame's size, so rows just unfolded would
            // scroll inside the old card instead of growing it.
            ui.set_max_height(height);
            frame
                .show(ui, |ui| {
                    ui.set_width(width - 2.0 * (FLOATING_MARGIN + 1.0));
                    ui.spacing_mut().item_spacing.y = t.space_2;
                    let toggle = card_header(ui, title, collapsed).clicked();
                    if !collapsed && body_height > 0.0 {
                        ui.separator();
                        ScrollArea::vertical().max_height(body_height).min_scrolled_height(0.0).auto_shrink([false, true]).show(ui, add);
                    }
                    toggle
                })
                .inner
        });
    (shown.response.rect, shown.inner)
}

/// The shared six-dot grip marks the title region that can move a floating or docked card.
pub(super) fn drag_grip(ui: &mut Ui, header: Rect) {
    let bounds = Rect::from_center_size(header.left_center() + vec2(7.0, 0.0), vec2(10.0, 18.0));
    let mut grip = ui.new_child(egui::UiBuilder::new().id_salt("panel_grip").max_rect(bounds));
    grip.set_clip_rect(ui.clip_rect().intersect(header));
    solidcraft_ui_kit::panel_grip(&mut grip);
}

/// A floating card's header: its title on the left and a chevron on the right, one button that
/// collapses or expands the card and is labelled for that.
fn card_header(ui: &mut Ui, title: &str, collapsed: bool) -> Response {
    let t = Tokens::get();
    let (id, rect) = ui.allocate_space(vec2(ui.available_width(), FLOATING_HEADER));
    let response = panel_header_control(ui, id, rect, title, false);
    let action = if collapsed { "Expand" } else { "Collapse" };
    response.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), format!("{action} {title}")));
    ui.ctx().accesskit_node_builder(response.id, |node| node.set_expanded(!collapsed));
    let icon = Rect::from_center_size(rect.right_center() - vec2(10.0, 0.0), vec2(12.0, 12.0));
    if collapsed {
        solidcraft_ui_kit::icons::paint_ink(ui.painter(), icon, "chevron", t.text_dim);
    } else {
        solidcraft_ui_kit::icons::paint_ink_flipped(ui.painter(), icon, "chevron", t.text_dim);
    }
    response.on_hover_text(if collapsed { "Expand panel" } else { "Collapse panel" })
}

/// Shared header interaction with host-owned identity and drag/collapse policy. Movable cards
/// use the compact catalog's grip spacing and a neutral hover, without a tab-selection tint.
pub(super) fn panel_header_control(ui: &mut Ui, id: egui::Id, rect: Rect, title: &str, drag: bool) -> Response {
    let response = solidcraft_ui_kit::panel_host_control(
        ui,
        id,
        rect,
        solidcraft_ui_kit::HostChrome {
            label: if drag { "" } else { title },
            icon: None,
            icon_only: false,
            attention: false,
            selected: false,
            enabled: ui.is_enabled(),
            drag,
            title: false,
            grip_only: false,
            rtl: false,
        },
    );
    if drag {
        drag_grip(ui, rect);
        let t = Tokens::get();
        let label = Rect::from_min_max(pos2((rect.left() + 18.0).min(rect.right()), rect.top()), rect.max);
        let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(label));
        painter.text(label.left_center(), Align2::LEFT_CENTER, title, fonts::interface_heading(ui, t.font_body), t.text);
    }
    response.on_hover_text(title)
}

/// A card's header, with its title and optional named Close button, over a body that scrolls
/// beyond `max_body_height`. Returns whether Close was clicked; persistent cards return false.
fn card_contents(ui: &mut Ui, title: &str, max_body_height: f32, closable: bool, add: impl FnOnce(&mut Ui)) -> bool {
    let t = Tokens::get();
    ui.set_width(ui.available_width());
    let header_size = vec2(ui.available_width(), t.control_height);
    let close = ui
        .allocate_ui_with_layout(header_size, Layout::right_to_left(Align::Center), |ui| {
            // Named after the card, so a screen reader can tell several cards apart.
            let close = closable && {
                let close = crate::controls::small_button(ui, "Close");
                close.widget_info(|| WidgetInfo::labeled(Role::Button, true, format!("Close {title}")));
                close.clicked()
            };
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add(Label::new(RichText::new(title).font(FontId::proportional(t.font_label))).truncate()).on_hover_text(title);
            });
            close
        })
        .inner;
    ui.separator();
    ScrollArea::vertical().max_height(max_body_height).auto_shrink([false, true]).show(ui, add);
    close
}

/// Shared bordered surface for the ribbon and inspector.
fn card(ctx: &egui::Context, inset: f32) -> Frame {
    solidcraft_ui_kit::Container::floating_card(solidcraft_ui_kit::Surface::Chrome)
        .frame(ctx, Tokens::get().panel, solidcraft_ui_kit::frame::Side::Left)
        .inner_margin(Margin::same(inset as i8))
}

/// Draw a workspace's icon, `size` points square, in the theme's icon colour roles. Planned
/// workspaces (`available` false) keep their colours at about a third of their strength, as
/// PCBcraft Next fades them. Geometry follows the 64-unit artwork: SolidCraft's schematic
/// sheet and render sphere, and PCBcraft Next's original board and simulation glyphs.
fn paint_workspace_glyph(p: &egui::Painter, icon: WorkspaceIcon, center: Pos2, size: f32, t: Tokens, available: bool) {
    let fade = |color: Color32| if available { color } else { color.gamma_multiply(0.35) };
    let [ink, face_1, face_2, face_3, active_1, active_2, active_3] =
        [t.icon_ink, t.icon_face_1, t.icon_face_2, t.icon_face_3, t.icon_active_1, t.icon_active_2, t.icon_active_3].map(fade);
    let scale = size / 64.0;
    let at = |x: f32, y: f32| center + vec2(x - 32.0, y - 32.0) * scale;
    let rect = |x1, y1, x2, y2| Rect::from_min_max(at(x1, y1), at(x2, y2));
    let stroke = |width: f32, color| Stroke::new(width * scale, color);
    let line = |points: &[(f32, f32)], stroke: Stroke| p.add(Shape::line(points.iter().map(|&(x, y)| at(x, y)).collect(), stroke));
    match icon {
        WorkspaceIcon::Schematic => {
            let outline = stroke(2.55, ink);
            let accent = stroke(3.0, active_2);
            p.rect(rect(10.0, 6.0, 54.0, 58.0), 2, face_1, outline, egui::StrokeKind::Middle);
            p.rect(rect(17.0, 14.0, 36.0, 30.0), 0, face_2, outline, egui::StrokeKind::Middle);
            p.rect(rect(33.0, 46.0, 48.0, 53.0), 1, active_1, stroke(2.2, ink), egui::StrokeKind::Middle);
            line(&[(17.0, 38.0), (36.0, 38.0)], accent);
            line(&[(17.0, 35.0), (17.0, 41.0)], accent);
            line(&[(36.0, 35.0), (36.0, 41.0)], accent);
            line(&[(44.0, 14.0), (44.0, 30.0)], accent);
        }
        WorkspaceIcon::Layout => {
            p.rect(rect(8.0, 9.0, 56.0, 55.0), 3, active_3, stroke(2.5, ink), egui::StrokeKind::Middle);
            let trace = stroke(2.5, active_1);
            line(&[(16.0, 23.0), (27.0, 23.0), (27.0, 39.0), (48.0, 39.0)], trace);
            line(&[(16.0, 44.0), (23.0, 44.0), (23.0, 17.0), (48.0, 17.0)], trace);
            p.rect(rect(31.0, 24.0, 44.0, 37.0), 1, ink, stroke(2.0, face_3), egui::StrokeKind::Middle);
            for (x, y) in [(16.0, 23.0), (48.0, 39.0), (16.0, 44.0), (48.0, 17.0)] {
                p.circle(at(x, y), 3.0 * scale, face_1, stroke(1.5, ink));
            }
        }
        WorkspaceIcon::Render => {
            // Shaded faces stacked toward the light, inside the sphere's outline.
            p.circle_filled(at(28.0, 38.0), 21.0 * scale, face_3);
            p.circle_filled(at(25.5, 35.0), 17.5 * scale, face_2);
            p.circle_filled(at(22.5, 31.5), 12.0 * scale, face_1);
            p.circle_filled(at(19.0, 27.0), 4.0 * scale, active_2);
            p.circle_stroke(at(28.0, 38.0), 21.0 * scale, stroke(2.55, ink));
            let spark = [(50.0, 5.0), (52.8, 11.2), (59.0, 14.0), (52.8, 16.8), (50.0, 23.0), (47.2, 16.8), (41.0, 14.0), (47.2, 11.2)];
            for pair in spark.windows(2).chain(std::iter::once(&[spark[7], spark[0]][..])) {
                if let [(x1, y1), (x2, y2)] = pair {
                    p.add(Shape::convex_polygon(vec![at(50.0, 14.0), at(*x1, *y1), at(*x2, *y2)], active_3, Stroke::NONE));
                }
            }
        }
        WorkspaceIcon::Simulate => {
            line(&[(9.0, 9.0), (9.0, 55.0), (57.0, 55.0)], stroke(3.0, face_3));
            // Two cubic arcs through the axis line, then a short rise.
            let cubic = |a: (f32, f32), b: (f32, f32), c: (f32, f32), d: (f32, f32)| {
                (0..=12).map(move |k| {
                    let u = k as f32 / 12.0;
                    let v = 1.0 - u;
                    let mix = |a: f32, b: f32, c: f32, d: f32| v * v * v * a + 3.0 * v * v * u * b + 3.0 * v * u * u * c + u * u * u * d;
                    (mix(a.0, b.0, c.0, d.0), mix(a.1, b.1, c.1, d.1))
                })
            };
            let mut wave: Vec<(f32, f32)> = cubic((12.0, 34.0), (18.0, 6.0), (24.0, 6.0), (30.0, 34.0)).collect();
            wave.extend(cubic((30.0, 34.0), (36.0, 62.0), (42.0, 62.0), (48.0, 34.0)).skip(1));
            wave.push((54.0, 16.0));
            line(&wave, stroke(4.0, active_3));
        }
    }
}

/// Draw a ribbon glyph in the theme's icon roles: line art in `icon_line` and accents in the
/// strongest icon blue, both faded when the action is unavailable. Registered SVG masters use
/// the embedded loader; other chrome glyphs retain their native painter and menu artwork.
fn paint_icon(p: &egui::Painter, icon: ToolIcon, center: Pos2, size: f32, t: Tokens, enabled: bool) {
    if crate::ribbon_icons::paint(p, icon, center, size, t, enabled) {
        return;
    }
    let fade = |color: Color32| if enabled { color } else { color.gamma_multiply(0.33) };
    let (ink, accent) = (fade(t.icon_line), fade(t.icon_active_3));
    let at = |x: f32, y: f32| center + vec2(x - 32.0, y - 32.0) * (size / 64.0);
    let stroke = |width: f32, color| Stroke::new(width * size / 64.0, color);
    let line = |points: &[(f32, f32)], stroke: Stroke| p.add(Shape::line(points.iter().map(|&(x, y)| at(x, y)).collect(), stroke));
    match icon {
        ToolIcon::Ground => {
            line(&[(32., 10.), (32., 29.)], stroke(3., ink));
            for (y, width) in [(29., 18.), (39., 12.), (49., 6.)] {
                line(&[(32. - width, y), (32. + width, y)], stroke(3., ink));
            }
        }
        ToolIcon::Line => {
            line(&[(10., 50.), (54., 14.)], stroke(4., ink));
            p.circle_filled(at(10., 50.), 4. * size / 64., accent);
            p.circle_filled(at(54., 14.), 4. * size / 64., accent);
        }
        ToolIcon::Rectangle => {
            line(&[(10.0, 16.0), (54.0, 16.0), (54.0, 48.0), (10.0, 48.0), (10.0, 16.0)], stroke(4.0, ink));
            for (x, y) in [(10.0, 16.0), (54.0, 16.0), (54.0, 48.0), (10.0, 48.0)] {
                p.rect_filled(Rect::from_center_size(at(x, y), vec2(7.0, 7.0) * (size / 64.0)), 0, accent);
            }
        }
        ToolIcon::Select => {
            line(&[(18.0, 8.0), (18.0, 52.0), (30.0, 39.0), (40.0, 56.0), (47.0, 51.0), (36.0, 34.0), (54.0, 32.0), (18.0, 8.0)], stroke(3.5, ink));
        }
        ToolIcon::Pin => {
            line(&[(12.0, 32.0), (46.0, 32.0)], stroke(4.0, ink));
            p.circle_stroke(at(12.0, 32.0), 5.0 * size / 64.0, stroke(3.0, accent));
            line(&[(46.0, 18.0), (46.0, 46.0)], stroke(4.0, accent));
        }
        ToolIcon::Pad => {
            p.rect_filled(Rect::from_min_max(at(15.0, 10.0), at(49.0, 54.0)), size / 10.0, accent);
            p.circle(at(32.0, 32.0), 7.0 * size / 64.0, t.panel, stroke(3.0, ink));
        }
        ToolIcon::Circle => {
            p.circle_stroke(at(32.0, 32.0), 22.0 * size / 64.0, stroke(4.0, ink));
        }
        ToolIcon::Subsheet
        | ToolIcon::Power
        | ToolIcon::Text
        | ToolIcon::Bus
        | ToolIcon::NetLabel
        | ToolIcon::Move
        | ToolIcon::Rotate
        | ToolIcon::Mirror
        | ToolIcon::Rules
        | ToolIcon::Erc
        | ToolIcon::Annotate
        | ToolIcon::Bom
        | ToolIcon::Sync => {}
        ToolIcon::Sheet | ToolIcon::HierarchicalLabel | ToolIcon::Symbol | ToolIcon::Wire => {}
        ToolIcon::Slice => {
            line(&[(8.0, 32.0), (56.0, 32.0)], stroke(4.0, ink));
            line(&[(32.0, 10.0), (32.0, 24.0)], stroke(4.0, accent));
            line(&[(32.0, 40.0), (32.0, 54.0)], stroke(4.0, accent));
            p.circle_filled(at(32.0, 32.0), 5.0 * size / 64.0, accent);
        }
        ToolIcon::Break => {
            line(&[(8.0, 32.0), (24.0, 32.0)], stroke(4.0, ink));
            line(&[(40.0, 32.0), (56.0, 32.0)], stroke(4.0, ink));
            line(&[(36.0, 12.0), (27.0, 27.0), (37.0, 37.0), (28.0, 52.0)], stroke(3.5, accent));
        }
        ToolIcon::Heal => {
            line(&[(8.0, 44.0), (24.0, 44.0), (24.0, 20.0), (56.0, 20.0)], stroke(4.0, ink));
            line(&[(35.0, 42.0), (43.0, 50.0), (58.0, 32.0)], stroke(4.0, accent));
        }
        ToolIcon::Delete => {
            line(&[(14.0, 18.0), (50.0, 18.0)], stroke(4.0, ink));
            line(&[(24.0, 18.0), (24.0, 10.0), (40.0, 10.0), (40.0, 18.0)], stroke(3.5, ink));
            line(&[(18.0, 24.0), (21.0, 54.0), (43.0, 54.0), (46.0, 24.0)], stroke(4.0, ink));
            for x in [28.0, 36.0] {
                line(&[(x, 28.0), (x, 46.0)], stroke(3.0, accent));
            }
        }
        ToolIcon::Nets => {
            for y in [17.0, 32.0, 47.0] {
                p.circle_filled(at(13.0, y), 4.0 * size / 64.0, accent);
                line(&[(25.0, y), (54.0, y)], stroke(4.0, ink));
            }
        }
        ToolIcon::Fit => {
            let s = stroke(5.6, ink);
            for corner in [
                [(10.0, 24.0), (10.0, 10.0), (24.0, 10.0)],
                [(40.0, 10.0), (54.0, 10.0), (54.0, 24.0)],
                [(54.0, 40.0), (54.0, 54.0), (40.0, 54.0)],
                [(24.0, 54.0), (10.0, 54.0), (10.0, 40.0)],
            ] {
                line(&corner, s);
            }
            p.rect_filled(Rect::from_min_max(at(24.0, 24.0), at(40.0, 40.0)), (2.0 * size / 64.0) as u8, ink);
        }
        ToolIcon::Grid => {
            for x in [-0.3, 0.0, 0.3] {
                for y in [-0.3, 0.0, 0.3] {
                    p.circle_filled(center + vec2(x, y) * size, size * 0.055, ink);
                }
            }
        }
        ToolIcon::Settings => {
            let mut painter = p.clone();
            if !enabled {
                painter.set_opacity(0.33);
            }
            crate::menu::MenuIcon::Settings.paint_sized(&painter, center, size, t, false);
        }
        ToolIcon::Browser => paint_folder(p, center, size, ink),
        ToolIcon::SplitView => {
            line(&[(8.0, 12.0), (56.0, 12.0), (56.0, 52.0), (8.0, 52.0), (8.0, 12.0)], stroke(4.0, ink));
            line(&[(32.0, 12.0), (32.0, 52.0)], stroke(4.0, accent));
        }
        ToolIcon::EditorWindow => {
            line(&[(12.0, 20.0), (8.0, 20.0), (8.0, 54.0), (44.0, 54.0), (44.0, 50.0)], stroke(4.0, ink));
            line(&[(20.0, 10.0), (56.0, 10.0), (56.0, 44.0), (20.0, 44.0), (20.0, 10.0)], stroke(4.0, accent));
        }
        ToolIcon::Footprint => {
            line(&[(18.0, 18.0), (46.0, 18.0), (46.0, 46.0), (18.0, 46.0), (18.0, 18.0)], stroke(4.0, ink));
            for offset in [24.0, 32.0, 40.0] {
                line(&[(8.0, offset), (16.0, offset)], stroke(3.5, accent));
                line(&[(48.0, offset), (56.0, offset)], stroke(3.5, accent));
                line(&[(offset, 8.0), (offset, 16.0)], stroke(3.5, accent));
                line(&[(offset, 48.0), (offset, 56.0)], stroke(3.5, accent));
            }
        }
        ToolIcon::Track => {
            line(&[(8.0, 48.0), (26.0, 48.0), (46.0, 28.0), (56.0, 28.0)], stroke(6.0, ink));
            p.circle_filled(at(8.0, 48.0), 5.0 * size / 64.0, accent);
            p.circle_filled(at(56.0, 28.0), 5.0 * size / 64.0, accent);
        }
        ToolIcon::Via => {
            p.circle_filled(at(32.0, 32.0), 20.0 * size / 64.0, accent);
            p.circle(at(32.0, 32.0), 9.0 * size / 64.0, t.panel, stroke(3.0, ink));
        }
        ToolIcon::Zone => {
            line(&[(10.0, 14.0), (44.0, 14.0), (54.0, 26.0), (54.0, 52.0), (10.0, 52.0), (10.0, 14.0)], stroke(4.0, ink));
            for x in [18.0, 28.0, 38.0] {
                line(&[(x, 46.0), (x + 12.0, 22.0)], stroke(3.0, accent));
            }
        }
        ToolIcon::Pair => {
            line(&[(8.0, 42.0), (24.0, 42.0), (40.0, 26.0), (56.0, 26.0)], stroke(4.0, ink));
            line(&[(8.0, 52.0), (28.0, 52.0), (44.0, 36.0), (56.0, 36.0)], stroke(4.0, accent));
        }
        ToolIcon::Tune => {
            line(
                &[
                    (6.0, 44.0),
                    (14.0, 44.0),
                    (14.0, 20.0),
                    (24.0, 20.0),
                    (24.0, 44.0),
                    (34.0, 44.0),
                    (34.0, 20.0),
                    (44.0, 20.0),
                    (44.0, 44.0),
                    (58.0, 44.0),
                ],
                stroke(4.0, ink),
            );
            p.circle_filled(at(6.0, 44.0), 4.0 * size / 64.0, accent);
            p.circle_filled(at(58.0, 44.0), 4.0 * size / 64.0, accent);
        }
        ToolIcon::Align => {
            line(&[(12.0, 8.0), (12.0, 56.0)], stroke(4.0, accent));
            for (y, length) in [(18.0, 38.0), (32.0, 26.0), (46.0, 32.0)] {
                p.rect_filled(Rect::from_min_max(at(16.0, y - 4.0), at(16.0 + length, y + 4.0)), (size / 32.0) as u8, ink);
            }
        }
        ToolIcon::Drc => {
            line(&[(10.0, 10.0), (54.0, 10.0), (54.0, 54.0), (10.0, 54.0), (10.0, 10.0)], stroke(4.0, ink));
            line(&[(20.0, 32.0), (29.0, 42.0), (46.0, 22.0)], stroke(5.0, accent));
        }
        ToolIcon::Measure => {
            line(&[(6.0, 26.0), (58.0, 26.0), (58.0, 40.0), (6.0, 40.0), (6.0, 26.0)], stroke(3.5, ink));
            for (index, x) in [14.0, 22.0, 30.0, 38.0, 46.0].into_iter().enumerate() {
                line(&[(x, 26.0), (x, if index % 2 == 0 { 35.0 } else { 31.0 })], stroke(3.0, accent));
            }
        }
        ToolIcon::FlipView => {
            line(&[(32.0, 6.0), (32.0, 58.0)], stroke(3.0, ink));
            line(&[(26.0, 16.0), (8.0, 46.0), (26.0, 46.0), (26.0, 16.0)], stroke(4.0, ink));
            line(&[(38.0, 16.0), (56.0, 46.0), (38.0, 46.0), (38.0, 16.0)], stroke(4.0, accent));
        }
        ToolIcon::Outputs => {
            line(&[(14.0, 6.0), (40.0, 6.0), (50.0, 16.0), (50.0, 58.0), (14.0, 58.0), (14.0, 6.0)], stroke(4.0, ink));
            line(&[(32.0, 20.0), (32.0, 46.0)], stroke(4.0, accent));
            line(&[(22.0, 36.0), (32.0, 46.0), (42.0, 36.0)], stroke(4.0, accent));
        }
        ToolIcon::Refill => {
            line(&[(10.0, 14.0), (44.0, 14.0), (54.0, 26.0), (54.0, 52.0), (10.0, 52.0), (10.0, 14.0)], stroke(4.0, ink));
            p.rect_filled(Rect::from_min_max(at(18.0, 22.0), at(46.0, 44.0)), (size / 32.0) as u8, accent);
        }
    }
}

/// Show the retained license texts for the embedded toolbar artwork inside About.
pub fn ribbon_licenses(ui: &mut Ui) {
    ui.collapsing("Toolbar licenses", |ui| {
        ScrollArea::vertical().id_salt("ribbon_licenses").max_height(Tokens::get().control_height * 8.0).show(ui, |ui| {
            ui.label(crate::ribbon_icons::NOTICES);
        });
    });
}

#[cfg(test)]
mod ribbon_tests {
    use super::*;

    /// A final Board group absorbs trailing space before the pinned fields. Its two tools and
    /// menu caption share the center of that visible section, rather than leaving a trailing gap.
    #[test]
    fn final_two_button_group_centers_in_remaining_space() {
        let ctx = egui::Context::default();
        crate::install(&ctx);
        let mut output =
            ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600., 200.))), ..Default::default() }, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let tools: Vec<_> = ["Update PCB", "Setup"]
                            .into_iter()
                            .map(|label| RibbonTool {
                                icon: ToolIcon::Fit,
                                label,
                                description: label,
                                shortcut: "",
                                enabled: true,
                                active: false,
                                menu_check: None,
                            })
                            .collect();
                        let response = ui.scope(|ui| ribbon_group_fill(ui, "Board", &tools)).response;
                        let first = solidcraft_ui_registry::rect_of("pcbcraft:ribbon:Board:button_rect:Update PCB").unwrap();
                        let second = solidcraft_ui_registry::rect_of("pcbcraft:ribbon:Board:button_rect:Setup").unwrap();
                        let caption = solidcraft_ui_registry::rect_of("pcbcraft:ribbon:Board:section_target:Board").unwrap();
                        let buttons = first.union(second);
                        assert!(response.rect.width() > buttons.width() + 100., "fixture must include trailing room");
                        assert!((buttons.center().x - response.rect.center().x).abs() <= 1.0);
                        assert!((buttons.center().x - caption.center().x).abs() <= 1.0);
                        assert_eq!(first.size(), second.size());
                    });
                });
            });
        output.textures_delta.clear();
    }

    /// The toolkit's fitting reserve belongs outside the visible group. A single button
    /// and its caption must be centered in the host allocation, even for a long caption.
    #[test]
    fn single_button_groups_are_centered_in_their_allocated_space() {
        let ctx = egui::Context::default();
        crate::install(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (title, label) in [("PCB", "Update PCB"), ("Navigate", "Fit"), ("Selected definition", "Edit copy")] {
                        let tools = [RibbonTool {
                            icon: ToolIcon::Fit,
                            label,
                            description: label,
                            shortcut: "",
                            enabled: true,
                            active: false,
                            menu_check: None,
                        }];
                        let response = ui.scope(|ui| ribbon_group(ui, title, &tools)).response;
                        let prefix = format!("pcbcraft:ribbon:{title}:");
                        let button = solidcraft_ui_registry::rect_of(&format!("{prefix}button_rect:{label}")).unwrap();
                        let caption = solidcraft_ui_registry::rect_of(&format!("{prefix}section_target:{title}")).unwrap();
                        assert!((button.center().x - response.rect.center().x).abs() <= 1.0, "{title}: {button:?}, {:?}", response.rect);
                        assert!((button.center().x - caption.center().x).abs() <= 1.0, "{title}");
                    }
                });
            });
        });
        output.textures_delta.clear();
    }
}
