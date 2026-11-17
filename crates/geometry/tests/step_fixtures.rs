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

/// Appends synthetic data records before the single data-section footer, regardless of checkout
/// line endings. A missing or repeated footer is a broken fixture, not a successful mutation.
fn append_entities(text: &str, lines: &[String]) -> String {
    let text = text.replace("\r\n", "\n");
    let footer = "ENDSEC;\nEND-ISO-10303-21;";
    assert_eq!(text.matches(footer).count(), 1, "one data-section footer");
    text.replacen(footer, &format!("{}\n{footer}", lines.join("\n")), 1)
}

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
    assert!(error.to_string().contains("assembly cycle"), "{error}");
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
    assert!(error.to_string().contains("outside the supported range"), "{error}");
}

/// `assembly.step` with both parts made surface models that share the board's closed shell, each
/// beside an open shell of one of its own faces.
fn assembly_sharing_a_closed_shell() -> Vec<u8> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/step");
    let text = std::fs::read_to_string(dir.join("assembly.step")).unwrap();
    let models = text.replacen("#198=MANIFOLD_SOLID_BREP('board',#197);", "#198=SHELL_BASED_SURFACE_MODEL('board',(#197,#901));", 1).replacen(
        "#360=MANIFOLD_SOLID_BREP('part',#359);",
        "#360=SHELL_BASED_SURFACE_MODEL('part',(#197,#902));",
        1,
    );
    let shared = append_entities(&models, &["#901=OPEN_SHELL('',(#196));".into(), "#902=OPEN_SHELL('',(#358));".into()]);
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
        .replacen("#169=SHAPE_DEFINITION_REPRESENTATION(#163,#168);", &format!("#169=SHAPE_DEFINITION_REPRESENTATION(#163,#{below});"), 1);
    let tree = append_entities(&tree, &lines);
    assert!(tree.contains(&format!("(#163,#{below})")));
    assert!(tree.contains(&format!("(#159,#167{bottom})")));
    tree.into_bytes()
}

/// The mapped-item expansion is bounded before it runs: a small mapped-item tree that would
/// place the block 2^20 times is refused, 100,000 placements read, and one more is refused. Both
/// refusals are the pre-count's (its message), not the walk's own instance limit.
#[test]
fn a_mapped_item_expansion_past_100000_placements_is_model_unreadable() {
    let huge = mapped_tree(&[2; 20], false, (0, 0));
    assert!(huge.len() < 20_000);
    let error = read_step(&huge).unwrap_err();
    assert_eq!(error.code(), pcbcraft_geometry::ErrorCode::ModelUnreadable, "{error}");
    assert!(error.to_string().contains("the assembly places more than 100000 bodies"), "{error}");

    let limit = [5, 5, 5, 5, 5, 2, 2, 2, 2, 2];
    let read = read_step(&mapped_tree(&limit, false, (0, 0))).unwrap();
    assert_eq!((read.bodies.len(), read.placements.len()), (1, 100_000));

    let error = read_step(&mapped_tree(&limit, true, (0, 0))).unwrap_err();
    assert_eq!(error.code(), pcbcraft_geometry::ErrorCode::ModelUnreadable, "{error}");
    assert!(error.to_string().contains("the assembly places more than 100000 bodies"), "{error}");
}

/// `tree` with `count` relationships between two representations nothing places, before the end
/// of its data section.
fn with_unrelated_links(tree: Vec<u8>, count: usize) -> Vec<u8> {
    let mut lines = vec!["#500001=SHAPE_REPRESENTATION('',(#167),#9);".to_string(), "#500002=SHAPE_REPRESENTATION('',(#167),#9);".to_string()];
    lines.extend((0..count).map(|i| format!("#{}=SHAPE_REPRESENTATION_RELATIONSHIP('','',#500001,#500002);", 500_003 + i)));
    let text = String::from_utf8(tree).unwrap();
    let out = append_entities(&text, &lines);
    assert_ne!(out, text);
    out.into_bytes()
}

