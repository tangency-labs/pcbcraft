//! STEP round-trip qualification: every fixture in `tests/fixtures/step` is read, its placed
//! bodies are checked against the box `expected.json` records for it, written again and read
//! back, and the box is checked once more. `scripts/generate-step-fixtures.py` writes the
//! fixtures and works the expected boxes out from the dimensions it writes, without a kernel.
#![cfg(not(target_arch = "wasm32"))]
// The helpers that load expected.json are not #[test] functions; a malformed file fails the test.
#![allow(clippy::unwrap_used)]

use std::path::Path;

use pcbcraft_geometry::{BoundingBox, read_step, write_step};
use serde_json::Value;

const FIXTURE_COUNT: usize = 10;

struct Fixture {
    file: String,
    bodies: usize,
    placements: usize,
    expected: BoundingBox,
}

fn corner(value: &Value) -> [i64; 3] {
    let list: Vec<i64> = value.as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect();
    list.try_into().unwrap()
}

fn fixtures(dir: &Path) -> (u64, Vec<Fixture>) {
    let document: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("expected.json")).unwrap()).unwrap();
    let tolerance = document["tolerance_nm"].as_u64().unwrap();
    let list = document["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| Fixture {
            file: f["file"].as_str().unwrap().to_string(),
            bodies: usize::try_from(f["bodies"].as_u64().unwrap()).unwrap(),
            placements: usize::try_from(f["placements"].as_u64().unwrap()).unwrap(),
            expected: BoundingBox { min_nm: corner(&f["min_nm"]), max_nm: corner(&f["max_nm"]) },
        })
        .collect();
    (tolerance, list)
}

/// Each listed fixture reads to its expected body and placement counts and box, and after a write
/// and a second read its box is still within 1 µm; the folder holds exactly the listed files.
#[test]
fn every_fixture_round_trips_within_one_micrometre() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/step");
    let (tolerance, fixtures) = fixtures(&dir);
    assert_eq!(tolerance, 1_000, "the qualification tolerance is one micrometre");
    assert_eq!(fixtures.len(), FIXTURE_COUNT);
    let mut on_disk: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name != "expected.json")
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> = fixtures.iter().map(|f| f.file.clone()).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "every file in the folder but expected.json has an expected box, and only those");

    for fixture in &fixtures {
        let name = &fixture.file;
        let read = read_step(&std::fs::read(dir.join(name)).unwrap()).unwrap();
        assert!(read.refused.is_empty(), "{name}: {:?}", read.refused);
        assert_eq!(read.bodies.len(), fixture.bodies, "{name}: bodies");
        assert_eq!(read.placements.len(), fixture.placements, "{name}: placements");
        let first = read.bounding_box().unwrap().unwrap();
        assert!(first.max_deviation_nm(&fixture.expected) <= tolerance, "{name}: read {first:?}, expected {:?}", fixture.expected);

        // Written as one part of placed bodies, in millimetres, and read back.
        let placed = read.placed_bodies().unwrap();
        let bytes = write_step(name, &placed).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.contains("SI_UNIT(.MILLI.,.METRE.)"), "{name}: written in millimetres");
        let again = read_step(&bytes).unwrap();
        assert!(again.refused.is_empty(), "{name}: {:?}", again.refused);
        assert_eq!(again.bodies.len(), fixture.placements, "{name}: one body for each placement");
        let second = again.bounding_box().unwrap().unwrap();
        assert!(second.max_deviation_nm(&fixture.expected) <= tolerance, "{name}: re-read {second:?}, expected {:?}", fixture.expected);
    }
}

/// The inch and metre blocks read to the nanometre boxes their dimensions give, within 1 µm.
#[test]
fn inch_and_metre_files_convert_to_nanometres() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/step");
    // 0.5 by 0.25 inch to 2.5 by 1.25 by 1/16 inch; 25.4 mm to the inch exactly.
    let cases = [
        ("box_inch.step", BoundingBox { min_nm: [12_700_000, 6_350_000, 0], max_nm: [63_500_000, 31_750_000, 1_587_500] }),
        ("box_metre.step", BoundingBox { min_nm: [-12_500_000, -5_000_000, 0], max_nm: [12_500_000, 5_000_000, 2_000_000] }),
    ];
    for (file, expected) in cases {
        let read = read_step(&std::fs::read(dir.join(file)).unwrap()).unwrap();
        let got = read.bounding_box().unwrap().unwrap();
        assert!(got.max_deviation_nm(&expected) <= 1_000, "{file}: {got:?}");
    }
}

