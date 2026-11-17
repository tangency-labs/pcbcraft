use super::*;
use crate::{
    INITIALIZE_BOARD_SETUP, PLACE_RESISTOR, SET_FOOTPRINT_TRANSFORM, SET_PLACEMENT_ORIGIN, SET_RULES,
    board::{
        assignment::{ASSIGN_FOOTPRINT, PLACE_ASSIGNED_FOOTPRINT},
        definition::basic_components,
        track::tests::view,
    },
    persistence::{decode_documents, encode_documents, test_board_path},
};

const MM: i64 = 1_000_000;
const LIBRARY: &str = "basic-components";

/// Two 0402 resistors and one 0402 capacitor on a two-layer board, placed through the shared
/// assignment and placement commands. Returns the session and the footprint ids in that order.
fn fixture() -> (Session, [String; 3]) {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    for index in 0..3 {
        s.apply(PLACE_RESISTOR, &json!({"x_nm":index * 10 * MM,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    }
    let components: Vec<_> = s.project().unwrap().symbols.iter().map(|symbol| symbol.component).collect();
    for (index, (component, package)) in components.iter().zip(["resistor-0402", "resistor-0402", "capacitor-0402"]).enumerate() {
        let definition = basic_components(package).unwrap();
        let component = format!("component-{component}");
        s.apply(
            ASSIGN_FOOTPRINT,
            &json!({"component":component,"library":definition.library,"definition":definition.id,
                "definition_revision":definition.revision,"mapping":{"1":"1","2":"2"},"expected_revision":s.revision()}),
        )
        .unwrap();
        s.apply(
            PLACE_ASSIGNED_FOOTPRINT,
            &json!({"component":component,"x_nm":index as i64 * 8 * MM,"y_nm":2 * MM,"expected_revision":s.revision()}),
        )
        .unwrap();
    }
    s.apply(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"resolve_existing_footprints":true,"expected_revision":s.revision()})).unwrap();
    let ids: Vec<_> = view(&s).footprints().map(|f| (f.id, f.definition.to_owned())).collect();
    let [(first, a), (second, b), (third, c)] = &ids[..] else { panic!("three footprints") };
    assert_eq!([a.as_str(), b, c], ["resistor-0402", "resistor-0402", "capacitor-0402"]);
    (s, [first.clone(), second.clone(), third.clone()])
}

/// Move a footprint through the shared transform command.
fn place(s: &mut Session, footprint: &str, at: Point, rotation: u16, side: &str) {
    let params = json!({"footprint":footprint,"x_nm":at.x_nm,"y_nm":at.y_nm,"rotation":rotation,"side":side,"expected_revision":s.revision()});
    s.apply(SET_FOOTPRINT_TRANSFORM, &params).unwrap();
}

/// Supply explicit content revision and authored provenance for command-built fixtures.
fn content(mut value: Value, revision: u32) -> Value {
    if let Some(object) = value.as_object_mut() {
        object.insert("revision".into(), json!(revision));
        object.insert("provenance".into(), json!({"kind":"authored"}));
        if object.get("corrections").is_some_and(|v| v.as_array().is_some_and(Vec::is_empty)) {
            object.remove("corrections");
        }
    }
    value
}

/// Pin through the shared project command, without adopting on any board.
fn pin(s: &mut Session, profile: Option<Uuid>, value: Value) -> Result<u64, CommandError> {
    let mut params = json!({"expected_revision":s.revision(),"content":value});
    if let Some(id) = profile {
        params["profile"] = json!(id);
    }
    s.apply(SET_ASSEMBLY_PROFILE, &params)
}

/// Preview and explicitly accept the reviewed board adoption through shared commands.
fn adopt(s: &mut Session, id: Uuid, revision: Option<u32>) -> Result<u64, CommandError> {
    let board = s.project().unwrap().board_target().unwrap().board;
    let mut params = json!({"board":board,"expected_revision":s.revision(),"profile":id,"revision":revision});
    let preview = s.execute(PREVIEW_ASSEMBLY_PROFILE, &params)?;
    params["acceptance"] = preview["acceptance"].clone();
    s.apply(ADOPT_ASSEMBLY_PROFILE, &params)
}

/// Pin a fresh content revision and explicitly adopt it, for the placement-transform fixtures.
fn set(s: &mut Session, profile: Option<Uuid>, value: Value) -> Result<u64, CommandError> {
    let id = profile.map_or_else(crate::fresh_id, Ok)?;
    let revision = s.project().unwrap().assembly_profiles.heads.iter().find(|h| h.profile == id).map_or(1, |h| h.revision + 1);
    pin(s, Some(id), content(value, revision))?;
    adopt(s, id, Some(revision))
}

/// Panics unless `result` is an invalid-parameters refusal whose message names `reason`.
fn invalid_because<T: std::fmt::Debug>(result: Result<T, CommandError>, reason: &str) {
    assert!(matches!(&result, Err(CommandError::InvalidParams(message)) if message.contains(reason)), "expected {reason:?}, got {result:?}");
}

/// Panics unless `result` is a refusal whose message names `reason`.
fn refused_because<T: std::fmt::Debug>(result: Result<T, String>, reason: &str) {
    assert!(matches!(&result, Err(message) if message.contains(reason)), "expected {reason:?}, got {result:?}");
}

/// Resolve the stable identity of a pinned profile by its fixture display name.
fn profile(s: &Session, name: &str) -> Uuid {
    s.project().unwrap().assembly_profiles.revisions.iter().find(|p| p.content.name == name).unwrap().id
}

/// The fixture with the first resistor on top at 30 degrees, the second on the bottom at 30
/// degrees and the capacitor on the bottom at 90 degrees, measured from board (10 mm, 30 mm).
fn placed() -> (Session, [String; 3]) {
    let (mut s, ids) = fixture();
    place(&mut s, &ids[0], Point { x_nm: 40 * MM, y_nm: 45 * MM }, 30, "front");
    place(&mut s, &ids[1], Point { x_nm: 40 * MM, y_nm: 25 * MM }, 30, "back");
    place(&mut s, &ids[2], Point { x_nm: 20 * MM, y_nm: 25 * MM }, 90, "back");
    let board = s.project().unwrap().board_target().unwrap().board;
    s.apply(SET_PLACEMENT_ORIGIN, &json!({"board":board,"expected_revision":s.revision(),"origin":{"kind":"point","x_nm":10 * MM,"y_nm":30 * MM}}))
        .unwrap();
    (s, ids)
}

/// A +180 degree offset for the 0402 resistor in one profile turns that profile's resistor rows by
/// 180 degrees more, on both sides and under both bottom-side conventions. The offset affects only
/// that profile's resistor corrections; generic rows and other profiles are unchanged. The capacitor
/// receives no correction but follows the selected bottom-side convention.
#[test]
fn half_turn_offset_changes_only_that_profile() {
    let (mut s, [top, bottom, capacitor]) = placed();
    let rows = |s: &Session, profile| [&top, &bottom, &capacitor].map(|id| view(s).placement_row(id, profile).unwrap());
    let generic = rows(&s, None);
    // Top resistor: X = 40 - 10 = 30 mm, Y up = 30 - 45 = -15 mm, rotation 30.
    assert_eq!(generic[0], Placed { at: Point { x_nm: 30 * MM, y_nm: -15 * MM }, rotation: 30 });
    // Bottom resistor seen from the top (IPC-2581, 3.3.2): X = 30 mm, Y = 5 mm, rotation 30.
    assert_eq!(generic[1], Placed { at: Point { x_nm: 30 * MM, y_nm: 5 * MM }, rotation: 30 });
    // Bottom capacitor: X = 20 - 10 = 10 mm, Y = 5 mm, rotation 90.
    assert_eq!(generic[2], Placed { at: Point { x_nm: 10 * MM, y_nm: 5 * MM }, rotation: 90 });

    let turned = json!({"library":LIBRARY,"definition":"resistor-0402","rotation":180});
    set(&mut s, None, json!({"name":"Turned","corrections":[turned]})).unwrap();
    set(&mut s, None, json!({"name":"Turned from below","bottom_side":"viewed_from_bottom","corrections":[turned]})).unwrap();
    set(&mut s, None, json!({"name":"Other","corrections":[{"library":LIBRARY,"definition":"capacitor-0402","rotation":90}]})).unwrap();
    assert_eq!(rows(&s, None), generic, "generic rows ignore every profile");

    let turned = rows(&s, Some(profile(&s, "Turned")));
    // 30 + 180 = 210 on top. Seen from the top the bottom part's offset subtracts, 30 - 180 =
    // -150, which is also 210 mod 360. Positions stay at the footprint origin.
    assert_eq!(turned[0], Placed { rotation: 210, ..generic[0] });
    assert_eq!(turned[1], Placed { rotation: 210, ..generic[1] });
    assert_eq!(turned[2], generic[2], "the capacitor has no offset in this profile");

    let below = rows(&s, Some(profile(&s, "Turned from below")));
    assert_eq!(below[0], Placed { rotation: 210, ..generic[0] });
    // Seen from below the bottom resistor is at 360 - 30 = 330 with X = -30 mm; 330 + 180 = 150.
    assert_eq!(below[1], Placed { at: Point { x_nm: -30 * MM, y_nm: 5 * MM }, rotation: 150 });
    // The capacitor follows the convention only: X = -10 mm, rotation 360 - 90 = 270.
    assert_eq!(below[2], Placed { at: Point { x_nm: -10 * MM, y_nm: 5 * MM }, rotation: 270 });

    let other = rows(&s, Some(profile(&s, "Other")));
    assert_eq!([other[0], other[1]], [generic[0], generic[1]], "the resistor offset belongs to another profile");
    // The capacitor's +90 subtracts seen from the top: 90 - 90 = 0.
    assert_eq!(other[2], Placed { rotation: 0, ..generic[2] });
}

/// A centroid offset is a point of the library footprint: the profile row lands exactly where
/// the board puts that point. Checked against pad 1's real position on both sides, under both
/// conventions, at every quarter turn, and off both axes at 90 degrees.
#[test]
fn centroid_offset_follows_the_footprint_transform() {
    let (mut s, [id, _, _]) = placed();
    let pad_1 = basic_components("resistor-0402").unwrap().pads.iter().find(|pad| pad.number == "1").unwrap().center;
    let package = json!([{"library":LIBRARY,"definition":"resistor-0402","centroid":pad_1}]);
    set(&mut s, None, json!({"name":"Top view","corrections":package})).unwrap();
    set(&mut s, None, json!({"name":"Bottom view","bottom_side":"viewed_from_bottom","corrections":package})).unwrap();
    let frame = view(&s).placement_origin().unwrap().frame();
    for side in ["front", "back"] {
        for rotation in [0, 90, 180, 270] {
            place(&mut s, &id, Point { x_nm: 40 * MM, y_nm: 25 * MM }, rotation, side);
            let footprint = view(&s).footprints().find(|f| f.id == id).unwrap();
            let pad = frame.to_output(footprint.pads.iter().find(|pad| pad.number == "1").unwrap().center);
            let generic = view(&s).placement_row(&id, None).unwrap();
            let top = view(&s).placement_row(&id, Some(profile(&s, "Top view"))).unwrap();
            assert_eq!(top, Placed { at: pad, rotation: generic.rotation }, "{side} {rotation}");
            let bottom = view(&s).placement_row(&id, Some(profile(&s, "Bottom view"))).unwrap();
            let x = if side == "back" { -pad.x_nm } else { pad.x_nm };
            assert_eq!(bottom.at, Point { x_nm: x, y_nm: pad.y_nm }, "{side} {rotation}");
        }
    }
    // Off both axes: local (0.3 mm, -0.2 mm), Y down, on a top part at 90 degrees around board
    // (40 mm, 25 mm). A counterclockwise quarter turn with Y down maps local (x, y) to (y, -x),
    // so the point is at board (39.8 mm, 24.7 mm): output X 39.8 - 10 = 29.8 mm, Y up 30 - 24.7
    // = 5.3 mm.
    let centroid = Point { x_nm: 300_000, y_nm: -200_000 };
    let top_view = profile(&s, "Top view");
    set(&mut s, Some(top_view), json!({"name":"Top view","corrections":[{"library":LIBRARY,"definition":"resistor-0402","centroid":centroid}]}))
        .unwrap();
    place(&mut s, &id, Point { x_nm: 40 * MM, y_nm: 25 * MM }, 90, "front");
    let row = view(&s).placement_row(&id, Some(top_view)).unwrap();
    assert_eq!(row, Placed { at: Point { x_nm: 29_800_000, y_nm: 5_300_000 }, rotation: 90 });
}

/// Rows refuse footprints and profiles that are not on the board; a profile without offsets
/// under the default convention writes the generic row.
#[test]
fn rows_name_what_is_missing() {
    let (mut s, [id, _, _]) = placed();
    assert!(view(&s).placement_row("footprint-999", None).unwrap_err().contains("footprint-999"));
    let missing = Uuid::from_u128(7);
    assert!(view(&s).placement_row(&id, Some(missing)).unwrap_err().contains(&missing.to_string()));
    set(&mut s, None, json!({"name":"Empty"})).unwrap();
    assert_eq!(view(&s).placement_row(&id, Some(profile(&s, "Empty"))).unwrap(), view(&s).placement_row(&id, None).unwrap());
}

/// AssemblyProfiles change no fabrication output, legacy markers never match an offset, and a row fails
/// while an outline-attached origin has no outline to resolve against.
#[test]
fn profiles_leave_fabrication_and_legacy_markers_alone() {
    let mut s = crate::board::track::tests::routed_fixture();
    crate::board::drc::tests::outline(&mut s);
    let [front, back] = crate::board::track::tests::copper_layers(&s)[..] else { panic!("two copper layers") };
    let y = 20 * MM;
    let route = [Point { x_nm: 5 * MM, y_nm: y }, Point { x_nm: 7 * MM, y_nm: y }, Point { x_nm: 9 * MM, y_nm: y }];
    crate::board::track::tests::route_with_vias(&mut s, front, &route, &[(1, back)]).unwrap();
    let before = s.execute(crate::EXPORT_FABRICATION, &json!({})).unwrap()["fabrication"].clone();
    let project = s.project().unwrap();
    let corrections: Vec<_> = project
        .board
        .footprints()
        .iter()
        .map(|f| (f.library.clone().unwrap_or_else(|| LIBRARY.into()), f.definition.clone()))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(|(library, definition)| json!({"library":library,"definition":definition,"rotation":90,"centroid":{"x_nm":MM,"y_nm":MM}}))
        .collect();
    set(&mut s, None, json!({"name":"Everything","bottom_side":"viewed_from_bottom","corrections":corrections})).unwrap();
    assert_eq!(s.execute(crate::EXPORT_FABRICATION, &json!({})).unwrap()["fabrication"], before);
    // Format 22 carries the tracks, vias, outline, setup and accepted baseline beside the profile.
    let documents = encode_documents(s.snapshot().unwrap()).unwrap();
    let saved: Value = serde_json::from_slice(&documents[&test_board_path(&documents)]).unwrap();
    assert_eq!(saved["format"], "pcbcraft-board/22");
    assert!(saved["tracks"].is_array() && saved["vias"].is_array() && saved["baseline"].is_object());
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);

    let marker = Footprint::new(1, 1, 1, Point { x_nm: 0, y_nm: 0 });
    let definition = crate::board::FOOTPRINT_DEFINITION;
    let profile = AssemblyProfile {
        format: FORMAT.into(),
        id: Uuid::from_u128(1),
        content: ProfileContent {
            revision: 1,
            name: "Marker".into(),
            provenance: Provenance::Authored {},
            vendor: None,
            bottom_side: BottomSide::ViewedFromTop,
            corrections: Arc::new(vec![Arc::new(PackageCorrection {
                library: LIBRARY.into(),
                definition: definition.into(),
                rotation: 90,
                centroid: None,
            })]),
        },
    };
    assert!(profile.validate().is_ok());
    assert!(profile.correction_for(&marker).is_none(), "a legacy marker has no library identity");

    let (mut s, [id, _, _]) = fixture();
    let board = s.project().unwrap().board_target().unwrap().board;
    s.apply(SET_PLACEMENT_ORIGIN, &json!({"board":board,"expected_revision":s.revision(),"origin":{"kind":"outline_lower_left"}})).unwrap();
    // No outline to measure from.
    refused_because(view(&s).placement_row(&id, None), "Draw a Board Outline");
}