/// The steps the reader counts for `mapped_tree(&[2; levels], false, pad)` with `links` unrelated
/// relationships, as the reader's `Expansion` defines a step. Before the walk, the one part's shape
/// reaches one representation, and every relationship is checked against it: 1 + links. The part:
/// 1, and 1 for its representation. The block's representation: 1 + 2 + pad.0 items, walked
/// 2^levels times. Each level: 1 + 3 items, plus links for each of its two mapped items, walked
/// 2^levels − 1 times in all. The part's shape: pad.1 more items.
fn steps(levels: u32, pad: (u64, u64), links: u64) -> u64 {
    let uses = 1u64 << levels;
    (1 + links) + 2 + uses * (3 + pad.0) + (4 + 2 * links) * (uses - 1) + pad.1
}

/// The input-scaled work budget for these synthetic files, which have one entity per line.
/// Repeating an item reference in a representation changes work, but not the entity count.
fn work_budget(text: &[u8]) -> u64 {
    let entities = std::str::from_utf8(text).unwrap().lines().filter(|line| line.starts_with('#')).count() as u64;
    1_000_000 + 1_000 * entities
}

/// Exactly the input-scaled work budget reads, one more step is refused, with only 65,536
/// placements. Top-level item padding preserves the same entity count on both sides.
#[test]
fn an_expansion_past_its_work_budget_is_model_unreadable() {
    let budget = work_budget(&mapped_tree(&[2; 16], false, (8, 0)));
    assert_eq!(budget, 1_233_000);
    let pad = usize::try_from(budget - steps(16, (8, 0), 0)).unwrap();
    assert_eq!(steps(16, (8, pad as u64), 0), budget);
    let exact = mapped_tree(&[2; 16], false, (8, pad));
    let past = mapped_tree(&[2; 16], false, (8, pad + 1));
    assert_eq!((work_budget(&exact), work_budget(&past)), (budget, budget));
    assert_eq!(read_step(&exact).unwrap().placements.len(), 65_536);
    let error = read_step(&past).unwrap_err();
    assert_eq!(error.code(), pcbcraft_geometry::ErrorCode::ModelUnreadable, "{error}");
    assert!(error.to_string().contains(&format!("the assembly takes more than {budget} steps to expand")), "{error}");
}

/// Relationships the walk scans for each mapped item count in the bound: 2^16 placements under
/// both limits read, and with 10,000 unrelated relationships (about 10^9 checks) the same tree is
/// refused before it is expanded. With 1,000 relationships and 1,024 placements, exactly the
/// input-scaled work budget reads and one more step is refused.
#[test]
fn relationships_scanned_for_each_mapped_item_count_in_the_bound() {
    let tree = mapped_tree(&[2; 16], false, (0, 0));
    assert!(steps(16, (0, 0), 0) < 1_000_000);
    assert_eq!(read_step(&tree).unwrap().placements.len(), 65_536);
    let huge = with_unrelated_links(tree, 10_000);
    let error = read_step(&huge).unwrap_err();
    assert_eq!(error.code(), pcbcraft_geometry::ErrorCode::ModelUnreadable, "{error}");
    assert!(error.to_string().contains(&format!("the assembly takes more than {} steps to expand", work_budget(&huge))), "{error}");

    let budget = work_budget(&with_unrelated_links(mapped_tree(&[2; 10], false, (0, 0)), 1_000));
    assert_eq!(budget, 2_211_000);
    let pad = usize::try_from(budget - steps(10, (0, 0), 1_000)).unwrap();
    let exact = with_unrelated_links(mapped_tree(&[2; 10], false, (0, pad)), 1_000);
    let past = with_unrelated_links(mapped_tree(&[2; 10], false, (0, pad + 1)), 1_000);
    assert_eq!(steps(10, (0, pad as u64), 1_000), budget);
    assert_eq!((work_budget(&exact), work_budget(&past)), (budget, budget));
    assert_eq!(read_step(&exact).unwrap().placements.len(), 1_024);
    let error = read_step(&past).unwrap_err();
    assert_eq!(error.code(), pcbcraft_geometry::ErrorCode::ModelUnreadable, "{error}");
    assert!(error.to_string().contains(&format!("the assembly takes more than {budget} steps to expand")), "{error}");
}

