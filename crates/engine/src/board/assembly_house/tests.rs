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

/// Create (no `house`), replace or remove a house through the shared command.
fn set(s: &mut Session, house: Option<Uuid>, profile: Value) -> Result<u64, CommandError> {
    let board = s.project().unwrap().board_target().unwrap().board;
    let mut params = json!({"board":board,"expected_revision":s.revision(),"profile":profile});
    if let Some(house) = house {
        params["house"] = json!(house);
    }
    s.apply(SET_ASSEMBLY_HOUSE, &params)
}

/// The id of the house with this name.
fn house(s: &Session, name: &str) -> Uuid {
    view(s).assembly_houses().iter().find(|house| house.name == name).unwrap().id
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

/// A +180 degree offset for the 0402 resistor in one house turns that house's resistor rows by
/// 180 degrees more, on both sides and under both bottom-side conventions. The generic rows,
/// the capacitor and a house without that offset are unchanged.
#[test]
fn half_turn_offset_changes_only_that_house() {
    let (mut s, [top, bottom, capacitor]) = placed();
    let rows = |s: &Session, house| [&top, &bottom, &capacitor].map(|id| view(s).placement_row(id, house).unwrap());
    let generic = rows(&s, None);
    // Top resistor: X = 40 - 10 = 30 mm, Y up = 30 - 45 = -15 mm, rotation 30.
    assert_eq!(generic[0], Placed { at: Point { x_nm: 30 * MM, y_nm: -15 * MM }, rotation: 30 });
    // Bottom resistor seen from the top (IPC-2581, 3.3.2): X = 30 mm, Y = 5 mm, rotation 30.
    assert_eq!(generic[1], Placed { at: Point { x_nm: 30 * MM, y_nm: 5 * MM }, rotation: 30 });
    // Bottom capacitor: X = 20 - 10 = 10 mm, Y = 5 mm, rotation 90.
    assert_eq!(generic[2], Placed { at: Point { x_nm: 10 * MM, y_nm: 5 * MM }, rotation: 90 });

    let turned = json!({"library":LIBRARY,"definition":"resistor-0402","rotation":180});
    set(&mut s, None, json!({"name":"Turned","packages":[turned]})).unwrap();
    set(&mut s, None, json!({"name":"Turned from below","bottom_side":"viewed_from_bottom","packages":[turned]})).unwrap();
    set(&mut s, None, json!({"name":"Other","packages":[{"library":LIBRARY,"definition":"capacitor-0402","rotation":90}]})).unwrap();
    assert_eq!(rows(&s, None), generic, "generic rows ignore every house");

    let turned = rows(&s, Some(house(&s, "Turned")));
    // 30 + 180 = 210 on top. Seen from the top the bottom part's offset subtracts, 30 - 180 =
    // -150, which is also 210 mod 360. Positions stay at the footprint origin.
    assert_eq!(turned[0], Placed { rotation: 210, ..generic[0] });
    assert_eq!(turned[1], Placed { rotation: 210, ..generic[1] });
    assert_eq!(turned[2], generic[2], "the capacitor has no offset in this house");

    let below = rows(&s, Some(house(&s, "Turned from below")));
    assert_eq!(below[0], Placed { rotation: 210, ..generic[0] });
    // Seen from below the bottom resistor is at 360 - 30 = 330 with X = -30 mm; 330 + 180 = 150.
    assert_eq!(below[1], Placed { at: Point { x_nm: -30 * MM, y_nm: 5 * MM }, rotation: 150 });
    // The capacitor follows the convention only: X = -10 mm, rotation 360 - 90 = 270.
    assert_eq!(below[2], Placed { at: Point { x_nm: -10 * MM, y_nm: 5 * MM }, rotation: 270 });

    let other = rows(&s, Some(house(&s, "Other")));
    assert_eq!([other[0], other[1]], [generic[0], generic[1]], "the resistor offset belongs to another house");
    // The capacitor's +90 subtracts seen from the top: 90 - 90 = 0.
    assert_eq!(other[2], Placed { rotation: 0, ..generic[2] });
}

/// A centroid offset is a point of the library footprint: the house row lands exactly where
/// the board puts that point. Checked against pad 1's real position on both sides, under both
/// conventions, at every quarter turn, and off both axes at 90 degrees.
#[test]
fn centroid_offset_follows_the_footprint_transform() {
    let (mut s, [id, _, _]) = placed();
    let pad_1 = basic_components("resistor-0402").unwrap().pads.iter().find(|pad| pad.number == "1").unwrap().center;
    let package = json!([{"library":LIBRARY,"definition":"resistor-0402","centroid":pad_1}]);
    set(&mut s, None, json!({"name":"Top view","packages":package})).unwrap();
    set(&mut s, None, json!({"name":"Bottom view","bottom_side":"viewed_from_bottom","packages":package})).unwrap();
    let frame = view(&s).placement_origin().unwrap().frame();
    for side in ["front", "back"] {
        for rotation in [0, 90, 180, 270] {
            place(&mut s, &id, Point { x_nm: 40 * MM, y_nm: 25 * MM }, rotation, side);
            let footprint = view(&s).footprints().find(|f| f.id == id).unwrap();
            let pad = frame.to_output(footprint.pads.iter().find(|pad| pad.number == "1").unwrap().center);
            let generic = view(&s).placement_row(&id, None).unwrap();
            let top = view(&s).placement_row(&id, Some(house(&s, "Top view"))).unwrap();
            assert_eq!(top, Placed { at: pad, rotation: generic.rotation }, "{side} {rotation}");
            let bottom = view(&s).placement_row(&id, Some(house(&s, "Bottom view"))).unwrap();
            let x = if side == "back" { -pad.x_nm } else { pad.x_nm };
            assert_eq!(bottom.at, Point { x_nm: x, y_nm: pad.y_nm }, "{side} {rotation}");
        }
    }
    // Off both axes: local (0.3 mm, -0.2 mm), Y down, on a top part at 90 degrees around board
    // (40 mm, 25 mm). A counterclockwise quarter turn with Y down maps local (x, y) to (y, -x),
    // so the point is at board (39.8 mm, 24.7 mm): output X 39.8 - 10 = 29.8 mm, Y up 30 - 24.7
    // = 5.3 mm.
    let centroid = Point { x_nm: 300_000, y_nm: -200_000 };
    let top_view = house(&s, "Top view");
    set(&mut s, Some(top_view), json!({"name":"Top view","packages":[{"library":LIBRARY,"definition":"resistor-0402","centroid":centroid}]}))
        .unwrap();
    place(&mut s, &id, Point { x_nm: 40 * MM, y_nm: 25 * MM }, 90, "front");
    let row = view(&s).placement_row(&id, Some(top_view)).unwrap();
    assert_eq!(row, Placed { at: Point { x_nm: 29_800_000, y_nm: 5_300_000 }, rotation: 90 });
}

/// Rows refuse footprints and houses that are not on the board; a house without offsets
/// under the default convention writes the generic row.
#[test]
fn rows_name_what_is_missing() {
    let (mut s, [id, _, _]) = placed();
    assert!(view(&s).placement_row("footprint-999", None).unwrap_err().contains("footprint-999"));
    let missing = Uuid::from_u128(7);
    assert!(view(&s).placement_row(&id, Some(missing)).unwrap_err().contains(&missing.to_string()));
    set(&mut s, None, json!({"name":"Empty"})).unwrap();
    assert_eq!(view(&s).placement_row(&id, Some(house(&s, "Empty"))).unwrap(), view(&s).placement_row(&id, None).unwrap());
}

/// Houses change no fabrication output, legacy markers never match an offset, and a row fails
/// while an outline-attached origin has no outline to resolve against.
#[test]
fn houses_leave_fabrication_and_legacy_markers_alone() {
    let mut s = crate::board::track::tests::routed_fixture();
    crate::board::drc::tests::outline(&mut s);
    let [front, back] = crate::board::track::tests::copper_layers(&s)[..] else { panic!("two copper layers") };
    let y = 20 * MM;
    let route = [Point { x_nm: 5 * MM, y_nm: y }, Point { x_nm: 7 * MM, y_nm: y }, Point { x_nm: 9 * MM, y_nm: y }];
    crate::board::track::tests::route_with_vias(&mut s, front, &route, &[(1, back)]).unwrap();
    let before = s.execute(crate::EXPORT_FABRICATION, &json!({})).unwrap()["fabrication"].clone();
    let project = s.project().unwrap();
    let packages: Vec<_> = project
        .board
        .footprints()
        .iter()
        .map(|f| (f.library.clone().unwrap_or_else(|| LIBRARY.into()), f.definition.clone()))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(|(library, definition)| json!({"library":library,"definition":definition,"rotation":90,"centroid":{"x_nm":MM,"y_nm":MM}}))
        .collect();
    set(&mut s, None, json!({"name":"Everything","bottom_side":"viewed_from_bottom","packages":packages})).unwrap();
    assert_eq!(s.execute(crate::EXPORT_FABRICATION, &json!({})).unwrap()["fabrication"], before);
    // Format 22 carries the tracks, vias, outline, setup and accepted baseline beside the house.
    let documents = encode_documents(s.snapshot().unwrap()).unwrap();
    let saved: Value = serde_json::from_slice(&documents[&test_board_path(&documents)]).unwrap();
    assert_eq!(saved["format"], "pcbcraft-board/22");
    assert!(saved["tracks"].is_array() && saved["vias"].is_array() && saved["baseline"].is_object());
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);

    let marker = Footprint::new(1, 1, 1, Point { x_nm: 0, y_nm: 0 });
    let definition = crate::board::FOOTPRINT_DEFINITION;
    let house = House {
        format: FORMAT.into(),
        id: Uuid::from_u128(1),
        name: "Marker".into(),
        bottom_side: BottomSide::ViewedFromTop,
        packages: vec![PackageOffset { library: LIBRARY.into(), definition: definition.into(), rotation: 90, centroid: None }],
    };
    assert!(house.validate().is_ok());
    assert!(house.offset_for(&marker).is_none(), "a legacy marker has no library identity");

    let (mut s, [id, _, _]) = fixture();
    let board = s.project().unwrap().board_target().unwrap().board;
    s.apply(SET_PLACEMENT_ORIGIN, &json!({"board":board,"expected_revision":s.revision(),"origin":{"kind":"outline_lower_left"}})).unwrap();
    assert!(view(&s).placement_row(&id, None).is_err(), "no outline to measure from");
}

