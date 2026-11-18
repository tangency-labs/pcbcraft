//! Original layer-group fixtures through catalog commands and production persistence.
use super::*;
use crate::{CommandError, INITIALIZE_BOARD_SETUP, PREVIEW_BOARD_SETUP, SET_BOARD_SETUP, Session, persistence};
use std::sync::Arc;

/// Seed a physical stack through shared commands, without hand-written project JSON.
fn session(count: u64) -> Session {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    s.apply(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":count,"expected_revision":s.revision()})).unwrap();
    s
}

/// Serialize the current typed setup for a complete, revision-checked candidate edit.
fn candidate(s: &Session) -> Value {
    json!(s.project().unwrap().board_setup().unwrap())
}

/// Adopt a valid draft through the only mutating stack command.
fn apply(s: &mut Session, setup: Value) {
    s.apply(SET_BOARD_SETUP, &json!({"setup":setup,"expected_revision":s.revision()})).unwrap();
}

/// Return physical copper identities rather than relying on catalog order or display names.
fn copper(s: &Session) -> Vec<Uuid> {
    s.project().unwrap().board_setup().unwrap().copper_stack().map(|c| c.layer).collect()
}

/// A role edit changes computed scopes but never physical membership; Undo restores the one edit.
#[test]
fn physical_and_role_groups_are_read_only_and_follow_stack_order() {
    let mut s = session(4);
    let ids = copper(&s);
    let mut setup = candidate(&s);
    for stratum in setup["stackup"]["strata"].as_array_mut().unwrap().iter_mut().step_by(2) {
        stratum["role"] = json!("signal");
    }
    apply(&mut s, setup.clone());
    let snapshot = s.snapshot().unwrap();
    let revision = s.revision();
    let query = s.execute(INSPECT_LAYER_GROUPS, &json!({})).unwrap();
    let group_ids = |name: &str| query["layer_groups"][name].as_array().unwrap().iter().map(|l| l["id"].clone()).collect::<Vec<_>>();
    assert_eq!(group_ids("outer"), vec![json!(ids[0]), json!(ids[3])]);
    assert_eq!(group_ids("inner"), vec![json!(ids[1]), json!(ids[2])]);
    assert_eq!(group_ids("signal"), ids.iter().map(|id| json!(id)).collect::<Vec<_>>());
    assert!(group_ids("plane").is_empty());
    assert_eq!(s.revision(), revision);
    assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
    setup["stackup"]["strata"][4]["role"] = json!("plane");
    apply(&mut s, setup);
    let groups = s.project().unwrap().board_setup().unwrap().layer_groups();
    assert_eq!(groups.plane.iter().map(|l| l.id).collect::<Vec<_>>(), vec![ids[2]]);
    assert_eq!(groups.signal.iter().map(|l| l.id).collect::<Vec<_>>(), vec![ids[0], ids[1], ids[3]]);
    assert_eq!(groups.inner.iter().map(|l| l.id).collect::<Vec<_>>(), vec![ids[1], ids[2]]);
    s.apply("edit.undo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
}

/// Mixed roles participate in both electrical scopes; an absent role invents neither.
#[test]
fn mixed_and_unknown_roles_have_explicit_memberships() {
    let mut s = session(4);
    let ids = copper(&s);
    let mut setup = candidate(&s);
    setup["stackup"]["strata"][2]["role"] = json!("mixed");
    apply(&mut s, setup);
    let groups = s.project().unwrap().board_setup().unwrap().layer_groups();
    assert_eq!(groups.plane.iter().map(|l| l.id).collect::<Vec<_>>(), vec![ids[1]]);
    assert_eq!(groups.signal.iter().map(|l| l.id).collect::<Vec<_>>(), vec![ids[1]]);
}

/// Saved membership uses stable IDs, canonical physical order and the lazy group-only format.
#[test]
fn named_memberships_round_trip_and_old_headers_refuse_the_field() {
    let mut s = session(4);
    let before = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let ids = copper(&s);
    let group = crate::fresh_id().unwrap();
    let mut setup = candidate(&s);
    setup["stackup"]["layer_groups"] = json!([{"id":group,"name":"Return layers","layers":[ids[3],ids[1]]}]);
    apply(&mut s, setup);
    let saved = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let board_path = persistence::test_board_path(&saved);
    let board: Value = serde_json::from_slice(&saved[&board_path]).unwrap();
    assert_eq!(board["format"], "pcbcraft-board/33");
    assert_eq!(board["setup"]["stackup"]["layer_groups"][0]["layers"], json!([ids[1], ids[3]]));
    assert!(board["setup"]["stackup"].get("outer").is_none());
    assert_eq!(persistence::encode_documents(persistence::decode_documents(&saved).unwrap()).unwrap(), saved);
    let mut old = saved.clone();
    for header in ["pcbcraft-board/3", "pcbcraft-board/31"] {
        let mut wrong = board.clone();
        wrong["format"] = json!(header);
        old.insert(board_path.clone(), serde_json::to_vec(&wrong).unwrap());
        assert!(persistence::decode_documents(&old).unwrap_err().contains("layer_groups field requires board format 33"));
        wrong["setup"]["stackup"]["layer_groups"] = json!([]);
        old.insert(board_path.clone(), serde_json::to_vec(&wrong).unwrap());
        assert!(persistence::decode_documents(&old).unwrap_err().contains("layer_groups field requires board format 33"));
    }
    let mut wrong = board;
    wrong["setup"]["stackup"]["layer_groups"] = json!([]);
    wrong["format"] = json!("pcbcraft-board/33");
    old.insert(board_path, serde_json::to_vec(&wrong).unwrap());
    assert!(persistence::decode_documents(&old).unwrap_err().contains("requires at least one group"));
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap(), before);
}

/// Removed copper drops out of a named group in Preview and Apply; Undo restores every member.
#[test]
fn removed_members_are_reported_and_pruned_by_the_shared_candidate() {
    let mut s = session(4);
    let ids = copper(&s);
    let group = json!({"id":crate::fresh_id().unwrap(),"name":"Inner","layers":[ids[1],ids[2]]});
    let mut setup = candidate(&s);
    setup["stackup"]["layer_groups"] = json!([group]);
    apply(&mut s, setup);
    let snapshot = s.snapshot().unwrap();
    let mut smaller = json!(snapshot.board_setup().unwrap().with_copper_count(2).unwrap());
    // Retain the deleted IDs in the request to exercise command normalization, not only resizing.
    smaller["stackup"]["layer_groups"] = json!([group]);
    let report = s.execute(PREVIEW_BOARD_SETUP, &json!({"setup":smaller,"expected_revision":s.revision()})).unwrap()["board_setup_preview"].clone();
    assert_eq!(report["changes"]["affected_groups"], json!([group]));
    assert_eq!(report["can_apply"], true);
    assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
    s.apply(SET_BOARD_SETUP, &json!({"setup":smaller,"plan_token":report["plan_token"],"expected_revision":s.revision()})).unwrap();
    assert!(s.project().unwrap().board_setup().unwrap().stackup().layer_groups[0].layers.is_empty());
    s.apply("edit.undo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
}

/// Pruning must not repair a malformed submitted membership in Preview or either Apply path.
#[test]
fn submitted_group_members_are_checked_before_pruning() {
    let mut s = session(64);
    let ids = copper(&s);
    let group = json!({"id":crate::fresh_id().unwrap(),"name":"All copper","layers":ids});
    let mut setup = candidate(&s);
    setup["stackup"]["layer_groups"] = json!([group]);
    apply(&mut s, setup);
    let snapshot = s.snapshot().unwrap();
    let revision = s.revision();
    let mut smaller = json!(snapshot.board_setup().unwrap().with_copper_count(2).unwrap());
    smaller["stackup"]["layer_groups"] = json!([group]);
    let valid = json!({"setup":smaller,"expected_revision":revision});
    let report = s.execute(PREVIEW_BOARD_SETUP, &valid).unwrap()["board_setup_preview"].clone();
    assert_eq!(report["changes"]["affected_groups"], json!([group]));
    assert_eq!(report["can_apply"], true);

    let mut oversized = ids.clone();
    oversized.push(ids[1]);
    for (members, reason) in [
        (vec![ids[0], ids[1], ids[1], ids[63]], "user layer groups need distinct submitted layer IDs"),
        (oversized, "user layer groups support at most 64 submitted members per group"),
    ] {
        let mut invalid = valid.clone();
        invalid["setup"]["stackup"]["layer_groups"][0]["layers"] = json!(members);
        for (command, token) in [(PREVIEW_BOARD_SETUP, None), (SET_BOARD_SETUP, None), (SET_BOARD_SETUP, Some(&report["plan_token"]))] {
            let mut params = invalid.clone();
            if let Some(token) = token {
                params["plan_token"] = token.clone();
            }
            let error = s.execute(command, &params).unwrap_err();
            assert!(matches!(&error, CommandError::InvalidParams(message) if message == reason), "{command}: {error}");
            assert_eq!(s.revision(), revision);
            assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
            assert_eq!(json!(s.board_setup_preview().unwrap().plan_token), report["plan_token"]);
        }
    }

    let mut approved = valid;
    approved["plan_token"] = report["plan_token"].clone();
    s.apply(SET_BOARD_SETUP, &approved).unwrap();
    assert_eq!(s.project().unwrap().board_setup().unwrap().stackup().layer_groups[0].layers, vec![ids[0], ids[63]]);
    s.apply("edit.undo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
}

/// Groups reject duplicate, foreign and malformed identities/names without silently repairing typos.
#[test]
fn group_validation_rejects_each_invalid_membership_reason() {
    let s = session(4);
    let ids = copper(&s);
    let mut setup = candidate(&s);
    let good = json!({"id":crate::fresh_id().unwrap(),"name":"Return","layers":[ids[0]]});
    setup["stackup"]["layer_groups"] = json!([good]);
    let validate = |groups: Value| {
        let mut value = setup.clone();
        value["stackup"]["layer_groups"] = groups;
        serde_json::from_value::<BoardSetup>(value)
    };
    assert!(validate(json!([good])).is_ok());
    for (field, value, reason) in [
        ("name", json!(""), "layer group name"),
        ("name", json!(" "), "layer group name"),
        ("name", json!("x".repeat(MAX_NAME + 1)), "layer group name"),
        ("id", json!(Uuid::nil()), "non-nil"),
        ("id", json!(ids[0]), "unique"),
        ("layers", json!([ids[0], ids[0]]), "distinct copper"),
        ("layers", json!([crate::fresh_id().unwrap()]), "distinct copper"),
        ("layers", json!([Uuid::nil()]), "distinct copper"),
        ("layers", json!(null), "sequence"),
        ("extra", json!(true), "unknown field"),
    ] {
        let mut bad = good.clone();
        bad[field] = value;
        let error = validate(json!([bad])).unwrap_err();
        assert!(error.to_string().contains(reason), "{field}: {error}");
    }
    let mut duplicate_name = good.clone();
    duplicate_name["id"] = json!(crate::fresh_id().unwrap());
    duplicate_name["name"] = json!("RETURN");
    assert!(validate(json!([good, duplicate_name])).unwrap_err().to_string().contains("names must be unique"));
}

/// Count and text limits accept their last legal values and refuse the first excess.
#[test]
fn maximum_named_memberships_are_bounded() {
    let s = session(64);
    let mut setup = candidate(&s);
    let ids = copper(&s);
    let mut groups: Vec<_> =
        (0..MAX_GROUPS).map(|n| json!({"id":crate::fresh_id().unwrap(),"name":format!("{n:02}{}","x".repeat(MAX_NAME-2)),"layers":ids})).collect();
    setup["stackup"]["layer_groups"] = json!(groups);
    let parsed: BoardSetup = serde_json::from_value(setup.clone()).unwrap();
    assert_eq!(parsed.stackup().layer_groups.len(), MAX_GROUPS);
    assert!(parsed.stackup().layer_groups.iter().all(|g| g.layers.len() == MAX_COPPER));
    groups.push(json!({"id":crate::fresh_id().unwrap(),"name":"Excess","layers":[]}));
    setup["stackup"]["layer_groups"] = json!(groups);
    assert!(serde_json::from_value::<BoardSetup>(setup).unwrap_err().to_string().contains("at most 64 user layer groups"));
}

/// Required fields, Unicode label bounds, whitespace, controls and repeated group identities
/// are validated independently of physical membership, not accepted as incomplete saved drafts.
#[test]
fn named_group_fields_and_unicode_labels_use_native_validation() {
    let s = session(4);
    let mut setup = candidate(&s);
    let good = json!({"id":crate::fresh_id().unwrap(),"name":"界".repeat(MAX_NAME),"layers":[]});
    setup["stackup"]["layer_groups"] = json!([good]);
    assert!(serde_json::from_value::<BoardSetup>(setup.clone()).is_ok());
    for field in ["id", "name", "layers"] {
        let mut missing = setup.clone();
        missing["stackup"]["layer_groups"][0].as_object_mut().unwrap().remove(field);
        let error = serde_json::from_value::<BoardSetup>(missing).unwrap_err();
        assert!(error.to_string().contains(&format!("missing field `{field}`")), "{error}");
        let mut null = setup.clone();
        null["stackup"]["layer_groups"][0][field] = Value::Null;
        let error = serde_json::from_value::<BoardSetup>(null).unwrap_err();
        assert!(error.to_string().contains("invalid type: null"), "{error}");
    }
    for name in ["界".repeat(MAX_NAME + 1), " padded".into(), "padded ".into(), "line\nfeed".into(), "\u{0085}name".into()] {
        let mut invalid = setup.clone();
        invalid["stackup"]["layer_groups"][0]["name"] = json!(name);
        let error = serde_json::from_value::<BoardSetup>(invalid).unwrap_err();
        assert!(error.to_string().contains("layer group name must contain"), "{error}");
    }
    let mut repeated = good.clone();
    repeated["name"] = json!("Different name");
    setup["stackup"]["layer_groups"] = json!([good, repeated]);
    let error = serde_json::from_value::<BoardSetup>(setup).unwrap_err();
    assert!(error.to_string().contains("IDs must be non-nil and unique"), "{error}");
}

/// A portable profile maps group member identities to the target board's retained copper.
#[test]
fn profile_groups_rebase_members_and_require_the_group_header() {
    let mut source = session(4);
    let source_ids = copper(&source);
    let mut setup = candidate(&source);
    setup["stackup"]["layer_groups"] = json!([{"id":crate::fresh_id().unwrap(),"name":"Inner","layers":[source_ids[1],source_ids[2]]}]);
    apply(&mut source, setup);
    let profile =
        super::super::profile::SetupProfile::new("Shared stack".into(), source.project().unwrap().board_setup().unwrap().clone(), Default::default())
            .unwrap();
    let bytes = profile.encode().unwrap();
    let mut raw: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(raw["format"], "pcbcraft-setup-profile/5");
    assert_eq!(super::super::profile::SetupProfile::decode(&bytes).unwrap(), profile);
    let target = session(4);
    let target_ids = copper(&target);
    let rebased = target.project().unwrap().board_setup().unwrap().with_profile(&profile).unwrap();
    assert_eq!(rebased.stackup().layer_groups[0].layers, vec![target_ids[1], target_ids[2]]);
    raw["format"] = json!("pcbcraft-setup-profile/1");
    raw["setup"]["stackup"]["layer_groups"] = json!([]);
    assert!(
        super::super::profile::SetupProfile::decode(&serde_json::to_vec(&raw).unwrap())
            .unwrap_err()
            .contains("layer_groups field requires setup profile version 5")
    );
    let mut query = session(4);
    let error = query.execute(INSPECT_LAYER_GROUPS, &json!({"board":crate::fresh_id().unwrap()})).unwrap_err();
    assert!(matches!(error, CommandError::InvalidParams(_)));
    assert!(error.to_string().contains("unknown board ID"));
}

/// Groups preserve reports, stack fields, origins, rules, zones, assigned vias and profile adoptions;
/// clearing groups restores the earlier format and bytes.
#[test]
fn groups_preserve_reports_vias_profile_adoptions_and_lazy_formats() {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    let edit = |s: &mut Session, command: &str, mut params: Value| {
        params["expected_revision"] = json!(s.revision());
        s.apply(command, &params).unwrap();
    };
    for x in [0, 25_400_000, 50_800_000] {
        edit(&mut s, crate::PLACE_RESISTOR, json!({"x_nm":x,"y_nm":0}));
    }
    edit(&mut s, crate::CONNECT_PINS, json!({"from":{"symbol":"symbol-2","terminal":"1"},"to":{"symbol":"symbol-3","terminal":"1"}}));
    edit(&mut s, INITIALIZE_BOARD_SETUP, json!({"copper_layers":4,"resolve_existing_footprints":true}));
    let board_id = s.project().unwrap().board_target().unwrap().board;
    let ids = copper(&s);
    let source = json!({"document":"Synthetic combined-feature report"});
    let mut material = json!(crate::board::materials::Material::new("Combined fixture laminate".into()).unwrap());
    material["thermal_conductivity"] = json!([{"conductivity_mw_per_m_k":1000,"basis":"measured","source":source}]);
    material["cte"] = json!([{"cte_ppb_per_k":50000,"basis":"measured","source":source}]);
    material["dielectric_strength"] = json!([{"breakdown_v":40000,"specimen_thickness_nm":1000000,"basis":"measured","source":source}]);
    material["cti"] = json!([{"material_group":"IIIa","basis":"measured","source":source}]);
    let mut setup = candidate(&s);
    setup["materials"] = json!([material]);
    setup["stackup"]["strata"][0]["role"] = json!("signal");
    apply(&mut s, setup);
    edit(&mut s, crate::board::origin::SET_PLACEMENT_ORIGIN, json!({"board":board_id,"origin":{"kind":"point","x_nm":3_000_000,"y_nm":-4_000_000}}));
    edit(&mut s, crate::board::rules::SET_RULES, json!({"board":board_id,"rules":crate::board::rules::Values::default()}));
    let nets = s.project().unwrap().inspect_net_registry().unwrap();
    edit(
        &mut s,
        crate::board::zones::ADD_ZONE,
        json!({"board":board_id,"layer":ids[0],"target":{"kind":"derived","anchor":nets["nets"][0]["anchor"]},
            "points":[{"x_nm":0,"y_nm":0},{"x_nm":10_000_000,"y_nm":0},
                {"x_nm":10_000_000,"y_nm":10_000_000},{"x_nm":0,"y_nm":10_000_000}]}),
    );
    edit(
        &mut s,
        crate::ADD_VIA,
        json!({"board":board_id,"at":{"x_nm":-30_000_000,"y_nm":-30_000_000},"target":{"kind":"derived","anchor":nets["nets"][0]["anchor"]}}),
    );
    let assembly = crate::fresh_id().unwrap();
    edit(
        &mut s,
        crate::board::assembly_profile::SET_ASSEMBLY_PROFILE,
        json!({"profile":assembly,"content":{"revision":1,"provenance":{"kind":"authored"},"name":"Combined fixture assembly"}}),
    );
    let adoption = json!({"board":board_id,"expected_revision":s.revision(),"profile":assembly,"revision":1});
    let preview = s.execute(crate::board::assembly_profile::PREVIEW_ASSEMBLY_PROFILE, &adoption).unwrap();
    edit(
        &mut s,
        crate::board::assembly_profile::ADOPT_ASSEMBLY_PROFILE,
        json!({"board":board_id,"profile":assembly,"revision":1,"acceptance":preview["acceptance"]}),
    );
    let earlier = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let mut setup = candidate(&s);
    setup["stackup"]["layer_groups"] = json!([{"id":crate::fresh_id().unwrap(),"name":"Inner returns","layers":[ids[1],ids[2]]}]);
    apply(&mut s, setup);
    let saved = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let path = persistence::test_board_path(&saved);
    let grouped: Value = serde_json::from_slice(&saved[&path]).unwrap();
    let ungrouped: Value = serde_json::from_slice(&earlier[&path]).unwrap();
    assert_eq!(ungrouped["format"], "pcbcraft-board/25");
    assert_eq!(grouped["format"], "pcbcraft-board/33");
    assert_eq!(grouped["vias"].as_array().unwrap().len(), 1);
    assert!(grouped["vias"][0]["net_assignment"].is_object());
    assert_eq!(grouped["assembly_profiles"], json!([{"profile":assembly,"revision":1}]));
    for field in ["placement_origin", "rules", "zones", "vias", "assembly_profiles"] {
        assert!(grouped[field].is_object() || grouped[field].is_array());
        assert_eq!(grouped[field], ungrouped[field]);
    }
    assert_eq!(grouped["setup"]["materials"], ungrouped["setup"]["materials"]);
    assert_eq!(grouped["setup"]["stackup"]["strata"], ungrouped["setup"]["stackup"]["strata"]);
    assert_eq!(persistence::encode_documents(persistence::decode_documents(&saved).unwrap()).unwrap(), saved);
    let profile =
        super::super::profile::SetupProfile::new("Combined setup".into(), s.project().unwrap().board_setup().unwrap().clone(), Default::default())
            .unwrap();
    let bytes = profile.encode().unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap()["format"], "pcbcraft-setup-profile/5");
    assert_eq!(super::super::profile::SetupProfile::decode(&bytes).unwrap(), profile);
    let mut without_groups = candidate(&s);
    without_groups["stackup"]["layer_groups"] = json!([]);
    apply(&mut s, without_groups);
    assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap(), earlier);
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap(), saved);
}
