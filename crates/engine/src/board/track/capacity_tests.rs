//! Board capacity: the indexed island builder against an exhaustive reference, and a board
//! filled to the track and via limits through the ordinary route command.
use super::*;
use crate::{
    ADD_DRAWING, ADD_TRACKS, ADD_VIA, CHECK_DESIGN, INITIALIZE_BOARD_SETUP,
    board::{
        clipboard::{COPY, CUT, DELETE},
        track::tests::{copper_layers, view},
    },
    edits::{CommittedEdit, EditObserver},
    persistence::{MAX_DOCUMENT_BYTES, decode_documents, encode_documents},
};
use serde_json::json;
use std::sync::{Arc, Mutex};

const MM: i64 = 1_000_000;

fn at(x_nm: i64, y_nm: i64) -> Point {
    Point { x_nm, y_nm }
}

/// Join eligible lower-numbered pairs from the builder's current node to `end`, without an index.
fn exhaustive(mut builder: IslandBuilder, end: usize) -> IslandBuilder {
    for i in builder.node..end {
        for j in 0..i {
            let layers = match (builder.layers.get(i), builder.layers.get(j)) {
                (Some(a), Some(b)) => a.iter().any(|layer| b.contains(layer)),
                (None, Some(b)) => !b.is_empty(),
                _ => true,
            };
            if layers && builder.contours[i].touches(&builder.contours[j]) {
                join(&mut builder.parent, i, j);
            }
        }
    }
    builder.node = end;
    builder
}

/// Square pads, front-only and through, tracks of mixed widths on both layers and vias, packed
/// closely enough that many touch. Rows one millimetre apart carry 1 mm tracks, so those rows
/// meet exactly at their edges.
fn crowded(tracks: usize) -> (Vec<Land>, Vec<Track>, Vec<Via>, [Uuid; 2]) {
    let layers = [Uuid::from_u128(1), Uuid::from_u128(2)];
    let lands = (0..tracks / 10)
        .map(|k| {
            let (x, y) = ((k as i64 * 3_100_000) % (20 * MM), (k as i64 * 7 % 20) * MM + 300_000);
            Land {
                key: (format!("footprint-{}", k + 1), "1".into()),
                corners: [at(x, y), at(x + 800_000, y), at(x + 800_000, y + 800_000), at(x, y + 800_000)],
                layers: if k % 3 == 0 { layers.to_vec() } else { vec![layers[k % 2]] },
                net: None,
                center: at(x + 400_000, y + 400_000),
                shape: if k % 4 == 0 { PadShape::Circle } else { PadShape::Rectangle },
                corner_radius_nm: 0,
            }
        })
        .collect();
    let tracks: Vec<_> = (0..tracks)
        .map(|k| {
            let (x, y) = ((k % 20) as i64 * MM, (k / 20) as i64 * MM);
            Track {
                id: Uuid::from_u128(1000 + k as u128),
                locked: false,
                layer: layers[k % 2],
                width_nm: if k % 5 == 0 { MM } else { 200_000 + (k % 7) as i64 * 50_000 },
                start: at(x, y),
                end: at(x + 500_000 * (1 + k as i64 % 3), y + if k % 11 == 0 { 700_000 } else { 0 }),
            }
        })
        .collect();
    let vias = (0..tracks.len() / 8)
        .map(|k| Via {
            id: Uuid::from_u128(100_000 + k as u128),
            locked: false,
            net_assignment: None,
            at: at((k as i64 * 3_300_000) % (20 * MM), (k as i64 * 7 % 20) * MM + 500_000),
            diameter_nm: 600_000,
            drill_nm: 300_000,
        })
        .collect();
    (lands, tracks, vias, layers)
}