/// The command creates, replaces in place and removes houses, one undo step each, keeps
/// identities, ignores repeats and puts package offsets in canonical order.
#[test]
fn houses_are_edited_in_one_undo_step() {
    let (mut s, _) = fixture();
    let original = s.snapshot().unwrap();
    let packages = json!([
        {"library":LIBRARY,"definition":"resistor-0402","rotation":180},
        {"library":LIBRARY,"definition":"capacitor-0402","centroid":{"x_nm":MM,"y_nm":0}}
    ]);
    set(&mut s, None, json!({"name":"A","packages":packages})).unwrap();
    set(&mut s, None, json!({"name":"B","bottom_side":"viewed_from_bottom"})).unwrap();
    let houses = view(&s).assembly_houses().to_vec();
    assert_eq!(houses.iter().map(|house| house.name.as_str()).collect::<Vec<_>>(), ["A", "B"]);
    assert_eq!(houses[0].packages.iter().map(|p| p.definition.as_str()).collect::<Vec<_>>(), ["capacitor-0402", "resistor-0402"]);
    assert_eq!(houses[0].bottom_side, BottomSide::ViewedFromTop);
    let inspected = &s.inspect()["project"]["boards"][0]["assembly_houses"];
    assert_eq!(inspected[1], json!({"format":"pcbcraft-assembly-house/1","id":houses[1].id,"name":"B","bottom_side":"viewed_from_bottom"}));

    let revision = s.revision();
    set(&mut s, Some(houses[0].id), json!({"name":"A","packages":packages})).unwrap();
    assert_eq!(s.revision(), revision, "a repeat changes nothing");
    let two = s.snapshot().unwrap();
    set(&mut s, Some(houses[0].id), json!({"name":"A2","packages":[]})).unwrap();
    assert_eq!(view(&s).assembly_houses()[0].id, houses[0].id);
    assert!(view(&s).assembly_houses()[0].packages.is_empty());
    s.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&two, &s.snapshot().unwrap());
    set(&mut s, Some(houses[0].id), Value::Null).unwrap();
    assert_eq!(view(&s).assembly_houses().iter().map(|house| house.id).collect::<Vec<_>>(), [houses[1].id]);
    set(&mut s, Some(houses[1].id), Value::Null).unwrap();
    crate::assert_same_content(&original, &s.snapshot().unwrap());
}

