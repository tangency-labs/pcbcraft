use super::{commands::*, tests::*, *};
use crate::{
    Session,
    board::{
        self, clipboard,
        definition::{Artwork, CopperBinding, FootprintDefinition, GraphicPurpose, GraphicRect, PadShape},
        update,
    },
    library_graphics::{Fill, Geometry, Graphic},
};
use serde_json::{Value, json};
use std::sync::Arc;

fn pin(s: &Session) -> FootprintDefinition {
    let p = s.project().unwrap();
    p.board.definitions[&p.board.footprints()[0].key().unwrap()].as_ref().clone()
}

fn stage(s: &mut Session, d: FootprintDefinition, mapping: Value) {
    let (library, definition, revision) = d.key();
    let mut p = s.project().unwrap().clone();
    let component = p.board.footprints()[0].component;
    Arc::make_mut(&mut p.board).definitions.insert(d.key(), Arc::new(d));
    // Only the independent test asset is injected; native assignment, accepted ECO,
    // correspondence, disclosure and publication remain production operations.
    s.project = Some(Arc::new(p));
    run(s, crate::ASSIGN_FOOTPRINT, json!({"component":format!("component-{component}"),"library":library,"definition":definition,"definition_revision":revision,"mapping":mapping})).unwrap();
}

fn eco(s: &mut Session, choices: Value) -> Value {
    run(s, update::PREVIEW_UPDATE, choices).unwrap()["board_update_preview"].clone()
}

fn apply(s: &mut Session, p: &Value) {
    let ids = |key| p["relation_detail"][key].as_array().unwrap().iter().map(|row| row["id"].clone()).collect::<Vec<_>>();
    let checked = s.checked_eco_candidate().unwrap().clone();
    run(s, update::APPLY_UPDATE, json!({"plan_token":p["plan_token"],"accept_loss":ids("losses"),"confirm_remaps":ids("remaps")})).unwrap();
    assert!(Arc::ptr_eq(&checked, &s.snapshot().unwrap()));
}

