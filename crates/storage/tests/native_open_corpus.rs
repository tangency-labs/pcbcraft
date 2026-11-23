//! Open frozen project archives through the normal native reader, without today's writer.

use pcbcraft_engine::{
    Session,
    netlist::{EXPORT_NETLIST, Netlist, TerminalRef},
    persistence::{Encoding, decode_packed},
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::Path};

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

const TWO_RESISTORS_MANIFESTLESS: Fixture = Fixture {
    file: "two-resistors-previous-no-manifest.pc.zip",
    bytes: include_bytes!("fixtures/native-open/two-resistors-previous-no-manifest.pc.zip"),
    sha256: "2b8245d11336de5161a2686ee57c4629f8d77ef24392b3384b03a56ae41a8500",
    ..TWO_RESISTORS
};

// File provenance and exact on-disk formats are recorded beside the archives.
const CORPUS: &[Fixture] = &[
    TWO_RESISTORS,
    TWO_RESISTORS_MANIFESTLESS,
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

struct Reference {
    junctions: usize,
    net_labels: usize,
    pads: usize,
    drawings: usize,
    // Each row is an authored name (empty for unnamed nets) and its terminals.
    nets: &'static [(&'static str, &'static str)],
    unconnected: &'static str,
}

const RESISTOR_REFERENCE: Reference =
    Reference { junctions: 0, net_labels: 0, pads: 0, drawings: 0, nets: &[("", "R1.2 R2.1")], unconnected: "R1.1 R2.2" };

const EDITING_REFERENCE: Reference = Reference {
    junctions: 4,
    net_labels: 0,
    pads: 15,
    drawings: 0,
    nets: &[("", "J1.1 R1.1 R2.1 C1.1"), ("", "J1.2 D1.2 D2.2 C1.2"), ("", "R1.2 D1.1"), ("", "R2.2 D2.1")],
    unconnected: "Q1.1 Q1.2 Q1.3",
};

const FLASHER_REFERENCE: Reference = Reference {
    junctions: 18,
    net_labels: 8,
    pads: 30,
    drawings: 1,
    nets: &[
        ("+3V3", "J1.1 R1.1 R2.1 R3.1 R4.1 C3.1 C4.1 PWR1.1"),
        ("GND", "J1.2 Q1.2 Q2.2 C3.2 C4.2 PWR2.1"),
        ("LED_A", "R1.2 D1.1"),
        ("LED_B", "R2.2 D2.1"),
        ("COL_A", "D1.2 Q1.3 C1.1 J2.1"),
        ("COL_B", "D2.2 Q2.3 C2.1 J2.2"),
        ("BASE_A", "Q1.1 R3.2 C2.2"),
        ("BASE_B", "Q2.1 R4.2 C1.2"),
    ],
    unconnected: "",
};

/// Open the frozen bytes through the native reader and compare to the source circuit,
/// not to the result of saving or reopening with the current build.
#[test]
fn archived_netlist_and_object_counts_are_preserved() -> Result<(), String> {
    let root = tempfile::tempdir().map_err(|error| error.to_string())?;
    for fixture in CORPUS {
        let expected = match fixture.file {
            "two-resistors-v2.pc.zip" | "two-resistors-previous-no-manifest.pc.zip" => &RESISTOR_REFERENCE,
            "dual-led-editing-demo-v3.pc" => &EDITING_REFERENCE,
            "dual-led-flasher-v3.pc" => &FLASHER_REFERENCE,
            file => return Err(format!("{file}: missing source reference for the compatibility test")),
        };
        let path = root.path().join(fixture.file);
        fs::write(&path, fixture.bytes).map_err(|error| format!("{}: staging archive: {error}", fixture.file))?;
        verify_board(&path, fixture, expected)?;
    }
    Ok(())
}

/// Exercise explicit conversion of both previous split archive layouts and reopen the output.
/// Compare it to the original circuit, so agreeing with an already lossy Open is not enough.
#[test]
fn converted_split_archives_preserve_netlist_and_object_counts() -> Result<(), String> {
    let root = tempfile::tempdir().map_err(|error| error.to_string())?;
    for fixture in [TWO_RESISTORS, TWO_RESISTORS_MANIFESTLESS] {
        let source = root.path().join(fixture.file);
        let archive = root.path().join(format!("converted-{}.pc", fixture.file));
        fs::write(&source, fixture.bytes).map_err(|error| format!("{}: staging archive: {error}", fixture.file))?;
        pcbcraft_storage::conversion::convert(&source, &archive, Encoding::Zip).map_err(|error| format!("{}: converting: {error}", fixture.file))?;
        let bytes = fs::read(&archive).map_err(|error| format!("{}: reading conversion output: {error}", fixture.file))?;
        let packed = decode_packed(&bytes).map_err(|error| format!("{}: validating conversion output: {error}", fixture.file))?;
        assert!(!packed.needs_manifest, "{}: conversion did not write a current manifest", fixture.file);
        verify_board(&archive, &fixture, &RESISTOR_REFERENCE)?;
        let original = fs::read(&source).map_err(|error| format!("{}: reading conversion source: {error}", fixture.file))?;
        assert_eq!(original.as_slice(), fixture.bytes, "{}: conversion changed the source bytes", fixture.file);
    }
    Ok(())
}

/// Check the single-sheet, single-board fixtures and run the same read-only netlist command
/// used by the app. A missing board or export is a test failure, not an empty result.
fn verify_board(path: &Path, fixture: &Fixture, expected: &Reference) -> Result<(), String> {
    let file = path.display();
    let project = pcbcraft_storage::open(path).map_err(|error| format!("{file}: opening: {error}"))?;
    assert_eq!(project.sheets().len(), 1, "{file}: sheets");
    assert_eq!(project.boards().count(), 1, "{file}: boards");
    assert_eq!(project.symbols().len(), fixture.symbols, "{file}: placed symbols");
    assert_eq!(project.wires().len(), fixture.wires, "{file}: wires");
    assert_eq!(project.junctions().count(), expected.junctions, "{file}: junctions");
    assert_eq!(project.net_labels().len(), expected.net_labels, "{file}: net labels");
    let board = project.board_target().and_then(|target| project.board_view(target)).ok_or_else(|| format!("{file}: missing board"))?;
    assert_eq!(board.footprint_ids().len(), fixture.footprints, "{file}: footprints");
    assert_eq!(board.footprints().map(|footprint| footprint.pads.len()).sum::<usize>(), expected.pads, "{file}: pads");
    assert_eq!(board.tracks().len(), fixture.tracks, "{file}: track segments");
    assert_eq!(board.vias().len(), fixture.vias, "{file}: vias");
    assert_eq!(board.drawings().len(), expected.drawings, "{file}: board drawings");

    let mut session = Session::default();
    session.open_snapshot(project);
    let reply = session.execute(EXPORT_NETLIST, &json!({})).map_err(|error| format!("{file}: exporting netlist: {error}"))?;
    let text = reply["netlist"]["files"][0]["contents"].as_str().ok_or_else(|| format!("{file}: missing netlist text"))?;
    let netlist = Netlist::parse(text).map_err(|error| format!("{file}: {error}"))?;
    let references: BTreeMap<_, _> = netlist.components.iter().map(|part| (part.id.as_str(), part.reference.as_str())).collect();
    assert_eq!(references.len(), netlist.components.len(), "{file}: duplicate part ID");
    // All fixtures have exactly one placed unit per logical part, including power symbols.
    assert_eq!(netlist.components.len(), fixture.symbols, "{file}: logical parts");

    let terminal = |pin: &TerminalRef| -> Result<String, String> {
        let reference = references.get(pin.part.as_str()).ok_or_else(|| format!("{file}: unknown netlist part {}", pin.part))?;
        Ok(format!("{reference}.{}", pin.terminal))
    };
    let nets = netlist
        .nets
        .iter()
        .map(|net| Ok((sorted(net.names.clone()), sorted(net.terminals.iter().map(terminal).collect::<Result<Vec<_>, String>>()?))))
        .collect::<Result<Vec<_>, String>>()?;
    let nets = sorted(nets);
    let expected_nets = sorted(
        expected
            .nets
            .iter()
            .map(|&(name, pins)| {
                let names = if name.is_empty() { Vec::new() } else { vec![name.to_owned()] };
                (names, tokens(pins))
            })
            .collect(),
    );
    assert_eq!(nets, expected_nets, "{file}: authored names and net membership");
    assert_eq!(
        sorted(netlist.unconnected.iter().map(terminal).collect::<Result<Vec<_>, _>>()?),
        tokens(expected.unconnected),
        "{file}: unconnected terminals",
    );
    let terminals = netlist.components.iter().flat_map(|part| part.terminals.iter().map(move |pin| format!("{}.{}", part.reference, pin.terminal)));
    let expected_terminals = expected_nets.into_iter().flat_map(|(_, pins)| pins).chain(tokens(expected.unconnected));
    assert_eq!(sorted(terminals.collect()), sorted(expected_terminals.collect()), "{file}: terminal inventory");
    Ok(())
}

/// Sort projections without losing duplicates that would indicate a changed circuit.
fn sorted<T: Ord>(mut values: Vec<T>) -> Vec<T> {
    values.sort_unstable();
    values
}

/// Turn one static reference row into the same sorted representation as the export.
fn tokens(row: &str) -> Vec<String> {
    sorted(row.split_ascii_whitespace().map(str::to_owned).collect())
}