/// Both contact phases equal the exhaustive eligible-pair result for tangent copper, pads and
/// vias, on the direct path of a small board and the packed index of a large one, at sampled
/// resume budgets.
#[test]
fn indexed_islands_match_exhaustive_pairs() {
    for count in [30, 600] {
        let (lands, tracks, vias, _) = crowded(count);
        let nodes = lands.len() + tracks.len() + vias.len();
        let expected = exhaustive(IslandBuilder::new(&lands, &tracks, &vias), nodes).finish(&lands, &tracks, &vias);
        let pads = exhaustive(IslandBuilder::pad_contacts(&lands, &tracks, &vias, &expected.1), lands.len()).finish(&lands, &tracks, &vias);
        for budget in [1, 3, 64, usize::MAX] {
            let mut builder = IslandBuilder::new(&lands, &tracks, &vias);
            assert!(!builder.step(0), "a zero budget never completes nonempty work");
            while !builder.step(budget) {}
            let found = builder.finish(&lands, &tracks, &vias);
            assert_eq!(found, expected, "{count} tracks, budget {budget}");
            let mut builder = IslandBuilder::pad_contacts(&lands, &tracks, &vias, &found.1);
            while !builder.step_pads(lands.len(), budget) {}
            assert_eq!(builder.finish(&lands, &tracks, &vias), pads, "{count} tracks, budget {budget}");
        }
        let (copper, _) = &expected;
        assert_eq!(copper.tracks[&tracks[0].id], copper.tracks[&tracks[20].id], "tangent rows share an island");
        assert!(expected.1.len() > 1 && expected.1.len() < nodes, "the fixture has both contacts and separate islands");
    }
}

/// Exactly `items` pads, tracks and vias from the crowded pattern, with a via on a track end
/// and two overlapping pads isolated from the other copper, to distinguish the contact phases.
fn crossing(items: usize) -> (Vec<Land>, Vec<Track>, Vec<Via>) {
    let (mut lands, mut tracks, mut vias, layers) = crowded(2 * items);
    lands.truncate(items / 10);
    for k in 0..2 {
        let (x, y) = (-10 * MM + k * 200_000, -10 * MM);
        lands.push(Land {
            key: (format!("isolated-{}", k + 1), "1".into()),
            corners: [at(x, y), at(x + 800_000, y), at(x + 800_000, y + 800_000), at(x, y + 800_000)],
            layers: vec![layers[0]],
            net: None,
            center: at(x + 400_000, y + 400_000),
            shape: PadShape::Rectangle,
            corner_radius_nm: 0,
        });
    }
    vias.truncate(items / 8);
    tracks.truncate(items - lands.len() - vias.len());
    vias[0].at = tracks[3].end;
    (lands, tracks, vias)
}

/// Both island paths give the exhaustive eligible-pair result below, at and above the switch
/// size, preserving separate legacy pad roots until the pad-contact phase, at sampled budgets.
#[test]
fn pairwise_and_indexed_islands_agree_around_the_switch_size() {
    for items in [PAIRWISE_BELOW - 1, PAIRWISE_BELOW, PAIRWISE_BELOW + 1] {
        let (lands, tracks, vias) = crossing(items);
        assert_eq!(lands.len() + tracks.len() + vias.len(), items);
        assert_eq!(compares_every_pair(items), items < PAIRWISE_BELOW, "{items} items");
        let expected = exhaustive(IslandBuilder::new(&lands, &tracks, &vias), items).finish(&lands, &tracks, &vias);
        let pads = exhaustive(IslandBuilder::pad_contacts(&lands, &tracks, &vias, &expected.1), lands.len()).finish(&lands, &tracks, &vias);
        for pairwise in [true, false] {
            for budget in [1, 7, usize::MAX] {
                let mut builder = IslandBuilder::new(&lands, &tracks, &vias).on_path(pairwise);
                assert!(!builder.step(0), "a zero budget never completes nonempty work");
                while !builder.step(budget) {}
                let found = builder.finish(&lands, &tracks, &vias);
                assert_eq!(found, expected, "{items} items, pairwise {pairwise}, budget {budget}");
                let isolated = [&lands[lands.len() - 2].key, &lands[lands.len() - 1].key];
                assert_ne!(found.0.pads[isolated[0]], found.0.pads[isolated[1]], "legacy islands keep direct pad contacts separate");
                let mut builder = IslandBuilder::pad_contacts(&lands, &tracks, &vias, &found.1).on_path(pairwise);
                while !builder.step_pads(lands.len(), budget) {}
                let joined = builder.finish(&lands, &tracks, &vias);
                assert_eq!(joined, pads, "{items} items, pairwise {pairwise}, budget {budget}");
                assert_eq!(joined.0.pads[isolated[0]], joined.0.pads[isolated[1]], "the pad-contact phase joins the isolated overlapping pads");
            }
        }
        let (copper, members) = &expected;
        assert_eq!(copper.tracks[&tracks[0].id], copper.tracks[&tracks[20].id], "tangent rows share an island");
        assert_eq!(copper.vias[&vias[0].id], copper.tracks[&tracks[3].id], "a via on a track end joins it");
        assert!(members.len() > 1 && members.len() < items, "the board has both contacts and separate islands");
    }
}

