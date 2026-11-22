//! Open frozen project archives through the normal native reader, without today's writer.

use sha2::{Digest, Sha256};
use std::fs;

struct Fixture {
    file: &'static str,
    bytes: &'static [u8],
    sha256: &'static str,
    project_id: &'static str,
    sheet_id: &'static str,
    r1_value: &'static str,
    symbols: usize,
    wires: usize,
    footprints: usize,
    tracks: usize,
    vias: usize,
}

const TWO_RESISTORS: Fixture = Fixture {
    file: "two-resistors-v2.pc.zip",
    bytes: include_bytes!("fixtures/native-open/two-resistors-v2.pc.zip"),
    sha256: "d54043f2e551599021525279d04367eeafc79dc16d31fd51972109ba7c73051f",
    project_id: "519a42d4-d658-4905-90fb-a9483718ac39",
    sheet_id: "0bbcf5e3-dd21-47b3-ae6b-bd66c8f97989",
    r1_value: "1k",
    symbols: 2,
    wires: 1,
    footprints: 0,
    tracks: 0,
    vias: 0,
};

// File provenance and exact on-disk formats are recorded beside the archives.
const CORPUS: &[Fixture] = &[
    TWO_RESISTORS,
    Fixture {
        file: "two-resistors-previous-no-manifest.pc.zip",
        bytes: include_bytes!("fixtures/native-open/two-resistors-previous-no-manifest.pc.zip"),
        sha256: "2b8245d11336de5161a2686ee57c4629f8d77ef24392b3384b03a56ae41a8500",
        ..TWO_RESISTORS
    },
    Fixture {
        file: "dual-led-editing-demo-v3.pc",
        bytes: include_bytes!("fixtures/native-open/dual-led-editing-demo-v3.pc"),
        sha256: "20761598da99b3cb688aa57d16cc6f2be71a06869be5446b05f7a26325ceb46c",
        project_id: "ed5fcf30-6942-4ed9-af3d-f73b7ac73ab0",
        sheet_id: "2e84006a-89e6-478f-a66b-df5c1d3907dc",
        r1_value: "1k",
        symbols: 7,
        wires: 12,
        footprints: 7,
        tracks: 0,
        vias: 0,
    },
    Fixture {
        file: "dual-led-flasher-v3.pc",
        bytes: include_bytes!("fixtures/native-open/dual-led-flasher-v3.pc"),
        sha256: "f15a2094331488881ef517968290cd5625b2e6bd77accfe4d34ede4fb59a9dcf",
        project_id: "19c9b4d6-180f-4c57-bcd1-1b9fd1ababd3",
        sheet_id: "842dd612-7afb-4fcd-b503-3c5547e885f0",
        r1_value: "470",
        symbols: 16,
        wires: 40,
        footprints: 14,
        tracks: 82,
        vias: 11,
    },
];

/// Every frozen archive must open with its original identities, example content and board
/// data. Copy just its bytes to a temporary path so the reader never receives a corpus path.
#[test]
fn frozen_native_archives_open() -> Result<(), String> {
    let root = tempfile::tempdir().map_err(|err| format!("creating corpus test directory: {err}"))?;
    for fixture in CORPUS {
        let file = fixture.file;
        assert_eq!(format!("{:x}", Sha256::digest(fixture.bytes)), fixture.sha256, "{file}: frozen archive bytes changed");
        let path = root.path().join(file);
        fs::write(&path, fixture.bytes).map_err(|err| format!("staging {file}: {err}"))?;
        let project = pcbcraft_storage::open(&path).map_err(|err| format!("opening {file}: {err}"))?;
        assert_eq!(project.id().to_string(), fixture.project_id, "{file}: project identity");
        assert_eq!(project.sheets().len(), 1, "{file}: sheet count");
        assert_eq!(project.sheet_target().id().to_string(), fixture.sheet_id, "{file}: sheet identity");
        assert_eq!(project.symbols().len(), fixture.symbols, "{file}: symbol count");
        assert_eq!(project.wires().len(), fixture.wires, "{file}: wire count");
        let r1 = project.components().find(|part| part.reference() == "R1").ok_or_else(|| format!("{file}: missing R1"))?;
        assert_eq!(r1.value(), fixture.r1_value, "{file}: R1 value");
        let board = project.board_target().and_then(|target| project.board_view(target)).ok_or_else(|| format!("{file}: missing board"))?;
        assert_eq!(board.footprint_ids().len(), fixture.footprints, "{file}: footprint count");
        assert_eq!(board.tracks().len(), fixture.tracks, "{file}: track count");
        assert_eq!(board.vias().len(), fixture.vias, "{file}: via count");
    }
    Ok(())
}
