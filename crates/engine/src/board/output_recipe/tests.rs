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

/// These historical recipe fixtures have no instance extensions and use the original instance file.
fn legacy_recipe_documents(documents: &Documents, format: &str) -> Documents {
    let mut legacy = replace_entry(documents, |entry| entry["format"] = json!(format));
    let path = crate::persistence::project_instances_path(&legacy[ENTRY_POINT]).unwrap().unwrap();
    let mut instances: Value = serde_json::from_slice(&legacy[&path]).unwrap();
    instances["format"] = json!("pcbcraft-instances/1");
    legacy.insert(path, serde_json::to_vec(&instances).unwrap());
    legacy
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
    let format = |documents: &Documents| serde_json::from_slice::<Value>(&documents[ENTRY_POINT]).unwrap()["format"].clone();
    assert_eq!(format(&empty), crate::persistence::CURRENT_PROJECT_FORMAT);
    let mut before_recipes = replace_entry(&empty, |entry| entry["format"] = json!("pcbcraft-project/61"));
    let path = crate::persistence::project_instances_path(&before_recipes[ENTRY_POINT]).unwrap().unwrap();
    let mut previous_instances: Value = serde_json::from_slice(&before_recipes[&path]).unwrap();
    previous_instances["format"] = json!("pcbcraft-instances/9");
    before_recipes.insert(path.clone(), serde_json::to_vec(&previous_instances).unwrap());
    assert_eq!(encode_documents(decode_documents(&before_recipes).unwrap()).unwrap(), empty);

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
    assert_eq!(entry["format"], crate::persistence::CURRENT_PROJECT_FORMAT);
    assert_eq!(entry["output_recipes"], json!([{"id":ID,"content":content}]));
    assert_eq!(session.inspect()["project"]["output_recipes"], entry["output_recipes"]);
    let reopened = decode_documents(&saved).unwrap();
    assert_eq!(reopened.output_recipes(), project.output_recipes());
    assert_eq!(encode_documents(reopened).unwrap(), saved);
    let mut previous_recipe_format = replace_entry(&saved, |entry| entry["format"] = json!("pcbcraft-project/62"));
    previous_recipe_format.insert(path, serde_json::to_vec(&previous_instances).unwrap());
    assert_eq!(encode_documents(decode_documents(&previous_recipe_format).unwrap()).unwrap(), saved);
    let packed = decode_packed(&encode_packed(project.clone()).unwrap()).unwrap();
    assert_eq!(packed.project.output_recipes(), project.output_recipes());
    assert_eq!(packed.documents, saved);
    assert!(crate::persistence::encode(project).err().unwrap().contains("single-file encoding cannot represent output recipes"));

    session.apply("edit.undo", &Value::Null).unwrap();
    assert!(session.project().unwrap().output_recipes().is_empty());
    let undone = encode_documents(session.snapshot().unwrap()).unwrap();
    assert_eq!(format(&undone), crate::persistence::CURRENT_PROJECT_FORMAT);
    assert_eq!(undone, empty);
    assert!(decode_documents(&undone).unwrap().output_recipes().is_empty());
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
    let removed = encode_documents(session.snapshot().unwrap()).unwrap();
    assert_eq!(removed, empty);
    assert!(decode_documents(&removed).unwrap().output_recipes().is_empty());
}