#[test]
fn a_cat_ref_native_correspondence_pad_changes_and_choice_binding() {
    for case in ["same_pin", "move", "remove", "round", "swap", "copper_binding", "keep_lost", "repick"] {
        let mut s = fixture(true);
        let f = pick(&mut s, 0, "pad.center", Some("1"));
        let id = add(&mut s, "horizontal", origin(), f.clone());
        let authored = s.project().unwrap().board.relations.as_ref().unwrap().relations[0].value.unwrap();
        let mut d = pin(&s);
        let mut mapping = json!({"1":"1","2":"2"});
        if case != "same_pin" && case != "swap" {
            d.revision += 1;
        }
        match case {
            "move" | "keep_lost" | "repick" => d.pads[0].center.x_nm += 200_000,
            "remove" => {
                d.pads[0].number = "3".into();
                mapping["1"] = json!("3");
            }
            "round" => {
                d.pads[0].shape = PadShape::Circle;
                d.pads[0].height_nm = d.pads[0].width_nm;
                d.pads[0].corner_radius_nm = 0;
            }
            "swap" => {
                mapping = json!({"1":"2","2":"1"});
            }
            "copper_binding" => {
                d.pads[0].copper = CopperBinding::AllCopper;
                d.pads[0].drill_nm = Some(100_000);
            }
            _ => {}
        }
        if case == "same_pin" {
            let fp = s.project().unwrap().board.footprints()[0].id;
            let report = reviewed(
                &mut s,
                crate::SET_FOOTPRINT_TRANSFORM,
                json!({"footprint":format!("footprint-{fp}"),"x_nm":5_000_000,"y_nm":1_000_000,"rotation":90}),
            );
            assert_eq!(report["disclosure"]["losses"], json!([]));
            assert_eq!(report["disclosure"]["remaps"], json!([]));
            run(&mut s, APPLY, decision(&report)).unwrap();
            assert_eq!(run(&mut s, LIST, json!({})).unwrap()["relations"][0]["refs"][1], f);
            continue;
        }
        stage(&mut s, d, mapping);
        let first = eco(&mut s, json!({}));
        let detail = &first["relation_detail"];
        let corresponds = ["move", "keep_lost", "repick"].contains(&case);
        assert_eq!(detail["losses"].as_array().unwrap().len(), usize::from(!corresponds), "{case}");
        assert_eq!(detail["remaps"].as_array().unwrap().len(), usize::from(corresponds), "{case}");
        if matches!(case, "keep_lost" | "repick") {
            let candidate = s.checked_eco_candidate().unwrap().clone();
            let mut lookup = Session::default();
            lookup.open_snapshot(candidate);
            let replacement = pick(&mut lookup, 0, "pad.center", Some("2"));
            let choice = if case == "keep_lost" { json!({"kind":"keep_lost"}) } else { json!({"kind":"repick","reference":replacement}) };
            let preview = |choice: Value| json!({"remap_choices":[{"relation":id,"pick":1,"choice":choice}]});
            let explicit = eco(&mut s, preview(json!({"kind":"correspond"})));
            assert_ne!(explicit["intent_digest"], first["intent_digest"]);
            assert_eq!(explicit["relation_detail"]["remaps"][0]["new"], detail["remaps"][0]["new"]);
            let latest = eco(&mut s, preview(choice));
            assert_ne!(latest["intent_digest"], explicit["intent_digest"]);
            unchanged(&mut s, |s| {
                run(s, update::APPLY_UPDATE, json!({"plan_token":first["plan_token"],"accept_loss":[],"confirm_remaps":[detail["remaps"][0]["id"]]}))
            });
            unchanged(&mut s, |s| run(s, update::PREVIEW_UPDATE, preview(json!({"kind":"repick","reference":f}))));
            assert_eq!(s.board_update_report().unwrap(), latest);
            if case == "repick" {
                assert_eq!(latest["relation_detail"]["remaps"][0]["new"], replacement);
            }
            apply(&mut s, &latest);
        } else {
            if !corresponds {
                unchanged(&mut s, |s| {
                    run(s, update::PREVIEW_UPDATE, json!({"remap_choices":[{"relation":id,"pick":1,"choice":{"kind":"correspond"}}]}))
                });
                assert_eq!(s.board_update_report().unwrap(), first);
            }
            apply(&mut s, &first);
        }
        let status = if !corresponds || case == "keep_lost" { "lost" } else { "reference" };
        let result = run(&mut s, LIST, json!({})).unwrap();
        assert_eq!(result["relations"][0]["status"], status);
        if matches!(case, "move" | "repick") {
            let signed = result["relations"][0]["measure"]["signed_nm"].as_i64().unwrap();
            assert_ne!(signed, authored);
            let saved = bytes(&s);
            let path = crate::persistence::test_board_path(&saved);
            let document: Value = serde_json::from_slice(&saved[&path]).unwrap();
            let row = &document["relations"]["relations"][0];
            // Automatic correspondence updates the exact address, not the authored literal.
            // Deliberate re-pick, in contrast, writes the newly measured magnitude and side.
            let (value, side) = if case == "move" { (authored, 1) } else { (signed.abs(), signed.signum()) };
            assert_eq!(row["value"], value);
            assert_eq!(row["side"], side);
        }
    }
}