/// One more record than either limit is refused by name before any record is examined.
#[test]
fn board_validation_names_the_track_and_via_limits() {
    let mut s = Session::default();
    s.apply("project.new", &json!({"name":"Limits","create_board":true})).unwrap();
    let project = s.project().unwrap();
    let layer = Uuid::from_u128(1);
    let track = |k: usize| Track {
        id: Uuid::from_u128(1 + k as u128),
        locked: false,
        layer,
        width_nm: DEFAULT_WIDTH_NM,
        start: at(0, k as i64 * MM),
        end: at(MM, k as i64 * MM),
    };
    let via = |k: usize| Via {
        id: Uuid::from_u128(1 << 64 | k as u128),
        locked: false,
        net_assignment: None,
        at: at(0, k as i64 * MM),
        diameter_nm: 600_000,
        drill_nm: 300_000,
    };
    let mut tracks: Vec<_> = (0..=MAX_TRACKS).map(track).collect();
    let mut vias: Vec<_> = (0..=MAX_VIAS).map(via).collect();
    assert_eq!(validate(project, &mut tracks, &mut []), Err(format!("a board can hold at most {MAX_TRACKS} track segments")));
    assert_eq!(validate(project, &mut [], &mut vias), Err(format!("a board can hold at most {MAX_VIAS} vias")));
}

/// Published parameter schemas let a client send a whole board's tracks and vias in one call.
#[test]
fn command_schemas_accept_a_whole_board() {
    let selection = crate::ParamKind::BoardSelection.schema();
    assert_eq!(selection["properties"]["tracks"]["maxItems"], MAX_TRACKS);
    assert_eq!(selection["properties"]["vias"]["maxItems"], MAX_VIAS);
    assert_eq!(crate::ParamKind::TrackIds.schema()["maxItems"], MAX_TRACKS);
    assert_eq!(crate::ParamKind::TrackEnds.schema()["maxItems"], MAX_TRACKS);
}