/// A shallow root that repeats one mapped item. Its empty target has duplicate plain links to
/// the block; those links must not multiply placements, but every use still scans them all.
/// Extra placement references pad work without changing the parsed entity count.
fn related_mapped_tree(uses: usize, links: usize, padding: usize) -> Vec<u8> {
    let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/step/box.step")).unwrap();
    let items =
        std::iter::repeat_n("#500003", uses).chain(std::iter::repeat_n("#167", padding)).chain(std::iter::once("#167")).collect::<Vec<_>>().join(",");
    let mut lines = vec![
        "#500001=SHAPE_REPRESENTATION('',(#167),#9);".to_string(),
        "#500002=REPRESENTATION_MAP(#167,#500001);".to_string(),
        "#500003=MAPPED_ITEM('',#500002,#167);".to_string(),
        format!("#500004=SHAPE_REPRESENTATION('',({items}),#9);"),
    ];
    lines.extend((0..links).map(|i| format!("#{}=SHAPE_REPRESENTATION_RELATIONSHIP('','',#500001,#168);", 500_005 + i)));
    let text = text.replacen("#169=SHAPE_DEFINITION_REPRESENTATION(#163,#168);", "#169=SHAPE_DEFINITION_REPRESENTATION(#163,#500004);", 1);
    assert!(text.contains("(#163,#500004)"));
    append_entities(&text, &lines).into_bytes()
}

/// Duplicate RELATED links and repeated mapped-item references preserve the exact work limit,
/// its first refused step and placement multiplicity. A fifty-thousand-by-fifty-thousand case
/// is refused by that work limit, not expanded or mistaken for too many body placements.
#[test]
fn related_companions_keep_the_work_bound_for_repeated_mapped_items() {
    let links = 1_000;
    let budget = work_budget(&related_mapped_tree(1, links, 0));
    assert_eq!(budget, 2_173_000);
    // Preliminary closure: 1 + links; definition: 2; root: 2 + uses + padding;
    // each use scans links, walks the empty target (2 steps) and the block (3 steps).
    let steps = |uses: u64| 5 + links as u64 + uses * (links as u64 + 6);
    let uses = (budget - 5 - links as u64) / (links as u64 + 6);
    let padding = usize::try_from(budget - steps(uses)).unwrap();
    assert_eq!(steps(uses) + padding as u64, budget);
    let exact = related_mapped_tree(usize::try_from(uses).unwrap(), links, padding);
    let past = related_mapped_tree(usize::try_from(uses).unwrap(), links, padding + 1);
    assert_eq!((work_budget(&exact), work_budget(&past)), (budget, budget));
    let read = read_step(&exact).unwrap();
    assert!(read.refused.is_empty());
    assert_eq!((read.bodies.len(), read.placements.len()), (1, usize::try_from(uses).unwrap()));
    let error = read_step(&past).unwrap_err();
    assert_eq!(error.code(), pcbcraft_geometry::ErrorCode::ModelUnreadable, "{error}");
    assert!(error.to_string().contains(&format!("the assembly takes more than {budget} steps to expand")), "{error}");

    let huge = related_mapped_tree(50_000, 50_000, 0);
    let budget = work_budget(&huge);
    assert_eq!(budget, 51_173_000);
    let error = read_step(&huge).unwrap_err();
    assert_eq!(error.code(), pcbcraft_geometry::ErrorCode::ModelUnreadable, "{error}");
    assert!(error.to_string().contains(&format!("the assembly takes more than {budget} steps to expand")), "{error}");
}

