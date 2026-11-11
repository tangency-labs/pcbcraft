use super::*;
use crate::{
    board::track::tests::{routed_fixture, view},
    persistence::{decode_documents, encode_documents},
};

/// Submit appearance through the same addressed command as the Properties card.
fn set(session: &mut Session, footprint: &str, appearance: Value) -> Result<u64, CommandError> {
    session.apply(
        SET_REFERENCE_TEXT,
        &json!({"board":view(session).target.board,"footprint":footprint,"appearance":appearance,"expected_revision":session.revision()}),
    )
}

/// The external saved-data schema and command discovery describe identical appearance fields.
#[test]
fn print_schema_matches_command_discovery() {
    let document: Value = serde_json::from_str(include_str!("../../../../../docs/schemas/footprint-placement-v15.schema.json")).unwrap();
    let mut appearance = Text::schema();
    appearance["type"] = json!("object");
    assert_eq!(document["properties"]["reference_text"], appearance);
}

/// A text-size override leaves the board-styled body extent and automatic bottom-ink anchor intact.
#[test]
fn local_stroke_keeps_the_automatic_anchor_above_the_actual_body() {
    let session = routed_fixture();
    let project = session.project().unwrap();
    let definition = project.board.definition_for(&project.board.footprints()[0]).unwrap();
    let style = view(&session).silkscreen();
    let bottom = |paths: Vec<Vec<Point>>, stroke: i64| paths.iter().flatten().map(|point| point.y_nm).max().unwrap() + (stroke + 1) / 2;
    let original = bottom(definition.reference_paths("R1", style, super::super::DEFAULT_REFERENCE_OFFSET), style.stroke_width_nm);
    for stroke in [50_000, 150_000] {
        let text = Text { size: Some(Size { height_nm: 1_500_000, stroke_width_nm: stroke }), ..Default::default() };
        let paths = definition.reference_paths_with("R1", style, super::super::DEFAULT_REFERENCE_OFFSET, Some(&text));
        assert_eq!(bottom(paths, stroke), original);
    }
}

/// Local print edits affect board data and exported geometry while preserving every component and pad.
#[test]
fn printed_words_and_style_are_local_and_reset_restores_inheritance() {
    let mut session = routed_fixture();
    crate::board::drc::tests::outline(&mut session);
    let outputs = view(&session).fabrication_outputs().unwrap();
    let original = encode_documents(session.snapshot().unwrap()).unwrap();
    let schematic = session.inspect()["project"]["schematic"].clone();
    let footprint = view(&session).footprints().next().unwrap();
    let id = footprint.id.clone();
    let reference = footprint.reference.to_owned();
    let pads = footprint.pads;
    let origin = footprint.position;
    set(&mut session, &id, json!({"text":"Power input","font":"pcbcraft-technical-1","size":{"height_nm":1_500_000,"stroke_width_nm":150_000},"rotation":90,"align":"start","visible":true})).unwrap();
    let changed = view(&session).footprints().find(|f| f.id == id).unwrap();
    assert_eq!(changed.reference, reference);
    assert_eq!(changed.reference_appearance.text, "Power input");
    assert_eq!(changed.reference_appearance.size.height_nm, 1_500_000);
    assert_eq!(changed.reference_appearance.rotation, 90);
    assert_eq!(changed.pads, pads);
    assert_eq!(changed.position, origin);
    assert_eq!(session.inspect()["project"]["schematic"], schematic);
    assert!(changed.graphics.iter().any(|graphic| graphic.reference));
    assert!(changed.graphics.iter().filter(|graphic| graphic.reference).all(|graphic| graphic.stroke_nm == Some(150_000)));
    let changed_outputs = view(&session).fabrication_outputs().unwrap();
    let changed_files: Vec<_> =
        outputs.iter().zip(&changed_outputs).filter(|(before, after)| before != after).map(|(_, after)| after.name.as_str()).collect();
    assert_eq!(changed_files.len(), 1);
    assert!(changed_files[0].ends_with("-Silkscreen_Top.gbr"));
    let saved = encode_documents(session.snapshot().unwrap()).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&saved[&crate::persistence::test_board_path(&saved)]).unwrap()["format"], "pcbcraft-board/15");
    assert_eq!(
        saved.iter().filter(|(path, bytes)| original.get(*path) != Some(*bytes)).map(|(path, _)| path.as_str()).collect::<Vec<_>>(),
        [crate::persistence::test_board_path(&saved)]
    );
    assert_eq!(encode_documents(decode_documents(&saved).unwrap()).unwrap(), saved);
    let mut older = saved.clone();
    let mut document: Value = serde_json::from_slice(&older[&crate::persistence::test_board_path(&older)]).unwrap();
    document["format"] = json!("pcbcraft-board/14");
    older.insert(crate::persistence::test_board_path(&older), serde_json::to_vec(&document).unwrap());
    assert!(decode_documents(&older).is_err());
    set(&mut session, &id, Value::Null).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), original);
    session.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), saved);
}

