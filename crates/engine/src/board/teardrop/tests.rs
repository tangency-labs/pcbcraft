use super::*;
use crate::{ADD_TRACKS, ADD_VIA, Session, persistence::encode_documents};
use serde_json::json;

const PARAMS: Parameters = Parameters { length: Length::Percent(20), max_length_nm: None, width_pct: 80, max_width_nm: None };

/// Name fixture points in board nanometres.
fn point(x_nm: i64, y_nm: i64) -> Point {
    Point { x_nm, y_nm }
}

/// Place an original via and one route segment through the production commands.
fn fixture(center: Point, start: Point, end: Point, width_nm: i64) -> (Session, ViaTeardrop) {
    use super::super::track::tests::{copper_layers, routed_fixture, view};
    let mut session = routed_fixture();
    let layer = copper_layers(&session)[0];
    session.apply(ADD_VIA, &json!({"at":center,"via_diameter_nm":500_000,"via_drill_nm":300_000,"expected_revision":session.revision()})).unwrap();
    session.apply(ADD_TRACKS, &json!({"layer":layer,"width_nm":width_nm,"points":[start,end],"expected_revision":session.revision()})).unwrap();
    let board = view(&session);
    let intent =
        ViaTeardrop { id: Uuid::from_u128(1), board: board.target, via: board.vias()[0].id, track: board.tracks()[0].id, parameters: PARAMS };
    (session, intent)
}

/// A 3-4-5 radius normal gives exact integer tangent points. Derivation is read-only.
#[test]
fn via_outline_matches_analytic_tangents() {
    use super::super::track::tests::view;
    let center = point(-1_000_000, 20_000_000);
    let outer = point(center.x_nm + 350_000, center.y_nm);
    let (session, intent) = fixture(center, outer, center, 100_000);
    let board = view(&session);
    let revision = session.revision();
    let before = encode_documents(session.snapshot().unwrap()).unwrap();
    // R=250000. T=(150000,200000) lies on the circle by 3²+4²=5².
    // B−T=(200000,−150000) is perpendicular to T; mirror for the other side.
    let expected =
        [(150_000, -200_000), (350_000, -50_000), (350_000, 50_000), (150_000, 200_000)].map(|(x, y)| point(center.x_nm + x, center.y_nm + y));
    let copper = intent.derive(board).unwrap();
    assert_eq!(copper, StraightCopper { board: board.target, source: intent.id, layer: board.tracks()[0].layer, vertices: expected });
    let capped_absolute = Parameters { length: Length::Nanometres(150_000), max_length_nm: Some(100_000), max_width_nm: Some(400_000), ..PARAMS };
    assert_eq!(ViaTeardrop { parameters: capped_absolute, ..intent }.derive(board), Ok(copper));
    let width_too_small = Parameters { max_width_nm: Some(399_999), ..capped_absolute };
    assert_eq!(ViaTeardrop { parameters: width_too_small, ..intent }.derive(board), Err(Error::WidthLimit));
    let too_long = Parameters { length: Length::Nanometres(100_001), ..PARAMS };
    assert_eq!(ViaTeardrop { parameters: too_long, ..intent }.derive(board), Err(Error::TrackTooShort));
    assert_eq!(session.revision(), revision);
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
}

/// A diagonal rotates the same quad, matching independently computed board coordinates.
#[test]
fn outline_follows_track_direction() {
    use super::super::track::tests::view;
    let center = point(0, 20_000_000);
    let (session, intent) = fixture(center, center, point(420_000, 20_560_000), 100_000);
    // For +X=(3/5,4/5) and +Y=(-4/5,3/5), rotate the exact local oracle.
    assert_eq!(
        intent.derive(view(&session)).unwrap().vertices,
        [point(250_000, 20_000_000), point(250_000, 20_250_000), point(170_000, 20_310_000), point(-70_000, 20_240_000)],
    );
}

/// Odd track widths keep half-nanometre precision until board-coordinate projection.
#[test]
fn projection_rounds_half_nanometres_away_from_zero() {
    use super::super::track::tests::view;
    let center = point(0, 20_000_000);
    let (session, intent) = fixture(center, center, point(0, 21_000_000), 100_001);
    let intent = ViaTeardrop { parameters: Parameters { width_pct: 100, ..PARAMS }, ..intent };
    let copper = intent.derive(view(&session)).unwrap();
    assert_eq!(copper.vertices[1], point(50_001, 20_350_000));
    assert_eq!(copper.vertices[2], point(-50_001, 20_350_000));
}

/// Sizing accepts both amount kinds, rejects zero, and enforces absolute dimension bounds.
#[test]
fn parameter_limits() {
    let valid =
        Parameters { length: Length::Nanometres(MAX_CENTER_NM), max_length_nm: Some(MAX_CENTER_NM), width_pct: u16::MAX, max_width_nm: Some(1) };
    assert_eq!(valid.validate(), Ok(()));
    assert_eq!(Parameters { length: Length::Percent(u16::MAX), ..valid }.validate(), Ok(()));
    for invalid in [
        Parameters { length: Length::Percent(0), ..valid },
        Parameters { length: Length::Nanometres(0), ..valid },
        Parameters { length: Length::Nanometres(MAX_CENTER_NM + 1), ..valid },
        Parameters { width_pct: 0, ..valid },
        Parameters { max_length_nm: Some(0), ..valid },
        Parameters { max_length_nm: Some(MAX_CENTER_NM + 1), ..valid },
        Parameters { max_width_nm: Some(0), ..valid },
        Parameters { max_width_nm: Some(MAX_CENTER_NM + 1), ..valid },
    ] {
        assert_eq!(invalid.validate(), Err(Error::InvalidSize), "{invalid:?}");
    }
}