/// A part two occurrences use is read once and placed twice, the second use turned and moved.
#[test]
fn a_part_used_twice_is_one_body_placed_twice() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/step");
    let read = read_step(&std::fs::read(dir.join("assembly_shared_part_turned.step")).unwrap()).unwrap();
    assert_eq!(read.bodies.len(), 1);
    let [first, turned] = read.placements.as_slice() else { panic!("{:?}", read.placements) };
    assert_eq!((first.body, turned.body), (0, 0));
    assert_eq!(first.transform.translation_nm, [0, 0, 0]);
    assert_eq!(turned.transform.translation_nm, [30_000_000, 10_000_000, 0]);
    // The second use turns the part a quarter about z: its x runs along the assembly's y.
    let x = turned.transform.axes[0];
    assert!(x[0].abs() < 1e-12 && (x[1] - 1.0).abs() < 1e-12 && x[2].abs() < 1e-12, "{x:?}");
}

/// An assembly cycle refuses the whole file.
#[test]
fn an_assembly_that_contains_itself_is_model_unreadable() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/step");
    let text = std::fs::read_to_string(dir.join("assembly_shared_part_turned.step")).unwrap();
    // The first use's parent and child swapped: the part holds the assembly that holds it.
    let cycle = text.replacen("'first','',#174,#162,$", "'first','',#162,#174,$", 1);
    assert_ne!(cycle, text);
    let error = read_step(cycle.as_bytes()).unwrap_err();
    assert_eq!(error.code(), pcbcraft_geometry::ErrorCode::ModelUnreadable, "{error}");
}

/// `assembly.step` with the part placed `x_mm` along x instead of 45 mm.
fn assembly_with_part_at(x_mm: &str) -> Vec<u8> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/step");
    let text = std::fs::read_to_string(dir.join("assembly.step")).unwrap();
    let moved = text.replacen("#385=CARTESIAN_POINT('',(45.,30.,1.6));", &format!("#385=CARTESIAN_POINT('',({x_mm},30.,1.6));"), 1);
    assert_ne!(moved, text);
    moved.into_bytes()
}

/// The placement limit is exact: 1 km out reads, the next representable step past it is refused.
#[test]
fn a_placement_one_kilometre_out_reads_and_one_step_further_is_model_unreadable() {
    // 1,000,000 mm is MAX_LENGTH_NM exactly; the placement is checked, not the box, which the
    // part's own size takes past the limit.
    let read = read_step(&assembly_with_part_at("1000000.")).unwrap();
    let xs: Vec<i64> = read.placements.iter().map(|p| p.transform.translation_nm[0]).collect();
    assert!(xs.contains(&pcbcraft_units::MAX_LENGTH_NM), "{xs:?}");

    let error = read_step(&assembly_with_part_at("1000000.000001")).unwrap_err();
    assert_eq!(error.code(), pcbcraft_geometry::ErrorCode::ModelUnreadable, "{error}");
}

/// `assembly.step` with both parts made surface models that share the board's closed shell, each
/// beside an open shell of one of its own faces.
fn assembly_sharing_a_closed_shell() -> Vec<u8> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/step");
    let text = std::fs::read_to_string(dir.join("assembly.step")).unwrap();
    let shared = text
        .replacen("#198=MANIFOLD_SOLID_BREP('board',#197);", "#198=SHELL_BASED_SURFACE_MODEL('board',(#197,#901));", 1)
        .replacen("#360=MANIFOLD_SOLID_BREP('part',#359);", "#360=SHELL_BASED_SURFACE_MODEL('part',(#197,#902));", 1)
        .replacen("ENDSEC;\nEND-ISO-10303-21;", "#901=OPEN_SHELL('',(#196));\n#902=OPEN_SHELL('',(#358));\nENDSEC;\nEND-ISO-10303-21;", 1);
    assert_eq!(shared.matches("SHELL_BASED_SURFACE_MODEL").count(), 2);
    assert_eq!(shared.matches("=OPEN_SHELL(").count(), 2);
    shared.into_bytes()
}

/// A closed shell two placed surface models share is one body placed by each model, beside each
/// model's own sheet: three bodies in four placements, none of them left at the origin.
#[test]
fn a_closed_shell_two_models_share_is_one_body_placed_by_each() {
    let read = read_step(&assembly_sharing_a_closed_shell()).unwrap();
    assert!(read.refused.is_empty(), "{:?}", read.refused);
    assert_eq!(read.bodies.len(), 3, "{:?}", read.bodies.iter().map(|b| b.name()).collect::<Vec<_>>());
    let part_at = [45_000_000, 30_000_000, 1_600_000];
    let mut uses: Vec<(usize, [i64; 3])> = read.placements.iter().map(|p| (p.body, p.transform.translation_nm)).collect();
    uses.sort();
    assert_eq!(uses.len(), 4, "{uses:?}");
    let shell_uses: Vec<[i64; 3]> = uses.iter().filter(|(b, _)| uses.iter().filter(|(o, _)| o == b).count() == 2).map(|(_, t)| *t).collect();
    assert_eq!(shell_uses, vec![[0, 0, 0], part_at], "the shared shell, placed by the board and by the part: {uses:?}");
    let sheets: Vec<[i64; 3]> = uses.iter().filter(|(b, _)| uses.iter().filter(|(o, _)| o == b).count() == 1).map(|(_, t)| *t).collect();
    assert_eq!(sheets.len(), 2, "{uses:?}");
    assert!(sheets.contains(&[0, 0, 0]) && sheets.contains(&part_at), "one sheet with each model: {uses:?}");
}

