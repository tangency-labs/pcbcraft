//! Versioned board-folder metadata and portable paths, independent of filesystem ownership.

use crate::Project;
use std::collections::BTreeSet;
use uuid::Uuid;

/// Project-level storage choices persist independently from display names and physical data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Layout {
    pub root: String,
    pub folder: Option<String>,
}

impl Default for Layout {
    /// A new project reserves its own root namespace without creating empty directories.
    fn default() -> Self {
        Self { root: ".".into(), folder: None }
    }
}

/// Bound one lowercase portable segment, including Windows device-name restrictions.
pub fn portable_segment(segment: &str) -> bool {
    (1..=96).contains(&segment.len())
        && segment.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !segment.starts_with('-')
        && !segment.ends_with('-')
        && !matches!(
            segment,
            "con"
                | "prn"
                | "aux"
                | "nul"
                | "com1"
                | "com2"
                | "com3"
                | "com4"
                | "com5"
                | "com6"
                | "com7"
                | "com8"
                | "com9"
                | "lpt1"
                | "lpt2"
                | "lpt3"
                | "lpt4"
                | "lpt5"
                | "lpt6"
                | "lpt7"
                | "lpt8"
                | "lpt9"
        )
}

/// A layout root is the project root or one portable non-system directory segment.
pub fn valid_root(root: &str) -> bool {
    root == "." || (portable_segment(root) && !matches!(root, "sheets" | "library" | "assets" | "boards" | "builds" | "documents" | "outputs"))
}

/// Derive a bounded ASCII slug without transliterating or assigning identity from a name.
fn slug(name: &str) -> String {
    let mut slug = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-').chars().take(48).collect::<String>();
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() { "board".into() } else { slug.into() }
}

/// Initial paths use the stable UUID prefix and remain fixed through display-name changes.
pub(crate) fn initial_folder(name: &str, id: Uuid) -> String {
    format!("{}-{}", slug(name), &id.simple().to_string()[..12])
}

/// Avoid case-insensitive collisions using the full UUID; never adopt an occupied directory.
pub fn folder_name(name: &str, id: Uuid, occupied: &BTreeSet<String>) -> Result<String, String> {
    let short = initial_folder(name, id);
    let occupied = |candidate: &str| occupied.iter().any(|name| name.eq_ignore_ascii_case(candidate));
    if !occupied(&short) {
        return Ok(short);
    }
    let full = format!("{}-{id}", slug(name));
    if occupied(&full) { Err("The board folder is occupied; choose another project destination.".into()) } else { Ok(full) }
}

/// Classify only syntactically portable physical-board paths; registry validation checks ownership.
pub(crate) fn is_board_path(path: &str) -> bool {
    let parts: Vec<_> = path.split('/').collect();
    match parts.as_slice() {
        ["boards", folder, "board.json"] => portable_segment(folder),
        [root, "boards", folder, "board.json"] => *root != "." && valid_root(root) && portable_segment(folder),
        _ => false,
    }
}

impl Project {
    /// The persisted physical-data namespace, or None for an untouched flat-board project.
    pub fn layout_root(&self) -> Option<&str> {
        self.layout.as_ref().map(|layout| layout.root.as_str())
    }

    /// The sole registered board's storage segment, independent of its current display name.
    pub fn board_folder(&self) -> Option<&str> {
        self.layout.as_ref().and_then(|layout| layout.folder.as_deref())
    }

    /// Resolve physical storage through the project registry; schematic-only projects own no board file.
    pub fn board_document_path(&self) -> Option<String> {
        self.board_target()?;
        Some(match &self.layout {
            None => "board.json".into(),
            Some(layout) => {
                let folder = layout.folder.as_deref()?;
                if layout.root == "." { format!("boards/{folder}/board.json") } else { format!("{}/boards/{folder}/board.json", layout.root) }
            }
        })
    }

    /// Explicitly adopt the folder layout while preserving project/board identities and payloads.
    /// An absent historical allocation watermark is initialized from retained live/reference IDs.
    pub fn with_board_folders(&self, root: &str) -> Result<Self, String> {
        if !valid_root(root) {
            return Err("Invalid project layout root.".into());
        }
        let mut project = self.clone();
        project.layout = Some(Layout { root: root.into(), folder: self.board_folder().map(str::to_owned) });
        project.numeric_id_high_water = Some(project.allocation_high_water());
        project.refresh_board_folder();
        Ok(project)
    }

    /// Apply the save worker's validated storage choice without altering electrical state or IDs.
    pub fn with_layout_root(&self, root: &str) -> Result<Self, String> {
        if !valid_root(root) || self.layout.is_none() {
            return Err("A portable root requires an adopted board-folder layout.".into());
        }
        let mut project = self.clone();
        if let Some(layout) = &mut project.layout {
            layout.root = root.into();
        }
        Ok(project)
    }