/// Track-ID, track-end and board-selection lists accept their declared bounds and refuse one more
/// with a limit-specific error; malformed IDs or points cannot stand in for the count refusal.
#[test]
fn command_lists_stop_at_the_track_and_via_limits() {
    use crate::ParamKind;
    let ids = |n: usize, kind: u128| (1..=n).map(|k| json!(Uuid::from_u128(kind << 64 | k as u128))).collect::<Vec<_>>();
    let ends = |n: usize| ids(n, 1).into_iter().map(|track| json!({"track":track,"start":at(0, 0),"end":at(MM, 0)})).collect::<Vec<_>>();
    let selection = |tracks: usize, vias: usize| json!({"tracks":ids(tracks, 1),"vias":ids(vias, 2)});

    assert_eq!(ParamKind::TrackIds.validate(&json!(ids(MAX_TRACKS, 1))), Ok(()));
    assert_eq!(ParamKind::TrackIds.validate(&json!(ids(MAX_TRACKS + 1, 1))), Err("must list at most 65536 IDs"));
    assert_eq!(ParamKind::TrackEnds.validate(&json!(ends(MAX_TRACKS))), Ok(()));
    assert_eq!(ParamKind::TrackEnds.validate(&json!(ends(MAX_TRACKS + 1))), Err("must list at most 65536 tracks"));
    assert_eq!(ParamKind::BoardSelection.validate(&selection(MAX_TRACKS, MAX_VIAS)), Ok(()));
    assert_eq!(ParamKind::BoardSelection.validate(&selection(MAX_TRACKS + 1, 0)), Err("must select at most 65536 tracks"));
    assert_eq!(ParamKind::BoardSelection.validate(&selection(0, MAX_VIAS + 1)), Err("must select at most 16384 vias"));
    // The messages are fixed text, so check that each names the constant it enforces.
    for (message, limit) in [
        ("must list at most 65536 IDs", MAX_TRACKS),
        ("must list at most 65536 tracks", MAX_TRACKS),
        ("must select at most 65536 tracks", MAX_TRACKS),
        ("must select at most 16384 vias", MAX_VIAS),
    ] {
        assert!(message.contains(&format!("at most {limit} ")), "{message}");
    }
}