/// Preview shares one outline frame across multiple old/proposed rows and agrees with single-row output.
#[test]
fn outline_preview_reuses_the_frame_for_all_placements() {
    let (mut s, ids) = fixture();
    let board = s.project().unwrap().board_target().unwrap().board;
    s.apply(SET_PLACEMENT_ORIGIN, &json!({"board":board,"expected_revision":s.revision(),"origin":{"kind":"outline_lower_left"}})).unwrap();
    let id = Uuid::from_u128(94);
    pin(
        &mut s,
        Some(id),
        content(
            json!({"name":"Outline","corrections":[{"library":LIBRARY,"definition":"resistor-0402",
        "rotation":180,"centroid":{"x_nm":MM,"y_nm":2*MM}}]}),
            1,
        ),
    )
    .unwrap();
    let before = s.snapshot().unwrap();
    invalid_because(
        s.execute(PREVIEW_ASSEMBLY_PROFILE, &json!({"board":board,"profile":id,"revision":1,"expected_revision":s.revision()})),
        "Draw a Board Outline",
    );
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    crate::board::drc::tests::outline(&mut s);
    let generic: Vec<_> = ids.iter().map(|footprint| view(&s).placement_row(footprint, None).unwrap()).collect();
    let before = s.snapshot().unwrap();
    let review = s.execute(PREVIEW_ASSEMBLY_PROFILE, &json!({"board":board,"profile":id,"revision":1,"expected_revision":s.revision()})).unwrap();
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    assert_eq!(
        review["affected_rows"],
        json!([
            {"footprint":ids[0],"before":{"at":{"x_nm":5*MM,"y_nm":33*MM},"rotation":0},"after":{"at":{"x_nm":6*MM,"y_nm":31*MM},"rotation":180}},
            {"footprint":ids[1],"before":{"at":{"x_nm":13*MM,"y_nm":33*MM},"rotation":0},"after":{"at":{"x_nm":14*MM,"y_nm":31*MM},"rotation":180}}
        ])
    );
    adopt(&mut s, id, Some(1)).unwrap();
    for index in 0..2 {
        assert_eq!(serde_json::to_value(view(&s).placement_row(&ids[index], Some(id)).unwrap()).unwrap(), review["affected_rows"][index]["after"]);
    }
    assert_eq!(view(&s).placement_row(&ids[2], Some(id)).unwrap(), generic[2]);
    assert_eq!(ids.iter().map(|footprint| view(&s).placement_row(footprint, None).unwrap()).collect::<Vec<_>>(), generic);
}

