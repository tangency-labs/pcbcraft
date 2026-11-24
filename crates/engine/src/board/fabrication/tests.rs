use super::*;
use crate::board::{
    drc::tests::outline,
    track::tests::{copper_layers, route, route_with_vias, routed_fixture, view},
};
use std::io::Read as _;

/// Two identical lands exercise aperture reuse as their placement rotates and flips.
fn aperture_fixture(shape: PadShape, width: i64, height: i64, pad_rotation: u16, rotation: u16, bottom: bool) -> Session {
    aperture_fixture_with_pads(&[(shape, width, height, pad_rotation); 2], rotation, bottom)
}

/// Author lands through the library editor, load its production encoding, then assign
/// and place the package through the same commands used by live and headless clients.
fn aperture_fixture_with_pads(pads: &[(PadShape, i64, i64, u16)], rotation: u16, bottom: bool) -> Session {
    aperture_fixture_at(pads, rotation, bottom, Point { x_nm: 0, y_nm: 0 }, Point { x_nm: 30_000_001, y_nm: -25_000_003 }, None, None)
}

/// Author the command-built fixture with explicit centers and optionally plate pads from a
/// zero-based index onward. A plated fixture has inner copper too. An explicit mapping
/// covers every pad; otherwise pads beyond the first two are left unconnected.
/// Rounded rectangles use a quarter of their short side as the corner radius.
fn aperture_fixture_at(
    pads: &[(PadShape, i64, i64, u16)],
    rotation: u16,
    bottom: bool,
    offset: Point,
    position: Point,
    plated_from: Option<usize>,
    mapping: Option<serde_json::Value>,
) -> Session {
    aperture_fixture_with_pitch(pads, rotation, bottom, (offset, Point { x_nm: 2_000_000, y_nm: 0 }), position, plated_from, mapping)
}

/// Author lands with an explicit pitch vector to exercise arbitrary library zero orientations.
/// Rounded rectangles use a quarter of their short side as the corner radius.
fn aperture_fixture_with_pitch(
    pads: &[(PadShape, i64, i64, u16)],
    rotation: u16,
    bottom: bool,
    (offset, pitch): (Point, Point),
    position: Point,
    plated_from: Option<usize>,
    mapping: Option<serde_json::Value>,
) -> Session {
    let pads: Vec<_> = pads
        .iter()
        .map(|&(shape, width, height, pad_rotation)| {
            let radius = if shape == PadShape::RoundedRectangle { (width.min(height) / 4).max(1) } else { 0 };
            (shape, width, height, pad_rotation, radius)
        })
        .collect();
    aperture_fixture_with_radii_and_pitch(&pads, rotation, bottom, (offset, pitch), position, plated_from, mapping)
}

/// Add optional authored graphics or pad labels before pinning and placing the library item.
fn decorated_aperture_fixture(
    pads: &[(PadShape, i64, i64, u16)],
    (rotation, bottom): (u16, bool),
    (offset, pitch): (Point, Point),
    position: Point,
    plated_from: Option<usize>,
    mapping: Option<serde_json::Value>,
    decorate: impl FnOnce(&mut crate::authoring::Session),
) -> Session {
    let pads: Vec<_> = pads
        .iter()
        .map(|&(shape, width, height, pad_rotation)| {
            let radius = if shape == PadShape::RoundedRectangle { (width.min(height) / 4).max(1) } else { 0 };
            (shape, width, height, pad_rotation, radius)
        })
        .collect();
    build_footprint_fixture(&pads, (rotation, bottom), (offset, pitch), (position, plated_from), mapping, &[], decorate)
}

/// Author command-built lands with an explicit corner radius per pad.
fn aperture_fixture_with_radii(
    pads: &[(PadShape, i64, i64, u16, i64)],
    rotation: u16,
    bottom: bool,
    offset: Point,
    position: Point,
    plated_from: Option<usize>,
) -> Session {
    aperture_fixture_with_radii_and_pitch(pads, rotation, bottom, (offset, Point { x_nm: 2_000_000, y_nm: 0 }), position, plated_from, None)
}

/// Author command-built lands with an explicit corner radius per pad, pitch vector and
/// optional pad mapping.
fn aperture_fixture_with_radii_and_pitch(
    pads: &[(PadShape, i64, i64, u16, i64)],
    rotation: u16,
    bottom: bool,
    (offset, pitch): (Point, Point),
    position: Point,
    plated_from: Option<usize>,
    mapping: Option<serde_json::Value>,
) -> Session {
    footprint_fixture(pads, (rotation, bottom), (offset, pitch), position, plated_from, mapping, &[])
}

/// Build a physical package with an explicit corner radius per pad and optional non-plated
/// holes through normal editor transactions.
fn footprint_fixture(
    pads: &[(PadShape, i64, i64, u16, i64)],
    (rotation, bottom): (u16, bool),
    (offset, pitch): (Point, Point),
    position: Point,
    plated_from: Option<usize>,
    mapping: Option<serde_json::Value>,
    holes: &[crate::board::definition::CircularNpth],
) -> Session {
    build_footprint_fixture(pads, (rotation, bottom), (offset, pitch), (position, plated_from), mapping, holes, |_| {})
}

/// Shared footprint-fixture builder supporting explicit pad radii, non-plated holes and
/// optional authored graphics or pad labels before pinning and placing the library item.
fn build_footprint_fixture(
    pads: &[(PadShape, i64, i64, u16, i64)],
    (rotation, bottom): (u16, bool),
    (offset, pitch): (Point, Point),
    (position, plated_from): (Point, Option<usize>),
    mapping: Option<serde_json::Value>,
    holes: &[crate::board::definition::CircularNpth],
    decorate: impl FnOnce(&mut crate::authoring::Session),
) -> Session {
    use crate::authoring::{Action, Item, Kind, Primitive};
    let mut editor = crate::authoring::Session::new(Item::new(Kind::Footprint).unwrap(), false).unwrap();
    editor.execute(editor.revision(), Action::Delete { objects: vec![2, 3] }).unwrap();
    let Primitive::Pad { pad: template } = editor.snapshot().objects[&1].clone() else { panic!("pad") };
    for (index, &(shape, width, height, pad_rotation, corner_radius_nm)) in pads.iter().enumerate() {
        let mut pad = template.clone();
        pad.number = (index + 1).to_string();
        pad.center = Point { x_nm: index as i64 * pitch.x_nm + offset.x_nm, y_nm: index as i64 * pitch.y_nm + offset.y_nm };
        if plated_from.is_some_and(|first| index >= first) {
            pad.copper = crate::board::definition::CopperBinding::AllCopper;
            pad.drill_nm = Some(width.min(height) / 2);
        }
        pad.shape = shape;
        pad.corner_radius_nm = corner_radius_nm;
        pad.width_nm = width;
        pad.height_nm = height;
        pad.rotation = pad_rotation;
        let primitive = Primitive::Pad { pad };
        let action = if index == 0 { Action::Replace { object: 1, primitive } } else { Action::Insert { primitive } };
        editor.execute(editor.revision(), action).unwrap();
    }
    decorate(&mut editor);
    for &hole in holes {
        editor.execute(editor.revision(), Action::Insert { primitive: Primitive::NonPlatedHole { hole } }).unwrap();
    }
    let definition = editor.snapshot().footprint().unwrap();
    let document = editor.snapshot().export_native().unwrap().to_string();
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    let manifest =
        json!({"format":crate::library::LIBRARY_FORMAT,"id":definition.library,"name":"Aperture test library","license":definition.license})
            .to_string();
    s.execute("library.load_source", &json!({"source":"apertures","documents":{"library.json":manifest,"footprints/lands.json":document}})).unwrap();
    // Include interior copper in the plated-land fixture.
    let copper_layers = if plated_from.is_some() { 4 } else { 2 };
    s.apply(crate::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":copper_layers,"expected_revision":s.revision()})).unwrap();
    s.apply(crate::PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let component = s.project().unwrap().components().next().unwrap().id();
    let unconnected: Vec<&str> = if mapping.is_some() { Vec::new() } else { definition.pads[2..].iter().map(|pad| pad.number.as_str()).collect() };
    s.apply(
        crate::ASSIGN_FOOTPRINT,
        &json!({"component":component,"library":definition.library,"definition":definition.id,
        "definition_revision":definition.revision,"mapping":mapping.unwrap_or_else(|| json!({"1":definition.pads[0].number,"2":definition.pads[1].number})),
        "unconnected_pads":unconnected,"expected_revision":s.revision()}),
    )
    .unwrap();
    s.apply(
        crate::PLACE_ASSIGNED_FOOTPRINT,
        &json!({"component":component,"x_nm":position.x_nm,"y_nm":position.y_nm,"expected_revision":s.revision()}),
    )
    .unwrap();
    let footprint = view(&s).footprints().next().unwrap().id;
    s.apply(
        crate::SET_FOOTPRINT_TRANSFORM,
        &json!({"footprint":footprint,"x_nm":position.x_nm,"y_nm":position.y_nm,
        "rotation":rotation,"side":if bottom {"back"} else {"front"},"expected_revision":s.revision()}),
    )
    .unwrap();
    outline(&mut s);
    s
}

/// Append a plain 0402 resistor through the live placement and transform commands.
fn add_assembly_resistor(s: &mut Session, schematic_x: i64, at: Point, rotation: u16) -> String {
    s.apply(crate::PLACE_RESISTOR, &json!({"x_nm":schematic_x,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let part = s.project().unwrap().components().last().unwrap().id();
    let definition = crate::board::definition::basic_components("resistor-0402").unwrap();
    s.apply(
        crate::ASSIGN_FOOTPRINT,
        &json!({"component":part,"library":definition.library,"definition":definition.id,"definition_revision":definition.revision,
            "mapping":{"1":"1","2":"2"},"expected_revision":s.revision()}),
    )
    .unwrap();
    s.apply(crate::PLACE_ASSIGNED_FOOTPRINT, &json!({"component":part,"x_nm":at.x_nm,"y_nm":at.y_nm,"expected_revision":s.revision()})).unwrap();
    let footprint = view(s).footprint_ids().last().unwrap();
    s.apply(
        crate::SET_FOOTPRINT_TRANSFORM,
        &json!({"footprint":footprint,"x_nm":at.x_nm,"y_nm":at.y_nm,"rotation":rotation,"side":"front","expected_revision":s.revision()}),
    )
    .unwrap();
    s.project().unwrap().symbols.last().unwrap().id()
}

/// Ucamco Gerber rev. 2026.05 §§5.6.3, 5.6.10, 5.6.15–16: the D10 aperture is a 0.300 mm
/// main flash with its own attributes. Both sides are read looking from the top; the bottom
/// layer follows the last physical copper layer. Library angles and drill geometry aren't metadata.
#[test]
fn gerber_x3_places_one_main_flash_per_part_on_its_own_side() {
    let mut s = aperture_fixture_with_pitch(
        &[(PadShape::Oval, 800_000, 900_000, 37); 2],
        30,
        true,
        (Point { x_nm: 0, y_nm: 0 }, Point { x_nm: 2_000_000, y_nm: 1_000_000 }),
        Point { x_nm: 31_000_001, y_nm: -21_000_003 },
        Some(0),
        None,
    );
    let top = add_assembly_resistor(&mut s, 10_000_000, Point { x_nm: 7_000_000, y_nm: 9_000_000 }, 270);
    add_assembly_resistor(&mut s, 20_000_000, Point { x_nm: 12_000_000, y_nm: 16_000_000 }, 90);
    let bottom_name = s
        .project()
        .unwrap()
        .board
        .footprints()
        .first()
        .and_then(|placement| s.project().unwrap().board.definition_for(placement))
        .unwrap()
        .name
        .clone();
    s.apply(crate::SET_SYMBOL_FIELDS, &json!({"symbol":top,"reference":"R2","value":"4k7,±1%*\\nominal","expected_revision":s.revision()})).unwrap();
    let before = view(&s).gerber_part_layers().unwrap();
    assert!(before.files[0].contents.contains("X7000000Y-9000000D03*"));
    assert!(before.files.iter().all(|file| !file.contents.contains(".SameCoordinates")));
    let board = s.project().unwrap().board_target().unwrap().board;
    s.apply(
        crate::SET_PLACEMENT_ORIGIN,
        &json!({"board":board,"expected_revision":s.revision(),
        "origin":{"kind":"point","x_nm":8_000_000,"y_nm":10_000_001}}),
    )
    .unwrap();

    let revision = s.revision();
    let snapshot = s.snapshot().unwrap();
    let bare_board = view(&s).fabrication_outputs().unwrap();
    let layers = view(&s).gerber_part_layers().unwrap();
    let [top, bottom] = layers.files.as_slice() else { panic!("Expected exactly two populated layers") };
    assert!(top.name.ends_with("-Assembly_Top.gbr"));
    assert!(bottom.name.ends_with("-Assembly_Bottom.gbr"));
    for (file, function) in [(top, "L1,Top"), (bottom, "L4,Bot")] {
        assert!(file.contents.contains(&format!("%TF.FileFunction,Component,{function}*%\n")));
        assert!(file.contents.contains("%FSLAX46Y46*%\n%MOMM*%\n"));
        assert!(file.contents.contains("%TA.AperFunction,ComponentMain*%\n%ADD10C,0.300000*%\n%TD.AperFunction*%\n"));
        assert!(file.contents.contains("%TA.AperFunction,ComponentPin*%\n%ADD11P,0.360000X4X0.0*%\n%TD.AperFunction*%\n"));
        assert_eq!(file.contents.lines().filter(|line| *line == "D10*").count(), if function == "L1,Top" { 2 } else { 1 });
        assert!(!file.contents.contains(".SameCoordinates"));
        assert!(!file.contents.contains("%TO.CRot,"));
    }
    let main_objects = |file: &Output| {
        file.contents
            .split("%TD*%\n")
            .filter_map(|object| {
                let start = object.find("%TO.C,")?;
                object.ends_with("D03*\n").then_some(&object[start..])
            })
            .collect::<String>()
    };
    assert_eq!(
        main_objects(top),
        concat!(
            "%TO.C,R2*%\n%TO.CVal,4k7\\u002C±1\\u0025\\u002A\\u005Cnominal*%\n",
            "%TO.CFtp,Resistor 0402 (1005 metric)*%\n%TO.CMnt,SMD*%\nX-1000000Y1000001D03*\n",
            "%TO.C,R3*%\n%TO.CVal,R*%\n",
            "%TO.CFtp,Resistor 0402 (1005 metric)*%\n%TO.CMnt,SMD*%\nX4000000Y-5999999D03*\n",
        )
    );
    assert_eq!(main_objects(bottom), format!("%TO.C,R1*%\n%TO.CVal,R*%\n%TO.CFtp,{bottom_name}*%\nX23000001Y31000004D03*\n"));
    assert_eq!(layers, view(&s).gerber_part_layers().unwrap());
    assert!(bare_board.iter().filter(|file| file.name.ends_with(".gbr")).all(|file| file.contents.contains("%TF.SameCoordinates,Original*%")));
    assert_eq!(bare_board, view(&s).fabrication_outputs().unwrap());
    assert_eq!(revision, s.revision());
    assert!(std::sync::Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
}

/// An adopted, nonzero centroid follows the placed rotation and side, but Gerber
/// stays in top view even if the profile's placement rows are viewed from below.
#[test]
fn gerber_x3_flashes_the_adopted_centroid_in_top_view() {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    s.apply(crate::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":s.revision()})).unwrap();
    add_assembly_resistor(&mut s, 0, Point { x_nm: 40_000_000, y_nm: 25_000_000 }, 90);
    let board = s.project().unwrap().board_target().unwrap().board;
    let footprint = view(&s).footprint_ids().next().unwrap();
    s.apply(
        crate::SET_PLACEMENT_ORIGIN,
        &json!({"board":board,"expected_revision":s.revision(),"origin":{"kind":"point","x_nm":10_000_000,"y_nm":30_000_000}}),
    )
    .unwrap();
    let flash = |s: &Session| {
        let layers = view(s).gerber_part_layers().unwrap();
        let [file] = layers.files.as_slice() else { panic!("Expected one populated layer") };
        file.contents.lines().find(|line| line.ends_with("D03*")).unwrap().to_owned()
    };
    let adopt = |s: &mut Session, profile, name, x_nm| {
        s.apply(
            crate::SET_ASSEMBLY_PROFILE,
            &json!({"profile":profile,"expected_revision":s.revision(),"content":{"revision":1,"name":name,
            "provenance":{"kind":"authored"},"bottom_side":"viewed_from_bottom","corrections":[
                {"library":"basic-components","definition":"resistor-0402","centroid":{"x_nm":x_nm,"y_nm":-200_000}}
            ]}}),
        )
        .unwrap();
        let mut params = json!({"board":board,"expected_revision":s.revision(),"profile":profile,"revision":1});
        let preview = s.execute(crate::PREVIEW_ASSEMBLY_PROFILE, &params).unwrap();
        params["acceptance"] = preview["acceptance"].clone();
        s.apply(crate::ADOPT_ASSEMBLY_PROFILE, &params).unwrap();
    };
    assert_eq!(flash(&s), "X30000000Y5000000D03*");
    adopt(&mut s, uuid::Uuid::from_u128(1), "First house", 300_000);
    // At 90 degrees, local (x, y) becomes (y, -x). The output frame then flips Y.
    assert_eq!(flash(&s), "X29800000Y5300000D03*");
    s.apply(
        crate::SET_FOOTPRINT_TRANSFORM,
        &json!({"footprint":footprint,"x_nm":40_000_000,"y_nm":25_000_000,"rotation":90,"side":"back","expected_revision":s.revision()}),
    )
    .unwrap();
    // A bottom footprint mirrors local X before rotating; the Gerber output does not mirror X.
    assert_eq!(flash(&s), "X29800000Y4700000D03*");
    adopt(&mut s, uuid::Uuid::from_u128(2), "Second house", 400_000);
    assert_eq!(flash(&s), "X30000000Y5000000D03*");
}

/// Ucamco Gerber rev. 2026.05 §§4.2.2, 4.7.2, 5.6.10 and 5.6.14–15: in 4.6 mm the
/// authored outline centerline is exact to a nanometre. `.P` uses physical pad labels and the
/// key pin uses the mandated 0.360 mm four-point diamond instead of the ordinary 0.100 mm circle.
#[test]
fn gerber_x3_pins_and_closed_outlines_use_the_placed_package_geometry() {
    use crate::{
        authoring::{Action, Primitive},
        board::definition::Artwork,
        library_graphics::{Fill, Geometry, Graphic},
    };
    let p = |x_nm, y_nm| Point { x_nm, y_nm };
    let mut s = decorated_aperture_fixture(
        &[(PadShape::Circle, 800_000, 800_000, 0); 2],
        (90, true),
        (p(123, 457), p(2_000_000, 0)),
        p(31_000_001, -21_000_003),
        None,
        None,
        |editor| {
            for (object, number) in [(1, "B2"), (4, "A1")] {
                let Primitive::Pad { mut pad } = editor.snapshot().objects[&object].clone() else { panic!("pad") };
                pad.number = number.into();
                editor.execute(editor.revision(), Action::Replace { object, primitive: Primitive::Pad { pad } }).unwrap();
            }
            let rectangle = Geometry::Rectangle { min: p(-600_001, -410_003), max: p(500_007, 420_013) };
            for (purpose, geometry) in [
                (GraphicPurpose::Fabrication, rectangle.clone()),
                (GraphicPurpose::Courtyard, Geometry::Circle { center: p(-10, 20), radius_nm: 900_019 }),
                (GraphicPurpose::Courtyard, Geometry::Arc { center: p(100, 200), radius_nm: 910_000, start_degrees: 19, sweep_degrees: 360 }),
                (GraphicPurpose::Courtyard, Geometry::Arc { center: p(-200, -300), radius_nm: 920_000, start_degrees: 14, sweep_degrees: -360 }),
                (GraphicPurpose::Fabrication, Geometry::Polyline { points: vec![p(13, 27), p(33, 27), p(13, 57), p(13, 27)], closed: false }),
                (GraphicPurpose::Fabrication, Geometry::Polyline { points: vec![p(13, 27), p(33, 27), p(13, 57)], closed: false }),
                (GraphicPurpose::Courtyard, Geometry::Arc { center: p(100, 200), radius_nm: 910_000, start_degrees: 19, sweep_degrees: 359 }),
                (GraphicPurpose::Fabrication, Geometry::Line { from: p(0, 0), to: p(4, 3) }),
                (GraphicPurpose::Fabrication, Geometry::Polyline { points: vec![p(0, 0), p(8, 8), p(0, 8), p(7, 0)], closed: true }),
                (GraphicPurpose::Silkscreen, rectangle),
            ] {
                let artwork = Artwork { purpose, graphic: Graphic { geometry, stroke_nm: 150_000, fill: Fill::None } };
                editor.execute(editor.revision(), Action::Insert { primitive: Primitive::Artwork { artwork } }).unwrap();
            }
        },
    );
    let board = s.project().unwrap().board_target().unwrap().board;
    s.apply(
        crate::SET_PLACEMENT_ORIGIN,
        &json!({"board":board,"expected_revision":s.revision(),"origin":{"kind":"point","x_nm":8_000_000,"y_nm":10_000_001}}),
    )
    .unwrap();
    let output = view(&s).gerber_part_layers().unwrap();
    let [bottom] = output.files.as_slice() else { panic!("Expected only a bottom layer") };
    let contents = &bottom.contents;
    assert!(contents.contains("%TF.FileFunction,Component,L2,Bot*%\n"));
    assert!(contents.contains("%FSLAX46Y46*%\n%MOMM*%\n"));
    assert!(contents.contains("%TA.AperFunction,ComponentPin*%\n%ADD11C,0.100000*%\n%TD.AperFunction*%\n"));
    assert!(contents.contains("%TA.AperFunction,ComponentPin*%\n%ADD12P,0.360000X4X0.0*%\n%TD.AperFunction*%\n"));
    assert!(contents.contains("%TA.AperFunction,ComponentOutline,Body*%\n%ADD13C,0.100000*%\n%TD.AperFunction*%\n"));
    assert!(contents.contains("%TA.AperFunction,ComponentOutline,Courtyard*%\n%ADD14C,0.100000*%\n%TD.AperFunction*%\n"));
    // Bottom +90° maps (x,y) to (y,-x) in this output frame. These are the independently
    // calculated centers and contour commands, including the closing draw and arc center.
    let (_, geometry) = contents.split_once("\nD11*\n").unwrap();
    assert_eq!(
        geometry,
        concat!(
            "%TO.P,R1,B2*%\nX23000458Y30999881D03*\n%TD*%\n",
            "D12*\n%TO.P,R1,A1*%\nX23000458Y28999881D03*\n%TD*%\n",
            "D13*\n%TO.C,R1*%\nX22589998Y31600005D02*\n",
            "X22589998Y30499997D01*\nX23420014Y30499997D01*\nX23420014Y31600005D01*\nX22589998Y31600005D01*\n%TD*%\n",
            "D14*\n%TO.C,R1*%\nX23900040Y31000014D02*\nG75*\nG03*\nX23900040Y31000014I-900019J0D01*\nG01*\n%TD*%\n",
            "%TO.C,R1*%\nX23910201Y30999904D02*\nG75*\nG03*\nX23910201Y30999904I-910000J0D01*\nG01*\n%TD*%\n",
            "%TO.C,R1*%\nX23919701Y31000204D02*\nG75*\nG03*\nX23919701Y31000204I-920000J0D01*\nG01*\n%TD*%\n",
            "D13*\n%TO.C,R1*%\nX23000028Y30999991D02*\n",
            "X23000028Y30999971D01*\nX23000058Y30999991D01*\nX23000028Y30999991D01*\n%TD*%\nM02*\n",
        )
    );
}

/// Empty sides stay absent, and unavailable setup or origin produces a specific refusal.
#[test]
fn gerber_x3_requires_setup_and_a_resolvable_origin() {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    assert_eq!(view(&s).gerber_part_layers().unwrap_err(), "Initialize the board setup before generating Gerber assembly layers.");
    s.apply(crate::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":s.revision()})).unwrap();
    assert!(view(&s).gerber_part_layers().unwrap().files.is_empty());
    add_assembly_resistor(&mut s, 0, Point { x_nm: 1_000_000, y_nm: 1_000_000 }, 0);
    let layers = view(&s).gerber_part_layers().unwrap();
    assert_eq!(layers.files.len(), 1);
    assert!(layers.files[0].name.ends_with("-Assembly_Top.gbr"));
    let board = s.project().unwrap().board_target().unwrap().board;
    s.apply(crate::SET_PLACEMENT_ORIGIN, &json!({"board":board,"expected_revision":s.revision(),"origin":{"kind":"outline_lower_left"}})).unwrap();
    assert!(view(&s).gerber_part_layers().unwrap_err().contains("Board Outline"));
}

/// Pending removals still flash while unique, even across sides; a reused live reference
/// would link that retained placement to the wrong part, so fail the entire export (§§5.6.15–16).
#[test]
fn gerber_x3_rejects_a_reference_reused_while_removal_is_pending() {
    let mut s = routed_fixture();
    let project = s.project().unwrap();
    let [first, second, ..] = &project.symbols[..] else { panic!("Expected at least two symbols") };
    let (first, second) = (first.id(), second.id());
    let placed_parts: Vec<_> = view(&s).footprints().collect();
    let [placed, live, ..] = placed_parts.as_slice() else { panic!("Expected at least two footprints") };
    let (retained, other) = (placed.id.clone(), live.id.clone());
    let at = placed.position;
    s.apply(
        crate::SET_FOOTPRINT_TRANSFORM,
        &json!({"footprint":retained,"x_nm":at.x_nm,"y_nm":at.y_nm,
        "rotation":0,"side":"back","expected_revision":s.revision()}),
    )
    .unwrap();
    s.apply(crate::DELETE_SYMBOL, &json!({"symbol":first,"expected_revision":s.revision()})).unwrap();
    assert!(view(&s).footprint(&retained).unwrap().pending_removal);
    let layers = view(&s).gerber_part_layers().unwrap();
    assert!(layers.files[1].contents.contains("%TO.C,R1*%"));
    s.apply(crate::SET_SYMBOL_FIELDS, &json!({"symbol":second,"reference":"R1","value":"R","expected_revision":s.revision()})).unwrap();
    assert_eq!(view(&s).gerber_part_layers().unwrap_err(), format!("Duplicate Gerber reference \"R1\" on placements {retained} and {other}."));
}

/// Test-only read-back of the emitted Gerber subset, independent of the writer's aperture key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReadAperture {
    kind: char,
    width: i64,
    height: i64,
    function: Option<&'static str>,
}

#[derive(Debug, Default)]
struct ReadGerber {
    apertures: BTreeMap<usize, ReadAperture>,
    flashes: Vec<(Point, ReadAperture)>,
    strokes: Vec<(Point, Point, ReadAperture)>,
    regions: Vec<Vec<Point>>,
    region_functions: Vec<&'static str>,
    rounded: BTreeMap<usize, ReadRounded>,
    rounded_flashes: Vec<(Point, usize)>,
    objects: Vec<(Option<&'static str>, ReadObject)>,
}

#[derive(Debug, Default)]
struct ReadRounded {
    circles: Vec<(i64, Point)>,
    strips: Vec<(i64, Point, Point)>,
    outline: Vec<Point>,
    function: Option<&'static str>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ReadObject {
    net: Option<String>,
    pin: Option<(String, String)>,
    component: Option<String>,
}

impl ReadObject {
    /// Read one writer attribute, refusing redefinition before the previous value is cleared.
    fn set(&mut self, line: &str, outer: bool) -> Result<(), &'static str> {
        let record = line.strip_prefix("%TO.").and_then(|s| s.strip_suffix("*%")).ok_or("invalid object attribute")?;
        let fields = record.split(',').collect::<Vec<_>>();
        match fields.as_slice() {
            ["N", value] if self.net.is_none() => self.net = Some(read_field(value)?),
            ["C", value] if self.component.is_none() => self.component = Some(read_field(value)?),
            ["P", reference, pin] if outer && self.pin.is_none() => self.pin = Some((read_field(reference)?, read_field(pin)?)),
            _ => return Err("invalid or duplicate object attribute"),
        }
        Ok(())
    }

    /// Require a named delete to cancel one live field, not the aperture dictionary.
    fn clear(&mut self, line: &str) -> Result<(), &'static str> {
        match line {
            "%TD.N*%" if self.net.take().is_some() => Ok(()),
            "%TD.C*%" if self.component.take().is_some() => Ok(()),
            "%TD.P*%" if self.pin.take().is_some() => Ok(()),
            _ => Err("invalid object attribute delete"),
        }
    }
}

/// Decode a Gerber field without applying the exporter's net-name policy.
fn read_field(value: &str) -> Result<String, &'static str> {
    let mut field = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let digits = match chars.next() {
                    Some('u') => 4,
                    Some('U') => 8,
                    _ => return Err("unknown field escape"),
                };
                let hex = chars.by_ref().take(digits).collect::<String>();
                let decoded = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32).ok_or("invalid field escape")?;
                if hex.len() != digits {
                    return Err("short field escape");
                }
                field.push(decoded);
            }
            '%' | '*' => return Err("unescaped delimiter"),
            c if c.is_control() => return Err("unescaped control character"),
            _ => field.push(c),
        }
    }
    Ok(field)
}

/// Parse a strictly positive unsigned millimeter dimension with exactly six decimals.
fn read_mm(value: &str) -> Result<i64, &'static str> {
    let invalid = "invalid aperture dimension";
    let (whole, fraction) = value.split_once('.').ok_or(invalid)?;
    if whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) || fraction.len() != 6 || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid);
    }
    let size = whole
        .parse::<i64>()
        .map_err(|_| invalid)?
        .checked_mul(1_000_000)
        .and_then(|w| fraction.parse::<i64>().ok().and_then(|f| w.checked_add(f)))
        .ok_or(invalid)?;
    if size <= 0 {
        return Err(invalid);
    }
    Ok(size)
}

