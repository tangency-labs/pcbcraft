//! The `envelope-v1` workload: one board at the documented capacity envelope with seeded
//! geometry and counts, built entirely through shared engine commands. Production commands
//! allocate random UUIDs, so bytes are stable across save/reopen/re-encode rather than across
//! separate generations.
//!
//! Envelope counts come from the engine's documented limits on `main`:
//! - `MAX_TRACKS = 65_536` (`crates/engine/src/board/track.rs:59`, `docs/formats/native-project-storage.md`)
//! - `MAX_VIAS = 16_384` (`crates/engine/src/board/track.rs:61`, `docs/formats/native-project-storage.md`)
//! - `MAX_PADS = 1,024`, `MAX_ARTWORK = 256` (`crates/engine/src/board/definition.rs:10,12`, `docs/formats/native-project-storage.md`)
//! - `MAX_TERMINALS = 1,024` (`crates/engine/src/library.rs:92`, `docs/formats/native-project-storage.md`)
//! - 512 footprints (chosen to match `board-limits-v1`, not a limit: 511 discrete resistors plus
//!   one 32 x 32 BGA, within `MAX_SYMBOLS = 8,192` at `crates/engine/src/schematic.rs:27`)
//! - Storage bounds: `MAX_DOCUMENT_BYTES = 16 MiB` (`crates/engine/src/persistence/documents.rs:41`),
//!   `MAX_DOCUMENTS_BYTES = 48 MiB` (`crates/engine/src/persistence/documents.rs:43`),
//!   `MAX_ARCHIVE_BYTES = 64 MiB` (`crates/engine/src/persistence.rs:63`).

use super::{Result, fixture::apply};
use pcbcraft_engine::{
    Session,
    board::{
        clipboard::{COPY, PASTE},
        drawing::ADD_DRAWING,
        setup::LayerPurpose,
        track::{ADD_TRACKS, MAX_TRACKS, MAX_VIAS},
        view::BoardTarget,
    },
    schematic::Point,
};
use serde_json::{Value, json};
use uuid::Uuid;

/// Discrete two-terminal resistors placed alongside the 1,024-ball array (512 footprints total).
pub(super) const DISCRETE_PARTS: i64 = 511;
/// Total physical footprints on the envelope board (511 resistors plus one 32 x 32 BGA).
pub(super) const FOOTPRINTS: usize = DISCRETE_PARTS as usize + 1;
/// Pad count on the envelope board's BGA footprint (`crates/engine/src/board/definition.rs:10`).
pub(super) const BGA_PADS: usize = 1024;
/// Multi-unit schematic symbols representing the 1,024-terminal BGA (`BALLS / PINS_PER_UNIT`,
/// `crates/app/src/performance/bga.rs:19,23`).
pub(super) const BGA_UNITS: usize = 8;

const MM: i64 = 1_000_000;
/// Columns per routed seed block: 32 columns x 128 segments = 4,096 tracks and 1,024 vias.
const COLUMNS: i64 = 32;
/// Pasted blocks in a 2 x 8 grid: 16 x 4,096 = 65,536 tracks and 16 x 1,024 = 16,384 vias.
const COPIES: i64 = 16;
/// Vertices per routed column (128 segments, one via every fourth vertex -> 32 vias per column).
const VERTICES: i64 = 129;
/// Track width in nanometres (0.20 mm), giving 0.40 mm edge clearance between 1 mm columns.
const TRACK_WIDTH_NM: i64 = 200_000;
/// Through-via pad diameter in nanometres (0.60 mm).
const VIA_DIAMETER_NM: i64 = 600_000;
/// Through-via drill diameter in nanometres (0.30 mm).
const VIA_DRILL_NM: i64 = 300_000;
/// Left origin of the routed copper grid, clear of the discrete footprints and BGA courtyard.
const COPPER_ORIGIN_X_NM: i64 = 100 * MM;
/// Bottom origin of the routed copper grid, clear of the BGA courtyard.
const COPPER_ORIGIN_Y_NM: i64 = 10 * MM;
/// Vertical spacing between the two rows of eight pasted copper blocks (64 mm tall + 6 mm gap).
const ROW_STEP_NM: i64 = 70 * MM;

