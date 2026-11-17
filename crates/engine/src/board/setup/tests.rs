//! Board setup acceptance through shared commands and the production codecs.

use super::*;
use crate::{INITIALIZE_BOARD_SETUP, PLACE_FOOTPRINT, PLACE_SYMBOL, SET_BOARD_SETUP, persistence};
use std::sync::Arc;

mod integers;

/// The reader's reason for refusing the stack fields under an older header.
const STACK_REFUSAL: &str = "require board format 23";

/// Start an ordinary project and initialize its board through the command catalog.
fn session(copper_layers: u64) -> Session {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true,})).unwrap();
    apply(&mut s, INITIALIZE_BOARD_SETUP, json!({"copper_layers":copper_layers}));
    s
}

/// Technical-layer candidates retain the source, topology and existing identities. Adoption
/// still uses the command's revision gate; invalid names, sides and copper fail before mutation.
#[test]
fn technical_layer_builder_is_disposable_until_shared_command_adoption() {
    let mut s = session(4);
    let before = s.snapshot().unwrap();
    let setup = before.board_setup().unwrap();
    let candidate = setup.with_added_technical_layer("Assembly notes".into(), LayerPurpose::User, None).unwrap();
    assert_eq!(s.snapshot().unwrap().board_setup().unwrap(), setup);
    assert_eq!(candidate.stackup(), setup.stackup());
    assert_eq!(candidate.layers().len(), setup.layers().len() + 1);
    for invalid in [
        ("Top", LayerPurpose::User, None),
        ("Extra copper", LayerPurpose::Copper, None),
        ("Unspecified side", LayerPurpose::Silkscreen, None),
        ("Sided edge", LayerPurpose::Outline, Some(Side::Front)),
    ] {
        assert!(setup.with_added_technical_layer(invalid.0.into(), invalid.1, invalid.2).is_err());
    }
    let revision = s.revision();
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":candidate}));
    assert_eq!(s.revision(), revision + 1);
    assert_eq!(s.project().unwrap().board_setup(), Some(&candidate));
    assert!(s.apply(SET_BOARD_SETUP, &json!({"setup":setup,"expected_revision":revision})).is_err());
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project().unwrap().board_setup(), Some(setup));
}

/// Supply the current revision as a GUI or headless MCP caller would.
fn apply(s: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(s.revision());
    s.apply(command, &params).unwrap();
}

/// Read a candidate from public inspection instead of reconstructing stored records.
fn setup(s: &Session) -> Value {
    s.inspect()["project"]["board"]["setup"].clone()
}

/// Find a layer by its current display name only for test setup, never as a foreign key.
fn layer<'a>(setup: &'a mut Value, name: &str) -> &'a mut Value {
    setup["layers"].as_array_mut().unwrap().iter_mut().find(|layer| layer["name"] == name).unwrap()
}

/// Add the supported footprint through commands; its component is created after setup.
fn populate(s: &mut Session) {
    apply(s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":0,"y_nm":0}));
    let symbol = s.inspect()["project"]["schematic"]["symbols"][0]["id"].clone();
    apply(s, PLACE_FOOTPRINT, json!({"symbol":symbol,"definition":"generic-through-hole-2","pin_1_pad":1,"x_nm":0,"y_nm":0}));
}

/// Failed adoption leaves the immutable snapshot, saved bytes, revision and both histories intact.
fn reject(s: &mut Session, command: &str, params: Value) {
    let snapshot = s.snapshot().unwrap();
    let state = s.inspect();
    let saved = persistence::encode_documents(snapshot.clone()).unwrap();
    assert!(s.apply(command, &params).is_err(), "accepted {params}");
    assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
    assert_eq!(s.inspect(), state);
    assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap(), saved);
}

/// Like `reject`, and the error names `reason`, so no other check refused the edit first.
fn reject_because(s: &mut Session, command: &str, params: Value, reason: &str) {
    let snapshot = s.snapshot().unwrap();
    let state = s.inspect();
    let saved = persistence::encode_documents(snapshot.clone()).unwrap();
    let error = s.apply(command, &params).expect_err(&format!("accepted {params}"));
    assert!(
        matches!(&error, CommandError::InvalidParams(message) if message.contains(reason)),
        "{params}: expected InvalidParams naming {reason:?}, got {error:?}"
    );
    assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
    assert_eq!(s.inspect(), state);
    assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap(), saved);
}

/// The saved documents fail to open, and the error names `reason`.
fn refused_because(documents: &persistence::Documents, reason: &str) {
    match persistence::decode_documents(documents) {
        Ok(_) => panic!("opened; expected {reason:?}"),
        Err(error) => assert!(error.contains(reason), "expected {reason:?}, got {error:?}"),
    }
}