/// Older headers refuse the field; the current header keeps it optional, nonempty and typed.
#[test]
fn recipe_reader_refuses_wrong_header_and_invalid_records() {
    let (mut session, content) = fixture();
    set(&mut session, ID, &content).unwrap();
    let saved = encode_documents(session.snapshot().unwrap()).unwrap();
    for header in
        ["pcbcraft-project/36", "pcbcraft-project/39", "pcbcraft-project/45", "pcbcraft-project/51", "pcbcraft-project/56", "pcbcraft-project/61"]
    {
        let invalid = replace_entry(&saved, |entry| entry["format"] = json!(header));
        assert!(decode_documents(&invalid).err().unwrap().contains("output_recipes requires project format"), "{header}");
    }
    let missing = replace_entry(&saved, |entry| {
        entry.as_object_mut().unwrap().remove("output_recipes");
    });
    assert!(decode_documents(&missing).unwrap().output_recipes().is_empty());
    let duplicate = json!([{"id":ID,"content":content},{"id":ID,"content":content}]);
    for (recipes, expected) in [
        (Value::Null, "invalid type: null"),
        (json!([]), "and a nonempty array"),
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
    let positional = positional(&content);
    for recipes in [json!([[ID, positional]]), json!([{"id":ID,"content":positional}])] {
        let invalid = replace_entry(&saved, |entry| entry["output_recipes"] = recipes);
        assert!(decode_documents(&invalid).err().unwrap().contains("invalid type: sequence, expected an object"));
    }
    let mut nested = content;
    nested["assembly_profile"] = json!([Uuid::from_u128(12), 2]);
    let invalid = replace_entry(&saved, |entry| entry["output_recipes"][0]["content"] = nested);
    assert!(decode_documents(&invalid).err().unwrap().contains("invalid type: sequence, expected an object"));
}

/// The same content in the field order of the struct, as a derived positional sequence.
fn positional(content: &Value) -> Value {
    let fields = ["name", "board", "units", "file_stem", "gerber_layers", "drill", "origin", "assembly_profile"];
    Value::Array(fields.iter().map(|field| content[field].clone()).collect())
}

/// Intrinsic and ownership refusals leave both the snapshot and the revision untouched.
#[test]
fn invalid_or_stale_command_does_not_change_history() {
    let (mut session, content) = fixture();
    let revision = session.revision();
    let initial = session.snapshot().unwrap();
    let history = (session.undo.len(), session.redo.len());
    let layer = content["gerber_layers"][0].clone();
    assert!(matches!(set(&mut session, Uuid::nil(), &content), Err(CommandError::InvalidParams(msg)) if msg.contains("non-nil UUID")));
    let stale = session.apply(SET_OUTPUT_RECIPE, &json!({"recipe":ID,"expected_revision":revision - 1,"content":content}));
    assert!(matches!(&stale, Err(CommandError::StaleRevision { current }) if *current == revision), "{stale:?}");
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
        ("assembly_profile", json!([Uuid::from_u128(12), 2])),
        ("origin", json!(["board_zero"])),
    ] {
        let mut invalid = content.clone();
        invalid[field] = value;
        let expected = if field == "board" { "target the registered board" } else { "must be valid output recipe content" };
        assert!(matches!(set(&mut session, ID, &invalid), Err(CommandError::InvalidParams(msg)) if msg.contains(expected)), "{field}: {invalid}");
    }
    assert!(
        matches!(set(&mut session, ID, &positional(&content)), Err(CommandError::InvalidParams(msg)) if msg.contains("must be valid output recipe content"))
    );
    let mut boundary = content;
    boundary["drill"] = json!(false);
    boundary["gerber_layers"] = json!([]);
    assert!(matches!(set(&mut session, ID, &boundary), Err(CommandError::InvalidParams(msg)) if msg.contains("must be valid output recipe content")));
    assert_eq!(session.revision(), revision);
    assert!(Arc::ptr_eq(&initial, &session.snapshot().unwrap()));
    assert_eq!((session.undo.len(), session.redo.len()), history);

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

/// Recipes and a designator lock are optional records of the current format and survive
/// save/reopen in both codecs. Removing either feature keeps the format and restores the bytes.
#[test]
fn recipe_and_designator_lock_share_the_current_format() {
    let (mut session, content) = fixture();
    session.apply(crate::PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    let symbol = session.project().unwrap().symbols()[0].id();
    let lock = |session: &mut Session, locked: bool| {
        session
            .apply(
                crate::SET_SYMBOL_FIELDS,
                &json!({"symbol":symbol,"reference":"R1","value":"R","designator_locked":locked,"expected_revision":session.revision()}),
            )
            .unwrap();
    };
    let format = |documents: &Documents| serde_json::from_slice::<Value>(&documents[ENTRY_POINT]).unwrap()["format"].clone();
    set(&mut session, ID, &content).unwrap();
    let recipes_only = encode_documents(session.snapshot().unwrap()).unwrap();
    assert_eq!(format(&recipes_only), crate::persistence::CURRENT_PROJECT_FORMAT);

    lock(&mut session, true);
    let project = session.snapshot().unwrap();
    let both = encode_documents(project.clone()).unwrap();
    assert_eq!(format(&both), crate::persistence::CURRENT_PROJECT_FORMAT);
    let reopened = decode_documents(&both).unwrap();
    assert_eq!(reopened.output_recipes(), project.output_recipes());
    assert!(reopened.components().any(|part| part.designator_locked()));
    assert_eq!(encode_documents(reopened).unwrap(), both);
    let packed = decode_packed(&encode_packed(project.clone()).unwrap()).unwrap();
    assert!(packed.project.components().any(|part| part.designator_locked()));
    assert_eq!(packed.project.output_recipes().len(), 1);
    assert_eq!(packed.documents, both);

    // The older lock headers refuse recipes.
    for header in ["pcbcraft-project/51", "pcbcraft-project/56"] {
        let downgraded = replace_entry(&both, |entry| entry["format"] = json!(header));
        assert!(decode_documents(&downgraded).err().unwrap().contains("output_recipes requires project format"), "{header}");
    }
    // Files saved earlier as project/47 (recipes) or /57 (recipes and a lock) still open, and the
    // next save writes the current format with the same records.
    for (header, current) in [("pcbcraft-project/47", &recipes_only), ("pcbcraft-project/57", &both)] {
        let old = legacy_recipe_documents(current, header);
        let reopened = decode_documents(&old).unwrap();
        assert_eq!(reopened.output_recipes(), project.output_recipes(), "{header}");
        assert_eq!(&encode_documents(reopened).unwrap(), current, "{header}");
    }
    // Those envelopes keep their own rules: recipes are required, /47 refuses locks and /57 requires one.
    let missing = replace_entry(&both, |entry| {
        entry["format"] = json!("pcbcraft-project/57");
        entry.as_object_mut().unwrap().remove("output_recipes");
    });
    assert!(decode_documents(&missing).err().unwrap().contains("formats 47, 57 and 60 require it"));
    let locked = replace_entry(&both, |entry| entry["format"] = json!("pcbcraft-project/47"));
    assert_eq!(decode_documents(&locked).unwrap_err(), "project.json: designator_locked not allowed in pcbcraft-project/47.");
    let unlocked = replace_entry(&recipes_only, |entry| entry["format"] = json!("pcbcraft-project/57"));
    assert!(decode_documents(&unlocked).err().unwrap().contains("at least one true"));

    set(&mut session, ID, &Value::Null).unwrap();
    let locks_only = encode_documents(session.snapshot().unwrap()).unwrap();
    assert_eq!(format(&locks_only), crate::persistence::CURRENT_PROJECT_FORMAT);
    session.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), both);
    lock(&mut session, false);
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), recipes_only);
}