/// Parse a signed, canonical macro coordinate in millimeters, on the nanometer grid.
fn read_macro_coordinate(value: &str) -> Result<i64, &'static str> {
    if value == "0.000000" {
        return Ok(0);
    }
    if let Some(magnitude) = value.strip_prefix('-') { read_mm(magnitude).map(|n| -n) } else { read_mm(value) }
}

/// Decode only the emitted fixed-parameter macro subset (Ucamco Gerber 2026.05 §§4.5.1.3, 4.5.1.4 and 4.5.1.6).
/// Exposure must be on; zero rotation means coordinates are already
/// in the aperture's axes. Refuse unsupported primitive parameters rather than ignoring them.
fn read_rounded_macro(lines: &mut std::str::Lines<'_>) -> Result<ReadRounded, &'static str> {
    let mut read = ReadRounded::default();
    let point = |x, y| Ok::<_, &'static str>(Point { x_nm: read_macro_coordinate(x)?, y_nm: read_macro_coordinate(y)? });
    for line in lines {
        if line == "%" {
            return if read.circles.is_empty() { Err("missing macro circles") } else { Ok(read) };
        }
        let fields: Vec<_> = line.strip_suffix('*').ok_or("invalid macro primitive")?.split(',').collect();
        match fields.as_slice() {
            ["1", "1", diameter, x, y, "0"] => read.circles.push((read_mm(diameter)?, point(x, y)?)),
            ["20", "1", width, x, y, end_x, end_y, "0"] => {
                let (start, end) = (point(x, y)?, point(end_x, end_y)?);
                if start == end {
                    return Err("collapsed macro strip");
                }
                read.strips.push((read_mm(width)?, start, end));
            }
            ["4", "1", "4", xy @ .., "0"] if read.outline.is_empty() && xy.len() == 10 => {
                for &[x, y] in xy.as_chunks::<2>().0 {
                    read.outline.push(point(x, y)?);
                }
                if read.outline.first() != read.outline.last() {
                    return Err("open macro outline");
                }
            }
            _ => return Err("unsupported macro primitive"),
        }
    }
    Err("unclosed macro")
}

/// Read canonical signed integer coordinates in the writer's absolute 4.6 millimeter format.
fn read_coordinate(value: &str) -> Result<i64, &'static str> {
    let number = value.parse::<i64>().map_err(|_| "invalid coordinate")?;
    if number.to_string() != value || number.unsigned_abs() > 9_999_999_999 {
        return Err("invalid coordinate");
    }
    Ok(number)
}

/// Read the fixture subset: file identity, modes, standard and rounded apertures,
/// the writer's per-definition/region TA/TD pairs and object attribute lifetimes,
/// operation order and contour closure.
/// This is not a general Gerber parser or a CAM validity check.
fn try_read_gerber(text: &str, expected_function: &str, expected_polarity: &str) -> Result<ReadGerber, &'static str> {
    if text.contains('\r') || !text.ends_with('\n') {
        return Err("invalid line endings");
    }
    let mut lines = text.lines();
    let software = format!("%TF.GenerationSoftware,PCBcraft,PCBcraft,{}*%", env!("CARGO_PKG_VERSION"));
    for expected in ["G04 PCBcraft fabrication output*", software.as_str(), "%TF.SameCoordinates,Original*%"] {
        if lines.next() != Some(expected) {
            return Err("invalid header");
        }
    }
    let function = lines
        .next()
        .and_then(|line| line.strip_prefix("%TF.FileFunction,"))
        .and_then(|line| line.strip_suffix("*%"))
        .ok_or("invalid file function")?;
    let parts: Vec<_> = function.split(',').collect();
    let valid_function = match parts.as_slice() {
        ["Copper", layer, "Top" | "Bot" | "Inr"] => {
            layer.strip_prefix('L').and_then(|n| n.parse::<usize>().ok()).is_some_and(|n| (1..=64).contains(&n))
        }
        ["Soldermask" | "Paste" | "Legend", "Top" | "Bot"] | ["Profile", "NP"] => true,
        _ => false,
    };
    if !valid_function {
        return Err("invalid file function");
    }
    if function != expected_function {
        let (kind, detail) = expected_function.split_once(',').unwrap();
        if parts[0] != kind {
            return Err("wrong file function");
        }
        if kind == "Copper" && Some(parts[1]) != detail.split(',').next() {
            return Err("wrong copper ordinal");
        }
        return Err("wrong file side");
    }
    let polarity = format!("%TF.FilePolarity,{expected_polarity}*%");
    if lines.next() != Some(polarity.as_str()) {
        return Err("invalid file polarity");
    }
    for expected in ["%FSLAX46Y46*%", "%MOMM*%", "%LPD*%", "G01*"] {
        if lines.next() != Some(expected) {
            return Err("unsupported coordinate mode");
        }
    }
    let mut read = ReadGerber::default();
    let mut macros = BTreeMap::new();
    let mut selected = None;
    let mut position = None;
    let mut region: Option<Vec<Point>> = None;
    let mut body = false;
    let mut pending_function = None;
    let mut object = ReadObject::default();
    let copper = parts[0] == "Copper";
    let outer = copper && matches!(parts[2], "Top" | "Bot");
    while let Some(line) = lines.next() {
        if pending_function.is_some() && !line.starts_with("%ADD") && line != "G36*" {
            return Err("aperture function outside definition or region");
        }
        if let Some(value) = line.strip_prefix("%TA.AperFunction,").and_then(|value| value.strip_suffix("*%")) {
            if region.is_some() {
                return Err("attribute in region");
            }
            pending_function = Some(match value {
                "SMDPad,CuDef" => "SMDPad,CuDef",
                "ComponentPad" => "ComponentPad",
                "ViaPad" => "ViaPad",
                "Conductor" => "Conductor",
                "Material" => "Material",
                "Profile" => "Profile",
                _ => return Err("unsupported aperture function"),
            });
            continue;
        }
        if line == "M02*" {
            if region.is_some() {
                return Err("unclosed region");
            }
            if object != ReadObject::default() {
                return Err("object attributes not cleared");
            }
            if lines.next().is_some() {
                return Err("records after end");
            }
            if !macros.is_empty() {
                return Err("unused macro definition");
            }
            return Ok(read);
        }
        if let Some(name) = line.strip_prefix("%AMRoundRect") {
            if body {
                return Err("late macro definition");
            }
            let name = name.strip_suffix('*').and_then(read_aperture_code).ok_or("invalid macro name")?;
            if macros.contains_key(&name) || read.rounded.contains_key(&name) {
                return Err("duplicate macro definition");
            }
            macros.insert(name, read_rounded_macro(&mut lines)?);
            continue;
        }
        if let Some(definition) = line.strip_prefix("%ADD") {
            if body {
                return Err("late aperture definition");
            }
            let definition = definition.strip_suffix("*%").ok_or("invalid aperture")?;
            let function = pending_function.take();
            if let Some((code, name)) = definition.split_once("RoundRect") {
                let code = read_aperture_code(code).ok_or("invalid aperture code")?;
                let name = read_aperture_code(name).ok_or("invalid macro name")?;
                let aperture = ReadRounded { function, ..macros.remove(&name).ok_or("undefined macro")? };
                if name != code || read.apertures.contains_key(&code) || read.rounded.insert(code, aperture).is_some() {
                    return Err("invalid macro aperture definition");
                }
                if function.is_some() && lines.next() != Some("%TD.AperFunction*%") {
                    return Err("aperture function not cleared");
                }
                continue;
            }
            let (code_kind, dimensions) = definition.split_once(',').ok_or("invalid aperture")?;
            let kind = code_kind.chars().last().ok_or("invalid aperture")?;
            if !matches!(kind, 'C' | 'R' | 'O') {
                return Err("unsupported aperture");
            }
            let code = read_aperture_code(&code_kind[..code_kind.len() - 1]).ok_or("invalid aperture code")?;
            let dimensions = dimensions.split('X').map(read_mm).collect::<Result<Vec<_>, _>>()?;
            if dimensions.len() != if kind == 'C' { 1 } else { 2 } {
                return Err("invalid aperture arity");
            }
            let aperture = ReadAperture { kind, width: dimensions[0], height: *dimensions.last().unwrap(), function };
            if read.rounded.contains_key(&code) || read.apertures.insert(code, aperture).is_some() {
                return Err("duplicate aperture");
            }
            if function.is_some() && lines.next() != Some("%TD.AperFunction*%") {
                return Err("aperture function not cleared");
            }
            continue;
        }
        body = true;
        if line.starts_with("%TO.") || line.starts_with("%TD.") {
            if !copper || region.is_some() {
                return Err("object attribute outside copper object");
            }
            if line.starts_with("%TO.") {
                object.set(line, outer)?;
            } else {
                object.clear(line)?;
            }
        } else if let Some(code) = line.strip_prefix('D') {
            if region.is_some() {
                return Err("selection in region");
            }
            let code = code.strip_suffix('*').and_then(read_aperture_code).ok_or("invalid selection")?;
            if !read.apertures.contains_key(&code) && !read.rounded.contains_key(&code) {
                return Err("undefined aperture");
            }
            selected = Some(code);
        } else if line == "G36*" {
            if region.replace(Vec::new()).is_some() {
                return Err("nested region");
            }
            let function = pending_function.take().ok_or("missing region function")?;
            read.region_functions.push(function);
            read.objects.push((Some(function), object.clone()));
        } else if line == "G37*" {
            let points = region.take().ok_or("region end without start")?;
            if points.len() < 4 || points.first() != points.last() {
                return Err("region contour not closed");
            }
            read.regions.push(points);
            if lines.next() != Some("%TD.AperFunction*%") {
                return Err("region function not cleared");
            }
        } else if let Some(coordinates) = line.strip_prefix('X') {
            let (x, rest) = coordinates.split_once('Y').ok_or("invalid coordinate")?;
            let (y, operation) = rest.split_once('D').ok_or("invalid coordinate")?;
            let point = Point { x_nm: read_coordinate(x)?, y_nm: read_coordinate(y)? };
            if let Some(points) = region.as_mut() {
                if operation != if points.is_empty() { "02*" } else { "01*" } {
                    return Err("invalid region operation");
                }
                points.push(point);
            } else {
                match operation {
                    "03*" => {
                        let code = selected.ok_or("undefined aperture")?;
                        let function = if let Some(&aperture) = read.apertures.get(&code) {
                            read.flashes.push((point, aperture));
                            aperture.function
                        } else {
                            read.rounded_flashes.push((point, code));
                            read.rounded[&code].function
                        };
                        read.objects.push((function, object.clone()));
                    }
                    "01*" => {
                        let aperture = *read.apertures.get(&selected.ok_or("undefined aperture")?).ok_or("unsupported stroke aperture")?;
                        if aperture.kind != 'C' {
                            return Err("unsupported stroke aperture");
                        }
                        read.strokes.push((position.ok_or("draw without position")?, point, aperture));
                        read.objects.push((aperture.function, object.clone()));
                    }
                    "02*" => (),
                    _ => return Err("unsupported coordinate operation"),
                }
            }
            position = Some(point);
        } else {
            return Err("unsupported record");
        }
    }
    Err("missing end record")
}