/// Availability follows initialization, undo, reopen and legacy footprints, while execution
/// keeps state-specific diagnostics and stale-revision errors without changing the session.
#[test]
fn setup_availability_follows_board_state() {
    let mut s = Session::default();
    let candidate = setup(&session(2));
    let unavailable = |s: &mut Session, command, mut params: Value, expected: &str| {
        assert!(!s.available(command));
        params["expected_revision"] = json!(s.revision());
        let before = s.inspect();
        assert_eq!(s.apply(command, &params).unwrap_err().to_string(), expected);
        assert_eq!(s.inspect(), before);
    };
    unavailable(&mut s, INITIALIZE_BOARD_SETUP, json!({"copper_layers":2}), "no project is open");
    unavailable(&mut s, SET_BOARD_SETUP, json!({"setup":candidate}), "no project is open");
    s.apply("project.new", &json!({"create_board":true,})).unwrap();
    assert!(s.available(INITIALIZE_BOARD_SETUP));
    unavailable(&mut s, SET_BOARD_SETUP, json!({"setup":candidate}), "initialize board setup first");
    let stale_revision = s.revision();
    apply(&mut s, INITIALIZE_BOARD_SETUP, json!({"copper_layers":2}));
    assert!(s.available(SET_BOARD_SETUP));
    unavailable(&mut s, INITIALIZE_BOARD_SETUP, json!({"copper_layers":2}), "board setup already exists; edit it with pcb.set_board_setup");
    let stale = s.apply(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":stale_revision})).unwrap_err();
    assert!(stale.to_string().contains("project changed"));
    s.apply("edit.undo", &json!({})).unwrap();
    assert!(s.available(INITIALIZE_BOARD_SETUP));
    assert!(!s.available(SET_BOARD_SETUP));
    s.apply("edit.redo", &json!({})).unwrap();
    assert!(!s.available(INITIALIZE_BOARD_SETUP));
    assert!(s.available(SET_BOARD_SETUP));
    let documents = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    s.open_snapshot(persistence::decode_documents(&documents).unwrap());
    assert!(!s.available(INITIALIZE_BOARD_SETUP));
    assert!(s.available(SET_BOARD_SETUP));
    populate(&mut s);
    assert!(!s.available(INITIALIZE_BOARD_SETUP));
    assert!(s.available(SET_BOARD_SETUP), "populated boards still permit metadata edits");

    s.apply("project.new", &json!({"create_board":true,})).unwrap();
    populate(&mut s);
    assert!(s.available(INITIALIZE_BOARD_SETUP), "existing placements require explicit binding resolution at execution");
    let before = s.inspect();
    let error = s.apply(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":s.revision()})).unwrap_err();
    assert_eq!(error.to_string(), "initial board setup with existing footprints requires resolve_existing_footprints=true");
    assert_eq!(s.inspect(), before);
    unavailable(&mut s, SET_BOARD_SETUP, json!({"setup":candidate}), "initialize board setup first");
    s.apply("edit.undo", &json!({})).unwrap();
    assert!(s.available(INITIALIZE_BOARD_SETUP), "undoing the footprint restores initialization availability");
}

/// Presets introduce stable identities without claiming a material or thickness specification.
#[test]
fn presets_are_unknown_until_specified_and_undo_atomically() {
    for copper in [2, 4] {
        let mut s = session(copper);
        let configured = s.snapshot().unwrap();
        let board = configured.board_setup().unwrap();
        assert_eq!(board.layers().iter().filter(|l| l.purpose == LayerPurpose::Copper).count(), copper as usize);
        assert_eq!(board.stackup().strata.len(), (2 * copper - 1) as usize);
        assert_eq!(board.laminate_thickness_nm(), None);
        assert!(s.inspect()["project"]["board"]["laminate_thickness_nm"].is_null());
        assert!(board.stackup().strata.iter().all(|s| s.thickness_nm().is_none()));
        s.apply("edit.undo", &json!({})).unwrap();
        assert!(s.snapshot().unwrap().board_setup().is_none());
        s.apply("edit.redo", &json!({})).unwrap();
        assert!(Arc::ptr_eq(&configured, &s.snapshot().unwrap()));
        let revision = s.revision();
        reject(&mut s, INITIALIZE_BOARD_SETUP, json!({"copper_layers":copper,"expected_revision":revision}));
    }
}

/// Names and physical dimensions can change independently of identity, connectivity and order.
#[test]
fn exact_lengths_rename_and_one_record_diffs() {
    let mut s = session(2);
    let original = setup(&s);
    let before = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let mut candidate = original.clone();
    layer(&mut candidate, "Top")["name"] = json!("Signal");
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":candidate}));
    let after = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(
        before.iter().filter(|(path, bytes)| after.get(*path) != Some(*bytes)).map(|(p, _)| p.as_str()).collect::<Vec<_>>(),
        [crate::persistence::test_board_path(&before)]
    );
    let before_lines = std::str::from_utf8(&before[&crate::persistence::test_board_path(&before)]).unwrap().lines();
    let after_lines = std::str::from_utf8(&after[&crate::persistence::test_board_path(&after)]).unwrap().lines();
    assert_eq!(before_lines.zip(after_lines).filter(|(a, b)| a != b).count(), 1);
    let mut candidate = setup(&s);
    for (i, stratum) in candidate["stackup"]["strata"].as_array_mut().unwrap().iter_mut().enumerate() {
        stratum["thickness_nm"] = json!(if i == 1 { 2_540_000 } else { 35_000 });
    }
    candidate["stackup"]["strata"][1]["material"] = json!("Unqualified test laminate");
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":candidate}));
    assert_eq!(s.snapshot().unwrap().board_setup().unwrap().laminate_thickness_nm(), Some(2_610_000));
    assert_eq!(setup(&s)["stackup"]["id"], original["stackup"]["id"]);
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap(), after);
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(setup(&s), original);
    let snapshot = s.snapshot().unwrap();
    let revision = s.revision();
    let mut reordered = original;
    reordered["layers"].as_array_mut().unwrap().reverse();
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":reordered}));
    assert_eq!(s.revision(), revision);
    assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
    assert!(s.available("edit.redo"));
}

/// Empty boards can grow to four layers while outer layers and surviving records keep identity.
#[test]
fn expand_empty_board_without_reidentifying_outer_layers() {
    let mut s = session(2);
    let before = setup(&s);
    let mut candidate = before.clone();
    let extra = BoardSetup::preset(4).unwrap();
    candidate["layers"]
        .as_array_mut()
        .unwrap()
        .extend(extra.layers().iter().filter(|l| l.purpose == LayerPurpose::Copper && l.side.is_none()).map(|l| json!(l)));
    let strata = candidate["stackup"]["strata"].as_array_mut().unwrap();
    let bottom = strata.pop().unwrap();
    strata.extend(extra.stackup().strata[2..6].iter().map(|s| json!(s)));
    strata.push(bottom);
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":candidate}));
    let expanded = setup(&s);
    assert_eq!(expanded["stackup"]["strata"].as_array().unwrap().len(), 7);
    for item in before["layers"].as_array().unwrap() {
        assert!(expanded["layers"].as_array().unwrap().contains(item));
    }
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(setup(&s), before);
    s.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(setup(&s), expanded);
}

/// Existing footprints allow metadata edits but no implicit initialization or structural changes.
#[test]
fn populated_boards_reject_structural_edits() {
    let mut s = session(2);
    populate(&mut s);
    let mut candidate = setup(&s);
    layer(&mut candidate, "Top")["name"] = json!("Signal");
    candidate["stackup"]["strata"][1]["thickness_nm"] = json!(1_500_000);
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":candidate}));
    let mut candidate = setup(&s);
    candidate["layers"].as_array_mut().unwrap().retain(|l| l["purpose"] != "outline");
    let revision = s.revision();
    reject(&mut s, SET_BOARD_SETUP, json!({"setup":candidate,"expected_revision":revision}));
    let configured = s.snapshot().unwrap();
    let mut legacy = Session::default();
    legacy.apply("project.new", &json!({"create_board":true,})).unwrap();
    populate(&mut legacy);
    let revision = legacy.revision();
    reject(&mut legacy, INITIALIZE_BOARD_SETUP, json!({"copper_layers":2,"expected_revision":revision}));
    let documents = persistence::encode_documents(configured).unwrap();
    assert_eq!(persistence::encode_documents(persistence::decode_documents(&documents).unwrap()).unwrap(), documents);
}

/// Invalid values and references are rejected before adoption, including unknown future semantics.
#[test]
fn malformed_setups_never_change_state() {
    let mut s = session(2);
    let base = setup(&s);
    let mut candidates = Vec::new();
    for value in [json!(0), json!(-1), json!(1.5), json!(100_000_001), json!(u64::MAX), json!("100 mil"), Value::Null] {
        let mut c = base.clone();
        c["stackup"]["strata"][1]["thickness_nm"] = value;
        candidates.push(c);
    }
    for (path, value) in [
        ("/extra", json!(true)),
        ("/stackup/extra", json!(true)),
        ("/layers/0/extra", json!(true)),
        ("/stackup/strata/1/extra", json!(true)),
        ("/stackup/strata/1/material", json!(" padded ")),
        ("/layers/0/name", json!(" ")),
        ("/layers/0/name", json!("a".repeat(81))),
        ("/layers/0/purpose", json!("future-conductor")),
        ("/layers/0/side", Value::Null),
        ("/stackup/strata/0/layer", json!(Uuid::from_u128(1))),
        ("/stackup/strata/1/id", base["stackup"]["id"].clone()),
        ("/layers/0/id", json!(Uuid::nil())),
        ("/layers/0/id", base["layers"][1]["id"].clone()),
        ("/layers/0/name", base["layers"][1]["name"].clone()),
        ("/stackup/id", json!(Uuid::from_u128(2))),
    ] {
        let mut c = base.clone();
        let (parent, key) = path.rsplit_once('/').unwrap();
        c.pointer_mut(parent).unwrap()[key] = value;
        candidates.push(c);
    }
    let mut c = base.clone();
    c["stackup"]["strata"][2] = c["stackup"]["strata"][0].clone();
    candidates.push(c);
    let mut c = base.clone();
    c["stackup"]["strata"].as_array_mut().unwrap().swap(0, 1);
    candidates.push(c);
    let mut c = base.clone();
    layer(&mut c, "Top")["side"] = json!("back");
    candidates.push(c);
    let mut c = base.clone();
    for stratum in c["stackup"]["strata"].as_array_mut().unwrap() {
        stratum["thickness_nm"] = json!(50_000_000);
    }
    candidates.push(c);
    let mut c = base.clone();
    c["layers"] = json!([]);
    candidates.push(c);
    for candidate in candidates {
        let revision = s.revision();
        reject(&mut s, SET_BOARD_SETUP, json!({"setup":candidate,"expected_revision":revision}));
    }
    let revision = s.revision();
    reject(&mut s, SET_BOARD_SETUP, json!({"setup":base,"expected_revision":revision-1}));
    s.apply("edit.undo", &json!({})).unwrap();
    reject(&mut s, INITIALIZE_BOARD_SETUP, json!({"copper_layers":2,"expected_revision":revision}));
    let revision = s.revision();
    let snapshot = s.snapshot().unwrap();
    s.open_snapshot(snapshot);
    reject(&mut s, INITIALIZE_BOARD_SETUP, json!({"copper_layers":2,"expected_revision":revision}));
}

/// Current archives round-trip offline; legacy encoders and mismatched versions fail explicitly.
#[test]
fn versioned_storage_round_trips_without_silent_loss() {
    let s = session(4);
    let snapshot = s.snapshot().unwrap();
    let documents = persistence::encode_documents(snapshot.clone()).unwrap();
    let board: Value = serde_json::from_slice(&documents[&crate::persistence::test_board_path(&documents)]).unwrap();
    assert_eq!(board["format"], "pcbcraft-board/2");
    assert_eq!(persistence::encode_documents(persistence::decode_documents(&documents).unwrap()).unwrap(), documents);
    let packed = persistence::encode_packed(snapshot.clone()).unwrap();
    let reopened = persistence::decode_packed(&packed).unwrap().project;
    assert_eq!(persistence::encode_packed(reopened.clone()).unwrap(), packed);
    assert_eq!(persistence::encode_documents(reopened).unwrap(), documents);
    for encoding in [persistence::Encoding::Json, persistence::Encoding::Zip] {
        assert!(persistence::encode_file(snapshot.clone(), encoding).unwrap_err().contains("cannot represent board setup"));
    }
    let mut invalid = vec![];
    let mut b = board.clone();
    b["format"] = json!("pcbcraft-board/1");
    invalid.push(b);
    let mut b = board.clone();
    b["format"] = json!("pcbcraft-board/7");
    invalid.push(b);
    let mut b = board.clone();
    b.as_object_mut().unwrap().remove("setup");
    invalid.push(b);
    let mut b = board.clone();
    b["setup"] = Value::Null;
    invalid.push(b);
    let mut b = board;
    b["setup"]["stackup"]["strata"][0]["thickness_nm"] = json!(-1);
    invalid.push(b);
    for board in invalid {
        let mut bad = documents.clone();
        bad.insert(crate::persistence::test_board_path(&bad), serde_json::to_vec(&board).unwrap());
        assert!(persistence::decode_documents(&bad).unwrap_err().contains("board.json"));
    }
}

/// Explicit initialization resolves authored bindings without rewriting any footprint, schematic
/// or drawing. Undo and offline reopen preserve the original IDs and exact placement bytes.
#[test]
fn populated_initialization_explicitly_resolves_existing_copper_bindings() {
    use crate::board::{definition::CopperBinding, view::CopperLayers};
    for count in [2, 4] {
        let mut s = Session::default();
        s.apply("project.new", &json!({"create_board":true,})).unwrap();
        for (index, definition) in ["resistor-0402", "header-1x02-254", "generic-through-hole-2"].into_iter().enumerate() {
            s.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":index as i64*20_000_000,"y_nm":0,"expected_revision":s.revision()}))
                .unwrap();
            let symbol = s.project().unwrap().symbols().last().unwrap().id();
            let mut params =
                json!({"symbol":symbol,"definition":definition,"x_nm":index as i64*20_000_000,"y_nm":0,"expected_revision":s.revision()});
            if index == 2 {
                params["pin_1_pad"] = json!(2);
            } else {
                params["mapping"] = json!({"1":"2","2":"1"});
            }
            s.apply(PLACE_FOOTPRINT, &params).unwrap();
        }
        let layer = s.project().unwrap().boards().next().unwrap().layers()[0].id;
        apply(&mut s, crate::ADD_DRAWING, json!({"layer":layer,"points":[{"x_nm":0,"y_nm":0},{"x_nm":30_000_000,"y_nm":0}],"closed":false}));
        let before = s.inspect();
        let bytes = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
        let revision = s.revision();
        for flag in [json!(false), json!("true"), json!(null)] {
            reject(&mut s, INITIALIZE_BOARD_SETUP, json!({"copper_layers":count,"resolve_existing_footprints":flag,"expected_revision":revision}));
        }
        reject(
            &mut s,
            INITIALIZE_BOARD_SETUP,
            json!({"board":"00000000-0000-4000-8000-000000000099","copper_layers":count,"resolve_existing_footprints":true,"expected_revision":revision}),
        );
        reject(&mut s, INITIALIZE_BOARD_SETUP, json!({"copper_layers":count,"resolve_existing_footprints":true,"expected_revision":revision-1}));
        let board = s.project().unwrap().board_target().unwrap().board;
        apply(&mut s, INITIALIZE_BOARD_SETUP, json!({"board":board,"copper_layers":count,"resolve_existing_footprints":true}));
        assert_eq!(s.revision(), revision + 1);
        let after = s.inspect();
        assert_eq!(after["project"]["schematic"], before["project"]["schematic"]);
        let footprint_records = |bytes: &std::collections::BTreeMap<String, Vec<u8>>| {
            serde_json::from_slice::<Value>(&bytes[&crate::persistence::test_board_path(bytes)]).unwrap()["footprints"].clone()
        };
        let adopted = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
        assert_eq!(footprint_records(&adopted), footprint_records(&bytes));
        let view = s.project().unwrap().boards().next().unwrap();
        let copper: Vec<_> = view
            .setup()
            .unwrap()
            .stackup()
            .strata
            .iter()
            .filter_map(|s| if let Stratum::Copper { layer, .. } = s { Some(*layer) } else { None })
            .collect();
        for (index, footprint) in view.footprints().enumerate() {
            for pad in footprint.pads {
                let expected = match index {
                    0 => Some(CopperLayers::Resolved { binding: CopperBinding::ComponentSide, layers: vec![copper[0]] }),
                    1 => Some(CopperLayers::Resolved { binding: CopperBinding::AllCopper, layers: copper.clone() }),
                    _ => None,
                };
                assert_eq!(pad.copper_layers, expected);
            }
        }
        assert_eq!(view.drawings().first().unwrap().layer, layer);
        assert_eq!(persistence::encode_documents(persistence::decode_documents(&adopted).unwrap()).unwrap(), adopted);
        s.apply("edit.undo", &json!({})).unwrap();
        assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap(), bytes);
        s.apply("edit.redo", &json!({})).unwrap();
        assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap(), adopted);
    }
}