/// Each request below is refused and leaves the project untouched. The last good value of
/// each edge is accepted at the end.
#[test]
fn bad_requests_change_nothing() {
    let (mut s, _) = fixture();
    set(&mut s, None, json!({"name":"Taken"})).unwrap();
    let taken = house(&s, "Taken");
    let before = s.snapshot().unwrap();
    let package = |extra: Value| {
        let mut entry = json!({"library":LIBRARY,"definition":"resistor-0402"});
        entry.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        json!({"name":"X","packages":[entry]})
    };
    let long = "n".repeat(MAX_NAME_CHARS + 1);
    let edge = MAX_OFFSET_NM + 1;
    for (house, profile) in [
        (None, Value::Null),
        (None, json!({})),
        (None, json!({"name":""})),
        (None, json!({"name":" padded"})),
        (None, json!({"name":"padded\u{feff}"})),
        (None, json!({"name":"tab\tinside"})),
        (None, json!({"name":long})),
        (None, json!({"name":"Taken"})),
        (None, json!({"name":"X","bottom_side":"mirrored"})),
        (None, json!({"name":"X","bottom_side":null})),
        (None, json!({"name":"X","packages":null})),
        (None, json!({"name":"X","extra":1})),
        (None, json!({"name":7})),
        (None, json!("X")),
        (None, package(json!({}))),
        (None, package(json!({"rotation":0}))),
        (None, package(json!({"rotation":360}))),
        (None, package(json!({"rotation":-1}))),
        (None, package(json!({"rotation":90.5}))),
        (None, package(json!({"centroid":{"x_nm":0,"y_nm":0}}))),
        (None, package(json!({"centroid":{"x_nm":edge,"y_nm":0}}))),
        (None, package(json!({"centroid":{"x_nm":0,"y_nm":-edge}}))),
        (None, package(json!({"centroid":{"x_nm":1}}))),
        (None, package(json!({"rotation":90,"centroid":null}))),
        (None, package(json!({"rotation":90,"revision":1}))),
        (None, package(json!({"rotation":90,"library":"Basic"}))),
        (None, package(json!({"rotation":90,"definition":""}))),
        (None, json!({"name":"X","packages":[{"library":LIBRARY,"definition":"a","rotation":1},{"library":LIBRARY,"definition":"a","rotation":2}]})),
        (Some(Uuid::nil()), json!({"name":"X"})),
        (Some(Uuid::from_u128(9)), json!({"name":"X"})),
        (Some(Uuid::from_u128(9)), Value::Null),
    ] {
        assert!(set(&mut s, house, profile.clone()).is_err(), "{house:?} {profile}");
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()), "{profile}");
    }
    let board = before.board_target().unwrap().board;
    let mut params = json!({"board":board,"expected_revision":s.revision(),"profile":{"name":"Y"},"house":null});
    assert!(s.apply(SET_ASSEMBLY_HOUSE, &params).is_err(), "a null house is refused, not read as a new one");
    params.as_object_mut().unwrap().remove("house");
    params["expected_revision"] = json!(s.revision() - 1);
    assert!(s.apply(SET_ASSEMBLY_HOUSE, &params).is_err());
    params["expected_revision"] = json!(s.revision());
    params.as_object_mut().unwrap().remove("board");
    assert!(s.apply(SET_ASSEMBLY_HOUSE, &params).is_err(), "the board is required");
    params["board"] = json!(Uuid::from_u128(5));
    assert!(s.apply(SET_ASSEMBLY_HOUSE, &params).is_err(), "another board is refused");
    params["board"] = json!(board);
    params.as_object_mut().unwrap().remove("profile");
    assert!(s.apply(SET_ASSEMBLY_HOUSE, &params).is_err(), "the profile is required");
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));

    let name = "n".repeat(MAX_NAME_CHARS);
    set(
        &mut s,
        None,
        json!({"name":name,"packages":[
            {"library":LIBRARY,"definition":"resistor-0402","rotation":359,"centroid":{"x_nm":MAX_OFFSET_NM,"y_nm":-MAX_OFFSET_NM}},
            {"library":LIBRARY,"definition":"capacitor-0402","rotation":1},
            {"library":LIBRARY,"definition":"led-0402","rotation":0,"centroid":{"x_nm":0,"y_nm":1}}
        ]}),
    )
    .unwrap();
    set(&mut s, Some(taken), json!({"name":"Taken inside"})).unwrap();
}

