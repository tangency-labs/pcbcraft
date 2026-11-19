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