/// `box.step` with its block reached only through mapped items: the part's shape lists
/// `factors[0]` mapped items of a representation that lists `factors[1]` of the next, and so on;
/// the last maps the block. With `extra`, the part's shape also lists the block itself. The block
/// is placed the product of the factors times, plus one with `extra`. `pad` lists the block's
/// placement that many more times in the block's representation and in the part's shape.
fn mapped_tree(factors: &[usize], extra: bool, pad: (usize, usize)) -> Vec<u8> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/step");
    let text = std::fs::read_to_string(dir.join("box.step")).unwrap();
    // #167 is the block's own placement (identity), #168 its representation, #159 the solid.
    let mut next = 1000;
    let mut id = || {
        next += 1;
        next
    };
    let mut lines = Vec::new();
    let mut below = 168;
    for (level, &count) in factors.iter().enumerate().rev() {
        let map = id();
        lines.push(format!("#{map}=REPRESENTATION_MAP(#167,#{below});"));
        let mut items: Vec<String> = (0..count)
            .map(|_| {
                let item = id();
                lines.push(format!("#{item}=MAPPED_ITEM('',#{map},#167);"));
                format!("#{item}")
            })
            .collect();
        if level == 0 {
            if extra {
                items.push("#159".to_string());
            }
            items.extend(std::iter::repeat_n("#167".to_string(), pad.1));
        }
        items.push("#167".to_string());
        below = id();
        lines.push(format!("#{below}=SHAPE_REPRESENTATION('',({}),#9);", items.join(",")));
    }
    let bottom = std::iter::repeat_n(",#167", pad.0).collect::<String>();
    let tree = text
        .replacen(
            "#168=ADVANCED_BREP_SHAPE_REPRESENTATION('box',(#159,#167),#9);",
            &format!("#168=ADVANCED_BREP_SHAPE_REPRESENTATION('box',(#159,#167{bottom}),#9);"),
            1,
        )
        .replacen("#169=SHAPE_DEFINITION_REPRESENTATION(#163,#168);", &format!("#169=SHAPE_DEFINITION_REPRESENTATION(#163,#{below});"), 1)
        .replacen("ENDSEC;\nEND-ISO-10303-21;", &format!("{}\nENDSEC;\nEND-ISO-10303-21;", lines.join("\n")), 1);
    assert!(tree.contains(&format!("(#163,#{below})")));
    assert!(tree.contains(&format!("(#159,#167{bottom})")));
    tree.into_bytes()
}

/// The mapped-item expansion is bounded before it runs: a few hundred bytes that would place the
/// block 2^20 times are refused, 100,000 placements read, and one more is refused.
#[test]
fn a_mapped_item_expansion_past_100000_placements_is_model_unreadable() {
    let huge = mapped_tree(&[2; 20], false, (0, 0));
    assert!(huge.len() < 20_000);
    let error = read_step(&huge).unwrap_err();
    assert_eq!(error.code(), pcbcraft_geometry::ErrorCode::ModelUnreadable, "{error}");

    let limit = [5, 5, 5, 5, 5, 2, 2, 2, 2, 2];
    let read = read_step(&mapped_tree(&limit, false, (0, 0))).unwrap();
    assert_eq!((read.bodies.len(), read.placements.len()), (1, 100_000));

    let error = read_step(&mapped_tree(&limit, true, (0, 0))).unwrap_err();
    assert_eq!(error.code(), pcbcraft_geometry::ErrorCode::ModelUnreadable, "{error}");
}

/// The expansion's work bound is exact: 1,000,000 steps read, one more is refused, with only
/// 65,536 placements. Each part counts one step and one for each representation its shape
/// reaches; each representation walked counts one and one for each item. Here the block's
/// representation has 10 items (11 steps), each of the 16 levels of two mapped items 3 items
/// (4 steps and its two uses), and the part's shape `pad` more: 1 + 1 + (7 + 8)·2^16 − 4 + pad.
#[test]
fn an_expansion_of_more_than_1000000_steps_is_model_unreadable() {
    let read = read_step(&mapped_tree(&[2; 16], false, (8, 16_962))).unwrap();
    assert_eq!(read.placements.len(), 65_536);
    let error = read_step(&mapped_tree(&[2; 16], false, (8, 16_963))).unwrap_err();
    assert_eq!(error.code(), pcbcraft_geometry::ErrorCode::ModelUnreadable, "{error}");
    assert!(error.to_string().contains("1000000 steps"), "{error}");
}