#[test]
fn a_cat_ref_curved_pad_and_courtyard_public_offering_and_correspondence() {
    for case in ["artwork", "duplicate_rect", "reorder"] {
        let mut s = fixture(false);
        let mut d = pin(&s);
        let court = GraphicRect {
            purpose: GraphicPurpose::Courtyard,
            center: Point { x_nm: 0, y_nm: 0 },
            width_nm: 3_000_000,
            height_nm: 4_000_000,
            rotation: 0,
        };
        let mut silk = court.clone();
        silk.purpose = GraphicPurpose::Silkscreen;
        d.graphics = vec![court.clone(), silk];
        d.pads[0].shape = PadShape::Circle;
        d.pads[0].height_nm = d.pads[0].width_nm;
        d.pads[0].corner_radius_nm = 0;
        d.pads[1].shape = PadShape::Oval;
        d.pads[1].height_nm = d.pads[1].width_nm + 200_000;
        d.pads[1].corner_radius_nm = 0;
        d.revision += 1;
        let old = s.project().unwrap().board.footprints()[0].clone();
        let component = format!("component-{}", old.component);
        run(&mut s, crate::DELETE_FOOTPRINTS, json!({"footprints":[format!("footprint-{}",old.id)]})).unwrap();
        stage_for_unplaced(&mut s, &component, d.clone());
        run(&mut s, crate::PLACE_ASSIGNED_FOOTPRINT, json!({"component":component,"x_nm":old.x_nm,"y_nm":old.y_nm})).unwrap();
        let index = 2;
        run(&mut s, update::INITIALIZE_BASELINE, json!({})).unwrap();
        let fp = s.project().unwrap().board.footprints()[index].id;
        let offered = |s: &mut Session| run(s, FEATURES, json!({"fp":format!("footprint-{fp}")})).unwrap()["features"].as_array().unwrap().clone();
        let rows = offered(&mut s);
        let count = |rows: &[Value], tag| rows.iter().filter(|r| r["reference"]["tag"] == tag).count();
        assert_eq!(count(&rows, "pad.center"), 2);
        assert_eq!(count(&rows, "pad.edge"), 0);
        assert_eq!(count(&rows, "pad.corner"), 0);
        assert_eq!(count(&rows, "courtyard.edge"), 4);
        let edge = pick(&mut s, index, "courtyard.edge", None);
        add(&mut s, "point_edge", origin(), edge);
        d.revision += 1;
        match case {
            "artwork" => {
                d.artwork.push(Artwork {
                    purpose: GraphicPurpose::Courtyard,
                    graphic: Graphic {
                        geometry: Geometry::Line { from: Point { x_nm: 0, y_nm: 0 }, to: Point { x_nm: 1000, y_nm: 0 } },
                        stroke_nm: 1000,
                        fill: Fill::None,
                    },
                });
            }
            "duplicate_rect" => d.graphics.push(court),
            _ => d.graphics.swap(0, 1),
        }
        stage_for_unplaced(&mut s, &component, d);
        let r = eco(&mut s, json!({}));
        assert_eq!(r["relation_detail"]["losses"].as_array().unwrap().len(), 1);
        assert_eq!(r["relation_detail"]["remaps"], json!([]));
        apply(&mut s, &r);
        assert_eq!(count(&offered(&mut s), "courtyard.edge"), if case == "reorder" { 4 } else { 0 });
    }
}

fn stage_for_unplaced(s: &mut Session, component: &str, d: FootprintDefinition) {
    stage_for(s, component, d, json!({"1":"1","2":"2"}));
}

fn stage_for(s: &mut Session, component: &str, d: FootprintDefinition, mapping: Value) {
    let (library, definition, revision) = d.key();
    let mut p = s.project().unwrap().clone();
    Arc::make_mut(&mut p.board).definitions.insert(d.key(), Arc::new(d));
    s.project = Some(Arc::new(p));
    run(
        s,
        crate::ASSIGN_FOOTPRINT,
        json!({"component":component,"library":library,"definition":definition,"definition_revision":revision,"mapping":mapping}),
    )
    .unwrap();
}

#[test]
fn a_cat_ref_explicit_eco_pick_cannot_disappear_into_other_pick_loss() {
    let mut s = fixture(true);
    let a = pick(&mut s, 0, "pad.center", Some("1"));
    let b = pick(&mut s, 1, "pad.center", Some("1"));
    let row = add(&mut s, "horizontal", a.clone(), b.clone());
    let before = run(&mut s, LIST, json!({})).unwrap();
    let mut da = pin(&s);
    let mut db: FootprintDefinition = board::definition::basic_components("resistor-0603").unwrap().as_ref().clone();
    da.revision += 1;
    db.revision += 1;
    db.pads.iter_mut().find(|p| p.number == "1").unwrap().number = "3".into();
    let component = format!("component-{}", s.project().unwrap().board.footprints()[1].component);
    stage(&mut s, da, json!({"1":"1","2":"2"}));
    stage_for(&mut s, &component, db, json!({"1":"3","2":"2"}));
    let observer: Arc<Observe> = Arc::default();
    s.observe_edits(Some(observer.clone()));
    let prior = eco(&mut s, json!({}));
    let candidate = s.checked_eco_candidate().unwrap().clone();
    assert_eq!(prior["relation_detail"]["losses"], json!([{"id":row,"refs":[a,b],"last_resolved":before["relations"][0]["measure"]["witnesses"]}]));
    assert_eq!(prior["relation_detail"]["remaps"], json!([]));
    let mut lookup = Session::default();
    lookup.open_snapshot(candidate.clone());
    let replacement = pick(&mut lookup, 0, "pad.center", Some("2"));
    for choice in [json!({"kind":"correspond"}), json!({"kind":"repick","reference":replacement})] {
        let error = unchanged(&mut s, |s| run(s, update::PREVIEW_UPDATE, json!({"remap_choices":[{"relation":row,"pick":0,"choice":choice}]})));
        assert_eq!(error.details()["reason"], "lost_reference");
        assert_eq!(s.board_update_report().unwrap(), prior);
        assert!(Arc::ptr_eq(s.checked_eco_candidate().unwrap(), &candidate));
    }
    unchanged(&mut s, |s| run(s, update::APPLY_UPDATE, json!({"plan_token":prior["plan_token"],"accept_loss":[],"confirm_remaps":[]})));
    assert!(observer.0.lock().unwrap().is_empty());
    apply(&mut s, &prior);
    assert_eq!(observer.0.lock().unwrap().len(), 1);
    assert_eq!(transaction::certify(s.snapshot().unwrap()).unwrap(), prior["candidate_digest"].as_str().unwrap());
    assert_eq!(run(&mut s, LIST, json!({})).unwrap()["relations"][0]["status"], "lost");
}