/// An empty board needs no resolved frame to preview or adopt a profile; no rows are fabricated.
#[test]
fn empty_outline_preview_does_not_resolve_an_unused_origin() {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    let board = s.project().unwrap().board_target().unwrap().board;
    s.apply(SET_PLACEMENT_ORIGIN, &json!({"board":board,"expected_revision":s.revision(),"origin":{"kind":"outline_lower_left"}})).unwrap();
    let id = Uuid::from_u128(95);
    pin(&mut s, Some(id), content(json!({"name":"Empty"}), 1)).unwrap();
    let before = s.snapshot().unwrap();
    let review = s.execute(PREVIEW_ASSEMBLY_PROFILE, &json!({"board":board,"profile":id,"revision":1,"expected_revision":s.revision()})).unwrap();
    assert_eq!(review["affected_rows"], json!([]));
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    adopt(&mut s, id, Some(1)).unwrap();
    assert_eq!(view(&s).assembly_profiles(), &[Adoption { profile: id, revision: 1 }]);
    refused_because(view(&s).placement_row("footprint-999", Some(id)), "No footprint footprint-999");
}

/// Pinning and adoption are separate undoable transactions; repeats are no-ops and revisions are immutable.
#[test]
fn profiles_are_edited_in_one_undo_step() {
    let (mut s, _) = fixture();
    let original = s.snapshot().unwrap();
    let id = Uuid::from_u128(70);
    let c = content(json!({"name":"A","corrections":[{"library":LIBRARY,"definition":"resistor-0402","rotation":180}]}), 1);
    pin(&mut s, Some(id), c.clone()).unwrap();
    let pinned = s.snapshot().unwrap();
    assert!(view(&s).assembly_profiles().is_empty());
    let revision = s.revision();
    pin(&mut s, Some(id), c.clone()).unwrap();
    assert_eq!(s.revision(), revision);
    adopt(&mut s, id, Some(1)).unwrap();
    let adopted = s.snapshot().unwrap();
    assert_eq!(view(&s).assembly_profiles(), &[Adoption { profile: id, revision: 1 }]);
    let inspected = s.inspect();
    assert_eq!(inspected["project"]["boards"][0]["assembly_profiles"], json!([{"profile":id,"revision":1}]));
    let mut canonical = c.clone();
    canonical["bottom_side"] = json!("viewed_from_top");
    assert_eq!(inspected["project"]["assembly_profiles"]["revisions"][0]["content"], canonical);
    s.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&pinned, &s.snapshot().unwrap());
    s.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&original, &s.snapshot().unwrap());
    s.apply("edit.redo", &Value::Null).unwrap();
    s.apply("edit.redo", &Value::Null).unwrap();
    crate::assert_same_content(&adopted, &s.snapshot().unwrap());
    let mut conflict = c;
    conflict["name"] = json!("Different");
    invalid_because(pin(&mut s, Some(id), conflict), "content conflict");
    assert!(Arc::ptr_eq(&adopted, &s.snapshot().unwrap()));
}

/// Every invalid content edge is refused for its profile reason; a stale revision cannot mask it.
#[test]
fn bad_requests_change_nothing() {
    let (mut s, _) = fixture();
    let before = s.snapshot().unwrap();
    let package = |extra: Value| {
        let mut entry = json!({"library":LIBRARY,"definition":"resistor-0402"});
        entry.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        json!({"name":"X","corrections":[entry]})
    };
    let edge = MAX_OFFSET_NM + 1;
    for (value, reason) in [
        (json!({}), "missing field `name`"),
        (json!({"name":""}), "metadata needs"),
        (json!({"name":" padded"}), "metadata needs"),
        (json!({"name":"padded\u{feff}"}), "metadata needs"),
        (json!({"name":"tab\tinside"}), "metadata needs"),
        (json!({"name":"n".repeat(MAX_NAME_CHARS+1)}), "metadata needs"),
        (json!({"name":"X","bottom_side":"mirrored"}), "unknown variant `mirrored`"),
        (json!({"name":"X","bottom_side":null}), "invalid type: null"),
        (json!({"name":"X","corrections":null}), "invalid type: null"),
        (json!({"name":"X","extra":1}), "unknown field `extra`"),
        (json!({"name":7}), "expected a string"),
        (json!("X"), "expected struct ProfileContent"),
        (package(json!({})), "changes nothing"),
        (package(json!({"rotation":0})), "changes nothing"),
        (package(json!({"rotation":360})), "0 through 359"),
        (package(json!({"rotation":-1})), "expected u16"),
        (package(json!({"rotation":90.5})), "expected u16"),
        (package(json!({"centroid":{"x_nm":0,"y_nm":0}})), "centroid correction must be nonzero"),
        (package(json!({"centroid":{"x_nm":edge,"y_nm":0}})), "within 100 mm"),
        (package(json!({"centroid":{"x_nm":0,"y_nm":-edge}})), "within 100 mm"),
        (package(json!({"centroid":{"x_nm":1}})), "missing field `y_nm`"),
        (package(json!({"rotation":90,"centroid":null})), "invalid type: null"),
        (package(json!({"rotation":90,"revision":1})), "unknown field `revision`"),
        (package(json!({"rotation":90,"library":"Basic"})), "valid library and footprint definition id"),
        (package(json!({"rotation":90,"definition":""})), "valid library and footprint definition id"),
        (
            json!({"name":"X","corrections":[{"library":LIBRARY,"definition":"a","rotation":1},{"library":LIBRARY,"definition":"a","rotation":2}]}),
            "unique and in library and definition order",
        ),
    ] {
        let profile = content(value.clone(), 1);
        refused_because(parse(&profile), reason);
        invalid_because(pin(&mut s, None, profile), "immutable profile content");
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()), "{value}");
    }
    let good = content(json!({"name":"Good"}), 1);
    invalid_because(pin(&mut s, Some(Uuid::nil()), good.clone()), "non-nil UUID");
    invalid_because(pin(&mut s, Some(Uuid::from_u128(99)), Value::Null), "existing assembly profile");
    let params = json!({"expected_revision":s.revision()-1,"content":good});
    let stale = s.apply(SET_ASSEMBLY_PROFILE, &params);
    assert!(matches!(&stale,Err(CommandError::Unavailable(message)) if message.contains("project changed")), "{stale:?}");
    invalid_because(s.apply(SET_ASSEMBLY_PROFILE, &json!({"expected_revision":s.revision()})), "needs `content`");
    let board = before.board_target().unwrap().board;
    invalid_because(
        s.execute(PREVIEW_ASSEMBLY_PROFILE, &json!({"expected_revision":s.revision(),"profile":Uuid::from_u128(8),"revision":1})),
        "needs `board`",
    );
    invalid_because(
        s.execute(
            PREVIEW_ASSEMBLY_PROFILE,
            &json!({"board":Uuid::from_u128(5),"expected_revision":s.revision(),"profile":Uuid::from_u128(8),"revision":1}),
        ),
        "unknown board ID",
    );
    for revision in [json!(0), json!(-1), json!(1.5), json!(u32::MAX as u64 + 1)] {
        invalid_because(
            s.execute(
                PREVIEW_ASSEMBLY_PROFILE,
                &json!({"board":board,"expected_revision":s.revision(),"profile":Uuid::from_u128(8),"revision":revision}),
            ),
            "positive content revision",
        );
    }
    set(
        &mut s,
        None,
        json!({"name":"n".repeat(MAX_NAME_CHARS),"corrections":[
            {"library":LIBRARY,"definition":"capacitor-0402","rotation":1},
            {"library":LIBRARY,"definition":"led-0402","centroid":{"x_nm":0,"y_nm":1}},
            {"library":LIBRARY,"definition":"resistor-0402","rotation":359,"centroid":{"x_nm":MAX_OFFSET_NM,"y_nm":-MAX_OFFSET_NM}}
        ]}),
    )
    .unwrap();
}

