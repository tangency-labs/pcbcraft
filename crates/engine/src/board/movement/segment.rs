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

/// Where `p` projects onto the segment from `a` to `b`, as the exact fraction `num / den` of the
/// way from `a`, clamped to the segment. Reversing the segment gives exactly `(den - num) / den`.
/// Board coordinates stay within about 10^9 nm, so every product fits in `i128`.
fn exact_fraction(p: Point, a: Point, b: Point) -> (i128, i128) {
    let (dx, dy) = (i128::from(b.x_nm) - i128::from(a.x_nm), i128::from(b.y_nm) - i128::from(a.y_nm));
    let den = dx * dx + dy * dy;
    if den == 0 {
        return (0, 1);
    }
    let num = (i128::from(p.x_nm) - i128::from(a.x_nm)) * dx + (i128::from(p.y_nm) - i128::from(a.y_nm)) * dy;
    (num.clamp(0, den), den)
}

/// The move `from` at fraction 0 blended with the move `to` at fraction 1, rounded to the nearest
/// nanometre with ties away from zero. The value is computed exactly, so swapping the ends
/// (`from` with `to`, `num` with `den - num`) gives the same result for every fraction and sign.
fn blend(from: i64, to: i64, (num, den): (i128, i128)) -> i64 {
    let n = i128::from(from) * (den - num) + i128::from(to) * num;
    let (quotient, remainder) = (n.abs() / den, n.abs() % den);
    let magnitude = if 2 * remainder >= den { quotient + 1 } else { quotient };
    // A blend lies between `from` and `to`, so it fits in `i64`.
    (if n < 0 { -magnitude } else { magnitude }) as i64
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
        let along = exact_fraction(p, dragged.start, dragged.end);
        add(p, (blend(from.0, to.0, along), blend(from.1, to.1, along)))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x_nm: i64, y_nm: i64) -> Point {
        Point { x_nm, y_nm }
    }

    fn track(id: u128, start: Point, end: Point) -> Track {
        Track { id: Uuid::from_u128(id), locked: false, layer: Uuid::from_u128(1), width_nm: 100_000, start, end }
    }

    /// Nearest whole nanometre to `n / d` (d > 0), ties away from zero.
    fn nearest(n: i128, d: i128) -> i128 {
        let q = (2 * n.abs() + d) / (2 * d);
        if n < 0 { -q } else { q }
    }

    /// Exact blends agree in both endpoint orders for every fraction, sign and tie, and match
    /// rounding of the true rational value; a 1/160 tie is where floating-point weights split.
    #[test]
    fn blend_is_independent_of_endpoint_order() {
        let (a, b) = (p(-3_000_000, 7_000_000), p(21_000_000, 25_000_000));
        let moves = [0, 1, -1, 7, -7, 254_000, -254_000, 254_001, -254_005, 2_540_000, -1_000_000_000];
        for k in 0..=160 {
            let at = p(a.x_nm + 150_000 * k, a.y_nm + 112_500 * k);
            let (forward, backward) = (exact_fraction(at, a, b), exact_fraction(at, b, a));
            assert_eq!(forward, (i128::from(k) * forward.1 / 160, forward.1));
            assert_eq!(backward.0, backward.1 - forward.0);
            for from in moves {
                for to in moves {
                    let blended = blend(from, to, forward);
                    assert_eq!(blended, blend(to, from, backward), "k={k} from={from} to={to}");
                    let exact = nearest(i128::from(from) * i128::from(160 - k) + i128::from(to) * i128::from(k), 160);
                    assert_eq!(i128::from(blended), exact, "k={k} from={from} to={to}");
                }
            }
        }
        assert_eq!(blend(0, 254_000, (1, 160)), 1588);
        assert_eq!(blend(254_000, 0, (159, 160)), 1588);
        assert_eq!(blend(0, -254_000, (1, 160)), -1588);
        assert_eq!(blend(-254_000, 0, (159, 160)), -1588);
        // Off the segment the fraction clamps to its ends.
        assert_eq!(exact_fraction(p(-9_000_000, 0), a, b).0, 0);
        assert_eq!(exact_fraction(p(40_000_000, 40_000_000), a, b), (exact_fraction(b, a, b).1, exact_fraction(b, a, b).1));
    }

    /// A pivoting drag moves every follower to the same place whichever way the dragged segment
    /// is stored, at many positions along it and for both signs of an off-axis step.
    #[test]
    fn pivoting_followers_land_alike_in_both_endpoint_orders() {
        let (pivot, free) = (p(0, 0), p(32_000_000, 0));
        let holder = track(2, p(0, -5_000_000), p(0, 5_000_000));
        let followers: Vec<_> = (0..=160).map(|k| track(10 + k as u128, p(200_000 * k, 0), p(200_000 * k, -1_000_000))).collect();
        for offset in [(0, 254_000), (0, -254_000), (254_001, -254_005), (-7, 1)] {
            let landed = |dragged: &Track| {
                let mut ends: Vec<_> = dragged_ends(dragged, std::iter::once(&holder).chain(&followers), offset)
                    .into_iter()
                    .filter(|(id, _, _)| *id != dragged.id)
                    .collect();
                ends.sort_by_key(|(id, _, _)| *id);
                ends
            };
            let forward = landed(&track(3, pivot, free));
            assert_eq!(forward, landed(&track(3, free, pivot)), "{offset:?}");
            for (id, start, end) in &forward {
                let k = i64::try_from(id.as_u128() - 10).unwrap_or_else(|_| panic!("follower id"));
                let shift = |o: i64| i64::try_from(nearest(i128::from(o) * i128::from(k), 160)).unwrap_or_else(|_| panic!("shift"));
                assert_eq!(*start, p(200_000 * k + shift(offset.0), shift(offset.1)), "k={k} {offset:?}");
                assert_eq!(*end, p(200_000 * k, -1_000_000), "k={k} {offset:?}");
            }
        }
    }
}
