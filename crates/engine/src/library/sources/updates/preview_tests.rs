use super::*;
use crate::{
    ADD_TRACKS, ADD_VIA, ASSIGN_FOOTPRINT, INITIALIZE_BOARD_SETUP, PLACE_ASSIGNED_FOOTPRINT, PLACE_SYMBOL, SET_FOOTPRINT_TRANSFORM, Session,
    SessionScope,
    authoring::{
        Item as Draft, Kind as DraftKind, Primitive,
        publishing::{new_library, prepare},
    },
    board::{
        definition::{CopperBinding, PadShape, RectPad},
        setup::{BoardSetup, LayerPurpose, Side},
        slot::PlatedSlot,
        track::Track,
    },
    library::sources::{Documents, LOAD, NativeSource, PREVIEW_FOOTPRINT_REVISION, REMOVE, part_digest, symbol_digest},
    part_instances::PLACE_PART,
    persistence::encode_documents,
};
use std::sync::Arc;

/// Author two original library footprints and an original symbol; build the board with the shared commands.
fn fixture(edit_pad: impl FnOnce(&mut RectPad), owned_part: bool) -> (Session, Documents, Value) {
    fixture_with_original(|_| {}, edit_pad, owned_part)
}

/// Author both versions when the change also needs a non-default starting shape.
fn fixture_with_original(original: impl FnOnce(&mut RectPad), edit_pad: impl FnOnce(&mut RectPad), owned_part: bool) -> (Session, Documents, Value) {
    let manifest = new_library("preview-test", "Preview test", "MIT OR Apache-2.0").unwrap();
    let symbol = prepare(Arc::new(Draft::new(DraftKind::Symbol).unwrap()), &manifest, 0, None, false).unwrap();
    let mut first_draft = Draft::new(DraftKind::Footprint).unwrap();
    let Primitive::Pad { pad } = first_draft.objects.get_mut(&2).unwrap() else { panic!("expected authored pad 2") };
    original(pad);
    first_draft.refresh_footprint_format();
    let first = prepare(Arc::new(first_draft), &manifest, 0, None, false).unwrap();
    let mut draft = first.item().as_ref().clone();
    let Primitive::Pad { pad } = draft.objects.get_mut(&2).unwrap() else { panic!("expected authored pad 2") };
    edit_pad(pad);
    draft.refresh_footprint_format();
    let second = prepare(Arc::new(draft), &manifest, 1, Some(1), false).unwrap();
    let mut documents = symbol.documents().clone();
    documents.extend(first.documents().clone());
    documents.extend(second.documents().clone());
    documents.insert("library.json".into(), manifest);
    let symbol = symbol.item().symbol().unwrap();
    let footprint = first.item().footprint().unwrap();
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    edit(&mut session, INITIALIZE_BOARD_SETUP, json!({"copper_layers":2}));
    if owned_part {
        // The newer owning part pins the newer geometry through the same package ID.
        let mut part = PartDefinition::read(
            &json!({"format":"pcbcraft-part/1","library":"preview-test","id":"test-part","revision":1,
            "name":"Test part","license":"MIT OR Apache-2.0","source":{"kind":"original","reference":"Preview test","locator":"synthetic"},
            "symbol":{"library":symbol.library,"id":symbol.id,"revision":1},"packages":{"same-package":{
                "name":"Original package","footprint":{"library":footprint.library,"id":footprint.id,"revision":1},"mapping":{"1":["1"],"2":["2"]}}}})
            .to_string(),
        )
        .unwrap();
        documents.insert("parts/part-r1.json".into(), part.encode().unwrap());
        part.revision = 2;
        part.packages.get_mut("same-package").unwrap().footprint.revision = 2;
        documents.insert("parts/part-r2.json".into(), part.encode().unwrap());
        load(&mut session, &documents);
        let original = Reference { library: part.library, id: part.id, revision: 1 };
        let digest = part_digest(session.libraries().resolve_part(&original).unwrap());
        edit(
            &mut session,
            PLACE_PART,
            json!({"library":original.library,"definition":original.id,"definition_revision":1,
            "definition_digest":digest,"package":"same-package","x_nm":0,"y_nm":0}),
        );
    } else {
        load(&mut session, &documents);
        let symbol = session.libraries().symbol(&symbol.library, &symbol.id, symbol.revision()).unwrap();
        edit(
            &mut session,
            PLACE_SYMBOL,
            json!({"library":symbol.library,"definition":symbol.id,"definition_revision":1,
            "definition_digest":symbol_digest(&symbol).unwrap(),"x_nm":0,"y_nm":0}),
        );
        let part = session.project().unwrap().components().next().unwrap().id();
        edit(
            &mut session,
            ASSIGN_FOOTPRINT,
            json!({"component":part,"library":footprint.library,"definition":footprint.id,
            "definition_revision":1,"mapping":{"1":"1","2":"2"}}),
        );
    }
    let part = session.project().unwrap().components().next().unwrap().id();
    let board = session.project().unwrap().board_target().unwrap().board;
    edit(&mut session, PLACE_ASSIGNED_FOOTPRINT, json!({"component":part,"x_nm":0,"y_nm":0}));
    (session, documents, json!({"board":board,"part_instance":part}))
}