/// Last valid profile and correction counts pass; one more or duplicate catalog identities fail.
#[test]
fn profile_and_project_limits() {
    let corrections = |count: usize| (0..count).map(|n| json!({"library":LIBRARY,"definition":format!("p{n:05}"),"rotation":90})).collect::<Vec<_>>();
    assert!(valid_profile(&content(json!({"name":"Full","corrections":corrections(MAX_CORRECTIONS)}), 1)));
    assert!(!valid_profile(&content(json!({"name":"Full","corrections":corrections(MAX_CORRECTIONS+1)}), 1)));
    let (mut s, _) = fixture();
    for n in 0..MAX_PROFILES {
        set(&mut s, Some(Uuid::from_u128(n as u128 + 1)), json!({"name":format!("P{n}")})).unwrap();
    }
    let full = s.snapshot().unwrap();
    invalid_because(pin(&mut s, Some(Uuid::from_u128(1000)), content(json!({"name":"More"}), 1)), "at most");
    assert!(Arc::ptr_eq(&full, &s.snapshot().unwrap()));
    let mut catalog = full.assembly_profiles.as_ref().clone();
    catalog.heads[1].profile = catalog.heads[0].profile;
    refused_because(catalog.validate(&full.board.assembly_profiles), "unique and sorted");
}

/// Corrections across pinned revisions are counted together, on the command path and on load.
#[test]
fn project_correction_total_is_bounded() {
    let corrections = |count: usize| (0..count).map(|n| json!({"library":LIBRARY,"definition":format!("p{n:05}"),"rotation":90})).collect::<Vec<_>>();
    let (mut s, _) = fixture();
    set(&mut s, Some(Uuid::from_u128(1)), json!({"name":"First","corrections":corrections(MAX_CORRECTIONS/2)})).unwrap();
    set(&mut s, Some(Uuid::from_u128(2)), json!({"name":"Second","corrections":corrections(MAX_CORRECTIONS/2-1)})).unwrap();
    set(&mut s, Some(Uuid::from_u128(3)), json!({"name":"Third","corrections":corrections(1)})).unwrap();
    let full = s.snapshot().unwrap();
    invalid_because(pin(&mut s, Some(Uuid::from_u128(4)), content(json!({"name":"Fourth","corrections":corrections(1)}), 1)), "corrections together");
    invalid_because(pin(&mut s, Some(Uuid::from_u128(3)), content(json!({"name":"Third","corrections":corrections(2)}), 2)), "corrections together");
    assert!(Arc::ptr_eq(&full, &s.snapshot().unwrap()));
    let docs = encode_documents(full).unwrap();
    assert!(decode_documents(&docs).is_ok());
    let mut over = docs.clone();
    let mut saved: Value = serde_json::from_slice(&over["project.json"]).unwrap();
    saved["assembly_profiles"]["revisions"][2]["content"]["corrections"]
        .as_array_mut()
        .unwrap()
        .push(json!({"library":LIBRARY,"definition":"z-extra","rotation":90}));
    over.insert("project.json".into(), serde_json::to_vec(&saved).unwrap());
    refused_because(decode_documents(&over), "corrections together");
}

/// The complete largest bounded catalog, including metadata and revision watermarks, is below 2 MiB.
#[test]
fn largest_allowed_profile_catalog_serializes_under_two_mebibytes() {
    let mut catalog = Catalog::default();
    for h in 0..MAX_PROFILES {
        let id = Uuid::from_u128(h as u128 + 1);
        catalog.heads.push(ProfileHead { profile: id, revision: u32::MAX, removed: false });
        for revision in [u32::MAX - 1, u32::MAX] {
            catalog.revisions.push(Arc::new(AssemblyProfile {
                format: FORMAT.into(),
                id,
                content: ProfileContent {
                    revision,
                    name: format!("{h:02}{}", "\u{10ffff}".repeat(MAX_NAME_CHARS - 2)),
                    vendor: Some("\u{10ffff}".repeat(MAX_NAME_CHARS)),
                    provenance: Provenance::Imported { source: "\u{10ffff}".repeat(MAX_NAME_CHARS) },
                    bottom_side: BottomSide::ViewedFromBottom,
                    corrections: Arc::new(
                        (0..MAX_CORRECTIONS / MAX_REVISIONS)
                            .map(|n| {
                                Arc::new(PackageCorrection {
                                    library: format!("{h:05}{}", "x".repeat(59)),
                                    definition: format!("{n:05}{}", "x".repeat(59)),
                                    rotation: 359,
                                    centroid: Some(Point { x_nm: -MAX_OFFSET_NM, y_nm: -MAX_OFFSET_NM }),
                                })
                            })
                            .collect(),
                    ),
                },
            }));
        }
    }
    let adoptions: Vec<_> = catalog.heads.iter().map(|head| Adoption { profile: head.profile, revision: head.revision - 1 }).collect();
    catalog.validate(&adoptions).unwrap();
    assert_eq!(catalog.revisions.len(), MAX_REVISIONS);
    assert_eq!(catalog.revisions.iter().map(|r| r.content.corrections.len()).sum::<usize>(), MAX_CORRECTIONS);
    let bytes = serde_json::to_vec_pretty(&catalog).unwrap().len();
    assert!(bytes < crate::persistence::MAX_DOCUMENT_BYTES / 8, "{bytes}");
    let mut extra = (*catalog.revisions[0]).clone();
    extra.content.revision -= 1;
    catalog.revisions.insert(0, Arc::new(extra));
    refused_because(catalog.validate(&adoptions), "128 revisions");
}