/// A recipe that would push the native project entry point past its size limit is refused
/// before publication through both command entry points. Nothing changes: the snapshot,
/// revision, history and saved bytes stay exactly as they were.
#[test]
fn recipe_refuses_native_project_overflow_without_publishing() {
    use crate::{persistence::MAX_DOCUMENT_BYTES, schematic::fields::Field};

    let (mut session, content) = fixture();
    for index in 0..140 {
        let position = json!({"definition":"resistor","x_nm":(index % 16) * 10 * crate::schematic::GRID_NM,
            "y_nm":(index / 16) * 10 * crate::schematic::GRID_NM,"expected_revision":session.revision()});
        session.apply(crate::PLACE_SYMBOL, &position).unwrap();
    }
    let mut project = Project::clone(session.project().unwrap());
    for part in Arc::make_mut(&mut project.components).values_mut() {
        for index in 1..=64 {
            part.metadata.fields.insert(Uuid::from_u128(index).to_string(), Field { name: format!("Padding {index}"), value: String::new() });
        }
    }
    session.change(project).unwrap();
    let mut project = Project::clone(session.project().unwrap());
    let baseline = encode_documents(Arc::new(project.clone())).unwrap()[ENTRY_POINT].len();
    let mut remaining = MAX_DOCUMENT_BYTES - 1 - baseline;
    // ASCII padding grows the canonical entry point by precisely one byte per character.
    for part in Arc::make_mut(&mut project.components).values_mut() {
        for field in part.metadata.fields.values_mut() {
            let count = remaining.min(2048);
            field.value = "x".repeat(count);
            remaining -= count;
        }
    }
    assert_eq!(remaining, 0);
    let saved = encode_documents(Arc::new(project)).unwrap();
    assert_eq!(saved[ENTRY_POINT].len(), MAX_DOCUMENT_BYTES - 1);
    session.open_snapshot(decode_documents(&saved).unwrap());
    let before = session.snapshot().unwrap();
    let revision = session.revision();
    let history = (session.undo.len(), session.redo.len());
    let request = json!({"recipe":ID,"expected_revision":revision,"content":content});
    for execute in [true, false] {
        let error =
            if execute { session.execute(SET_OUTPUT_RECIPE, &request).unwrap_err() } else { session.apply(SET_OUTPUT_RECIPE, &request).unwrap_err() };
        assert!(matches!(&error, CommandError::InvalidParams(reason) if reason.contains("encoded document exceeds its size limit")), "{error:?}");
        assert!(Arc::ptr_eq(&session.snapshot().unwrap(), &before));
        assert_eq!(session.revision(), revision);
        assert_eq!((session.undo.len(), session.redo.len()), history);
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), saved);
    }
}