/// Keep fixture edits on the same revision-checked path as normal callers.
fn edit(session: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(session.revision());
    session.apply(command, &params).unwrap();
}

/// Load synthetic definition bytes through the existing library catalog command.
fn load(session: &mut Session, documents: &Documents) {
    session.execute(LOAD, &json!({"source":"test","documents":documents})).unwrap();
}

/// Read a layer ID by physical side rather than stack order or display name.
fn layer(session: &Session, side: Side) -> uuid::Uuid {
    let board = session.project().unwrap().boards().next().unwrap();
    board.setup().unwrap().layers().iter().find(|layer| layer.purpose == LayerPurpose::Copper && layer.side == Some(side)).unwrap().id
}

/// Pad 2 moves 0.2 mm from a tangent front track end; a geometrically identical back
/// track is not attached. Preview leaves history, project bytes and loaded sources intact.
#[test]
fn footprint_preview_moves_pad_two_and_reports_one_loose_end_at_point_two_mm() {
    let (mut session, _, params) = fixture(|pad| pad.center.x_nm += 200_000, false);
    // This via overlaps both revisions, so it is not a loose end.
    edit(&mut session, ADD_VIA, json!({"at":{"x_nm":650_000,"y_nm":0},"via_diameter_nm":120_000,"via_drill_nm":60_000}));
    for side in [Side::Front, Side::Back] {
        let layer = layer(&session, side);
        edit(
            &mut session,
            ADD_TRACKS,
            json!({"layer":layer,"width_nm":100_000,
            "points":[{"x_nm":100_000,"y_nm":0},{"x_nm":200_000,"y_nm":0}]}),
        );
    }
    session.apply("project.rename", &json!({"name":"Redo remains"})).unwrap();
    session.apply("edit.undo", &Value::Null).unwrap();
    let before = session.snapshot().unwrap();
    let bytes = encode_documents(before.clone()).unwrap();
    let inspect = session.inspect();
    let catalog = session.libraries().generation();
    let track = before.board.tracks.iter().find(|track| track.layer == layer(&session, Side::Front)).unwrap();
    let report = session.execute(PREVIEW_FOOTPRINT_REVISION, &params).unwrap();
    assert_eq!(report["revision"], session.revision());
    assert_eq!(report["catalog_generation"], catalog);
    assert_eq!(report["pinned_root"]["kind"], "footprint");
    assert_eq!(report["pinned_footprint"]["revision"], 1);
    assert_eq!(report["candidate_footprint"]["revision"], 2);
    assert_eq!(report["delta"]["pad_numbers_changed"], false);
    let pads = report["delta"]["pads"].as_array().unwrap();
    assert_eq!(pads.len(), 1);
    let row = &pads[0];
    assert_eq!(row["number"], "2");
    assert_eq!(row["changes"], json!(["moved", "mask", "paste"]));
    assert_eq!(row["before"]["board_center"]["x_nm"], 500_000);
    assert_eq!(row["after"]["board_center"]["x_nm"], 700_000);
    assert_eq!(row["center_shift_nm"], 200_000.0);
    assert_eq!(row["loose_ends"], json!([{"id":track.id,"kind":"track_end","layer":track.layer,"at":track.end,"gap_nm":200_000.0}]));
    assert_eq!(session.inspect(), inspect);
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), bytes);
    assert_eq!(session.libraries().generation(), catalog);
    session.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(session.inspect()["project"]["name"], "Redo remains");
    session.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), bytes);
}