/// A surface model of many shells placed many times is read in time that does not grow with
/// shells × uses: which bodies the model gives is worked out once, not at each placement. The
/// block's representation also lists a model of 20,000 empty closed shells (refused), and the
/// tree places both 32,768 times; working the bodies out at each use would read the shell list
/// about 6.5·10^8 times, which takes minutes in a debug build.
#[test]
fn a_surface_model_of_many_shells_placed_many_times_reads_in_under_30_s() {
    const SHELLS: usize = 20_000;
    let tree = String::from_utf8(mapped_tree(&[2; 15], false, (0, 0))).unwrap();
    let shells: Vec<String> = (0..SHELLS).map(|i| format!("#{}", 600_001 + i)).collect();
    let mut lines = vec![format!("#600000=SHELL_BASED_SURFACE_MODEL('',({}));", shells.join(","))];
    lines.extend(shells.iter().map(|shell| format!("{shell}=CLOSED_SHELL('',());")));
    let text = append_entities(&tree.replacen("('box',(#159,#167", "('box',(#159,#600000,#167", 1), &lines);
    assert!(text.contains("#600000,#167") && text.contains("#600001=CLOSED_SHELL"));

    let start = std::time::Instant::now();
    let read = read_step(text.as_bytes()).unwrap();
    let seconds = start.elapsed().as_secs_f64();
    assert_eq!(read.bodies.len(), 1);
    assert_eq!(read.placements.len(), 32_768);
    assert!(read.refused.iter().any(|r| r.entity == 600_000), "{:?}", read.refused);
    assert!(seconds < 30.0, "read in {seconds:.1} s");
}

/// Both checkout encodings receive the intended data records, and both raw payload encodings
/// read to the same body and placement counts. Missing/repeated footers fail fixture construction.
#[test]
fn fixture_splices_and_reader_payloads_support_lf_and_crlf() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/step");
    for file in ["box.step", "assembly.step"] {
        let lf = std::fs::read_to_string(dir.join(file)).unwrap().replace("\r\n", "\n");
        let expected = read_step(lf.as_bytes()).unwrap();
        for source in [lf.clone(), lf.replace('\n', "\r\n")] {
            let text = append_entities(&source, &["#900000=SHAPE_REPRESENTATION('',(),#9);".into()]);
            assert_eq!(text.matches("#900000=SHAPE_REPRESENTATION").count(), 1);
            assert!(!text.contains('\r'));
            for payload in [text.clone(), text.replace('\n', "\r\n")] {
                let read = read_step(payload.as_bytes()).unwrap();
                assert!(read.refused.is_empty());
                assert_eq!((read.bodies.len(), read.placements.len()), (expected.bodies.len(), expected.placements.len()));
                assert_eq!(read.bounding_box().unwrap(), expected.bounding_box().unwrap());
            }
        }
    }
    for broken in ["not a STEP footer", "ENDSEC;\nEND-ISO-10303-21;\nENDSEC;\nEND-ISO-10303-21;"] {
        let error = std::panic::catch_unwind(|| append_entities(broken, &[])).unwrap_err();
        let message = error.downcast_ref::<String>().unwrap();
        assert!(message.contains("one data-section footer"), "{message}");
    }
}

/// Alternating mapped targets and plain relationship companions, ending in the block. Each
/// direct mapped chain has length one; the actual placement route has `levels` mapped calls.
fn mixed_mapped_chain(levels: usize) -> (String, u64) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/step");
    let text = std::fs::read_to_string(dir.join("box.step")).unwrap();
    let mut lines = Vec::new();
    let mut below = 168;
    for level in 0..levels as u64 {
        let target = 1_000 + 5 * level;
        let (link, map, item, rep) = (target + 1, target + 2, target + 3, target + 4);
        lines.extend([
            format!("#{target}=SHAPE_REPRESENTATION('',(#167),#9);"),
            format!("#{link}=SHAPE_REPRESENTATION_RELATIONSHIP('','',#{target},#{below});"),
            format!("#{map}=REPRESENTATION_MAP(#167,#{target});"),
            format!("#{item}=MAPPED_ITEM('',#{map},#167);"),
            format!("#{rep}=SHAPE_REPRESENTATION('',(#{item},#167),#9);"),
        ]);
        below = rep;
    }
    let text = text.replacen("#169=SHAPE_DEFINITION_REPRESENTATION(#163,#168);", &format!("#169=SHAPE_DEFINITION_REPRESENTATION(#163,#{below});"), 1);
    (append_entities(&text, &lines), below)
}