/// Displacement of each pasted copper block relative to the initial 32-column block.
fn block_offset(copy: i64) -> Point {
    Point { x_nm: (copy % 8) * COLUMNS * MM, y_nm: (copy / 8) * ROW_STEP_NM }
}

/// Add the board outline and fill copper to `MAX_TRACKS` and `MAX_VIAS` using only shared
/// routing and board-clipboard commands, then verify every envelope count on the board.
pub(super) fn populate_board(session: &mut Session, board: BoardTarget, copper: [Uuid; 2]) -> Result<()> {
    let outline = session
        .project()
        .and_then(|project| project.board_view(board))
        .ok_or("missing envelope board")?
        .layers()
        .into_iter()
        .find(|layer| layer.purpose == LayerPurpose::Outline)
        .map(|layer| layer.id)
        .ok_or("envelope board requires an outline layer")?;
    apply(
        session,
        ADD_DRAWING,
        json!({
            "board": board.board,
            "layer": outline,
            "points": [
                Point { x_nm: -10 * MM, y_nm: -65 * MM },
                Point { x_nm: 365 * MM, y_nm: -65 * MM },
                Point { x_nm: 365 * MM, y_nm: 155 * MM },
                Point { x_nm: -10 * MM, y_nm: 155 * MM },
            ],
            "closed": true,
        }),
    )?;

    for column in 0..COLUMNS {
        let points: Vec<_> =
            (0..VERTICES).map(|vertex| Point { x_nm: COPPER_ORIGIN_X_NM + column * MM, y_nm: COPPER_ORIGIN_Y_NM + vertex * 500_000 }).collect();
        let changes: Vec<_> = (1..=VERTICES / 4).map(|step| json!({"at": step * 4, "layer": copper[step as usize % 2]})).collect();
        apply(
            session,
            ADD_TRACKS,
            json!({
                "board": board.board,
                "layer": copper[0],
                "width_nm": TRACK_WIDTH_NM,
                "points": points,
                "layer_changes": changes,
                "via_diameter_nm": VIA_DIAMETER_NM,
                "via_drill_nm": VIA_DRILL_NM,
            }),
        )?;
    }

    let (track_ids, via_ids) = {
        let project = session.project().ok_or("missing envelope project")?;
        let view = project.board_view(board).ok_or("missing envelope board")?;
        (view.tracks().iter().map(|track| track.id).collect::<Vec<_>>(), view.vias().iter().map(|via| via.id).collect::<Vec<_>>())
    };
    let copied = session
        .execute(
            COPY,
            &json!({
                "board": board.board,
                "items": {
                    "footprints": [],
                    "drawings": [],
                    "tracks": track_ids,
                    "vias": via_ids,
                },
                "expected_revision": session.revision(),
            }),
        )
        .map_err(|error| format!("{COPY}: {error}"))?;
    let text = copied["clipboard"].as_str().ok_or("board copy returned no clipboard text")?;
    let fragment: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let anchor: Point = serde_json::from_value(fragment["anchor"].clone()).map_err(|error| error.to_string())?;

    for copy in 1..COPIES {
        let offset = block_offset(copy);
        apply(
            session,
            PASTE,
            json!({
                "board": board.board,
                "text": text,
                "x_nm": anchor.x_nm + offset.x_nm,
                "y_nm": anchor.y_nm + offset.y_nm,
            }),
        )?;
    }

    let project = session.project().ok_or("missing envelope project")?;
    let view = project.board_view(board).ok_or("missing envelope board")?;
    let symbols = project.all_symbols().count();
    let expected_symbols = DISCRETE_PARTS as usize + BGA_UNITS;
    let footprints = view.footprints().count();
    let max_pads = view.footprints().map(|footprint| footprint.pads.len()).max().unwrap_or(0);
    if view.tracks().len() != MAX_TRACKS
        || view.vias().len() != MAX_VIAS
        || footprints != FOOTPRINTS
        || symbols != expected_symbols
        || max_pads != BGA_PADS
    {
        return Err(format!(
            "envelope board counts mismatch: tracks={} (expected {MAX_TRACKS}), vias={} (expected {MAX_VIAS}), footprints={footprints} (expected {FOOTPRINTS}), symbols={symbols} (expected {expected_symbols}), max_pads={max_pads} (expected {BGA_PADS})",
            view.tracks().len(),
            view.vias().len(),
        ));
    }
    Ok(())
}