/// Parse an unsigned D-code of at least 10 with no sign or leading zeroes.
fn read_aperture_code(value: &str) -> Option<usize> {
    let code = value.parse::<usize>().ok()?;
    (code >= 10 && code.to_string() == value).then_some(code)
}

/// Filename suffix and independent expected X2 file function and polarity.
type GerberFile = (&'static str, &'static str, &'static str);
const TOP_COPPER: GerberFile = ("-Top.gbr", "Copper,L1,Top", "Positive");
const BOTTOM_PASTE: GerberFile = ("-Paste_Bottom.gbr", "Paste,Bot", "Positive");
const TOP: [GerberFile; 3] = [TOP_COPPER, ("-Mask_Top.gbr", "Soldermask,Top", "Negative"), ("-Paste_Top.gbr", "Paste,Top", "Positive")];
const BOTTOM: [GerberFile; 3] = [("-Bottom.gbr", "Copper,L2,Bot", "Positive"), ("-Mask_Bottom.gbr", "Soldermask,Bot", "Negative"), BOTTOM_PASTE];

/// Return the two-layer fixture's copper, mask and paste expectations in that order
/// for the given side. Bottom copper is L2.
fn surface_files(bottom: bool) -> [GerberFile; 3] {
    if bottom { BOTTOM } else { TOP }
}

fn read_gerber(outputs: &[Output], (suffix, function, polarity): GerberFile) -> ReadGerber {
    try_read_gerber(file(outputs, suffix), function, polarity).unwrap_or_else(|error| panic!("{suffix}: {error}"))
}

/// A mixed, command-built package uses C/R/O and a rounded macro in one file, including the swapped rectangle.
/// Ucamco Gerber Layer Format Specification 2026.05 section 4.4 defines these standard templates.
#[test]
fn mixed_pad_shapes_use_standard_flashes_with_resolved_sizes() {
    let s = aperture_fixture_with_pads(
        &[
            (PadShape::Circle, 400_000, 400_000, 0),
            (PadShape::Rectangle, 800_000, 400_000, 0),
            (PadShape::Rectangle, 800_000, 400_000, 90),
            (PadShape::Oval, 800_000, 400_000, 0),
            (PadShape::RoundedRectangle, 800_000, 400_000, 0),
        ],
        0,
        false,
    );
    let outputs = view(&s).fabrication_outputs().unwrap();
    let read = read_gerber(&outputs, TOP_COPPER);
    assert!(read.strokes.is_empty() && read.regions.is_empty());
    assert_eq!(read.apertures.len(), 4);
    assert_eq!(read.rounded.len(), 1);
    assert_eq!(read.rounded[&14].function, Some("SMDPad,CuDef"));
    assert_eq!(read.rounded_flashes, [(Point { x_nm: 38_000_001, y_nm: 25_000_003 }, 14)]);
    let expected = [('C', 400_000, 400_000), ('R', 800_000, 400_000), ('R', 400_000, 800_000), ('O', 800_000, 400_000)];
    assert_eq!(read.flashes.len(), expected.len());
    for (((center, aperture), pad), (kind, width, height)) in read.flashes.iter().zip(view(&s).footprints().next().unwrap().pads).zip(expected) {
        assert_eq!(*center, Point { x_nm: pad.center.x_nm, y_nm: -pad.center.y_nm });
        assert_eq!(*aperture, ReadAperture { kind, width, height, function: Some("SMDPad,CuDef") });
    }
    assert_eq!(outputs, view(&s).fabrication_outputs().unwrap());
    assert_eq!(zip(&outputs).unwrap(), zip(&view(&s).fabrication_outputs().unwrap()).unwrap());
}

/// Untransformed standard apertures retain global X/Y dimensions (Ucamco Gerber 2026.05 section 4.4).
/// Rectangle flash extents and half-grid fallback regions match the resolved corners exactly;
/// equal-parity obround flashes match the land capsule, while mixed-parity obrounds retain
/// the existing rounded-endpoint stroke rather than claiming exact contour equivalence.
#[test]
fn standard_pad_apertures_read_back_on_copper_mask_and_paste() {
    for (shape, width, height, kind) in [
        (PadShape::Circle, 400_001, 400_001, 'C'),
        (PadShape::Rectangle, 800_000, 400_002, 'R'),
        (PadShape::Rectangle, 800_001, 400_003, 'R'),
        (PadShape::Rectangle, 800_000, 400_003, 'R'),
        (PadShape::Oval, 400_003, 800_001, 'O'),
        (PadShape::Oval, 400_001, 400_001, 'O'),
        (PadShape::Rectangle, 1, 2, 'R'),
        (PadShape::Oval, 2, 1, 'O'),
    ] {
        for (pad_rotation, rotation) in [(0, 0), (90, 0), (0, 90), (90, 90), (0, 270)] {
            for bottom in [false, true] {
                let s = aperture_fixture(shape, width, height, pad_rotation, rotation, bottom);
                let outputs = view(&s).fabrication_outputs().unwrap();
                let pads: Vec<_> = view(&s).footprints().flat_map(|f| f.pads).collect();
                for (output, function) in surface_files(bottom).into_iter().zip(["SMDPad,CuDef", "Material", "Material"]) {
                    let read = read_gerber(&outputs, output);
                    assert!(read.region_functions.iter().all(|&actual| actual == function));
                    if shape == PadShape::Oval && width % 2 != height % 2 {
                        let function = (function == "Material").then_some(function);
                        assert!(read.flashes.is_empty() && read.regions.is_empty());
                        assert_eq!(read.strokes.len(), pads.len());
                        for ((a, b, aperture), pad) in read.strokes.iter().zip(&pads) {
                            let corners = pad.copper.unwrap();
                            let x = (corners[1].x_nm - corners[0].x_nm).abs() + (corners[2].x_nm - corners[1].x_nm).abs();
                            let y = (corners[1].y_nm - corners[0].y_nm).abs() + (corners[2].y_nm - corners[1].y_nm).abs();
                            assert_eq!(*aperture, ReadAperture { kind: 'C', width: x.min(y), height: x.min(y), function });
                            assert_eq!((a.x_nm + b.x_nm, a.y_nm + b.y_nm), (2 * pad.center.x_nm, -2 * pad.center.y_nm));
                            let span = (x - y).abs() + 1;
                            assert_eq!(((a.x_nm - b.x_nm).abs(), (a.y_nm - b.y_nm).abs()), if x > y { (span, 0) } else { (0, span) });
                        }
                        continue;
                    }
                    assert!(read.strokes.is_empty());
                    assert!(read.apertures.values().all(|aperture| aperture.function == Some(function)));
                    if shape == PadShape::Rectangle && (width % 2 != 0 || height % 2 != 0) {
                        assert!(read.flashes.is_empty() && read.apertures.is_empty());
                        assert_eq!(read.regions.len(), pads.len());
                        for (region, pad) in read.regions.iter().zip(&pads) {
                            let corners = pad.copper.unwrap();
                            let expected: Vec<_> =
                                corners.iter().chain(std::iter::once(&corners[0])).map(|p| Point { x_nm: p.x_nm, y_nm: -p.y_nm }).collect();
                            assert_eq!(*region, expected, "half-grid rectangle keeps its exact contour");
                        }
                        continue;
                    }
                    assert!(read.regions.is_empty());
                    assert_eq!(read.apertures.len(), 1, "equal pads share an aperture");
                    assert_eq!(read.flashes.len(), pads.len());
                    for ((center, aperture), pad) in read.flashes.iter().zip(&pads) {
                        assert_eq!(*center, Point { x_nm: pad.center.x_nm, y_nm: -pad.center.y_nm });
                        assert_eq!(aperture.kind, kind);
                        let expected_size = if (pad_rotation / 90 + rotation / 90).is_multiple_of(2) { (width, height) } else { (height, width) };
                        assert_eq!((aperture.width, aperture.height), expected_size, "final axis order, including two quarter turns and mirror");
                        let corners = pad.copper.unwrap();
                        let (min_x, max_x) = (corners.iter().map(|p| p.x_nm).min().unwrap(), corners.iter().map(|p| p.x_nm).max().unwrap());
                        let (min_y, max_y) = (corners.iter().map(|p| p.y_nm).min().unwrap(), corners.iter().map(|p| p.y_nm).max().unwrap());
                        assert_eq!((aperture.width, aperture.height), (max_x - min_x, max_y - min_y));
                        if shape == PadShape::Rectangle {
                            assert_eq!(2 * center.x_nm - aperture.width, 2 * min_x);
                            assert_eq!(2 * center.x_nm + aperture.width, 2 * max_x);
                            assert_eq!(2 * center.y_nm - aperture.height, -2 * max_y);
                            assert_eq!(2 * center.y_nm + aperture.height, -2 * min_y);
                        }
                    }
                }
                for output in surface_files(!bottom) {
                    let opposite = read_gerber(&outputs, output);
                    assert!(opposite.flashes.is_empty() && opposite.regions.is_empty() && opposite.strokes.is_empty(), "{}", output.0);
                }
            }
        }
    }
}

/// Shape and X/Y order are part of aperture identity, even when numerical sizes coincide.
/// A round stroke after a rectangular/obround flash must reselect a circular aperture.
#[test]
fn aperture_identity_and_selection_retain_shape_and_orientation() {
    let mut gerber = Gerber::new(Frame::BOARD_ZERO);
    let point = Point { x_nm: 0, y_nm: 0 };
    for aperture in [
        Aperture::Circle(400_000),
        Aperture::Rectangle(400_000, 400_000),
        Aperture::Obround(400_000, 400_000),
        Aperture::Rectangle(800_000, 400_000),
        Aperture::Rectangle(400_000, 800_000),
        Aperture::Obround(800_000, 400_000),
        Aperture::Obround(400_000, 800_000),
    ] {
        gerber.aperture(aperture, Some(ApertureFunction::Material));
        gerber.body.push_str("X0Y0D03*\n");
    }
    gerber.stroke(&[point, Point { x_nm: 1_000_000, y_nm: 0 }], 400_000, Some(ApertureFunction::Material));
    let text = gerber.finish("Soldermask,Top", "Negative");
    let read = try_read_gerber(&text, "Soldermask,Top", "Negative").unwrap();
    assert_eq!(read.apertures.len(), 7);
    assert_eq!(read.flashes.len(), 7);
    assert_eq!(
        read.flashes,
        [
            ('C', 400_000, 400_000),
            ('R', 400_000, 400_000),
            ('O', 400_000, 400_000),
            ('R', 800_000, 400_000),
            ('R', 400_000, 800_000),
            ('O', 800_000, 400_000),
            ('O', 400_000, 800_000),
        ]
        .map(|(kind, width, height)| (point, ReadAperture { kind, width, height, function: Some("Material") }))
    );
    assert_eq!(
        read.strokes,
        [(point, Point { x_nm: 1_000_000, y_nm: 0 }, ReadAperture { kind: 'C', width: 400_000, height: 400_000, function: Some("Material") })]
    );
    assert_eq!(read.strokes[0].2.kind, 'C');
    assert_eq!(read.strokes[0].2.width, 400_000);
    assert_eq!(text.matches("D10*\n").count(), 2, "circle is selected again after the last O flash");
}

/// Non-cardinal rectangles retain their exact polygon and obrounds their existing round stroke.
#[test]
fn rotated_rectangular_and_obround_pad_fallbacks_retain_geometry() {
    for shape in [PadShape::Rectangle, PadShape::Oval] {
        for (pad_rotation, rotation) in [(45, 0), (0, 37), (13, 90)] {
            for bottom in [false, true] {
                let s = aperture_fixture(shape, 800_000, 400_000, pad_rotation, rotation, bottom);
                let outputs = view(&s).fabrication_outputs().unwrap();
                assert_eq!(outputs, view(&s).fabrication_outputs().unwrap());
                assert_eq!(zip(&outputs).unwrap(), zip(&view(&s).fabrication_outputs().unwrap()).unwrap());
                let read = read_gerber(&outputs, surface_files(bottom)[2]);
                let pads: Vec<_> = view(&s).footprints().flat_map(|f| f.pads).collect();
                assert!(read.flashes.is_empty());
                if shape == PadShape::Oval {
                    assert!(read.regions.is_empty());
                    assert_eq!(read.strokes.len(), pads.len());
                    for ((a, b, aperture), pad) in read.strokes.iter().zip(pads) {
                        assert_eq!(aperture.kind, 'C');
                        assert!((aperture.width - 400_000).abs() <= 1);
                        assert_eq!(a.x_nm + b.x_nm, 2 * pad.center.x_nm);
                        assert_eq!(a.y_nm + b.y_nm, -2 * pad.center.y_nm);
                        let corners = pad.copper.unwrap();
                        let (dx, dy) = ((corners[1].x_nm - corners[0].x_nm) as f64, -(corners[1].y_nm - corners[0].y_nm) as f64);
                        let length = dx.hypot(dy);
                        // Existing endpoint and projected-corner rounding each contributes <= 1 nm.
                        assert!(((b.x_nm - a.x_nm) as f64 - dx * 400_000. / length).abs() <= 2.);
                        assert!(((b.y_nm - a.y_nm) as f64 - dy * 400_000. / length).abs() <= 2.);
                    }
                } else {
                    assert!(read.strokes.is_empty());
                    assert_eq!(read.regions.len(), pads.len());
                    for (region, pad) in read.regions.iter().zip(pads) {
                        assert_eq!(region.first(), region.last());
                        let corners = pad.copper.unwrap();
                        let expected: Vec<_> =
                            corners.iter().chain(std::iter::once(&corners[0])).map(|p| Point { x_nm: p.x_nm, y_nm: -p.y_nm }).collect();
                        assert_eq!(*region, expected);
                    }
                }
            }
        }
    }
}

/// Read exact corner diameters and verify the whole union: four circles centered on the
/// inner contour, one diameter-wide strip along each edge, and a filled closed inner outline.
/// Ucamco Gerber 2026.05 §§4.5.1.3, 4.5.1.4 and 4.5.1.6 specify the parsed primitives.
#[test]
fn rounded_pad_macros_read_back_radius_and_orientation() {
    for (local, placed) in [(0, 0), (0, 90), (0, 37), (37, 0), (13, 37)] {
        for bottom in [false, true] {
            let mut s = aperture_fixture(PadShape::RoundedRectangle, 800_000, 400_000, local, placed, bottom);
            s.apply(crate::SET_PLACEMENT_ORIGIN, &json!({"board":s.project().unwrap().board_target().unwrap().board,"origin":{"kind":"point","x_nm":1_000_003,"y_nm":-2_000_007},"expected_revision":s.revision()})).unwrap();
            let fabrication = view(&s).fabrication().unwrap();
            if (local, placed) == (0, 37) {
                let revision = s.revision();
                let returned = s.execute(EXPORT_FABRICATION, &json!({})).unwrap();
                let capture = s.export_fabrication_archive(&json!({})).unwrap();
                assert_eq!(
                    returned["fabrication"],
                    json!({"files": fabrication.files, "origin": fabrication.origin, "manifest": embedded_manifest(&capture)})
                );
                assert_eq!(s.revision(), revision);
                assert_eq!(zip(&fabrication.files).unwrap(), zip(&view(&s).fabrication_outputs().unwrap()).unwrap());
            }
            let pads = view(&s).footprints().next().unwrap().pads;
            let expected_angle = if bottom { f64::from(placed) - f64::from(local) } else { f64::from(placed) + f64::from(local) };
            let (sin, cos) = expected_angle.to_radians().sin_cos();
            for output in surface_files(bottom) {
                let read = read_gerber(&fabrication.files, output);
                assert!(read.regions.is_empty() && read.strokes.is_empty() && read.flashes.is_empty(), "{}", output.0);
                assert_eq!(read.rounded_flashes.len(), pads.len());
                if placed.is_multiple_of(90) {
                    assert_eq!(read.rounded.len(), 1, "identical projected geometry reuses its aperture");
                }
                for (&(at, code), pad) in read.rounded_flashes.iter().zip(&pads) {
                    assert_eq!(at, Point { x_nm: pad.center.x_nm - 1_000_003, y_nm: -(pad.center.y_nm + 2_000_007) });
                    assert_eq!(pad.corner_radius_nm, 100_000);
                    let aperture = &read.rounded[&code];
                    assert_eq!((aperture.circles.len(), aperture.strips.len(), aperture.outline.len()), (4, 4, 5));
                    assert!(aperture.circles.iter().all(|(d, _)| *d == 2 * pad.corner_radius_nm), "exact authored radius in nanometers");
                    let vertices = &aperture.outline[..4];
                    for (&vertex, (_, circle)) in vertices.iter().zip(&aperture.circles) {
                        assert_eq!(vertex, *circle);
                    }
                    for ((&start, &end), strip) in aperture.outline.iter().zip(&aperture.outline[1..]).zip(&aperture.strips) {
                        assert_eq!(*strip, (2 * pad.corner_radius_nm, start, end));
                    }
                    // Check the model-to-file frame independently of the emitter. The model
                    // rounds each rotation and the final inner vertices, not the Gerber radius.
                    for (x, y) in [(-300_000., -100_000.), (-300_000., 100_000.), (300_000., -100_000.), (300_000., 100_000.)] {
                        let (x, y) = (x * cos - y * sin, x * sin + y * cos);
                        assert!(vertices.iter().any(|p| (p.x_nm as f64 - x).abs() <= 2. && (p.y_nm as f64 - y).abs() <= 2.));
                    }
                }
            }
            for output in surface_files(!bottom) {
                assert!(read_gerber(&fabrication.files, output).rounded_flashes.is_empty());
            }
        }
    }
}

/// The maximum legal radius fills a point core as one disk, or a line core as a capsule.
#[test]
fn rounded_pad_macros_handle_point_and_line_cores() {
    for width in [2, 4] {
        let s = aperture_fixture(PadShape::RoundedRectangle, width, 2, 0, 0, false);
        let outputs = view(&s).fabrication_outputs().unwrap();
        let read = read_gerber(&outputs, TOP_COPPER);
        assert!(read.regions.is_empty());
        assert_eq!(read.rounded.len(), 1);
        assert_eq!(read.rounded_flashes.len(), 2);
        let aperture = &read.rounded[&read.rounded_flashes[0].1];
        assert!(aperture.outline.is_empty());
        if width == 2 {
            assert_eq!(aperture.circles, [(2, Point { x_nm: 0, y_nm: 0 })]);
            assert!(aperture.strips.is_empty());
        } else {
            let (start, end) = (Point { x_nm: -1, y_nm: 0 }, Point { x_nm: 1, y_nm: 0 });
            assert_eq!(aperture.circles, [(2, start), (2, end)]);
            assert_eq!(aperture.strips, [(2, start, end)]);
        }
    }
}

/// Assert exact refusal reasons for wrong X2 metadata, unsupported modes, noncanonical
/// aperture spellings and misordered or structurally unclosed contours.
#[test]
fn gerber_subset_reader_rejects_header_syntax_and_order_mutations() {
    let s = aperture_fixture_with_pads(
        &[(PadShape::Circle, 400_000, 400_000, 0), (PadShape::Rectangle, 800_000, 400_000, 0), (PadShape::Oval, 800_000, 400_000, 0)],
        0,
        false,
    );
    let outputs = view(&s).fabrication_outputs().unwrap();
    let (suffix, function, polarity) = TOP_COPPER;
    let text = file(&outputs, suffix);
    assert_eq!(read_gerber(&outputs, TOP_COPPER).flashes.len(), 3);
    let first_definition = text.lines().find(|line| line.starts_with("%ADD")).unwrap();
    let set = "%TA.AperFunction,SMDPad,CuDef*%\n";
    let clear = "%TD.AperFunction*%\n";
    let definition_block = format!("{set}{first_definition}\n{clear}");
    for (mutant, reason) in [
        (text.replace("%TF.SameCoordinates,Original*%\n", ""), "invalid header"),
        (text.replace("%TF.FileFunction,Copper,L1,Top*%", "%TF.FileFunction,Unknown*%"), "invalid file function"),
        (text.replace("%TF.FileFunction,Copper,L1,Top*%", "%TF.FileFunction,Legend,Top*%"), "wrong file function"),
        (text.replace("Copper,L1,Top", "Copper,L1,Bot"), "wrong file side"),
        (text.replace("Copper,L1,Top", "Copper,L2,Top"), "wrong copper ordinal"),
        (text.replace("%TF.FilePolarity,Positive*%", "%TF.FilePolarity,Negative*%"), "invalid file polarity"),
        (text.replace("%MOMM*%", "%MOIN*%"), "unsupported coordinate mode"),
        (text.replace("%FSLAX46Y46*%", "%FSLAX45Y45*%"), "unsupported coordinate mode"),
        (text.replace("G01*", "G02*"), "unsupported coordinate mode"),
        (text.replace("%LPD*%", "%LPC*%"), "unsupported coordinate mode"),
        (text.replace("G01*\n", "G01*\n%LPC*%\n"), "unsupported record"),
        (text.replace("G01*\n", "G01*\n%LR90*%\n"), "unsupported record"),
        (text.replace("G01*\n", "G01*\n%LS2*%\n"), "unsupported record"),
        (text.replace("G01*\n", "G01*\n%LMX*%\n"), "unsupported record"),
        (text.replace("R,0.800000", "R,-0.800000"), "invalid aperture dimension"),
        (text.replace("R,0.800000", "R,-0.000001"), "invalid aperture dimension"),
        (text.replace("R,0.800000", "R,0.000000"), "invalid aperture dimension"),
        (text.replace("R,0.800000", "R,0.80000"), "invalid aperture dimension"),
        (text.replace("R,0.800000", "R,0.800000X0.100000"), "invalid aperture arity"),
        (text.replace("\nD10*\n", "\nD99*\n"), "undefined aperture"),
        (text.replace("\nD10*\n", "\nD+10*\n"), "invalid selection"),
        (text.replace("\nD10*\n", "\nD010*\n"), "invalid selection"),
        (text.replace("%ADD10", "%ADD+10").replace("\nD10*\n", "\nD+10*\n"), "invalid aperture code"),
        (text.replace("%ADD10", "%ADD010"), "invalid aperture code"),
        (text.replacen(&definition_block, &definition_block.repeat(2), 1), "duplicate aperture"),
        (text.replace("M02*", &format!("{definition_block}M02*")), "late aperture definition"),
        (text.replacen(set, "", 1), "invalid object attribute delete"),
        (text.replacen(clear, "", 1), "aperture function not cleared"),
        (text.replacen("SMDPad,CuDef", "SMDPad", 1), "unsupported aperture function"),
        (text.replace("M02*", &format!("{set}M02*")), "aperture function outside definition or region"),
        (text.replace("M02*\n", ""), "missing end record"),
        (format!("{text}G01*\n"), "records after end"),
    ] {
        assert_ne!(mutant, text, "mutation must affect fixture");
        assert_eq!(try_read_gerber(&mutant, function, polarity).unwrap_err(), reason);
    }
    let bottom = aperture_fixture(PadShape::Circle, 400_000, 400_000, 0, 0, true);
    let outputs = view(&bottom).fabrication_outputs().unwrap();
    let (paste_suffix, paste_function, paste_polarity) = BOTTOM_PASTE;
    let paste = file(&outputs, paste_suffix);
    assert_eq!(read_gerber(&outputs, BOTTOM_PASTE).flashes.len(), 2);
    for (replacement, reason) in [("Paste,Top", "wrong file side"), ("Legend,Top", "wrong file function")] {
        let mutant = paste.replace(paste_function, replacement);
        assert_ne!(mutant, paste, "mutation must affect fixture");
        assert_eq!(try_read_gerber(&mutant, paste_function, paste_polarity).unwrap_err(), reason);
    }
    assert_eq!(read_mm("0.000001"), Ok(1));
    assert_eq!(read_mm("100.000000"), Ok(100_000_000));
    assert_eq!(read_mm("9223372036854.775807"), Ok(i64::MAX));
    assert_eq!(read_mm("9223372036854.775808"), Err("invalid aperture dimension"));
    for coordinate in ["0", "1", "-1", "9999999999", "-9999999999"] {
        assert_eq!(read_coordinate(coordinate), Ok(coordinate.parse().unwrap()));
    }
    for invalid in ["10000000000", "-10000000000", "9223372036854775808", "+1", "01", "-0", "1.0", ""] {
        assert_eq!(read_coordinate(invalid), Err("invalid coordinate"));
    }
    for invalid in ["-0.000001", "-0.000000", "+0.000001", "0.000000", "0.0000010", "0.00000x", "9223372036854775808.000000"] {
        assert_eq!(read_mm(invalid), Err("invalid aperture dimension"));
    }
    let s = aperture_fixture(PadShape::Rectangle, 800_001, 400_003, 0, 0, false);
    let outputs = view(&s).fabrication_outputs().unwrap();
    let text = file(&outputs, suffix);
    assert_eq!(read_gerber(&outputs, TOP_COPPER).regions.len(), 2);
    let start = text.lines().find(|line| line.ends_with("D02*")).unwrap().strip_suffix("D02*").unwrap();
    // Remove the first region's close and all records before the next region opens.
    let (_, after_close) = text.split_once("G37*\n").unwrap();
    let between_regions = format!("G37*\n{}", after_close.split_once("G36*\n").unwrap().0);
    for (mutant, reason) in [
        (text.replacen(&format!("{start}D01*"), "X0Y0D01*", 1), "region contour not closed"),
        (format!("{}M02*\n", text.split_once("G37*\n").unwrap().0), "unclosed region"),
        (text.replace("D01*", "D02*"), "invalid region operation"),
        (text.replacen("D02*", "D01*", 1), "invalid region operation"),
        (text.replacen("D01*", "D03*", 1), "invalid region operation"),
        (text.replacen(&between_regions, "", 1), "nested region"),
        (text.replace("G36*", "G36*\nG36*"), "nested region"),
        (text.replacen(set, "", 1), "missing region function"),
        (text.replacen(clear, "", 1), "region function not cleared"),
        (text.replacen("G36*\n", &format!("G36*\n{set}"), 1), "attribute in region"),
    ] {
        assert_ne!(mutant, text, "mutation must affect fixture");
        assert_eq!(try_read_gerber(&mutant, function, polarity).unwrap_err(), reason);
    }
}

/// Reject wrong macro exposure, nonzero rotation, or a flash using an undefined template.
#[test]
fn gerber_subset_reader_rejects_invalid_rounded_macro_parameters() {
    let s = aperture_fixture(PadShape::RoundedRectangle, 800_000, 400_000, 0, 0, false);
    let outputs = view(&s).fabrication_outputs().unwrap();
    let (suffix, function, polarity) = TOP_COPPER;
    let text = file(&outputs, suffix);
    assert_eq!(read_gerber(&outputs, TOP_COPPER).rounded_flashes.len(), 2);
    for (mutant, reason) in [
        (text.replacen("1,1,0.200000,", "1,0,0.200000,", 1), "unsupported macro primitive"),
        (text.replacen(",0*\n", ",90*\n", 1), "unsupported macro primitive"),
        (text.replace("%ADD10RoundRect10*%", "%ADD10RoundRect99*%"), "undefined macro"),
    ] {
        assert_ne!(mutant, text);
        assert_eq!(try_read_gerber(&mutant, function, polarity).unwrap_err(), reason);
    }
}

/// Snapped boxes of each shape are equal within one placed definition, but each pad's
/// local rotation independently decides whether it can flash R or O.
#[test]
fn aperture_eligibility_is_per_pad_within_one_footprint() {
    let s = aperture_fixture_with_pads(
        &[(PadShape::Rectangle, 2, 2, 0), (PadShape::Rectangle, 2, 2, 1), (PadShape::Oval, 2, 4, 0), (PadShape::Oval, 2, 4, 1)],
        0,
        false,
    );
    let pads = view(&s).footprints().next().unwrap().pads;
    assert_eq!(
        pads.iter().map(|pad| axis_aligned_size(&pad.copper.unwrap())).collect::<Vec<_>>(),
        [Some((2, 2)), Some((2, 2)), Some((2, 4)), Some((2, 4))]
    );
    let [rect, rotated_rect, oval, rotated_oval] = [30_000_001, 32_000_001, 34_000_001, 36_000_001].map(|x_nm| Point { x_nm, y_nm: 25_000_003 });
    let outputs = view(&s).fabrication_outputs().unwrap();
    let read = read_gerber(&outputs, TOP_COPPER);
    assert_eq!(
        read.flashes,
        [
            (rect, ReadAperture { kind: 'R', width: 2, height: 2, function: Some("SMDPad,CuDef") }),
            (oval, ReadAperture { kind: 'O', width: 2, height: 4, function: Some("SMDPad,CuDef") })
        ]
    );
    assert_eq!(read.regions.len(), 1);
    assert_eq!(
        read.regions[0],
        [(-1, 1), (1, 1), (1, -1), (-1, -1), (-1, 1)].map(|(dx, dy)| Point { x_nm: rotated_rect.x_nm + dx, y_nm: rotated_rect.y_nm + dy })
    );
    assert_eq!(
        read.strokes,
        [(
            Point { y_nm: rotated_oval.y_nm + 1, ..rotated_oval },
            Point { y_nm: rotated_oval.y_nm - 1, ..rotated_oval },
            ReadAperture { kind: 'C', width: 2, height: 2, function: None },
        )]
    );
}

/// A legal 1 nm radius clamps to about 0.707 nm after the corners of a rotated 2×2 nm pad
/// snap to a diamond. Its point core flashes as the nearest whole-nanometre disk.
#[test]
fn tiny_rounded_pad_flashes_its_clamped_point_core() {
    let s = aperture_fixture(PadShape::RoundedRectangle, 2, 2, 45, 0, false);
    let pads = view(&s).footprints().next().unwrap().pads;
    assert_eq!(pads.len(), 2);
    assert!(pads.iter().all(|pad| pad.corner_radius_nm == 1));
    let contour = Contour::pad(pads[0].center, pads[0].copper.unwrap(), pads[0].shape, pads[0].corner_radius_nm, pads[0].size_nm);
    assert!((contour.radius - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-9);
    let outputs = view(&s).fabrication_outputs().unwrap();
    let read = read_gerber(&outputs, TOP_COPPER);
    assert!(read.apertures.is_empty() && read.flashes.is_empty() && read.strokes.is_empty() && read.regions.is_empty());
    assert_eq!(read.rounded.len(), 1);
    let at: Vec<_> = read.rounded_flashes.iter().map(|&(at, _)| at).collect();
    assert_eq!(at, [30_000_001, 32_000_001].map(|x_nm| Point { x_nm, y_nm: 25_000_003 }));
    let aperture = &read.rounded[&read.rounded_flashes[0].1];
    assert_eq!(aperture.circles, [(1, Point { x_nm: 0, y_nm: 0 })]);
    assert!(aperture.strips.is_empty() && aperture.outline.is_empty());
}

/// A maximum radius on a rotated 0.6 × 0.4 mm pad clamps below 200,000 nm when its corners
/// snap. It still flashes circular corners, within 1 nm of the authored and model radii.
/// Includes one- and two-stage projections; the latter can cross the authored tolerance
/// before diameter rounding but meet it after rounding.
#[test]
fn rotated_maximum_radius_pads_keep_circular_corners() {
    for (local, placed, bottom, offset, first_diameter) in [
        (45, 0, false, (0, 0), 399_999),
        (0, 45, false, (0, 0), 399_999),
        (45, 0, true, (0, 0), 399_999),
        (13, 0, false, (0, 0), 400_000),
        (30, 0, false, (0, 0), 400_000),
        (18, 32, false, (3, 11), 399_998),
    ] {
        let pads = [(PadShape::RoundedRectangle, 600_000, 400_000, local, 200_000); 2];
        let offset = Point { x_nm: offset.0, y_nm: offset.1 };
        let s = aperture_fixture_with_radii(&pads, placed, bottom, offset, Point { x_nm: 30_000_001, y_nm: -25_000_003 }, None);
        let fabrication = view(&s).fabrication().unwrap();
        let pads = view(&s).footprints().next().unwrap().pads;
        let contours: Vec<_> =
            pads.iter().map(|pad| Contour::pad(pad.center, pad.copper.unwrap(), pad.shape, pad.corner_radius_nm, pad.size_nm)).collect();
        // The second pad's different local center can cause different placement rounding.
        assert!(contours[0].radius < 200_000., "the first pad must clamp the radius");
        if (local, placed) == (18, 32) {
            assert!(contours[0].radius < 199_999., "the model radius must miss the authored tolerance before diameter rounding");
        }
        let expected_angle = if bottom { f64::from(placed) - f64::from(local) } else { f64::from(placed) + f64::from(local) };
        let (sin, cos) = expected_angle.to_radians().sin_cos();
        for output in surface_files(bottom) {
            let read = read_gerber(&fabrication.files, output);
            assert!(read.regions.is_empty() && read.strokes.is_empty() && read.flashes.is_empty(), "{} at {local}/{placed}", output.0);
            assert_eq!(read.rounded_flashes.len(), pads.len());
            assert_eq!(read.rounded[&read.rounded_flashes[0].1].circles[0].0, first_diameter);
            for ((&(at, code), pad), contour) in read.rounded_flashes.iter().zip(&pads).zip(&contours) {
                assert_eq!(at, Point { x_nm: pad.center.x_nm, y_nm: -pad.center.y_nm });
                let aperture = &read.rounded[&code];
                assert!(aperture.outline.is_empty(), "the short sides meet, leaving a line core");
                assert_eq!(aperture.circles.len(), 2);
                let diameter = aperture.circles[0].0;
                assert!(diameter.abs_diff(2 * pad.corner_radius_nm) <= 2, "authored radius within 1 nm");
                assert!((diameter as f64 / 2. - contour.radius).abs() <= 0.25, "diameter rounded to the nearest 1 nm");
                let [(_, start), (_, end)] = [aperture.circles[0], aperture.circles[1]];
                assert!(aperture.circles.iter().all(|&(d, _)| d == diameter));
                assert_eq!(aperture.strips, [(diameter, start, end)]);
                // The core runs along the long axis, 100,000 nm each side of the center.
                let (x, y) = (100_000. * cos, 100_000. * sin);
                // One endpoint must reach each end, in either order, so a collapsed core fails.
                let near = |p: Point, s: f64| (p.x_nm as f64 - s * x).abs() <= 2. && (p.y_nm as f64 - s * y).abs() <= 2.;
                assert!((near(start, -1.) && near(end, 1.)) || (near(start, 1.) && near(end, -1.)), "{start:?}..{end:?} at {local}/{placed}");
            }
        }
    }
}

/// A core narrower than the corner diameter is covered by its edge strips alone, so the
/// macro writes no sliver outline that coordinate perturbation could make self-intersect.
/// Ucamco Gerber 2026.05 §§4.5.1.6 and 4.10.3.
#[test]
fn rounded_pad_macros_omit_the_outline_of_a_core_narrower_than_the_diameter() {
    for local in [0, 90] {
        let pads = [(PadShape::RoundedRectangle, 600_000, 400_000, local, 199_999); 2];
        let s = aperture_fixture_with_radii(&pads, 0, false, Point { x_nm: 0, y_nm: 0 }, Point { x_nm: 30_000_001, y_nm: -25_000_003 }, None);
        let read = read_gerber(&view(&s).fabrication_outputs().unwrap(), TOP_COPPER);
        assert!(read.regions.is_empty() && read.strokes.is_empty() && read.flashes.is_empty());
        assert_eq!(read.rounded_flashes.len(), 2);
        let aperture = &read.rounded[&read.rounded_flashes[0].1];
        assert!(aperture.outline.is_empty(), "a 2 nm core needs no outline at {local}");
        assert_eq!((aperture.circles.len(), aperture.strips.len()), (4, 4));
        assert!(aperture.circles.iter().all(|&(d, _)| d == 399_998));
        assert!(aperture.strips.iter().all(|&(d, _, _)| d == 399_998));
    }
}

/// When opposite strips overlap by at most the Gerber §4.14.2 coordinate and size tolerance
/// bound (1 µm edge shift plus 0.5 µm diameter shrinkage = 1.5 µm on axis-aligned edges),
/// perturbations could open a gap through the middle, so the macro keeps its outline.
/// Once the overlap exceeds 1.5 µm, strips alone stay closed and the outline is omitted.
#[test]
fn rounded_pad_macros_keep_the_outline_when_strip_overlap_is_too_small() {
    for local in [0, 90] {
        for (height, keep_outline) in [(199_996, true), (198_998, true), (198_500, true), (198_498, false)] {
            let pads = [(PadShape::RoundedRectangle, 600_000, height, local, 50_000); 2];
            let s = aperture_fixture_with_radii(&pads, 0, false, Point { x_nm: 0, y_nm: 0 }, Point { x_nm: 30_000_001, y_nm: -25_000_003 }, None);
            let read = read_gerber(&view(&s).fabrication_outputs().unwrap(), TOP_COPPER);
            assert!(read.regions.is_empty() && read.strokes.is_empty() && read.flashes.is_empty());
            assert_eq!(read.rounded_flashes.len(), 2);
            let aperture = &read.rounded[&read.rounded_flashes[0].1];
            assert_eq!((aperture.circles.len(), aperture.strips.len()), (4, 4));
            assert_eq!(aperture.outline.len(), if keep_outline { 5 } else { 0 }, "height {height} at {local}");
        }
    }
}

/// Axis-aligned narrow cores that cannot use an outline and are not covered by opposite edge
/// strips cannot keep non-coincident corner circles connected to center-line rectangles under
/// Gerber §4.14.2 coordinate and size tolerances, so export refuses them without partial files.
#[test]
fn fabrication_refuses_axis_aligned_narrow_cores_where_corner_circles_can_separate() {
    for local in [0, 90] {
        for (height, radius) in [(1_500, 300), (1_501, 300), (1_000, 200), (1_600, 300), (3_300, 1_200)] {
            let pads = [(PadShape::RoundedRectangle, 600_000, height, local, radius); 2];
            let mut s = aperture_fixture_with_radii(&pads, 0, false, Point { x_nm: 0, y_nm: 0 }, Point { x_nm: 30_000_001, y_nm: -25_000_003 }, None);
            let before = s.revision();
            let error = view(&s).fabrication_outputs().unwrap_err();
            assert!(error.starts_with("Cannot export rounded pad at ("), "{error} (height={height}, r={radius}, local={local})");
            assert!(
                error.ends_with("its core has no supported exact Gerber fill and its outline can self-intersect."),
                "{error} (height={height}, r={radius}, local={local})"
            );
            assert!(matches!(s.execute(EXPORT_FABRICATION, &json!({})), Err(CommandError::Unavailable(message)) if message == error));
            assert!(matches!(s.export_fabrication_archive(&json!({})), Err(CommandError::Unavailable(message)) if message == error));
            assert_eq!(s.revision(), before);
        }
    }
}

/// Immediately outside the narrow-core refusal band—once opposite strips cover the core
/// (diameter - 0.5 µm > min core span + 1 µm) or opposite outline edges stay > 1 µm apart—
/// the macro emits strips (and outline when needed) whose endpoints coincide with every corner
/// circle center, so every corner circle stays connected under Gerber §4.14.2 perturbations.
#[test]
fn rounded_pad_macro_emits_when_corner_circles_stay_connected_at_narrow_core_boundary() {
    for local in [0, 90] {
        for (height, radius, expect_outline) in [(3_299, 1_200, false), (3_302, 1_201, false), (1_601, 300, true)] {
            let diameter = 2 * radius;
            let pads = [(PadShape::RoundedRectangle, 600_000, height, local, radius); 2];
            let s = aperture_fixture_with_radii(&pads, 0, false, Point { x_nm: 0, y_nm: 0 }, Point { x_nm: 30_000_001, y_nm: -25_000_003 }, None);
            let outputs = view(&s).fabrication_outputs().unwrap();
            for output in surface_files(false) {
                let read = read_gerber(&outputs, output);
                assert!(read.regions.is_empty() && read.strokes.is_empty() && read.flashes.is_empty());
                assert_eq!(read.rounded_flashes.len(), 2);
                for &(_, code) in &read.rounded_flashes {
                    let aperture = &read.rounded[&code];
                    assert_eq!((aperture.circles.len(), aperture.strips.len()), (4, 4));
                    assert_eq!(aperture.outline.len(), if expect_outline { 5 } else { 0 }, "height {height} at {local}");
                    for &(circle_diameter, corner) in &aperture.circles {
                        assert_eq!(circle_diameter, diameter);
                        let anchored_strips = aperture
                            .strips
                            .iter()
                            .filter(|&&(strip_diameter, start, end)| strip_diameter == diameter && (start == corner || end == corner))
                            .count();
                        assert_eq!(anchored_strips, 2, "corner {corner:?} must coincide with two strip endpoints");
                        if expect_outline {
                            assert!(aperture.outline.contains(&corner), "corner {corner:?} must coincide with an outline vertex");
                        }
                    }
                }
            }
        }
    }
}

/// A rotated narrow core whose opposite edges can meet under Gerber perturbation must not
/// produce a partial file set or an archive if no safe exact fill is available.
#[test]
fn fabrication_refuses_a_narrow_core_without_an_exact_gerber_fill() {
    let pads = [(PadShape::RoundedRectangle, 600_000, 1_500, 37, 300); 2];
    let mut s = aperture_fixture_with_radii(&pads, 0, false, Point { x_nm: 0, y_nm: 0 }, Point { x_nm: 30_000_001, y_nm: -25_000_003 }, None);
    let before = s.revision();
    let error = view(&s).fabrication_outputs().unwrap_err();
    assert!(error.starts_with("Cannot export rounded pad at ("), "{error}");
    assert!(error.ends_with("its core has no supported exact Gerber fill and its outline can self-intersect."), "{error}");
    assert!(matches!(s.execute(EXPORT_FABRICATION, &json!({})), Err(CommandError::Unavailable(message)) if message == error));
    assert!(matches!(s.export_fabrication_archive(&json!({})), Err(CommandError::Unavailable(message)) if message == error));
    assert_eq!(s.revision(), before);
}

/// A rounded macro flash is drawn inside its pad's copper object scope: it carries the net,
/// component and outer-layer pin like any other pad flash, and the scope is closed afterwards.
#[test]
fn rounded_pad_flashes_keep_copper_object_attributes() {
    for bottom in [false, true] {
        let s = aperture_fixture(PadShape::RoundedRectangle, 800_000, 400_000, 0, 0, bottom);
        let outputs = view(&s).fabrication_outputs().unwrap();
        let layer = if bottom { ("-Bottom.gbr", "Copper,L2,Bot", "Positive") } else { TOP_COPPER };
        let read = read_gerber(&outputs, layer);
        assert_eq!(read.rounded_flashes.len(), 2);
        assert!(read.regions.is_empty());
        let pads: Vec<_> = read.objects.iter().filter(|(function, _)| *function == Some("SMDPad,CuDef")).map(|(_, object)| object).collect();
        assert_eq!(pads.len(), 2);
        let mut pins = Vec::new();
        for object in pads {
            let component = object.component.clone().unwrap();
            let (reference, pin) = object.pin.clone().unwrap();
            assert_eq!(reference, component);
            assert!(object.net.is_some());
            pins.push(pin);
        }
        pins.sort();
        assert_eq!(pins, ["1", "2"]);
    }
}

/// Two rotations can snap a tiny inner box onto three edges without clamping its radius.
/// Keep that pad's original closed region rather than writing a malformed quad primitive.
/// The second land is a plain rectangle: the same rounded land 2 mm along the rotated pitch
/// snaps to a strictly convex 2 nm-diameter sliver, which export refuses by design.
#[test]
fn tiny_rounded_pad_with_non_strict_core_retains_region() {
    let s = aperture_fixture_at(
        &[(PadShape::RoundedRectangle, 3, 3, 165), (PadShape::Rectangle, 3, 3, 165)],
        45,
        false,
        Point { x_nm: -1, y_nm: 1 },
        Point { x_nm: 30_000_001, y_nm: -25_000_003 },
        None,
        None,
    );
    let pads = view(&s).footprints().next().unwrap().pads;
    let first = &pads[0];
    let contour = Contour::pad(first.center, first.copper.unwrap(), first.shape, first.corner_radius_nm, first.size_nm);
    assert_eq!(contour.radius, 1.);
    let at = Point { x_nm: first.center.x_nm, y_nm: -first.center.y_nm };
    let read = read_gerber(&view(&s).fabrication_outputs().unwrap(), TOP_COPPER);
    assert!(!read.rounded_flashes.iter().any(|(center, _)| *center == at));
    assert_eq!(read.regions.len(), 2, "the rectangular second land keeps its region too");
    assert_eq!(read.regions[0].len(), 29, "the positive fallback radius still produces four chorded corners");
    assert_eq!(read.regions[0].first(), read.regions[0].last());
}

/// Tiny non-cardinal and twice-rounded collapsed lands stay on the existing fallback, even
/// when their snapped corners look cardinal. Net-cardinal mixed stages are conservative fallbacks.
#[test]
fn tiny_and_composed_rotations_do_not_flash_rectangular_or_obround_apertures() {
    for (shape, width, height, local, rotation, bottom, offset) in [
        (PadShape::Oval, 1, 2, 31, 40, true, Point { x_nm: 0, y_nm: 2 }),
        (PadShape::Rectangle, 1, 2, 31, 42, true, Point { x_nm: 0, y_nm: 2 }),
        (PadShape::Rectangle, 2, 2, 0, 1, false, Point { x_nm: 0, y_nm: 0 }),
        (PadShape::Rectangle, 2, 2, 1, 0, false, Point { x_nm: 0, y_nm: 0 }),
        (PadShape::Oval, 2, 4, 0, 1, false, Point { x_nm: 0, y_nm: 0 }),
        (PadShape::Oval, 2, 4, 1, 0, false, Point { x_nm: 0, y_nm: 0 }),
        (PadShape::Oval, 2, 1, 1, 0, false, Point { x_nm: 0, y_nm: 0 }),
        (PadShape::Oval, 1, 2, 0, 1, false, Point { x_nm: 0, y_nm: 0 }),
        (PadShape::Oval, 400_000, 400_000, 45, 0, false, Point { x_nm: 0, y_nm: 0 }),
        (PadShape::Rectangle, 800_000, 400_000, 45, 45, false, Point { x_nm: 0, y_nm: 0 }),
        (PadShape::Oval, 800_000, 400_000, 45, 45, true, Point { x_nm: 0, y_nm: 0 }),
    ] {
        let s = aperture_fixture_at(
            &[(shape, width, height, local); 2],
            rotation,
            bottom,
            offset,
            Point { x_nm: 30_000_001, y_nm: -25_000_003 },
            None,
            None,
        );
        let pads: Vec<_> = view(&s).footprints().flat_map(|f| f.pads).collect();
        if width == 2 && (height == 2 || height == 4) {
            assert_eq!(axis_aligned_size(&pads[0].copper.unwrap()), Some((width, height)), "snapped geometry alone would permit a flash");
        }
        if local == 31 {
            let corners = pads[0].copper.unwrap();
            assert!(corners.iter().all(|p| p.x_nm == corners[0].x_nm), "counterexample collapses onto one axis");
            assert_eq!(axis_aligned_size(&corners), None);
        }
        let outputs = view(&s).fabrication_outputs().unwrap();
        for output in surface_files(bottom) {
            let read = read_gerber(&outputs, output);
            assert!(read.apertures.values().all(|a| a.kind == 'C' && a.width > 0 && a.height > 0));
            if shape == PadShape::Rectangle {
                assert!(read.flashes.is_empty() && read.strokes.is_empty());
                assert_eq!(read.regions.len(), pads.len());
                for (region, pad) in read.regions.iter().zip(&pads) {
                    let c = pad.copper.unwrap();
                    assert_eq!(*region, c.iter().chain(std::iter::once(&c[0])).map(|p| Point { x_nm: p.x_nm, y_nm: -p.y_nm }).collect::<Vec<_>>());
                }
                if local == 31 {
                    let path = &read.regions[0];
                    assert!(path.iter().all(|p| p.x_nm == path[0].x_nm));
                    // The closing edge retraces the preceding collinear steps; structural
                    // closure does not imply a filled land.
                    let low = path.iter().map(|p| p.y_nm).min().unwrap();
                    let high = path.iter().map(|p| p.y_nm).max().unwrap();
                    assert_eq!(high - low, 3);
                    assert_eq!(path.first(), path.last());
                }
            } else {
                assert!(read.regions.is_empty());
                assert_eq!(read.flashes.len() + read.strokes.len(), pads.len());
                if local == 31 {
                    let first = Point { x_nm: pads[0].center.x_nm, y_nm: -pads[0].center.y_nm };
                    assert!(read.flashes.iter().any(|(at, a)| *at == first && a.kind == 'C' && a.width == 1));
                }
                if width == height {
                    assert!(read.strokes.is_empty());
                    for ((at, a), pad) in read.flashes.iter().zip(&pads) {
                        assert_eq!(*at, Point { x_nm: pad.center.x_nm, y_nm: -pad.center.y_nm });
                        assert_eq!(a.kind, 'C');
                        assert!((a.width - width).abs() <= 1, "rounded square fallback diameter");
                    }
                }
            }
        }
    }
}

/// Standard O flashes match the engine land capsule only when both dimension parities agree.
/// Mixed parity retains rounded-endpoint strokes; those approximations need not equal the land.
#[test]
fn obround_capsules_match_land_geometry_across_parity_sides_and_output_origins() {
    let mut observed_fallback_difference = false;
    for (width, height) in [(2, 1), (1, 2), (3, 2), (2, 3), (4, 2), (3, 1), (2, 2), (1, 1)] {
        for (center, origin) in [(0, 0), (3, -5), (-3, 5)] {
            for rotation in [0, 90] {
                for bottom in [false, true] {
                    let mut s = aperture_fixture_at(
                        &[(PadShape::Oval, width, height, 0); 2],
                        rotation,
                        bottom,
                        Point { x_nm: 0, y_nm: 0 },
                        Point { x_nm: center, y_nm: -center },
                        None,
                        None,
                    );
                    s.apply(crate::SET_PLACEMENT_ORIGIN, &json!({"board":s.project().unwrap().board_target().unwrap().board,"origin":{"kind":"point","x_nm":origin,"y_nm":-origin},"expected_revision":s.revision()})).unwrap();
                    let fabrication = view(&s).fabrication().unwrap();
                    let frame = fabrication.origin.frame();
                    let lands = crate::board::track::lands(view(&s));
                    let pads: Vec<_> = view(&s).footprints().flat_map(|f| f.pads).collect();
                    for (output, function) in surface_files(bottom).into_iter().zip(["SMDPad,CuDef", "Material", "Material"]) {
                        let read = read_gerber(&fabrication.files, output);
                        assert!(read.regions.is_empty());
                        if width % 2 == height % 2 {
                            assert!(read.strokes.is_empty());
                            assert_eq!(read.flashes.len(), lands.len());
                            for ((at, a), land) in read.flashes.iter().zip(&lands) {
                                assert_eq!(a.kind, 'O');
                                let contour = land.contour();
                                assert_eq!(2. * contour.radius, a.width.min(a.height) as f64);
                                let expected: BTreeSet<_> = contour
                                    .core
                                    .map(|p| {
                                        let p = frame.to_output(p);
                                        (2 * p.x_nm, 2 * p.y_nm)
                                    })
                                    .into_iter()
                                    .collect();
                                let span = (a.width - a.height).abs();
                                let actual: BTreeSet<_> = [-1, 1]
                                    .map(|sign| {
                                        (
                                            2 * at.x_nm + if a.width >= a.height { sign * span } else { 0 },
                                            2 * at.y_nm + if a.height > a.width { sign * span } else { 0 },
                                        )
                                    })
                                    .into_iter()
                                    .collect();
                                assert_eq!(actual, expected, "analytic O core and land core");
                            }
                        } else {
                            let function = (function == "Material").then_some(function);
                            assert!(read.flashes.is_empty());
                            assert_eq!(read.strokes.len(), lands.len());
                            for ((a, b, aperture), land) in read.strokes.iter().zip(&lands) {
                                assert_eq!(*aperture, ReadAperture { kind: 'C', width: width.min(height), height: width.min(height), function });
                                let contour = land.contour();
                                let expected: BTreeSet<_> = contour
                                    .core
                                    .map(|p| {
                                        let p = frame.to_output(p);
                                        (p.x_nm, p.y_nm)
                                    })
                                    .into_iter()
                                    .collect();
                                let actual = BTreeSet::from([(a.x_nm, a.y_nm), (b.x_nm, b.y_nm)]);
                                let center = frame.to_output(pads.iter().find(|pad| pad.number == land.key.1).unwrap().center);
                                let half = ((width - height).abs() + 1) / 2;
                                let along_x = (rotation == 0) == (width > height);
                                let legacy: BTreeSet<_> = [-1, 1]
                                    .map(|sign| {
                                        (center.x_nm + if along_x { sign * half } else { 0 }, center.y_nm + if along_x { 0 } else { sign * half })
                                    })
                                    .into_iter()
                                    .collect();
                                assert_eq!(actual, legacy, "existing rounded-endpoint fallback position and axis");
                                observed_fallback_difference |= actual != expected;
                                assert_eq!((a.x_nm - b.x_nm).abs() + (a.y_nm - b.y_nm).abs(), (width - height).abs() + 1);
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(observed_fallback_difference, "legacy endpoint rounding is not general exact land equivalence");
}

/// Ucamco Gerber 2026.05 §§5.6.10 and 6.4: a swept circular tool is not the pad. A
/// mixed-parity obround retains its endpoints, but does not share the same-size pad flash's tag.
#[test]
fn legacy_copper_obround_stroke_does_not_inherit_pad_function() {
    let s = aperture_fixture_with_pads(&[(PadShape::Circle, 400_000, 400_000, 0), (PadShape::Oval, 800_001, 400_000, 0)], 0, false);
    let outputs = view(&s).fabrication_outputs().unwrap();
    let read = read_gerber(&outputs, TOP_COPPER);
    let tagged = ReadAperture { kind: 'C', width: 400_000, height: 400_000, function: Some("SMDPad,CuDef") };
    assert_eq!(read.apertures.len(), 2);
    assert_eq!(read.flashes, [(Point { x_nm: 30_000_001, y_nm: 25_000_003 }, tagged)]);
    assert_eq!(
        read.strokes,
        [(Point { x_nm: 31_800_000, y_nm: 25_000_003 }, Point { x_nm: 32_200_002, y_nm: 25_000_003 }, ReadAperture { function: None, ..tagged },)]
    );
    assert!(read.regions.is_empty());
}

/// All-copper plated pads reach every copper and both outer mask files, but not paste.
#[test]
fn through_hole_pad_flashes_are_excluded_from_paste() {
    for (shape, kind) in [(PadShape::Circle, 'C'), (PadShape::Rectangle, 'R'), (PadShape::Oval, 'O')] {
        let width = if shape == PadShape::Circle { 400_000 } else { 800_000 };
        let s = aperture_fixture_at(
            &[(shape, width, 400_000, 0); 2],
            90,
            true,
            Point { x_nm: 0, y_nm: 0 },
            Point { x_nm: 3_000_000, y_nm: -3_000_000 },
            Some(0),
            None,
        );
        let outputs = view(&s).fabrication_outputs().unwrap();
        // The back-side quarter turn swaps the aperture axes and mirrors the local X offsets.
        for output in [
            TOP_COPPER,
            ("-Inner_1.gbr", "Copper,L2,Inr", "Positive"),
            ("-Inner_2.gbr", "Copper,L3,Inr", "Positive"),
            ("-Bottom.gbr", "Copper,L4,Bot", "Positive"),
            TOP[1],
            BOTTOM[1],
        ] {
            let function = if output.1.starts_with("Copper,") { "ComponentPad" } else { "Material" };
            let aperture = ReadAperture { kind, width: 400_000, height: width, function: Some(function) };
            let expected = [3_000_000, 1_000_000].map(|y_nm| (Point { x_nm: 3_000_000, y_nm }, aperture));
            let read = read_gerber(&outputs, output);
            assert_eq!(read.flashes, expected, "{}", output.0);
            assert!(read.regions.is_empty() && read.strokes.is_empty());
            let reference = view(&s).footprints().next().unwrap().reference.into_owned();
            for (index, (_, object)) in read.objects.iter().enumerate() {
                if output.1.starts_with("Copper,") {
                    assert_eq!(object.net.as_deref(), Some(""));
                    assert_eq!(object.component.as_deref(), Some(reference.as_str()));
                    let pin = (!output.1.ends_with(",Inr")).then(|| (reference.to_owned(), (index + 1).to_string()));
                    assert_eq!(object.pin, pin);
                } else {
                    assert_eq!(*object, ReadObject::default());
                }
            }
        }
        for output in [TOP[2], BOTTOM[2]] {
            let read = read_gerber(&outputs, output);
            assert!(read.flashes.is_empty() && read.regions.is_empty() && read.strokes.is_empty());
        }
    }
}

/// Distinct accepted pad-only identities can share the same display name without merging.
#[test]
fn gerber_distinguishes_unwired_multi_pad_terminal_groups() {
    check_unwired_multi_pad_terminal_groups(true);
}

/// Fabrication also accepts a board that has not initialized reviewed board updates.
#[test]
fn gerber_distinguishes_unwired_multi_pad_groups_without_baseline() {
    check_unwired_multi_pad_terminal_groups(false);
}

/// Check separate pad groups in read-back both before routing and after connecting
/// one group through the back layer. The baseline toggle changes where intent is sourced.
fn check_unwired_multi_pad_terminal_groups(accept_baseline: bool) {
    let mut s = aperture_fixture_at(
        &[(PadShape::Circle, 400_000, 400_000, 0); 4],
        0,
        false,
        Point { x_nm: 0, y_nm: 0 },
        Point { x_nm: 3_000_000, y_nm: 3_000_000 },
        None,
        Some(json!({"1":["1","2"],"2":["3","4"]})),
    );
    if accept_baseline {
        s.apply(crate::board::update::INITIALIZE_BASELINE, &json!({"expected_revision":s.revision()})).unwrap();
    }
    assert_eq!(view(&s).project().board.baseline.is_some(), accept_baseline);
    let pads = view(&s).footprints().next().unwrap().pads;
    assert_eq!(pads.len(), 4);
    let display_name = accept_baseline.then_some("Unconnected");
    assert!(pads.iter().all(|pad| pad.net.as_deref() == display_name));
    let pad_names = |read: &ReadGerber| {
        read.objects.iter().filter_map(|(_, object)| Some((object.pin.as_ref()?.1.clone(), object.net.clone().unwrap()))).collect::<BTreeMap<_, _>>()
    };
    let outputs = view(&s).fabrication_outputs().unwrap();
    let names = pad_names(&read_gerber(&outputs, TOP_COPPER));
    assert_eq!(names.len(), 4);
    assert_eq!(names["1"], names["2"]);
    assert_eq!(names["3"], names["4"]);
    assert_ne!(names["1"], names["3"]);
    assert!(names.values().all(|name| !name.is_empty()));

    let center = |number| pads.iter().find(|pad| pad.number == number).unwrap().center;
    let (a, b) = (center("1"), center("2"));
    let y_nm = a.y_nm + 2_000_000;
    let [front, back] = copper_layers(&s)[..] else { panic!("two copper layers") };
    route_with_vias(&mut s, front, &[a, Point { y_nm, ..a }, Point { y_nm, ..b }, b], &[(1, back), (2, front)]).unwrap();
    let outputs = view(&s).fabrication_outputs().unwrap();
    let top = read_gerber(&outputs, TOP_COPPER);
    assert_eq!(pad_names(&top), names);
    let bottom = read_gerber(&outputs, ("-Bottom.gbr", "Copper,L2,Bot", "Positive"));
    for (layer, tracks) in [(top, 2), (bottom, 1)] {
        for (function, count) in [("Conductor", tracks), ("ViaPad", 2)] {
            let routed: Vec<_> = layer.objects.iter().filter(|(kind, _)| *kind == Some(function)).collect();
            assert_eq!(routed.len(), count);
            assert!(routed.iter().all(|(_, object)| object.net.as_deref() == Some(names["1"].as_str())));
        }
    }
}

/// Rebuild the CAD netlist from .N/.P/.C attached to copper objects, following
/// Ucamco Gerber 2026.05 §§5.4–5.5, 5.6.13–15 and 6.8. Compare with accepted logical
/// membership projected via the fixture's deliberately swapped physical pad mapping.
#[test]
fn gerber_object_attributes_round_trip_two_layer_board_netlist() {
    let mut s = routed_fixture();
    outline(&mut s);
    let [front, back] = copper_layers(&s)[..] else { panic!("two copper layers") };
    let last = view(&s).footprints().last().unwrap();
    s.apply(
        crate::SET_FOOTPRINT_TRANSFORM,
        &json!({"footprint":last.id,"x_nm":last.position.x_nm,"y_nm":last.position.y_nm,
        "rotation":last.rotation,"side":"back","expected_revision":s.revision()}),
    )
    .unwrap();
    let accepted_pad = |footprint: super::super::view::FootprintView<'_>| {
        footprint.pads.into_iter().find(|pad| pad.number == "2" && pad.net.as_deref() == Some("ACCEPTED")).unwrap().center
    };
    let a = accepted_pad(view(&s).footprints().next().unwrap());
    let b = accepted_pad(view(&s).footprints().last().unwrap());
    let y = a.y_nm + 2_000_000;
    route_with_vias(&mut s, front, &[a, Point { y_nm: y, ..a }, Point { y_nm: y, ..b }, b], &[(1, back)]).unwrap();
    let x = 1_000_000;
    route(&mut s, back, &[Point { x_nm: x, y_nm: 20_000_000 }, Point { x_nm: 2 * x, y_nm: 20_000_000 }]).unwrap();

    let revision = s.revision();
    let board = view(&s).target.board;
    let outputs = s.export_fabrication(&json!({"board":board})).unwrap();
    assert_eq!(outputs, s.export_fabrication(&json!({"board":board})).unwrap());
    assert_eq!(s.revision(), revision);
    let references: BTreeMap<_, _> = view(&s).footprints().map(|f| (f.component, f.reference.into_owned())).collect();
    let inspection = s.inspect();
    let netlist = &inspection["project"]["board"]["netlist"];
    assert_eq!(netlist["basis"], "accepted");
    let mut expected = BTreeMap::<String, BTreeSet<(String, String)>>::new();
    for net in netlist["nets"].as_array().unwrap() {
        let mut members = BTreeSet::new();
        for terminal in net["terminals"].as_array().unwrap() {
            let reference = &references[&format!("component-{}", terminal["component"].as_u64().unwrap())];
            let pad = match terminal["terminal"].as_str().unwrap() {
                "1" => "2",
                "2" => "1",
                _ => panic!("unexpected terminal in fixture"),
            };
            members.insert((reference.clone(), pad.to_owned()));
        }
        assert!(expected.insert(net["name"].as_str().unwrap().to_owned(), members).is_none());
    }
    assert_eq!(expected.len(), 2);
    assert_eq!(expected.values().map(BTreeSet::len).sum::<usize>(), 5);

    let mut reconstructed = BTreeMap::<String, BTreeSet<(String, String)>>::new();
    let mut pins_without_net = 0;
    for (layer, expected_tracks) in [(TOP_COPPER, [1, 0]), (BOTTOM[0], [2, 1])] {
        let read = read_gerber(&outputs, layer);
        let mut tracks = [0, 0];
        let mut vias = 0;
        for (function, object) in read.objects {
            let net = object.net.as_deref().unwrap();
            match function {
                Some("SMDPad,CuDef") => {
                    let (reference, pad) = object.pin.unwrap();
                    assert_eq!(object.component.as_deref(), Some(reference.as_str()));
                    assert!(!pad.is_empty());
                    if net.is_empty() {
                        pins_without_net += 1;
                    } else {
                        assert!(reconstructed.entry(net.to_owned()).or_default().insert((reference, pad)));
                    }
                }
                Some("Conductor") => {
                    assert!(object.component.is_none() && object.pin.is_none());
                    let index = match net {
                        "ACCEPTED" => 0,
                        "" => 1,
                        _ => panic!("unexpected route intent: {net}"),
                    };
                    tracks[index] += 1;
                }
                Some("ViaPad") => {
                    assert_eq!(net, "ACCEPTED");
                    assert!(object.component.is_none() && object.pin.is_none());
                    vias += 1;
                }
                _ => panic!("unexpected copper function: {function:?}"),
            }
        }
        assert_eq!(tracks, expected_tracks);
        assert_eq!(vias, 1);
    }
    assert_eq!(pins_without_net, 1);
    assert_eq!(reconstructed, expected);
    for layer in [TOP[1], TOP[2], BOTTOM[1], BOTTOM[2], ("-Silkscreen_Top.gbr", "Legend,Top", "Positive"), ("-Outline.gbr", "Profile,NP", "Positive")]
    {
        assert!(read_gerber(&outputs, layer).objects.iter().all(|(_, attrs)| *attrs == ReadObject::default()));
    }

    // Removing one named delete leaves the previous pad's component live. Putting a
    // TO in a pad contour is also invalid, even if its value would be unchanged (§5.4).
    let (suffix, function, polarity) = TOP_COPPER;
    let text = file(&outputs, suffix);
    let leaked = text.replacen("%TD.C*%\n", "", 1);
    assert_eq!(try_read_gerber(&leaked, function, polarity).unwrap_err(), "invalid or duplicate object attribute");
    // Routed built-in 0402 rounded pads now flash as macros, so use a non-cardinal rectangle
    // fixture that still emits G36 pad regions to exercise the in-region attribute guard.
    let region_fixture = aperture_fixture(PadShape::Rectangle, 800_000, 400_000, 37, 0, false);
    let region_outputs = view(&region_fixture).fabrication_outputs().unwrap();
    let region_text = file(&region_outputs, suffix);
    let in_region = region_text.replacen("G36*\n", "G36*\n%TO.N,ILLEGAL*%\n", 1);
    assert_ne!(in_region, region_text);
    assert_eq!(try_read_gerber(&in_region, function, polarity).unwrap_err(), "object attribute outside copper object");
}

/// A board update can split the intended nets while leaving a plated bridge. Pads retain
/// individual intent; every conductor on the shorted island has no .N. Wholly unassigned
/// copper instead carries an empty .N, as in Ucamco Gerber 2026.05 §5.6.13.
#[test]
fn gerber_conflicting_and_unnamed_islands_are_not_conflated() {
    use crate::board::drc::short_tests::{Bridge, bridged_split};
    let mut s = bridged_split(Bridge::Vias);
    outline(&mut s);
    let front = copper_layers(&s)[0];
    route(&mut s, front, &[Point { x_nm: 0, y_nm: 20_000_000 }, Point { x_nm: 2_000_000, y_nm: 20_000_000 }]).unwrap();
    let outputs = view(&s).fabrication_outputs().unwrap();
    let mut pad_names = BTreeSet::new();
    for (layer, unnamed) in [(TOP_COPPER, 1), (BOTTOM[0], 0)] {
        let mut unresolved = 0;
        let mut empty = 0;
        for (function, object) in read_gerber(&outputs, layer).objects {
            if matches!(function, Some("Conductor" | "ViaPad")) {
                assert!(object.pin.is_none() && object.component.is_none());
                match object.net.as_deref() {
                    None => unresolved += 1,
                    Some("") => empty += 1,
                    Some(name) => panic!("misleading net on conductor: {name}"),
                }
            } else if let Some(name) = object.net.filter(|name| !name.is_empty()) {
                pad_names.insert(name);
            }
        }
        assert!(unresolved > 0, "the short crosses both layers");
        assert_eq!(empty, unnamed);
    }
    assert_eq!(pad_names.len(), 2, "pad intent is still visible");
}

/// An isolated explicitly assigned via exports its net on both layers until the
/// identity is retired; afterward it must not masquerade as automatic/unnamed.
#[test]
fn gerber_via_assignment_exports_current_intent_but_not_retired_name() {
    let mut s = routed_fixture();
    outline(&mut s);
    s.apply(crate::net_labels::ADD, &json!({"name":"VIA_NET","style":"global","x_nm":-50_000_000,"y_nm":0,"expected_revision":s.revision()}))
        .unwrap();
    let project = s.project().unwrap();
    let label = project.net_labels().iter().find(|label| label.name == "VIA_NET").unwrap().id;
    let target = project.inspect_net_registry().unwrap()["nets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|net| net["name"] == "VIA_NET")
        .map(|net| json!({"kind":"derived","anchor":net["anchor"]}))
        .unwrap();
    s.apply(crate::ADD_VIA, &json!({"at":{"x_nm":-30_000_000,"y_nm":20_000_000},"target":target,"expected_revision":s.revision()})).unwrap();
    let read_via = |s: &Session, layer| {
        let outputs = view(s).fabrication_outputs().unwrap();
        let mut vias = read_gerber(&outputs, layer).objects.into_iter().filter(|(function, _)| *function == Some("ViaPad"));
        let object = vias.next().unwrap().1;
        assert!(vias.next().is_none());
        object
    };
    for layer in [TOP_COPPER, BOTTOM[0]] {
        assert_eq!(read_via(&s, layer), ReadObject { net: Some("VIA_NET".to_owned()), ..ReadObject::default() });
    }
    s.apply(crate::net_labels::DELETE, &json!({"id":format!("net-label-{label}"),"expected_revision":s.revision()})).unwrap();
    for layer in [TOP_COPPER, BOTTOM[0]] {
        assert_eq!(read_via(&s, layer), ReadObject::default());
    }
}

/// X2's reserved and delimiter characters must not change the field count or cause
/// two distinct CAD net names to appear as the same name (§§3.4.3–4 and 5.6.13).
#[test]
fn gerber_object_field_escaping_is_reversible_and_keeps_reserved_names_distinct() {
    let reference = "R,1%";
    let pin = "a*\\\n";
    for (name, exported) in [("", ""), ("N/C", "N/C:N/C"), ("N/C:N/C", "N/C:N/C:N/C"), ("a,b%*\\é", "a,b%*\\é")] {
        let mut gerber = Gerber::new(Frame::BOARD_ZERO);
        gerber.object(Object { net: Some(name), component: Some((reference, Some(pin))) }, |gerber| {
            gerber.stroke(&[Point { x_nm: 0, y_nm: 0 }], 100_000, Some(ApertureFunction::ThroughHolePad));
        });
        let text = gerber.finish("Copper,L1,Top", "Positive");
        let object = try_read_gerber(&text, "Copper,L1,Top", "Positive").unwrap().objects.remove(0).1;
        assert_eq!(object.net.as_deref(), Some(exported));
        assert_eq!(object.pin, Some((reference.to_owned(), pin.to_owned())));
        assert_eq!(object.component.as_deref(), Some(reference));
    }
}

/// Ucamco Gerber 2026.05 §5.6.10: same-sized copper with different functions must not share
/// an aperture. Equal circles with the same function still share; read-back checks geometry too.
#[test]
fn equal_size_copper_uses_distinct_aperture_functions() {
    let size = 600_000;
    let mut s = aperture_fixture_at(
        &[(PadShape::Circle, size, size, 0); 4],
        0,
        false,
        Point { x_nm: 0, y_nm: 0 },
        Point { x_nm: 3_000_000, y_nm: -3_000_000 },
        Some(2),
        None,
    );
    let front = copper_layers(&s)[0];
    let points = [Point { x_nm: 1_000_000, y_nm: 20_000_000 }, Point { x_nm: 3_000_000, y_nm: 20_000_000 }];
    s.apply(crate::ADD_TRACKS, &json!({"layer":front,"width_nm":size,"points":points,"expected_revision":s.revision()})).unwrap();
    let via = Point { x_nm: 6_000_000, y_nm: 20_000_000 };
    s.apply(crate::ADD_VIA, &json!({"at":via,"via_diameter_nm":size,"via_drill_nm":300_000,"expected_revision":s.revision()})).unwrap();

    let outputs = s.export_fabrication(&json!({})).unwrap();
    let top = read_gerber(&outputs, TOP_COPPER);
    let circle = |function| ReadAperture { kind: 'C', width: size, height: size, function: Some(function) };
    let expected_apertures = ["SMDPad,CuDef", "ComponentPad", "Conductor", "ViaPad"].map(circle);
    assert_eq!(top.apertures.len(), expected_apertures.len(), "one D-code per function, not per pad");
    for aperture in expected_apertures {
        assert_eq!(top.apertures.values().filter(|&&actual| actual == aperture).count(), 1);
    }
    assert_eq!(
        top.flashes,
        [
            (Point { x_nm: 3_000_000, y_nm: 3_000_000 }, circle("SMDPad,CuDef")),
            (Point { x_nm: 5_000_000, y_nm: 3_000_000 }, circle("SMDPad,CuDef")),
            (Point { x_nm: 7_000_000, y_nm: 3_000_000 }, circle("ComponentPad")),
            (Point { x_nm: 9_000_000, y_nm: 3_000_000 }, circle("ComponentPad")),
            (Point { y_nm: -via.y_nm, ..via }, circle("ViaPad")),
        ]
    );
    let [start, end] = points.map(|p| Point { y_nm: -p.y_nm, ..p });
    assert_eq!(top.strokes, [(start, end, circle("Conductor"))]);
    assert!(top.regions.is_empty());
    let repeat = s.export_fabrication(&json!({})).unwrap();
    assert_eq!(outputs, repeat);
    assert_eq!(zip(&outputs).unwrap(), zip(&repeat).unwrap());
}

/// Shared export dispatch is read-only, deterministic, origin-aware and survives native reopen.
/// Exports between undo and redo cannot consume either history entry or alter saved bytes.
#[test]
fn aperture_exports_preserve_revision_storage_and_history() {
    use crate::persistence::{decode_documents, encode_documents};
    let mut s = aperture_fixture(PadShape::Oval, 800_001, 400_003, 90, 0, true);
    let original = encode_documents(s.snapshot().unwrap()).unwrap();
    s.apply(crate::SET_PLACEMENT_ORIGIN, &json!({"board":s.project().unwrap().board_target().unwrap().board,"origin":{"kind":"point","x_nm":1_000_003,"y_nm":-2_000_007},"expected_revision":s.revision()}))
        .unwrap();
    let saved = encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    let result = s.execute(EXPORT_FABRICATION, &json!({})).unwrap();
    let direct = view(&s).fabrication().unwrap();
    let records: Vec<_> = direct
        .files
        .iter()
        .map(|output| {
            let bytes = output.contents.as_bytes();
            json!({"name": output.name, "size_bytes": bytes.len(), "sha256": format!("{:x}", Sha256::digest(bytes))})
        })
        .collect();
    assert_eq!(
        result["fabrication"],
        json!({"files": direct.files, "origin": direct.origin, "manifest": {"generator_version": env!("CARGO_PKG_VERSION"), "files": records}})
    );
    assert_eq!(s.revision(), revision);
    assert_eq!(s.apply(EXPORT_FABRICATION, &json!({})).unwrap(), revision);
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), saved);
    let read = read_gerber(&direct.files, BOTTOM_PASTE);
    let pad = view(&s).footprints().next().unwrap().pads.remove(0);
    assert_eq!(read.flashes[0].0, Point { x_nm: pad.center.x_nm - 1_000_003, y_nm: -(pad.center.y_nm + 2_000_007) });
    let reopened = decode_documents(&saved).unwrap();
    assert_eq!(encode_documents(reopened.clone()).unwrap(), saved);
    assert_eq!(reopened.board_view(reopened.board_target().unwrap()).unwrap().fabrication().unwrap(), direct);
    assert_eq!(zip(&direct.files).unwrap(), zip(&view(&s).fabrication_outputs().unwrap()).unwrap());
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), original);
    s.execute(EXPORT_FABRICATION, &json!({})).unwrap();
    s.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), saved);
}

fn file<'a>(outputs: &'a [Output], suffix: &str) -> &'a str {
    &outputs
        .iter()
        .find(|output| output.name.ends_with(suffix))
        .unwrap_or_else(|| panic!("no {suffix} in {:?}", outputs.iter().map(|o| &o.name).collect::<Vec<_>>()))
        .contents
}

/// Outputs need a board setup and a closed outline, and say which is missing.
#[test]
fn outputs_require_setup_and_a_closed_outline() {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    let project = s.project().unwrap();
    let error = project.board_view(project.board_target().unwrap()).unwrap().fabrication_outputs().unwrap_err();
    assert!(error.contains("board setup"), "{error}");
    let s = routed_fixture();
    let error = view(&s).fabrication_outputs().unwrap_err();
    assert!(error.contains("closed Board Outline"), "{error}");
}

/// Ucamco Gerber 2026.05 §§5.6.10 and 6.5: a disconnected line is not the physical profile.
/// Preserve the existing line and loop coordinates but separate the same-width apertures.
#[test]
fn only_the_closed_outline_drawing_gets_the_profile_function() {
    let mut s = routed_fixture();
    outline(&mut s);
    let layer = view(&s).layers().into_iter().find(|layer| layer.purpose == LayerPurpose::Outline).unwrap().id;
    let stray = [Point { x_nm: 30_000_000, y_nm: 0 }, Point { x_nm: 35_000_000, y_nm: 0 }];
    s.apply(crate::ADD_DRAWING, &json!({"layer":layer,"points":stray,"closed":false,"expected_revision":s.revision()})).unwrap();
    let outputs = view(&s).fabrication_outputs().unwrap();
    let read = read_gerber(&outputs, ("-Outline.gbr", "Profile,NP", "Positive"));
    let profile = ReadAperture { kind: 'C', width: 100_000, height: 100_000, function: Some("Profile") };
    let loop_points =
        [(-5_000_000, 5_000_000), (25_000_000, 5_000_000), (25_000_000, -35_000_000), (-5_000_000, -35_000_000), (-5_000_000, 5_000_000)]
            .map(|(x_nm, y_nm)| Point { x_nm, y_nm });
    let expected: Vec<_> = loop_points.windows(2).map(|pair| (pair[0], pair[1], profile)).collect();
    let (tagged, untagged): (Vec<_>, Vec<_>) = read.strokes.into_iter().partition(|(_, _, aperture)| aperture.function.is_some());
    assert_eq!(read.apertures.len(), 2);
    assert!(read.flashes.is_empty() && read.regions.is_empty());
    assert_eq!(tagged, expected);
    assert_eq!(untagged, [(stray[0], stray[1], ReadAperture { function: None, ..profile })]);
}

/// Add an open or closed path through the shared drawing command, not by mutating a test board.
fn add_fabrication_drawing(s: &mut Session, purpose: LayerPurpose, points: &[Point], closed: bool) {
    let layer = view(s).layers().into_iter().find(|layer| layer.purpose == purpose).unwrap().id;
    s.apply(crate::ADD_DRAWING, &json!({"layer":layer,"points":points,"closed":closed,"stroke_nm":100_000,"expected_revision":s.revision()}))
        .unwrap();
}

/// An open path alone is not a board edge: export stays unavailable and leaves no undo step.
/// Once a loop is closed, the open path is plotted as drawn, without the Profile function.
#[test]
fn fabrication_requires_a_closed_loop_and_plots_open_outline_paths_as_drawn() {
    use crate::persistence::encode_documents;

    let mut s = routed_fixture();
    let open = [
        Point { x_nm: -1_250_001, y_nm: 2_000_003 },
        Point { x_nm: -2_000_000, y_nm: 3_000_000 },
        Point { x_nm: -1_000_000, y_nm: 3_000_000 },
        Point { x_nm: -1_250_000, y_nm: 2_000_003 },
    ];
    let drawings = view(&s).drawings().len();
    add_fabrication_drawing(&mut s, LayerPurpose::Outline, &open, false);
    let revision = s.revision();
    let saved = encode_documents(s.snapshot().unwrap()).unwrap();
    let error = s.execute(EXPORT_FABRICATION, &json!({})).unwrap_err();
    assert!(
        matches!(&error, CommandError::Unavailable(message) if message == "Draw a closed Board Outline before generating fabrication outputs."),
        "{error}"
    );
    assert_eq!(s.revision(), revision);
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), saved);
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(view(&s).drawings().len(), drawings);
    s.apply("edit.redo", &json!({})).unwrap();

    outline(&mut s);
    let outputs = view(&s).fabrication_outputs().unwrap();
    let read = read_gerber(&outputs, ("-Outline.gbr", "Profile,NP", "Positive"));
    let profile = ReadAperture { kind: 'C', width: 100_000, height: 100_000, function: Some("Profile") };
    let untagged = ReadAperture { function: None, ..profile };
    let (tagged, plain): (Vec<_>, Vec<_>) = read.strokes.into_iter().partition(|(_, _, aperture)| aperture.function.is_some());
    assert_eq!(tagged.len(), 4);
    assert!(tagged.iter().all(|&(_, _, aperture)| aperture == profile));
    let drawn: Vec<_> = open.map(|point| Point { x_nm: point.x_nm, y_nm: -point.y_nm }).windows(2).map(|pair| (pair[0], pair[1], untagged)).collect();
    assert_eq!(plain, drawn);
}

