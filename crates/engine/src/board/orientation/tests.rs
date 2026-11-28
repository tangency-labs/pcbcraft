use super::*;
use crate::{
    PLACE_RESISTOR,
    board::{
        assignment::{ASSIGN_FOOTPRINT, PLACE_ASSIGNED_FOOTPRINT},
        definition::{CURRENT_FORMAT, LEGACY_FORMAT, MIXED_HOLES_FORMAT, NPTH_FORMAT, ORIENTATION_FORMAT, basic_components, read_saved, write_saved},
    },
    library::sources::LOAD,
    persistence::{decode_documents, encode_documents, test_board_path},
};

const MM: i64 = 1_000_000;

/// Assert that `result` was refused, and refused for the reason `needle` names.
#[track_caller]
fn refused<T: std::fmt::Debug, E: std::fmt::Display>(result: Result<T, E>, needle: &str) {
    let message = result.expect_err(needle).to_string();
    assert!(message.contains(needle), "refused with {message:?}, expected {needle:?}");
}
const LIBRARY: &str = "orientation-test";

/// The geometry check as a board with the default silkscreen settings prints the package.
fn orientation_marked(definition: &FootprintDefinition) -> bool {
    super::orientation_marked(definition, Settings::default())
}

/// A test package copied from a bundled footprint under a new library identity, with an
/// optional orientation statement. Use the current format 13 when adding a statement (format 7
/// also supports it); without one the bundled package's historical format is kept.
fn package(base: &str, id: &str, orientation: Option<Value>) -> Value {
    let mut definition = serde_json::to_value(basic_components(base).unwrap().as_ref()).unwrap();
    definition["library"] = json!(LIBRARY);
    definition["id"] = json!(id);
    definition["revision"] = json!(1);
    definition["name"] = json!(format!("Test {id}"));
    if let Some(orientation) = orientation {
        definition["format"] = json!(CURRENT_FORMAT);
        definition["orientation"] = orientation;
    }
    definition
}