/// Project/36 pins profiles and board/22 adopts them; malformed records and dangling adoptions fail.
#[test]
fn profile_format_round_trips_and_undo_restores_previous_bytes() {
    let (mut s, _) = placed();
    let original = encode_documents(s.snapshot().unwrap()).unwrap();
    let board = s.project().unwrap().board_target().unwrap().board;
    s.apply(SET_RULES, &json!({"board":board,"expected_revision":s.revision(),"rules":crate::board::rules::Values::default()})).unwrap();
    let before_profiles = s.snapshot().unwrap();
    set(
        &mut s,
        Some(Uuid::from_u128(1)),
        json!({"name":"Reel","bottom_side":"viewed_from_bottom","corrections":[
        {"library":LIBRARY,"definition":"resistor-0402","rotation":180,"centroid":{"x_nm":1,"y_nm":-2}},
        {"library":LIBRARY,"definition":"sot23","rotation":270}]}),
    )
    .unwrap();
    set(&mut s, Some(Uuid::from_u128(2)), json!({"name":"Plain"})).unwrap();
    let docs = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&docs).unwrap()).unwrap(), docs);
    let path = test_board_path(&docs);
    let saved: Value = serde_json::from_slice(&docs[&path]).unwrap();
    assert_eq!(saved["format"], "pcbcraft-board/22");
    assert!(saved["rules"].is_object() && saved["placement_origin"].is_object());
    let project: Value = serde_json::from_slice(&docs["project.json"]).unwrap();
    assert_eq!(project["format"], "pcbcraft-project/36");
    assert_eq!(project["assembly_profiles"]["revisions"][0]["content"]["corrections"][0]["centroid"], json!({"x_nm":1,"y_nm":-2}));
    assert!(project["assembly_profiles"]["revisions"][1]["content"].get("corrections").is_none());
    let text = String::from_utf8(docs["project.json"].clone()).unwrap();
    let refused = |from: &str, to: &str, reason: &str| {
        assert!(text.contains(from), "{from}");
        let mut altered = docs.clone();
        altered.insert("project.json".into(), text.replacen(from, to, 1).into_bytes());
        refused_because(decode_documents(&altered), reason);
    };
    refused("pcbcraft-project/36", "pcbcraft-project/23", "assembly profiles require");
    refused("pcbcraft-assembly-profile/1", "pcbcraft-assembly-profile/2", "Unsupported assembly profile");
    refused("\"rotation\":270", "\"rotation\":360", "rotation correction");
    refused("\"rotation\":270", "\"rotation\":0", "zero rotation");
    refused("\"rotation\":180,", "\"rotation\":0,", "zero rotation");
    refused("\"rotation\":180,", "\"rotation\":null,", "null");
    refused("\"rotation\":270", "\"rotation\":270,\"extra\":1", "unknown field");
    refused("\"centroid\":{\"x_nm\":1,\"y_nm\":-2}", "\"centroid\":null", "null");
    refused("\"definition\":\"sot23\"", "\"definition\":\"capacitor-0402\"", "in library and definition order");
    refused("\"name\":\"Plain\"", "\"name\":\"Plain\",\"note\":1", "unknown field");
    refused("\"name\":\"Plain\"", "\"name\":\"Plain\",\"corrections\":[]", "empty corrections");
    let mut missing = docs.clone();
    let mut bad: Value = serde_json::from_slice(&missing[&path]).unwrap();
    bad["assembly_profiles"][0]["revision"] = json!(99);
    missing.insert(path.clone(), serde_json::to_vec(&bad).unwrap());
    refused_because(decode_documents(&missing), "reference pinned content revisions");
    for _ in 0..4 {
        s.apply("edit.undo", &Value::Null).unwrap();
    }
    crate::assert_same_content(&before_profiles, &s.snapshot().unwrap());
    s.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), original);
}

/// A profile adoption without an origin or rules survives an offline save and reopen.
#[test]
fn profile_without_origin_round_trips() {
    let (mut s, _) = fixture();
    set(&mut s, None, json!({"name":"Only"})).unwrap();
    let docs = encode_documents(s.snapshot().unwrap()).unwrap();
    let saved: Value = serde_json::from_slice(&docs[&test_board_path(&docs)]).unwrap();
    assert_eq!(saved["format"], "pcbcraft-board/22");
    assert!(saved.get("placement_origin").is_none() && saved.get("rules").is_none());
    assert_eq!(encode_documents(decode_documents(&docs).unwrap()).unwrap(), docs);
}

/// A qualified stack, placement origin, and adopted profile keep their values through Save/Open.
/// Clearing or undoing the adoption restores the same stack-bearing board bytes.
#[test]
fn stack_format_carries_an_assembly_profile_adoption() {
    let (mut s, [footprint, _, _]) = placed();
    let target = s.project().unwrap().board_target().unwrap();
    let mut setup = s.inspect()["project"]["board"]["setup"].clone();
    setup["stackup"]["strata"][0]["role"] = json!("signal");
    setup["stackup"]["finished_thickness_nm"] = json!(1_600_000);
    s.apply(crate::SET_BOARD_SETUP, &json!({"board":target.board,"setup":setup,"expected_revision":s.revision()})).unwrap();
    let id = Uuid::from_u128(84);
    let profile = content(json!({"name":"Stacked","corrections":[{"library":LIBRARY,"definition":"resistor-0402","rotation":90}]}), 1);
    pin(&mut s, Some(id), profile).unwrap();
    let pinned = encode_documents(s.snapshot().unwrap()).unwrap();
    let path = test_board_path(&pinned);
    let board: Value = serde_json::from_slice(&pinned[&path]).unwrap();
    assert_eq!(board["format"], "pcbcraft-board/23");
    assert!(board.get("assembly_profiles").is_none());

    adopt(&mut s, id, Some(1)).unwrap();
    let adopted = encode_documents(s.snapshot().unwrap()).unwrap();
    let mut saved: Value = serde_json::from_slice(&adopted[&path]).unwrap();
    assert_eq!(saved["format"], "pcbcraft-board/23");
    assert_eq!(saved["setup"], board["setup"]);
    assert_eq!(saved["placement_origin"], board["placement_origin"]);
    assert_eq!(saved["assembly_profiles"], json!([{"profile":id,"revision":1}]));
    let reopened = decode_documents(&adopted).unwrap();
    let reopened_board = reopened.board_view(target).unwrap();
    assert_eq!(reopened.board_setup(), s.project().unwrap().board_setup());
    assert_eq!(reopened_board.authored_placement_origin(), view(&s).authored_placement_origin());
    assert_eq!(reopened_board.placement_row(&footprint, Some(id)).unwrap(), Placed { at: Point { x_nm: 30 * MM, y_nm: -15 * MM }, rotation: 120 });
    assert_eq!(encode_documents(reopened).unwrap(), adopted);
    let mut downgraded = adopted.clone();
    saved["format"] = json!("pcbcraft-board/22");
    downgraded.insert(path, serde_json::to_vec(&saved).unwrap());
    refused_because(decode_documents(&downgraded), "require board format 23");

    s.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), pinned);
    s.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), adopted);
    adopt(&mut s, id, None).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), pinned);
}

/// Profile pinning preserves both formats for a board's part-group memberships and their Undo history.
#[test]
fn assembly_profiles_preserve_expanded_and_compact_group_memberships() {
    use crate::part_groups::{CREATE, SET_GROUPS};
    use crate::persistence::{decode_packed, encode_packed};

    let (mut s, _) = fixture();
    s.apply(CREATE, &json!({"name":"Sample group","expected_revision":s.revision()})).unwrap();
    let project = s.project().unwrap();
    let group = project.part_groups().groups[0].id;
    let member = project.part_instance_addresses().next().unwrap();
    s.apply(SET_GROUPS, &json!({"part_instance":member,"groups":[group],"expected_revision":s.revision()})).unwrap();
    let grouped = s.snapshot().unwrap();
    let instance_path = grouped.instances_document_path().unwrap();
    let id = Uuid::from_u128(85);
    for (compact, project_format, instances_format) in
        [(false, "pcbcraft-project/24", "pcbcraft-instances/5"), (true, "pcbcraft-project/39", "pcbcraft-instances/7")]
    {
        let grouped = Arc::new(Project { prefer_compact_part_groups: compact, ..Project::clone(&grouped) });
        let original = encode_documents(grouped.clone()).unwrap();
        let entry: Value = serde_json::from_slice(&original["project.json"]).unwrap();
        let instances: Value = serde_json::from_slice(&original[&instance_path]).unwrap();
        assert_eq!(entry["format"], project_format);
        assert_eq!(instances["format"], instances_format);
        s.open_snapshot(decode_documents(&original).unwrap());

        pin(&mut s, Some(id), content(json!({"name":"Grouped project"}), 1)).unwrap();
        let pinned = s.snapshot().unwrap();
        let docs = encode_documents(pinned.clone()).unwrap();
        let entry: Value = serde_json::from_slice(&docs["project.json"]).unwrap();
        assert_eq!(entry["format"], "pcbcraft-project/36");
        assert_eq!(docs[&instance_path], original[&instance_path]);
        let reopened = decode_documents(&docs).unwrap();
        assert_eq!(reopened.part_groups(), grouped.part_groups());
        assert_eq!(reopened.part_group_memberships(), grouped.part_group_memberships());
        assert_eq!(reopened.assembly_profiles(), pinned.assembly_profiles());
        assert_eq!(encode_documents(reopened).unwrap(), docs);
        assert_eq!(decode_packed(&encode_packed(pinned).unwrap()).unwrap().documents, docs);

        let mut invalid = docs.clone();
        let mut wrong_header = instances.clone();
        wrong_header["format"] = json!("pcbcraft-instances/1");
        invalid.insert(instance_path.clone(), serde_json::to_vec(&wrong_header).unwrap());
        refused_because(decode_documents(&invalid), if compact { "instances format 7" } else { "unsupported instances format" });
        if compact {
            let mut mixed = instances;
            mixed["part_group_memberships"] = json!(grouped.part_group_memberships().iter().collect::<Vec<_>>());
            invalid.insert(instance_path.clone(), serde_json::to_vec(&mixed).unwrap());
            refused_because(decode_documents(&invalid), "exactly one membership encoding");
        }

        s.apply("edit.undo", &Value::Null).unwrap();
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), original);
        s.apply("edit.redo", &Value::Null).unwrap();
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), docs);
    }
}

