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
    let id = profile.unwrap_or_else(Uuid::new_v4);
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
/// 180 degrees more, on both sides and under both bottom-side conventions. The generic rows,
/// the capacitor and a profile without that offset are unchanged.
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
            provenance: Provenance::Authored,
            vendor: None,
            bottom_side: BottomSide::ViewedFromTop,
            corrections: vec![PackageCorrection { library: LIBRARY.into(), definition: definition.into(), rotation: 90, centroid: None }],
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
    assert_eq!(inspected["project"]["assembly_profiles"]["revisions"][0]["content"], c);
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
    for value in [
        json!({}),
        json!({"name":""}),
        json!({"name":" padded"}),
        json!({"name":"padded\u{feff}"}),
        json!({"name":"tab\tinside"}),
        json!({"name":"n".repeat(MAX_NAME_CHARS+1)}),
        json!({"name":"X","bottom_side":"mirrored"}),
        json!({"name":"X","bottom_side":null}),
        json!({"name":"X","corrections":null}),
        json!({"name":"X","extra":1}),
        json!({"name":7}),
        json!("X"),
        package(json!({})),
        package(json!({"rotation":0})),
        package(json!({"rotation":360})),
        package(json!({"rotation":-1})),
        package(json!({"rotation":90.5})),
        package(json!({"centroid":{"x_nm":0,"y_nm":0}})),
        package(json!({"centroid":{"x_nm":edge,"y_nm":0}})),
        package(json!({"centroid":{"x_nm":0,"y_nm":-edge}})),
        package(json!({"centroid":{"x_nm":1}})),
        package(json!({"rotation":90,"centroid":null})),
        package(json!({"rotation":90,"revision":1})),
        package(json!({"rotation":90,"library":"Basic"})),
        package(json!({"rotation":90,"definition":""})),
        json!({"name":"X","corrections":[{"library":LIBRARY,"definition":"a","rotation":1},{"library":LIBRARY,"definition":"a","rotation":2}]}),
    ] {
        invalid_because(pin(&mut s, None, content(value.clone(), 1)), "immutable profile content");
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

/// Last valid profile and correction counts pass; one more, duplicate ids and invalid revisions fail.
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
        catalog.revisions.push(Arc::new(AssemblyProfile {
            format: FORMAT.into(),
            id,
            content: ProfileContent {
                revision: u32::MAX,
                name: format!("{h:02}{}", "\u{10ffff}".repeat(MAX_NAME_CHARS - 2)),
                vendor: Some("\u{10ffff}".repeat(MAX_NAME_CHARS)),
                provenance: Provenance::Imported { source: "\u{10ffff}".repeat(MAX_NAME_CHARS) },
                bottom_side: BottomSide::ViewedFromBottom,
                corrections: (0..MAX_CORRECTIONS / MAX_PROFILES)
                    .map(|n| PackageCorrection {
                        library: format!("{h:05}{}", "x".repeat(59)),
                        definition: format!("{n:05}{}", "x".repeat(59)),
                        rotation: 359,
                        centroid: Some(Point { x_nm: -MAX_OFFSET_NM, y_nm: -MAX_OFFSET_NM }),
                    })
                    .collect(),
            },
        }));
    }
    catalog.validate(&[]).unwrap();
    let bytes = serde_json::to_vec_pretty(&catalog).unwrap().len();
    assert!(bytes < crate::persistence::MAX_DOCUMENT_BYTES / 8, "{bytes}");
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