/// A long percentage request fits this track only when an explicit length cap shortens it.
#[test]
fn long_percent_extension_requires_an_explicit_cap() {
    let center = point(-500_000_000, 0);
    let outer = point(500_000_000, 0);
    let diameter_nm = 10_000_000;
    let parameters = Parameters { length: Length::Percent(u16::MAX), width_pct: 100, ..PARAMS };
    let outline = |parameters: Parameters| {
        let (extension, width_cap) = parameters.for_via(diameter_nm);
        tangent_quad(center, outer, diameter_nm, 100_000, extension, width_cap)
    };
    // The old implicit cap made the reach 992,300,000 nm, short enough for this track.
    assert_eq!(outline(parameters), Err(Error::TrackTooShort));
    let capped = outline(Parameters { max_length_nm: Some(MAX_CENTER_NM), ..parameters }).unwrap();
    assert_eq!(capped[1].x_nm, center.x_nm + diameter_nm / 2 + MAX_CENTER_NM);
}

/// Do not resolve a foreign board or a missing, nil or conflicting source identity.
#[test]
fn identity_and_attachment_are_checked() {
    use super::super::track::tests::view;
    let center = point(0, 20_000_000);
    let (session, intent) = fixture(center, center, point(1_000_000, 20_000_000), 100_000);
    let board = view(&session);
    let missing = Uuid::from_u128(2);
    for (candidate, error) in [
        (ViaTeardrop { id: intent.via, ..intent }, Error::InvalidIdentity),
        (ViaTeardrop { track: Uuid::nil(), ..intent }, Error::InvalidIdentity),
        (ViaTeardrop { board: BoardTarget { project: missing, ..board.target }, ..intent }, Error::WrongBoard),
        (ViaTeardrop { track: missing, ..intent }, Error::MissingTrack),
        (ViaTeardrop { via: missing, ..intent }, Error::MissingVia),
    ] {
        assert_eq!(candidate.derive(board), Err(error));
    }
}

/// A matching intent and view label cannot redirect the backing board's copper.
#[test]
fn retagged_view_cannot_redirect_copper() {
    use super::super::track::tests::view;
    let center = point(0, 20_000_000);
    let (session, intent) = fixture(center, center, point(1_000_000, 20_000_000), 100_000);
    let board = view(&session);
    assert_eq!(intent.derive(board).unwrap().board, board.target);
    let foreign_project = Uuid::from_u128(board.target.project.as_u128() ^ 1);
    let foreign_board = Uuid::from_u128(board.target.board.as_u128() ^ 1);
    for target in [BoardTarget { project: foreign_project, ..board.target }, BoardTarget { board: foreign_board, ..board.target }] {
        let mut retagged = board;
        retagged.target = target;
        assert_eq!(ViaTeardrop { board: target, ..intent }.derive(retagged), Err(Error::WrongBoard));
    }
}

/// Off-center entries, non-tapers, and rounded vertices outside the board are refused.
#[test]
fn unsupported_or_unrepresentable_shapes_are_refused() {
    use super::super::track::tests::view;
    for (center, start, width, error) in [
        (point(0, 20_000_000), point(1, 20_000_000), 100_000, Error::NotCentered),
        (point(0, 20_000_000), point(0, 20_000_000), 500_000, Error::NoTaper),
        (point(0, MAX_CENTER_NM), point(0, MAX_CENTER_NM), 100_000, Error::UnrepresentableContour),
    ] {
        let (session, intent) = fixture(center, start, point(1_000_000, center.y_nm), width);
        assert_eq!(intent.derive(view(&session)), Err(error));
    }
}

mod regeneration {
    use super::*;
    use crate::{
        ADD_DRAWING, CommandError, DELETE_TRACKS, MOVE_TRACKS, SET_RULES,
        board::{rules::Values, setup::LayerPurpose, track::tests::view},
    };
    use serde_json::Value;

    /// Create a closed outline and a checked, authored teardrop through the shared commands.
    fn checked_site() -> (Session, ViaTeardrop) {
        let center = point(0, 20_000_000);
        let (mut session, intent) = fixture(center, center, point(600_000, center.y_nm), 100_000);
        let layer = view(&session).layers().into_iter().find(|layer| layer.purpose == LayerPurpose::Outline).unwrap().id;
        let corners = [point(-5_000_000, -5_000_000), point(25_000_000, -5_000_000), point(25_000_000, 45_000_000), point(-5_000_000, 45_000_000)];
        let board = intent.board.board;
        session
            .apply(ADD_DRAWING, &json!({"board":board,"layer":layer,"points":corners,"closed":true,"expected_revision":session.revision()}))
            .unwrap();
        let rules = Values { clearance_nm: 35_000, ..Values::default() };
        session.apply(SET_RULES, &json!({"board":board,"rules":rules,"expected_revision":session.revision()})).unwrap();
        let report = session
            .execute(
                EDIT_TEARDROPS,
                &json!({"board":intent.board.board,"expected_revision":session.revision(),"edits":{"add":[Record::from(intent)],"remove":[]}}),
            )
            .unwrap();
        assert_eq!(report["teardrops"]["added"], json!([Record::from(intent)]));
        (session, intent)
    }

