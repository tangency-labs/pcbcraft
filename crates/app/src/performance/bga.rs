//! The `bga-v1` workload: one original 32 x 32 ball grid array at the footprint, symbol and
//! terminal-map limits (#251), loaded, placed and captured through the shared commands. It has
//! no routing; fan-out and high-pin-count routing workloads belong to #253.
//!
//! The library records match the engine's capacity test (`part_instances::tests::capacity`).

use super::{Result, fixture::apply, timed};
use pcbcraft_engine::{
    PLACE_ASSIGNED_FOOTPRINT, PLACE_UNIT, Project, Session,
    board::{drc::Rules, update::INITIALIZE_BASELINE},
    library::sources::{self, Documents},
    part_instances::PLACE_PART,
    persistence::{decode_packed, encode_documents, encode_packed},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, hint::black_box, sync::Arc};

/// Balls in the grid: the footprint pad and symbol terminal limits.
const BALLS: usize = 1024;
/// Fabrication strokes: the footprint artwork limit.
const ARTWORK: usize = 256;
/// Pins drawn in each symbol unit; one unit has only 441 grid spots within the pin reach.
const PINS_PER_UNIT: usize = 128;
/// Board position of the array, clear of the reference fixture's parts, tracks, replay route and probes.
const BOARD_AT: (i64, i64) = (40_000_000, -40_000_000);
/// Schematic row of the units, above the reference fixture's resistors.
const SHEET_Y: i64 = -76_200_000;

/// Row-letter, column-number ball names in row order, 32 balls to a row (no I, O, Q, S, X or Z).
fn balls() -> Vec<String> {
    const ROWS: [&str; 32] = [
        "A", "B", "C", "D", "E", "F", "G", "H", "J", "K", "L", "M", "N", "P", "R", "T", "U", "V", "W", "Y", "AA", "AB", "AC", "AD", "AE", "AF", "AG",
        "AH", "AJ", "AK", "AL", "AM",
    ];
    (0..BALLS).map(|index| format!("{}{}", ROWS[index / 32], index % 32 + 1)).collect()
}

/// The native library: round 0.5 mm lands at 1 mm pitch, a symbol in eight format 4 units and
/// a part mapping every terminal to the ball of the same name.
fn documents() -> Documents {
    let names = balls();
    let pads: Vec<_> = names
        .iter()
        .enumerate()
        .map(|(index, number)| {
            let (row, column) = ((index / 32) as i64, (index % 32) as i64);
            json!({"number":number,"center":{"x_nm":(2 * column - 31) * 500_000,"y_nm":(2 * row - 31) * 500_000},
                "width_nm":500_000,"height_nm":500_000,"rotation":0,"shape":"circle","copper":"component_side"})
        })
        .collect();
    let artwork: Vec<_> = (0..ARTWORK as i64)
        .map(|index| {
            let x = index * 100_000 - 12_800_000;
            json!({"purpose":"fabrication","graphic":{"geometry":{"type":"line","from":{"x_nm":x,"y_nm":-17_000_000},"to":{"x_nm":x,"y_nm":-16_600_000}},
                "stroke_nm":50_000,"fill":"none"}})
        })
        .collect();
    let source = json!({"kind":"original","reference":"PCBcraft performance fixture","locator":"Synthetic 1 mm pitch grid"});
    let footprint = json!({"format":"pcbcraft-footprint/1","library":"grid","id":"bga-1024","revision":1,"name":"Ball grid array, 32 x 32",
        "license":"MIT OR Apache-2.0","source":source,"pads":pads,"artwork":artwork,
        "graphics":[{"purpose":"courtyard","center":{"x_nm":0,"y_nm":0},"width_nm":34_000_000,"height_nm":34_000_000,"rotation":0}]});
    let units: Vec<_> = names
        .chunks(PINS_PER_UNIT)
        .enumerate()
        .map(|(unit, chunk)| {
            let pins: Vec<_> = chunk
                .iter()
                .enumerate()
                .map(|(index, name)| json!({"terminal":name,"at_nm":[(index as i64 % 16 - 8) * 1_270_000,(index as i64 / 16 - 4) * 1_270_000]}))
                .collect();
            json!({"id":format!("u{}", unit + 1),"drawing":[{"type":"rect","min_nm":[-11_430_000,-6_350_000],"max_nm":[10_160_000,5_080_000]}],"pins":pins})
        })
        .collect();
    let terminals: Vec<_> = names.iter().map(|name| json!({"id":name,"electrical_type":"passive"})).collect();
    let symbol = json!({"format":"pcbcraft-symbol/4","id":"bga-1024","revision":1,"name":"Ball grid array, 32 x 32","reference_prefix":"U",
        "default_value":"BGA","terminals":terminals,"units":units});
    let mapping: serde_json::Map<_, _> = names.iter().map(|name| (name.clone(), json!([name]))).collect();
    let part = json!({"format":"pcbcraft-part/1","library":"grid","id":"bga-1024","revision":1,"name":"Ball grid array, 32 x 32",
        "license":"MIT OR Apache-2.0","source":source,"symbol":{"library":"grid","id":"bga-1024","revision":1},
        "packages":{"bga":{"name":"BGA","footprint":{"library":"grid","id":"bga-1024","revision":1},"mapping":mapping}}});
    [
        ("library.json", json!({"format":"pcbcraft-library/1","id":"grid","name":"Grid arrays","license":"MIT OR Apache-2.0"})),
        ("symbols/bga-1024.json", symbol),
        ("footprints/bga-1024.json", footprint),
        ("parts/bga-1024.json", part),
    ]
    .into_iter()
    .map(|(path, record)| (path.to_owned(), record.to_string()))
    .collect()
}