/// Copper counts use the complete supported range, preserve surface/surviving IDs, and require
/// explicit pad-resolution consent on populated boards. Invalid or lossy reductions are atomic.
#[test]
fn expanded_copper_counts_preserve_identity_and_require_explicit_resolution() {
    for count in (4..=64).step_by(2) {
        let mut s = session(2);
        let before = s.snapshot().unwrap();
        let setup = before.board_setup().unwrap();
        let expanded = setup.with_copper_count(count).unwrap();
        assert_eq!(expanded.stackup().strata.len(), count as usize * 2 - 1);
        for surface in setup.layers().iter().filter(|l| l.purpose == LayerPurpose::Copper) {
            assert!(expanded.layers().contains(surface));
        }
        assert_eq!(expanded.with_copper_count(2).unwrap(), *setup);
        populate(&mut s);
        let unchanged = s.inspect();
        let revision = s.revision();
        assert!(s.apply(SET_BOARD_SETUP, &json!({"setup":expanded,"expected_revision":revision})).is_err());
        assert_eq!(s.inspect(), unchanged);
        s.apply(SET_BOARD_SETUP, &json!({"setup":expanded,"resolve_existing_footprints":true,"expected_revision":revision})).unwrap();
        assert_eq!(s.project().unwrap().board_setup(), Some(&expanded));
        let docs = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
        assert_eq!(persistence::encode_documents(persistence::decode_documents(&docs).unwrap()).unwrap(), docs);
        s.apply("edit.undo", &json!({})).unwrap();
        assert_eq!(s.inspect()["project"], unchanged["project"]);
    }
    let mut s = session(4);
    let mut value = setup(&s);
    value["stackup"]["strata"][3]["thickness_nm"] = json!(1_000_000);
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":value}));
    assert!(s.project().unwrap().board_setup().unwrap().with_copper_count(2).is_err());
    for count in [0, 1, 3, 65, 128] {
        assert!(s.project().unwrap().board_setup().unwrap().with_copper_count(count).is_err());
    }
}