    /// Moving both source objects regenerates the polygon under the same ID and history entry.
    #[test]
    fn moved_via_teardrop_follows_and_undo_restores_both() {
        let (mut session, intent) = checked_site();
        let before = encode_documents(session.snapshot().unwrap()).unwrap();
        let original = intent.derive(view(&session)).unwrap();
        let original_via = view(&session).vias()[0].at;
        let revision = session.revision();
        let (dx, dy) = (400_000, 200_000);
        session.apply(MOVE_TRACKS, &json!({"tracks":[intent.track],"vias":[intent.via],"dx_nm":dx,"dy_nm":dy,"expected_revision":revision})).unwrap();
        assert_eq!(session.revision(), revision + 1);
        assert_eq!(view(&session).vias()[0].at, point(original_via.x_nm + dx, original_via.y_nm + dy));
        assert_eq!(view(&session).teardrops(), &[Record::from(intent)]);
        let moved = intent.derive(view(&session)).unwrap();
        assert_eq!(moved.vertices, original.vertices.map(|p| point(p.x_nm + dx, p.y_nm + dy)));
        session.apply("edit.undo", &Value::Null).unwrap();
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
        assert_eq!(intent.derive(view(&session)), Ok(original));
        session.apply("edit.redo", &Value::Null).unwrap();
        assert_eq!(intent.derive(view(&session)), Ok(moved));
    }

    /// A lone via move or deletion of either source drops the now-detached intent; Undo restores it.
    #[test]
    fn detached_or_deleted_via_teardrop_is_removed_atomically() {
        let (mut session, intent) = checked_site();
        let before = encode_documents(session.snapshot().unwrap()).unwrap();
        let original = intent.derive(view(&session)).unwrap();
        for (id, mut params, why) in [
            (MOVE_TRACKS, json!({"vias":[intent.via],"dx_nm":0,"dy_nm":1}), Error::NotCentered),
            (DELETE_TRACKS, json!({"vias":[intent.via]}), Error::MissingVia),
            (DELETE_TRACKS, json!({"tracks":[intent.track]}), Error::MissingTrack),
        ] {
            params["expected_revision"] = json!(session.revision());
            session.apply(id, &params).unwrap();
            assert!(view(&session).teardrops().is_empty());
            assert_eq!(intent.derive(view(&session)), Err(why));
            session.apply("edit.undo", &Value::Null).unwrap();
            assert_eq!(view(&session).teardrops(), &[Record::from(intent)]);
            assert_eq!(intent.derive(view(&session)), Ok(original));
            assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
        }
    }

    /// A new obstacle can hit the taper's clearance without hitting the underlying via or
    /// track. The rule block leaves both the authored snapshot and its history untouched.
    #[test]
    fn candidate_clearance_block_does_not_publish_or_take_undo() {
        let (mut session, intent) = checked_site();
        let layer = intent.derive(view(&session)).unwrap().layer;
        let add_obstacle = |session: &mut Session, y_nm| {
            session.apply(
                ADD_TRACKS,
                &json!({"layer":layer,"width_nm":100_000,
                "points":[point(260_000,y_nm),point(260_000,y_nm - 50_000)],"expected_revision":session.revision()}),
            )
        };
        add_obstacle(&mut session, 19_770_000).unwrap();
        let obstacle = view(&session).tracks().iter().find(|track| track.start == point(260_000, 19_770_000)).unwrap().id;
        let before = session.snapshot().unwrap();
        let revision = session.revision();
        let params = json!({"tracks":[obstacle],"dx_nm":0,"dy_nm":10_000,"expected_revision":revision});
        let mut job = session.prepare_copper_edit(MOVE_TRACKS, &params).unwrap();
        for _ in 0..128 {
            if job.step(64) {
                break;
            }
        }
        assert!(matches!(job.take_result(), Err(CommandError::InvalidParams(message)) if message.contains("CopperClearance")
            && message.contains(&intent.id.to_string()) && message.contains(&obstacle.to_string())));
        assert!(std::sync::Arc::ptr_eq(&before, &session.snapshot().unwrap()));
        assert_eq!(session.revision(), revision);
        session.apply("edit.undo", &Value::Null).unwrap();
        assert_eq!(view(&session).tracks().len(), 1);
        assert_eq!(view(&session).teardrops(), &[Record::from(intent)]);
    }