/// The pinned product reader accepts the mapped-only depth of this graph. Any too-deep result
/// must be the wrapper pre-count's combined-depth refusal, not a later dependency-walk error.
fn check_mixed_depth(text: &str, placements: Option<usize>) {
    let exchange = solidcore_step::p21::parse(text).unwrap();
    let structure = solidcore_record::step::product::read(&exchange).unwrap();
    assert_eq!(structure.definitions[structure.roots[0]].entity, if structure.occurrences.is_empty() { 162 } else { 900_006 });
    match placements {
        Some(count) => {
            let read = read_step(text.as_bytes()).unwrap();
            assert!(read.refused.is_empty());
            assert_eq!((read.bodies.len(), read.placements.len()), (1, count));
        }
        None => {
            let error = read_step(text.as_bytes()).unwrap_err();
            assert_eq!(error.code(), pcbcraft_geometry::ErrorCode::ModelUnreadable, "{error}");
            assert!(error.to_string().contains("the assembly nests deeper than 32 levels"), "{error}");
        }
    }
}

/// The real mixed graph's last good and first bad depth, plus a long acyclic route which must
/// be refused before native recursion can grow with its length.
#[test]
fn mixed_mapped_relationship_depth_is_bounded_before_expansion() {
    check_mixed_depth(&mixed_mapped_chain(32).0, Some(1));
    check_mixed_depth(&mixed_mapped_chain(33).0, None);
    let long = mixed_mapped_chain(2_048).0;
    assert!(long.len() < 1_000_000);
    check_mixed_depth(&long, None);
}

/// The first root caches a tail shallowly; the second root reaches it one mapped call deeper.
#[test]
fn a_cached_mixed_tail_is_checked_at_its_callers_depth() {
    for (levels, expected) in [(31, Some(2)), (32, None)] {
        let (text, tail) = mixed_mapped_chain(levels);
        let lines = [
            "#900001=SHAPE_REPRESENTATION('',(#167),#9);".into(),
            format!("#900002=SHAPE_REPRESENTATION_RELATIONSHIP('','',#900001,#{tail});"),
            "#900003=REPRESENTATION_MAP(#167,#900001);".into(),
            "#900004=MAPPED_ITEM('',#900003,#167);".into(),
            "#900005=SHAPE_REPRESENTATION('',(#900004,#167),#9);".into(),
            "#900006=PRODUCT_DEFINITION('design','',#161,#4);".into(),
            "#900007=PRODUCT_DEFINITION_SHAPE('','',#900006);".into(),
            "#900008=SHAPE_DEFINITION_REPRESENTATION(#900007,#900005);".into(),
        ];
        check_mixed_depth(&append_entities(&text, &lines), expected);
    }
}

/// Occurrences and mapped calls share the same depth allowance; a plain relationship companion
/// does not consume another level. A shared definition cached through the shallower root also
/// carries its relative mapped height into the deeper occurrence route.
#[test]
fn occurrence_and_mixed_mapping_depth_share_the_limit() {
    for (levels, expected) in [(31, Some(1)), (32, None)] {
        let (text, _) = mixed_mapped_chain(levels);
        let lines = [
            "#900005=SHAPE_REPRESENTATION('',(#167),#9);".into(),
            "#900006=PRODUCT_DEFINITION('design','',#161,#4);".into(),
            "#900007=PRODUCT_DEFINITION_SHAPE('','',#900006);".into(),
            "#900008=SHAPE_DEFINITION_REPRESENTATION(#900007,#900005);".into(),
            "#900009=NEXT_ASSEMBLY_USAGE_OCCURRENCE('1','child','',#900006,#162,$);".into(),
        ];
        check_mixed_depth(&append_entities(&text, &lines), expected);
    }
    for (levels, expected) in [(30, Some(2)), (31, None)] {
        let (text, _) = mixed_mapped_chain(levels);
        let lines = [
            "#900005=SHAPE_REPRESENTATION('',(#167),#9);".into(),
            "#900006=PRODUCT_DEFINITION('design','',#161,#4);".into(),
            "#900007=PRODUCT_DEFINITION_SHAPE('','',#900006);".into(),
            "#900008=SHAPE_DEFINITION_REPRESENTATION(#900007,#900005);".into(),
            "#900009=NEXT_ASSEMBLY_USAGE_OCCURRENCE('1','shallow','',#900006,#162,$);".into(),
            "#900106=PRODUCT_DEFINITION('design','',#161,#4);".into(),
            "#900107=PRODUCT_DEFINITION_SHAPE('','',#900106);".into(),
            "#900108=SHAPE_DEFINITION_REPRESENTATION(#900107,#900005);".into(),
            "#900109=NEXT_ASSEMBLY_USAGE_OCCURRENCE('2','deep','',#900106,#900206,$);".into(),
            "#900206=PRODUCT_DEFINITION('design','',#161,#4);".into(),
            "#900207=PRODUCT_DEFINITION_SHAPE('','',#900206);".into(),
            "#900208=SHAPE_DEFINITION_REPRESENTATION(#900207,#900005);".into(),
            "#900209=NEXT_ASSEMBLY_USAGE_OCCURRENCE('3','tail','',#900206,#162,$);".into(),
        ];
        check_mixed_depth(&append_entities(&text, &lines), expected);
    }
}

