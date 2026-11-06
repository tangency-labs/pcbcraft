//! Immutable, board-addressed presentation and correspondence over the native model.

use crate::{Project, schematic::Point, selection::Selection};
use std::collections::BTreeSet;
use uuid::Uuid;

/// A board address distinguishes its project and physical board, independently of names,
/// stackup identity and runtime opening. Legacy files have one deterministically addressed board.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BoardTarget {
    pub project: Uuid,
    pub board: Uuid,
}

/// Derive the legacy board identity from a fixed domain and saved project UUID. This compatibility
/// address survives no-op saves without silently migrating files; a future explicit board list
/// must preserve it when migrating. It is not a rule for generating identities of additional boards.
fn legacy_board_id(project: Uuid) -> Uuid {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(b"pcbcraft:legacy-board:1\0");
    hash.update(project.as_bytes());
    let digest = hash.finalize();
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    // UUID v8: application-defined identity with the standard variant.
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

/// Resolved board data scoped to an explicitly validated target, independent of the GUI.
#[derive(Clone, Copy)]
pub struct BoardView<'a> {
    project: &'a Project,
    pub target: BoardTarget,
}

/// A physical pad center within the containing board and footprint. Shape and drill are absent
/// from the legacy placement model; viewers must not treat markers as fabrication geometry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PadView {
    pub number: String,
    pub center: Point,
}

/// A placement resolved in one addressed board, including its logical component association.
#[derive(Clone, Debug)]
pub struct FootprintView<'a> {
    pub board: BoardTarget,
    pub id: String,
    pub component: String,
    pub reference: &'a str,
    pub value: &'a str,
    pub definition: &'a str,
    pub position: Point,
    pub pads: Vec<PadView>,
}

impl Project {
    /// Enumerate available boards in document order. The legacy representation has one board;
    /// callers should retain its target rather than infer identity from list position or name.
    pub fn boards(&self) -> impl Iterator<Item = BoardView<'_>> {
        std::iter::once(BoardView { project: self, target: self.board_target() })
    }

    /// Compatibility default for single-board callers. Additional-board support must replace
    /// implicit resolution with a unique-board check, never silently pick the first board.
    pub fn board_target(&self) -> BoardTarget {
        BoardTarget { project: self.id, board: legacy_board_id(self.id) }
    }

    /// Resolve exactly this board in this project; unknown or foreign targets never fall back.
    pub fn board_view(&self, target: BoardTarget) -> Option<BoardView<'_>> {
        self.boards().find(|board| board.target == target)
    }

    /// Validate an explicit board ID, retaining the legacy single-board default on omission.
    pub fn resolve_board(&self, id: Option<&str>) -> Result<BoardTarget, crate::CommandError> {
        let default = self.board_target();
        match id {
            None => Ok(default),
            Some(id) if id == default.board.to_string() => Ok(default),
            Some(_) => Err(crate::CommandError::InvalidParams("unknown board ID".to_owned())),
        }
    }
}

