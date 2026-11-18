use super::*;
use crate::persistence::{Documents, ENTRY_POINT, decode_documents, decode_packed, encode_documents, encode_packed};

const ID: Uuid = Uuid::from_u128(0x123);

/// Keep board identity and layer choice tied to the project created by the shared command path.
fn fixture() -> (Session, Value) {
    let mut session = Session::default();
    session.apply("project.new", &json!({"name":"Recipe", "create_board":true})).unwrap();
    let board = session.project().unwrap().boards().next().unwrap();
    let content = json!({"name":"Prototype", "board":board.target.board, "units":"mm", "file_stem":"prototype",
        "gerber_layers":[board.layers()[0].id], "drill":true,
        "origin":{"kind":"point","x_nm":100,"y_nm":-200},
        "assembly_profile":{"profile":Uuid::from_u128(12),"revision":2}});
    (session, content)
}

/// Edit a recipe using the current revision rather than changing the project directly.
fn set(session: &mut Session, id: Uuid, content: &Value) -> Result<u64, CommandError> {
    session.apply(SET_OUTPUT_RECIPE, &json!({"recipe":id,"expected_revision":session.revision(),"content":content}))
}

/// Change the stored envelope for refusal cases; the normal fixture is command-generated.
fn replace_entry(documents: &Documents, edit: impl FnOnce(&mut Value)) -> Documents {
    let mut modified = documents.clone();
    let mut entry: Value = serde_json::from_slice(&modified[ENTRY_POINT]).unwrap();
    edit(&mut entry);
    modified.insert(ENTRY_POINT.to_owned(), serde_json::to_vec(&entry).unwrap());
    modified
}

/// A recipe round-trips in both codecs. Creating and removing it affect only the entry point.
#[test]
fn recipe_round_trips_and_undo_restores_recipe_free_bytes() {
    let (mut session, content) = fixture();
    let original = session.snapshot().unwrap();
    let empty = encode_documents(original.clone()).unwrap();
    let empty_archive = encode_packed(original).unwrap();
    assert_eq!(encode_documents(decode_documents(&empty).unwrap()).unwrap(), empty);
    assert!(session.inspect()["project"].get("output_recipes").is_none());

    let revision = set(&mut session, ID, &content).unwrap();
    assert_eq!(set(&mut session, ID, &content).unwrap(), revision);
    let project = session.snapshot().unwrap();
    let saved = encode_documents(project.clone()).unwrap();
    assert_eq!(saved.len(), empty.len());
    for (path, bytes) in &empty {
        if path != ENTRY_POINT {
            assert_eq!(&saved[path], bytes, "{path}");
        }
    }
    let entry: Value = serde_json::from_slice(&saved[ENTRY_POINT]).unwrap();
    assert_eq!(entry["format"], "pcbcraft-project/47");
    assert_eq!(entry["output_recipes"], json!([{"id":ID,"content":content}]));
    assert_eq!(session.inspect()["project"]["output_recipes"], entry["output_recipes"]);
    let reopened = decode_documents(&saved).unwrap();
    assert_eq!(reopened.output_recipes(), project.output_recipes());
    assert_eq!(encode_documents(reopened).unwrap(), saved);
    let packed = decode_packed(&encode_packed(project.clone()).unwrap()).unwrap();
    assert_eq!(packed.project.output_recipes(), project.output_recipes());
    assert_eq!(packed.documents, saved);
    assert!(crate::persistence::encode(project).err().unwrap().contains("single-file encoding cannot represent output recipes"));

    session.apply("edit.undo", &Value::Null).unwrap();
    assert!(session.project().unwrap().output_recipes().is_empty());
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), empty);
    assert_eq!(encode_packed(session.snapshot().unwrap()).unwrap(), empty_archive);
    session.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), saved);
    let mut amended = content;
    amended["units"] = json!("in");
    set(&mut session, ID, &amended).unwrap();
    assert_eq!(session.project().unwrap().output_recipes()[0].content.units, Units::In);
    session.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), saved);
    set(&mut session, ID, &Value::Null).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), empty);
}

