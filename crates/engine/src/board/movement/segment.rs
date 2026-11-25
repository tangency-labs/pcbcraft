use super::{Point, Uuid};
use crate::board::track::Track;

/// Where `p` projects onto `track`, as a fraction from its start (0) to its end (1), and how far
/// it lies from that point in nanometers.
pub fn track_projection(p: Point, track: &Track) -> (f64, f64) {
    let (ax, ay, bx, by) = (track.start.x_nm as f64, track.start.y_nm as f64, track.end.x_nm as f64, track.end.y_nm as f64);
    let (dx, dy, px, py) = (bx - ax, by - ay, p.x_nm as f64 - ax, p.y_nm as f64 - ay);
    let length = dx * dx + dy * dy;
    let t = if length > 0. { ((px * dx + py * dy) / length).clamp(0., 1.) } else { 0. };
    (t, (px - t * dx).hypot(py - t * dy))
}

/// Whether `p`, on copper `width_nm` wide, overlaps `track`'s copper.
fn touches(p: Point, width_nm: i64, track: &Track) -> bool {
    track_projection(p, track).1 <= (width_nm + track.width_nm) as f64 / 2.
}

/// New ends for a segment drag of `track` by `offset`, listing only tracks that change.
///
/// The dragged segment moves by `offset`, except that an end meeting the middle of another
/// track on its layer is held there, so the segment pivots and that junction stays attached.
/// Every other track on the layer whose end touches the dragged segment, at one of its ends or
/// along it, follows that point to the same place on the moved segment. Empty when nothing
/// would move, such as when both ends are held.
pub fn dragged_ends<'a>(dragged: &Track, tracks: impl Iterator<Item = &'a Track>, offset: (i64, i64)) -> Vec<(Uuid, Point, Point)> {
    let others: Vec<&Track> = tracks.filter(|t| t.id != dragged.id && t.layer == dragged.layer).collect();
    // Ends of other tracks that touch the dragged segment stretch with it rather than hold it.
    let attached = |other: &Track| touches(other.start, other.width_nm, dragged) || touches(other.end, other.width_nm, dragged);
    let held = |p: Point| others.iter().any(|other| !attached(other) && touches(p, dragged.width_nm, other));
    let shift = |p: Point| if held(p) { (0, 0) } else { offset };
    let (from, to) = (shift(dragged.start), shift(dragged.end));
    if (from, to) == ((0, 0), (0, 0)) {
        return Vec::new();
    }
    let add = |p: Point, (dx, dy): (i64, i64)| Point { x_nm: p.x_nm.saturating_add(dx), y_nm: p.y_nm.saturating_add(dy) };
    // A point on the dragged segment moves by the blend of its ends' moves at its position.
    let follow = |p: Point| {
        let t = if p == dragged.start {
            0.
        } else if p == dragged.end {
            1.
        } else {
            track_projection(p, dragged).0
        };
        let blend = |a: i64, b: i64| a + ((b - a) as f64 * t).round() as i64;
        add(p, (blend(from.0, to.0), blend(from.1, to.1)))
    };
    let mut out = vec![(dragged.id, add(dragged.start, from), add(dragged.end, to))];
    for other in others {
        let ends = [other.start, other.end].map(|p| if touches(p, other.width_nm, dragged) { follow(p) } else { p });
        if ends != [other.start, other.end] {
            out.push((other.id, ends[0], ends[1]));
        }
    }
    out
}
