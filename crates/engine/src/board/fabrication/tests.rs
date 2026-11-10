use super::*;
use crate::board::{
    drc::tests::outline,
    track::tests::{copper_layers, route, route_with_vias, routed_fixture, view},
};

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

/// A routed two-layer board produces each layer file with the right content: pad regions and
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
        assert!(text.lines().all(|line| line.ends_with('*') || line.ends_with("*%")), "{}: every Gerber line ends its command", output.name);
    }

    let pads: Vec<_> = view(&s).footprints().flat_map(|f| f.pads).collect();
    let top = file(&outputs, "-Top.gbr");
    assert!(top.contains("%TF.FileFunction,Copper,L1,Top*%"));
    assert_eq!(top.matches("G36*").count(), pads.len(), "one region per front pad");
    assert!(top.contains("C,0.250000*%"), "the 0.25 mm track aperture");
    assert!(top.contains(&format!("X1000000Y{}D02*\nX3000000Y{}D01*", -y, -y)), "the front track, Y negated");
    assert!(top.contains(&format!("X7000000Y{}D03*", -y)), "the via is flashed on the top layer");
    let bottom = file(&outputs, "-Bottom.gbr");
    assert!(bottom.contains("%TF.FileFunction,Copper,L2,Bot*%"));
    assert_eq!(bottom.matches("G36*").count(), 0, "SMD pads stay on the front");
    assert!(bottom.contains(&format!("X7000000Y{}D03*", -y)) && bottom.contains(&format!("X9000000Y{}D01*", -y)), "via and back track");
    assert_eq!(file(&outputs, "-Mask_Top.gbr").matches("G36*").count(), pads.len());
    assert!(file(&outputs, "-Mask_Top.gbr").contains("%TF.FilePolarity,Negative*%"));
    assert_eq!(file(&outputs, "-Paste_Top.gbr").matches("G36*").count(), pads.len());
    assert_eq!(file(&outputs, "-Paste_Bottom.gbr").matches("G36*").count(), 0);
    let profile = file(&outputs, "-Outline.gbr");
    assert!(profile.contains("%TF.FileFunction,Profile,NP*%"));
    assert_eq!(profile.matches("D01*").count(), 4, "a closed rectangle is four strokes");

    let drill = file(&outputs, "-PTH.drl");
    assert!(drill.starts_with("M48\n") && drill.ends_with("M30\n"));
    assert!(drill.contains("T1C0.300\n"), "{drill}");
    assert!(drill.contains("X7.000Y-20.000\n"), "{drill}");
}

/// The same board gives byte-identical files and archive, and the archive lists every file.
#[test]
fn outputs_and_their_archive_are_deterministic() {
    let mut s = routed_fixture();
    outline(&mut s);
    let first = view(&s).fabrication_outputs().unwrap();
    assert_eq!(first, view(&s).fabrication_outputs().unwrap());
    let bytes = zip(&first).unwrap();
    assert_eq!(bytes, zip(&first).unwrap());
    let archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut names: Vec<String> = archive.file_names().map(|name| name.unwrap().into_owned()).collect();
    names.sort();
    let mut expected: Vec<_> = first.iter().map(|output| output.name.clone()).collect();
    expected.sort();
    assert_eq!(names, expected);
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