/// A read-only host can inspect an installed revision without changing the document, history,
/// or the source catalog. Client commands still cannot edit the project or sources.
#[test]
fn footprint_preview_succeeds_in_read_only_session_without_editing_history() {
    let (setup, documents, params) = fixture(|pad| pad.center.x_nm += 200_000, false);
    let snapshot = setup.snapshot().unwrap();
    let mut read_only = Session::with_scope(SessionScope::ReadOnly);
    read_only.open_snapshot(snapshot.clone());
    read_only.install_library_source("test", Arc::new(NativeSource::load(&documents).unwrap())).unwrap();
    let before = read_only.inspect();
    assert_eq!(before["can_undo"], false);
    assert_eq!(before["can_redo"], false);
    let bytes = encode_documents(snapshot.clone()).unwrap();
    let generation = read_only.libraries().generation();
    let result = read_only.execute(PREVIEW_FOOTPRINT_REVISION, &params).unwrap();
    assert_eq!(result["delta"]["pads"][0]["center_shift_nm"], 200_000.0);
    assert_eq!(result["candidate_footprint"]["revision"], 2);
    for (command, args) in [("project.rename", json!({"name":"No edit"})), (REMOVE, json!({"source":"test"}))] {
        assert!(matches!(read_only.execute(command, &args), Err(crate::CommandError::OutOfScope { .. })));
    }
    assert_eq!(read_only.inspect(), before);
    let after = read_only.snapshot().unwrap();
    assert!(Arc::ptr_eq(&snapshot, &after));
    assert_eq!(encode_documents(after).unwrap(), bytes);
    assert_eq!(read_only.libraries().generation(), generation);
}

/// Half-turning an odd-width rectangle or quarter-turning an odd square moves one
/// projected edge by 1 nm. A previously tangent cap loses contact with that edge.
#[test]
fn footprint_preview_reports_odd_pad_rotation_and_tangent_cap_loss() {
    for (rotation, height, start, end) in [(180, 400_000, (1_000_000, 0), (800_001, 0)), (90, 500_001, (500_000, 500_000), (500_000, 300_001))] {
        let (mut session, _, params) = fixture_with_original(
            |pad| {
                pad.width_nm = 500_001;
                pad.height_nm = height;
                pad.shape = PadShape::Rectangle;
                pad.corner_radius_nm = 0;
                pad.rotation = 0;
            },
            |pad| pad.rotation = rotation,
            false,
        );
        let front = layer(&session, Side::Front);
        edit(
            &mut session,
            ADD_TRACKS,
            json!({"layer":front,"width_nm":100_000,
                "points":[{"x_nm":start.0,"y_nm":start.1},{"x_nm":end.0,"y_nm":end.1}]}),
        );
        let snapshot = session.snapshot().unwrap();
        let track = &snapshot.board.tracks[0];
        let report = session.execute(PREVIEW_FOOTPRINT_REVISION, &params).unwrap();
        let row = &report["delta"]["pads"][0];
        assert_eq!(row["number"], "2");
        assert_eq!(row["changes"], json!(["rotated", "mask", "paste"]), "rotation {rotation}");
        assert_eq!(row["loose_ends"], json!([{"id":track.id,"kind":"track_end","layer":front,"at":track.end,"gap_nm":1.0}]));
    }
}

/// Reordered corners on a truly symmetric integer projection are not an opening change.
#[test]
fn footprint_preview_keeps_even_square_openings_after_quarter_turn() {
    let (mut session, _, params) = fixture_with_original(
        |pad| {
            pad.width_nm = 500_000;
            pad.height_nm = 500_000;
            pad.shape = PadShape::Rectangle;
            pad.corner_radius_nm = 0;
            pad.rotation = 0;
        },
        |pad| pad.rotation = 90,
        false,
    );
    let report = session.execute(PREVIEW_FOOTPRINT_REVISION, &params).unwrap();
    let row = &report["delta"]["pads"][0];
    assert_eq!(row["number"], "2");
    assert_eq!(row["changes"], json!(["rotated"]));
    assert_eq!(row["loose_ends"], json!([]));
}