/// A flat board-like product graph uses the writer's two representations per part, one plain
/// link between them and one transformed occurrence link. Geometry is shared to keep the test
/// small: 1,000 distinct products/definitions, not 1,000 kernel-built solids or a performance run.
#[test]
fn a_thousand_distinct_products_with_writer_style_links_read() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/step");
    let text = std::fs::read_to_string(dir.join("box.step")).unwrap();
    let text = text.replacen("#168=ADVANCED_BREP_SHAPE_REPRESENTATION('box',(#159,#167),#9);", "#168=SHAPE_REPRESENTATION('board',(#167),#9);", 1);
    let mut lines = Vec::new();
    for part in 0..1_000 {
        let id = 10_000 + 16 * part;
        lines.extend([
            format!("#{id}=PRODUCT('part{part}','part{part}','',(#3));"),
            format!("#{}=PRODUCT_DEFINITION_FORMATION('','',#{id});", id + 1),
            format!("#{}=PRODUCT_DEFINITION('design','',#{},#4);", id + 2, id + 1),
            format!("#{}=PRODUCT_DEFINITION_SHAPE('','',#{});", id + 3, id + 2),
            format!("#{}=ADVANCED_BREP_SHAPE_REPRESENTATION('',(#159,#167),#9);", id + 4),
            format!("#{}=SHAPE_REPRESENTATION('',(#167),#9);", id + 5),
            format!("#{}=SHAPE_DEFINITION_REPRESENTATION(#{},#{});", id + 6, id + 3, id + 5),
            format!("#{}=SHAPE_REPRESENTATION_RELATIONSHIP('','',#{},#{});", id + 7, id + 4, id + 5),
            format!("#{}=NEXT_ASSEMBLY_USAGE_OCCURRENCE('{part}','part','',#162,#{},$);", id + 8, id + 2),
            format!("#{}=PRODUCT_DEFINITION_SHAPE('','',#{});", id + 9, id + 8),
            format!("#{}=ITEM_DEFINED_TRANSFORMATION('','',#167,#167);", id + 10),
            format!("#{}=(REPRESENTATION_RELATIONSHIP('','',#{},#168)REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#{})SHAPE_REPRESENTATION_RELATIONSHIP());", id + 11, id + 5, id + 10),
            format!("#{}=CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#{},#{});", id + 12, id + 11, id + 9),
        ]);
    }
    let text = append_entities(&text, &lines);
    assert!(text.len() < 2_000_000, "small metadata fixture, not a body-heavy benchmark");
    let exchange = solidcore_step::p21::parse(&text).unwrap();
    let structure = solidcore_record::step::product::read(&exchange).unwrap();
    assert_eq!((structure.definitions.len(), structure.occurrences.len(), structure.links.len()), (1_001, 1_000, 2_000));
    assert_eq!(structure.roots.len(), 1);
    let read = read_step(text.as_bytes()).unwrap();
    assert!(read.refused.is_empty());
    assert_eq!((read.bodies.len(), read.placements.len()), (1, 1_000));
    assert!(read.placements.iter().all(|p| p.body == 0 && p.transform.translation_nm == [0, 0, 0]));
    assert_eq!(
        read.bodies[0].bounding_box().unwrap(),
        BoundingBox { min_nm: [-5_000_000, 2_500_000, 0], max_nm: [25_000_000, 22_500_000, 10_000_000] }
    );
}