/// Explicit topology consent still cannot remove a graphic's owning layer or swap physical
/// surface identities. Every rejection keeps model bytes, revision and undo/redo intact.
#[test]
fn populated_topology_consent_does_not_discard_drawings_or_rebind_surfaces() {
    let mut s = session(4);
    populate(&mut s);
    let layer = s.project().unwrap().board_setup().unwrap().layers().iter().find(|l| l.purpose == LayerPurpose::Outline).unwrap().id;
    apply(&mut s, crate::ADD_DRAWING, json!({"layer":layer,"points":[{"x_nm":0,"y_nm":0},{"x_nm":1000000,"y_nm":0}],"closed":false}));
    let before = s.inspect();
    let mut value = setup(&s);
    value["layers"].as_array_mut().unwrap().retain(|l| l["id"] != json!(layer));
    assert!(s.apply(SET_BOARD_SETUP, &json!({"setup":value,"resolve_existing_footprints":true,"expected_revision":s.revision()})).is_err());
    assert_eq!(s.inspect(), before);
    let fresh = BoardSetup::preset(4).unwrap();
    let mut value = serde_json::to_value(fresh).unwrap();
    value["stackup"]["id"] = setup(&s)["stackup"]["id"].clone();
    assert!(s.apply(SET_BOARD_SETUP, &json!({"setup":value,"resolve_existing_footprints":true,"expected_revision":s.revision()})).is_err());
    assert_eq!(s.inspect(), before);
}

