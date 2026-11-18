//! Synthetic setup reviews through shared commands and the production codecs.
use super::*;
use crate::{ADD_TRACKS, INITIALIZE_BOARD_SETUP, PLACE_FOOTPRINT, PLACE_SYMBOL, SET_BOARD_SETUP, persistence};

/// Configure a fresh synthetic board through the shared command path.
fn session() -> Session {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    edit(&mut s, INITIALIZE_BOARD_SETUP, json!({"copper_layers":4}));
    s
}

/// Supply the current revision without disguising the reason of a later refusal.
fn edit(s: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(s.revision());
    s.apply(command, &params).unwrap();
}

/// Inspect the canonical setup and build an ordinary count-change candidate.
fn reduced(s: &Session) -> Value {
    json!(s.project().unwrap().board_setup().unwrap().with_copper_count(2).unwrap())
}

/// Freeze one current review and return its JSON command response.
fn preview(s: &mut Session, setup: Value, consent: bool) -> Value {
    s.execute(PREVIEW_BOARD_SETUP, &json!({"setup":setup,"resolve_existing_footprints":consent,"expected_revision":s.revision()})).unwrap()["board_setup_preview"].clone()
}

/// Assert both error category and reason, not merely that some check failed first.
fn unavailable(result: Result<u64, CommandError>, reason: &str) {
    let error = result.unwrap_err();
    assert!(matches!(error, CommandError::Unavailable(_)), "{error:?}");
    assert!(error.to_string().contains(reason), "{error}");
}

/// Three tracks on a removed inner layer appear by stable ID and kind. Preview and
/// refused Apply agree exactly and neither touches saved bytes, history or the snapshot.
#[test]
fn removed_inner_tracks_review_matches_refusal_without_authored_changes() {
    let mut s = session();
    let inner = s.project().unwrap().board_setup().unwrap().copper_stack().nth(1).unwrap().layer;
    edit(
        &mut s,
        ADD_TRACKS,
        json!({"layer":inner,"width_nm":200000,"points":[
            {"x_nm":0,"y_nm":0},{"x_nm":1000000,"y_nm":0},{"x_nm":2000000,"y_nm":0},{"x_nm":3000000,"y_nm":0}
        ]}),
    );
    let snapshot = s.snapshot().unwrap();
    let before = s.inspect();
    let saved = persistence::encode_documents(snapshot.clone()).unwrap();
    let candidate = reduced(&s);
    let report = preview(&mut s, candidate.clone(), false);
    assert_eq!(report["revision"], s.revision());
    assert!(!report["plan_token"].as_str().unwrap().is_empty());
    assert_eq!(report["can_apply"], false);
    assert_eq!(report["refusal"]["kind"], "invalid_params");
    let objects = report["changes"]["affected_objects"].as_array().unwrap();
    assert_eq!(objects.len(), 3);
    for track in &snapshot.board.tracks {
        assert!(objects.iter().any(|o| o["id"] == track.id.to_string() && o["kind"] == "track" && o["layers"] == json!([inner])));
    }
    assert_eq!(report["changes"]["removed_layers"].as_array().unwrap().len(), 2);
    let error = s.apply(SET_BOARD_SETUP, &json!({"setup":candidate,"plan_token":report["plan_token"],"expected_revision":s.revision()})).unwrap_err();
    assert!(matches!(error, CommandError::InvalidParams(_)));
    assert_eq!(error.to_string(), report["refusal"]["message"].as_str().unwrap());
    assert!(error.to_string().starts_with("tracks must stay on a copper layer"));
    assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
    assert_eq!(s.inspect(), before);
    assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap(), saved);
}