/// The published schema shape and runtime agree on reserved file stems and trimmed name rules,
/// while both accept an ordinary valid recipe.
#[test]
fn recipe_command_schema_matches_runtime_validation() {
    let described = crate::catalog().iter().find(|command| command.id == SET_OUTPUT_RECIPE).unwrap().to_json();
    let content_schema = &described["params"]["properties"]["content"];
    // Discovery adds each parameter's summary as its description; the rest is the runtime schema.
    assert_eq!(content_schema["description"], "Complete recipe configuration, or null to remove it");
    let mut published = content_schema.clone();
    published.as_object_mut().unwrap().remove("description");
    assert_eq!(published, schema());
    let properties = &content_schema["oneOf"][1]["properties"];
    let name_schema = &properties["name"];
    let stem_schema = &properties["file_stem"];

    assert_eq!(name_schema["type"], "string");
    assert_eq!(name_schema["minLength"], 1);
    assert_eq!(name_schema["maxLength"], MAX_RECIPE_NAME);
    assert_eq!(name_schema["pattern"], RECIPE_NAME_PATTERN);

    assert_eq!(stem_schema["type"], "string");
    assert_eq!(stem_schema["minLength"], 1);
    assert_eq!(stem_schema["maxLength"], MAX_FILE_STEM);
    assert_eq!(stem_schema["pattern"], FILE_STEM_PATTERN);
    let forbidden_stems: Vec<&str> = stem_schema["not"]["enum"].as_array().unwrap().iter().map(|item| item.as_str().unwrap()).collect();
    assert_eq!(forbidden_stems, RESERVED_PORTABLE_SEGMENTS);

    // `scripts/test_output_recipe_schema.py` evaluates the published `pattern`, `minLength`,
    // `maxLength` and `not.enum` rules in `docs/schemas/command-contracts.json` against these same
    // cases through `Draft202012Validator` and ECMA-262 `RegExp`; this test keeps the runtime side.
    let name_matches_schema = |value: &str| {
        let count = value.chars().count();
        let min = name_schema["minLength"].as_u64().unwrap() as usize;
        let max = name_schema["maxLength"].as_u64().unwrap() as usize;
        (min..=max).contains(&count)
            && !value.chars().any(char::is_control)
            && value.chars().next().is_some_and(|ch| !is_edge_whitespace(ch))
            && value.chars().next_back().is_some_and(|ch| !is_edge_whitespace(ch))
    };
    let stem_matches_schema = |value: &str| {
        let len = value.len();
        let min = stem_schema["minLength"].as_u64().unwrap() as usize;
        let max = stem_schema["maxLength"].as_u64().unwrap() as usize;
        (min..=max).contains(&len)
            && value.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !value.starts_with('-')
            && !value.ends_with('-')
            && !forbidden_stems.contains(&value)
    };

    let (mut session, content) = fixture();
    let revision = session.revision();
    let initial = session.snapshot().unwrap();

    for reserved in RESERVED_PORTABLE_SEGMENTS {
        assert!(!stem_matches_schema(reserved), "schema should reject reserved stem {reserved}");
        let mut invalid = content.clone();
        invalid["file_stem"] = json!(reserved);
        assert!(matches!(set(&mut session, ID, &invalid), Err(CommandError::InvalidParams(_))), "command should reject reserved stem {reserved}");
    }

    for stem in ["", "-proto", "proto-", "Proto", "prototype\n", "../package"] {
        assert!(!stem_matches_schema(stem), "schema should reject stem {stem:?}");
        let mut invalid = content.clone();
        invalid["file_stem"] = json!(stem);
        assert!(matches!(set(&mut session, ID, &invalid), Err(CommandError::InvalidParams(_))), "command should reject stem {stem:?}");
    }

    for name in [
        "",
        " ",
        " Prototype ",
        " Prototype",
        "Prototype ",
        "Proto\ntype",
        "Prototype\n",
        "Proto\ttype",
        "Proto\u{0000}type",
        "Proto\u{007f}type",
        "Proto\u{0085}type",
        "\u{00a0}Prototype",
        "\u{feff}Prototype",
        "Prototype\u{feff}",
    ] {
        assert!(!name_matches_schema(name), "schema should reject name {name:?}");
        let mut invalid = content.clone();
        invalid["name"] = json!(name);
        assert!(matches!(set(&mut session, ID, &invalid), Err(CommandError::InvalidParams(_))), "command should reject name {name:?}");
    }
    assert_eq!(session.revision(), revision);
    assert!(Arc::ptr_eq(&initial, &session.snapshot().unwrap()));

    for (name, stem) in [("Prototype", "prototype"), ("A", "a"), ("Rev 2 / Proto", "con-1"), ("Board 1", "lpt10")] {
        assert!(name_matches_schema(name), "schema should accept name {name:?}");
        assert!(stem_matches_schema(stem), "schema should accept stem {stem:?}");
        let mut valid = content.clone();
        valid["name"] = json!(name);
        valid["file_stem"] = json!(stem);
        set(&mut session, ID, &valid).unwrap();
    }
    assert_eq!(session.project().unwrap().output_recipes()[0].content.file_stem, "lpt10");
}