/// Old headers refuse the new field; the new header requires a nonempty, typed recipe array.
#[test]
fn recipe_reader_refuses_wrong_header_and_invalid_records() {
    let (mut session, content) = fixture();
    let original = encode_documents(session.snapshot().unwrap()).unwrap();
    let old_header: Value = serde_json::from_slice::<Value>(&original[ENTRY_POINT]).unwrap()["format"].clone();
    set(&mut session, ID, &content).unwrap();
    let saved = encode_documents(session.snapshot().unwrap()).unwrap();
    for header in [old_header, json!("pcbcraft-project/36"), json!("pcbcraft-project/39"), json!("pcbcraft-project/45"), json!("pcbcraft-project/46")] {
        let invalid = replace_entry(&saved, |entry| entry["format"] = header);
        assert!(decode_documents(&invalid).err().unwrap().contains("earlier formats do not allow"));
    }
    let missing = replace_entry(&saved, |entry| {
        entry.as_object_mut().unwrap().remove("output_recipes");
    });
    assert!(decode_documents(&missing).err().unwrap().contains("requires a nonempty output_recipes array"));
    let duplicate = json!([{"id":ID,"content":content},{"id":ID,"content":content}]);
    for (recipes, expected) in [
        (Value::Null, "invalid type: null"),
        (json!([]), "requires a nonempty output_recipes array"),
        (duplicate, "distinct non-nil IDs"),
        (json!([{"id":Uuid::nil(),"content":content}]), "distinct non-nil IDs"),
    ] {
        let invalid = replace_entry(&saved, |entry| entry["output_recipes"] = recipes);
        assert!(decode_documents(&invalid).err().unwrap().contains(expected));
    }
    for (field, value, expected) in [
        ("board", json!(Uuid::from_u128(9)), "target the registered board"),
        ("units", json!({"mm":null}), "expected a string"),
        ("destination", json!("path"), "unknown field `destination`"),
    ] {
        let invalid = replace_entry(&saved, |entry| entry["output_recipes"][0]["content"][field] = value);
        assert!(decode_documents(&invalid).err().unwrap().contains(expected));
    }
}

/// Intrinsic and ownership refusals leave both the snapshot and the revision untouched.
#[test]
fn invalid_or_stale_command_does_not_change_history() {
    let (mut session, content) = fixture();
    let revision = session.revision();
    let initial = session.snapshot().unwrap();
    let layer = content["gerber_layers"][0].clone();
    assert!(matches!(set(&mut session, Uuid::nil(), &content), Err(CommandError::InvalidParams(msg)) if msg.contains("non-nil UUID")));
    let stale = session.apply(SET_OUTPUT_RECIPE, &json!({"recipe":ID,"expected_revision":revision - 1,"content":content}));
    assert!(matches!(stale, Err(CommandError::Unavailable(msg)) if msg.contains("project changed")));
    for (field, value) in [
        ("name", json!("a".repeat(MAX_RECIPE_NAME + 1))),
        ("board", json!(Uuid::from_u128(9))),
        ("file_stem", json!("../package")),
        ("file_stem", json!("con")),
        ("file_stem", json!("a".repeat(MAX_FILE_STEM + 1))),
        ("gerber_layers", json!([layer, layer])),
        ("gerber_layers", json!([Uuid::nil()])),
        ("origin", json!({"kind":"board_zero", "x_nm":1})),
        ("assembly_profile", json!({"profile":Uuid::from_u128(12),"revision":0})),
    ] {
        let mut invalid = content.clone();
        invalid[field] = value;
        let expected = if field == "board" { "target the registered board" } else { "must be valid output recipe content" };
        assert!(matches!(set(&mut session, ID, &invalid), Err(CommandError::InvalidParams(msg)) if msg.contains(expected)), "{field}: {invalid}");
    }
    let mut boundary = content;
    boundary["drill"] = json!(false);
    boundary["gerber_layers"] = json!([]);
    assert!(matches!(set(&mut session, ID, &boundary), Err(CommandError::InvalidParams(msg)) if msg.contains("must be valid output recipe content")));
    assert_eq!(session.revision(), revision);
    assert!(Arc::ptr_eq(&initial, &session.snapshot().unwrap()));

    boundary["name"] = json!("a".repeat(MAX_RECIPE_NAME));
    boundary["file_stem"] = json!("a".repeat(MAX_FILE_STEM));
    boundary["gerber_layers"] = json!((1..=MAX_RECIPE_LAYERS as u128).map(Uuid::from_u128).collect::<Vec<_>>());
    for index in (1..=MAX_RECIPES as u128).rev() {
        set(&mut session, Uuid::from_u128(index), &boundary).unwrap();
    }
    let before = session.snapshot().unwrap();
    assert_eq!(before.output_recipes()[0].id, Uuid::from_u128(1));
    assert!(matches!(set(&mut session, ID, &boundary), Err(CommandError::InvalidParams(msg)) if msg.contains("at most 32 output recipes")));
    boundary["gerber_layers"].as_array_mut().unwrap().push(json!(Uuid::from_u128(MAX_RECIPE_LAYERS as u128 + 1)));
    assert!(
        matches!(set(&mut session, Uuid::from_u128(1), &boundary), Err(CommandError::InvalidParams(msg)) if msg.contains("must be valid output recipe content"))
    );
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
}