/// Preview exposes corrected rows before acceptance; stale or altered reviews cannot change state.
#[test]
fn adoption_reviews_values_and_checks_revision_atomically() {
    let (mut s, ids) = placed();
    let id = Uuid::from_u128(45);
    pin(
        &mut s,
        Some(id),
        content(json!({"name":"Portable","vendor":"Any vendor","corrections":[{"library":LIBRARY,"definition":"resistor-0402","rotation":180}]}), 1),
    )
    .unwrap();
    let board = s.project().unwrap().board_target().unwrap().board;
    let mut params = json!({"board":board,"expected_revision":s.revision(),"profile":id,"revision":1});
    let before = s.snapshot().unwrap();
    let preview = s.execute(PREVIEW_ASSEMBLY_PROFILE, &params).unwrap();
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    assert_eq!(preview["changed_corrections"].as_array().unwrap().len(), 1);
    assert_eq!(preview["affected_rows"].as_array().unwrap().len(), 2);
    assert_eq!(preview["affected_rows"][0]["before"]["rotation"], 30);
    assert_eq!(preview["affected_rows"][0]["after"]["rotation"], 210);
    params["acceptance"] = json!("0".repeat(64));
    invalid_because(s.apply(ADOPT_ASSEMBLY_PROFILE, &params), "preview changed");
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    params["acceptance"] = preview["acceptance"].clone();
    place(&mut s, &ids[0], Point { x_nm: 41 * MM, y_nm: 45 * MM }, 30, "front");
    let moved = s.snapshot().unwrap();
    let stale = s.apply(ADOPT_ASSEMBLY_PROFILE, &params);
    assert!(matches!(stale,Err(CommandError::Unavailable(ref m)) if m.contains("project changed")), "{stale:?}");
    assert!(Arc::ptr_eq(&moved, &s.snapshot().unwrap()));
    // Even replacing only the revision number cannot reuse an old acceptance token.
    params["expected_revision"] = json!(s.revision());
    invalid_because(s.apply(ADOPT_ASSEMBLY_PROFILE, &params), "preview changed");
    adopt(&mut s, id, Some(1)).unwrap();
    assert_eq!(view(&s).placement_row(&ids[0], Some(id)).unwrap().rotation, 210);
}

/// Retiring profiles keeps adopted bytes; cleanup releases only unused revisions and Undo restores them.
#[test]
fn removal_and_cleanup_preserve_adoptions_and_undo_redo() {
    let (mut s, ids) = placed();
    let id = Uuid::from_u128(51);
    set(&mut s, Some(id), json!({"name":"Stable","corrections":[{"library":LIBRARY,"definition":"resistor-0402","rotation":180}]})).unwrap();
    let first = s.snapshot().unwrap();
    let row = view(&s).placement_row(&ids[0], Some(id)).unwrap();
    pin(&mut s, Some(id), content(json!({"name":"Stable","corrections":[{"library":LIBRARY,"definition":"resistor-0402","rotation":90}]}), 2))
        .unwrap();
    assert_eq!(view(&s).placement_row(&ids[0], Some(id)).unwrap(), row, "pinning is not adoption");
    assert_eq!(s.project().unwrap().assembly_profiles.revisions.len(), 2);
    adopt(&mut s, id, Some(2)).unwrap();
    let second = s.snapshot().unwrap();
    assert_eq!(second.assembly_profiles.revisions.len(), 1);
    s.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(view(&s).placement_row(&ids[0], Some(id)).unwrap(), row);
    assert_eq!(s.project().unwrap().assembly_profiles.revisions.len(), 2);
    s.apply("edit.redo", &Value::Null).unwrap();
    crate::assert_same_content(&second, &s.snapshot().unwrap());
    pin(&mut s, Some(id), Value::Null).unwrap();
    assert_eq!(view(&s).placement_row(&ids[0], Some(id)).unwrap(), Placed { rotation: 120, ..row });
    let retired = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&retired).unwrap()).unwrap(), retired);
    adopt(&mut s, id, None).unwrap();
    assert!(s.project().unwrap().assembly_profiles.revisions.is_empty());
    let empty = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&empty).unwrap()).unwrap(), empty);
    invalid_because(pin(&mut s, Some(id), content(json!({"name":"Different"}), 1)), "revision conflict");
    s.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), retired);
    assert_eq!(first.assembly_profiles.revisions[0].content.revision, 1, "cleanup cannot mutate history");
}

/// Pinning alone migrates supported legacy metadata, preserves identities and undoes in one step.
fn legacy_pin_round_trip(project: Arc<Project>) {
    let mut s = Session::default();
    s.open_snapshot(project);
    let before = s.snapshot().unwrap();
    let board = before.board_target().unwrap();
    let sheets: Vec<_> = before.sheets.iter().map(|sheet| sheet.sheet_id).collect();
    let id = Uuid::from_u128(93);
    let profile = content(json!({"name":"Legacy"}), 1);
    pin(&mut s, Some(id), profile.clone()).unwrap();
    let pinned = s.snapshot().unwrap();
    assert_eq!(pinned.id(), before.id());
    assert_eq!(pinned.board_target(), Some(board));
    assert_eq!(pinned.sheets.iter().map(|sheet| sheet.sheet_id).collect::<Vec<_>>(), sheets);
    assert!(Arc::ptr_eq(&pinned.components, &before.components));
    assert!(Arc::ptr_eq(&pinned.board, &before.board), "pinning does not adopt or edit a board");
    assert!(pinned.board.assembly_profiles.is_empty());
    assert!(pinned.has_sheet_occurrences());
    assert!(pinned.numeric_id_high_water.is_some());
    assert_eq!(pinned.layout_root(), Some(before.layout_root().unwrap_or(".")));
    if before.board_folder().is_some() {
        assert_eq!(pinned.board_folder(), before.board_folder());
    }
    let saved = encode_documents(pinned.clone()).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&saved["project.json"]).unwrap()["format"], "pcbcraft-project/36");
    assert_eq!(encode_documents(decode_documents(&saved).unwrap()).unwrap(), saved);
    let packed = crate::persistence::encode_packed(pinned.clone()).unwrap();
    assert_eq!(encode_documents(crate::persistence::decode_packed(&packed).unwrap().project).unwrap(), saved);
    let revision = s.revision();
    assert_eq!(pin(&mut s, Some(id), profile).unwrap(), revision);
    assert!(Arc::ptr_eq(&pinned, &s.snapshot().unwrap()));
    s.apply("edit.undo", &Value::Null).unwrap();
    let restored = s.snapshot().unwrap();
    crate::assert_same_content(&before, &restored);
    assert!(restored.assembly_profiles.heads.is_empty());
    assert_eq!(restored.layout_root(), before.layout_root());
    assert_eq!(restored.has_sheet_occurrences(), before.has_sheet_occurrences());
    let restored_documents = encode_documents(restored).unwrap();
    assert_eq!(encode_documents(decode_documents(&restored_documents).unwrap()).unwrap(), restored_documents);
    s.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), saved);
}