/// Read the profile's exact move and draw sequence at its existing nanometre resolution.
/// See Ucamco Gerber Layer Format Specification 2026.05 sections 4.7–4.8 for plotting modes and operations.
#[test]
fn fabrication_emits_one_closed_profile_path_per_outline_loop() {
    let mut s = routed_fixture();
    outline(&mut s);
    let outer = ["X-5000000Y5000000D02*", "X25000000Y5000000D01*", "X25000000Y-35000000D01*", "X-5000000Y-35000000D01*", "X-5000000Y5000000D01*"];
    let inner = ["X1000001Y-2000003D02*", "X2000005Y-2000003D01*", "X2000005Y-3000007D01*", "X1000001Y-3000007D01*", "X1000001Y-2000003D01*"];
    let cutout = [
        Point { x_nm: 1_000_001, y_nm: 2_000_003 },
        Point { x_nm: 2_000_005, y_nm: 2_000_003 },
        Point { x_nm: 2_000_005, y_nm: 3_000_007 },
        Point { x_nm: 1_000_001, y_nm: 3_000_007 },
    ];
    add_fabrication_drawing(&mut s, LayerPurpose::Silkscreen, &cutout[..2], false);
    for has_cutout in [false, true] {
        if has_cutout {
            add_fabrication_drawing(&mut s, LayerPurpose::Outline, &cutout, true);
        }
        let outputs = view(&s).fabrication_outputs().unwrap();
        let profile = file(&outputs, "-Outline.gbr");
        let read = try_read_gerber(profile, "Profile,NP", "Positive").unwrap();
        assert_eq!(read.strokes.len(), if has_cutout { 8 } else { 4 });
        assert!(read.strokes.iter().all(|(_, _, aperture)| aperture.function == Some("Profile")), "every closed loop is the board profile");
        assert!(read.flashes.is_empty() && read.regions.is_empty());
        let records: Vec<_> = profile.lines().filter(|line| line.starts_with('X')).collect();
        let (paths, remainder) = records.as_chunks::<5>();
        assert!(remainder.is_empty());
        assert_eq!(paths.len(), if has_cutout { 2 } else { 1 });
        assert!(paths.contains(&outer));
        assert_eq!(paths.contains(&inner), has_cutout);
    }
}

