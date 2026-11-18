//! View-owned terminal/land inspection, separate from every authored edit selection.
use crate::{
    PcbApp,
    editor_view::{DocumentTarget, EditorView, ViewId, Workspace},
};
use egui::{Modifiers, Painter, Rect, Stroke};
use pcbcraft_engine::{
    Session,
    board::{
        Pad,
        correspondence::{PadTarget, Source, TerminalTarget},
        view::{BoardView, PadView},
    },
    schematic::Terminal,
};
#[cfg(not(target_arch = "wasm32"))]
use serde_json::{Value, json};

/// Bound inspection state independently of project object counts.
pub(super) const MAX_TARGETS: usize = 256;

/// Parse bounded distinct canonical addresses without inferring owners from geometry or names.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn parse(value: &Value) -> Result<Vec<Source>, String> {
    let values = value.as_array().filter(|values| values.len() <= MAX_TARGETS).ok_or("Inspection requires at most 256 targets")?;
    let mut targets = Vec::new();
    for value in values {
        let target = Source::parse(value)?;
        if targets.contains(&target) {
            return Err("Duplicate inspection target".into());
        }
        targets.push(target);
    }
    Ok(targets)
}

/// Resolve the exact pad from the addressed board's actual placements, including unmapped lands.
pub(super) fn pad(board: BoardView<'_>, target: Pad) -> Option<PadView> {
    let address = target.inspect();
    let footprint = address["footprint"].as_str()?;
    let number = address["pad"].as_str()?;
    board.footprints().find(|item| item.id == footprint)?.pads.into_iter().find(|pad| pad.number == number)
}

/// Filters, physical layer visibility and qualified ownership all apply to local inspection.
pub(super) fn valid(view: &EditorView, session: &Session, source: Source) -> bool {
    if view.auxiliary() {
        return false;
    }
    match source {
        Source::Terminal(target) => {
            view.workspace == Workspace::Schematic
                && view.filters.pins
                && view.target() == Some(DocumentTarget::Sheet(target.sheet))
                && session
                    .project()
                    .and_then(|project| project.for_sheet(target.sheet))
                    .is_some_and(|sheet| sheet.terminal_position(target.terminal).is_some())
        }
        Source::Pad(target) => {
            view.workspace == Workspace::Board
                && view.filters.pads
                && view.target() == Some(DocumentTarget::Board(target.board))
                && session.project().and_then(|project| project.board_view(target.board)).is_some_and(|board| {
                    pad(board, target.pad).is_some_and(|pad| crate::board_canvas::pad_visible(&pad, board, &view.board.hidden_layers))
                })
        }
    }
}