/// Every recipe ID field in the published command schema rejects the nil UUID and agrees with
/// runtime validation, while ordinary non-nil UUIDs are accepted in every field.
#[test]
fn recipe_command_schema_and_runtime_reject_nil_uuids() {
    let described = crate::catalog().iter().find(|command| command.id == SET_OUTPUT_RECIPE).unwrap().to_json();
    let mut recipe_schema = described["params"]["properties"]["recipe"].clone();
    recipe_schema.as_object_mut().unwrap().remove("description");
    let properties = &described["params"]["properties"]["content"]["oneOf"][1]["properties"];
    let expected = crate::non_nil_uuid_schema();
    assert_eq!(expected["type"], "string");
    assert_eq!(expected["format"], "uuid");
    assert_eq!(expected["not"]["const"], Uuid::nil().to_string());

    let mut layer = expected.clone();
    layer["pattern"] = json!(LAYER_ID_PATTERN);
    for (field, field_schema, expected) in [
        ("recipe", &recipe_schema, &expected),
        ("board", &properties["board"], &expected),
        ("gerber_layers.items", &properties["gerber_layers"]["items"], &layer),
        ("assembly_profile.profile", &properties["assembly_profile"]["properties"]["profile"], &expected),
    ] {
        assert_eq!(field_schema, expected, "{field} schema should require a non-nil UUID");
        let schema_accepts = |id: Uuid| {
            let text = id.to_string();
            Uuid::parse_str(&text).is_ok() && field_schema["not"]["const"].as_str() != Some(text.as_str())
        };
        assert!(!schema_accepts(Uuid::nil()), "{field} schema should reject nil UUID");
        assert!(schema_accepts(ID), "{field} schema should accept ordinary UUID");
    }

    let (mut session, content) = fixture();
    let revision = session.revision();
    let initial = session.snapshot().unwrap();

    // Nil recipe ID is rejected for both create/replace and null removal.
    assert!(matches!(set(&mut session, Uuid::nil(), &content), Err(CommandError::InvalidParams(_))));
    assert!(matches!(set(&mut session, Uuid::nil(), &Value::Null), Err(CommandError::InvalidParams(_))));

    // Nil board ID is rejected.
    let mut nil_board = content.clone();
    nil_board["board"] = json!(Uuid::nil());
    assert!(matches!(set(&mut session, ID, &nil_board), Err(CommandError::InvalidParams(_))));

    // Nil Gerber layer ID is rejected alone or alongside a valid layer ID.
    let mut nil_layer = content.clone();
    nil_layer["gerber_layers"] = json!([Uuid::nil()]);
    assert!(matches!(set(&mut session, ID, &nil_layer), Err(CommandError::InvalidParams(_))));
    let valid_layer = content["gerber_layers"][0].clone();
    let mut mixed_layers = content.clone();
    mixed_layers["gerber_layers"] = json!([valid_layer, Uuid::nil()]);
    assert!(matches!(set(&mut session, ID, &mixed_layers), Err(CommandError::InvalidParams(_))));

    // Nil assembly profile ID is rejected.
    let mut nil_profile = content.clone();
    nil_profile["assembly_profile"]["profile"] = json!(Uuid::nil());
    assert!(matches!(set(&mut session, ID, &nil_profile), Err(CommandError::InvalidParams(_))));

    assert_eq!(session.revision(), revision);
    assert!(Arc::ptr_eq(&initial, &session.snapshot().unwrap()));

    // Ordinary non-nil UUIDs in every field are accepted.
    set(&mut session, ID, &content).unwrap();
    assert_eq!(session.project().unwrap().output_recipes().len(), 1);
    assert_eq!(session.project().unwrap().output_recipes()[0].id, ID);
}

