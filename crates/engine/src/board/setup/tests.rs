//! Board setup acceptance through shared commands and the production codecs.

use super::*;
use crate::{INITIALIZE_BOARD_SETUP, PLACE_FOOTPRINT, PLACE_SYMBOL, SET_BOARD_SETUP, persistence};
use std::sync::Arc;

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
    assert_eq!(before.iter().filter(|(path, bytes)| after.get(*path) != Some(*bytes)).map(|(p, _)| p.as_str()).collect::<Vec<_>>(), ["board.json"]);
    let before_lines = std::str::from_utf8(&before["board.json"]).unwrap().lines();
    let after_lines = std::str::from_utf8(&after["board.json"]).unwrap().lines();
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
    let board: Value = serde_json::from_slice(&documents["board.json"]).unwrap();
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
        bad.insert("board.json".to_owned(), serde_json::to_vec(&board).unwrap());
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
            serde_json::from_slice::<Value>(&bytes["board.json"]).unwrap()["footprints"].clone()
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
