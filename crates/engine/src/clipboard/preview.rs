//! Stable transient identities and cached logical work for schematic clipboard motion.
use super::*;

/// A disposable schematic preview caches child-sheet creation, identity/reference/package mapping
/// and validation once. Cursor motion transforms only introduced root geometry; the host must
/// still submit the original portable text to the revision-checked paste command for publication.
#[derive(Clone, Debug)]
pub struct SchematicPastePreview {
    project: Arc<Project>,
    anchor: Point,
    selected: Selection,
    junctions: BTreeSet<u64>,
    labels: BTreeSet<u64>,
    blocks: BTreeSet<u64>,
}
impl Fragment {
    /// Prepare the complete logical candidate once with stable transient sheet and object IDs.
    /// Whole-group rotation is applied by the preview's at method, independently of preparation.
    pub fn preview(&self, destination: &Project, first_id: u64, options: &PasteOptions) -> Result<SchematicPastePreview, CommandError> {
        let mut policy = options.clone();
        policy.quarter_turns = 0;
        let (project, selected) = self.placed(destination, self.anchor, first_id, &policy)?;
        let junctions = project.junctions.keys().filter(|id| !destination.junctions.contains_key(id)).copied().collect();
        let labels = project.labels.iter().filter(|l| !destination.labels.iter().any(|old| old.id == l.id)).map(|l| l.id).collect();
        let blocks = project.subsheets.iter().filter(|b| !destination.subsheets.iter().any(|old| old.id == b.id)).map(|b| b.id).collect();
        Ok(SchematicPastePreview { project: Arc::new(project), anchor: self.anchor, selected, junctions, labels, blocks })
    }
}
impl SchematicPastePreview {
    /// Introduced flat objects on the addressed destination root, excluding child-sheet contents.
    pub fn selection(&self) -> &Selection {
        &self.selected
    }
    /// A new root sheet symbol can be highlighted without using display names as identities.
    pub fn sheet_symbol(&self) -> Option<u64> {
        self.blocks.first().copied()
    }
    /// Translate/rotate introduced root geometry only, retaining cached child sheets, IDs and
    /// package/reference choices. Sheet symbols retain their model-supported orientation.
    pub fn at(&self, at: Point, turns: u8) -> Result<Arc<Project>, CommandError> {
        if (!self.blocks.is_empty() || !self.labels.is_empty()) && !turns.is_multiple_of(4) {
            return Err(invalid("hierarchical sheet symbols and interfaces retain their orientation"));
        }
        let mut next = (*self.project).clone();
        let sheet = next.sheet_mut();
        for symbol in Arc::make_mut(&mut sheet.symbols) {
            if self.selected.symbols.contains(&symbol.id()) {
                symbol.position = transformed(symbol.position, self.anchor, at, turns)?;
                if symbol.position.x_nm.abs() > crate::schematic::MAX_CENTER_NM || symbol.position.y_nm.abs() > crate::schematic::MAX_CENTER_NM {
                    return Err(invalid("pasted symbol exceeds sheet bounds"));
                }
                symbol.rotation = (symbol.rotation + turns % 4) % 4;
            }
        }
        for (id, point) in Arc::make_mut(&mut sheet.junctions) {
            if self.junctions.contains(id) {
                *point = transformed(*point, self.anchor, at, turns)?;
            }
        }
        for label in Arc::make_mut(&mut sheet.net_labels) {
            if self.selected.net_labels.contains(&label.object_id()) {
                label.at = transformed(label.at, self.anchor, at, turns)?;
                label.rotation = (label.rotation + u16::from(turns % 4) * 90) % 360;
            }
        }
        for label in Arc::make_mut(&mut sheet.labels) {
            if self.labels.contains(&label.id) {
                label.at = transformed(label.at, self.anchor, at, turns)?;
            }
        }
        for block in Arc::make_mut(&mut sheet.subsheets) {
            if self.blocks.contains(&block.id) {
                block.at = transformed(block.at, self.anchor, at, 0)?;
            }
        }
        for wire in Arc::make_mut(&mut sheet.wires) {
            if self.selected.wires.contains(&wire.id()) {
                wire.bends =
                    wire.bends.as_ref().map(|points| points.iter().map(|p| transformed(*p, self.anchor, at, turns)).collect()).transpose()?;
            }
        }
        sheet.connectivity = Arc::default();
        next.named_graph = Arc::default();
        next.validate_hierarchy().map_err(invalid)?;
        Ok(Arc::new(next))
    }
}