    /// Run the live-style prepare/commit split on a board with over 1,000 tracks. Bound
    /// worker steps and ensure neither stage falls back to the synchronous geometry builder.
    #[test]
    fn large_board_prepares_teardrops_without_synchronous_geometry() {
        let (mut session, intent) = checked_site();
        let original = intent.derive(view(&session)).unwrap();
        for row in 0..5 {
            let points: Vec<_> = (0..=240).map(|x| point(x * 500_000, -20_000_000 - row * 1_000_000)).collect();
            session
                .apply(
                    ADD_TRACKS,
                    &json!({"layer":original.layer,"width_nm":100_000,
                "points":points,"expected_revision":session.revision()}),
                )
                .unwrap();
        }
        assert_eq!(view(&session).tracks().len(), 1_201);
        let before = session.snapshot().unwrap();
        let revision = session.revision();
        let params = json!({"vias":[intent.via],"tracks":[intent.track],"dx_nm":400_000,"dy_nm":0,"expected_revision":revision});
        use crate::board::clearance::FORBID_SYNCHRONOUS_GEOMETRY;
        FORBID_SYNCHRONOUS_GEOMETRY.set(true);
        let mut job = session.prepare_copper_edit(MOVE_TRACKS, &params).unwrap();
        assert!(!job.step(1));
        assert!(job.take_result().unwrap().is_none());
        for _ in 0..25_000 {
            if job.step(128) {
                break;
            }
        }
        let prepared = job.take_result().unwrap().unwrap();
        assert!(std::sync::Arc::ptr_eq(&before, &session.snapshot().unwrap()));
        let committed = session.commit_copper_edit(prepared);
        FORBID_SYNCHRONOUS_GEOMETRY.set(false);
        committed.unwrap();
        assert_eq!(session.revision(), revision + 1);
        assert_eq!(view(&session).teardrops(), &[Record::from(intent)]);
        assert_eq!(intent.derive(view(&session)).unwrap().vertices, original.vertices.map(|p| point(p.x_nm + 400_000, p.y_nm)));
    }

    /// On an active-intent board, source-contact defaults fail closed but explicitly sized
    /// track and via additions still run regeneration. Outside that state, defaults still work.
    #[test]
    fn additions_require_explicit_dimensions_with_active_teardrops() {
        let (mut session, intent) = checked_site();
        let layer = intent.derive(view(&session)).unwrap().layer;
        let revision = session.revision();
        let before = session.snapshot().unwrap();
        for (id, params) in [
            (ADD_TRACKS, json!({"layer":layer,"points":[point(3_000_000,20_000_000),point(4_000_000,20_000_000)]})),
            (ADD_VIA, json!({"at":point(5_000_000,20_000_000)})),
        ] {
            let mut params = params;
            params["expected_revision"] = json!(session.revision());
            let snapshot = session.snapshot().unwrap();
            assert!(matches!(session.apply(id, &params), Err(CommandError::Unavailable(message))
                if message == crate::board::teardrop::EXPLICIT_DIMENSIONS));
            assert!(std::sync::Arc::ptr_eq(&snapshot, &session.snapshot().unwrap()));
            let mut explicit = params.clone();
            if id == ADD_TRACKS {
                explicit["width_nm"] = json!(100_000);
            } else {
                explicit["via_diameter_nm"] = json!(500_000);
                explicit["via_drill_nm"] = json!(300_000);
            }
            session.apply(id, &explicit).unwrap();
            assert_eq!(view(&session).teardrops(), &[Record::from(intent)]);
        }
        assert_eq!(session.revision(), revision + 2);
        session.open_snapshot(before);
        session
            .apply(
                EDIT_TEARDROPS,
                &json!({"board":intent.board.board,"expected_revision":session.revision(),"edits":{"add":[],"remove":[intent.id]}}),
            )
            .unwrap();
        session.apply(ADD_VIA, &json!({"at":point(5_000_000,20_000_000),"expected_revision":session.revision()})).unwrap();
    }

