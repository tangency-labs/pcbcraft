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
}