/// The authored number changes instead of receiving an inferred correspondence, including
/// at a rotated bottom placement. A via is reported once, with no gap to a missing same-number pad.
#[test]
fn footprint_preview_does_not_map_renumbered_pad_or_repeat_through_via() {
    let (mut session, _, params) = fixture(|pad| pad.number = "02".into(), false);
    let id = session.project().unwrap().boards().next().unwrap().footprint_ids().next().unwrap();
    edit(&mut session, SET_FOOTPRINT_TRANSFORM, json!({"footprint":id,"side":"back","rotation":90,"x_nm":2_000_000,"y_nm":3_000_000}));
    edit(&mut session, ADD_VIA, json!({"at":{"x_nm":2_000_000,"y_nm":3_500_000}}));
    let report = session.execute(PREVIEW_FOOTPRINT_REVISION, &params).unwrap();
    let pads = report["delta"]["pads"].as_array().unwrap();
    assert_eq!(report["delta"]["pad_numbers_changed"], true);
    assert_eq!(pads.len(), 2);
    assert_eq!(pads[0]["number"], "02");
    assert_eq!(pads[0]["changes"], json!(["added"]));
    assert_eq!(pads[0]["loose_ends"], json!([]));
    assert_eq!(pads[1]["number"], "2");
    assert_eq!(pads[1]["changes"], json!(["removed"]));
    assert_eq!(pads[1]["before"]["board_center"], json!({"x_nm":2_000_000,"y_nm":3_500_000}));
    assert_eq!(pads[1]["before"]["mask_sides"], json!(["back"]));
    assert!(pads[1]["center_shift_nm"].is_null());
    let ends = pads[1]["loose_ends"].as_array().unwrap();
    assert_eq!(ends.len(), 1);
    assert_eq!(ends[0]["kind"], "via");
    assert!(ends[0]["gap_nm"].is_null());
    let mut filtered = params;
    filtered["pad"] = json!("2");
    let only_old = session.execute(PREVIEW_FOOTPRINT_REVISION, &filtered).unwrap();
    assert_eq!(only_old["delta"]["pads"], json!([pads[1]]));
    assert_eq!(only_old["delta"]["pad_numbers_changed"], true);
    filtered["pad"] = json!("3");
    let error = session.execute(PREVIEW_FOOTPRINT_REVISION, &filtered).unwrap_err();
    assert_eq!(error.code(), crate::error_codes::ErrorCode::NotFound);
    assert!(error.to_string().contains("Pad number not present"));
}

/// The owning part revision resolves the selected package, not the newest standalone drawing.
/// A surface-to-plated change reports drill/mask/paste changes without calling it a loose end.
#[test]
fn footprint_preview_resolves_owned_package_and_reports_drill_and_openings() {
    let (mut session, _, params) = fixture(
        |pad| {
            pad.width_nm = 650_000;
            pad.shape = PadShape::Oval;
            pad.corner_radius_nm = 0;
            pad.copper = CopperBinding::AllCopper;
            pad.drill_nm = Some(300_000);
        },
        true,
    );
    let report = session.execute(PREVIEW_FOOTPRINT_REVISION, &params).unwrap();
    assert_eq!(report["pinned_root"]["kind"], "part");
    assert_eq!(report["candidate_root"]["revision"], 2);
    assert_eq!(report["candidate_footprint"]["revision"], 2);
    let pad = &report["delta"]["pads"][0];
    assert_eq!(pad["changes"], json!(["resized", "shape", "copper_layers", "drill", "mask", "paste"]));
    assert_eq!(pad["after"]["geometry"]["drill_nm"], 300_000);
    assert_eq!(pad["after"]["mask_sides"], json!(["front", "back"]));
    assert_eq!(pad["after"]["paste_sides"], json!([]));
    assert_eq!(pad["loose_ends"], json!([]));
}