/// The largest package list and house count are accepted; one more is refused.
#[test]
fn profile_and_board_limits() {
    let packages = |count: usize| (0..count).map(|n| json!({"library":LIBRARY,"definition":format!("p{n}"),"rotation":90})).collect::<Vec<_>>();
    assert!(valid_profile(&json!({"name":"Full","packages":packages(MAX_PACKAGES)})));
    assert!(!valid_profile(&json!({"name":"Full","packages":packages(MAX_PACKAGES + 1)})));
    let houses = |count: usize| {
        (0..count)
            .map(|n| House {
                format: FORMAT.into(),
                id: Uuid::from_u128(n as u128 + 1),
                name: format!("H{n}"),
                bottom_side: BottomSide::ViewedFromTop,
                packages: Vec::new(),
            })
            .collect::<Vec<_>>()
    };
    assert!(validate_all(&houses(MAX_HOUSES)).is_ok());
    assert!(validate_all(&houses(MAX_HOUSES + 1)).is_err());
    let mut repeated = houses(2);
    repeated[1].id = repeated[0].id;
    assert!(validate_all(&repeated).is_err());

    let (mut s, _) = fixture();
    for n in 0..MAX_HOUSES {
        set(&mut s, None, json!({"name":format!("H{n}")})).unwrap();
    }
    let before = s.snapshot().unwrap();
    assert!(set(&mut s, None, json!({"name":"One more"})).is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
}

/// The package offsets of all houses on a board are bounded together: the last offset that
/// fits is accepted, the first beyond it is refused as invalid params before the edit applies,
/// and a saved board beyond it is refused on load.
#[test]
fn board_package_total_is_bounded() {
    let packages = |from: usize, count: usize| {
        (from..from + count).map(|n| json!({"library":LIBRARY,"definition":format!("p{n:05}"),"rotation":90})).collect::<Vec<_>>()
    };
    let half = MAX_BOARD_PACKAGES / 2;
    let (mut s, _) = fixture();
    set(&mut s, None, json!({"name":"First","packages":packages(0, half)})).unwrap();
    set(&mut s, None, json!({"name":"Second","packages":packages(half, MAX_BOARD_PACKAGES - half - 1)})).unwrap();
    set(&mut s, None, json!({"name":"Third","packages":packages(0, 1)})).unwrap();
    let full = s.snapshot().unwrap();
    assert_eq!(full.board.assembly_houses.iter().map(|house| house.packages.len()).sum::<usize>(), MAX_BOARD_PACKAGES);
    let refused = set(&mut s, None, json!({"name":"Fourth","packages":packages(0, 1)}));
    assert!(matches!(refused, Err(CommandError::InvalidParams(ref message)) if message.contains("together")), "{refused:?}");
    assert!(Arc::ptr_eq(&full, &s.snapshot().unwrap()));
    let third = full.board.assembly_houses[2].id;
    assert!(set(&mut s, Some(third), json!({"name":"Third","packages":packages(0, 2)})).is_err());
    assert!(Arc::ptr_eq(&full, &s.snapshot().unwrap()));

    let documents = encode_documents(full.clone()).unwrap();
    assert!(decode_documents(&documents).is_ok());
    let path = test_board_path(&documents);
    let mut saved: Value = serde_json::from_slice(&documents[&path]).unwrap();
    saved["assembly_houses"][2]["packages"].as_array_mut().unwrap().push(json!({"library":LIBRARY,"definition":"z-extra","rotation":90}));
    let mut over = documents.clone();
    over.insert(path, serde_json::to_vec(&saved).unwrap());
    assert!(decode_documents(&over).is_err(), "a saved board beyond the total is refused");
}

/// The largest board the limits allow (every house, every offset, the longest names and ids and
/// the widest values) serializes far below the board document limit.
#[test]
fn largest_allowed_houses_fit_one_board_document() {
    let long_id = |n: usize| format!("{n:05}{}", "x".repeat(59));
    let per_house = MAX_BOARD_PACKAGES / MAX_HOUSES;
    let houses = (0..MAX_HOUSES)
        .map(|h| House {
            format: FORMAT.into(),
            id: Uuid::from_u128(u128::MAX - h as u128),
            name: format!("{h:02}{}", "\u{10ffff}".repeat(MAX_NAME_CHARS - 2)),
            bottom_side: BottomSide::ViewedFromBottom,
            packages: (0..per_house)
                .map(|n| PackageOffset {
                    library: long_id(h),
                    definition: long_id(n),
                    rotation: 359,
                    centroid: Some(crate::schematic::Point { x_nm: -MAX_OFFSET_NM, y_nm: -MAX_OFFSET_NM }),
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    assert_eq!(houses.iter().map(|house| house.packages.len()).sum::<usize>(), MAX_BOARD_PACKAGES);
    validate_all(&houses).unwrap();
    let bytes = serde_json::to_vec_pretty(&houses).unwrap().len();
    assert!(bytes < crate::persistence::MAX_DOCUMENT_BYTES / 8, "{bytes} bytes");
}

/// Format 22 is written only while a house exists, carries an origin, rules and every earlier
/// field, reopens to the same bytes, and removing the houses restores the earlier encoding.
#[test]
fn house_format_round_trips_and_reset_restores_previous_bytes() {
    let (mut s, _) = placed();
    let original = encode_documents(s.snapshot().unwrap()).unwrap();
    let path = test_board_path(&original);
    let earlier: Value = serde_json::from_slice(&original[&path]).unwrap();
    assert_eq!(earlier["format"], "pcbcraft-board/21");
    assert!(earlier.get("assembly_houses").is_none());
    let board = s.project().unwrap().board_target().unwrap().board;
    s.apply(SET_RULES, &json!({"board":board,"expected_revision":s.revision(),"rules":crate::board::rules::Values::default()})).unwrap();
    set(
        &mut s,
        None,
        json!({"name":"Reel","bottom_side":"viewed_from_bottom","packages":[
            {"library":LIBRARY,"definition":"resistor-0402","rotation":180,"centroid":{"x_nm":1,"y_nm":-2}},
            {"library":LIBRARY,"definition":"sot23","rotation":270}
        ]}),
    )
    .unwrap();
    set(&mut s, None, json!({"name":"Plain"})).unwrap();
    let documents = encode_documents(s.snapshot().unwrap()).unwrap();
    let saved: Value = serde_json::from_slice(&documents[&path]).unwrap();
    assert_eq!(saved["format"], "pcbcraft-board/22");
    let reel = &saved["assembly_houses"][0];
    assert_eq!(reel["packages"][0], json!({"library":LIBRARY,"definition":"resistor-0402","rotation":180,"centroid":{"x_nm":1,"y_nm":-2}}));
    assert_eq!(reel["packages"][1], json!({"library":LIBRARY,"definition":"sot23","rotation":270}));
    assert!(saved["assembly_houses"][1].get("packages").is_none(), "empty package lists are omitted");
    assert!(saved["rules"].is_object() && saved["placement_origin"].is_object());
    let reopened = decode_documents(&documents).unwrap();
    assert_eq!(reopened.board.assembly_houses, s.project().unwrap().board.assembly_houses);
    assert_eq!(encode_documents(reopened).unwrap(), documents);

    let text = String::from_utf8(documents[&path].clone()).unwrap();
    let refused = |from: &str, to: &str| {
        assert!(text.contains(from), "{from}");
        let mut altered = documents.clone();
        altered.insert(path.clone(), text.replacen(from, to, 1).into_bytes());
        decode_documents(&altered).is_err()
    };
    assert!(refused("pcbcraft-board/22", "pcbcraft-board/21"), "older headers refuse houses");
    assert!(refused("\"pcbcraft-assembly-house/1\"", "\"pcbcraft-assembly-house/2\""));
    assert!(refused("\"bottom_side\":\"viewed_from_bottom\",", ""), "the saved convention is required");
    assert!(refused("\"rotation\":270", "\"rotation\":360"));
    assert!(refused("\"rotation\":270", "\"rotation\":0"), "an entry that changes nothing");
    assert!(refused("\"rotation\":180,", "\"rotation\":0,"), "an explicit zero is never written");
    assert!(refused("\"rotation\":180,", "\"rotation\":null,"), "a null rotation is never written");
    let reel_id = house(&s, "Reel").to_string();
    assert!(refused(&reel_id, &Uuid::nil().to_string()), "nil identities are refused");
    assert!(refused(&reel_id, &house(&s, "Plain").to_string()), "identities must stay unique");
    assert!(refused("\"rotation\":270", "\"rotation\":270,\"revision\":1"), "unknown package fields are refused");
    assert!(refused("\"centroid\":{\"x_nm\":1,\"y_nm\":-2}", "\"centroid\":null"));
    assert!(refused("\"definition\":\"sot23\"", "\"definition\":\"capacitor-0402\""), "packages must stay in order");
    assert!(refused("\"name\":\"Plain\"", "\"name\":\"Reel\""), "names must stay unique");
    assert!(refused("\"name\":\"Plain\"", "\"name\":\"Plain\",\"note\":1"), "unknown fields are refused");
    assert!(refused("\"name\":\"Plain\"", "\"name\":\"Plain\",\"packages\":[]"), "an empty package list is never written");
    let houses = serde_json::to_string(&s.project().unwrap().board.assembly_houses).unwrap();
    assert!(refused(&houses, "[]"), "an empty house list is never written");
    assert!(refused(&houses, "null"));

    for id in [house(&s, "Reel"), house(&s, "Plain")] {
        set(&mut s, Some(id), Value::Null).unwrap();
    }
    s.apply(SET_RULES, &json!({"board":board,"expected_revision":s.revision(),"rules":null})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), original);
    let mut missing = original.clone();
    let text = String::from_utf8(original[&path].clone()).unwrap();
    missing.insert(path, text.replacen("pcbcraft-board/21", "pcbcraft-board/22", 1).into_bytes());
    assert!(decode_documents(&missing).is_err(), "format 22 requires a house");
}

/// A house without an origin or rules still writes format 22 and reopens to the same bytes.
#[test]
fn house_without_origin_round_trips() {
    let (mut s, _) = fixture();
    set(&mut s, None, json!({"name":"Only"})).unwrap();
    let documents = encode_documents(s.snapshot().unwrap()).unwrap();
    let path = test_board_path(&documents);
    let saved: Value = serde_json::from_slice(&documents[&path]).unwrap();
    assert_eq!(saved["format"], "pcbcraft-board/22");
    assert!(saved.get("placement_origin").is_none() && saved.get("rules").is_none());
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
}
