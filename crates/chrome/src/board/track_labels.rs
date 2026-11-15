//! Frame-local track annotations follow copper paint order without leaving partial words.
use super::*;

/// Track text already submitted to this painter. Later copper removes overlapping annotations
/// as whole shapes, preserving the active layer's visual priority at junctions and crossings.
pub struct TrackLabels {
    painter: Painter,
    labels: Vec<(Rect, egui::layers::ShapeIdx)>,
}

impl TrackLabels {
    /// Begin an annotation pass on the same layer and clip rectangle as the copper painter.
    pub fn new(painter: Painter) -> Self {
        Self { painter, labels: Vec::new() }
    }

    /// Paint fitting names, omitting candidates that overlap an existing readable name.
    pub fn track(&mut self, ends: [Pos2; 2], width: f32, name: &str, copper: Color32) {
        track_net_labels(&self.painter, ends, width, name, copper, |shape| {
            let bounds = shape.visual_bounding_rect().expand(2.);
            if !self.labels.iter().any(|(other, _)| other.intersects(bounds)) {
                self.labels.push((bounds, self.painter.add(shape)));
            }
        });
    }

    /// Omit earlier names touched by a later track, including its rounded ends. The expanded
    /// rectangle test is conservative near corners so no glyph is left partly obscured.
    pub fn cover_track(&mut self, ends: [Pos2; 2], width: f32) {
        self.remove_where(|bounds| {
            let bounds = bounds.expand(width / 2.);
            let delta = ends[1] - ends[0];
            let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
            for (origin, step, min, max) in [(ends[0].x, delta.x, bounds.min.x, bounds.max.x), (ends[0].y, delta.y, bounds.min.y, bounds.max.y)] {
                if step.abs() <= f32::EPSILON {
                    if origin < min || origin > max {
                        return false;
                    }
                } else {
                    let a = (min - origin) / step;
                    let b = (max - origin) / step;
                    lo = lo.max(a.min(b));
                    hi = hi.min(a.max(b));
                }
            }
            lo <= hi
        });
    }

    /// Omit earlier names beneath a via, including the hole that replaces its center.
    pub fn cover_disc(&mut self, center: Pos2, radius: f32) {
        self.remove_where(|bounds| bounds.distance_to_pos(center) <= radius);
    }

    /// Omit earlier names beneath a later pad's projected bounds. Rounded and rotated pad
    /// corners conservatively reserve their bounding rectangle rather than clipping text.
    pub fn cover_rect(&mut self, copper: Rect) {
        self.remove_where(|bounds| bounds.intersects(copper));
    }

    /// Remove covered shapes from the existing paint list, retaining only visible annotations.
    fn remove_where(&mut self, covered: impl Fn(Rect) -> bool) {
        self.labels.retain(|(bounds, index)| {
            if covered(*bounds) {
                self.painter.set(*index, egui::Shape::Noop);
                false
            } else {
                true
            }
        });
    }
}
