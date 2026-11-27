//! Arrow nudges belong to the idle board canvas, not the application-wide shortcut dispatcher.
use crate::{PcbApp, ViewId, editor_view::ViewInput};
use egui::{Event, Key, Modifiers, Ui};
use pcbcraft_engine::{
    SET_FOOTPRINT_REFERENCE_OFFSET,
    board::{
        clipboard::BoardSelection,
        movement::{MOVE_ITEMS, NudgeBurst},
    },
    schematic::Point,
};
use serde_json::json;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Selection {
    Items(BoardSelection),
    Reference(String),
}

/// Only successive repeat events for the captured arrow and selection join the original undo step.
#[derive(Debug, Default)]
pub(super) struct State {
    held: Option<(Key, ViewInput, Selection)>,
    history: Option<NudgeBurst>,
}

/// Resolve physical Ctrl aliases without accepting Command arrows or mixed Alt chords.
/// Distances use whole nanometres; the fine step rounds to the nearest nanometre.
pub(super) fn arrow_delta(key: Key, keys: Modifiers, grid_nm: i64, mirrored: bool) -> Option<Point> {
    if keys.mac_cmd || keys.command && !keys.ctrl || keys.alt && (keys.shift || keys.ctrl) {
        return None;
    }
    let step = if keys.alt {
        (grid_nm + 5) / 10
    } else if keys.shift {
        10 * grid_nm
    } else {
        grid_nm
    };
    let x = if mirrored { -step } else { step };
    let (x_nm, y_nm) = match key {
        Key::ArrowLeft => (-x, 0),
        Key::ArrowRight => (x, 0),
        Key::ArrowUp => (0, -step),
        Key::ArrowDown => (0, step),
        _ => return None,
    };
    Some(Point { x_nm, y_nm })
}

impl PcbApp {
    /// While an idle board canvas owns input, apply each press immediately. Read exact raw-event
    /// modifiers, so Ctrl aliases and press/release pairs in the same frame have one owner.
    pub(super) fn board_nudge(&mut self, ui: &Ui, id: ViewId, enabled: bool) {
        if id != self.views.active.id() {
            return;
        }
        let view = &self.views.active;
        let eligible = enabled
            && ui.rect_contains_pointer(ui.available_rect_before_wrap())
            && !ui.ctx().egui_wants_keyboard_input()
            && !self.parts.is_modal()
            && !self.files.busy()
            && !crate::board_clipboard::busy(&view.board)
            && !ui.input(|i| {
                !i.focused || i.pointer.any_down() || i.pointer.any_released() || i.events.iter().any(|e| matches!(e, Event::WindowFocused(false)))
            });
        if !eligible {
            self.views.active.board.nudge = State::default();
            return;
        }
        // Avoid inspecting a project's population on frames without arrow keys. Only a release
        // for the currently held key needs handling, even if its modifier has since changed.
        let has_arrows = ui.input(|i| {
            i.events.iter().any(|event| matches!(event, Event::Key { key: Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown, .. }))
        });
        if !has_arrows {
            return;
        }
        let (Some(project), Some(target)) = (self.session.project(), view.board.target) else { return };
        let Some(source) = view.input(&self.session) else { return };
        let items = crate::board_clipboard::selection(&view.board, project);
        let selection = if !items.is_empty() {
            Selection::Items(items)
        } else if let Some(reference) = &view.board.reference.selected {
            Selection::Reference(reference.clone())
        } else {
            self.views.active.board.nudge = State::default();
            return;
        };
        let grid = view.camera.snap_nm();
        let mirrored = view.camera.mirrored();
        let mut state = std::mem::take(&mut self.views.active.board.nudge);
        let mut source = source;
        let events = ui.input_mut(|input| {
            let mut events = Vec::new();
            input.events.retain(|event| {
                if let Event::Key { key, modifiers, pressed, repeat, .. } = *event
                    && let Some(delta) = arrow_delta(key, modifiers, grid, mirrored)
                {
                    events.push((key, pressed, repeat, delta));
                    return !pressed;
                }
                if let Event::Key { key, pressed: false, .. } = *event {
                    events.push((key, false, false, Point { x_nm: 0, y_nm: 0 }));
                }
                true
            });
            events
        });
        for (key, pressed, repeat, delta) in events {
            if !pressed {
                if state.held.as_ref().is_some_and(|(held, _, _)| *held == key) {
                    state = State::default();
                }
                continue;
            }
            if repeat {
                if state.held.as_ref().is_none_or(|(held, captured, selected)| *held != key || *captured != source || *selected != selection) {
                    continue;
                }
            } else {
                state = State::default();
            }
            let revision = source.revision;
            let (command, params) = match &selection {
                Selection::Items(items) => (
                    MOVE_ITEMS,
                    json!({"board":target.board, "items":items,
                    "dx_nm":delta.x_nm, "dy_nm":delta.y_nm, "preserve_contacts":true, "expected_revision":revision}),
                ),
                Selection::Reference(id) => {
                    let Some(footprint) = self.session.project().and_then(|p| p.board_view(target)).and_then(|board| board.footprint(id)) else {
                        break;
                    };
                    if footprint.locked {
                        self.message = Some("Unlock the footprint before nudging its reference text.".into());
                        break;
                    }
                    let (x, y) = crate::board_canvas::repose(delta.x_nm, delta.y_nm, (footprint.rotation, footprint.bottom), (0, false));
                    let offset = Point { x_nm: footprint.reference_offset.x_nm + x, y_nm: footprint.reference_offset.y_nm + y };
                    (SET_FOOTPRINT_REFERENCE_OFFSET, json!({"board":target.board, "footprint":id,"offset":offset,"expected_revision":revision}))
                }
            };
            self.run_project_command(command, &params, Some(&mut state.history));
            let Some(current) = self.views.active.input(&self.session) else { return };
            source = current;
            if self.message.is_none() || state.history.is_some() {
                state.held = Some((key, source, selection.clone()));
            }
        }
        self.views.active.board.nudge = state;
    }
}