impl<'a> BoardView<'a> {
    /// Display name of the legacy board; identity never depends on this label.
    pub fn name(self) -> &'a str {
        "Board"
    }

    /// The addressed physical board's setup, including local layers and material definitions.
    pub fn setup(self) -> Option<&'a super::setup::BoardSetup> {
        self.project.board.setup.as_ref()
    }

    /// Resolve placements without assuming starter-definition geometry in a renderer.
    pub fn footprints(self) -> impl Iterator<Item = FootprintView<'a>> {
        self.project.board.footprints().iter().filter_map(move |footprint| {
            let component = self.project.components.get(&footprint.component)?;
            Some(FootprintView {
                board: self.target,
                id: format!("footprint-{}", footprint.id),
                component: component.id(),
                reference: component.reference(),
                value: component.value(),
                definition: &footprint.definition,
                position: Point { x_nm: footprint.x_nm, y_nm: footprint.y_nm },
                pads: (1..=2).map(|pad| PadView { number: pad.to_string(), center: footprint.pad_position(pad) }).collect(),
            })
        })
    }

    /// Project schematic component selection into this board's placement IDs. Wires and missing
    /// assignments produce no match; reference text and matching object ordinals are not links.
    pub fn footprints_for_selection(self, selection: &Selection) -> BTreeSet<String> {
        let components: BTreeSet<_> =
            self.project.symbols().iter().filter(|symbol| selection.symbols.contains(&symbol.id())).map(|symbol| symbol.component).collect();
        self.project
            .board
            .footprints()
            .iter()
            .filter(|footprint| components.contains(&footprint.component))
            .map(|footprint| format!("footprint-{}", footprint.id))
            .collect()
    }

    /// Resolve this board's selected placements to schematic units through their shared component.
    /// The result is a highlight suggestion, not authorization to edit those schematic units.
    pub fn selection_for_footprints(self, footprints: &BTreeSet<String>) -> Selection {
        let components: BTreeSet<_> = self
            .project
            .board
            .footprints()
            .iter()
            .filter(|footprint| footprints.contains(&format!("footprint-{}", footprint.id)))
            .map(|footprint| footprint.component)
            .collect();
        Selection {
            symbols: self.project.symbols().iter().filter(|symbol| components.contains(&symbol.component)).map(|symbol| symbol.id()).collect(),
            ..Selection::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Session,
        persistence::{decode_documents, encode_documents},
    };
    use serde_json::json;

    /// Lock the published compatibility derivation so a refactor cannot silently change addresses.
    #[test]
    fn legacy_board_identity_has_a_fixed_compatibility_value() {
        assert_eq!(legacy_board_id(Uuid::from_u128(1)).to_string(), "f42e536f-9ed5-8fdc-bcec-20fab0ad35a9");
    }

    /// Board IDs survive rename, ordinary edits and no-op round trips; scoped views and pads
    /// cannot resolve the same object ordinal in another project.
    #[test]
    fn board_addresses_and_correspondence_survive_edits_and_storage() {
        let mut session = Session::default();
        session.apply("project.new", &json!({})).unwrap();
        let target = session.project().unwrap().board_target();
        session.apply(crate::PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":1})).unwrap();
        let selected = Selection::one_symbol("symbol-2".into());
        assert!(session.project().unwrap().board_view(target).unwrap().footprints_for_selection(&selected).is_empty());
        session.apply(crate::PLACE_FOOTPRINT, &json!({"board":target.board,"symbol":"symbol-2","definition":"generic-through-hole-2","pin_1_pad":2,"x_nm":20_000_000,"y_nm":0,"expected_revision":2})).unwrap();
        session.apply("project.rename", &json!({"name":"Renamed"})).unwrap();
        session.apply(crate::SET_SYMBOL_FIELDS, &json!({"symbol":"symbol-2","reference":"R20","value":"22k","expected_revision":4})).unwrap();
        let snapshot = session.snapshot().unwrap();
        let bytes = encode_documents(snapshot.clone()).unwrap();
        let restored = decode_documents(&bytes).unwrap();
        assert_eq!(restored.board_target(), target);
        assert_eq!(encode_documents(restored.clone()).unwrap(), bytes);
        let board = restored.board_view(target).unwrap();
        let ids = board.footprints_for_selection(&selected);
        assert_eq!(ids, BTreeSet::from(["footprint-3".into()]));
        assert_eq!(board.selection_for_footprints(&ids), selected);
        assert_eq!(board.footprints().next().unwrap().reference, "R20");
        assert_eq!(board.footprints().next().unwrap().board, target);
        let pad = restored.pad_for_terminal(restored.symbols()[0].terminals()[0]).unwrap();
        assert_eq!(pad.inspect()["board"], target.board.to_string());
        let before = session.inspect();
        for (command, params) in [
            (crate::PLACE_FOOTPRINT, json!({"symbol":"symbol-2","definition":"generic-through-hole-2","pin_1_pad":1,"x_nm":0,"y_nm":0})),
            (crate::INITIALIZE_BOARD_SETUP, json!({"copper_layers":2})),
            (crate::SET_BOARD_SETUP, json!({"setup":{}})),
        ] {
            let mut params = params;
            params["board"] = json!("00000000-0000-0000-0000-000000000001");
            params["expected_revision"] = json!(session.revision());
            assert!(session.apply(command, &params).unwrap_err().to_string().contains("unknown board"));
            assert_eq!(session.inspect(), before);
        }
        session.apply("project.new", &json!({})).unwrap();
        let other = session.project().unwrap();
        assert_ne!(other.board_target(), target);
        assert!(other.board_view(target).is_none());
        assert!(other.terminal_for_pad(pad).is_none());
        assert!(other.pad_position(pad).is_none());
    }
}