/// Differently cased spellings of one layer UUID would pass the published `uniqueItems` and then
/// decode to one layer, so the schema and the command both require the lowercase hyphenated form.
#[test]
fn recipe_layer_ids_have_one_spelling_in_schema_and_runtime() {
    let described = crate::catalog().iter().find(|command| command.id == SET_OUTPUT_RECIPE).unwrap().to_json();
    let layers = &described["params"]["properties"]["content"]["oneOf"][1]["properties"]["gerber_layers"];
    assert_eq!(layers["uniqueItems"], true);
    // `scripts/test_output_recipe_schema.py` evaluates this published pattern, with UUID format
    // checking, against the same spellings; this test keeps the runtime side.
    assert_eq!(layers["items"]["pattern"], LAYER_ID_PATTERN);

    let (mut session, content) = fixture();
    let layer = content["gerber_layers"][0].as_str().unwrap().to_owned();
    assert_eq!(layer, layer.to_lowercase());
    let upper = layer.to_uppercase();
    let revision = session.revision();
    let initial = session.snapshot().unwrap();
    for spelled in [json!([layer, upper]), json!([upper]), json!([layer.replace('-', "")]), json!([format!("{{{layer}}}")])] {
        let mut invalid = content.clone();
        invalid["gerber_layers"] = spelled.clone();
        // Parameter checking refuses the content before the edit, with the generic content message.
        assert!(
            matches!(set(&mut session, ID, &invalid), Err(CommandError::InvalidParams(message)) if message.contains("must be valid output recipe content")),
            "command should reject {spelled}"
        );
        assert!(parse(&invalid).is_err_and(|message| message.contains("lowercase hyphenated")), "parse should name the spelling for {spelled}");
    }
    assert_eq!(session.revision(), revision);
    assert!(Arc::ptr_eq(&initial, &session.snapshot().unwrap()));
    set(&mut session, ID, &content).unwrap();
    assert_eq!(session.project().unwrap().output_recipes()[0].content.gerber_layers[0].to_string(), layer);
}