/// A local reference move affects only that footprint's silk and round-trips through native storage.
#[test]
fn moved_reference_changes_only_its_gerber_silkscreen() {
    use crate::persistence::{decode_documents, encode_documents};
    let mut s = routed_fixture();
    outline(&mut s);
    let before = view(&s).fabrication_outputs().unwrap();
    let original = encode_documents(s.snapshot().unwrap()).unwrap();
    let footprint = view(&s).footprints().next().unwrap();
    let id = footprint.id.clone();
    let pads = footprint.pads;
    let offset = Point { x_nm: 2_000_000, y_nm: -1_000_000 };
    let revision = s.revision();
    let params = json!({"footprint":id,"offset":offset,"expected_revision":revision});
    s.apply(crate::SET_FOOTPRINT_REFERENCE_OFFSET, &params).unwrap();
    assert!(s.apply(crate::SET_FOOTPRINT_REFERENCE_OFFSET, &params).is_err());
    assert_eq!(view(&s).footprints().next().unwrap().pads, pads);
    let after = view(&s).fabrication_outputs().unwrap();
    assert_eq!(
        before.iter().zip(&after).filter(|(a, b)| a != b).map(|(_, b)| b.name.as_str()).collect::<Vec<_>>(),
        [after.iter().find(|output| output.name.ends_with("-Silkscreen_Top.gbr")).unwrap().name.as_str()]
    );
    let documents = encode_documents(s.snapshot().unwrap()).unwrap();
    assert!(String::from_utf8_lossy(&documents[&crate::persistence::test_board_path(&documents)]).contains("pcbcraft-board/15"));
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), original);
    s.apply("edit.redo", &json!({})).unwrap();
    let revision = s.revision();
    s.apply(
        crate::SET_FOOTPRINT_REFERENCE_OFFSET,
        &json!({"footprint":id,"offset":crate::board::DEFAULT_REFERENCE_OFFSET,"expected_revision":revision}),
    )
    .unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), original);
}