/// A pasted fragment is held to the board's counts. Vias reach the count check; tracks do not,
/// because the text for one more track than the limit, even in records this short, exceeds the
/// 8 MiB text limit, which refuses the paste first.
#[test]
fn pasted_fragments_stop_at_the_via_limit_and_the_text_limit() {
    use crate::board::clipboard::{BoardFragment, PASTE};
    let fragment = |layers: Value, tracks: Vec<Value>, vias: Vec<Value>| {
        json!({
            "format":"pcbcraft-clipboard/1","kind":"board","source_project":Uuid::from_u128(7),"source_board":Uuid::from_u128(8),
            "cut":false,"anchor":at(0, 0),"circuits":[],"footprints":[],"layers":layers,"drawings":[],"tracks":tracks,"vias":vias
        })
        .to_string()
    };
    let via = |k: usize| json!({"id":Uuid::from_u128(2 << 64 | k as u128),"at":at((k % 128) as i64 * MM, (k / 128) as i64 * MM),"diameter_nm":600_000,"drill_nm":300_000});
    let track = |k: usize| json!({"id":Uuid::from_u128(1 << 64 | k as u128),"layer":Uuid::from_u128(9),"width_nm":DEFAULT_WIDTH_NM,"start":at(0, 0),"end":at(MM, 0)});

    let mut s = Session::default();
    s.apply("project.new", &json!({"name":"Paste limits","create_board":true})).unwrap();
    let mut paste = |text: String| s.apply(PASTE, &json!({"text":text,"x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap_err();

    assert!(BoardFragment::from_text(&fragment(json!([]), vec![], (0..MAX_VIAS).map(via).collect())).is_ok());
    let error = paste(fragment(json!([]), vec![], (0..=MAX_VIAS).map(via).collect()));
    assert!(matches!(&error, crate::CommandError::InvalidParams(_)), "{error:?}");
    assert!(error.to_string().contains("board clipboard resource limit exceeded"), "{error}");

    let text = fragment(json!([]), (0..=MAX_TRACKS).map(track).collect(), vec![]);
    assert!(text.len() > crate::clipboard::MAX_BYTES, "{} bytes", text.len());
    let error = paste(text);
    assert!(matches!(&error, crate::CommandError::InvalidParams(_)), "{error:?}");
    assert!(error.to_string().contains("8 MiB"), "{error}");
}

/// Columns of 129 vertices, 0.5 mm apart: 128 segments with a layer change every fourth vertex,
/// so each column adds 32 vias and ends on one. Sixteen pasted copies of a 32-column block, in
/// two rows of eight, make 512 columns and fill both limits exactly.
const COLUMNS: i64 = 32;
const COPIES: i64 = 16;
const VERTICES: i64 = 129;
/// Block placement: copies sit side by side, and the second row of eight starts 70 mm up.
fn block_offset(copy: i64) -> Point {
    at((copy % 8) * COLUMNS * MM, (copy / 8) * 70 * MM)
}

/// Collect committed commands and their resulting revisions, ignoring history-root notifications.
#[derive(Debug, Default)]
struct EditLog(Mutex<Vec<(String, u64)>>);

impl EditObserver for EditLog {
    /// The test measures commits, not the initial root notification.
    fn opened(&self, _: Arc<Project>) {}

    /// Record each publication so the test can distinguish zero, one or multiple transactions.
    fn committed(&self, edit: CommittedEdit) {
        self.0.lock().unwrap().push((edit.command, edit.revision));
    }
}

/// Each public Cut route publishes one edit for a small selection. Execute returns the bounded
/// cut fragment; one Undo from either route restores the removed track.
#[test]
fn small_board_cut_routes_publish_once_and_undo() {
    let mut s = Session::default();
    s.apply("project.new", &json!({"name":"Cut","create_board":true})).unwrap();
    s.apply(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":s.revision()})).unwrap();
    let layer = copper_layers(&s)[0];
    s.apply(ADD_TRACKS, &json!({"layer":layer,"points":[at(MM, MM),at(2 * MM, MM)],"expected_revision":s.revision()})).unwrap();
    let track = view(&s).tracks()[0].id;
    for with_text in [true, false] {
        let log = Arc::new(EditLog::default());
        s.observe_edits(Some(log.clone()));
        let before = s.revision();
        let cut = json!({"items":{"tracks":[track]},"expected_revision":before});
        let revision = if with_text {
            let result = s.execute(CUT, &cut).unwrap();
            let fragment: Value = serde_json::from_str(result["clipboard"].as_str().unwrap()).unwrap();
            assert_eq!(fragment["cut"], true);
            assert_eq!(fragment["tracks"][0]["id"], json!(track));
            result["revision"].as_u64().unwrap()
        } else {
            s.apply(CUT, &cut).unwrap()
        };
        assert_eq!(revision, before + 1);
        assert!(view(&s).tracks().is_empty());
        assert_eq!(*log.0.lock().unwrap(), [(CUT.to_owned(), revision)]);
        s.apply("edit.undo", &Value::Null).unwrap();
        assert_eq!(view(&s).tracks()[0].id, track);
    }
}

/// A two-layer board filled to both limits through route, copy and paste refuses one more of
/// either. Full-selection copy and both cut routes refuse the text size without an edit; plain
/// delete still takes one Undo. The board fits the document limit, reloads identically and checks clean.
#[test]
fn board_at_the_track_and_via_limits_saves_reloads_and_checks_clean() {
    let mut s = Session::default();
    s.apply("project.new", &json!({"name":"Capacity","create_board":true})).unwrap();
    s.apply(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":s.revision()})).unwrap();
    let outline = view(&s).layers().into_iter().find(|layer| layer.purpose == LayerPurpose::Outline).unwrap().id;
    let (right, top) = (10 * MM + 8 * COLUMNS * MM + 5 * MM, 10 * MM + 2 * 70 * MM + 5 * MM);
    s.apply(
        ADD_DRAWING,
        &json!({"layer":outline,"points":[at(5 * MM, 5 * MM),at(right, 5 * MM),at(right, top),at(5 * MM, top)],"closed":true,"expected_revision":s.revision()}),
    )
    .unwrap();
    let copper = copper_layers(&s);
    for column in 0..COLUMNS {
        let points: Vec<_> = (0..VERTICES).map(|v| at(10 * MM + column * MM, 10 * MM + v * 500_000)).collect();
        let changes: Vec<_> = (1..VERTICES / 4 + 1).map(|k| json!({"at":k * 4,"layer":copper[k as usize % 2]})).collect();
        s.apply(
            ADD_TRACKS,
            &json!({"layer":copper[0],"width_nm":200_000,"points":points,"layer_changes":changes,"via_diameter_nm":600_000,"via_drill_nm":300_000,"expected_revision":s.revision()}),
        )
        .unwrap();
    }
    let board = &s.project().unwrap().board;
    let block = json!({
        "footprints":[],
        "drawings":[],
        "tracks":board.tracks.iter().map(|t| t.id).collect::<Vec<_>>(),
        "vias":board.vias.iter().map(|v| v.id).collect::<Vec<_>>(),
    });
    let copied = s.execute(crate::board::clipboard::COPY, &json!({"items":block,"expected_revision":s.revision()})).unwrap();
    let text = copied["clipboard"].as_str().unwrap();
    let anchor: Point = serde_json::from_value(serde_json::from_str::<Value>(text).unwrap()["anchor"].clone()).unwrap();
    for copy in 1..COPIES {
        let offset = block_offset(copy);
        let (x, y) = (anchor.x_nm + offset.x_nm, anchor.y_nm + offset.y_nm);
        s.apply(crate::board::clipboard::PASTE, &json!({"text":text,"x_nm":x,"y_nm":y,"expected_revision":s.revision()})).unwrap();
    }
    let board = &s.project().unwrap().board;
    assert_eq!((board.tracks.len(), board.vias.len()), (MAX_TRACKS, MAX_VIAS));

    let extra = [at(5 * MM + MM, 5 * MM + MM), at(5 * MM + 2 * MM, 5 * MM + MM)];
    let error = s.apply(ADD_TRACKS, &json!({"layer":copper[0],"points":extra,"expected_revision":s.revision()})).unwrap_err();
    assert!(matches!(&error, crate::CommandError::InvalidParams(_)), "{error:?}");
    assert!(error.to_string().contains(&format!("a board can hold at most {MAX_TRACKS} track segments")), "{error}");
    let error = s.apply(ADD_VIA, &json!({"at":extra[0],"expected_revision":s.revision()})).unwrap_err();
    assert!(matches!(&error, crate::CommandError::InvalidParams(_)), "{error:?}");
    assert!(error.to_string().contains(&format!("a board can hold at most {MAX_VIAS} vias")), "{error}");
    let error = s.apply(crate::board::clipboard::PASTE, &json!({"text":text,"x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap_err();
    assert!(matches!(&error, crate::CommandError::InvalidParams(_)), "{error:?}");
    assert!(error.to_string().contains(&format!("a board can hold at most {MAX_TRACKS} track segments")), "{error}");

    let board = &s.project().unwrap().board;
    let everything = json!({
        "tracks":board.tracks.iter().map(|t| t.id).collect::<Vec<_>>(),
        "vias":board.vias.iter().map(|v| v.id).collect::<Vec<_>>(),
    });
    let snapshot = s.snapshot().unwrap();
    let revision = s.revision();
    let history = (s.undo.len(), s.redo.len());
    let log = Arc::new(EditLog::default());
    s.observe_edits(Some(log.clone()));
    let selection = json!({"items":everything,"expected_revision":revision});
    for (command, with_result) in [(COPY, true), (CUT, true), (CUT, false)] {
        let error = if with_result { s.execute(command, &selection).unwrap_err() } else { s.apply(command, &selection).unwrap_err() };
        assert!(matches!(&error, crate::CommandError::InvalidParams(reason) if reason == "clipboard exceeds the 8 MiB limit"), "{error:?}");
        assert_eq!(s.revision(), revision);
        assert!(Arc::ptr_eq(&s.snapshot().unwrap(), &snapshot));
        assert_eq!((s.undo.len(), s.redo.len()), history);
        assert!(log.0.lock().unwrap().is_empty());
    }
    s.apply(DELETE, &selection).unwrap();
    let board = &s.project().unwrap().board;
    assert_eq!((board.tracks.len(), board.vias.len()), (0, 0));
    s.apply("edit.undo", &Value::Null).unwrap();
    let board = &s.project().unwrap().board;
    assert_eq!((board.tracks.len(), board.vias.len()), (MAX_TRACKS, MAX_VIAS));

    let saved = encode_documents(s.snapshot().unwrap()).unwrap();
    let largest = saved.values().map(Vec::len).max().unwrap();
    assert!(largest <= MAX_DOCUMENT_BYTES, "{largest} bytes");
    let reopened = decode_documents(&saved).unwrap();
    assert_eq!(encode_documents(reopened.clone()).unwrap(), saved, "saving a reopened board changes nothing");

    let mut loaded = Session::default();
    loaded.open_snapshot(reopened);
    let result = loaded.execute(CHECK_DESIGN, &json!({})).unwrap();
    assert_eq!(result["design_check"]["total"], 0, "{:#}", result["design_check"]["counts"]);
}

/// Track `k` of `n` parallel minimum-width diagonals 20 um apart along both axes, so 14 um apart
/// across their width: each runs 1 mm past the origin on both axes, so every track's bounds
/// contain the origin, while no two tracks touch.
#[cfg(not(target_arch = "wasm32"))]
fn diagonal(k: usize, n: usize) -> Track {
    const PITCH: i64 = 20_000;
    let c = (k as i64 - n as i64 / 2) * PITCH;
    let (start, end) = if c >= 0 { (at(-MM, c + MM), at(c + MM, -MM)) } else { (at(c - MM, MM), at(MM, c - MM)) };
    Track { id: Uuid::from_u128(1 << 64 | k as u128), locked: false, layer: Uuid::from_u128(9), width_nm: MIN_WIDTH_NM, start, end }
}

/// Parallel tracks with overlapping bounds and no copper contact exercise the island check's
/// all-pairs case. The test checks the candidate count and that each track stays isolated, and that the
/// layout still fits the coordinate range at the track limit. It then writes the build time at
/// each size to standard error, which the test harness does not capture, for the extrapolation
/// in `docs/performance-coverage.md`. This timing diagnostic is run only on native targets.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn overlapping_copper_reaches_every_pair_and_reports_its_island_time() {
    use std::io::Write;
    let widest = diagonal(0, MAX_TRACKS);
    assert!(widest.start.x_nm.abs().max(widest.end.y_nm.abs()) <= crate::schematic::MAX_CENTER_NM);

    let payload = MAX_TRACKS * std::mem::size_of::<Track>() + MAX_VIAS * std::mem::size_of::<Via>();
    std::io::stderr()
        .write_all(
            format!(
                "copper vector payload: {payload} bytes per full board, {} bytes across {} Undo states (excluding capacity and other board data)\n",
                payload * crate::MAX_HISTORY,
                crate::MAX_HISTORY
            )
            .as_bytes(),
        )
        .unwrap();
    for n in [1024, 2048, 4096] {
        let tracks: Vec<_> = (0..n).map(|k| diagonal(k, n)).collect();
        let started = std::time::Instant::now();
        let (_, members) = islands(&[], &tracks, &[]);
        let elapsed = started.elapsed();
        assert_eq!(members.len(), n, "no two tracks touch");

        let builder = IslandBuilder::new(&[], &tracks, &[]);
        let index = builder.index.as_ref().unwrap();
        let reached: usize = (0..n).map(|i| index.candidates(builder.bounds[i], 0.).into_iter().filter(|&j| j < i).count()).sum();
        assert_eq!(reached, n * (n - 1) / 2, "every pair reaches the contact test");
        let line = format!("overlapping copper islands: {n} tracks, {reached} pairs, {} ms\n", elapsed.as_millis());
        std::io::stderr().write_all(line.as_bytes()).unwrap();
    }
}