    /// Regenerating many saved teardrops honors small step budgets across both derivation and
    /// clearance checks; dropping the job mid-pass leaves project, revision and Undo unchanged.
    #[test]
    fn regeneration_yields_in_bounded_batches_and_cancel_leaves_session_unchanged() {
        use crate::board::track::{
            Track, Via,
            tests::{copper_layers, routed_fixture},
        };
        use crate::persistence::decode_documents;
        use std::sync::Arc;

        const COUNT: usize = 300;
        let mut session = routed_fixture();
        let board = view(&session).target.board;
        let copper_layer = copper_layers(&session)[0];
        let outline_layer = view(&session).layers().into_iter().find(|layer| layer.purpose == LayerPurpose::Outline).unwrap().id;
        let corners = [point(-5_000_000, -5_000_000), point(45_000_000, -5_000_000), point(45_000_000, 55_000_000), point(-5_000_000, 55_000_000)];
        session
            .apply(ADD_DRAWING, &json!({"board":board,"layer":outline_layer,"points":corners,"closed":true,"expected_revision":session.revision()}))
            .unwrap();
        let rules = Values { clearance_nm: 35_000, ..Values::default() };
        session.apply(SET_RULES, &json!({"board":board,"rules":rules,"expected_revision":session.revision()})).unwrap();

        let mut project = session.project().unwrap().clone();
        let mut first_record = None;
        {
            let board_data = Arc::make_mut(&mut project.board);
            for index in 0..COUNT {
                let col = (index % 20) as i64;
                let row = (index / 20) as i64;
                let at = point(col * 2_000_000, 20_000_000 + row * 2_000_000);
                let via = Uuid::from_u128(0x10_0000 + index as u128);
                let track = Uuid::from_u128(0x20_0000 + index as u128);
                let id = Uuid::from_u128(0x30_0000 + index as u128);
                board_data.vias.push(Via { id: via, net_assignment: None, locked: false, at, diameter_nm: 500_000, drill_nm: 300_000 });
                board_data.tracks.push(Track {
                    id: track,
                    locked: false,
                    layer: copper_layer,
                    width_nm: 100_000,
                    start: at,
                    end: point(at.x_nm + 600_000, at.y_nm),
                });
                let record = Record { id, via, track, parameters: PARAMS };
                if first_record.is_none() {
                    first_record = Some(record);
                }
                board_data.teardrops.push(record);
            }
            board_data.vias.sort_by_key(|via| via.id);
            board_data.tracks.sort_by_key(|track| track.id);
            board_data.teardrops.sort_by_key(|record| record.id);
        }
        let saved = encode_documents(Arc::new(project)).unwrap();
        session.open_snapshot(decode_documents(&saved).unwrap());
        session.apply("project.rename", &json!({"name":"Teardrop board"})).unwrap();

        let first = first_record.unwrap().on(view(&session).target);
        let original = first.derive(view(&session)).unwrap();
        let before = session.snapshot().unwrap();
        let before_bytes = encode_documents(before.clone()).unwrap();
        let revision = session.revision();
        let history = (session.undo.len(), session.redo.len());
        assert_eq!(view(&session).teardrops().len(), COUNT);
        assert!(history.0 > 0);

        let (dx, dy) = (200_000, 200_000);
        let params = json!({"tracks":[first.track],"vias":[first.via],"dx_nm":dx,"dy_nm":dy,"expected_revision":revision});

        // Advance into teardrop regeneration, verify step(1) yields during both derivation
        // and clearance checking, then drop mid-pass to simulate worker cancellation.
        let mut canceled = session.prepare_copper_edit(MOVE_TRACKS, &params).unwrap();
        for _ in 0..50_000 {
            if canceled.progress().stage == "Regenerating teardrops" {
                break;
            }
            assert!(!canceled.step(256));
        }
        assert_eq!(canceled.progress().stage, "Regenerating teardrops");
        assert!(!canceled.step(0));
        let mut last_completed = canceled.progress().completed;
        for _ in 0..8 {
            assert!(!canceled.step(1));
            let progress = canceled.progress();
            assert_eq!(progress.stage, "Regenerating teardrops");
            assert!(progress.completed > last_completed);
            assert!(progress.completed < COUNT);
            last_completed = progress.completed;
        }
        assert!(!canceled.step(COUNT));
        let entered_check = canceled.progress().completed;
        assert!(entered_check >= COUNT);
        assert!(!canceled.step(1));
        let mid_check = canceled.progress();
        assert_eq!(mid_check.stage, "Regenerating teardrops");
        assert!(mid_check.completed > entered_check && mid_check.completed < mid_check.total);
        assert!(canceled.take_result().unwrap().is_none());
        drop(canceled);

        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before_bytes);
        assert_eq!(session.revision(), revision);
        assert_eq!((session.undo.len(), session.redo.len()), history);
        assert_eq!(view(&session).teardrops().len(), COUNT);