/// Pad and via spans are reviewed together; missing consent is the same real refusal,
/// while explicit consent permits one undoable compatible change on an otherwise empty layer.
#[test]
fn populated_consent_and_spanning_objects_share_apply_validation() {
    let mut s = session();
    edit(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":0,"y_nm":0}));
    let symbol = s.inspect()["project"]["schematic"]["symbols"][0]["id"].clone();
    edit(&mut s, PLACE_FOOTPRINT, json!({"symbol":symbol,"definition":"header-1x02-254","mapping":{"1":"1","2":"2"},"x_nm":0,"y_nm":0}));
    edit(&mut s, crate::ADD_VIA, json!({"at":{"x_nm":10000000,"y_nm":0},"via_diameter_nm":600000,"via_drill_nm":300000}));
    let old = s.snapshot().unwrap();
    let candidate = reduced(&s);
    let denied = preview(&mut s, candidate.clone(), false);
    assert_eq!(denied["requires_populated_board_consent"], true);
    assert_eq!(denied["can_apply"], false);
    assert_eq!(denied["refusal"]["kind"], "unavailable");
    let error = s.apply(SET_BOARD_SETUP, &json!({"setup":candidate,"expected_revision":s.revision()})).unwrap_err();
    assert_eq!(error.to_string(), denied["refusal"]["message"].as_str().unwrap());
    let report = preview(&mut s, candidate.clone(), true);
    assert_eq!(report["can_apply"], true);
    let objects = report["changes"]["affected_objects"].as_array().unwrap();
    assert_eq!(objects.iter().filter(|o| o["kind"] == "pad").count(), 2);
    assert_eq!(objects.iter().filter(|o| o["kind"] == "via").count(), 1);
    let revision = s.revision();
    edit(&mut s, SET_BOARD_SETUP, json!({"setup":candidate,"resolve_existing_footprints":true,"plan_token":report["plan_token"]}));
    assert_eq!(s.revision(), revision + 1);
    assert!(s.board_setup_preview().is_none());
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project().unwrap().board_setup(), old.board_setup());
}

