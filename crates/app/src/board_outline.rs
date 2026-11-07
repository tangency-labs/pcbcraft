//! Revision-bound board boundary forms; exact geometry is committed only through engine commands.
use crate::{Dialog, PcbApp, ViewId, editor_view::DocumentTarget};
use pcbcraft_chrome::{board::OutlineAction, units};
use pcbcraft_engine::{
    SET_BOARD_OUTLINE,
    board::{outline::Outline, view::BoardTarget},
    schematic::Point,
};
use serde_json::{Value, json};

/// One modal draft retains exact untouched dimensions and its originating board/opening.
#[derive(Debug)]
pub(super) struct Edit {
    source: ViewId,
    board: BoardTarget,
    opening: u64,
    revision: u64,
    unit: units::Unit,
    values: [String; 4],
    initial: [String; 4],
    original: [i64; 4],
    can_remove: bool,
    focused: bool,
    error: Option<String>,
}

impl Edit {
    /// Preserve exact nanometers for untouched rounded readouts; parse all changed fields before
    /// validating the complete rectangle, so no individual field can partially change the board.
    fn outline(&self) -> Result<Outline, String> {
        let mut lengths = self.original;
        for (index, value) in self.values.iter().enumerate() {
            if value.trim() != self.initial[index] {
                lengths[index] = units::parse_length(value, self.unit).map_err(|error| error.to_string())?;
            }
        }
        let [x, y, width, height] = lengths;
        if width <= 0 || height <= 0 {
            return Err("Width and height must be positive.".into());
        }
        let max = Point {
            x_nm: x.checked_add(width).ok_or("Outline exceeds coordinate bounds.")?,
            y_nm: y.checked_add(height).ok_or("Outline exceeds coordinate bounds.")?,
        };
        Outline::rectangle(Point { x_nm: x, y_nm: y }, max)
    }
}

impl PcbApp {
    /// Seed a board-addressed form without inventing geometry in an empty project. The suggested
    /// 40 by 30 mm rectangle is only a draft until Apply; an existing boundary keeps its exact size.
    pub(super) fn begin_board_outline(&mut self) {
        let Some(project) = self.session.project() else { return };
        let Some(board) = self.views.active.board.target.and_then(|target| project.board_view(target)) else { return };
        let outline = board.outline();
        let (min, max) = match outline {
            Some(Outline::Rectangle { min, max }) => (min, max),
            None => (Point { x_nm: -20_000_000, y_nm: -15_000_000 }, Point { x_nm: 20_000_000, y_nm: 15_000_000 }),
        };
        let original = [min.x_nm, min.y_nm, max.x_nm - min.x_nm, max.y_nm - min.y_nm];
        let initial = original.map(|n| units::label(n, self.preferences.units));
        self.dialog = Dialog::BoardOutline(Edit {
            source: self.views.active.id(),
            board: board.target,
            opening: self.session.history_root_revision(),
            revision: self.session.revision(),
            unit: self.preferences.units,
            values: initial.clone(),
            initial,
            original,
            can_remove: outline.is_some(),
            focused: false,
            error: None,
        });
        self.views.cancel_gestures();
    }

    /// Keep the form tied to its original board and project opening across modal focus changes.
    /// A closed/retargeted source, concurrent edit or reopen expires the form without mutation.
    pub(super) fn board_outline_dialog(&mut self, ctx: &egui::Context, mut edit: Edit) {
        if self.session.history_root_revision() != edit.opening
            || self.session.revision() != edit.revision
            || self.views.view(edit.source).and_then(|view| view.target()) != Some(DocumentTarget::Board(edit.board))
        {
            self.message = Some("View or project changed; reopen Board Outline to continue.".into());
            return;
        }
        let action = pcbcraft_chrome::board::outline_dialog(ctx, &mut edit.values, &mut edit.focused, edit.error.as_deref(), edit.can_remove);
        let candidate = match action {
            OutlineAction::Apply => edit.outline().map(|outline| json!(outline)),
            OutlineAction::Remove => Ok(Value::Null),
            OutlineAction::Cancel => return,
            OutlineAction::None => {
                self.dialog = Dialog::BoardOutline(edit);
                return;
            }
        };
        let result = candidate.and_then(|outline| {
            self.session
                .apply(SET_BOARD_OUTLINE, &json!({"board":edit.board.board,"outline":outline,"expected_revision":edit.revision}))
                .map(|_| ())
                .map_err(|error| error.to_string())
        });
        match result {
            Ok(()) => {
                self.message = None;
                self.views.sync(&self.session);
                self.repaint_editors(ctx);
            }
            Err(error) => {
                edit.error = Some(error);
                self.dialog = Dialog::BoardOutline(edit);
            }
        }
    }
}