/// A circular surface land retains both openings when only its authored angle changes.
#[test]
fn footprint_preview_keeps_circular_openings_when_authored_rotation_changes() {
    let (mut session, _, params) = fixture_with_original(
        |pad| {
            pad.shape = PadShape::Circle;
            pad.height_nm = pad.width_nm;
            pad.corner_radius_nm = 0;
        },
        |pad| pad.rotation = 90,
        false,
    );
    let report = session.execute(PREVIEW_FOOTPRINT_REVISION, &params).unwrap();
    let pad = &report["delta"]["pads"][0];
    assert_eq!(pad["number"], "2");
    assert_eq!(pad["changes"], json!(["rotated"]));
    assert_eq!(pad["loose_ends"], json!([]));
}

/// A revision changing only a plated slot reports a drill change without triggering loose ends.
#[test]
fn footprint_preview_reports_plated_slot_only_changes() {
    let (mut session, _, params) = fixture_with_original(
        |pad| {
            pad.width_nm = 800_000;
            pad.height_nm = 500_000;
            pad.shape = PadShape::Oval;
            pad.corner_radius_nm = 0;
            pad.copper = CopperBinding::AllCopper;
            pad.drill_nm = None;
            pad.plated_slot = Some(PlatedSlot::new(200_000, 400_000, 0).unwrap());
        },
        |pad| pad.plated_slot = Some(PlatedSlot::new(200_000, 500_000, 0).unwrap()),
        false,
    );
    let report = session.execute(PREVIEW_FOOTPRINT_REVISION, &params).unwrap();
    let pad = &report["delta"]["pads"][0];
    assert_eq!(pad["number"], "2");
    assert_eq!(pad["changes"], json!(["drill"]));
    assert_eq!(pad["after"]["geometry"]["plated_slot"]["length_nm"], 500_000);
    assert_eq!(pad["loose_ends"], json!([]));
}

/// Rounded rotated bounds do not change a circle at angles that are not quarter turns, whether the
/// pad turns 0 to 30 degrees on a square placement or 0 to 90 degrees on a 23-degree placement.
/// A tangent track cap stays attached, as no opening or contact check changes.
#[test]
fn footprint_preview_keeps_circle_outline_at_non_quarter_angles() {
    for (placement, rotation) in [(0, 30), (23, 90)] {
        let (mut session, _, params) = fixture_with_original(
            |pad| {
                pad.shape = PadShape::Circle;
                pad.width_nm = 500_000;
                pad.height_nm = 500_000;
                pad.corner_radius_nm = 0;
                pad.rotation = 0;
            },
            |pad| pad.rotation = rotation,
            false,
        );
        let id = session.project().unwrap().boards().next().unwrap().footprint_ids().next().unwrap();
        edit(&mut session, SET_FOOTPRINT_TRANSFORM, json!({"footprint":id,"rotation":placement,"x_nm":0,"y_nm":0}));
        let (x, y) = crate::board::rotate(500_000, 0, placement);
        let front = layer(&session, Side::Front);
        edit(
            &mut session,
            ADD_TRACKS,
            json!({"layer":front,"width_nm":100_000,
                "points":[{"x_nm":x + 600_000,"y_nm":y},{"x_nm":x + 300_000,"y_nm":y}]}),
        );
        let report = session.execute(PREVIEW_FOOTPRINT_REVISION, &params).unwrap();
        let pad = &report["delta"]["pads"][0];
        assert_eq!(pad["number"], "2");
        assert_eq!(pad["changes"], json!(["rotated"]), "placement {placement}, pad {rotation}");
        assert_eq!(pad["loose_ends"], json!([]), "placement {placement}, pad {rotation}");
    }
}