/// Format 15 retains routes, vias and bottom-side geometry, with reference offsets transformed locally.
#[test]
fn moved_reference_round_trips_with_bottom_tracks_and_vias() {
    use crate::persistence::{decode_documents, encode_documents};
    let mut session = routed_fixture();
    let [front, back] = copper_layers(&session)[..] else { panic!("two copper layers") };
    route_with_vias(
        &mut session,
        front,
        &[Point { x_nm: 0, y_nm: 20_000_000 }, Point { x_nm: 3_000_000, y_nm: 20_000_000 }, Point { x_nm: 6_000_000, y_nm: 20_000_000 }],
        &[(1, back)],
    )
    .unwrap();
    let footprint = view(&session).footprints().next().unwrap();
    let id = footprint.id.clone();
    let origin = footprint.position;
    session
        .apply(
            crate::SET_FOOTPRINT_TRANSFORM,
            &json!({"footprint":id,"x_nm":origin.x_nm,"y_nm":origin.y_nm,"rotation":90,"side":"back","expected_revision":session.revision()}),
        )
        .unwrap();
    let original = encode_documents(session.snapshot().unwrap()).unwrap();
    let before = view(&session).footprints().find(|f| f.id == id).unwrap();
    let points: Vec<_> = before.graphics.iter().filter(|g| g.reference).flat_map(|g| g.points.iter().copied()).collect();
    let pads = before.pads;
    session
        .apply(
            crate::SET_FOOTPRINT_REFERENCE_OFFSET,
            &json!({"footprint":id,"offset":{"x_nm":2_000_000,"y_nm":-1_000_000},"expected_revision":session.revision()}),
        )
        .unwrap();
    let moved = view(&session).footprints().find(|f| f.id == id).unwrap();
    assert_eq!(moved.pads, pads);
    let actual: Vec<_> = moved.graphics.iter().filter(|g| g.reference).flat_map(|g| g.points.iter().copied()).collect();
    let expected: Vec<_> = points.iter().map(|p| Point { x_nm: p.x_nm - 1_000_000, y_nm: p.y_nm + 2_000_000 }).collect();
    assert!(!expected.is_empty());
    assert_eq!(actual, expected, "mirror local X before the quarter turn");
    let saved = encode_documents(session.snapshot().unwrap()).unwrap();
    let board: serde_json::Value = serde_json::from_slice(&saved[&crate::persistence::test_board_path(&saved)]).unwrap();
    assert_eq!(board["format"], "pcbcraft-board/15");
    assert!(!board["tracks"].as_array().unwrap().is_empty());
    assert!(!board["vias"].as_array().unwrap().is_empty());
    assert!(board["footprints"].as_array().unwrap().iter().any(|f| f["bottom"] == true));
    assert_eq!(encode_documents(decode_documents(&saved).unwrap()).unwrap(), saved);
    session
        .apply(
            crate::SET_FOOTPRINT_REFERENCE_OFFSET,
            &json!({"footprint":id,"offset":crate::board::DEFAULT_REFERENCE_OFFSET,"expected_revision":session.revision()}),
        )
        .unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), original);
}