/// Thickness fitting is a reusable engine candidate operation: fixed gaps/copper retain exact
/// values, residual nanometers are deterministic, and invalid locks/targets cannot alter source data.
#[test]
fn fitted_dielectrics_keep_fixed_data_and_reject_invalid_locks() {
    let mut s = session(4);
    let mut value = setup(&s);
    for (i, stratum) in value["stackup"]["strata"].as_array_mut().unwrap().iter_mut().enumerate() {
        stratum["thickness_nm"] = json!(if i % 2 == 0 { 35000 } else { 1000000 });
    }
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":value}));
    let source = s.project().unwrap().board_setup().unwrap();
    let Stratum::Dielectric { id, .. } = source.stackup().strata[1] else { panic!("gap") };
    let locks = BTreeSet::from([id]);
    let fitted = source.with_fitted_dielectrics(1600001, &locks).unwrap();
    assert_eq!(fitted.laminate_thickness_nm(), Some(1600001));
    assert_eq!(fitted.stackup().strata[1].thickness_nm(), Some(1000000));
    assert_eq!(fitted.stackup().strata[3].thickness_nm(), Some(230001));
    assert_eq!(fitted.stackup().strata[5].thickness_nm(), Some(230000));
    for target in [0, -1, 100000, MAX_THICKNESS_NM + 1] {
        assert!(source.with_fitted_dielectrics(target, &locks).is_err());
    }
    let copper = source.layers().iter().find(|l| l.purpose == LayerPurpose::Copper).unwrap().id;
    assert!(source.with_fitted_dielectrics(1600000, &BTreeSet::from([copper])).is_err());
    assert_eq!(source.laminate_thickness_nm(), Some(3140000));
}

/// Optional print settings validate with topology before publication, share one undo step and
/// retain byte-stable no-op behavior for both initialization and later setup edits.
#[test]
fn setup_and_silkscreen_publish_atomically() {
    for initialize in [true, false] {
        let mut s = if initialize {
            let mut s = Session::default();
            s.apply("project.new", &json!({"create_board":true,})).unwrap();
            s
        } else {
            session(2)
        };
        let before = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
        let revision = s.revision();
        let command = if initialize { INITIALIZE_BOARD_SETUP } else { SET_BOARD_SETUP };
        let mut params = if initialize {
            json!({"copper_layers":4})
        } else {
            let candidate = s.project().unwrap().board_setup().unwrap().with_copper_count(4).unwrap();
            json!({"setup":candidate})
        };
        params["expected_revision"] = json!(revision);
        params["silkscreen"] = json!({"text_height_nm":1_200_000,"stroke_width_nm":2_000_000,"pad_clearance_nm":250_000});
        assert!(s.apply(command, &params).is_err());
        assert_eq!(s.revision(), revision);
        assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap(), before);
        params["silkscreen"]["stroke_width_nm"] = json!(200_000);
        s.apply(command, &params).unwrap();
        assert_eq!(s.revision(), revision + 1);
        let after = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
        assert_ne!(after, before);
        assert!(s.apply(command, &params).is_err(), "stale form must fail");
        s.apply("edit.undo", &json!({})).unwrap();
        assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap(), before);
        s.apply("edit.redo", &json!({})).unwrap();
        assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap(), after);
        let revision = s.revision();
        let setup = s.project().unwrap().board_setup().unwrap().clone();
        s.apply(SET_BOARD_SETUP, &json!({"setup":setup,"silkscreen":params["silkscreen"],"expected_revision":revision})).unwrap();
        assert_eq!(s.revision(), revision);
        assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap(), after);
    }
}

/// Initial previews preserve legacy graphic layer identities without changing the session.
/// Complete drafts reject missing occupied layers and contradictory counts before adoption.
#[test]
fn complete_initial_setup_drafts_preserve_legacy_drawings() {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true,})).unwrap();
    let edge = s.project().unwrap().boards().next().unwrap().layers().into_iter().find(|l| l.purpose == LayerPurpose::Outline).unwrap().id;
    apply(&mut s, crate::ADD_DRAWING, json!({"layer":edge,"points":[{"x_nm":0,"y_nm":0},{"x_nm":1000000,"y_nm":0}],"closed":false}));
    let before = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    let draft = s.project().unwrap().boards().next().unwrap().setup_draft(4).unwrap();
    assert!(draft.layers().iter().any(|l| l.id == edge));
    assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap(), before);
    let mut missing = serde_json::to_value(&draft).unwrap();
    missing["layers"].as_array_mut().unwrap().retain(|l| l["id"] != json!(edge));
    for (count, value) in [(4, missing), (2, json!(draft))] {
        assert!(s.apply(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":count,"setup":value,"expected_revision":revision})).is_err());
        assert_eq!(s.revision(), revision);
        assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap(), before);
    }
    apply(&mut s, INITIALIZE_BOARD_SETUP, json!({"copper_layers":4,"setup":draft}));
    assert_eq!(s.project().unwrap().board_setup(), Some(&draft));
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap(), before);
}

/// Write the board's file, check its header, and require a decode/encode round trip to keep every byte.
fn saved_board(s: &Session, format: &str) -> persistence::Documents {
    let documents = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let board: Value = serde_json::from_slice(&documents[&persistence::test_board_path(&documents)]).unwrap();
    assert_eq!(board["format"], format);
    assert_eq!(persistence::encode_documents(persistence::decode_documents(&documents).unwrap()).unwrap(), documents);
    documents
}

/// Replace the board header of saved documents, keeping its body.
fn with_header(documents: &persistence::Documents, from: &str, to: &str) -> persistence::Documents {
    let path = persistence::test_board_path(documents);
    let text = String::from_utf8(documents[&path].clone()).unwrap();
    assert!(text.contains(from));
    let mut changed = documents.clone();
    changed.insert(path, text.replace(from, to).into_bytes());
    changed
}

