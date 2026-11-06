//! The parts browser: a card docked at the left of the canvas that lists the built-in parts as
//! searchable thumbnails, in place of the old Place Symbol dialog. Click a part to place it, or
//! drag parts onto the sheet one after another. Browsing never edits the project; placing goes through the same
//! `schematic.place_symbol` command as before. Adapted from PCBcraft Next's parts browser.

use egui::{Align2, FontId, Rect, Response, Sense, Ui, pos2, vec2};
use pcbcraft_chrome::{Tokens, workspace};
use pcbcraft_engine::schematic::{BUILTIN_SYMBOLS, SymbolDefinition};

use crate::schematic_canvas::{PartDrag, paint_preview};

/// How the browser lays out parts, as PCBcraft Next offers them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum View {
    Small,
    #[default]
    Medium,
    Large,
    List,
}

impl View {
    /// The square thumbnail size in points; the list uses a small one beside each name.
    fn thumbnail(self) -> f32 {
        match self {
            Self::Small => 56.0,
            Self::Medium => 84.0,
            Self::Large => 116.0,
            Self::List => 30.0,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct PartsBrowser {
    pub(crate) open: bool,
    query: String,
    view: View,
    /// Focus the search field on the next frame, as after pressing A.
    focus: bool,
    /// Whether the search field had focus at the end of the last frame. egui drops focus on
    /// Escape before the frame's code runs, so this is how Escape knows it was meant here.
    search_focused: bool,
}

impl PartsBrowser {
    /// Open the browser with an empty, focused search, as Capture › Symbol and A do. If it's
    /// already open, keep the search and just focus it.
    pub(crate) fn open_and_focus(&mut self) {
        if !self.open {
            self.query.clear();
        }
        self.open = true;
        self.focus = true;
    }

    /// Close the browser on Escape when its search had focus or workspace keys are available.
    /// Call after giving dialogs, popups and the assistant priority, but before tool cancellation.
    /// Return true when the key was consumed and the browser closed.
    pub(crate) fn take_escape(&mut self, ctx: &egui::Context) -> bool {
        if self.open
            && (self.search_focused || !ctx.egui_wants_keyboard_input())
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.open = false;
            self.search_focused = false;
            return true;
        }
        false
    }

    /// The built-in parts whose name, ID or reference prefix contains the search text, in
    /// catalog order. Matching ignores case; an empty search lists every part.
    fn matches(&self) -> Vec<&'static SymbolDefinition> {
        let query = self.query.trim().to_lowercase();
        BUILTIN_SYMBOLS
            .iter()
            .copied()
            .filter(|definition| {
                query.is_empty()
                    || definition.name.to_lowercase().contains(&query)
                    || definition.id.contains(&query)
                    || definition.reference_prefix().to_string().eq_ignore_ascii_case(&query)
            })
            .collect()
    }

    /// Draw the browser while it's open and return the part clicked, or the first match when
    /// Enter is pressed in the search field. Either closes the browser.
    /// `placing` is the part being placed, which is highlighted. See [`Self::take_escape`].
    pub(crate) fn show(&mut self, ui: &mut Ui, placing: Option<&'static SymbolDefinition>) -> Option<&'static SymbolDefinition> {
        if !self.open {
            return None;
        }
        let mut picked = None;
        let closed = workspace::left_card(ui, "parts_browser", "Parts", |ui| {
            let t = Tokens::get();
            let search = ui.add(egui::TextEdit::singleline(&mut self.query).hint_text("Search parts").desired_width(f32::INFINITY));
            if std::mem::take(&mut self.focus) {
                search.request_focus();
            }
            self.search_focused = search.has_focus();
            let enter = search.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.add_space(t.space_4);
            // Compact toggles: the card is narrower than a Preferences row.
            ui.horizontal(|ui| {
                for (view, label, tip) in
                    [(View::Small, "S", "Small"), (View::Medium, "M", "Medium"), (View::Large, "L", "Large"), (View::List, "List", "List")]
                {
                    ui.selectable_value(&mut self.view, view, label).on_hover_text(tip);
                }
            });
            ui.add_space(t.space_6);
            let parts = self.matches();
            // Enter in the search places the first match, as Enter does in KiCad's chooser.
            if enter {
                picked = parts.first().copied();
            }
            if parts.is_empty() {
                ui.label(pcbcraft_chrome::shell::dim(format!("No parts match “{}”", self.query.trim())));
            }
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(t.space_6, t.space_6);
                for definition in parts {
                    if part(ui, definition, self.view, placing == Some(definition)).clicked() {
                        picked = Some(definition);
                    }
                }
            });
        });
        // Clicking a part closes the browser and hands the canvas its room back, as the Place
        // Symbol dialog and KiCad's chooser do. Dragging leaves it open for the next part.
        if closed || picked.is_some() {
            self.open = false;
        }
        picked
    }
}