/// A routed two-layer board produces each layer file with the right content: pad flashes and
/// tracks on their copper, vias on both layers, mask and paste openings for front SMD pads,
/// the silkscreen, the outline, and one drill tool per hole size. Y is negated throughout.
#[test]
fn a_routed_board_produces_gerber_and_drill_files() {
    let mut s = routed_fixture();
    outline(&mut s);
    let [front, back] = copper_layers(&s)[..] else { panic!("two copper layers") };
    let y = 20_000_000;
    route(&mut s, front, &[Point { x_nm: 1_000_000, y_nm: y }, Point { x_nm: 3_000_000, y_nm: y }]).unwrap();
    route_with_vias(
        &mut s,
        front,
        &[Point { x_nm: 5_000_000, y_nm: y }, Point { x_nm: 7_000_000, y_nm: y }, Point { x_nm: 9_000_000, y_nm: y }],
        &[(1, back)],
    )
    .unwrap();
    let outputs = view(&s).fabrication_outputs().unwrap();
    let names: Vec<_> = outputs.iter().map(|output| output.name.as_str()).collect();
    for suffix in [
        "-Top.gbr",
        "-Bottom.gbr",
        "-Mask_Top.gbr",
        "-Mask_Bottom.gbr",
        "-Paste_Top.gbr",
        "-Paste_Bottom.gbr",
        "-Silkscreen_Top.gbr",
        "-Outline.gbr",
        "-PTH.drl",
    ] {
        assert!(names.iter().any(|name| name.ends_with(suffix)), "{suffix} in {names:?}");
    }
    for output in outputs.iter().filter(|output| output.name.ends_with(".gbr")) {
        let text = &output.contents;
        assert!(text.contains("%FSLAX46Y46*%") && text.contains("%MOMM*%") && text.ends_with("M02*\n"), "{}", output.name);
        assert!(text.lines().all(|line| line.ends_with('*') || line.ends_with("*%") || line == "%"), "{}: Gerber command delimiters", output.name);
    }

    let pads: Vec<_> = view(&s).footprints().flat_map(|f| f.pads).collect();
    let top = file(&outputs, "-Top.gbr");
    assert!(top.contains("%TF.FileFunction,Copper,L1,Top*%"));
    assert_eq!(read_gerber(&outputs, TOP_COPPER).rounded_flashes.len(), pads.len(), "one flash per front pad");
    assert!(top.contains("C,0.250000*%"), "the 0.25 mm track aperture");
    assert!(top.contains(&format!("X1000000Y{}D02*\nX3000000Y{}D01*", -y, -y)), "the front track, Y negated");
    assert!(top.contains(&format!("X7000000Y{}D03*", -y)), "the via is flashed on the top layer");
    let bottom = file(&outputs, "-Bottom.gbr");
    assert!(bottom.contains("%TF.FileFunction,Copper,L2,Bot*%"));
    let bottom_copper = read_gerber(&outputs, BOTTOM[0]);
    assert!(bottom_copper.rounded_flashes.is_empty() && bottom_copper.regions.is_empty(), "SMD pads stay on the front");
    assert!(bottom.contains(&format!("X7000000Y{}D03*", -y)) && bottom.contains(&format!("X9000000Y{}D01*", -y)), "via and back track");
    assert_eq!(read_gerber(&outputs, TOP[1]).rounded_flashes.len(), pads.len());
    assert!(file(&outputs, "-Mask_Top.gbr").contains("%TF.FilePolarity,Negative*%"));
    assert_eq!(read_gerber(&outputs, TOP[2]).rounded_flashes.len(), pads.len());
    let bottom_paste = read_gerber(&outputs, BOTTOM_PASTE);
    assert!(bottom_paste.rounded_flashes.is_empty() && bottom_paste.regions.is_empty());
    let profile = read_gerber(&outputs, ("-Outline.gbr", "Profile,NP", "Positive"));
    assert_eq!(profile.strokes.len(), 4, "a closed rectangle is four strokes");
    assert!(profile.strokes.iter().all(|(_, _, aperture)| aperture.function == Some("Profile")));
    let silk = read_gerber(&outputs, ("-Silkscreen_Top.gbr", "Legend,Top", "Positive"));
    assert!(!silk.apertures.is_empty());
    assert!(silk.apertures.values().all(|aperture| aperture.function == Some("Material")));

    let drill = file(&outputs, "-PTH.drl");
    assert!(drill.starts_with("M48\n") && drill.ends_with("M30\n"));
    assert!(drill.contains("T1C0.300\n"), "{drill}");
    assert!(drill.contains("X7.000Y-20.000\n"), "{drill}");
}