/// A literal equal to today's designator stays literal after an explicit shared rename; Reset follows the new name.
#[test]
fn literal_print_text_and_component_rename_have_independent_ownership() {
    let mut session = routed_fixture();
    let footprint = view(&session).footprints().next().unwrap();
    let id = footprint.id.clone();
    let literal = footprint.reference.to_owned();
    let value = footprint.value.to_owned();
    let component = session.project().unwrap().board.footprints()[0].component;
    let symbol = session.project().unwrap().all_symbols().find(|symbol| symbol.component == component).unwrap().id();
    set(&mut session, &id, json!({"text":literal})).unwrap();
    session
        .apply(crate::SET_SYMBOL_FIELDS, &json!({"symbol":symbol,"reference":"R99","value":value,"expected_revision":session.revision()}))
        .unwrap();
    let changed = view(&session).footprints().find(|f| f.id == id).unwrap();
    assert_eq!(changed.reference, "R99");
    assert_eq!(changed.reference_appearance.text, literal);
    let duplicate = view(&session).footprints().find(|f| f.id != id).unwrap().reference.to_owned();
    let saved = encode_documents(session.snapshot().unwrap()).unwrap();
    let revision = session.revision();
    assert!(
        session.apply(crate::SET_SYMBOL_FIELDS, &json!({"symbol":symbol,"reference":duplicate,"value":value,"expected_revision":revision})).is_err()
    );
    assert_eq!(session.revision(), revision);
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), saved);
    set(&mut session, &id, Value::Null).unwrap();
    assert_eq!(view(&session).footprints().find(|f| f.id == id).unwrap().reference_appearance.text, "R99");
}

/// Invalid appearance, stale authority and malformed offsets never partially change a placement.
#[test]
fn invalid_print_edits_are_atomic() {
    let mut session = routed_fixture();
    let id = view(&session).footprints().next().unwrap().id.clone();
    let original = encode_documents(session.snapshot().unwrap()).unwrap();
    let revision = session.revision();
    for appearance in [
        json!({}),
        json!({"text":""}),
        json!({"text":"R1\n"}),
        json!({"text":"x".repeat(129)}),
        json!({"font":"unknown"}),
        json!({"rotation":360}),
        json!({"visible":"yes"}),
        json!({"size":{"height_nm":1_000_000,"stroke_width_nm":600_000}}),
        json!({"size":{"height_nm":0,"stroke_width_nm":1}}),
        json!({"align":"unknown"}),
        json!({"text":null,"visible":false}),
        json!({"size":null,"rotation":90}),
    ] {
        assert!(set(&mut session, &id, appearance).is_err());
        assert_eq!(session.revision(), revision);
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), original);
    }
    let params = json!({"footprint":id,"appearance":{"text":"Local"},"offset":{"x_nm":100_000_001,"y_nm":0},"expected_revision":revision});
    assert!(session.apply(SET_REFERENCE_TEXT, &params).is_err());
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), original);
    let mut stale = params;
    stale["offset"] = json!({"x_nm":0,"y_nm":0});
    stale["expected_revision"] = json!(revision - 1);
    assert!(session.apply(SET_REFERENCE_TEXT, &stale).is_err());
    assert_eq!(session.revision(), revision);
}

/// Hiding only generated ink retains the placement, and local sizes survive board-wide style changes.
#[test]
fn visibility_and_local_size_resolve_without_freezing_other_placements() {
    let mut session = routed_fixture();
    let id = view(&session).footprints().next().unwrap().id.clone();
    set(&mut session, &id, json!({"size":{"height_nm":1_500_000,"stroke_width_nm":150_000},"visible":false})).unwrap();
    session.apply(crate::SET_SILKSCREEN, &json!({"settings":{"text_height_nm":2_000_000,"stroke_width_nm":200_000,"pad_clearance_nm":250_000},"expected_revision":session.revision()})).unwrap();
    let selected = view(&session).footprints().find(|f| f.id == id).unwrap();
    assert!(selected.physical_reference);
    assert!(!selected.reference_appearance.visible);
    assert!(!selected.graphics.iter().any(|graphic| graphic.reference));
    assert_eq!(selected.reference_appearance.size.height_nm, 1_500_000);
    assert_eq!(view(&session).footprints().find(|f| f.id != id).unwrap().reference_appearance.size.height_nm, 2_000_000);
}

/// Nonphysical overrides retain the editor annotation; only explicitly enabled ink replaces it.
#[test]
fn fontless_footprints_keep_editor_labels_until_local_ink_is_enabled() {
    let mut session = routed_fixture();
    // Model a valid pinned native definition from before generated references were enabled.
    // All placements, mappings and connectivity come from the command-built fixture.
    let mut project = (*session.snapshot().unwrap()).clone();
    let key = project.board.footprints()[0].key().unwrap();
    let board = Arc::make_mut(&mut project.board);
    Arc::make_mut(board.definitions.get_mut(&key).unwrap()).reference = None;
    session.open_snapshot(Arc::new(project));
    let initial = view(&session).footprints().next().unwrap();
    let id = initial.id.clone();
    let pads = initial.pads.clone();
    let reference = initial.reference.to_owned();
    assert!(!initial.physical_reference);
    for appearance in [
        json!({"text":"Local words"}),
        json!({"font":"pcbcraft-technical-1"}),
        json!({"visible":false}),
        json!({"text":"Local words","visible":true}),
        json!({"text":"Local words","visible":false}),
        Value::Null,
    ] {
        let ink = appearance["visible"] == true;
        set(&mut session, &id, appearance).unwrap();
        let footprint = view(&session).footprints().find(|f| f.id == id).unwrap();
        assert_eq!(footprint.physical_reference, ink, "Nonphysical changes must leave the editor label available");
        assert_eq!(footprint.graphics.iter().any(|graphic| graphic.reference), ink);
        assert_eq!(footprint.reference, reference);
        assert_eq!(footprint.pads, pads);
        let saved = encode_documents(session.snapshot().unwrap()).unwrap();
        let reopened = decode_documents(&saved).unwrap();
        assert_eq!(encode_documents(reopened.clone()).unwrap(), saved);
        let loaded = reopened.board_view(reopened.board_target().unwrap()).unwrap().footprints().find(|f| f.id == id).unwrap();
        assert_eq!(loaded.physical_reference, ink);
    }
}