    /// Choose an unoccupied segment during first publication, never to relocate an existing board.
    pub fn with_board_folder(&self, folder: &str) -> Result<Self, String> {
        if !portable_segment(folder) || self.layout.is_none() || self.board_target().is_none() {
            return Err("A portable folder requires an adopted physical board.".into());
        }
        let mut project = self.clone();
        if let Some(layout) = &mut project.layout {
            layout.folder = Some(folder.into());
        }
        Ok(project)
    }

    /// Fill a newly created board's path once; existing names/paths survive ordinary edits and renames.
    pub(crate) fn refresh_board_folder(&mut self) {
        let folder = self.boards().next().map(|board| initial_folder(board.name(), board.target.board));
        if let Some(layout) = &mut self.layout {
            if folder.is_none() {
                layout.folder = None;
            } else if layout.folder.is_none() {
                layout.folder = folder;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Slugs, reserved names, Unicode-only names and case collisions remain portable.
    #[test]
    fn portable_names_and_collision_fallbacks() {
        let id = Uuid::from_u128(0x123456789abcdef0123456789abcdef0);
        for (name, slug) in [("CON", "con"), ("aux", "aux"), ("日本語", "board"), ("  Main / PCB  ", "main-pcb")] {
            let folder = folder_name(name, id, &BTreeSet::new()).unwrap();
            assert_eq!(folder, format!("{slug}-123456789abc"));
            assert!(portable_segment(&folder));
            let mut occupied = BTreeSet::from([folder.to_uppercase()]);
            let full = folder_name(name, id, &occupied).unwrap();
            assert_eq!(full, format!("{slug}-{id}"));
            occupied.insert(full.to_uppercase());
            assert!(folder_name(name, id, &occupied).is_err());
        }
        assert!(portable_segment(&initial_folder(&"a".repeat(200), id)));
        for name in ["con", "aux", "nul", "com1", "lpt9", "../board", "A", "", "a/b", "-a", "a-"] {
            assert!(!portable_segment(name), "{name}");
        }
        for root in ["boards", "sheets", "../data", "a/b", "CON", "outputs"] {
            assert!(!valid_root(root));
        }
        assert!(valid_root("."));
        assert!(valid_root("pcbcraft-data-1234"));
    }
}

#[cfg(test)]
mod migration_tests {
    use crate::{
        Session,
        persistence::{ENTRY_POINT, decode, decode_documents, encode, encode_documents, migrate_board_layout},
    };
    use serde_json::json;
    use std::sync::Arc;

    /// Legacy cross-kind UUID collisions remain typed compatibility identities after folder migration.
    #[test]
    fn migration_preserves_the_legacy_board_id_even_when_a_sheet_shares_it() {
        let mut project = decode(include_bytes!("../../../app/tests/fixtures/two-resistors.pc")).unwrap();
        let board = project.board_target().unwrap();
        Arc::make_mut(&mut Arc::make_mut(&mut project).sheets)[0].sheet_id = board.board;
        let project = decode(&encode(project).unwrap()).unwrap();
        let before = encode_documents(project).unwrap();
        let after = migrate_board_layout(before.clone(), ".").unwrap();
        let loaded = decode_documents(&after).unwrap();
        assert_eq!(loaded.board_target(), Some(board));
        assert_eq!(loaded.sheets().next().unwrap().target.sheet, board.board);
        assert_eq!(after[&loaded.board_document_path().unwrap()], before["board.json"]);
        assert_eq!(encode_documents(loaded).unwrap(), after);
    }

    /// Save metadata follows the project history while board creation remains undoable.
    #[test]
    fn undoing_board_creation_retains_root_without_a_dummy_board_folder() {
        let mut s = Session::default();
        s.apply("project.new", &json!({})).unwrap();
        let original = s.snapshot().unwrap();
        let saved = Arc::new(original.with_layout_root("pcbcraft-data-test").unwrap());
        s.apply("pcb.create_board", &json!({"expected_revision":s.revision()})).unwrap();
        let target = s.project().unwrap().board_target();
        assert!(s.adopt_saved_metadata(&original, &saved));
        assert_eq!(s.project().unwrap().board_target(), target);
        assert_eq!(s.project().unwrap().layout_root(), Some("pcbcraft-data-test"));
        assert!(s.project().unwrap().board_folder().is_some());
        s.apply("edit.undo", &json!({})).unwrap();
        assert_eq!(s.project().unwrap().layout_root(), Some("pcbcraft-data-test"));
        assert!(s.project().unwrap().board_folder().is_none());
        s.apply("edit.redo", &json!({})).unwrap();
        let original = s.snapshot().unwrap();
        let saved = Arc::new(original.with_board_folder("chosen-board").unwrap());
        s.adopt_saved_metadata(&original, &saved);
        s.apply("edit.undo", &json!({})).unwrap();
        assert!(s.project().unwrap().board_folder().is_none());
    }

    /// Each supported explicit flat vocabulary migrates without losing metadata or package intent.
    #[test]
    fn flat_registry_metadata_and_schematic_packages_migrate_to_fourteen() {
        for version in [9, 12, 13] {
            let mut session = Session::default();
            session.apply("project.new", &json!({"create_board":version != 13})).unwrap();
            session.apply(crate::PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":session.revision()})).unwrap();
            if version == 12 {
                let mut metadata = crate::schematic::fields::Metadata::default();
                metadata.attributes.do_not_populate = true;
                session
                    .apply(
                        crate::SET_SYMBOL_FIELDS,
                        &json!({"symbol":"symbol-2","reference":"R1","value":"10k","metadata":metadata,"expected_revision":session.revision()}),
                    )
                    .unwrap();
            }
            if version == 13 {
                let component = session.project().unwrap().components().next().unwrap().id();
                session.apply(crate::ASSIGN_FOOTPRINT, &json!({"component":component,"library":"starter","definition":"resistor-0402","definition_revision":4,"mapping":{"1":"2","2":"1"},"expected_revision":session.revision()})).unwrap();
            }
            // Recreate a historical storage mode in memory, then use production codecs throughout.
            let mut historical = session.snapshot().unwrap();
            Arc::make_mut(&mut historical).layout = None;
            let before = encode_documents(historical.clone()).unwrap();
            let header: serde_json::Value = serde_json::from_slice(&before[ENTRY_POINT]).unwrap();
            assert_eq!(header["format"], format!("pcbcraft-project/{version}"));
            let after = migrate_board_layout(before.clone(), ".").unwrap();
            let loaded = decode_documents(&after).unwrap();
            assert_eq!(loaded.board_target(), historical.board_target());
            assert_eq!(loaded.inspect_schematic(), historical.inspect_schematic());
            assert_eq!(loaded.footprint_assignments(), historical.footprint_assignments());
            for (path, bytes) in &before {
                if path == ENTRY_POINT {
                    continue;
                }
                let target = if path == "board.json" { loaded.board_document_path().unwrap() } else { path.clone() };
                assert_eq!(&after[&target], bytes);
            }
        }
    }

    /// Inspection reports the same path the codec writes, including a nondefault storage root.
    #[test]
    fn inspected_board_paths_follow_the_registered_layout() {
        let historical = decode(include_bytes!("../../../app/tests/fixtures/two-resistors.pc")).unwrap();
        for root in [None, Some("."), Some("pcbcraft-data-test")] {
            let project = match root {
                None => historical.clone(),
                Some(root) => Arc::new(historical.with_board_folders(root).unwrap()),
            };
            let path = project.board_document_path().unwrap();
            let mut session = Session::default();
            session.open_snapshot(project.clone());
            assert_eq!(session.inspect()["project"]["boards"][0]["file"], path);
            assert!(encode_documents(project).unwrap().contains_key(&path));
        }
    }

    /// Unsupported/mixed vocabularies, unsafe paths and extra physical data are never adopted.
    #[test]
    fn folder_codec_refuses_unsafe_roots_folders_and_unregistered_documents() {
        let mut s = Session::default();
        s.apply("project.new", &json!({"create_board":true})).unwrap();
        let docs = encode_documents(s.snapshot().unwrap()).unwrap();
        let entry: serde_json::Value = serde_json::from_slice(&docs[ENTRY_POINT]).unwrap();
        for bad in ["../outside", "boards", "outputs", "con", "a/b", "A"] {
            let mut header = entry.clone();
            header["layout_root"] = json!(bad);
            let mut malformed = docs.clone();
            malformed.insert(ENTRY_POINT.into(), serde_json::to_vec(&header).unwrap());
            assert!(decode_documents(&malformed).is_err(), "{bad}");
        }
        for bad in ["../outside", "aux", "con", "a/b", "A"] {
            let mut header = entry.clone();
            header["boards"][0]["folder"] = json!(bad);
            let mut malformed = docs.clone();
            malformed.insert(ENTRY_POINT.into(), serde_json::to_vec(&header).unwrap());
            assert!(decode_documents(&malformed).is_err(), "{bad}");
        }
        let mut extra = docs.clone();
        extra.insert("boards/foreign/board.json".into(), b"{}".to_vec());
        assert!(decode_documents(&extra).is_err());
        let mut older = entry;
        older["format"] = json!("pcbcraft-project/12");
        let mut malformed = docs;
        malformed.insert(ENTRY_POINT.into(), serde_json::to_vec(&older).unwrap());
        assert!(decode_documents(&malformed).is_err());
    }
}