/// The published integer fields accept integer-valued JSON numbers such as `2.0`, as JSON Schema
/// does, and the command stores them as integers; fractional values are refused on both sides.
/// `scripts/test_output_recipe_schema.py` checks the same values against the published contract.
#[test]
fn recipe_integer_fields_accept_integer_valued_numbers_like_schema() {
    let described = crate::catalog().iter().find(|command| command.id == SET_OUTPUT_RECIPE).unwrap().to_json();
    let properties = &described["params"]["properties"]["content"]["oneOf"][1]["properties"];
    let point = properties["origin"]["oneOf"].as_array().unwrap().iter().find(|item| item["properties"]["kind"]["const"] == "point").unwrap();
    let revision = &properties["assembly_profile"]["properties"]["revision"];
    // A JSON Schema integer is any number with a zero fractional part.
    let schema_integer = |value: &Value| value.as_f64().is_some_and(|number| number.fract() == 0.0);
    for (field, field_schema) in
        [("origin.x_nm", &point["properties"]["x_nm"]), ("origin.y_nm", &point["properties"]["y_nm"]), ("assembly_profile.revision", revision)]
    {
        assert_eq!(field_schema["type"], "integer", "{field}");
        assert!(schema_integer(&json!(2.0)), "{field} schema should accept 2.0");
        assert!(!schema_integer(&json!(2.5)), "{field} schema should reject 2.5");
    }

    let (mut session, content) = fixture();
    let revision = session.revision();
    let initial = session.snapshot().unwrap();
    /// Set one integer field of the recipe content to the given JSON number.
    type Edit = fn(&mut Value, Value);
    let edits: [(&str, Edit); 3] = [
        ("origin.x_nm", |content, number| content["origin"]["x_nm"] = number),
        ("origin.y_nm", |content, number| content["origin"]["y_nm"] = number),
        ("assembly_profile.revision", |content, number| content["assembly_profile"]["revision"] = number),
    ];
    for (field, edit) in edits {
        let mut fractional = content.clone();
        edit(&mut fractional, json!(2.5));
        let refused = set(&mut session, ID, &fractional);
        assert!(
            matches!(&refused, Err(CommandError::InvalidParams(msg)) if msg.contains("must be valid output recipe content")),
            "{field}: {refused:?}"
        );
        assert_eq!(session.revision(), revision);
        assert!(Arc::ptr_eq(&initial, &session.snapshot().unwrap()));
    }

    let mut integer_valued = content.clone();
    for (_, edit) in edits {
        edit(&mut integer_valued, json!(2.0));
    }
    set(&mut session, ID, &integer_valued).unwrap();
    let stored = &session.project().unwrap().output_recipes()[0].content;
    assert_eq!(stored.origin, Some(Anchor::Point { x_nm: 2, y_nm: 2 }));
    assert_eq!(stored.assembly_profile, Some(Adoption { profile: Uuid::from_u128(12), revision: 2 }));
    let accepted = session.revision();
    let mut exact = content;
    for (_, edit) in edits {
        edit(&mut exact, json!(2));
    }
    set(&mut session, ID, &exact).unwrap();
    assert_eq!(session.revision(), accepted, "the same integers written exactly are a no-op");
}

/// Verify set, replace and remove each succeed and round-trip through native persistence while a
/// board geometry command remains refused with the expected unsupported-feature error.
fn assert_recipe_edits_and_geometry_refusal(session: &mut Session, content: &Value, board: Uuid, layer: Uuid, expected_error: &str) {
    set(session, ID, content).unwrap();
    let created = session.snapshot().unwrap();
    let reopened = decode_documents(&encode_documents(created.clone()).unwrap()).unwrap();
    assert_eq!(reopened.output_recipes(), created.output_recipes());
    assert_eq!(reopened.output_recipes().len(), 1);

    let mut replaced = content.clone();
    replaced["units"] = json!("in");
    replaced["file_stem"] = json!("prototype-rev2");
    set(session, ID, &replaced).unwrap();
    let updated = session.snapshot().unwrap();
    let reopened = decode_documents(&encode_documents(updated.clone()).unwrap()).unwrap();
    assert_eq!(reopened.output_recipes(), updated.output_recipes());
    assert_eq!(reopened.output_recipes()[0].content.units, Units::In);
    assert_eq!(reopened.output_recipes()[0].content.file_stem, "prototype-rev2");

    set(session, ID, &Value::Null).unwrap();
    let removed = session.snapshot().unwrap();
    let reopened = decode_documents(&encode_documents(removed).unwrap()).unwrap();
    assert!(reopened.output_recipes().is_empty());

    let err = session
        .apply(
            crate::ADD_TRACKS,
            &json!({
                "board": board,
                "layer": layer,
                "width_nm": 250_000,
                "points": [{"x_nm": 0, "y_nm": 0}, {"x_nm": 1_000_000, "y_nm": 0}],
                "expected_revision": session.revision(),
            }),
        )
        .unwrap_err();
    assert_eq!(err, CommandError::Unavailable(expected_error.into()));
}