/// Expanding a stack reports the existing ordered span, not the two new inner copper IDs.
#[test]
fn expanding_stack_reports_previous_via_and_pad_spans() {
    let mut s = session();
    let two = reduced(&s);
    edit(&mut s, SET_BOARD_SETUP, json!({"setup":two}));
    edit(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":0,"y_nm":0}));
    let symbol = s.inspect()["project"]["schematic"]["symbols"][0]["id"].clone();
    edit(&mut s, PLACE_FOOTPRINT, json!({"symbol":symbol,"definition":"header-1x02-254","mapping":{"1":"1","2":"2"},"x_nm":0,"y_nm":0}));
    edit(&mut s, crate::ADD_VIA, json!({"at":{"x_nm":10000000,"y_nm":0},"via_diameter_nm":600000,"via_drill_nm":300000}));
    let before = s.project().unwrap().board_setup().unwrap();
    let old_span: Vec<_> = before.copper_stack().map(|layer| layer.layer).collect();
    let expanded = before.with_copper_count(4).unwrap();
    let new_inner: Vec<_> = expanded.copper_stack().skip(1).take(2).map(|layer| layer.layer).collect();
    let report = preview(&mut s, json!(expanded), true);
    assert_eq!(report["can_apply"], true);
    let added = report["changes"]["added_layers"].as_array().unwrap();
    assert_eq!(added.len(), 2);
    assert!(new_inner.iter().all(|id| added.iter().any(|layer| layer["id"] == json!(id))));
    let objects = report["changes"]["affected_objects"].as_array().unwrap();
    assert_eq!(objects.iter().filter(|object| object["kind"] == "pad").count(), 2);
    assert_eq!(objects.iter().filter(|object| object["kind"] == "via").count(), 1);
    assert!(objects.iter().all(|object| object["layers"] == json!(old_span)));
}

/// A save replacing the current snapshot revokes both uses of the token at the same revision.
#[test]
fn save_metadata_adoption_revokes_setup_review_at_same_revision() {
    let mut s = session();
    let candidate = reduced(&s);
    let report = preview(&mut s, candidate.clone(), false);
    let plan_token = &report["plan_token"];
    let revision = s.revision();
    let original = s.snapshot().unwrap();
    let saved = Arc::new(Project { name: "Saved".to_owned(), ..Project::clone(&original) });
    assert!(s.available(CANCEL_BOARD_SETUP_PREVIEW));
    assert!(s.adopt_saved_metadata(&original, &saved));
    assert_eq!(s.revision(), revision);
    assert!(Arc::ptr_eq(&saved, &s.snapshot().unwrap()));
    assert!(s.setup_preview.is_none());
    assert!(s.board_setup_preview().is_none());
    assert!(!s.available(CANCEL_BOARD_SETUP_PREVIEW));
    unavailable(s.apply(CANCEL_BOARD_SETUP_PREVIEW, &json!({"plan_token":plan_token})), "unknown or replaced");
    unavailable(
        s.apply(SET_BOARD_SETUP, &json!({"setup":candidate,"plan_token":plan_token,"expected_revision":revision})),
        "unknown, stale or replaced",
    );
}

/// A malformed optional style is an input error, never a replacement preview capability.
#[test]
fn invalid_silkscreen_candidates_preserve_current_authority() {
    let mut s = session();
    let setup = reduced(&s);
    let old = preview(&mut s, setup.clone(), false);
    let snapshot = s.snapshot().unwrap();
    for (style, reason) in [
        (json!({}), "text_height_nm"),
        (json!({"text_height_nm":1000000,"stroke_width_nm":150000}), "pad_clearance_nm"),
        (json!({"text_height_nm":1000000,"stroke_width_nm":150000,"pad_clearance_nm":0,"unknown":true}), "unknown"),
        (json!({"text_height_nm":0,"stroke_width_nm":1,"pad_clearance_nm":0}), "positive text height"),
        (json!({"text_height_nm":10000001,"stroke_width_nm":1,"pad_clearance_nm":0}), "positive text height"),
        (json!({"text_height_nm":2,"stroke_width_nm":2,"pad_clearance_nm":0}), "stroke width"),
        (json!({"text_height_nm":2,"stroke_width_nm":1,"pad_clearance_nm":-1}), "clearance"),
        (json!({"text_height_nm":2,"stroke_width_nm":1,"pad_clearance_nm":10000001}), "clearance"),
    ] {
        let error = s.apply(PREVIEW_BOARD_SETUP, &json!({"setup":setup,"silkscreen":style,"expected_revision":s.revision()})).unwrap_err();
        assert!(matches!(error, CommandError::InvalidParams(_)), "{error:?}");
        assert!(error.to_string().contains(reason), "expected {reason}, got {error}");
        assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
        assert_eq!(s.board_setup_preview().unwrap().plan_token.to_string(), old["plan_token"].as_str().unwrap());
    }
    for style in [
        json!({"text_height_nm":2,"stroke_width_nm":1,"pad_clearance_nm":0}),
        json!({"text_height_nm":10000000,"stroke_width_nm":5000000,"pad_clearance_nm":10000000}),
    ] {
        s.apply(PREVIEW_BOARD_SETUP, &json!({"setup":setup,"silkscreen":style,"expected_revision":s.revision()})).unwrap();
    }
    assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
}

/// Dense native through-hole definitions exercise pad count times copper span, not only tracks.
#[test]
fn over_budget_pad_reports_refuse_preview_without_affecting_apply() {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    edit(&mut s, INITIALIZE_BOARD_SETUP, json!({"copper_layers":64}));
    edit(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":0,"y_nm":0}));
    let symbol = s.inspect()["project"]["schematic"]["symbols"][0]["id"].clone();
    edit(&mut s, PLACE_FOOTPRINT, json!({"symbol":symbol,"definition":"header-1x02-254","mapping":{"1":"1","2":"2"},"x_nm":0,"y_nm":0}));
    let project = s.project().unwrap();
    let mut definition = project.board.definition_for(&project.board.footprints()[0]).unwrap().clone();
    definition.library = "stack-review-fixture".into();
    definition.id = "dense-through-hole".into();
    definition.revision = 1;
    let pad = definition.pads[0].clone();
    definition.pads = (0..128)
        .map(|i| {
            let mut pad = pad.clone();
            pad.number = (i + 1).to_string();
            pad.center.x_nm = (i % 16) * 2_000_000;
            pad.center.y_nm = (i / 16) * 2_000_000;
            pad
        })
        .collect();
    let manifest = json!({"format":"pcbcraft-library/1","id":"stack-review-fixture","name":"Synthetic stack review","license":"MIT OR Apache-2.0"});
    s.apply("library.load_source", &json!({"source":"stack-review-fixture","documents":{"library.json":manifest.to_string(),"footprints/dense-through-hole.json":serde_json::to_string(&definition).unwrap()}})).unwrap();
    let disconnected: Vec<_> = (3..=128).map(|i| i.to_string()).collect();
    for i in 0..16 {
        edit(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":i*40_000_000,"y_nm":0}));
        let component = s.project().unwrap().components().last().unwrap().id();
        edit(
            &mut s,
            crate::ASSIGN_FOOTPRINT,
            json!({"component":component,"library":"stack-review-fixture","definition":"dense-through-hole","definition_revision":1,"mapping":{"1":"1","2":"2"},"unconnected_pads":disconnected}),
        );
        edit(&mut s, crate::PLACE_ASSIGNED_FOOTPRINT, json!({"component":component,"x_nm":i*40_000_000,"y_nm":40_000_000}));
    }
    let snapshot = s.snapshot().unwrap();
    let old = preview(&mut s, json!(snapshot.board_setup().unwrap()), false);
    let smaller = reduced(&s);
    let params = json!({"setup":smaller,"expected_revision":s.revision()});
    let preview_error = s.apply(PREVIEW_BOARD_SETUP, &params).unwrap_err();
    let apply_error = s.apply(SET_BOARD_SETUP, &params).unwrap_err();
    assert!(matches!(preview_error, CommandError::Unavailable(_)));
    assert!(preview_error.to_string().contains("complete board setup review exceeds its byte budget"));
    assert!(matches!(apply_error, CommandError::Unavailable(_)));
    assert_eq!(
        apply_error.to_string(),
        "populated layer changes require resolve_existing_footprints=true and compatible retained surface identities and layer roles"
    );
    assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
    assert_eq!(s.board_setup_preview().unwrap().plan_token.to_string(), old["plan_token"].as_str().unwrap());
}

/// Counting accepts the exact boundary and refuses one extra byte without retaining output.
#[test]
fn report_counter_stops_at_its_exact_byte_boundary() {
    let mut size = ReportSize::new(3);
    size.write_all(b"abc").unwrap();
    let error = size.write_all(b"d").unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert!(error.to_string().contains("byte budget"));
    assert_eq!(size.bytes, 3);
}

/// Capabilities are session-local, replaced, canceled, consumed even for no-ops and stale
/// after edits, undo and reopening; changing a candidate cannot reuse approval authority.
#[test]
fn setup_tokens_bind_candidate_and_opening_and_are_single_use() {
    let mut s = session();
    let candidate = reduced(&s);
    let first = preview(&mut s, candidate.clone(), false);
    let second = preview(&mut s, candidate.clone(), false);
    unavailable(
        s.apply(SET_BOARD_SETUP, &json!({"setup":candidate,"plan_token":first["plan_token"],"expected_revision":s.revision()})),
        "replaced board setup preview",
    );
    unavailable(s.apply(CANCEL_BOARD_SETUP_PREVIEW, &json!({"plan_token":first["plan_token"]})), "replaced board setup preview");
    s.apply(CANCEL_BOARD_SETUP_PREVIEW, &json!({"plan_token":second["plan_token"]})).unwrap();
    unavailable(
        s.apply(SET_BOARD_SETUP, &json!({"setup":candidate,"plan_token":second["plan_token"],"expected_revision":s.revision()})),
        "replaced board setup preview",
    );
    let report = preview(&mut s, candidate.clone(), false);
    let mut other = Session::default();
    other.open_snapshot(s.snapshot().unwrap());
    unavailable(
        other.apply(SET_BOARD_SETUP, &json!({"setup":candidate,"plan_token":report["plan_token"],"expected_revision":other.revision()})),
        "unknown",
    );
    let unchanged = json!(s.project().unwrap().board_setup().unwrap());
    let error = s.apply(SET_BOARD_SETUP, &json!({"setup":unchanged,"plan_token":report["plan_token"],"expected_revision":s.revision()})).unwrap_err();
    assert!(matches!(error, CommandError::InvalidParams(_)));
    assert!(error.to_string().contains("does not match the candidate"));
    s.apply("project.rename", &json!({"name":"Changed"})).unwrap();
    s.apply("edit.undo", &json!({})).unwrap();
    unavailable(s.apply(SET_BOARD_SETUP, &json!({"setup":candidate,"plan_token":report["plan_token"],"expected_revision":s.revision()})), "stale");
    let report = preview(&mut s, unchanged.clone(), false);
    let snapshot = s.snapshot().unwrap();
    let revision = s.revision();
    edit(&mut s, SET_BOARD_SETUP, json!({"setup":unchanged,"plan_token":report["plan_token"]}));
    assert_eq!(s.revision(), revision);
    assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
    unavailable(s.apply(SET_BOARD_SETUP, &json!({"setup":unchanged,"plan_token":report["plan_token"],"expected_revision":s.revision()})), "unknown");
    let report = preview(&mut s, unchanged.clone(), false);
    s.open_snapshot(snapshot);
    unavailable(s.apply(SET_BOARD_SETUP, &json!({"setup":unchanged,"plan_token":report["plan_token"],"expected_revision":s.revision()})), "unknown");
}

/// Drawings and authored zone outlines are both listed, including when drawing validation
/// refuses first. Without that removal, preview catches the zone refusal from Apply itself.
#[test]
fn occupied_drawings_and_zone_outlines_are_reviewed_before_apply() {
    let mut s = session();
    for x in [0, 25000000] {
        edit(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":x,"y_nm":0}));
    }
    let symbols = s.inspect()["project"]["schematic"]["symbols"].clone();
    edit(&mut s, crate::CONNECT_PINS, json!({"from":{"symbol":symbols[0]["id"],"terminal":"1"},"to":{"symbol":symbols[1]["id"],"terminal":"1"}}));
    let p = s.project().unwrap();
    let setup = p.board_setup().unwrap();
    let inner = setup.copper_stack().nth(1).unwrap().layer;
    let silk = setup.layers().iter().find(|l| l.purpose == crate::board::setup::LayerPurpose::Silkscreen).unwrap().id;
    let anchor = p.inspect_net_registry().unwrap()["nets"][0]["anchor"].clone();
    let points = json!([{"x_nm":0,"y_nm":0},{"x_nm":1000000,"y_nm":0},{"x_nm":1000000,"y_nm":1000000}]);
    edit(&mut s, crate::board::zones::ADD_ZONE, json!({"layer":inner,"points":points,"target":{"kind":"derived","anchor":anchor}}));
    edit(&mut s, crate::ADD_DRAWING, json!({"layer":silk,"points":points,"closed":true,"stroke_nm":10000}));
    let mut candidate = reduced(&s);
    candidate["layers"].as_array_mut().unwrap().retain(|l| l["id"] != silk.to_string());
    let report = preview(&mut s, candidate.clone(), false);
    let objects = report["changes"]["affected_objects"].as_array().unwrap();
    assert_eq!(objects.iter().filter(|o| o["kind"] == "drawing").count(), 1);
    assert_eq!(objects.iter().filter(|o| o["kind"] == "zone").count(), 1);
    let error = s.apply(SET_BOARD_SETUP, &json!({"setup":candidate,"expected_revision":s.revision()})).unwrap_err();
    assert!(matches!(error, CommandError::InvalidParams(_)));
    assert_eq!(error.to_string(), report["refusal"]["message"].as_str().unwrap());
    let candidate = reduced(&s);
    let report = preview(&mut s, candidate.clone(), false);
    assert!(report["refusal"]["message"].as_str().unwrap().starts_with("Zone needs a copper layer"));
    let error = s.apply(SET_BOARD_SETUP, &json!({"setup":candidate,"expected_revision":s.revision()})).unwrap_err();
    assert_eq!(error.to_string(), report["refusal"]["message"].as_str().unwrap());
}

/// At the authored track limit and 64 copper layers, one complete review stays bounded.
/// The queued test's wall time includes command-generated fixture construction, not just review.
#[test]
fn maximum_track_layer_review_is_complete_and_bounded() {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    edit(&mut s, INITIALIZE_BOARD_SETUP, json!({"copper_layers":64}));
    let inner = s.project().unwrap().board_setup().unwrap().copper_stack().nth(1).unwrap().layer;
    let mut count = 0;
    while count < crate::board::track::MAX_TRACKS {
        let n = (crate::board::track::MAX_TRACKS - count).min(crate::board::track::MAX_PATH_POINTS - 1);
        let points: Vec<_> = (count..=count + n).map(|i| json!({"x_nm":(i as i64)*20000,"y_nm":0})).collect();
        edit(&mut s, ADD_TRACKS, json!({"layer":inner,"width_nm":10000,"points":points}));
        count += n;
    }
    let candidate = reduced(&s);
    let snapshot = s.snapshot().unwrap();
    let report = preview(&mut s, candidate.clone(), false);
    assert_eq!(report["changes"]["affected_objects"].as_array().unwrap().len(), crate::board::track::MAX_TRACKS);
    assert!(serde_json::to_vec(&report["changes"]).unwrap().len() <= MAX_CHANGES_BYTES);
    assert!(serde_json::to_vec(&report).unwrap().len() <= MAX_REPORT_BYTES);
    let error = s.apply(SET_BOARD_SETUP, &json!({"setup":candidate,"expected_revision":s.revision()})).unwrap_err();
    assert!(matches!(error, CommandError::InvalidParams(_)));
    let reason = "tracks must stay on a copper layer of the board setup";
    assert_eq!(error.to_string(), reason);
    assert_eq!(report["refusal"], json!({"kind":"invalid_params","message":reason}));
    assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
}

/// Invalid addressing, revisions, candidate fields and token spellings fail before mutation.
#[test]
fn malformed_preview_inputs_preserve_current_authority_and_project() {
    let mut s = session();
    let candidate = reduced(&s);
    let report = preview(&mut s, candidate.clone(), false);
    let snapshot = s.snapshot().unwrap();
    for (params, reason, unavailable) in [
        (json!({"setup":candidate,"expected_revision":s.revision()-1}), "project changed", true),
        (json!({"setup":candidate,"expected_revision":s.revision(),"board":crate::fresh_id().unwrap()}), "unknown board ID", false),
        (json!({"setup":{},"expected_revision":s.revision()}), "missing field `layers`", false),
        (json!({"setup":candidate,"expected_revision":s.revision(),"extra":true}), "extra", false),
    ] {
        let error = s.apply(PREVIEW_BOARD_SETUP, &params).unwrap_err();
        assert_eq!(matches!(error, CommandError::Unavailable(_)), unavailable, "{error:?}");
        assert!(matches!(error, CommandError::InvalidParams(_) | CommandError::Unavailable(_)));
        assert!(error.to_string().contains(reason), "expected {reason}, got {error}");
        assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
        assert_eq!(s.board_setup_preview().unwrap().plan_token.to_string(), report["plan_token"].as_str().unwrap());
    }
    for bad in [json!(null), json!(""), json!("bad"), json!(Uuid::nil()), json!(7)] {
        let error = s.apply(SET_BOARD_SETUP, &json!({"setup":candidate,"expected_revision":s.revision(),"plan_token":bad})).unwrap_err();
        assert!(matches!(error, CommandError::InvalidParams(_)));
        assert!(error.to_string().contains("token") || error.to_string().contains("plan_token"), "{error}");
    }
}
