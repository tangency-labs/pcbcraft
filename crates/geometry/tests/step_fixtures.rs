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

#[test]
fn every_fixture_round_trips_within_one_micrometre() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/step");
    let (tolerance, fixtures) = fixtures(&dir);
    assert_eq!(tolerance, 1_000, "the qualification tolerance is one micrometre");
    assert_eq!(fixtures.len(), FIXTURE_COUNT);

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