/// Copper roles, base copper and finished thickness qualifiers, and the declared board thickness,
/// are one undoable setup edit that saves losslessly in its own board format. Unset values stay
/// unknown and boards without these fields keep their earlier format and bytes.
///
/// 47.9 µm is IPC-2221A clause 10.1.1, Table 10-1: the minimum external conductor thickness after
/// processing for 1 oz foil, Class 1 and 2. Top records it as nominal and Bottom as minimum, so both
/// bases are covered. The 35 µm foil for 1 oz follows IPC-2251 clause 4.1.1.3; the other values are
/// arbitrary.
#[test]
fn copper_strata_and_board_thickness_round_trip_in_board_format_23() {
    let mut s = session(4);
    let original = saved_board(&s, "pcbcraft-board/2");
    let revision = s.revision();
    let mut candidate = setup(&s);
    let ids: Vec<Uuid> = s.project().unwrap().board_setup().unwrap().copper_stack().map(|copper| copper.layer).collect();
    let [top, inner_1, inner_2, bottom] = ids[..] else { panic!("four copper layers") };
    let strata = &mut candidate["stackup"]["strata"];
    strata[0]["role"] = json!("signal");
    strata[0]["base_copper"] = json!({"weight_milli_oz":1000,"thickness_nm":35_000});
    strata[0]["thickness_nm"] = json!(47_900);
    strata[0]["thickness_tolerance_nm"] = json!(4_800);
    strata[0]["thickness_basis"] = json!("nominal");
    strata[4]["role"] = json!("plane");
    strata[6]["thickness_nm"] = json!(47_900);
    strata[6]["thickness_basis"] = json!("minimum");
    candidate["stackup"]["finished_thickness_nm"] = json!(1_600_000);
    candidate["stackup"]["finished_thickness_tolerance_nm"] = json!(160_000);
    candidate["stackup"]["declared_minimum_thickness_nm"] = json!(1_500_000);
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":candidate}));
    assert_eq!(s.revision(), revision + 1);
    assert_eq!(setup(&s), candidate);

    let documents = saved_board(&s, "pcbcraft-board/23");
    let reopened = persistence::decode_documents(&documents).unwrap();
    let stack = reopened.board_setup().unwrap();
    assert_eq!(stack, s.project().unwrap().board_setup().unwrap());
    let base = BaseCopper { weight_milli_oz: Some(1000), thickness_nm: Some(35_000) };
    let finished = CopperThickness { thickness_nm: 47_900, tolerance_nm: Some(4_800), basis: Some(ThicknessBasis::Nominal) };
    assert_eq!(
        stack.copper(top),
        Some(CopperFacts { layer: top, role: Some(CopperRole::Signal), base_copper: Some(base), finished: Some(finished) })
    );
    assert_eq!(stack.copper(inner_1), Some(CopperFacts { layer: inner_1, role: None, base_copper: None, finished: None }));
    assert_eq!(stack.copper(inner_2).unwrap().role, Some(CopperRole::Plane));
    assert_eq!(stack.copper(inner_2).unwrap().finished, None);
    assert_eq!(
        stack.copper(bottom).unwrap().finished,
        Some(CopperThickness { thickness_nm: 47_900, tolerance_nm: None, basis: Some(ThicknessBasis::Minimum) })
    );
    assert_eq!(stack.copper(stack.stackup().id), None);
    assert_eq!(stack.stackup().finished_thickness_nm, Some(1_600_000));
    assert_eq!(stack.stackup().finished_thickness_tolerance_nm, Some(160_000));
    assert_eq!(stack.board_thickness_for_checks(), Some((1_500_000, ThicknessBasis::Minimum)));
    let inspected = s.inspect()["project"]["board"]["setup"]["stackup"]["strata"].clone();
    assert!(inspected[2].get("thickness_nm").is_none() && inspected[4].get("thickness_nm").is_none());

    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(saved_board(&s, "pcbcraft-board/2"), original);
    s.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(saved_board(&s, "pcbcraft-board/23"), documents);

    // Headers 19 and 21 need rules or an origin, which this board lacks; their refusal of the
    // stack fields is shown on boards that have them (`board_format_23_carries_rules` and
    // `board::origin::tests::stack_format_carries_the_placement_origin`).
    for older in ["pcbcraft-board/3", "pcbcraft-board/5"] {
        refused_because(&with_header(&documents, "pcbcraft-board/23", older), STACK_REFUSAL);
    }
    // Header 20 requires zones; a zone-bearing fixture below tests its stack-field refusal.
    refused_because(&with_header(&documents, "pcbcraft-board/23", "pcbcraft-board/20"), "format 20 requires zones");
    refused_because(&with_header(&original, "pcbcraft-board/2", "pcbcraft-board/23"), STACK_REFUSAL);
}

/// Zone outlines survive stack fields with or without an origin; clearing the stack fields
/// restores the zone/origin format and byte-identical Undo/Redo snapshots.
#[test]
fn board_format_23_carries_zone_outlines() {
    for with_origin in [false, true] {
        let mut s = session(2);
        for x_nm in [0, 25_400_000] {
            apply(&mut s, crate::PLACE_RESISTOR, json!({"x_nm":x_nm,"y_nm":0}));
        }
        let symbols = s.inspect()["project"]["schematic"]["symbols"].clone();
        assert_eq!(symbols.as_array().unwrap().len(), 2);
        apply(
            &mut s,
            crate::CONNECT_PINS,
            json!({"from":{"symbol":symbols[0]["id"],"terminal":"1"},"to":{"symbol":symbols[1]["id"],"terminal":"1"}}),
        );
        let layer = s.project().unwrap().board_setup().unwrap().copper_stack().next().unwrap().layer;
        let nets = s.project().unwrap().inspect_net_registry().unwrap();
        assert_eq!(nets["nets"].as_array().unwrap().len(), 1);
        let anchor: crate::nets::Anchor = serde_json::from_value(nets["nets"][0]["anchor"].clone()).unwrap();
        anchor.validate_form().unwrap();
        assert_eq!(anchor.scope, s.project().unwrap().net_scope());
        apply(
            &mut s,
            crate::board::zones::ADD_ZONE,
            json!({
                "layer":layer,
                "points":[{"x_nm":0,"y_nm":0},{"x_nm":10_000_000,"y_nm":0},{"x_nm":10_000_000,"y_nm":10_000_000},{"x_nm":0,"y_nm":10_000_000}],
                "target":{"kind":"derived","anchor":anchor}
            }),
        );
        if with_origin {
            let board = s.project().unwrap().board_target().unwrap().board;
            apply(&mut s, crate::board::origin::SET_PLACEMENT_ORIGIN, json!({"board":board,"origin":{"kind":"board_zero"}}));
        }
        let earlier_format = if with_origin { "pcbcraft-board/21" } else { "pcbcraft-board/20" };
        let original = saved_board(&s, earlier_format);
        let plain = setup(&s);
        let mut candidate = plain.clone();
        candidate["stackup"]["strata"][0]["role"] = json!("signal");
        apply(&mut s, SET_BOARD_SETUP, json!({"setup":candidate}));
        let stacked = saved_board(&s, "pcbcraft-board/23");
        refused_because(&with_header(&stacked, "pcbcraft-board/23", earlier_format), STACK_REFUSAL);
        let path = persistence::test_board_path(&stacked);
        let document: Value = serde_json::from_slice(&stacked[&path]).unwrap();
        assert_eq!(document["zones"].as_array().unwrap().len(), 1);
        assert_eq!(document.get("placement_origin").is_some(), with_origin);
        apply(&mut s, SET_BOARD_SETUP, json!({"setup":plain}));
        assert_eq!(saved_board(&s, earlier_format), original);
        s.apply("edit.undo", &json!({})).unwrap();
        assert_eq!(saved_board(&s, "pcbcraft-board/23"), stacked);
        s.apply("edit.redo", &json!({})).unwrap();
        assert_eq!(saved_board(&s, earlier_format), original);
    }
}