        // Completing a fresh job in small batches yields throughout regeneration and publishes
        // atomically only when committed, preserving prior Undo history.
        let mut job = session.prepare_copper_edit(MOVE_TRACKS, &params).unwrap();
        let mut regeneration_yields = 0usize;
        let mut last_regen_completed = 0usize;
        let mut finished = false;
        for _ in 0..50_000 {
            let budget = if job.progress().stage == "Regenerating teardrops" { 16 } else { 256 };
            if job.step(budget) {
                finished = true;
                break;
            }
            let progress = job.progress();
            if progress.stage == "Regenerating teardrops" {
                regeneration_yields += 1;
                assert!(progress.completed >= last_regen_completed);
                last_regen_completed = progress.completed;
            }
        }
        assert!(finished);
        assert!(regeneration_yields >= COUNT / 16 * 2);
        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
        assert_eq!(session.revision(), revision);
        let prepared = job.take_result().unwrap().unwrap();
        session.commit_copper_edit(prepared).unwrap();
        assert_eq!(session.revision(), revision + 1);
        assert_eq!(view(&session).teardrops().len(), COUNT);
        assert_eq!(first.derive(view(&session)).unwrap().vertices, original.vertices.map(|p| point(p.x_nm + dx, p.y_nm + dy)));
        session.apply("edit.undo", &Value::Null).unwrap();
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before_bytes);
        assert_eq!(session.undo.len(), history.0);
    }

    /// At the 8,192-teardrop cap, neither the final regeneration transition nor the final
    /// completion step re-derives teardrops or performs an unbudgeted whole-board encode count;
    /// mid-pass cancellation leaves project, revision and Undo unchanged.
    #[test]
    fn regeneration_final_step_is_bounded_at_max_intents_and_cancel_leaves_session_unchanged() {
        use crate::board::track::{
            Track, Via,
            tests::{copper_layers, routed_fixture},
        };
        use crate::persistence::{decode_documents, take_board_record_writes};
        use std::sync::Arc;

        const COUNT: usize = edit::MAX_INTENTS;
        let mut session = routed_fixture();
        let board = view(&session).target.board;
        let copper_layer = copper_layers(&session)[0];
        let outline_layer = view(&session).layers().into_iter().find(|layer| layer.purpose == LayerPurpose::Outline).unwrap().id;
        let corners =
            [point(-5_000_000, -5_000_000), point(265_000_000, -5_000_000), point(265_000_000, 155_000_000), point(-5_000_000, 155_000_000)];
        session
            .apply(ADD_DRAWING, &json!({"board":board,"layer":outline_layer,"points":corners,"closed":true,"expected_revision":session.revision()}))
            .unwrap();
        let rules = Values { clearance_nm: 35_000, ..Values::default() };
        session.apply(SET_RULES, &json!({"board":board,"rules":rules,"expected_revision":session.revision()})).unwrap();

        let mut project = session.project().unwrap().clone();
        let mut first_record = None;
        {
            let board_data = Arc::make_mut(&mut project.board);
            for index in 0..COUNT {
                let col = (index % 128) as i64;
                let row = (index / 128) as i64;
                let at = point(col * 2_000_000, 20_000_000 + row * 2_000_000);
                let via = Uuid::from_u128(0x10_0000 + index as u128);
                let track = Uuid::from_u128(0x20_0000 + index as u128);
                let id = Uuid::from_u128(0x30_0000 + index as u128);
                board_data.vias.push(Via { id: via, net_assignment: None, locked: false, at, diameter_nm: 500_000, drill_nm: 300_000 });
                board_data.tracks.push(Track {
                    id: track,
                    locked: false,
                    layer: copper_layer,
                    width_nm: 100_000,
                    start: at,
                    end: point(at.x_nm + 600_000, at.y_nm),
                });
                let record = Record { id, via, track, parameters: PARAMS };
                if first_record.is_none() {
                    first_record = Some(record);
                }
                board_data.teardrops.push(record);
            }
            board_data.vias.sort_by_key(|via| via.id);
            board_data.tracks.sort_by_key(|track| track.id);
            board_data.teardrops.sort_by_key(|record| record.id);
        }
        let saved = encode_documents(Arc::new(project)).unwrap();
        session.open_snapshot(decode_documents(&saved).unwrap());
        session.apply("project.rename", &json!({"name":"Max teardrop board"})).unwrap();

        let first = first_record.unwrap().on(view(&session).target);
        let original = first.derive(view(&session)).unwrap();
        let before = session.snapshot().unwrap();
        let before_bytes = encode_documents(before.clone()).unwrap();
        let revision = session.revision();
        let history = (session.undo.len(), session.redo.len());
        assert_eq!(view(&session).teardrops().len(), 8_192);
        assert!(history.0 > 0);

        let (dx, dy) = (200_000, 200_000);
        let params = json!({"tracks":[first.track],"vias":[first.via],"dx_nm":dx,"dy_nm":dy,"expected_revision":revision});

        // Cancel mid-pass during small-step regeneration and confirm nothing changed.
        let mut canceled = session.prepare_copper_edit(MOVE_TRACKS, &params).unwrap();
        for _ in 0..50_000 {
            if canceled.progress().stage == "Regenerating teardrops" {
                break;
            }
            assert!(!canceled.step(256));
        }
        assert_eq!(canceled.progress().stage, "Regenerating teardrops");
        let _ = (take_derive_calls(), take_board_record_writes());
        assert!(!canceled.step(1));
        assert_eq!(take_derive_calls(), 1);
        assert_eq!(take_board_record_writes(), 0);
        assert!(canceled.take_result().unwrap().is_none());
        drop(canceled);

        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before_bytes);
        assert_eq!(session.revision(), revision);
        assert_eq!((session.undo.len(), session.redo.len()), history);

        // Drain a fresh job at 8,192 teardrops and count work on every regeneration and
        // board-size-check step, using step(1) across the regeneration transition and final step.
        let mut job = session.prepare_copper_edit(MOVE_TRACKS, &params).unwrap();
        for _ in 0..50_000 {
            if job.progress().stage == "Regenerating teardrops" {
                break;
            }
            assert!(!job.step(256));
        }
        assert_eq!(job.progress().stage, "Regenerating teardrops");

        let mut transitioned_on_step_one = false;
        for _ in 0..50_000 {
            let progress = job.progress();
            if progress.stage != "Regenerating teardrops" {
                break;
            }
            let budget = if progress.total.saturating_sub(progress.completed) <= 4 { 1 } else { 256 };
            let _ = (take_derive_calls(), take_board_record_writes());
            let done = job.step(budget);
            let derives = take_derive_calls();
            let records = take_board_record_writes();
            assert!(!done);
            assert!(derives <= budget);
            assert_eq!(records, 0);
            if job.progress().stage == "Checking board size" {
                assert_eq!(budget, 1);
                assert_eq!(derives, 0);
                transitioned_on_step_one = true;
                break;
            }
        }
        assert!(transitioned_on_step_one);
        assert_eq!(job.progress().stage, "Checking board size");

        // First header unit (validated opening, zero teardrop re-derivation).
        let _ = (take_derive_calls(), take_board_record_writes());
        assert!(!job.step(1));
        assert_eq!(take_derive_calls(), 0);
        assert!(take_board_record_writes() <= 1);

        let mut size_check_yields = 1usize;
        let mut finished = false;
        let mut final_step_budget = 0usize;
        let mut final_step_derives = usize::MAX;
        let mut final_step_records = usize::MAX;
        for _ in 0..50_000 {
            let progress = job.progress();
            // Batches of at most 256 stop exactly 8 units short of the total, so the final
            // steps always run at budget 1 whatever the header and footprint unit counts are.
            let remaining = progress.total.saturating_sub(progress.completed);
            let budget = if remaining <= 8 { 1 } else { (remaining - 8).min(256) };
            let _ = (take_derive_calls(), take_board_record_writes());
            let done = job.step(budget);
            let derives = take_derive_calls();
            let records = take_board_record_writes();
            assert_eq!(derives, 0);
            assert!(records <= budget);
            if done {
                finished = true;
                final_step_budget = budget;
                final_step_derives = derives;
                final_step_records = records;
                break;
            }
            size_check_yields += 1;
        }
        assert!(finished);
        assert_eq!(final_step_budget, 1);
        assert_eq!(final_step_derives, 0);
        assert!(final_step_records <= 1);
        assert!(size_check_yields >= 2);
        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
        assert_eq!(session.revision(), revision);

        let prepared = job.take_result().unwrap().unwrap();
        session.commit_copper_edit(prepared).unwrap();
        assert_eq!(session.revision(), revision + 1);
        assert_eq!(view(&session).teardrops().len(), COUNT);
        assert_eq!(first.derive(view(&session)).unwrap().vertices, original.vertices.map(|p| point(p.x_nm + dx, p.y_nm + dy)));
        session.apply("edit.undo", &Value::Null).unwrap();
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before_bytes);
        assert_eq!(session.undo.len(), history.0);
    }

    /// A board header with hundreds of graphics and zones is size-checked in bounded units: every
    /// step serializes at most its budget of header records, dropping a job mid-header leaves
    /// project, revision and Undo unchanged, and budget-1 stepping, one large step and a check
    /// resumed after stopping mid-header all count exactly the saved board document's bytes.
    #[test]
    fn board_size_check_header_is_bounded_resumable_and_byte_exact() {
        use crate::board::track::{
            Track, Via,
            tests::{copper_layers, routed_fixture},
        };
        use crate::persistence::{BoardSizeCheck, ENTRY_POINT, decode_documents, project_board_path, take_board_record_writes};
        use std::sync::Arc;

        const COUNT: usize = 64;
        const DRAWINGS: usize = 400;
        const ZONES: usize = 200;
        let mut session = routed_fixture();
        let board = view(&session).target.board;
        let copper_layer = copper_layers(&session)[0];
        let outline_layer = view(&session).layers().into_iter().find(|layer| layer.purpose == LayerPurpose::Outline).unwrap().id;
        let silkscreen_layer = view(&session).layers().into_iter().find(|layer| layer.purpose == LayerPurpose::Silkscreen).unwrap().id;
        let corners =
            [point(-5_000_000, -5_000_000), point(265_000_000, -5_000_000), point(265_000_000, 155_000_000), point(-5_000_000, 155_000_000)];
        session
            .apply(ADD_DRAWING, &json!({"board":board,"layer":outline_layer,"points":corners,"closed":true,"expected_revision":session.revision()}))
            .unwrap();
        let rules = Values { clearance_nm: 35_000, ..Values::default() };
        session.apply(SET_RULES, &json!({"board":board,"rules":rules,"expected_revision":session.revision()})).unwrap();
        // One authored zone and one stroked graphic, far from the copper, as templates.
        let square =
            [point(200_000_000, 100_000_000), point(210_000_000, 100_000_000), point(210_000_000, 110_000_000), point(200_000_000, 110_000_000)];
        let nets = session.project().unwrap().inspect_net_registry().unwrap();
        session
            .apply(
                crate::board::zones::ADD_ZONE,
                &json!({"board":board,"layer":copper_layer,"target":{"kind":"derived","anchor":nets["nets"][0]["anchor"]},
                    "points":square,"expected_revision":session.revision()}),
            )
            .unwrap();
        session
            .apply(
                ADD_DRAWING,
                &json!({"board":board,"layer":silkscreen_layer,"points":square,"closed":true,"stroke_nm":150_000,"expected_revision":session.revision()}),
            )
            .unwrap();

        let mut project = session.project().unwrap().clone();
        let mut first_record = None;
        {
            let board_data = Arc::make_mut(&mut project.board);
            for index in 0..COUNT {
                let at = point(index as i64 * 2_000_000, 20_000_000);
                let via = Uuid::from_u128(0x10_0000 + index as u128);
                let track = Uuid::from_u128(0x20_0000 + index as u128);
                let id = Uuid::from_u128(0x30_0000 + index as u128);
                board_data.vias.push(Via { id: via, net_assignment: None, locked: false, at, diameter_nm: 500_000, drill_nm: 300_000 });
                board_data.tracks.push(Track {
                    id: track,
                    locked: false,
                    layer: copper_layer,
                    width_nm: 100_000,
                    start: at,
                    end: point(at.x_nm + 600_000, at.y_nm),
                });
                let record = Record { id, via, track, parameters: PARAMS };
                if first_record.is_none() {
                    first_record = Some(record);
                }
                board_data.teardrops.push(record);
            }
            board_data.vias.sort_by_key(|via| via.id);
            board_data.tracks.sort_by_key(|track| track.id);
            board_data.teardrops.sort_by_key(|record| record.id);
            let zone = board_data.zones[0].clone();
            board_data.zones = (0..ZONES)
                .map(|index| {
                    let mut copy = zone.clone();
                    copy.id = Uuid::from_u128(0x40_0000 + index as u128);
                    copy
                })
                .collect();
            let graphics = board_data.drawings.as_mut().unwrap();
            let stroked = graphics.items.iter().find(|item| item.stroke_nm.is_some()).unwrap().clone();
            graphics.items.extend((0..DRAWINGS).map(|index| {
                let mut copy = stroked.clone();
                copy.id = Uuid::from_u128(0x50_0000 + index as u128);
                copy
            }));
            graphics.items.sort_by_key(|item| item.id);
        }
        let saved = encode_documents(Arc::new(project)).unwrap();
        session.open_snapshot(decode_documents(&saved).unwrap());
        session.apply("project.rename", &json!({"name":"Header-heavy board"})).unwrap();
        assert_eq!(view(&session).zones().len(), ZONES);
        assert!(view(&session).drawings().len() >= DRAWINGS + 2);
        assert_eq!(view(&session).teardrops().len(), COUNT);

        let before = session.snapshot().unwrap();
        let header_units = {
            let setup = before.board.setup.as_ref().unwrap();
            let graphics = before.board.drawings.as_ref().unwrap();
            2 + setup.materials().len()
                + setup.layers().len()
                + setup.stackup().strata.len()
                + graphics.layers.len()
                + graphics.items.len()
                + before.board.zones.len()
                + before.board.arcs.len()
        };
        assert!(header_units > DRAWINGS + ZONES);
        let before_bytes = encode_documents(before.clone()).unwrap();
        let revision = session.revision();
        let history = (session.undo.len(), session.redo.len());
        assert!(history.0 > 0);
        let first = first_record.unwrap().on(view(&session).target);
        let params = json!({"tracks":[first.track],"vias":[first.via],"dx_nm":200_000,"dy_nm":200_000,"expected_revision":revision});

        // Stop a job partway through the header (inside graphics and zones) and drop it.
        let mut canceled = session.prepare_copper_edit(MOVE_TRACKS, &params).unwrap();
        for _ in 0..50_000 {
            if canceled.progress().stage == "Checking board size" {
                break;
            }
            assert!(!canceled.step(256));
        }
        assert_eq!(canceled.progress().stage, "Checking board size");
        assert_eq!(canceled.progress().completed, 0);
        for expected in 1..=DRAWINGS {
            let _ = (take_derive_calls(), take_board_record_writes());
            assert!(!canceled.step(1));
            assert_eq!(take_derive_calls(), 0);
            assert!(take_board_record_writes() <= 1);
            assert_eq!(canceled.progress().completed, expected);
        }
        assert!(canceled.progress().completed < header_units);
        assert!(canceled.take_result().unwrap().is_none());
        drop(canceled);

        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before_bytes);
        assert_eq!(session.revision(), revision);
        assert_eq!((session.undo.len(), session.redo.len()), history);

        // A fresh job drains the size check with mixed budgets: every unfinished step does exactly
        // its budget of units and serializes at most that many records, and progress reaches total.
        let mut job = session.prepare_copper_edit(MOVE_TRACKS, &params).unwrap();
        for _ in 0..50_000 {
            if job.progress().stage == "Checking board size" {
                break;
            }
            assert!(!job.step(256));
        }
        assert_eq!(job.progress().stage, "Checking board size");
        let mut last = 0usize;
        let mut finished = false;
        for budget in [1usize, 7, 64].into_iter().cycle().take(50_000) {
            let _ = (take_derive_calls(), take_board_record_writes());
            let done = job.step(budget);
            assert_eq!(take_derive_calls(), 0);
            assert!(take_board_record_writes() <= budget);
            if done {
                finished = true;
                break;
            }
            let progress = job.progress();
            assert_eq!(progress.stage, "Checking board size");
            assert_eq!(progress.completed, last + budget);
            assert!(progress.completed <= progress.total);
            last = progress.completed;
        }
        assert!(finished);
        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
        let prepared = job.take_result().unwrap().unwrap();
        session.commit_copper_edit(prepared).unwrap();
        assert_eq!(session.revision(), revision + 1);

        // The real serializer, one large step, budget-1 stepping and a check stopped mid-header
        // then resumed all count the same board document bytes.
        let committed = session.snapshot().unwrap();
        let documents = encode_documents(committed.clone()).unwrap();
        let saved_board = documents[&project_board_path(&documents[ENTRY_POINT]).unwrap().unwrap()].len();
        let mut whole = BoardSizeCheck::new(true);
        assert!(whole.step(&committed, usize::MAX).unwrap());
        assert_eq!(whole.byte_count(), saved_board);
        let (whole_completed, total) = whole.progress(&committed);
        assert_eq!(whole_completed, total);

        let mut single = BoardSizeCheck::new(true);
        let mut steps = 0usize;
        let _ = take_board_record_writes();
        while !single.step(&committed, 1).unwrap() {
            assert!(take_board_record_writes() <= 1);
            steps += 1;
            assert_eq!(single.progress(&committed), (steps, total));
        }
        assert!(take_board_record_writes() <= 1);
        assert!(steps >= header_units);
        assert_eq!(single.progress(&committed), (total, total));
        assert_eq!(single.byte_count(), saved_board);

        let mut resumed = BoardSizeCheck::new(true);
        for _ in 0..header_units / 2 {
            assert!(!resumed.step(&committed, 1).unwrap());
        }
        assert_eq!(resumed.progress(&committed), (header_units / 2, total));
        assert!(resumed.step(&committed, usize::MAX).unwrap());
        assert_eq!(resumed.progress(&committed), (total, total));
        assert_eq!(resumed.byte_count(), saved_board);
    }
}