/// At the response limit, a dense pad still fits in the live reply and MCP JSON text wrapper.
/// One more contact fails closed, with distinct recovery for full and single-pad queries.
#[test]
fn footprint_preview_bounds_dense_single_pad_replies() {
    let (mut session, _, params) = fixture_with_original(
        |pad| {
            pad.copper = CopperBinding::AllCopper;
            pad.drill_nm = Some(100_000);
        },
        |pad| pad.center.x_nm += 3_000_000,
        false,
    );
    Arc::make_mut(&mut Arc::make_mut(session.project.as_mut().unwrap()).board).setup = Some(BoardSetup::preset(64).unwrap());
    let front = layer(&session, Side::Front);
    let track = |ordinal: u128| {
        let offset = i64::try_from(ordinal).unwrap();
        Track {
            id: uuid::Uuid::from_u128(ordinal),
            locked: false,
            layer: front,
            width_nm: 100_000,
            start: crate::schematic::Point { x_nm: 500_000 - offset, y_nm: 0 },
            end: crate::schematic::Point { x_nm: 500_000 + offset, y_nm: 0 },
        }
    };
    // Seed thousands of valid straight segments directly in this disposable fixture. They
    // touch only the old land; exercising route creation or an undo stack is not needed here.
    Arc::make_mut(&mut Arc::make_mut(session.project.as_mut().unwrap()).board).tracks = (1..=5_120).map(track).collect();
    let mut filtered = params.clone();
    filtered["pad"] = json!("2");
    let report = session.execute(PREVIEW_FOOTPRINT_REVISION, &filtered).unwrap();
    assert_eq!(report["delta"]["pads"][0]["loose_ends"].as_array().unwrap().len(), 10_240);
    let live_reply = json!({"result":report}).to_string();
    let stdio_reply = json!({"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":report.to_string()}]}}).to_string();
    // Reserve headroom under the 16 MiB live reply reader and 8 MiB agent subprocess line reader.
    assert!(live_reply.len() < (4 << 20));
    assert!(stdio_reply.len() < (4 << 20));

    Arc::make_mut(&mut Arc::make_mut(session.project.as_mut().unwrap()).board).tracks.push(track(5_121));
    let before = session.snapshot().unwrap();
    let whole = session.execute(PREVIEW_FOOTPRINT_REVISION, &params).unwrap_err();
    let single = session.execute(PREVIEW_FOOTPRINT_REVISION, &filtered).unwrap_err();
    for error in [&whole, &single] {
        assert_eq!(error.code(), crate::error_codes::ErrorCode::CapExceeded);
    }
    assert!(whole.to_string().contains("request one pad by number"));
    assert!(single.to_string().contains("inspect its copper in the board editor"));
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
}

/// Target errors, unavailable catalog content, and a conflicting latest revision never substitute
/// another board, instance, footprint or older candidate.
#[test]
fn footprint_preview_refuses_unknown_targets_offline_and_conflicting_sources() {
    let (mut session, documents, params) = fixture(|pad| pad.center.x_nm += 200_000, false);
    let before = session.snapshot().unwrap();
    let mut bad = params.clone();
    bad["board"] = json!(uuid::Uuid::nil());
    let error = session.execute(PREVIEW_FOOTPRINT_REVISION, &bad).unwrap_err();
    assert_eq!(error.code(), crate::error_codes::ErrorCode::NotFound);
    assert!(error.to_string().contains("unknown board"));
    bad = params.clone();
    bad["part_instance"] = json!("component-9999");
    let error = session.execute(PREVIEW_FOOTPRINT_REVISION, &bad).unwrap_err();
    assert_eq!(error.code(), crate::error_codes::ErrorCode::NotFound);
    assert!(error.to_string().contains("Unknown part instance"));
    session.execute(REMOVE, &json!({"source":"test"})).unwrap();
    let error = session.execute(PREVIEW_FOOTPRINT_REVISION, &params).unwrap_err();
    assert_eq!(error.code(), crate::error_codes::ErrorCode::Unavailable);
    assert!(error.to_string().contains("Source unavailable"));
    load(&mut session, &documents);
    session.execute(PREVIEW_FOOTPRINT_REVISION, &params).unwrap();
    let mut conflicting = documents;
    let second = conflicting
        .values_mut()
        .find(|text| {
            let native: Value = serde_json::from_str(text).unwrap();
            native["format"] == "pcbcraft-footprint/1" && native["revision"] == 2
        })
        .unwrap();
    let mut native: Value = serde_json::from_str(second).unwrap();
    native["name"] = json!("Conflicting name");
    *second = native.to_string();
    session.execute(LOAD, &json!({"source":"conflicting","documents":conflicting})).unwrap();
    assert!(session.execute(PREVIEW_FOOTPRINT_REVISION, &params).unwrap_err().to_string().contains("Conflict"));
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
}