/// Exercise one bottom-side, quarter-turned footprint with plated lands and a board via.
fn drill_fixture(holes: &[crate::board::definition::CircularNpth]) -> Session {
    let mut s = footprint_fixture(
        &[(PadShape::Circle, 2_000_000, 2_000_000, 0, 0); 2],
        (90, true),
        (Point { x_nm: 0, y_nm: 0 }, Point { x_nm: 2_000_000, y_nm: 0 }),
        Point { x_nm: 30_000_000, y_nm: -25_000_000 },
        Some(0),
        None,
        holes,
    );
    s.apply(
        crate::ADD_VIA,
        &json!({"at":{"x_nm":12_000_000,"y_nm":20_000_000},"via_diameter_nm":600_000,"via_drill_nm":300_000,"expected_revision":s.revision()}),
    )
    .unwrap();
    let board = s.project().unwrap().board_target().unwrap().board;
    s.apply(
        crate::SET_PLACEMENT_ORIGIN,
        &json!({"board":board,"origin":{"kind":"point","x_nm":1_000_000,"y_nm":-2_000_000},"expected_revision":s.revision()}),
    )
    .unwrap();
    s
}

/// Separate round hits, preserve the plated output bytes, and include the new file in the manifest.
#[test]
fn mechanical_holes_use_their_own_drill_file() {
    use crate::board::definition::CircularNpth;
    let holes = [
        CircularNpth { center: Point { x_nm: 3_000_000, y_nm: 5_000_000 }, diameter_nm: 3_200_000 },
        CircularNpth { center: Point { x_nm: -4_000_000, y_nm: 6_000_000 }, diameter_nm: 3_200_000 },
    ];
    let mut with = drill_fixture(&holes);
    let without = drill_fixture(&[]);
    let params = json!({});
    let legacy = without.export_fabrication(&params).unwrap();
    let mixed = with.export_fabrication(&params).unwrap();
    assert_eq!(&mixed[..mixed.len() - 1], legacy.as_slice(), "no NPTH hits in plated, copper, mask or paste files");
    // The legacy serializer emits via tools first here, then the two pads in board coordinate order.
    assert_eq!(
        file(&legacy, "-PTH.drl"),
        "M48\n; PCBcraft plated drill file\n; TYPE=PLATED\nFMAT,2\nMETRIC\nT1C0.300\nT2C1.000\n%\nG90\nG05\nT1\nX11.000Y-22.000\nT2\nX29.000Y23.000\nX29.000Y21.000\nM30\n"
    );
    assert!(!legacy.iter().any(|out| out.name.ends_with("-NPTH.drl")));
    // Mirror local X, rotate 90°, add board placement, subtract origin, then negate Y.
    let npth = mixed.last().unwrap();
    assert!(npth.name.ends_with("-NPTH.drl"));
    assert_eq!(
        npth.contents,
        "M48\n; PCBcraft non-plated drill file\n; TYPE=NON_PLATED\nFMAT,2\nMETRIC\nT1C3.200\n%\nG90\nG05\nT1\nX34.000Y20.000\nX35.000Y27.000\nM30\n"
    );
    let capture = with.export_fabrication_archive(&params).unwrap();
    assert_eq!(capture, with.export_fabrication_archive(&params).unwrap());
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(capture.bytes())).unwrap();
    let mut drill = String::new();
    zip.by_name(&npth.name).unwrap().read_to_string(&mut drill).unwrap();
    assert_eq!(drill, npth.contents);
    let manifest = embedded_manifest(&capture);
    assert_eq!(with.execute(EXPORT_FABRICATION, &params).unwrap()["fabrication"]["manifest"], manifest);
    assert_eq!(
        manifest["files"].as_array().unwrap().last().unwrap(),
        &json!({"name":npth.name,"size_bytes":npth.contents.len(),"sha256":format!("{:x}", Sha256::digest(npth.contents.as_bytes()))})
    );
}

/// A half-unit diameter rounds up to a nonzero tool; a smaller diameter cannot produce any archive.
#[test]
fn non_plated_drill_rejects_a_diameter_that_rounds_to_zero() {
    use crate::board::definition::CircularNpth;
    let at = Point { x_nm: 3_000_000, y_nm: 5_000_000 };
    let minimum = drill_fixture(&[CircularNpth { center: at, diameter_nm: 500 }]);
    let params = json!({});
    let outputs = minimum.export_fabrication(&params).unwrap();
    assert!(file(&outputs, "-NPTH.drl").contains("T1C0.001\n"));
    let too_small = drill_fixture(&[CircularNpth { center: at, diameter_nm: 499 }]);
    assert!(matches!(too_small.export_fabrication_archive(&params), Err(CommandError::Unavailable(message))
        if message.contains("499 nm") && message.contains("rounds to zero")));
}

/// Read the JSON actually stored in a captured ZIP.
fn embedded_manifest(capture: &Archive) -> Value {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(capture.bytes())).unwrap();
    serde_json::from_reader(archive.by_name("manifest.json").unwrap()).unwrap()
}

/// Even an unfilled or suppressed zone carries copper intent that fabrication would omit.
#[test]
fn fabrication_refuses_boards_with_zones_in_every_fill_state() {
    use crate::board::{
        fill::{INSPECT_FILL, REFILL_ZONES, UNFILL_ZONES},
        zones::{ADD_ZONE, Connection, DELETE_ZONES, Options},
    };
    use std::sync::Arc;

    let mut session = routed_fixture();
    outline(&mut session);
    let (target, points) = {
        let view = view(&session);
        let airwires = view.ratsnest().unwrap();
        let end = &airwires.lines[0].from;
        let target = view.net_class_target_for_copper(&format!("{}:{}", end.footprint, end.pad.number.as_str())).unwrap().unwrap();
        let points = view.drawings().first().unwrap().points.clone();
        (target, points)
    };
    let layer = copper_layers(&session)[0];
    let options = Options { connection: Connection::Solid, ..Options::default() };
    session
        .apply(ADD_ZONE, &json!({"target":target,"points":points,"layer":layer,"options":options,"expected_revision":session.revision()}))
        .unwrap();
    let params = json!({});
    let revision = session.revision();
    let snapshot = session.snapshot().unwrap();
    for (command, state) in [(INSPECT_FILL, "pending"), (REFILL_ZONES, "ready"), (UNFILL_ZONES, "unfilled")] {
        let result = session.execute(command, &params).unwrap();
        assert_eq!(result["zone_fill"]["result"]["state"], state);
        let check = |error: CommandError| assert!(matches!(error, CommandError::Unavailable(message) if message.contains("copper zones")));
        check(session.export_fabrication_archive(&params).unwrap_err());
        check(session.execute(EXPORT_FABRICATION, &params).unwrap_err());
        assert_eq!(session.revision(), revision);
        assert!(Arc::ptr_eq(&snapshot, &session.snapshot().unwrap()));
    }
    session.apply(DELETE_ZONES, &json!({"zones":[view(&session).zones()[0].id],"expected_revision":revision})).unwrap();
    assert!(session.export_fabrication_archive(&params).is_ok());
}

/// The same board gives byte-identical outputs and archives, including rounded pad macros.
/// The sole extra member describes the exact generated bytes in order, not itself or the
/// outer ZIP. All entries have fixed metadata.
#[test]
fn embedded_manifest_matches_every_generated_member() {
    let mut session = routed_fixture();
    outline(&mut session);
    let outputs = session.export_fabrication(&json!({})).unwrap();
    assert!(!read_gerber(&outputs, TOP_COPPER).rounded_flashes.is_empty(), "the archive fixture exercises macros");
    assert_eq!(outputs, session.export_fabrication(&json!({})).unwrap());
    let capture = session.export_fabrication_archive(&json!({})).unwrap();
    assert_eq!(capture.bytes(), zip(&outputs).unwrap());
    assert_eq!(capture, session.export_fabrication_archive(&json!({})).unwrap());
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(capture.bytes())).unwrap();
    let names: Vec<_> = archive.file_names().map(|name| name.unwrap().into_owned()).collect();
    assert_eq!(names.iter().filter(|name| name.as_str() == "manifest.json").count(), 1);
    assert!(!outputs.is_empty());
    assert_eq!(archive.len(), outputs.len() + 1);
    let mut records = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).unwrap();
        assert_eq!(entry.system(), zip::System::Unix);
        assert_eq!(entry.unix_mode(), Some(0o100644));
        assert_eq!(entry.last_modified(), Some(zip::DateTime::default()));
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        if let Some(output) = outputs.get(index) {
            assert_eq!(entry.name().unwrap(), output.name);
            assert_eq!(bytes, output.contents.as_bytes());
            records.push(json!({"name": output.name, "size_bytes": bytes.len(), "sha256": format!("{:x}", Sha256::digest(&bytes))}));
        } else {
            assert_eq!(entry.name().unwrap(), "manifest.json");
            assert_eq!(bytes.last(), Some(&b'\n'));
            assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), json!({"generator_version": env!("CARGO_PKG_VERSION"), "files": records}));
        }
    }
}

/// Byte sizes and digests include multibyte UTF-8 and preserve LF versus CRLF without normalization.
#[test]
fn manifest_counts_bytes_not_characters_or_lines() {
    let outputs = [Output { name: "text.txt".into(), contents: "é\r\nλ\n".into() }];
    let capture = Archive::new(&outputs).unwrap();
    let manifest = embedded_manifest(&capture);
    let record = &manifest["files"][0];
    assert_eq!(record["size_bytes"], 7);
    assert_eq!(record["sha256"], format!("{:x}", Sha256::digest(b"\xc3\xa9\r\n\xce\xbb\n")));
    assert_ne!(record["sha256"], format!("{:x}", Sha256::digest("é\nλ\n".as_bytes())));
}

/// Generic callers cannot shadow the embedded manifest with another file of that name.
#[test]
fn archive_reserves_the_manifest_member_name() {
    for name in ["manifest.json", "MANIFEST.JSON"] {
        let output = Output { name: name.into(), contents: "not the manifest".into() };
        assert!(Archive::new(&[output]).unwrap_err().contains("manifest.json is reserved"));
    }
}

/// The command result and embedded manifest agree. Neither export edits documents, revision or
/// undo/redo; a later edit affects the new capture but not the earlier ZIP or its manifest.
#[test]
fn manifest_is_read_only_and_stays_bound_to_its_capture() {
    use crate::persistence::encode_documents;
    let mut session = routed_fixture();
    outline(&mut session);
    let before = encode_documents(session.snapshot().unwrap()).unwrap();
    let revision = session.revision();
    let history = (session.undo.len(), session.redo.len());
    let capture = session.export_fabrication_archive(&json!({})).unwrap();
    let manifest = embedded_manifest(&capture);
    let result = session.execute(EXPORT_FABRICATION, &json!({})).unwrap();
    assert_eq!(result["fabrication"]["manifest"], manifest);
    assert_eq!(result["revision"], revision);
    assert_eq!(session.revision(), revision);
    assert_eq!((session.undo.len(), session.redo.len()), history);
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
    let [front, _] = copper_layers(&session)[..] else { panic!("two copper layers") };
    route(&mut session, front, &[Point { x_nm: 1_000_000, y_nm: 20_000_000 }, Point { x_nm: 3_000_000, y_nm: 20_000_000 }]).unwrap();
    let changed = session.export_fabrication_archive(&json!({})).unwrap();
    assert_ne!(capture.bytes(), changed.bytes());
    assert_ne!(manifest["files"], embedded_manifest(&changed)["files"]);
    assert_eq!(embedded_manifest(&capture), manifest);
}

/// The command edits nothing and returns every file through execute.
#[test]
fn export_command_returns_files_without_editing() {
    let mut s = routed_fixture();
    outline(&mut s);
    let revision = s.revision();
    let result = s.execute(EXPORT_FABRICATION, &json!({})).unwrap();
    assert_eq!(result["revision"], revision);
    assert_eq!(s.revision(), revision);
    let files = result["fabrication"]["files"].as_array().unwrap();
    assert!(files.iter().any(|file| file["name"].as_str().unwrap().ends_with("-PTH.drl")));
}

/// Sanitization and case folding cannot merge copper files with each other or the fixed outputs.
/// Names and ZIP bytes remain stable, and fixed file functions keep their conventional names.
#[test]
fn colliding_layer_names_produce_unique_deterministic_archives() {
    for second in ["F_Cu", "f_Cu"] {
        let mut s = Session::default();
        s.apply("project.new", &json!({"create_board":true})).unwrap();
        s.apply(crate::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":6,"expected_revision":s.revision()})).unwrap();
        let mut setup = s.inspect()["project"]["board"]["setup"].clone();
        let names = ["F.Cu", second, "Mask_Top", "Paste_Top", "Silkscreen_Top", "Outline"];
        let order: Vec<_> = view(&s)
            .setup()
            .unwrap()
            .stackup()
            .strata
            .iter()
            .filter_map(|stratum| if let Stratum::Copper { layer, .. } = stratum { Some(*layer) } else { None })
            .collect();
        assert_eq!(order.len(), names.len());
        for (id, name) in order.into_iter().zip(names) {
            let layer = setup["layers"].as_array_mut().unwrap().iter_mut().find(|layer| layer["id"] == json!(id)).unwrap();
            layer["name"] = json!(name);
        }
        s.apply(crate::SET_BOARD_SETUP, &json!({"setup":setup,"expected_revision":s.revision()})).unwrap();
        outline(&mut s);
        let outputs = view(&s).fabrication_outputs().unwrap();
        let unique: BTreeSet<_> = outputs.iter().map(|output| output.name.to_ascii_lowercase()).collect();
        assert_eq!(unique.len(), outputs.len());
        assert!(outputs[0].name.ends_with("-F_Cu.gbr"));
        assert!(outputs[1].name.ends_with("-2.gbr"));
        assert!(outputs[2].name.ends_with("-Mask_Top-2.gbr"));
        assert!(file(&outputs, "-Mask_Top.gbr").contains("%TF.FileFunction,Soldermask,Top*%"));
        assert!(file(&outputs, "-Silkscreen_Top.gbr").contains("%TF.FileFunction,Legend,Top*%"));
        assert!(file(&outputs, "-Outline.gbr").contains("%TF.FileFunction,Profile,NP*%"));
        assert_eq!(outputs, view(&s).fabrication_outputs().unwrap());
        assert_eq!(zip(&outputs).unwrap(), zip(&view(&s).fabrication_outputs().unwrap()).unwrap());
    }
}

/// Footprint graphics are already transformed into board space; export only flips their Y axis.
#[test]
fn translated_rotated_silkscreen_preserves_board_coordinates() {
    let mut s = routed_fixture();
    outline(&mut s);
    let id = view(&s).footprints().next().unwrap().id;
    s.apply(
        crate::SET_FOOTPRINT_TRANSFORM,
        &json!({"footprint":id,"x_nm":30_000_000,"y_nm":-25_000_000,"rotation":90,"expected_revision":s.revision()}),
    )
    .unwrap();
    let footprint = view(&s).footprints().find(|footprint| footprint.id == id).unwrap();
    assert_eq!(footprint.position, Point { x_nm: 30_000_000, y_nm: -25_000_000 });
    assert_eq!(footprint.rotation, 90);
    let graphic = footprint.graphics.iter().find(|graphic| graphic.purpose == GraphicPurpose::Silkscreen && graphic.points.len() >= 2).unwrap();
    let (from, to) = (graphic.points[0], graphic.points[1]);
    let expected = format!("X{}Y{}D02*\nX{}Y{}D01*", from.x_nm, -from.y_nm, to.x_nm, -to.y_nm);
    let outputs = view(&s).fabrication_outputs().unwrap();
    assert!(file(&outputs, "-Silkscreen_Top.gbr").contains(&expected), "{expected}");
}

/// Both command entry points reject the same invalid targets, parameters and unavailable boards,
/// and refusals retain revision, snapshot and undo/redo availability.
#[test]
fn fabrication_apply_and_execute_share_validation() {
    let mut s = Session::default();
    for phase in 0..3 {
        if phase == 1 {
            s.apply("project.new", &json!({"create_board":true})).unwrap();
        } else if phase == 2 {
            s.apply(crate::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":s.revision()})).unwrap();
        }
        let before = s.inspect();
        for params in [json!({}), json!({"board":true}), json!({"board":uuid::Uuid::from_u128(1)})] {
            let applied = s.apply(EXPORT_FABRICATION, &params).unwrap_err().to_string();
            assert_eq!(s.execute(EXPORT_FABRICATION, &params).unwrap_err().to_string(), applied);
            assert_eq!(s.inspect(), before);
        }
    }
    outline(&mut s);
    let before = s.inspect();
    assert_eq!(s.apply(EXPORT_FABRICATION, &json!({})).unwrap(), s.revision());
    let exported = s.execute(EXPORT_FABRICATION, &json!({})).unwrap();
    assert!(!exported["fabrication"]["files"].as_array().unwrap().is_empty());
    assert_eq!(s.inspect(), before);
}