/// Without a declared minimum, checks use the nominal finished thickness and say so.
#[test]
fn board_thickness_for_checks_says_which_value_it_uses() {
    let mut s = session(2);
    assert_eq!(s.project().unwrap().board_setup().unwrap().board_thickness_for_checks(), None);
    let mut candidate = setup(&s);
    candidate["stackup"]["finished_thickness_nm"] = json!(1_600_000);
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":candidate}));
    assert_eq!(s.project().unwrap().board_setup().unwrap().board_thickness_for_checks(), Some((1_600_000, ThicknessBasis::Nominal)));
    saved_board(&s, "pcbcraft-board/23");
}

/// Qualifiers without their value, empty or out-of-range base copper, inverted board limits and
/// explicit nulls are refused before any state, history or saved byte changes.
#[test]
fn invalid_copper_and_board_thickness_fields_never_change_state() {
    let mut s = session(2);
    let base = setup(&s);
    let mut invalid = vec![];
    for (field, value, reason) in [
        ("thickness_tolerance_nm", json!(1_000), "copper tolerance requires a specified thickness"),
        ("thickness_basis", json!("nominal"), "copper thickness basis requires a specified thickness"),
        ("base_copper", json!({}), "base copper needs a weight or a foil thickness"),
        ("base_copper", json!({"weight_milli_oz":0}), "base copper weight must be from"),
        ("base_copper", json!({"thickness_nm":0}), "base copper foil thickness must be from"),
        ("base_copper", json!({"weight_milli_oz":1000,"grade":"rtf"}), "unknown field `grade`"),
        ("role", json!("ground"), "unknown variant `ground`"),
        ("role", Value::Null, "invalid type: null"),
    ] {
        let mut candidate = base.clone();
        candidate["stackup"]["strata"][0][field] = value;
        invalid.push((candidate, reason));
    }
    for (thickness, tolerance, reason) in [
        (35_000, 35_000, "copper tolerance requires a specified thickness with positive lower"),
        (35_000, -1, "copper tolerance requires a specified thickness with positive lower"),
    ] {
        let mut candidate = base.clone();
        candidate["stackup"]["strata"][0]["thickness_nm"] = json!(thickness);
        candidate["stackup"]["strata"][0]["thickness_tolerance_nm"] = json!(tolerance);
        invalid.push((candidate, reason));
    }
    for (stackup, reason) in [
        (json!({"finished_thickness_tolerance_nm":160_000}), "board thickness tolerance requires a finished thickness"),
        (
            json!({"finished_thickness_nm":1_600_000,"finished_thickness_tolerance_nm":1_600_000}),
            "board thickness tolerance requires a finished thickness with positive lower",
        ),
        (json!({"finished_thickness_nm":1_600_000,"declared_minimum_thickness_nm":1_600_001}), "declared minimum board thickness cannot exceed"),
        (json!({"finished_thickness_nm":0}), "finished board thickness must be a positive"),
        (json!({"declared_minimum_thickness_nm":0}), "declared minimum board thickness must be a positive"),
        (json!({"finished_thickness_nm":Value::Null}), "invalid type: null"),
    ] {
        let mut candidate = base.clone();
        for (field, value) in stackup.as_object().unwrap() {
            candidate["stackup"][field] = value.clone();
        }
        invalid.push((candidate, reason));
    }
    for (candidate, reason) in invalid {
        let revision = s.revision();
        reject_because(&mut s, SET_BOARD_SETUP, json!({"setup":candidate,"expected_revision":revision}), reason);
    }
}