#[test]
fn a_cat_ref_explicit_eco_choice_does_not_capture_another_rows_default_loss() {
    let mut s = fixture(true);
    let a = pick(&mut s, 0, "pad.edge", Some("1"));
    let b = pick(&mut s, 1, "pad.edge", Some("1"));
    let center = pick(&mut s, 0, "pad.center", Some("1"));
    let lost = add(&mut s, "gap", a.clone(), b);
    let selected = add(&mut s, "horizontal", origin(), center);
    let reference: Reference = serde_json::from_value(a).unwrap();
    let mut d = pin(&s);
    d.revision += 1;
    d.pads.iter_mut().find(|p| p.number == "1").unwrap().rotation = 90;
    stage(&mut s, d, json!({"1":"1","2":"2"}));
    let prior = eco(&mut s, json!({}));
    let candidate = s.checked_eco_candidate().unwrap();
    assert_ne!(catalog::correspondence(s.project().unwrap(), &catalog::Resolver::new(candidate), &reference).unwrap(), reference);
    assert_eq!(prior["relation_detail"]["losses"][0]["id"], lost);
    let reviewed = eco(&mut s, json!({"remap_choices":[{"relation":selected,"pick":1,"choice":{"kind":"correspond"}}]}));
    let detail = &reviewed["relation_detail"];
    assert_eq!(detail["losses"], prior["relation_detail"]["losses"]);
    assert_eq!(detail["remaps"].as_array().unwrap().len(), 1);
    assert_eq!(detail["remaps"][0]["relation"], selected);
    let remap = &detail["remaps"][0]["id"];
    unchanged(&mut s, |s| run(s, update::APPLY_UPDATE, json!({"plan_token":reviewed["plan_token"],"accept_loss":[remap],"confirm_remaps":[lost]})));
    apply(&mut s, &reviewed);
    let result = run(&mut s, LIST, json!({})).unwrap();
    let rows = result["relations"].as_array().unwrap();
    assert!(rows.iter().any(|r| r["id"] == lost && r["status"] == "lost"));
    assert!(rows.iter().any(|r| r["id"] == selected && r["status"] == "reference" && r["refs"][1] == detail["remaps"][0]["new"]));
}

#[test]
fn a_preview_edge_eco_discloses_changed_unreferenced_package_pose() {
    let mut s = fixture(true);
    let retained = pick(&mut s, 1, "fp.origin", None);
    add(&mut s, "horizontal", origin(), retained);
    let previous = s.project().unwrap().board.footprints().to_vec();
    let mut d = pin(&s);
    d.revision += 1;
    stage(&mut s, d, json!({"1":"1","2":"2"}));
    let observer: Arc<Observe> = Arc::default();
    s.observe_edits(Some(observer.clone()));
    let report = eco(&mut s, json!({}));
    let f = &previous[0];
    assert_eq!(
        report["relation_detail"],
        json!({"losses":[],"remaps":[],"poses":[{"fp":f.id,"x_nm":f.x_nm,"y_nm":f.y_nm,"rotation":f.rotation,"bottom":f.bottom}]})
    );
    let candidate = s.checked_eco_candidate().unwrap().clone();
    let next = candidate.board.footprints();
    assert_ne!(&next[0], f);
    assert_eq!(next[1..], previous[1..]);
    assert!(observer.0.lock().unwrap().is_empty());
    let expected = crate::persistence::encode_documents(candidate.clone()).unwrap();
    apply(&mut s, &report);
    assert_eq!(bytes(&s), expected);
    assert_eq!(transaction::certify(s.snapshot().unwrap()).unwrap(), report["candidate_digest"].as_str().unwrap());
    let events = observer.0.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert!(Arc::ptr_eq(&candidate, &events[0].project));
}