/// One part: a thumbnail tile, or a row with a small thumbnail beside its name and reference
/// prefix. Clicking reports it; dragging carries it to the canvas.
fn part(ui: &mut Ui, definition: &'static SymbolDefinition, view: View, selected: bool) -> Response {
    let t = Tokens::get();
    let thumb = view.thumbnail();
    let size = match view {
        View::List => vec2(ui.available_width(), thumb + t.space_6),
        View::Small => vec2(thumb, thumb),
        View::Medium | View::Large => vec2(thumb, thumb + t.space_16),
    };
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, definition.name));
    response.dnd_set_drag_payload(PartDrag(definition));
    let hot = response.hovered() || response.has_focus() || response.dragged();
    if selected || hot {
        ui.painter().rect_filled(rect, t.radius_sm, if selected { t.accent_soft } else { t.hover });
    }
    let ink = if selected { t.ribbon_tab_text } else { t.text };
    let painter = ui.painter().with_clip_rect(rect);
    match view {
        View::List => {
            let preview = Rect::from_min_size(rect.min + vec2(t.space_4, t.space_3), vec2(thumb, thumb));
            paint_preview(&painter, definition, preview.shrink(t.space_2), ink);
            let text = pos2(preview.right() + t.space_8, rect.center().y);
            painter.text(text, Align2::LEFT_CENTER, definition.name, FontId::proportional(t.font_label), ink);
            let prefix = format!("{}?", definition.reference_prefix());
            painter.text(rect.right_center() - vec2(t.space_6, 0.0), Align2::RIGHT_CENTER, prefix, FontId::monospace(t.font_small), t.text_dim);
        }
        View::Small => paint_preview(&painter, definition, rect.shrink(t.space_6), ink),
        View::Medium | View::Large => {
            let preview = Rect::from_min_size(rect.min, vec2(thumb, thumb)).shrink(t.space_6);
            paint_preview(&painter, definition, preview, ink);
            // Long names end in an ellipsis inside the tile; the tooltip has the full name.
            let mut job = egui::text::LayoutJob::simple_singleline(definition.name.to_owned(), FontId::proportional(t.font_small), ink);
            job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - t.space_4);
            let galley = painter.layout_job(job);
            let at = pos2(rect.center().x - galley.size().x / 2.0, rect.bottom() - t.space_2 - galley.size().y);
            painter.galley(at, galley, ink);
        }
    }
    let tip = format!("{} · drag onto the sheet or click to place", definition.name);
    response.on_hover_text(tip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_matches_names_ids_and_prefixes_ignoring_case() {
        let mut browser = PartsBrowser::default();
        let names = |browser: &PartsBrowser| browser.matches().into_iter().map(|definition| definition.name).collect::<Vec<_>>();
        assert_eq!(names(&browser).len(), BUILTIN_SYMBOLS.len());
        browser.query = "CAP".to_owned();
        assert_eq!(names(&browser), ["Capacitor"]);
        browser.query = "j".to_owned();
        assert_eq!(names(&browser), ["2-pin connector"], "J is the connector's prefix");
        browser.query = "zzz".to_owned();
        assert!(names(&browser).is_empty());
    }
}
