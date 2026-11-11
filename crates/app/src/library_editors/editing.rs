//! Shared editor operations and a revision-bound array form; every authored change uses Action.
use super::*;
use pcbcraft_engine::{authoring::Primitive, library_graphics::Transform, schematic::Point};

#[derive(Clone)]
struct Clipboard {
    kind: Kind,
    primitives: Vec<Primitive>,
}
impl Document {
    /// Rotate/mirror the selection about its first stable anchor; the whole batch validates atomically.
    pub(super) fn transform_selection(&mut self, turns: i16, mirror_x: bool, mirror_y: bool) {
        let item = self.session.snapshot();
        let Some(pivot) = self.selection.first().and_then(|id| item.objects.get(id)).map(Primitive::anchor) else { return };
        let result = self.execute(
            self.session.revision(),
            Action::Transform {
                objects: self.selection.iter().copied().collect(),
                transform: Transform { pivot, offset: Point { x_nm: 0, y_nm: 0 }, quarter_turns: turns, mirror_x, mirror_y },
            },
        );
        if let Err(error) = result {
            self.error = Some(error);
        }
    }
    /// Rebase the entire definition at the selected anchor, retaining pin/pad numbers and undo.
    pub(super) fn set_origin(&mut self) {
        let item = self.session.snapshot();
        let Some(anchor) = self.selected.and_then(|id| item.objects.get(&id)).map(Primitive::anchor) else { return };
        let offset = Point { x_nm: -anchor.x_nm, y_nm: -anchor.y_nm };
        if let Err(error) = self.execute(
            self.session.revision(),
            Action::Transform {
                objects: item.objects.keys().copied().collect(),
                transform: Transform { pivot: Point { x_nm: 0, y_nm: 0 }, offset, quarter_turns: 0, mirror_x: false, mirror_y: false },
            },
        ) {
            self.error = Some(error);
        }
    }
    /// Copy immutable primitives between library documents; the OS clipboard is not required.
    pub(super) fn copy(&self, ctx: &Context) {
        let item = self.session.snapshot();
        let primitives = self.selection.iter().filter_map(|id| item.objects.get(id).cloned()).collect::<Vec<_>>();
        if !primitives.is_empty() {
            ctx.data_mut(|data| data.insert_temp(egui::Id::unique("library_clipboard"), Clipboard { kind: item.kind, primitives }));
        }
    }
    /// Enter a cancellable paste preview. Fresh IDs and unused electrical numbers allocate only on commit.
    pub(super) fn paste(&mut self, ctx: &Context) {
        if self.pending() {
            self.error = Some("Apply or reset the property fields before pasting.".into());
            return;
        }
        let Some(clipboard) = ctx.data(|data| data.get_temp::<Clipboard>(egui::Id::unique("library_clipboard"))) else { return };
        if clipboard.kind != self.session.snapshot().kind {
            self.error = Some("Paste into a definition of the same kind.".into());
            return;
        }
        self.start_copy(clipboard.primitives);
    }
    /// Duplicate uses the same placement/transaction path as clipboard paste.
    pub(super) fn duplicate(&mut self) {
        if self.pending() {
            self.error = Some("Apply or reset the property fields before duplicating.".into());
            return;
        }
        let item = self.session.snapshot();
        self.start_copy(self.selection.iter().filter_map(|id| item.objects.get(id).cloned()).collect());
    }
    /// Capture an immutable batch and its first anchor without allocating document identities.
    fn start_copy(&mut self, primitives: Vec<Primitive>) {
        if let Some(anchor) = primitives.first().map(Primitive::anchor) {
            self.tool = canvas::Tool::Paste { primitives, anchor, revision: self.session.revision() };
        }
    }
}
/// Array parameters remain independent of the document until one validated Apply.
#[derive(Debug)]
pub(crate) struct ArrayDraft {
    document: u64,
    view: crate::ViewId,
    revision: u64,
    objects: Vec<u64>,
    columns: String,
    rows: String,
    column_pitch: String,
    row_pitch: String,
    error: Option<String>,
}
impl ArrayDraft {
    /// Capture selection and revision so later host/focus changes cannot redirect the operation.
    pub(super) fn new(document: u64, view: crate::ViewId, doc: &Document) -> Self {
        let pitch = doc.camera.snap_nm() as f64 * 2. / 1_000_000.;
        Self {
            document,
            view,
            revision: doc.session.revision(),
            objects: doc.selection.iter().copied().collect(),
            columns: "4".into(),
            rows: "1".into(),
            column_pitch: pitch.to_string(),
            row_pitch: pitch.to_string(),
            error: None,
        }
    }
    /// Resolve typed units before submitting a bounded row/column array transaction.
    fn action(&self) -> Result<Action, String> {
        let columns = self.columns.trim().parse::<u16>().map_err(|_| "Columns must be a positive whole number.")?;
        let rows = self.rows.trim().parse::<u16>().map_err(|_| "Rows must be a positive whole number.")?;
        let x = pcbcraft_chrome::units::parse_length(&self.column_pitch, pcbcraft_chrome::units::Unit::Mm).map_err(|e| e.to_string())?;
        let y = pcbcraft_chrome::units::parse_length(&self.row_pitch, pcbcraft_chrome::units::Unit::Mm).map_err(|e| e.to_string())?;
        if columns == 0 || rows == 0 || usize::from(columns) * usize::from(rows) < 2 {
            return Err("An array must contain at least two cells, including the original.".into());
        }
        Ok(Action::Array {
            objects: self.objects.clone(),
            columns,
            rows,
            column_step: Point { x_nm: x, y_nm: 0 },
            row_step: Point { x_nm: 0, y_nm: y },
        })
    }
}
impl PcbApp {
    /// Arrays apply to their captured definition/view/revision; cancellation changes no geometry or history.
    pub(crate) fn library_array_dialog(&mut self, ctx: &Context, mut draft: Box<ArrayDraft>) {
        let mut apply = false;
        let mut cancel = false;
        let dismissed = shell::dialog(ctx, "library_array", "Create Array", |ui| {
            properties::text(ui, "Columns", &mut draft.columns);
            properties::text(ui, "Rows", &mut draft.rows);
            properties::text(ui, "Column pitch (mm)", &mut draft.column_pitch);
            properties::text(ui, "Row pitch (mm)", &mut draft.row_pitch);
            if let Some(error) = &draft.error {
                ui.label(shell::error(error));
            }
            ui.horizontal(|ui| {
                apply = pcbcraft_chrome::controls::button(ui, "Create").clicked();
                cancel = pcbcraft_chrome::controls::button(ui, "Cancel").clicked();
            });
        });
        if apply {
            let result = draft.action().and_then(|action| {
                if self.bind_library_view(draft.view) != Some(draft.document) {
                    return Err("The source definition pane is no longer open.".into());
                }
                self.library_editors.edit(draft.document, draft.revision, action)
            });
            if let Err(error) = result {
                draft.error = Some(error);
                self.dialog = Dialog::LibraryArray(draft);
            } else {
                self.repaint_editors(ctx);
            }
        } else if !(cancel || dismissed) {
            self.dialog = Dialog::LibraryArray(draft);
        }
    }
}