/// A board with one two-pin component per package, assigned and placed through the shared
/// library, assignment and placement commands. Bundled packages are named by id alone.
fn fixture(packages: &[Value]) -> Session {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    let mut documents = BTreeMap::from([(
        "library.json".to_owned(),
        json!({"format":"pcbcraft-library/1","id":LIBRARY,"name":"Orientation test","license":"MIT"}).to_string(),
    )]);
    for definition in packages.iter().filter(|d| d.is_object()) {
        documents.insert(format!("footprints/{}.json", definition["id"].as_str().unwrap()), definition.to_string());
    }
    s.execute(LOAD, &json!({"source":"orientation-test","documents":documents})).unwrap();
    for index in 0..packages.len() {
        s.apply(PLACE_RESISTOR, &json!({"x_nm":index as i64 * 10 * MM,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    }
    let components: Vec<_> = s.project().unwrap().symbols.iter().map(|symbol| symbol.component).collect();
    for (index, (component, definition)) in components.iter().zip(packages).enumerate() {
        let (library, id, revision) = match definition.as_str() {
            Some(bundled) => {
                let definition = basic_components(bundled).unwrap();
                (definition.library.clone(), definition.id.clone(), u64::from(definition.revision))
            }
            None => (LIBRARY.to_owned(), definition["id"].as_str().unwrap().to_owned(), definition["revision"].as_u64().unwrap()),
        };
        let component = format!("component-{component}");
        s.apply(
            ASSIGN_FOOTPRINT,
            &json!({"component":component,"library":library,"definition":id,"definition_revision":revision,
                "mapping":{"1":"1","2":"2"},"expected_revision":s.revision()}),
        )
        .unwrap();
        s.apply(
            PLACE_ASSIGNED_FOOTPRINT,
            &json!({"component":component,"x_nm":index as i64 * 8 * MM,"y_nm":2 * MM,"expected_revision":s.revision()}),
        )
        .unwrap();
    }
    s
}

/// Run the check through `execute` and return its findings, asserting it changed nothing.
fn findings(s: &mut Session) -> Vec<Value> {
    let revision = s.revision();
    let report = s.execute(CHECK_ASSEMBLY, &json!({})).unwrap();
    assert_eq!(report["revision"], json!(revision));
    assert_eq!(s.revision(), revision, "the check edits nothing");
    report["assembly_check"]["findings"].as_array().unwrap().clone()
}

/// Done test: a polarized two-pad diode package whose lands and silkscreen are symmetric under
/// a half turn, with no stated zero orientation (IPC-7351 sections 7.3.1 and 16), raises one
/// warning naming its placement and package. A plain resistor beside it raises none.
#[test]
fn unmarked_polarized_diode_raises_one_finding() {
    let diode = package("resistor-0402", "diode-0402", Some(json!({"polarized":true,"pin_1":"1"})));
    let mut s = fixture(&[diode, json!("resistor-0402")]);
    let found = findings(&mut s);
    assert_eq!(found.len(), 1, "{found:#?}");
    let finding = &found[0];
    let placed: Vec<_> =
        crate::board::track::tests::view(&s).footprints().map(|f| (f.id, f.reference.into_owned(), f.definition.into_owned())).collect();
    let (id, reference, _) = placed.iter().find(|(.., definition)| definition == "diode-0402").unwrap();
    assert_eq!(finding["objects"], json!([{"id":id,"kind":"footprint","role":"placement","reference":reference}]));
    assert_eq!(finding["package"], json!({"library":LIBRARY,"id":"diode-0402","revision":1,"name":"Test diode-0402"}));
    assert_eq!(finding["check_kind"], json!(CHECK_KIND));
    assert_eq!(finding["rule_id"], json!(RULE_ID));
    assert_eq!(finding["severity"], json!("warning"));
    assert!(finding["rule_source"].as_str().unwrap().contains("section 16 (zero component orientations"));
    assert!(finding["message"].as_str().unwrap().contains(reference.as_str()));
    for field in ["key", "fingerprint"] {
        let text = finding[field].as_str().unwrap();
        assert!(text.len() == 32 && text.bytes().all(|b| b.is_ascii_hexdigit()), "{field}: {text}");
    }
    assert_eq!(finding["locations"][0], json!({"point":{"x_nm":0,"y_nm":2 * MM}}));
    assert_eq!(findings(&mut s), found, "the report is repeatable");
}

/// A read-only session admits the check and gets the editing session's report. Through both
/// `execute` and `apply`, the project, revision and history stay exactly as they were.
#[test]
fn read_only_session_runs_the_check_without_changing_anything() {
    let diode = package("resistor-0402", "diode-0402", Some(json!({"polarized":true,"pin_1":"1"})));
    let mut editor = fixture(&[diode]);
    let expected = editor.execute(CHECK_ASSEMBLY, &json!({})).unwrap();
    assert_eq!(expected["assembly_check"]["findings"].as_array().unwrap().len(), 1, "{expected:#}");
    let project = editor.snapshot().unwrap();
    let documents = encode_documents(project.clone()).unwrap();
    let board = project.board_target().unwrap().board;
    let mut reader = Session::with_scope(crate::SessionScope::ReadOnly);
    reader.open_snapshot(project.clone());
    let before = reader.inspect();
    let revision = reader.revision();
    let root = reader.history_root_revision();
    for params in [json!({}), json!({"board":board})] {
        let report = reader.execute(CHECK_ASSEMBLY, &params).unwrap();
        assert_eq!(report["revision"], json!(revision));
        assert_eq!(report["assembly_check"], expected["assembly_check"]);
        assert_eq!(reader.apply(CHECK_ASSEMBLY, &params).unwrap(), revision);
    }
    assert_eq!(reader.revision(), revision);
    assert_eq!(reader.history_root_revision(), root);
    assert!(reader.undo.is_empty() && reader.redo.is_empty(), "no history entry is recorded");
    assert!(!reader.available("edit.undo") && !reader.available("edit.redo"));
    assert_eq!(reader.inspect(), before);
    assert!(std::sync::Arc::ptr_eq(&project, &reader.snapshot().unwrap()));
    assert_eq!(encode_documents(reader.snapshot().unwrap()).unwrap(), documents);
}

/// A circular land and a rotated equal-dimension oval are indistinguishable in copper.
#[test]
fn equivalent_circular_lands_still_warn() {
    let mut diode = package("resistor-0402", "circular-lands", Some(json!({"polarized":true})));
    let pads = diode["pads"].as_array_mut().unwrap();
    for (pad, (shape, rotation)) in pads.iter_mut().zip([("circle", 0), ("oval", 37)]) {
        pad["shape"] = json!(shape);
        pad["rotation"] = json!(rotation);
        pad.as_object_mut().unwrap().remove("corner_radius_nm");
    }
    let mut s = fixture(&[diode]);
    let found = findings(&mut s);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0]["package"]["id"], "circular-lands");
}

/// A valid, off-centre path with coincident endpoints has no ink and cannot clear the warning.
#[test]
fn zero_length_silkscreen_still_warns() {
    let mut diode = package("resistor-0402", "empty-silk", Some(json!({"polarized":true})));
    let p = json!({"x_nm":0,"y_nm":900_000});
    diode["silkscreen"].as_array_mut().unwrap().push(json!({"points":[&p,&p],"closed":false}));
    let mut s = fixture(&[diode]);
    let found = findings(&mut s);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0]["package"]["id"], "empty-silk");
}