/// Real legacy flat/split codecs, including explicit flat registration, can pin without board adoption.
#[test]
fn legacy_pin_only_round_trips_flat_split_and_packed_projects() {
    use crate::persistence::{decode, decode_packed, encode};
    for bytes in [
        include_bytes!("../../../../app/tests/fixtures/two-resistors-format-1.pc").as_slice(),
        include_bytes!("../../../../app/tests/fixtures/two-resistors-format-2.pc").as_slice(),
        include_bytes!("../../../../app/tests/fixtures/two-resistors.pc").as_slice(),
    ] {
        let project = decode(bytes).unwrap();
        assert_eq!(project.layout_root(), None);
        assert!(!project.has_sheet_occurrences());
        legacy_pin_round_trip(project);
    }
    let old_split = decode_packed(include_bytes!("../../../../app/tests/fixtures/split/two-resistors.pc")).unwrap().project;
    assert_eq!(old_split.layout_root(), None);
    legacy_pin_round_trip(old_split.clone());
    let mut registered = Session::default();
    registered.open_snapshot(old_split);
    registered
        .apply(
            crate::PLACE_FOOTPRINT,
            &json!({"symbol":"symbol-2","definition":"generic-through-hole-2","pin_1_pad":1,
        "x_nm":0,"y_nm":0,"expected_revision":registered.revision()}),
        )
        .unwrap();
    let flat = encode(registered.snapshot().unwrap()).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&flat).unwrap()["format"], "pcbcraft-project/10");
    let flat = decode(&flat).unwrap();
    assert_eq!(flat.boards().next().unwrap().name(), "Board");
    assert_eq!(flat.layout_root(), None);
    assert!(!flat.has_sheet_occurrences());
    legacy_pin_round_trip(flat.clone());
    let split = encode_documents(flat.clone()).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&split["project.json"]).unwrap()["format"], "pcbcraft-project/9");
    legacy_pin_round_trip(decode_documents(&split).unwrap());
    let foldered = flat.with_board_folders("manufacturing").unwrap().with_board_folder("existing-board").unwrap();
    legacy_pin_round_trip(Arc::new(foldered));
}

/// Catalog commands retain the existing no-board availability rule without migrating a refused edit.
#[test]
fn schematic_only_profile_pin_remains_unavailable() {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":false})).unwrap();
    let before = s.snapshot().unwrap();
    assert_eq!(before.board_target(), None);
    let result = pin(&mut s, None, content(json!({"name":"Unavailable"}), 1));
    assert!(
        matches!(&result, Err(CommandError::Unavailable(message)) if message.contains(SET_ASSEMBLY_PROFILE) && message.contains("can't run now")),
        "{result:?}"
    );
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    let saved = encode_documents(before).unwrap();
    assert_eq!(encode_documents(decode_documents(&saved).unwrap()).unwrap(), saved);
}

/// Provenance and optional vendor metadata reject null, unknown fields and first oversized values.
#[test]
fn metadata_edges_change_nothing_and_imported_content_reopens() {
    let (mut s, _) = fixture();
    let before = s.snapshot().unwrap();
    for (field, value, reason) in [
        ("vendor", Value::Null, "expected a string"),
        ("vendor", json!(7), "expected a string"),
        ("vendor", json!(""), "metadata needs"),
        ("vendor", json!(" padded"), "metadata needs"),
        ("vendor", json!("x".repeat(MAX_NAME_CHARS + 1)), "metadata needs"),
        ("provenance", Value::Null, "invalid type: null"),
        ("provenance", json!("authored"), "internally tagged enum Provenance"),
        ("provenance", json!({}), "missing field `kind`"),
        ("provenance", json!({"kind":"unknown"}), "unknown variant `unknown`"),
        ("provenance", json!({"kind":"authored","source":"extra"}), "unknown field `source`"),
        ("provenance", json!({"kind":"imported"}), "missing field `source`"),
        ("provenance", json!({"kind":"imported","source":null}), "expected a string"),
        ("provenance", json!({"kind":"imported","source":""}), "bounded source hint"),
        ("provenance", json!({"kind":"imported","source":"padded "}), "bounded source hint"),
        ("provenance", json!({"kind":"imported","source":"x".repeat(MAX_NAME_CHARS+1)}), "bounded source hint"),
        ("provenance", json!({"kind":"imported","source":"valid","extra":true}), "unknown field `extra`"),
    ] {
        let mut invalid = content(json!({"name":"Metadata"}), 1);
        invalid[field] = value;
        refused_because(parse(&invalid), reason);
        invalid_because(pin(&mut s, None, invalid), "immutable profile content");
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()), "{field}");
    }
    let last = "\u{10ffff}".repeat(MAX_NAME_CHARS);
    let mut good = content(json!({"name":"Imported","vendor":last}), 1);
    good["provenance"] = json!({"kind":"imported","source":last});
    pin(&mut s, None, good).unwrap();
    let docs = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&docs).unwrap()).unwrap(), docs);
}

/// Saved revisions and coordinates use canonical integer tokens, unlike normalized command inputs.
#[test]
fn saved_numbers_refuse_decimal_tokens() {
    let (mut s, _) = fixture();
    set(
        &mut s,
        Some(Uuid::from_u128(1)),
        json!({"name":"Canonical","corrections":[{"library":LIBRARY,"definition":"resistor-0402","rotation":90,"centroid":{"x_nm":1,"y_nm":1}}]}),
    )
    .unwrap();
    let docs = encode_documents(s.snapshot().unwrap()).unwrap();
    let path = test_board_path(&docs);
    for pointer in [
        "/assembly_profiles/revisions/0/content/revision",
        "/assembly_profiles/revisions/0/content/corrections/0/rotation",
        "/assembly_profiles/revisions/0/content/corrections/0/centroid/x_nm",
    ] {
        let mut altered = docs.clone();
        let mut project: Value = serde_json::from_slice(&altered["project.json"]).unwrap();
        *project.pointer_mut(pointer).unwrap() = serde_json::from_str("1.0").unwrap();
        altered.insert("project.json".into(), serde_json::to_vec(&project).unwrap());
        refused_because(decode_documents(&altered), "floating point");
    }
    let mut altered = docs.clone();
    let mut board: Value = serde_json::from_slice(&altered[&path]).unwrap();
    board["assembly_profiles"][0]["revision"] = serde_json::from_str("1e0").unwrap();
    altered.insert(path, serde_json::to_vec(&board).unwrap());
    refused_because(decode_documents(&altered), "floating point");
}

/// Metadata-only revisions share correction lists; changing one correction shares every intact sibling.
#[test]
fn content_revisions_share_unchanged_corrections_with_history() {
    let (mut s, _) = fixture();
    let id = Uuid::from_u128(91);
    let initial = content(
        json!({"name":"First","corrections":[
            {"library":LIBRARY,"definition":"capacitor-0402","rotation":90},
            {"library":LIBRARY,"definition":"resistor-0402","rotation":180}
        ]}),
        1,
    );
    pin(&mut s, Some(id), initial.clone()).unwrap();
    let first = s.snapshot().unwrap();
    let mut metadata = initial;
    metadata["revision"] = json!(2);
    metadata["name"] = json!("Renamed");
    pin(&mut s, Some(id), metadata.clone()).unwrap();
    let second = s.snapshot().unwrap();
    let a = &first.assembly_profiles.revisions[0].content.corrections;
    let b = &second.assembly_profiles.revisions[0].content.corrections;
    assert!(Arc::ptr_eq(a, b), "metadata edits do not copy the correction list");
    metadata["revision"] = json!(3);
    metadata["corrections"][1]["rotation"] = json!(270);
    pin(&mut s, Some(id), metadata).unwrap();
    let third = s.snapshot().unwrap();
    let c = &third.assembly_profiles.revisions[0].content.corrections;
    assert!(!Arc::ptr_eq(b, c));
    assert!(Arc::ptr_eq(&b[0], &c[0]));
    assert!(!Arc::ptr_eq(&b[1], &c[1]));
    assert_eq!(a[1].rotation, 180, "old snapshots remain immutable");
    s.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&second, &s.snapshot().unwrap());
    s.apply("edit.redo", &Value::Null).unwrap();
    crate::assert_same_content(&third, &s.snapshot().unwrap());
}