/// Record a timing only when a sample map is given; fixture generation passes none.
fn step<T>(samples: &mut Option<&mut BTreeMap<String, Vec<f64>>>, id: &str, operation: impl FnOnce() -> T) -> T {
    match samples {
        Some(samples) => timed(samples, id, operation),
        None => operation(),
    }
}

/// Load the library, place the part with all eight units and its footprint with the default
/// map of all 1,024 terminals, all through the shared revision-checked commands.
pub(super) fn place(session: &mut Session, mut samples: Option<&mut BTreeMap<String, Vec<f64>>>) -> Result<()> {
    let documents = documents();
    let loaded = step(&mut samples, "bga.library_load", || session.execute(sources::LOAD, &json!({"source":"grid","documents":documents})))
        .map_err(|e| e.to_string())?;
    let problems = loaded.as_array().and_then(|all| all.iter().find(|source| source["source"] == "grid")).map(|source| &source["problems"]);
    if problems != Some(&json!([])) {
        return Err(format!("BGA library did not load cleanly: {problems:?}"));
    }
    let resolved = session
        .execute(sources::RESOLVE_PART, &json!({"library":"grid","definition":"bga-1024","definition_revision":1}))
        .map_err(|e| e.to_string())?;
    step(&mut samples, "bga.place_part", || {
        apply(
            session,
            PLACE_PART,
            json!({"library":"grid","definition":"bga-1024","definition_revision":1,"definition_digest":resolved["digest"],"x_nm":30_480_000,"y_nm":SHEET_Y}),
        )
    })?;
    let component = session.project().and_then(|p| p.symbols().last().map(|s| s.component_id())).ok_or("missing BGA symbol")?;
    step(&mut samples, "bga.place_units", || {
        (2..=BALLS / PINS_PER_UNIT).try_for_each(|unit| {
            apply(session, PLACE_UNIT, json!({"component":component,"unit":format!("u{unit}"),"x_nm":unit as i64 * 30_480_000,"y_nm":SHEET_Y}))
        })
    })?;
    step(&mut samples, "bga.place_footprint", || {
        apply(session, PLACE_ASSIGNED_FOOTPRINT, json!({"component":component,"x_nm":BOARD_AT.0,"y_nm":BOARD_AT.1}))
    })?;
    check_mapping(session)
}

/// Require the placed array's default map, read through the shared project inspection (the
/// MCP and CLI view): all 1,024 terminals, each mapped to the ball of the same name.
fn check_mapping(session: &Session) -> Result<()> {
    let inspection = session.inspect();
    let footprints = inspection["project"]["board"]["footprints"].as_array().ok_or("inspection lists no board footprints")?;
    let mapping = footprints
        .iter()
        .filter_map(|footprint| footprint["mapping"].as_object())
        .max_by_key(|mapping| mapping.len())
        .ok_or("BGA footprint has no terminal map")?;
    if mapping.len() != BALLS {
        return Err(format!("BGA footprint maps {} terminals, expected {BALLS}", mapping.len()));
    }
    let mismatched = mapping.iter().filter(|(terminal, pads)| **pads != json!(terminal) && **pads != json!([terminal])).count();
    if mismatched != 0 {
        return Err(format!("BGA footprint maps {mismatched} terminals to a pad of another name, expected 0"));
    }
    Ok(())
}

/// Require the accepted baseline to hold every terminal, read from the saved board document.
fn check_baseline(project: Arc<Project>) -> Result<()> {
    let documents = encode_documents(project)?;
    let board = documents.iter().find(|(path, _)| path.ends_with("board.json")).map(|(_, bytes)| bytes).ok_or("no saved board document")?;
    let board: Value = serde_json::from_slice(board).map_err(|e| e.to_string())?;
    let components = board["baseline"]["components"].as_object().ok_or("saved board has no accepted baseline")?;
    let terminals: usize = components.values().filter_map(|component| component["terminals"].as_array()).map(Vec::len).sum();
    if terminals != BALLS {
        return Err(format!("BGA baseline holds {terminals} terminals, expected {BALLS}"));
    }
    Ok(())
}

/// Time one BGA case in a fresh project: library load, placement, baseline capture, packed
/// encode and decode, and the synchronous design check with the array placed. Correctness
/// checks stay outside the timed operations.
pub(super) fn measure(samples: &mut BTreeMap<String, Vec<f64>>) -> Result<()> {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true,"name":"bga-v1"})).map_err(|e| e.to_string())?;
    place(&mut session, Some(&mut *samples))?;
    timed(samples, "bga.baseline", || apply(&mut session, INITIALIZE_BASELINE, json!({})))?;
    let project = session.snapshot().ok_or("missing BGA project")?;
    check_baseline(project.clone())?;
    let bytes = timed(samples, "bga.encode_packed", || encode_packed(project.clone()))?;
    let reopened = timed(samples, "bga.decode_packed", || decode_packed(&bytes))?.project;
    if encode_packed(reopened)? != bytes {
        return Err("BGA project did not reopen to identical bytes".into());
    }
    let board = project.board_target().ok_or("missing BGA board")?;
    let view = project.board_view(board).ok_or("missing BGA board view")?;
    timed(samples, "bga.check_design", || black_box(view.check_design(Rules::default())));
    Ok(())
}