/// Duplicating a silkscreen or artwork stroke in place, or splitting it into touching or
/// overlapping collinear segments with the same layer and stroke style, keeps the warning;
/// a genuinely asymmetric mark, different stroke width or different layer still clears it.
#[test]
fn duplicate_and_collinear_strokes_still_warn_while_asymmetric_marks_pass() {
    let line = |purpose: &str, stroke_nm: i64, x1: i64, y1: i64, x2: i64, y2: i64| {
        json!({
            "purpose": purpose,
            "graphic": {
                "geometry": {"type":"line","from":{"x_nm":x1,"y_nm":y1},"to":{"x_nm":x2,"y_nm":y2}},
                "stroke_nm": stroke_nm,
                "fill": "none",
            }
        })
    };

    let mut duplicated = package("resistor-0402", "duplicate-silk", Some(json!({"polarized":true})));
    let first = duplicated["silkscreen"][0].clone();
    duplicated["silkscreen"].as_array_mut().unwrap().push(first);

    let mut split = package("resistor-0402", "split-silk", Some(json!({"polarized":true})));
    split["silkscreen"][0]["points"] = json!([
        {"x_nm":-400_000,"y_nm":-650_000},
        {"x_nm":0,"y_nm":-650_000},
        {"x_nm":400_000,"y_nm":-650_000},
    ]);

    let mut overlapped = package("resistor-0402", "overlap-silk", Some(json!({"polarized":true})));
    overlapped["silkscreen"][0]["points"] = json!([
        {"x_nm":-400_000,"y_nm":-650_000},
        {"x_nm":100_000,"y_nm":-650_000},
    ]);
    overlapped["silkscreen"].as_array_mut().unwrap().push(json!({
        "points": [{"x_nm":400_000,"y_nm":-650_000}, {"x_nm":-100_000,"y_nm":-650_000}],
        "closed": false,
    }));

    let mut artwork_equiv = package("resistor-0402", "artwork-equiv", Some(json!({"polarized":true})));
    artwork_equiv.as_object_mut().unwrap().remove("silkscreen");
    artwork_equiv["artwork"] = json!([
        line("silkscreen", 150_000, -400_000, -650_000, 400_000, -650_000),
        line("silkscreen", 150_000, 400_000, -650_000, -400_000, -650_000),
        line("silkscreen", 150_000, -400_000, 650_000, 0, 650_000),
        line("silkscreen", 150_000, 0, 650_000, 400_000, 650_000),
    ]);

    let mut asymmetric = package("resistor-0402", "asymmetric-bar", Some(json!({"polarized":true})));
    asymmetric["silkscreen"].as_array_mut().unwrap().push(json!({
        "points": [{"x_nm":-400_000,"y_nm":-650_000}, {"x_nm":-400_000,"y_nm":650_000}],
        "closed": false,
    }));

    let mut different_stroke = package("resistor-0402", "different-stroke", Some(json!({"polarized":true})));
    different_stroke.as_object_mut().unwrap().remove("silkscreen");
    different_stroke["artwork"] =
        json!(
            [line("silkscreen", 150_000, -400_000, -650_000, 400_000, -650_000), line("silkscreen", 250_000, -400_000, 650_000, 400_000, 650_000),]
        );

    let mut different_layer = package("resistor-0402", "different-layer", Some(json!({"polarized":true})));
    different_layer.as_object_mut().unwrap().remove("silkscreen");
    different_layer["artwork"] = json!([
        line("silkscreen", 150_000, -400_000, -650_000, 400_000, -650_000),
        line("fabrication", 150_000, -400_000, 650_000, 400_000, 650_000),
    ]);

    let mut s = fixture(&[duplicated, split, overlapped, artwork_equiv, asymmetric, different_stroke, different_layer]);
    let found = findings(&mut s);
    let ids: Vec<_> = found.iter().map(|f| f["package"]["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["artwork-equiv", "duplicate-silk", "overlap-silk", "split-silk"], "{found:#?}");
}

/// A centerline silkscreen stroke uses the board's effective silkscreen stroke width when compared
/// with an explicit silkscreen artwork stroke under a half turn, both at the board's default
/// 0.15 mm width and after changing the board's silkscreen settings.
#[test]
fn centerline_silkscreen_uses_effective_board_stroke_width() {
    let make_pair = |id: &str, artwork_stroke_nm: i64| {
        let mut diode = package("resistor-0402", id, Some(json!({"polarized":true})));
        diode["silkscreen"] = json!([{
            "points": [{"x_nm": -400_000, "y_nm": -800_000}, {"x_nm": 400_000, "y_nm": -800_000}],
            "closed": false,
        }]);
        diode["artwork"] = json!([{
            "purpose": "silkscreen",
            "graphic": {
                "geometry": {"type": "line", "from": {"x_nm": -400_000, "y_nm": 800_000}, "to": {"x_nm": 400_000, "y_nm": 800_000}},
                "stroke_nm": artwork_stroke_nm,
                "fill": "none",
            }
        }]);
        diode
    };

    let mut s = fixture(&[make_pair("match-default-width", 150_000), make_pair("match-custom-width", 200_000)]);
    let initial: Vec<_> = findings(&mut s).into_iter().map(|f| f["package"]["id"].as_str().unwrap().to_owned()).collect();
    assert_eq!(initial, vec!["match-default-width"]);

    s.apply(
        crate::SET_SILKSCREEN,
        &json!({
            "settings": {"text_height_nm": 1_000_000, "stroke_width_nm": 200_000, "pad_clearance_nm": 250_000},
            "expected_revision": s.revision(),
        }),
    )
    .unwrap();
    let updated: Vec<_> = findings(&mut s).into_iter().map(|f| f["package"]["id"].as_str().unwrap().to_owned()).collect();
    assert_eq!(updated, vec!["match-custom-width"]);
}

/// Half-turned silkscreen rectangles leave orientation ambiguous only when their printed
/// widths match: the legacy rectangle uses Gerber's 0.12 mm fallback, not the board setting.
#[test]
fn legacy_silkscreen_rectangle_uses_gerber_fallback_width() {
    let make_pair = |id: &str, artwork_stroke_nm: i64| {
        let mut diode = package("resistor-0402", id, Some(json!({"polarized":true})));
        diode["silkscreen"] = json!([]);
        diode["graphics"] = json!([{
            "purpose": "silkscreen", "center": {"x_nm": 0, "y_nm": 2 * MM},
            "width_nm": 800_000, "height_nm": 400_000, "rotation": 0,
        }]);
        diode["artwork"] = json!([{
            "purpose": "silkscreen",
            "graphic": {
                "geometry": {"type": "rectangle", "min": {"x_nm": -400_000, "y_nm": -2_200_000}, "max": {"x_nm": 400_000, "y_nm": -1_800_000}},
                "stroke_nm": artwork_stroke_nm,
                "fill": "none",
            }
        }]);
        diode
    };
    let mut s = fixture(&[make_pair("gerber-width", 120_000), make_pair("board-width", 150_000)]);
    let found = findings(&mut s);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0]["package"]["id"], "gerber-width");
}

/// Silkscreen counts only as the board prints it. A stroke lying wholly inside pad 1 is clipped
/// away by pad clearance, so it cannot clear the warning; the same stroke beyond the clearance
/// does. The board's own setting decides: a stroke just outside pad 1 is clipped by the default
/// clearance and marks the part once a smaller clearance leaves part of it printed.
#[test]
fn silkscreen_clipped_by_pad_clearance_does_not_clear_the_warning() {
    // Pad 1 spans x from -825 to -225 um and y from -300 to 300 um.
    let stroke = |id: &str, y1: i64, y2: i64| {
        let mut diode = package("resistor-0402", id, Some(json!({"polarized":true})));
        diode["silkscreen"].as_array_mut().unwrap().push(json!({"points":[{"x_nm":-700_000,"y_nm":y1},{"x_nm":-700_000,"y_nm":y2}],"closed":false}));
        diode
    };
    let mut s =
        fixture(&[stroke("inside-pad", -100_000, 100_000), stroke("near-pad", 320_000, 500_000), stroke("beyond-clearance", 700_000, 900_000)]);
    // The board view prints the two bundled lines of each package, plus only the visible strokes.
    let printed = |s: &Session| -> Vec<usize> {
        let project = s.project().unwrap();
        let view = project.board_view(project.board_target().unwrap()).unwrap();
        view.footprints()
            .map(|footprint| footprint.graphics.iter().filter(|g| g.purpose == GraphicPurpose::Silkscreen && !g.reference).count())
            .collect()
    };
    let ids = |s: &mut Session| -> Vec<String> { findings(s).iter().map(|finding| finding["package"]["id"].as_str().unwrap().to_owned()).collect() };
    assert_eq!(printed(&s), [2, 2, 3]);
    assert_eq!(ids(&mut s), ["inside-pad", "near-pad"]);
    let style = Settings { pad_clearance_nm: 0, ..Settings::default() };
    s.apply(crate::SET_SILKSCREEN, &json!({"settings":style,"expected_revision":s.revision()})).unwrap();
    assert_eq!(printed(&s), [2, 3, 3]);
    assert_eq!(ids(&mut s), ["inside-pad"]);
}

/// A stated zero orientation, a silkscreen polarity mark or no polarity at all clears it.
#[test]
fn stated_zero_marks_and_unpolarized_packages_raise_nothing() {
    let stated = package("resistor-0402", "diode-0402", Some(json!({"polarized":true,"zero":"ipc-7351"})));
    let marked = package("sod123", "diode-sod123", Some(json!({"polarized":true})));
    let plain = package("resistor-0402", "plain-0402", Some(json!({"polarized":false})));
    let mut s = fixture(&[stated, marked, plain, json!("resistor-0402")]);
    assert_eq!(findings(&mut s), Vec::<Value>::new());
}

/// Two placements of one unmarked package give one finding naming both, in board order.
#[test]
fn one_finding_per_package() {
    let diode = package("resistor-0402", "diode-0402", Some(json!({"polarized":true})));
    let mut s = fixture(&[diode.clone(), json!("capacitor-0402"), diode]);
    let found = findings(&mut s);
    assert_eq!(found.len(), 1);
    let ids: Vec<_> = found[0]["objects"].as_array().unwrap().iter().map(|o| o["id"].as_str().unwrap().to_owned()).collect();
    assert_eq!(ids.len(), 2);
    assert_eq!(found[0]["locations"].as_array().unwrap().len(), 3, "two points and one box");
}

/// The check refuses an unknown board, a malformed board parameter and unknown parameters,
/// and reports the board it checked.
#[test]
fn check_validates_its_board() {
    let mut s = fixture(&[json!("resistor-0402")]);
    refused(s.execute(CHECK_ASSEMBLY, &json!({"board":uuid::Uuid::from_u128(9)})), "unknown board ID");
    for (params, reason) in [
        (json!({"board":5}), "`board` must identify a board by UUID"),
        (json!({"board":""}), "`board` must identify a board by UUID"),
        (json!({"boards":[]}), "has no parameter `boards`"),
        (json!([]), "takes an object of parameters"),
    ] {
        refused(s.execute(CHECK_ASSEMBLY, &params), reason);
    }
    let board = s.project().unwrap().board_target().unwrap().board;
    assert_eq!(s.execute(CHECK_ASSEMBLY, &json!({"board":board})).unwrap()["assembly_check"]["board"]["board"], json!(board));
}

/// The bundled packages: polarized parts carry a mark, chip resistors and capacitors do not.
#[test]
fn bundled_marks() {
    for (id, marked) in [
        ("sod123", true),
        ("led-0402", true),
        ("header-1x02-254", true),
        ("sot23", true),
        ("resistor-0402", false),
        ("resistor-0603", false),
        ("capacitor-0402", false),
    ] {
        assert_eq!(orientation_marked(&basic_components(id).unwrap()), marked, "{id}");
    }
}

/// A square of four identical lands is symmetric under every quarter turn. A symmetric ring
/// marks nothing; a dot, a half arc, a turned label, a corner bar, a fabrication dot or a bar
/// of another stroke does. A courtyard dot does not.
#[test]
fn quarter_turn_symmetry() {
    let mut definition = basic_components("resistor-0402").unwrap().as_ref().clone();
    let mut land = definition.pads[0].clone();
    land.height_nm = land.width_nm;
    definition.pads = [(1, 0), (0, 1), (-1, 0), (0, -1)]
        .into_iter()
        .enumerate()
        .map(|(n, (x, y))| super::super::definition::RectPad {
            number: (n + 1).to_string(),
            center: crate::schematic::Point { x_nm: x * 2 * MM, y_nm: y * 2 * MM },
            ..land.clone()
        })
        .collect();
    definition.silkscreen.clear();
    definition.graphics.clear();
    definition.artwork.clear();
    assert!(!orientation_marked(&definition));
    let graphic = |geometry| super::super::definition::Artwork {
        purpose: GraphicPurpose::Silkscreen,
        graphic: crate::library_graphics::Graphic { geometry, stroke_nm: 100_000, fill: crate::library_graphics::Fill::None },
    };
    let point = |x: i64, y: i64| crate::schematic::Point { x_nm: x, y_nm: y };
    let ring = graphic(Geometry::Circle { center: point(0, 0), radius_nm: 3 * MM });
    let full = graphic(Geometry::Arc { center: point(0, 0), radius_nm: 4 * MM, start_degrees: 45, sweep_degrees: -360 });
    let half = graphic(Geometry::Arc { center: point(0, 0), radius_nm: 4 * MM, start_degrees: 0, sweep_degrees: 180 });
    let dot = graphic(Geometry::Circle { center: point(MM, MM), radius_nm: MM / 4 });
    let opposite_dot = graphic(Geometry::Circle { center: point(-MM, -MM), radius_nm: MM / 4 });
    let label = graphic(Geometry::Text { at: point(0, 0), text: "+".into(), height_nm: MM, rotation: 0, align: TextAlign::Middle });
    let frame = graphic(Geometry::Rectangle { min: point(-3 * MM, -3 * MM), max: point(3 * MM, 3 * MM) });
    let overlapped_edge = graphic(Geometry::Line { from: point(-3 * MM, -3 * MM), to: point(-3 * MM, -2 * MM) });
    let bar = graphic(Geometry::Line { from: point(-3 * MM, -2 * MM), to: point(-2 * MM, -3 * MM) });
    for (artwork, marked) in [
        (vec![ring.clone()], false),
        (vec![ring.clone(), full], false),
        (vec![frame.clone()], false),
        (vec![frame.clone(), overlapped_edge], false),
        (vec![ring.clone(), half], true),
        (vec![ring.clone(), dot.clone(), opposite_dot], false),
        (vec![ring.clone(), dot], true),
        (vec![frame.clone(), label], true),
        (vec![frame, bar], true),
    ] {
        definition.artwork = artwork.clone();
        assert_eq!(orientation_marked(&definition), marked, "{artwork:?}");
    }
    // A pin 1 dot on the fabrication layer marks the part, as the assembly drawing shows it;
    // the same dot on the courtyard does not.
    for (purpose, marked) in [(GraphicPurpose::Fabrication, true), (GraphicPurpose::Courtyard, false)] {
        definition.artwork =
            vec![super::super::definition::Artwork { purpose, ..graphic(Geometry::Circle { center: point(MM, MM), radius_nm: MM / 4 }) }];
        assert_eq!(orientation_marked(&definition), marked, "{purpose:?}");
    }
    // A symmetric ring on the fabrication layer beside the silkscreen one marks nothing either.
    definition.artwork = vec![super::super::definition::Artwork { purpose: GraphicPurpose::Fabrication, ..ring.clone() }, ring.clone()];
    assert!(!orientation_marked(&definition));
    // Four bars that turn onto each other mark the part only when one is drawn differently.
    let bar = |x: i64, y: i64, stroke_nm: i64| super::super::definition::Artwork {
        purpose: GraphicPurpose::Silkscreen,
        graphic: crate::library_graphics::Graphic {
            geometry: Geometry::Line { from: point(3 * MM * x - MM * y, 3 * MM * y - MM * x), to: point(3 * MM * x + MM * y, 3 * MM * y + MM * x) },
            stroke_nm,
            fill: crate::library_graphics::Fill::None,
        },
    };
    definition.artwork = [(1, 0), (0, 1), (-1, 0), (0, -1)].into_iter().map(|(x, y)| bar(x, y, 100_000)).collect();
    assert!(!orientation_marked(&definition), "four equal bars");
    definition.artwork[0].graphic.stroke_nm = 300_000;
    assert!(orientation_marked(&definition), "one thicker bar");
    // One land of a different size breaks the symmetry on its own.
    definition.pads[0].width_nm -= 10_000;
    assert!(orientation_marked(&definition));
}

/// A half-nanometre centroid stays exact, and a land rotated by a quarter turn matches a
/// land of swapped size.
#[test]
fn exact_half_turn_and_rotated_lands() {
    let mut definition = basic_components("resistor-0402").unwrap().as_ref().clone();
    definition.silkscreen.clear();
    definition.graphics.clear();
    definition.artwork.clear();
    definition.pads[1].center.x_nm += 1;
    assert!(!orientation_marked(&definition), "a centroid at a half nanometre is still exact");
    let pad = &mut definition.pads[1];
    pad.width_nm += 2;
    assert!(orientation_marked(&definition));
    let pad = &mut definition.pads[1];
    pad.width_nm -= 2;
    (pad.width_nm, pad.height_nm) = (pad.width_nm, pad.width_nm + 200_000);
    let other = &mut definition.pads[0];
    (other.height_nm, other.width_nm, other.rotation) = (other.width_nm, other.width_nm + 200_000, 90);
    assert!(!orientation_marked(&definition), "a quarter-turned land of swapped size is the same copper");
    definition.pads.clear();
    assert!(!orientation_marked(&definition));
}

/// Readers refuse a malformed orientation statement and a format that does not match it;
/// definitions without one keep format 1, and the withdrawn format 2 is unknown.
#[test]
fn orientation_reader() {
    let good = package("resistor-0402", "diode-0402", Some(json!({"polarized":true,"pin_1":"2","zero":"ipc-7351"})));
    let parsed = read_saved(good.to_string().as_bytes()).unwrap();
    assert_eq!(parsed.orientation.as_ref().unwrap().pin_1.as_deref(), Some("2"));
    assert_eq!(serde_json::to_value(parsed.as_ref()).unwrap(), good, "round trip is exact");
    let plain = package("resistor-0402", "plain-0402", None);
    assert_eq!(plain["format"], json!(LEGACY_FORMAT));
    assert!(read_saved(plain.to_string().as_bytes()).is_ok());
    let mut wrong = Vec::new();
    for (pointer, value, reason) in [
        ("/orientation/side", json!("top"), "unknown field `side`"),
        ("/orientation/zero", json!("ipc-7350"), "unknown variant `ipc-7350`"),
        ("/orientation/zero", Value::Null, "expected a string"),
        ("/orientation/pin_1", json!("3"), "orientation pin_1 names no pad of this footprint"),
        ("/orientation/pin_1", Value::Null, "invalid type: null"),
        ("/orientation/polarized", Value::Null, "invalid type: null"),
        ("/orientation/polarized", json!("yes"), "invalid type: string \"yes\""),
        ("/orientation/pin_1", json!(1), "invalid type: integer `1`"),
        ("/orientation/zero", json!(0), "expected a string"),
        ("/orientation/zero", json!({"ipc-7351":null}), "expected a string"),
        ("/orientation", Value::Null, "expected an orientation object"),
        ("/orientation", json!([true, "1", "ipc-7351"]), "expected an orientation object"),
        ("/orientation", json!(true), "expected an orientation object"),
        ("/format", json!(LEGACY_FORMAT), "for a definition with an orientation statement"),
        ("/format", json!(NPTH_FORMAT), "for a definition with an orientation statement"),
        ("/format", json!("pcbcraft-footprint/2"), "unsupported footprint format pcbcraft-footprint/2"),
    ] {
        let mut bad = good.clone();
        let (parent, field) = pointer.rsplit_once('/').unwrap();
        bad.pointer_mut(parent).unwrap()[field] = value;
        let message = read_saved(bad.to_string().as_bytes()).expect_err(pointer);
        if !message.contains(reason) {
            wrong.push(format!("{pointer} = {}: refused with {message:?}, expected {reason:?}", bad.pointer(pointer).unwrap()));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
    let mut missing = good.clone();
    missing["orientation"].as_object_mut().unwrap().remove("polarized");
    refused(read_saved(missing.to_string().as_bytes()), "missing field `polarized`");
    let mut historical = good.clone();
    historical["format"] = json!(MIXED_HOLES_FORMAT);
    refused(read_saved(historical.to_string().as_bytes()), "for a definition with an orientation statement");
}

/// Formats 6 and 7 retain their original bytes when read or pinned. An editor export
/// writes format 13 even when neither optional assembly record is present.
#[test]
fn plain_current_and_historical_mixed_hole_formats_round_trip() {
    let mut plain = package("resistor-0402", "plain-current", None);
    plain["format"] = json!(ORIENTATION_FORMAT);
    let parsed = read_saved(plain.to_string().as_bytes()).unwrap();
    assert_eq!(parsed.format, ORIENTATION_FORMAT);
    assert!(parsed.orientation.is_none());
    let written: Value = serde_json::from_slice(&write_saved(&parsed).unwrap()).unwrap();
    assert_eq!(written, plain, "round trip is exact");
    let exported = crate::authoring::Item::edit_footprint(&parsed).unwrap().export_native().unwrap();
    assert_eq!(exported["format"], json!(CURRENT_FORMAT));
    assert!(exported.get("orientation").is_none());
    assert!(read_saved(exported.to_string().as_bytes()).unwrap().orientation.is_none());

    let mut historical = plain.clone();
    historical["id"] = json!("plain-historical");
    historical["format"] = json!("pcbcraft-footprint/6");
    historical["non_plated_holes"] = json!([{"center":{"x_nm":0,"y_nm":2 * MM},"diameter_nm":MM / 2}]);
    let parsed = read_saved(historical.to_string().as_bytes()).unwrap();
    assert_eq!(parsed.format, MIXED_HOLES_FORMAT);
    let written: Value = serde_json::from_slice(&write_saved(&parsed).unwrap()).unwrap();
    assert_eq!(written, historical, "round trip is exact");
    let s = fixture(&[historical.clone()]);
    let documents = encode_documents(s.snapshot().unwrap()).unwrap();
    let saved: Value = serde_json::from_slice(&documents[&format!("library/footprints/{LIBRARY}/plain-historical.r1.json")]).unwrap();
    assert_eq!(saved["format"], json!(MIXED_HOLES_FORMAT));
    assert_eq!(saved["non_plated_holes"], historical["non_plated_holes"]);
    assert!(saved.get("orientation").is_none());
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
    let exported = crate::authoring::Item::edit_footprint(&parsed).unwrap().export_native().unwrap();
    assert_eq!(exported["format"], json!(CURRENT_FORMAT));
    assert_eq!(exported["non_plated_holes"], historical["non_plated_holes"]);
}

/// An oriented format 13 package keeps its circular non-plated hole and label settings
/// through read, write and project pinning. Older formats reject unsupported fields.
#[test]
fn oriented_footprint_with_npth_round_trip() {
    let mut mounted = package("resistor-0402", "diode-mounted", Some(json!({"polarized":true,"pin_1":"1","zero":"ipc-7351"})));
    mounted["non_plated_holes"] = json!([{"center":{"x_nm":0,"y_nm":2 * MM},"diameter_nm":MM / 2}]);
    mounted["assembly_label"] = json!({"rotation": 90});
    let mut old_oriented = mounted.clone();
    old_oriented["format"] = json!(ORIENTATION_FORMAT);
    refused(read_saved(old_oriented.to_string().as_bytes()), "assembly label settings require footprint format 13");
    let parsed = read_saved(mounted.to_string().as_bytes()).unwrap();
    assert_eq!(parsed.non_plated_holes.len(), 1);
    assert_eq!(parsed.orientation.as_ref().unwrap().pin_1.as_deref(), Some("1"));
    let written: Value = serde_json::from_slice(&write_saved(&parsed).unwrap()).unwrap();
    assert_eq!(written, mounted, "round trip is exact");
    for format in [LEGACY_FORMAT, NPTH_FORMAT] {
        let mut downgraded = mounted.clone();
        if let Some(fields) = downgraded.as_object_mut() {
            fields.remove("assembly_label");
        }
        downgraded["format"] = json!(format);
        refused(read_saved(downgraded.to_string().as_bytes()), "for a definition with an orientation statement");
    }
    let s = fixture(&[mounted.clone()]);
    let documents = encode_documents(s.snapshot().unwrap()).unwrap();
    let saved: Value = serde_json::from_slice(&documents[&format!("library/footprints/{LIBRARY}/diode-mounted.r1.json")]).unwrap();
    assert_eq!(saved["format"], json!(CURRENT_FORMAT));
    assert_eq!(saved["non_plated_holes"], mounted["non_plated_holes"]);
    assert_eq!(saved["orientation"], mounted["orientation"]);
    assert_eq!(saved["assembly_label"], mounted["assembly_label"]);
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
}

#[test]
fn oriented_footprint_preserves_plated_slots_and_fabrication_guard() {
    let mut slotted = package("resistor-0402", "slotted-orientation", Some(json!({"polarized":true,"pin_1":"1"})));
    for pad in slotted["pads"].as_array_mut().unwrap() {
        pad["copper"] = json!("all_copper");
        pad["plated_slot"] = json!({"width_nm":125_000,"length_nm":400_000,"rotation_mdeg":30_000});
    }
    let symmetric = read_saved(slotted.to_string().as_bytes()).unwrap();
    assert!(symmetric.has_plated_slots());
    assert!(!orientation_marked(&symmetric));
    let mut asymmetric = slotted.clone();
    asymmetric["pads"][1]["plated_slot"]["rotation_mdeg"] = json!(60_000);
    assert!(orientation_marked(&read_saved(asymmetric.to_string().as_bytes()).unwrap()));

    let mut s = fixture(&[slotted]);
    assert_eq!(findings(&mut s).len(), 1);
    let documents = encode_documents(s.snapshot().unwrap()).unwrap();
    let reopened = decode_documents(&documents).unwrap();
    assert_eq!(encode_documents(reopened.clone()).unwrap(), documents);
    let board = reopened.boards().next().unwrap();
    assert!(board.footprints().next().unwrap().pads.iter().all(|pad| pad.plated_slot.is_some()));
    refused(board.fabrication(), "cannot yet machine plated slots");
}

/// A project pins the format 7 copy and saves it again byte for byte; the editor declines to
/// open it rather than drop the statement.
#[test]
fn pinned_copy_round_trip() {
    let mut diode = package("resistor-0402", "diode-0402", Some(json!({"polarized":true,"pin_1":"1"})));
    diode["format"] = json!(ORIENTATION_FORMAT);
    let s = fixture(&[diode, json!("resistor-0402")]);
    let documents = encode_documents(s.snapshot().unwrap()).unwrap();
    let pinned = &documents[&format!("library/footprints/{LIBRARY}/diode-0402.r1.json")];
    let saved: Value = serde_json::from_slice(pinned).unwrap();
    assert_eq!(saved["format"], json!(ORIENTATION_FORMAT));
    assert_eq!(saved["orientation"], json!({"polarized":true,"pin_1":"1"}));
    let bundled: Value = serde_json::from_slice(&documents["library/footprints/basic-components/resistor-0402.r4.json"]).unwrap();
    assert_eq!(bundled["format"], json!(LEGACY_FORMAT));
    assert!(bundled.get("orientation").is_none());
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
    refused(crate::authoring::Item::edit_footprint(&read_saved(pinned).unwrap()), "states an assembly orientation, which the editor cannot keep yet");
    refused(crate::authoring::Item::from_footprint(&read_saved(pinned).unwrap()), "states an assembly orientation, which the editor cannot keep yet");
}

/// A released profile correction changes the output row but does not make the package less
/// ambiguous. The profile and the footprint's statement survive native save/reopen together.
#[test]
fn adopted_profile_does_not_replace_package_orientation() {
    use crate::{ADOPT_ASSEMBLY_PROFILE, PREVIEW_ASSEMBLY_PROFILE, SET_ASSEMBLY_PROFILE, board::track::tests::view};
    let diode = package("resistor-0402", "diode-0402", Some(json!({"polarized":true})));
    let mut s = fixture(&[diode]);
    let found = findings(&mut s);
    assert_eq!(found.len(), 1);
    let footprint = found[0]["objects"][0]["id"].as_str().unwrap();
    let board = s.project().unwrap().board_target().unwrap().board;
    let profile = uuid::Uuid::from_u128(1);
    s.apply(
        SET_ASSEMBLY_PROFILE,
        &json!({"expected_revision":s.revision(),"profile":profile,"content":{
            "revision":1,"name":"Placement correction","provenance":{"kind":"authored"},
            "corrections":[{"library":LIBRARY,"definition":"diode-0402","rotation":180}]
        }}),
    )
    .unwrap();
    let mut adoption = json!({"board":board,"expected_revision":s.revision(),"profile":profile,"revision":1});
    adoption["acceptance"] = s.execute(PREVIEW_ASSEMBLY_PROFILE, &adoption).unwrap()["acceptance"].clone();
    s.apply(ADOPT_ASSEMBLY_PROFILE, &adoption).unwrap();
    assert_eq!(view(&s).placement_row(footprint, None).unwrap().rotation, 0);
    assert_eq!(view(&s).placement_row(footprint, Some(profile)).unwrap().rotation, 180);
    assert_eq!(findings(&mut s), found);

    let docs = encode_documents(s.snapshot().unwrap()).unwrap();
    let entry: Value = serde_json::from_slice(&docs["project.json"]).unwrap();
    let board_doc: Value = serde_json::from_slice(&docs[&test_board_path(&docs)]).unwrap();
    assert_eq!(entry["format"], crate::persistence::CURRENT_PROJECT_FORMAT);
    assert_eq!(board_doc["format"], crate::persistence::CURRENT_BOARD_FORMAT);
    assert_eq!(board_doc["assembly_profiles"], json!([{"profile":profile,"revision":1}]));
    let reopened = decode_documents(&docs).unwrap();
    assert_eq!(encode_documents(reopened.clone()).unwrap(), docs);
    s.open_snapshot(reopened);
    assert_eq!(view(&s).placement_row(footprint, Some(profile)).unwrap().rotation, 180);
    assert_eq!(findings(&mut s), found);
}