/// The command schema matches its checked-in snapshot and accepts equivalent numeric spellings.
#[test]
fn command_numeric_spellings_match_the_schema_snapshot() {
    assert_eq!(schema(), serde_json::from_str::<Value>(include_str!("content-schema.json")).unwrap());
    for (number, expected) in [
        ("1.0", Some(1)),
        ("1e0", Some(1)),
        ("-0", Some(0)),
        ("1.5", None),
        ("1e-1", None),
        ("4294967295.0", Some(u32::MAX as i64)),
        ("9007199254740993", Some(9007199254740993)),
        ("9223372036854775806", Some(i64::MAX - 1)),
        ("9223372036854775807", Some(i64::MAX)),
        ("9223372036854775808", None),
        ("-9223372036854775808", Some(i64::MIN)),
        ("-9223372036854775809", None),
        ("18446744073709551615", None),
    ] {
        assert_eq!(integer(&serde_json::from_str::<Value>(number).unwrap()), expected, "{number}");
    }
    for spelling in ["1.0", "1e0", "4294967295.0"] {
        let (mut s, _) = fixture();
        let id = Uuid::from_u128(92);
        let number: Value = serde_json::from_str(spelling).unwrap();
        let profile = json!({"name":"Numeric","revision":number,"provenance":{"kind":"authored"},"corrections":[
            {"library":LIBRARY,"definition":"resistor-0402","rotation":serde_json::from_str::<Value>("-0").unwrap(),"centroid":{"x_nm":serde_json::from_str::<Value>("1e0").unwrap(),"y_nm":serde_json::from_str::<Value>("-0").unwrap()}}
        ]});
        crate::ParamKind::AssemblyProfile.validate(&profile).unwrap();
        pin(&mut s, Some(id), profile).unwrap();
        let board = s.project().unwrap().board_target().unwrap().board;
        let mut adoption = json!({"board":board,"expected_revision":s.revision(),"profile":id,"revision":number});
        let preview = s.execute(PREVIEW_ASSEMBLY_PROFILE, &adoption).unwrap();
        adoption["acceptance"] = preview["acceptance"].clone();
        s.apply(ADOPT_ASSEMBLY_PROFILE, &adoption).unwrap();
        assert_eq!(view(&s).assembly_profiles()[0].revision, integer(&number).unwrap() as u32);
        let saved = encode_documents(s.snapshot().unwrap()).unwrap();
        assert_eq!(encode_documents(decode_documents(&saved).unwrap()).unwrap(), saved);
    }
    for (bad, reason) in [
        ("0", "content revision must be positive"),
        ("-0", "content revision must be positive"),
        ("-1", "expected u32"),
        ("1.5", "expected u32"),
        ("4294967296", "expected u32"),
        ("9007199254740993", "expected u32"),
        ("18446744073709551615", "expected u32"),
    ] {
        let (mut s, _) = fixture();
        let mut profile = content(json!({"name":"Bad"}), 1);
        profile["revision"] = serde_json::from_str(bad).unwrap();
        let before = s.snapshot().unwrap();
        refused_because(parse(&profile), reason);
        invalid_because(pin(&mut s, None, profile), "immutable profile content");
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    }
}

/// Raw decimal input follows the shared Value precision; conversion never rounds a remaining fraction.
#[test]
fn command_numbers_use_the_shared_parsed_value_precision() {
    let parsed = |revision: &str, rotation: &str, x: &str| {
        serde_json::from_str::<Value>(&format!(
            r#"{{"revision":{revision},"name":"Parsed","provenance":{{"kind":"authored"}},"corrections":[{{"library":"{LIBRARY}","definition":"resistor-0402","rotation":{rotation},"centroid":{{"x_nm":{x},"y_nm":1}}}}]}}"#
        ))
        .unwrap()
    };
    for (revision, rotation, x, expected_revision, expected_rotation, expected_x) in [
        ("1.00000000000000000001", "90.00000000000000000001", "1e-400", 1, 90, 0),
        ("4294967295.0000001", "359.00000000000000000001", "100000000.000000001", u32::MAX, 359, MAX_OFFSET_NM),
        ("1e0", "-0", "-100000000.000000001", 1, 0, -MAX_OFFSET_NM),
    ] {
        let value = parsed(revision, rotation, x);
        crate::ParamKind::AssemblyProfile.validate(&value).unwrap();
        let (mut s, _) = fixture();
        let id = Uuid::from_u128(93);
        pin(&mut s, Some(id), value.clone()).unwrap();
        let snapshot = s.snapshot().unwrap();
        let content = &snapshot.assembly_profiles.revisions[0].content;
        assert_eq!(content.revision, expected_revision);
        assert_eq!(content.corrections[0].rotation, expected_rotation);
        assert_eq!(content.corrections[0].centroid, Some(Point { x_nm: expected_x, y_nm: 1 }));
        let board = snapshot.board_target().unwrap().board;
        let mut adoption = json!({"board":board,"expected_revision":s.revision(),"profile":id,"revision":value["revision"]});
        let preview = s.execute(PREVIEW_ASSEMBLY_PROFILE, &adoption).unwrap();
        adoption["acceptance"] = preview["acceptance"].clone();
        s.apply(ADOPT_ASSEMBLY_PROFILE, &adoption).unwrap();
        assert_eq!(view(&s).assembly_profiles()[0].revision, expected_revision);
        let docs = encode_documents(s.snapshot().unwrap()).unwrap();
        assert_eq!(encode_documents(decode_documents(&docs).unwrap()).unwrap(), docs);
    }
    for (revision, rotation, x, reason) in [
        ("1.0000000000000002", "90", "1", "expected u32"),
        ("4294967295.000001", "90", "1", "expected u32"),
        ("1", "90.00000000000001", "1", "expected u16"),
        ("1", "359.99999999999999999", "1", "rotation correction"),
        ("1", "90", "1.0000000000000002", "expected i64"),
        ("1", "90", "100000000.00000002", "expected i64"),
        ("1", "90", "1e-100", "expected i64"),
    ] {
        let (mut s, _) = fixture();
        let before = s.snapshot().unwrap();
        let value = parsed(revision, rotation, x);
        refused_because(parse(&value), reason);
        invalid_because(pin(&mut s, None, value), "immutable profile content");
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    }
}

/// Required saved fields, unique identities and references are enforced without masking errors.
#[test]
fn saved_catalog_and_adoption_edges_are_refused() {
    let (mut s, _) = fixture();
    set(&mut s, Some(Uuid::from_u128(1)), json!({"name":"One"})).unwrap();
    set(&mut s, Some(Uuid::from_u128(2)), json!({"name":"Two"})).unwrap();
    let docs = encode_documents(s.snapshot().unwrap()).unwrap();
    let path = test_board_path(&docs);
    let project: Value = serde_json::from_slice(&docs["project.json"]).unwrap();
    let board: Value = serde_json::from_slice(&docs[&path]).unwrap();
    let check = |key: &str, value: Value, reason: &str| {
        let mut altered = docs.clone();
        altered.insert(key.into(), serde_json::to_vec(&value).unwrap());
        refused_because(decode_documents(&altered), reason);
    };
    let mut missing = project.clone();
    missing["assembly_profiles"]["revisions"][0]["content"].as_object_mut().unwrap().remove("bottom_side");
    check("project.json", missing, "bottom_side");
    let mut authored_extra = project.clone();
    authored_extra["assembly_profiles"]["revisions"][0]["content"]["provenance"]["source"] = json!("Not imported");
    check("project.json", authored_extra, "unknown field `source`");
    let mut nil = project.clone();
    nil["assembly_profiles"]["heads"][0]["profile"] = json!(Uuid::nil());
    check("project.json", nil, "non-nil");
    let mut duplicate = project.clone();
    duplicate["assembly_profiles"]["revisions"][1]["id"] = json!(Uuid::from_u128(1));
    check("project.json", duplicate, "unique and sorted");
    let mut names = project.clone();
    names["assembly_profiles"]["revisions"][1]["content"]["name"] = json!("One");
    check("project.json", names, "names must be unique");
    let mut no_catalog = project.clone();
    no_catalog.as_object_mut().unwrap().remove("assembly_profiles");
    check("project.json", no_catalog, "assembly profiles require");
    for (adoptions, reason) in [(json!([]), "empty assembly_profiles"), (Value::Null, "null")] {
        let mut bad = board.clone();
        bad["assembly_profiles"] = adoptions;
        check(&path, bad, reason);
    }
    let mut older = board.clone();
    older["format"] = json!("pcbcraft-board/21");
    check(&path, older, "adoptions require format 22");
    let mut omitted = board.clone();
    omitted.as_object_mut().unwrap().remove("assembly_profiles");
    check(&path, omitted, "adoptions require format 22");
    let mut duplicates = board;
    duplicates["assembly_profiles"][1] = duplicates["assembly_profiles"][0].clone();
    check(&path, duplicates, "unique, sorted");
    let before = s.snapshot().unwrap();
    invalid_because(pin(&mut s, None, content(json!({"name":"One"}), 1)), "names must be unique");
    invalid_because(
        s.apply(SET_ASSEMBLY_PROFILE, &json!({"profile":null,"content":content(json!({"name":"Null"}),1),"expected_revision":s.revision()})),
        "non-nil UUID",
    );
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
}