/// Output recipes store project-owned selections and remain editable on boards containing arcs,
/// while copper geometry commands stay refused.
#[test]
fn recipe_edits_succeed_on_board_with_arcs() {
    use crate::board::track::tests::{copper_layers, routed_fixture, view};

    let mut session = routed_fixture();
    let board = view(&session).target.board;
    let layer = copper_layers(&session)[0];
    let arc = json!({
        "id": Uuid::from_u128(0xa11c),
        "layer": layer,
        "width_nm": 250_000,
        "center": {"x_nm": 0, "y_nm": 0},
        "start": {"x_nm": 5_000_000, "y_nm": 0},
        "end": {"x_nm": 0, "y_nm": 5_000_000},
        "direction": "clockwise",
    });
    session.apply(crate::board::arc::SET_ARCS, &json!({"board": board, "expected_revision": session.revision(), "arcs": [arc]})).unwrap();
    assert!(!session.project().unwrap().board.arcs.is_empty());

    let content = json!({
        "name": "Prototype",
        "board": board,
        "units": "mm",
        "file_stem": "prototype",
        "gerber_layers": [layer],
        "drill": true,
        "origin": {"kind": "point", "x_nm": 100, "y_nm": -200},
        "assembly_profile": {"profile": Uuid::from_u128(12), "revision": 2},
    });
    assert_recipe_edits_and_geometry_refusal(&mut session, &content, board, layer, crate::board::arc::UNSUPPORTED);
}

/// Output recipes remain editable on boards containing teardrops, while copper geometry commands
/// stay refused.
#[test]
fn recipe_edits_succeed_on_board_with_teardrops() {
    use crate::{
        ADD_DRAWING, ADD_TRACKS, ADD_VIA,
        board::{
            setup::LayerPurpose,
            teardrop::{EDIT_TEARDROPS, Length, Parameters, Record},
            track::tests::{copper_layers, routed_fixture, view},
        },
        schematic::Point,
    };

    let mut session = routed_fixture();
    let layer = copper_layers(&session)[0];
    let center = Point { x_nm: 0, y_nm: 20_000_000 };
    let end = Point { x_nm: 600_000, y_nm: 20_000_000 };
    session
        .apply(ADD_VIA, &json!({"at": center, "via_diameter_nm": 500_000, "via_drill_nm": 300_000, "expected_revision": session.revision()}))
        .unwrap();
    session
        .apply(ADD_TRACKS, &json!({"layer": layer, "width_nm": 100_000, "points": [center, end], "expected_revision": session.revision()}))
        .unwrap();
    let target = view(&session).target;
    let outline = view(&session).layers().into_iter().find(|l| l.purpose == LayerPurpose::Outline).unwrap().id;
    let corners = [(-5_000_000, -5_000_000), (25_000_000, -5_000_000), (25_000_000, 45_000_000), (-5_000_000, 45_000_000)]
        .map(|(x_nm, y_nm)| Point { x_nm, y_nm });
    session
        .apply(
            ADD_DRAWING,
            &json!({"board": target.board, "layer": outline, "points": corners, "closed": true, "expected_revision": session.revision()}),
        )
        .unwrap();
    let board_view = view(&session);
    let via = board_view.vias().iter().find(|via| via.at == center).unwrap().id;
    let track = board_view.tracks().iter().find(|track| track.start == center).unwrap().id;
    let record = Record {
        id: Uuid::from_u128(0x7ea1),
        via,
        track,
        parameters: Parameters { length: Length::Percent(20), max_length_nm: None, width_pct: 80, max_width_nm: None },
    };
    session
        .execute(
            EDIT_TEARDROPS,
            &json!({"board": target.board, "expected_revision": session.revision(), "edits": {"add": [record], "remove": []}, "dry_run": false}),
        )
        .unwrap();
    assert!(!session.project().unwrap().board.teardrops.is_empty());

    let content = json!({
        "name": "Prototype",
        "board": target.board,
        "units": "mm",
        "file_stem": "prototype",
        "gerber_layers": [layer],
        "drill": true,
        "origin": {"kind": "point", "x_nm": 100, "y_nm": -200},
        "assembly_profile": {"profile": Uuid::from_u128(12), "revision": 2},
    });
    assert_recipe_edits_and_geometry_refusal(&mut session, &content, target.board, layer, crate::board::teardrop::UNSUPPORTED);
}