/// Each new bound accepts its last good value, which also saves and reopens, and refuses the
/// first bad one, a wrong type and an unknown field; the file reader refuses an out-of-range weight.
#[test]
fn copper_and_board_thickness_bounds_accept_the_last_good_value() {
    let mut s = session(2);
    let base = setup(&s);
    let candidate = |copper: Value, stackup: Value| {
        let mut candidate = base.clone();
        for (field, value) in copper.as_object().unwrap() {
            candidate["stackup"]["strata"][0][field] = value.clone();
        }
        for (field, value) in stackup.as_object().unwrap() {
            candidate["stackup"][field] = value.clone();
        }
        candidate
    };
    let good = [
        (json!({"base_copper":{"weight_milli_oz":1}}), json!({})),
        (json!({"base_copper":{"weight_milli_oz":100_000}}), json!({})),
        (json!({"base_copper":{"thickness_nm":1}}), json!({})),
        (json!({"base_copper":{"thickness_nm":100_000_000}}), json!({})),
        (json!({"thickness_nm":35_000,"thickness_tolerance_nm":0,"thickness_basis":"minimum"}), json!({})),
        (json!({"thickness_nm":35_000,"thickness_tolerance_nm":34_999}), json!({})),
        (json!({"thickness_nm":99_000_000,"thickness_tolerance_nm":1_000_000}), json!({})),
        (json!({}), json!({"finished_thickness_nm":1})),
        (json!({}), json!({"finished_thickness_nm":100_000_000})),
        (json!({}), json!({"finished_thickness_nm":99_000_000,"finished_thickness_tolerance_nm":1_000_000})),
        (json!({}), json!({"finished_thickness_nm":1_600_000,"finished_thickness_tolerance_nm":0})),
        (json!({}), json!({"finished_thickness_nm":1_600_000,"declared_minimum_thickness_nm":1_600_000})),
        (json!({}), json!({"declared_minimum_thickness_nm":100_000_000})),
    ];
    for (copper, stackup) in good {
        apply(&mut s, SET_BOARD_SETUP, json!({"setup":candidate(copper.clone(), stackup.clone())}));
        let stored = setup(&s);
        for (field, value) in copper.as_object().unwrap() {
            assert_eq!(&stored["stackup"]["strata"][0][field], value);
        }
        for (field, value) in stackup.as_object().unwrap() {
            assert_eq!(&stored["stackup"][field], value);
        }
        saved_board(&s, "pcbcraft-board/23");
    }
    let bad = [
        (json!({"base_copper":{"weight_milli_oz":100_001}}), json!({}), "base copper weight must be from 0.001 to 100 oz"),
        (json!({"base_copper":{"weight_milli_oz":0}}), json!({}), "base copper weight must be from 0.001 to 100 oz"),
        (json!({"base_copper":{}}), json!({}), "base copper needs a weight or a foil thickness"),
        (json!({"base_copper":{"weight_milli_oz":-1}}), json!({}), "invalid value: integer `-1`"),
        (json!({"base_copper":{"weight_milli_oz":4_294_967_296_u64}}), json!({}), "invalid value: integer `4294967296`"),
        (json!({"base_copper":{"weight_milli_oz":"1000"}}), json!({}), "invalid type: string \"1000\""),
        (json!({"base_copper":{"weight_milli_oz":Value::Null}}), json!({}), "invalid type: null"),
        (json!({"base_copper":{"thickness_nm":100_000_001}}), json!({}), "base copper foil thickness must be from 1 nm to 100 mm"),
        (json!({"base_copper":{"thickness_nm":-1}}), json!({}), "base copper foil thickness must be from 1 nm to 100 mm"),
        (json!({"base_copper":{"thickness_nm":0}}), json!({}), "base copper foil thickness must be from 1 nm to 100 mm"),
        (json!({"thickness_nm":99_000_000,"thickness_tolerance_nm":1_000_001}), json!({}), "copper tolerance requires"),
        (json!({"thickness_nm":35_000,"thickness_basis":"typical"}), json!({}), "unknown variant `typical`"),
        (json!({"thickness_nm":35_000,"thickness_basis":Value::Null}), json!({}), "invalid type: null"),
        (json!({"thickness_nm":35_000,"thickness_tolerance_nm":Value::Null}), json!({}), "invalid type: null"),
        (json!({"base_copper":Value::Null}), json!({}), "invalid type: null"),
        (json!({"role":1}), json!({}), "invalid type: integer `1`"),
        (json!({}), json!({"finished_thickness_nm":100_000_001}), "finished board thickness must be"),
        (json!({}), json!({"finished_thickness_nm":-1}), "finished board thickness must be"),
        (json!({}), json!({"finished_thickness_nm":"1600000"}), "invalid type: string \"1600000\""),
        (json!({}), json!({"finished_thickness_nm":99_000_000,"finished_thickness_tolerance_nm":1_000_001}), "board thickness tolerance requires"),
        (json!({}), json!({"finished_thickness_nm":1_600_000,"finished_thickness_tolerance_nm":-1}), "board thickness tolerance requires"),
        (json!({}), json!({"finished_thickness_nm":1_600_000,"finished_thickness_tolerance_nm":Value::Null}), "invalid type: null"),
        (json!({}), json!({"declared_minimum_thickness_nm":100_000_001}), "declared minimum board thickness must be a positive"),
        (json!({}), json!({"declared_minimum_thickness_nm":0}), "declared minimum board thickness must be a positive"),
        (json!({}), json!({"declared_minimum_thickness_nm":Value::Null}), "invalid type: null"),
        (
            json!({}),
            json!({"finished_thickness_nm":1_600_000,"declared_minimum_thickness_nm":1_600_001}),
            "declared minimum board thickness cannot exceed",
        ),
        (json!({}), json!({"finish":"enig"}), "unknown field `finish`"),
    ];
    for (copper, stackup, reason) in bad {
        let revision = s.revision();
        reject_because(&mut s, SET_BOARD_SETUP, json!({"setup":candidate(copper, stackup),"expected_revision":revision}), reason);
    }
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":candidate(json!({"base_copper":{"weight_milli_oz":100_000}}), json!({}))}));
    let saved = saved_board(&s, "pcbcraft-board/23");
    let path = persistence::test_board_path(&saved);
    let text = String::from_utf8(saved[&path].clone()).unwrap();
    let field = text.find("\"weight_milli_oz\"").unwrap();
    let value = field + text[field..].find("100000").unwrap();
    let mut changed = saved.clone();
    changed.insert(path, format!("{}100001{}", &text[..value], &text[value + 6..]).into_bytes());
    refused_because(&changed, "base copper weight must be from 0.001 to 100 oz");
}

/// A copper-count reduction refuses to discard an inner layer that has only a role.
#[test]
fn reducing_copper_count_keeps_authored_roles() {
    let mut s = session(4);
    let mut candidate = setup(&s);
    candidate["stackup"]["strata"][2]["role"] = json!("plane");
    candidate["stackup"]["finished_thickness_nm"] = json!(1_600_000);
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":candidate}));
    let stack = s.project().unwrap().board_setup().unwrap();
    let error = stack.with_copper_count(2).unwrap_err();
    assert!(matches!(&error, CommandError::InvalidParams(m) if m.contains("before reducing the layer count")), "{error}");
    let mut plain = candidate.clone();
    plain["stackup"]["strata"][2].as_object_mut().unwrap().remove("role");
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":plain}));
    assert!(s.project().unwrap().board_setup().unwrap().with_copper_count(2).is_ok(), "only the role blocks the reduction");
    s.apply("edit.undo", &json!({})).unwrap();
    let stack = s.project().unwrap().board_setup().unwrap();
    let grown = stack.with_copper_count(6).unwrap();
    assert_eq!(grown.stackup().finished_thickness_nm, Some(1_600_000));
    assert_eq!(grown.copper_stack().filter(|copper| copper.role == Some(CopperRole::Plane)).count(), 1);
}

/// Board rules and the stack fields share format 23; removing the stack fields returns to format 19.
#[test]
fn board_format_23_carries_rules() {
    let mut s = crate::board::track::tests::routed_fixture();
    let board = s.project().unwrap().board_target().unwrap().board;
    apply(&mut s, crate::board::rules::SET_RULES, json!({"board":board,"rules":crate::board::rules::Values::default()}));
    let ruled = saved_board(&s, "pcbcraft-board/19");
    let mut candidate = setup(&s);
    candidate["stackup"]["strata"][0]["role"] = json!("mixed");
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":candidate}));
    let stacked = saved_board(&s, "pcbcraft-board/23");
    refused_because(&with_header(&stacked, "pcbcraft-board/23", "pcbcraft-board/19"), STACK_REFUSAL);
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(saved_board(&s, "pcbcraft-board/19"), ruled);
}

/// Locked copper and the stack fields share format 23; removing the stack fields returns to format 18.
#[test]
fn board_format_23_carries_locked_copper() {
    use crate::board::track::{ADD_VIA, SET_COPPER_LOCKED};
    let mut s = crate::board::track::tests::routed_fixture();
    let board = s.project().unwrap().board_target().unwrap().board;
    apply(&mut s, ADD_VIA, json!({"board":board,"at":{"x_nm":0,"y_nm":40_000_000}}));
    let via = s.project().unwrap().board.vias[0].id;
    apply(&mut s, SET_COPPER_LOCKED, json!({"board":board,"vias":[via],"locked":true}));
    let locked = saved_board(&s, "pcbcraft-board/18");
    let mut candidate = setup(&s);
    candidate["stackup"]["strata"][0]["role"] = json!("signal");
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":candidate}));
    saved_board(&s, "pcbcraft-board/23");
    assert!(s.project().unwrap().board.vias.iter().any(|via| via.locked));
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(saved_board(&s, "pcbcraft-board/18"), locked);
}