/// Pick copper with its actual contour; only legacy center markers use a screen-space radius.
pub(super) fn pad_at(
    board: BoardView<'_>,
    footprints: &[pcbcraft_engine::board::view::FootprintView<'_>],
    camera: &crate::canvas_camera::Camera,
    rect: Rect,
    pointer: egui::Pos2,
    hidden: &std::collections::BTreeSet<String>,
) -> Option<Pad> {
    let point = camera.unsnapped(pointer, rect);
    footprints.iter().rev().find_map(|footprint| {
        footprint
            .pads
            .iter()
            .rev()
            .find(|pad| {
                crate::board_canvas::pad_visible(pad, board, hidden)
                    && (pad.contains_point(point) || (pad.copper.is_none() && camera.screen(pad.center, rect).distance(pointer) <= 6.))
            })
            .and_then(|pad| Pad::parse(&footprint.id, &pad.number))
            .map(|pad| pad.on_board(board.target.board))
    })
}

/// Paint selected pin endpoints only; owner symbols and counterpart views acquire no authority.
pub(super) fn paint_pins(
    painter: &Painter,
    camera: &crate::canvas_camera::Camera,
    rect: Rect,
    project: &pcbcraft_engine::Project,
    targets: &[Source],
) {
    for source in targets {
        if let Source::Terminal(target) = source
            && target.sheet == project.sheet_target()
            && let Some(at) = project.terminal_position(target.terminal)
        {
            ring(painter, camera.screen(at, rect));
        }
    }
}

/// Capture exact selected pad positions from the already resolved paint geometry.
/// This avoids projecting the board again after the footprint painter consumes its previews.
pub(super) fn pad_centers(
    board: BoardView<'_>,
    footprints: &[pcbcraft_engine::board::view::FootprintView<'_>],
    targets: &[Source],
) -> Vec<pcbcraft_engine::schematic::Point> {
    targets
        .iter()
        .filter_map(|source| {
            let Source::Pad(target) = source else { return None };
            if target.board != board.target {
                return None;
            }
            let address = target.pad.inspect();
            let footprint = footprints.iter().find(|footprint| Some(footprint.id.as_str()) == address["footprint"].as_str())?;
            Some(footprint.pads.iter().find(|pad| Some(pad.number.as_str()) == address["pad"].as_str())?.center)
        })
        .collect()
}

/// Paint only the selected lands over the finished copper, without owner outlines.
pub(super) fn paint_pads(painter: &Painter, camera: &crate::canvas_camera::Camera, rect: Rect, centers: &[pcbcraft_engine::schematic::Point]) {
    for at in centers {
        ring(painter, camera.screen(*at, rect));
    }
}

/// Original electronics inspection geometry reuses the shared selection ink.
fn ring(painter: &Painter, center: egui::Pos2) {
    painter.circle_stroke(center, 7., Stroke::new(2., pcbcraft_chrome::Tokens::get().accent));
}

impl PcbApp {
    /// Atomically validate replacement/add/toggle inspection through the same path for every
    /// client. Replacement clears edit selections; additive inspection never creates owners.
    pub(super) fn select_inspection(&mut self, id: ViewId, targets: Vec<Source>, mode: &str) -> Result<(), String> {
        self.views.sync(&self.session);
        let view = self.views.view(id).ok_or("The addressed view closed")?;
        if targets.iter().any(|target| !valid(view, &self.session, *target)) {
            return Err("Inspection target is unavailable, hidden, filtered or belongs to another view document".into());
        }
        let mut next = if mode == "replace" { Vec::new() } else { view.inspection.clone() };
        for target in targets {
            if let Some(index) = next.iter().position(|old| *old == target) {
                if mode == "toggle" {
                    next.remove(index);
                }
            } else {
                next.push(target);
            }
        }
        if next.len() > MAX_TARGETS {
            return Err("Inspection selection exceeds 256 targets".into());
        }
        if !matches!(mode, "replace" | "add" | "toggle") {
            return Err("Unknown inspection selection mode".into());
        }
        let previous = self.views.active.id();
        self.views.bind(id);
        self.views.active.cancel_gesture();
        if mode == "replace" {
            match self.views.active.workspace {
                Workspace::Schematic => self.select(Default::default()),
                Workspace::Board => self.select_board_items(Default::default()),
            }
        }
        self.views.active.inspection = next;
        self.views.bind(previous);
        Ok(())
    }

    /// Inspection-only selections never turn hovered owner shortcuts into edit authority.
    /// Explicit drawing tools and modified navigation shortcuts remain available.
    pub(super) fn consume_inspection_edit_keys(&self, ctx: &egui::Context) {
        let view = &self.views.active;
        let only = !view.inspection.is_empty()
            && match view.workspace {
                Workspace::Schematic => view.schematic.selection.is_empty(),
                Workspace::Board => view.board.selection.is_empty() && view.board.tracks.selected.is_empty(),
            };
        if only {
            ctx.input_mut(|input| {
                for key in [egui::Key::M, egui::Key::G, egui::Key::R, egui::Key::E, egui::Key::Delete, egui::Key::Backspace] {
                    input.consume_key(Modifiers::NONE, key);
                }
                let keys = if view.workspace == Workspace::Board { &[egui::Key::F][..] } else { &[egui::Key::X, egui::Key::Y][..] };
                for key in keys {
                    input.consume_key(Modifiers::NONE, *key);
                }
            });
        }
    }

    /// GUI modifier routing uses the same explicit semantic addresses as the live command.
    pub(super) fn inspect_pin(&mut self, terminal: Terminal, modifiers: Modifiers) {
        let Some(DocumentTarget::Sheet(sheet)) = self.views.active.target() else { return };
        self.pick_browser_inspection(Source::Terminal(TerminalTarget { sheet, terminal }), modifiers);
    }

    /// Qualify a canvas land by the active physical board before semantic selection.
    pub(super) fn inspect_pad(&mut self, pad: Pad, modifiers: Modifiers) {
        let Some(board) = self.views.active.board.target else { return };
        self.pick_browser_inspection(Source::Pad(PadTarget { board, pad }), modifiers);
    }

    /// Report validation errors without leaving a partial edit or inspection selection.
    pub(super) fn pick_browser_inspection(&mut self, source: Source, modifiers: Modifiers) {
        let mode = if modifiers.ctrl || modifiers.command {
            "toggle"
        } else if modifiers.shift {
            "add"
        } else {
            "replace"
        };
        if let Err(error) = self.select_inspection(self.views.active.id(), vec![source], mode) {
            self.message = Some(error);
        }
    }
}

/// Contextual inspection labels never expose editable owner properties.
pub(super) fn labels(session: &Session, targets: &[Source]) -> Vec<String> {
    targets
        .iter()
        .filter_map(|source| match source {
            Source::Terminal(target) => {
                let project = session.project()?.for_sheet(target.sheet)?;
                let symbol = project.terminal_symbol(target.terminal)?;
                Some(format!("Pin {}.{}", project.reference(symbol), target.terminal.number()))
            }
            Source::Pad(target) => {
                let board = session.project()?.board_view(target.board)?;
                let address = target.pad.inspect();
                let footprint = board.footprint(address["footprint"].as_str()?)?;
                Some(format!("Pad {}.{}", footprint.reference, address["pad"].as_str()?))
            }
        })
        .collect()
}

/// Inspect runtime targets through the published qualified correspondence vocabulary.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn inspect(targets: &[Source]) -> Value {
    json!(targets.iter().map(|source| source.inspect()).collect::<Vec<_>>())
}