#[test]
fn a_cat_ref_vanished_flat_at_same_pose_stays_lost_when_restored() {
    let mut s = fixture(true);
    let original = pin(&s);
    let edge = pick(&mut s, 0, "pad.edge", Some("1"));
    let point = pick(&mut s, 1, "fp.origin", None);
    add(&mut s, "point_edge", edge.clone(), point);
    let witnesses = run(&mut s, LIST, json!({})).unwrap()["relations"][0]["measure"]["witnesses"].clone();
    let pose = s.project().unwrap().board.footprints()[0].clone();
    let mut round = original.clone();
    let pad = round.pads.iter_mut().find(|p| p.number == "1").unwrap();
    pad.shape = PadShape::RoundedRectangle;
    pad.height_nm = pad.width_nm;
    pad.corner_radius_nm = pad.width_nm / 2;
    round.revision += 1;
    let mapping = json!({"1":"1","2":"2"});
    stage(&mut s, round, mapping.clone());
    let p = eco(&mut s, json!({}));
    assert_eq!(p["relation_detail"]["losses"][0]["last_resolved"], witnesses);
    apply(&mut s, &p);
    stage(&mut s, original, mapping);
    let p = eco(&mut s, json!({}));
    assert_eq!(p["relation_detail"]["losses"], json!([]));
    assert_eq!(p["relation_detail"]["remaps"], json!([]));
    assert_eq!(
        p["relation_detail"]["poses"],
        json!([{"fp":pose.id,"x_nm":pose.x_nm,"y_nm":pose.y_nm,"rotation":pose.rotation,"bottom":pose.bottom}])
    );
    run(&mut s, update::APPLY_UPDATE, json!({"plan_token":p["plan_token"]})).unwrap();
    assert_eq!(s.project().unwrap().board.footprints()[0], pose);
    assert_eq!(pick(&mut s, 0, "pad.edge", Some("1")), edge);
    let rows = run(&mut s, LIST, json!({})).unwrap();
    assert_eq!(rows["relations"][0]["status"], "lost");
    assert_eq!(rows["relations"][0]["last_resolved"], witnesses);
    assert!(rows["relations"][0].get("measure").is_none());
}

#[test]
fn a_cat_ref_same_position_replacement_is_new_body_and_needs_repick() {
    let mut s = fixture(false);
    let f = s.project().unwrap().board.footprints()[0].clone();
    let old = pick(&mut s, 0, "fp.origin", None);
    let id = add(&mut s, "horizontal", origin(), old.clone());
    let args = json!({"items":{"footprints":[format!("footprint-{}", f.id)]}});
    let prior = reviewed(&mut s, clipboard::DELETE, args.clone());
    let live = reviewed(&mut s, clipboard::DELETE, args);
    unchanged(&mut s, |s| run(s, APPLY, decision(&prior)));
    run(&mut s, APPLY, decision(&live)).unwrap();
    run(&mut s, crate::PLACE_ASSIGNED_FOOTPRINT, json!({"component":format!("component-{}",f.component),"x_nm":f.x_nm,"y_nm":f.y_nm})).unwrap();
    let new = pick(&mut s, 2, "fp.origin", None);
    assert_ne!(old["fp"], new["fp"]);
    assert_eq!(run(&mut s, LIST, json!({})).unwrap()["relations"][0]["status"], "lost");
    unchanged(&mut s, |s| run(s, EDIT, json!({"id":id,"edit":{"repick":{"kind":"horizontal","refs":[origin(),old]}}})));
    run(&mut s, EDIT, json!({"id":id,"edit":{"repick":{"kind":"horizontal","refs":[origin(),new]}}})).unwrap();
    let result = run(&mut s, LIST, json!({})).unwrap();
    assert_eq!(result["relations"][0]["status"], "reference");
    assert_eq!(result["relations"][0]["refs"][1], new);
    assert_eq!(s.project().unwrap().board.footprints()[2].x_nm, f.x_nm);
}
