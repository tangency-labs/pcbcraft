use super::*;
use crate::{
    ADD_DRAWING, ADD_TRACKS, INITIALIZE_BOARD_SETUP, MOVE_SYMBOL, PLACE_RESISTOR, SET_ASSEMBLY_PROFILE, authoring, catalog,
    error_codes::ErrorCode,
    library::parts::{Package, Reference},
    part_authoring::Record,
    persistence::encode_documents,
};
use serde_json::{Value, json};
use std::sync::Arc;

/// Create the same board and symbol through the ordinary authoring commands used by clients.
fn fixture() -> Session {
    let mut editor = Session::default();
    editor.apply("project.new", &json!({"name":"Review fixture","create_board":true})).unwrap();
    editor.apply(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":editor.revision()})).unwrap();
    editor.apply(PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":editor.revision()})).unwrap();
    editor
}

/// A publisher assertion for the demo resistor symbol, built through the native assertion codec.
fn resistor_assertion() -> sources::lifecycle::Assertion {
    use sources::lifecycle::{Assertion, FORMAT, Policy, Provenance};
    Assertion {
        format: FORMAT.into(),
        address: Reference { library: "parts-demo".into(), id: "resistor-assessment".into(), revision: 1 },
        target: sources::Key { library: "parts-demo".into(), kind: sources::Kind::Symbol, id: "resistor".into(), revision: 1 },
        scheme: Policy::standard().scheme,
        state: "released".into(),
        reason: "Drawing review completed".into(),
        provenance: Provenance { publisher: "Native Parts Demo".into() },
    }
}

/// Load the same validated native source into both hosts before testing the part and lifecycle readers.
fn add_native_part(reader: &mut Session, control: &mut Session) {
    let docs = sources::Documents::from([
        ("lifecycle-assertions/resistor-assessment.r1.json".into(), resistor_assertion().encode().unwrap()),
        ("library.json".into(), include_str!("../../../../examples/native-parts/library.json").into()),
        ("symbols/resistor.json".into(), include_str!("../../../../examples/native-parts/symbols/resistor.json").into()),
        ("footprints/resistor-0603.json".into(), include_str!("../../../../examples/native-parts/footprints/resistor-0603.json").into()),
        ("categories/resistor.json".into(), include_str!("../../../../examples/native-parts/categories/resistor.json").into()),
        ("parts/resistor.json".into(), include_str!("../../../../examples/native-parts/parts/resistor.json").into()),
    ]);
    let source = Arc::new(sources::NativeSource::load(&docs).unwrap());
    reader.install_library_source("scope-test", Arc::clone(&source)).unwrap();
    control.install_library_source("scope-test", source).unwrap();
}

/// Prepare original symbol and part drafts for the two dry-run publication commands.
fn publication_requests() -> (Value, Value) {
    let item = authoring::Item::new(authoring::Kind::Symbol).unwrap();
    let manifest = authoring::publishing::new_library("scope-workshop", "Scope workshop", &item.license).unwrap();
    let reference = |id: &str, revision| Reference { library: "basic-components".into(), id: id.into(), revision };
    let mut record = Record::new_part(reference("connector", 4)).unwrap();
    if let Record::Part(part) = &mut record {
        part.packages.insert(
            "header".into(),
            Package {
                name: "Header".into(),
                footprint: reference("header-1x02-254", 3),
                mapping: [("1".into(), vec!["1".into()]), ("2".into(), vec!["2".into()])].into(),
                unconnected_pads: Vec::new(),
            },
        );
    }
    (json!({"item":item,"documents":{"library.json":manifest}}), json!({"record":record,"documents":{"library.json":manifest}}))
}

/// Preview and cancel through both dispatch paths, using the actual token issued to each
/// session. Tokens intentionally differ, but the rest of the report must agree.
fn preview_and_cancel(reader: &mut Session, control: &mut Session, pair: (&str, &str, &str), params: &Value) {
    let (preview, cancel, key) = pair;
    let mut reports = [Value::Null, Value::Null];
    for (session, report) in [reader, control].into_iter().zip(&mut reports) {
        let before = session.inspect();
        let revision = session.revision();
        *report = session.execute(preview, params).unwrap_or_else(|error| panic!("{preview}: {error}"));
        let token = report[key].as_object_mut().unwrap().remove("plan_token").unwrap();
        assert!(token.is_string());
        if preview == PREVIEW_UPDATE {
            // The input digest is stable, but each preview captures that input under a fresh UUID.
            let capture = report[key]["source"].as_object_mut().unwrap().remove("capture_id").unwrap();
            assert!(capture.as_str().and_then(|text| uuid::Uuid::parse_str(text).ok()).is_some_and(|id| !id.is_nil()));
        }
        session.execute(cancel, &json!({"plan_token":token})).unwrap_or_else(|error| panic!("{cancel}: {error}"));
        assert_eq!(session.inspect(), before);
        assert_eq!(session.apply(preview, params).unwrap(), revision);
        let token = if preview == PREVIEW_UPDATE {
            session.board_update_report().unwrap()["plan_token"].clone()
        } else {
            json!(session.board_setup_preview().unwrap().plan_token)
        };
        assert_eq!(session.apply(cancel, &json!({"plan_token":token})).unwrap(), revision);
        assert_eq!(session.inspect(), before);
    }
    assert_eq!(reports[0], reports[1], "{preview}");
}

/// Exercise every catalog entry against an unrestricted control with applicable inputs.
/// Every authoring command must fail before stale or malformed input affects the result.
#[test]
fn catalog_reads_succeed_and_authoring_commands_are_refused() {
    let mut editor = fixture();
    let profile = uuid::Uuid::from_u128(1);
    editor.apply(SET_ASSEMBLY_PROFILE, &json!({"profile":profile,"content":{"name":"Review","revision":1,"provenance":{"kind":"authored"}},"expected_revision":editor.revision()})).unwrap();
    let edge = editor
        .project()
        .unwrap()
        .boards()
        .next()
        .unwrap()
        .layers()
        .into_iter()
        .find(|layer| layer.purpose == board::setup::LayerPurpose::Outline)
        .unwrap()
        .id;
    editor.apply(ADD_DRAWING, &json!({"layer":edge,"points":[{"x_nm":-20_000_000,"y_nm":-20_000_000},{"x_nm":20_000_000,"y_nm":-20_000_000},{"x_nm":20_000_000,"y_nm":20_000_000},{"x_nm":-20_000_000,"y_nm":20_000_000}],"closed":true,"expected_revision":editor.revision()})).unwrap();
    let project = editor.snapshot().unwrap();
    let board = project.board_target().unwrap().board;
    let drawing = project.boards().next().unwrap().drawings()[0].id;
    let (item_publication, part_publication) = publication_requests();
    let remap_item = authoring::Item::new(authoring::Kind::Symbol).unwrap();
    let mut reader = Session::with_scope(SessionScope::ReadOnly);
    reader.open_snapshot(project.clone());
    // Use a fresh unrestricted control so openings and runtime revisions match.
    let mut control = Session::default();
    control.open_snapshot(project.clone());
    add_native_part(&mut reader, &mut control);
    let before = reader.inspect();
    let documents = encode_documents(project.clone()).unwrap();
    let revision = reader.revision();
    let symbol = &project.symbols()[0];
    let setup = project.board_setup().unwrap();
    let layer = setup.layers().iter().find(|layer| layer.purpose == board::setup::LayerPurpose::Copper).unwrap().id;

    for command in catalog() {
        let id = command.id;
        // These are independent expectations, not derived from the production access policy.
        let params = match id {
            netlist::EXPORT_BOARD_NETLIST => json!({"board":board}),
            no_connect::CHECK
            | sheets::page::LIST_TEMPLATES
            | netlist::EXPORT_NETLIST
            | CHECK_DESIGN
            | net_classes::INSPECT
            | nets::INSPECT_CLASSES
            | part_groups::INSPECT
            | part_groups::INSPECT_GROUPS
            | sources::LIST
            | sources::AVAILABLE_REVISIONS
            | sources::lifecycle::INSPECT
            | EXPORT_FABRICATION => json!({}),
            clipboard_text::LABELS => json!({"text":"N1","style":"local","spacing_nm":2_540_000,"expected_revision":revision}),
            clipboard_text::OUTLINE => json!({"text":"0,0\n10,0\n10,10","units":"mm","closed":true,"expected_revision":revision}),
            clipboard::COPY => json!({"items":{"symbols":[symbol.id()],"wires":[]},"expected_revision":revision}),
            board::clipboard::COPY => json!({"items":{"drawings":[drawing]},"expected_revision":revision}),
            parameters::LIST => json!({"scope":"project"}),
            parameters::RESOLVE => json!({"text":"@project.Revision"}),
            PREVIEW_UPDATE => {
                preview_and_cancel(
                    &mut reader,
                    &mut control,
                    (PREVIEW_UPDATE, CANCEL_UPDATE, "board_update_preview"),
                    &json!({"expected_revision":revision}),
                );
                continue;
            }
            PREVIEW_BOARD_SETUP => {
                preview_and_cancel(
                    &mut reader,
                    &mut control,
                    (PREVIEW_BOARD_SETUP, CANCEL_BOARD_SETUP_PREVIEW, "board_setup_preview"),
                    &json!({"setup":setup,"expected_revision":revision}),
                );
                continue;
            }
            CANCEL_UPDATE | CANCEL_BOARD_SETUP_PREVIEW => continue, // Tested with their corresponding preview.
            PREVIEW_ASSEMBLY_PROFILE => json!({"board":board,"profile":profile,"revision":1,"expected_revision":revision}),
            board::class_rules::ROUTE_RULES => json!({"at":{"x_nm":0,"y_nm":0},"layer":layer}),
            board::route::PLAN_ROUTE => json!({"from":{"x_nm":0,"y_nm":0},"to":{"x_nm":1_000_000,"y_nm":0},"layer":layer}),
            net_classes::RESOLVE => json!({"classes":[]}),
            sources::SEARCH => json!({"query":"resistor"}),
            sources::RESOLVE_PART => json!({"library":"parts-demo","definition":"resistor","definition_revision":1}),
            sources::PARSE_QUANTITY => json!({"quantity":"resistance","value":"100 mΩ"}),
            sources::PREVIEW_PIN_REMAP => json!({"item":remap_item,"pins":[{"unit":"main","object":"1","number":"7"}]}),
            sources::PREVIEW_FOOTPRINT_REVISION => {
                // A read: this fixture has no library placement, so both hosts must give the same
                // lookup failure, never a scope refusal. The successful read-only preview is in
                // library::sources::updates::preview_tests.
                let params = json!({"board":board,"part_instance":"absent"});
                let expected = control.execute(id, &params).unwrap_err();
                assert_eq!(expected.code(), ErrorCode::NotFound, "{id}: {expected}");
                for error in [reader.execute(id, &params).unwrap_err(), reader.apply(id, &params).unwrap_err()] {
                    assert_eq!((error.code(), error.to_string()), (expected.code(), expected.to_string()), "{id}");
                }
                continue;
            }
            sources::PREPARE_PART_PUBLICATION => part_publication.clone(),
            sources::PREPARE_PUBLICATION => item_publication.clone(),
            sources::lifecycle::LOOKUP => json!({"library":"parts-demo","assertion":"resistor-assessment","assertion_revision":1}),
            _ => {
                let invalid = json!({"expected_revision":revision + 1,"extra":true});
                let executed = reader.execute(id, &invalid).unwrap_err();
                let applied = reader.apply(id, &invalid).unwrap_err();
                for error in [executed, applied] {
                    assert_eq!(error.code(), ErrorCode::OutOfScope, "{id}: {error}");
                    assert_eq!(error.details(), json!({"command":id,"scope":"read_only"}));
                }
                continue;
            }
        };
        let expected = control.execute(id, &params).unwrap_or_else(|error| panic!("control {id}: {error}"));
        let actual = reader.execute(id, &params).unwrap_or_else(|error| panic!("reader {id}: {error}"));
        if matches!(id, clipboard_text::LABELS | clipboard_text::OUTLINE) {
            // These readers create a disposable project to build a fragment with its own IDs.
            assert!(actual["clipboard"].as_str().is_some_and(|text| !text.is_empty()), "{id}");
            assert_eq!(actual["revision"], expected["revision"], "{id}");
        } else {
            assert_eq!(actual, expected, "{id}");
        }
        if id == sources::PREVIEW_PIN_REMAP {
            assert_eq!(actual["changes"][0]["after"], "7");
            assert_eq!(actual["saved"], false);
            assert_eq!(actual["placed_instances_changed"], false);
        }
        // Assembly profile preview returns its data only via execute, including in MCP.
        if id != PREVIEW_ASSEMBLY_PROFILE {
            assert_eq!(reader.apply(id, &params).unwrap_or_else(|error| panic!("apply {id}: {error}")), revision, "{id}");
        }
    }

    assert_eq!(reader.inspect(), before);
    assert!(Arc::ptr_eq(&project, &reader.snapshot().unwrap()));
    assert_eq!(encode_documents(reader.snapshot().unwrap()).unwrap(), documents);
    assert_eq!(reader.execute("not.a.command", &Value::Null).unwrap_err().code(), ErrorCode::UnknownCommand);
}

/// A valid move and its undo still work in the default scope. Reopening a snapshot does not
/// widen the restricted scope or make an otherwise valid move run there.
#[test]
fn reopening_never_widens_scope_but_default_sessions_can_edit() {
    let mut editor = fixture();
    let original = editor.snapshot().unwrap();
    let symbol = original.symbols()[0].id();
    let mut reader = Session::with_scope(SessionScope::ReadOnly);
    for _ in 0..2 {
        reader.open_snapshot(original.clone());
        let params = json!({"symbol":symbol,"x_nm":1_000_000,"y_nm":0,"expected_revision":reader.revision()});
        assert_eq!(reader.apply(MOVE_SYMBOL, &params).unwrap_err().code(), ErrorCode::OutOfScope);
        assert_eq!(reader.scope(), SessionScope::ReadOnly);
        assert!(Arc::ptr_eq(&original, &reader.snapshot().unwrap()));
    }
    let params = json!({"symbol":symbol,"x_nm":1_000_000,"y_nm":0,"expected_revision":editor.revision()});
    let revision = editor.revision();
    assert!(editor.apply(MOVE_SYMBOL, &params).unwrap() > revision);
    assert_ne!(encode_documents(editor.snapshot().unwrap()).unwrap(), encode_documents(original.clone()).unwrap());
    editor.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(encode_documents(editor.snapshot().unwrap()).unwrap(), encode_documents(original).unwrap());
}

/// Prepared copper cannot get around the command dispatcher, either by preparing in the
/// reader through direct or reusable interactive contexts, or by offering it a candidate
/// that a writable session prepared.
#[test]
fn prepared_copper_uses_the_same_scope() {
    let editor = fixture();
    let project = editor.snapshot().unwrap();
    let setup = project.board_setup().unwrap();
    let layer = setup.layers().iter().find(|layer| layer.purpose == board::setup::LayerPurpose::Copper).unwrap().id;
    let params =
        |revision| json!({"layer":layer,"width_nm":250_000,"points":[{"x_nm":0,"y_nm":0},{"x_nm":1_000_000,"y_nm":0}],"expected_revision":revision});
    let mut reader = Session::with_scope(SessionScope::ReadOnly);
    reader.open_snapshot(project.clone());
    let refused = reader.prepare_copper_edit(ADD_TRACKS, &params(reader.revision())).err().unwrap();
    assert_eq!(refused.code(), ErrorCode::OutOfScope);
    let context = reader.copper_preparation(project.board_target().unwrap()).unwrap();
    let refused = reader.prepare_copper_edit_with(&context, ADD_TRACKS, &params(reader.revision())).err().unwrap();
    assert_eq!(refused.code(), ErrorCode::OutOfScope);
    let refused = reader.prepare_interactive_route_with(&context, &params(reader.revision())).err().unwrap();
    assert_eq!(refused.code(), ErrorCode::OutOfScope);
    let mut job = editor.prepare_copper_edit(ADD_TRACKS, &params(editor.revision())).unwrap();
    assert!((0..100).any(|_| job.step(128)), "small fixture must finish");
    assert_eq!(reader.commit_copper_edit(job.take_result().unwrap().unwrap()).unwrap_err().code(), ErrorCode::OutOfScope);
    assert!(Arc::ptr_eq(&project, &reader.snapshot().unwrap()));
}
