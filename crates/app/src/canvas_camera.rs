use egui::{Event, MouseWheelUnit, PointerButton, Pos2, Rect, Response, Vec2, pos2, vec2};
use pcbcraft_chrome::{grid, theme};
use pcbcraft_engine::{
    Project,
    schematic::{GRID_NM, MAX_CENTER_NM, MAX_PIN_OFFSET_NM, Point},
    selection::Selection,
};

use crate::preferences::{Navigation, ScrollAction, ZoomAround};
use crate::schematic_canvas::grid_bounds;

const MIN_ZOOM: f32 = 0.005;
pub(super) const MAX_ZOOM: f32 = 8.0;
/// Board navigation needs to show fine copper and clearance at a useful scale.
pub(super) const MAX_BOARD_ZOOM: f32 = 64.0;
/// Start empty boards at 32 logical points/mm so ordinary copper widths are visible.
const INITIAL_BOARD_ZOOM: f32 = 4.0;
/// A wheel notch settles within this interval; live pinch gestures remain immediate.
const WHEEL_ZOOM_SECONDS: f32 = 0.14;

/// Transient wheel motion belongs to one camera and is never saved in an arrangement.
#[derive(Clone, Debug)]
struct WheelZoom {
    from: f32,
    target: f32,
    elapsed: f32,
    anchor: Vec2,
    navigation: Navigation,
    max_zoom: f32,
}

/// Persistent navigation excludes viewport geometry and pending input.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CameraState {
    zoom: f32,
    center_nm: [f64; 2],
    snap_nm: i64,
}

impl CameraState {
    /// Validate navigation against the saved workspace's zoom limit before restoring it.
    pub(super) fn valid(&self, max_zoom: f32) -> bool {
        (MIN_ZOOM..=max_zoom).contains(&self.zoom)
            && grid::SPACING_RANGE_NM.contains(&self.snap_nm)
            && self.center_nm.iter().all(|n| n.is_finite() && n.abs() <= (MAX_CENTER_NM + MAX_PIN_OFFSET_NM) as f64 + 1.0)
    }
}

/// Viewport and snapping state in egui points. Navigation and the snap grid stay outside the
/// project and its undo history. Clones retain pending navigation and host geometry; independent
/// views are seeded with `fork_navigation` instead.
#[derive(Clone, Debug)]
pub(crate) struct Camera {
    zoom: f32,
    wheel_zoom: Option<WheelZoom>,
    navigation_frame: Option<(egui::ViewportId, u64)>,
    schematic_rect: Option<Rect>,
    #[cfg(feature = "performance-harness")]
    pub(super) performance_rect: Option<Rect>,
    offset: Vec2,
    fit_requested: bool,
    /// A Browser destination waits for its own panel geometry before centering.
    center_requested: Option<Point>,
    /// Usable canvas rectangle relative to its center, clear of floating cards.
    uncovered: Option<Rect>,
    /// Spacing of the points the pointer snaps to, in nanometres. It never changes the scale:
    /// drawing is always in [`GRID_NM`] units, so a new spacing leaves everything where it is.
    snap_nm: i64,
    /// View the board from below: model X is drawn right-to-left. Only board views set this; it
    /// changes presentation and picking, never the model, and is not saved with the arrangement.
    mirrored: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            wheel_zoom: None,
            navigation_frame: None,
            schematic_rect: None,
            #[cfg(feature = "performance-harness")]
            performance_rect: None,
            offset: Vec2::ZERO,
            fit_requested: false,
            center_requested: None,
            uncovered: None,
            snap_nm: grid::SCHEMATIC_SPACING_DEFAULT_NM,
            mirrored: false,
        }
    }
}

impl Camera {
    /// Fresh PCB navigation uses a closer physical scale than the schematic. Restored camera
    /// state and deliberate navigation keep their own scale; this is not a minimum zoom.
    pub(super) fn board() -> Self {
        Self { zoom: INITIAL_BOARD_ZOOM, snap_nm: grid::BOARD_SPACING_DEFAULT_NM, ..Self::default() }
    }

    /// Last painted schematic bounds, for prioritizing hovered-object shortcuts before layout.
    /// This disposable rectangle never enters saved camera state or document identity.
    pub(crate) fn schematic_rect(&self) -> Option<Rect> {
        self.schematic_rect
    }

    /// Capture world-space navigation independently of pane size and transient fit requests.
    pub(super) fn saved_state(&self) -> CameraState {
        let scale = f64::from(self.unit_step()) / GRID_NM as f64;
        // The runtime offset is f32; rounding at the navigation boundary must not invalidate
        // an otherwise valid snapshot when converted back to nanometres.
        let limit = (MAX_CENTER_NM + MAX_PIN_OFFSET_NM) as f64;
        let mut center_nm = [self.offset.x, self.offset.y].map(|offset| (-f64::from(offset) / scale).clamp(-limit, limit));
        center_nm[0] *= self.sign() as f64;
        CameraState { zoom: self.zoom, center_nm, snap_nm: self.snap_nm }
    }

    /// Restore validated navigation with fresh host geometry and no deferred fit.
    pub(super) fn from_state(state: &CameraState) -> Self {
        let mut camera = Self { zoom: state.zoom, snap_nm: state.snap_nm, ..Self::default() };
        let scale = f64::from(camera.unit_step()) / GRID_NM as f64;
        camera.offset = vec2((-state.center_nm[0] * scale) as f32, (-state.center_nm[1] * scale) as f32);
        camera
    }

    /// Seed another view from the currently displayed transform and grid, leaving its host
    /// geometry unset. Splitting cancels wheel motion, Fit and centering requests so both
    /// views start at the displayed zoom without inheriting pending movement.
    pub(crate) fn fork_navigation(&mut self) -> Self {
        self.cancel_zoom();
        self.fit_requested = false;
        self.center_requested = None;
        Self { zoom: self.zoom, offset: self.offset, snap_nm: self.snap_nm, mirrored: self.mirrored, ..Self::default() }
    }

    /// Whether this view shows the board from below.
    pub(crate) fn mirrored(&self) -> bool {
        self.mirrored
    }

    /// Convert a counterclockwise on-screen rotation in degrees into model rotation, normalized
    /// to 0..360. Reflection reverses the model angle so user controls keep their visible direction.
    pub(crate) fn model_rotation(&self, degrees: u16) -> u16 {
        let degrees = degrees % 360;
        if self.mirrored { (360 - degrees) % 360 } else { degrees }
    }

    /// Show the board from below or above, keeping the same model point at the center of the
    /// uncovered canvas and the zoom unchanged. Only presentation and picking change.
    pub(crate) fn set_mirrored(&mut self, mirrored: bool) {
        if self.mirrored != mirrored {
            self.cancel_zoom();
            self.mirrored = mirrored;
            self.offset.x = 2.0 * self.uncovered_center().x - self.offset.x;
        }
    }

    /// -1 when model X is drawn right-to-left, otherwise 1.
    fn sign(&self) -> f32 {
        if self.mirrored { -1.0 } else { 1.0 }
    }

    /// The snap spacing in nanometres.
    pub(crate) fn snap_nm(&self) -> i64 {
        self.snap_nm
    }

    /// Snap to points `spacing_nm` apart from now on. Values outside [`grid::SPACING_RANGE_NM`]
    /// are ignored and return false; nothing moves, and the zoom and pan stay as they are.
    pub(crate) fn set_snap_nm(&mut self, spacing_nm: i64) -> bool {
        let valid = grid::SPACING_RANGE_NM.contains(&spacing_nm);
        if valid {
            self.snap_nm = spacing_nm;
        }
        valid
    }

    /// The on-screen distance between snap points, before the grid thins its dots out.
    pub(crate) fn snap_step(&self) -> f32 {
        (f64::from(self.unit_step()) * self.snap_nm as f64 / GRID_NM as f64) as f32
    }

    /// Reveal the selected symbols and wire paths if they extend outside the usable canvas.
    /// Zoom remains unchanged; oversized groups are centered even when they cannot fully fit.
    /// Empty selections, stale IDs and unavailable host geometry do not move the camera.
    pub(crate) fn reveal_selection(&mut self, project: &Project, selection: &Selection) {
        let mut bounds = Rect::NOTHING;
        let rect = Rect::from_center_size(Pos2::ZERO, Vec2::ZERO);
        for symbol in project.symbols().iter().filter(|symbol| selection.symbols.contains(&symbol.id())) {
            let center = self.screen(symbol.position, rect).to_vec2();
            let extent = grid_bounds(symbol.unit_definition(), symbol.turns(), symbol.is_mirrored());
            let step = self.unit_step();
            bounds = bounds.union(
                Rect::from_min_max(pos2(extent.min.x * step, extent.min.y * step), pos2(extent.max.x * step, extent.max.y * step)).translate(center),
            );
        }
        for point in project.wires().iter().filter(|wire| selection.wires.contains(&wire.id())).flat_map(|wire| project.wire_path(wire)) {
            bounds = bounds.union(Rect::from_pos(self.screen(point, rect)));
        }
        for (_, at) in project.no_connect_appearances().filter(|(target, _)| selection.no_connects.contains(target)) {
            let radius = crate::no_connect::radius(self.unit_step());
            bounds = bounds.union(Rect::from_center_size(self.screen(at, rect), Vec2::splat(2. * radius)));
        }
        self.reveal_bounds(bounds);
    }

    /// Center already projected bounds only when they are outside the usable region. Keep a
    /// label margin where space permits; a visible selection never nudges the camera.
    fn reveal_bounds(&mut self, bounds: Rect) {
        if let Some(uncovered) = self.uncovered
            && uncovered.is_positive()
            && bounds.is_finite()
        {
            let margin = theme::SCHEMATIC_FIT_MARGIN.min(uncovered.width() / 4.0).min(uncovered.height() / 4.0);
            if !uncovered.shrink(margin).contains_rect(bounds) {
                self.cancel_zoom();
                self.offset += uncovered.center() - bounds.center();
                self.fit_requested = false;
            }
        }
    }

    /// Fit after the menu closes using the next canvas rectangle, superseding pending centering.
    pub(crate) fn request_fit(&mut self) {
        self.cancel_zoom();
        self.center_requested = None;
        self.fit_requested = true;
    }

    /// Center a Browser pick once the destination workspace has supplied its panel geometry.
    /// This supersedes Fit while retaining zoom, including for a newly created document view.
    pub(crate) fn request_center(&mut self, point: Point) {
        self.cancel_zoom();
        self.fit_requested = false;
        self.center_requested = Some(point);
    }

    /// Set the rectangle available to Fit and centering, relative to the full canvas. Opening
    /// or moving a card changes this target without moving the camera unless centering is pending.
    /// A Browser center request is consumed only after panel geometry settles; true requests
    /// another frame. Explicit navigation cancels that pending authority.
    pub(crate) fn set_uncovered(&mut self, canvas: Rect, uncovered: Rect) -> bool {
        let next = Some(uncovered.translate(-canvas.center().to_vec2()));
        let stable = self.uncovered == next;
        self.uncovered = next;
        if let Some(point) = self.center_requested.take() {
            self.center_on(point);
            if !stable {
                self.center_requested = Some(point);
                return true;
            }
        }
        false
    }

    /// The canvas offset that puts the drawing's origin at the middle of the uncovered part.
    fn uncovered_center(&self) -> Vec2 {
        self.uncovered.map_or(Vec2::ZERO, |rect| rect.center().to_vec2())
    }

    /// Pan so a model point sits at the center of the uncovered canvas, keeping the zoom.
    /// Cancels pending fit and centering requests, which would otherwise move the view away again.
    pub(crate) fn center_on(&mut self, point: Point) {
        self.cancel_zoom();
        self.center_requested = None;
        let scale = self.unit_step() / GRID_NM as f32;
        self.offset = self.uncovered_center() - vec2(self.sign() * point.x_nm as f32, point.y_nm as f32) * scale;
        self.fit_requested = false;
    }

    /// The current zoom factor, where 1 shows one 50 mil drawing unit at its default size.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn zoom(&self) -> f32 {
        self.zoom
    }

    /// The model point at the center of the uncovered canvas, to the nearest nanometre. Before
    /// the view has been painted this is relative to the whole canvas.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn center(&self) -> Point {
        let scale = self.unit_step() / GRID_NM as f32;
        let center = (self.uncovered_center() - self.offset) / scale;
        Point { x_nm: (self.sign() * center.x).round() as i64, y_nm: center.y.round() as i64 }
    }

    /// Zoom about the uncovered canvas center within the definition and schematic limits.
    /// Cancels a pending fit, like any navigation.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn set_zoom(&mut self, zoom: f32) {
        self.set_zoom_with_limit(zoom, MAX_ZOOM);
    }

    /// Zoom about the uncovered canvas center, clamped to the target workspace's limits.
    /// Cancels a pending fit, like any navigation.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn set_zoom_with_limit(&mut self, zoom: f32, max_zoom: f32) {
        let center = self.center();
        self.zoom = zoom.clamp(MIN_ZOOM, max_zoom);
        self.center_on(center);
    }

    /// The on-screen size of one [`GRID_NM`] drawing unit (50 mil), which symbol geometry is
    /// measured in. It depends on zoom only, never on the snap spacing.
    pub(crate) fn unit_step(&self) -> f32 {
        theme::SCHEMATIC_GRID_SPACING * self.zoom
    }

    /// Convert a model point to a point on the canvas.
    pub(crate) fn screen(&self, point: Point, rect: Rect) -> Pos2 {
        let scale = self.unit_step() / GRID_NM as f32;
        rect.center() + self.offset + vec2(self.sign() * point.x_nm as f32, point.y_nm as f32) * scale
    }

    /// Snap the pointer to the nearest multiple of the snap spacing in schematic coordinates,
    /// even when the displayed grid is coarser. The division is in `f64`, so points far from
    /// the origin still land exactly on the grid.
    pub(crate) fn snap(&self, pointer: Pos2, rect: Rect) -> Point {
        self.point_at_spacing(pointer, rect, self.snap_nm.max(1))
    }

    /// Convert a pointer to integer nanometres without applying the view's placement grid.
    /// Callers still validate model bounds and resolve explicit connection targets.
    pub(crate) fn unsnapped(&self, pointer: Pos2, rect: Rect) -> Point {
        self.point_at_spacing(pointer, rect, 1)
    }

    /// Convert screen coordinates at a positive spacing, using f64 before rounding.
    fn point_at_spacing(&self, pointer: Pos2, rect: Rect, spacing: i64) -> Point {
        let relative = pointer - rect.center() - self.offset;
        let nm_per_point = GRID_NM as f64 / f64::from(self.unit_step());
        let snap = |points: f32| (f64::from(points) * nm_per_point / spacing as f64).round() as i64 * spacing;
        Point { x_nm: snap(self.sign() * relative.x), y_nm: snap(relative.y) }
    }

    /// Apply navigation only when the canvas owns input, following the person's settings.
    /// The zoom anchor (the pointer, or the view center) stays fixed while zooming. When
    /// scrolling pans, Ctrl or ⌘ with the wheel still zooms, and a pinch always zooms.
    pub(crate) fn navigate(&mut self, response: &Response, rect: Rect, project: &Project, enabled: bool, navigation: Navigation) {
        self.schematic_rect = Some(rect);
        if std::mem::take(&mut self.fit_requested) {
            self.fit(project, rect);
        }
        self.interact(response, rect, enabled, navigation, MAX_ZOOM);
    }

    /// Navigate and fit resolved board copper/graphics, independently of schematic geometry.
    /// Wheel and pinch can magnify beyond the whole-board Fit range to inspect fine copper.
    /// Fit includes the authored board boundary independently of layer visibility.
    /// An empty board returns to its initial drawing scale, centered in the uncovered canvas.
    pub(crate) fn navigate_board(
        &mut self,
        response: &Response,
        rect: Rect,
        project: pcbcraft_engine::board::view::BoardView<'_>,
        enabled: bool,
        navigation: Navigation,
    ) {
        #[cfg(feature = "performance-harness")]
        {
            self.performance_rect = Some(rect);
        }
        if std::mem::take(&mut self.fit_requested) {
            *self = Self { uncovered: self.uncovered, snap_nm: self.snap_nm, mirrored: self.mirrored, ..Self::default() };
            let copper = project.tracks().iter().flat_map(|track| [track.start, track.end]).chain(project.vias().iter().map(|via| via.at));
            let bounds = project
                .footprints()
                .flat_map(board_extent)
                .chain(project.drawings().into_iter().flat_map(|drawing| drawing.points))
                .chain(copper)
                .fold(Rect::NOTHING, |bounds, point| {
                    bounds.union(Rect::from_pos((self.screen(point, rect) - rect.center()).to_pos2()).expand(theme::SCHEMATIC_FIT_MARGIN))
                });
            if bounds.is_finite() {
                self.fit_bounds(bounds, rect, MAX_ZOOM);
            } else {
                self.zoom = INITIAL_BOARD_ZOOM;
                self.offset = self.uncovered_center();
            }
        }
        self.interact(response, rect, enabled, navigation, MAX_BOARD_ZOOM);
    }

    /// Fit native library primitives and navigate without a project, sheet or placed board.
    /// Caller-provided extents use integer nanometres in the definition's local coordinates.
    pub(crate) fn navigate_definition(
        &mut self,
        response: &Response,
        rect: Rect,
        points: impl Iterator<Item = Point>,
        enabled: bool,
        navigation: Navigation,
    ) {
        self.schematic_rect = Some(rect);
        if std::mem::take(&mut self.fit_requested) {
            *self = Self { uncovered: self.uncovered, snap_nm: self.snap_nm, mirrored: self.mirrored, ..Self::default() };
            let bounds = points.fold(Rect::NOTHING, |bounds, point| {
                bounds.union(Rect::from_pos((self.screen(point, rect) - rect.center()).to_pos2()).expand(theme::SCHEMATIC_FIT_MARGIN))
            });
            self.fit_bounds(bounds, rect, MAX_ZOOM);
        }
        self.interact(response, rect, enabled, navigation, MAX_ZOOM);
    }

    /// Reveal selected board placements in their own coordinate space without changing zoom.
    pub(crate) fn reveal_board(&mut self, project: pcbcraft_engine::board::view::BoardView<'_>, selected: &std::collections::BTreeSet<String>) {
        let bounds =
            project.footprints().filter(|footprint| selected.contains(&footprint.id)).flat_map(board_extent).fold(Rect::NOTHING, |bounds, point| {
                bounds.union(Rect::from_pos(self.screen(point, Rect::ZERO)).expand(theme::SCHEMATIC_FIT_MARGIN))
            });
        self.reveal_bounds(bounds);
    }

    /// Stop transient wheel motion at its displayed transform, without changing saved navigation.
    pub(crate) fn cancel_zoom(&mut self) {
        self.wheel_zoom = None;
    }

    /// Apply a scale about a canvas-relative anchor, preserving its model point even in a mirrored view.
    fn zoom_at(&mut self, zoom: f32, anchor: Vec2, max_zoom: f32) {
        let before = (anchor - self.offset) / self.zoom;
        self.zoom = zoom.clamp(MIN_ZOOM, max_zoom);
        self.offset = anchor - before * self.zoom;
    }

    /// Accumulate wheel notches toward a bounded target. Reversal or changed settings replace
    /// outstanding motion so the camera responds immediately in the new direction.
    fn wheel_target(&mut self, delta: f32, anchor: Vec2, navigation: Navigation, max_zoom: f32) {
        let base = self
            .wheel_zoom
            .as_ref()
            .filter(|motion| motion.navigation == navigation && (motion.target - self.zoom).signum() == delta.signum())
            .map_or(self.zoom, |motion| motion.target);
        let target = (base * delta.clamp(-2.0, 2.0).exp()).clamp(MIN_ZOOM, max_zoom);
        self.wheel_zoom = (target != self.zoom).then_some(WheelZoom { from: self.zoom, target, elapsed: 0.0, anchor, navigation, max_zoom });
    }

    /// Advance a finite, eased transition in logarithmic scale. Return whether another frame
    /// is needed, and land exactly on the target rather than leaving an endless animation tail.
    fn advance_zoom(&mut self, dt: f32) -> bool {
        let Some(mut motion) = self.wheel_zoom.take() else { return false };
        let dt = if dt.is_finite() { dt.clamp(0.0, 0.05) } else { 1.0 / 60.0 };
        motion.elapsed += dt;
        let progress = (motion.elapsed / WHEEL_ZOOM_SECONDS).min(1.0);
        let zoom = if progress == 1.0 {
            motion.target
        } else {
            let eased = 1.0 - (1.0 - progress).powi(3);
            (motion.from.ln() + (motion.target.ln() - motion.from.ln()) * eased).exp()
        };
        self.zoom_at(zoom, motion.anchor, motion.max_zoom);
        if progress < 1.0 {
            self.wheel_zoom = Some(motion);
            true
        } else {
            false
        }
    }

    /// Apply navigation once per native viewport frame. Wheel motion eases while this canvas
    /// owns input; pinch and pan stay direct. A click, focus loss, blocked input or leaving the
    /// canvas stops the transition, so motion cannot leak into another pane or an editing click.
    fn interact(&mut self, response: &Response, rect: Rect, enabled: bool, navigation: Navigation, max_zoom: f32) {
        let frame = (response.ctx.viewport_id(), response.ctx.cumulative_frame_nr());
        if self.navigation_frame == Some(frame) {
            return;
        }
        self.navigation_frame = Some(frame);
        if !enabled || !response.ctx.input(|input| input.focused) {
            self.cancel_zoom();
            return;
        }
        if response.ctx.input(|input| input.pointer.any_pressed()) {
            self.cancel_zoom();
        }
        if response.dragged_by(PointerButton::Middle) {
            self.cancel_zoom();
            self.center_requested = None;
            self.offset += response.drag_delta();
        }
        if response.contains_pointer()
            && let Some(pointer) = response.ctx.input(|i| i.pointer.hover_pos())
        {
            let Wheel { zoom, pinch, pan } = response.ctx.input(|input| wheel(&input.events, rect.height(), navigation));
            if pan != Vec2::ZERO || zoom != 0.0 || pinch != 0.0 {
                self.center_requested = None;
            }
            self.offset += pan;
            if pan != Vec2::ZERO || self.wheel_zoom.as_ref().is_some_and(|motion| motion.navigation != navigation || motion.max_zoom != max_zoom) {
                self.cancel_zoom();
            }
            let anchor = match navigation.zoom_around {
                ZoomAround::Pointer => pointer - rect.center(),
                ZoomAround::Center => Vec2::ZERO,
            };
            if pinch != 0.0 {
                self.cancel_zoom();
                self.zoom_at(self.zoom * (zoom + pinch).clamp(-2.0, 2.0).exp(), anchor, max_zoom);
            } else if zoom != 0.0 {
                self.wheel_target(zoom, anchor, navigation, max_zoom);
            }
            if self.advance_zoom(response.ctx.input(|input| input.stable_dt)) {
                response.ctx.request_repaint();
            }
        } else {
            self.cancel_zoom();
        }
        // Keep the view center within reach of the full schematic coordinate range.
        let limit = Vec2::splat((MAX_CENTER_NM + MAX_PIN_OFFSET_NM) as f32 / GRID_NM as f32 * self.unit_step());
        self.offset = self.offset.clamp(-limit, limit);
    }

    /// Center symbol and wire bounds, including free routes and a margin for labels, in the part
    /// of the canvas left uncovered by all visible floating cards.
    fn fit(&mut self, project: &Project, rect: Rect) {
        *self = Self { uncovered: self.uncovered, snap_nm: self.snap_nm, mirrored: self.mirrored, ..Self::default() };
        let mut bounds = Rect::NOTHING;
        for symbol in project.symbols() {
            let center = self.screen(symbol.position, rect) - rect.center();
            let extent = grid_bounds(symbol.unit_definition(), symbol.turns(), symbol.is_mirrored());
            let step = self.unit_step();
            let symbol_bounds = Rect::from_min_max(pos2(extent.min.x * step, extent.min.y * step), pos2(extent.max.x * step, extent.max.y * step))
                .translate(center)
                .expand(theme::SCHEMATIC_FIT_MARGIN);
            bounds = bounds.union(symbol_bounds);
        }
        for point in project.wires().iter().flat_map(|wire| project.wire_path(wire)) {
            bounds = bounds.union(Rect::from_pos((self.screen(point, rect) - rect.center()).to_pos2()).expand(theme::SCHEMATIC_FIT_MARGIN));
        }
        for point in project.net_labels().iter().map(|l| l.at).chain(project.hierarchical_labels().iter().map(|l| l.at)).chain(
            project.sheet_symbols().iter().flat_map(|block| {
                let size = project.sheet_symbol_size(block);
                [block.at, Point { x_nm: block.at.x_nm + size.x_nm, y_nm: block.at.y_nm + size.y_nm }]
            }),
        ) {
            bounds = bounds.union(Rect::from_pos((self.screen(point, rect) - rect.center()).to_pos2()).expand(theme::SCHEMATIC_FIT_MARGIN));
        }
        self.fit_bounds(bounds, rect, 1.0);
    }

    /// Fit resolved document bounds in the unobscured viewport, retaining the grid spacing.
    fn fit_bounds(&mut self, bounds: Rect, rect: Rect, zoom_limit: f32) {
        if bounds.is_finite() {
            let size = self.uncovered.map_or(rect.size(), |clear| clear.size());
            let available = (size - Vec2::splat(2.0 * theme::SCHEMATIC_FIT_MARGIN)).max(Vec2::splat(1.0));
            self.zoom = (available.x / bounds.width()).min(available.y / bounds.height()).clamp(MIN_ZOOM, zoom_limit);
            self.offset = self.uncovered_center() - bounds.center().to_vec2() * self.zoom;
        }
    }
}

/// This frame's separate logarithmic wheel/pinch deltas and direct pan in points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Wheel {
    zoom: f32,
    pinch: f32,
    pan: Vec2,
}

/// Normalize mouse-wheel units and pinch gestures, then apply the navigation settings.
/// Speed and direction change wheel zoom only; a pinch follows the fingers.
fn wheel(events: &[Event], viewport_height: f32, navigation: Navigation) -> Wheel {
    let direction = if navigation.reverse_zoom { -1.0 } else { 1.0 };
    let mut out = Wheel::default();
    for event in events {
        match event {
            Event::MouseWheel { unit, delta, modifiers, .. } if delta.is_finite() => {
                let scale = match unit {
                    MouseWheelUnit::Point => 1.0,
                    MouseWheelUnit::Line => 40.0,
                    MouseWheelUnit::Page => viewport_height,
                };
                if navigation.scroll == ScrollAction::Pan && !modifiers.command {
                    out.pan += *delta * scale;
                } else {
                    out.zoom += (delta.y * scale * 0.004 * navigation.zoom_speed()).clamp(-1.5, 1.5) * direction;
                }
            }
            Event::Zoom(factor) if factor.is_finite() && *factor > 0.0 => out.pinch += factor.ln().clamp(-1.5, 1.5),
            _ => {}
        }
    }
    out
}

/// Fit/reveal includes real copper and technical extents; legacy footprints retain their centers.
fn board_extent(footprint: pcbcraft_engine::board::view::FootprintView<'_>) -> impl Iterator<Item = Point> {
    footprint.pads.into_iter().flat_map(|pad| std::iter::once(pad.center).chain(pad.copper.into_iter().flatten())).chain(
        footprint.graphics.into_iter().flat_map(|g| {
            let half = (g.stroke_nm.unwrap_or(0) + 1) / 2;
            g.points
                .into_iter()
                .flat_map(move |p| [Point { x_nm: p.x_nm - half, y_nm: p.y_nm - half }, Point { x_nm: p.x_nm + half, y_nm: p.y_nm + half }])
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Visible targets do not move, while out-of-view and oversized bounds center in the
    /// uncovered region without zooming. Missing or invalid geometry cannot move the view.
    #[test]
    fn reveal_respects_uncovered_geometry_without_changing_zoom() {
        let mut camera = Camera { zoom: 2.0, offset: vec2(12.0, 30.0), ..Camera::default() };
        let canvas = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let clear = Rect::from_min_size(pos2(300.0, 0.0), vec2(500.0, 600.0));
        camera.set_uncovered(canvas, clear);
        let usable = camera.uncovered.unwrap();
        let visible = Rect::from_center_size(usable.center(), vec2(40.0, 40.0));
        camera.reveal_bounds(visible);
        assert_eq!(camera.offset, vec2(12.0, 30.0));
        let outside = visible.translate(vec2(-600.0, 800.0));
        camera.reveal_bounds(outside);
        assert_eq!(camera.offset, vec2(612.0, -770.0));
        let oversized = Rect::from_center_size(pos2(500.0, 400.0), vec2(1000.0, 1000.0));
        let before = camera.offset;
        camera.reveal_bounds(oversized);
        assert_eq!(camera.offset, before + (usable.center() - oversized.center()));
        let before = camera.offset;
        camera.reveal_bounds(Rect::NOTHING);
        camera.uncovered = None;
        camera.reveal_bounds(outside);
        assert_eq!(camera.offset, before);
        assert_eq!(camera.zoom, 2.0);
    }

    /// A mirrored camera draws model X right-to-left, maps the pointer back to the same model
    /// point, keeps the centered point when toggled, and saves its center in model coordinates.
    #[test]
    fn mirroring_round_trips_points_and_keeps_the_center() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let mut camera = Camera { zoom: 2.0, offset: vec2(-30.0, 14.0), snap_nm: 1, ..Camera::default() };
        let point = Point { x_nm: 3_810_000, y_nm: -1_270_000 };
        let center = camera.saved_state().center_nm;
        let plain = camera.screen(point, rect);
        camera.set_mirrored(true);
        let mirrored = camera.screen(point, rect);
        assert!((mirrored.y - plain.y).abs() < 0.01);
        assert_ne!(mirrored.x, plain.x);
        assert_eq!(camera.unsnapped(mirrored, rect), point);
        let saved = camera.saved_state().center_nm;
        assert!((saved[0] - center[0]).abs() < 1.0 && (saved[1] - center[1]).abs() < 1.0, "{saved:?} vs {center:?}");
        camera.set_mirrored(false);
        assert!((camera.screen(point, rect) - plain).length() < 0.01);
    }

    /// Snapping follows the chosen spacing without changing where model points are drawn.
    #[test]
    fn snapping_follows_the_spacing_and_keeps_the_scale() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let mut camera = Camera { zoom: 2.0, offset: vec2(-30.0, 14.0), ..Camera::default() };
        let mil = grid::NM_PER_MIL;
        let at = |camera: &Camera, x: i64, y: i64| camera.screen(Point { x_nm: x * mil, y_nm: y * mil }, rect);
        let pointer = at(&camera, 37, -63);
        assert_eq!(camera.snap(pointer, rect), Point { x_nm: 50 * mil, y_nm: -50 * mil });
        let screen_before = at(&camera, 37, -63);
        assert!(camera.set_snap_nm(25 * mil));
        assert_eq!(camera.snap(pointer, rect), Point { x_nm: 25 * mil, y_nm: -75 * mil });
        assert!(camera.set_snap_nm(10 * mil));
        assert_eq!(camera.snap(pointer, rect), Point { x_nm: 40 * mil, y_nm: -60 * mil });
        assert!(camera.set_snap_nm(100 * mil));
        assert_eq!(camera.snap(pointer, rect), Point { x_nm: 0, y_nm: -100 * mil });
        assert_eq!(at(&camera, 37, -63), screen_before);
        // The unit is 50 mil, so 100 mil snap points are two units apart.
        assert_eq!(camera.snap_step(), 2.0 * camera.unit_step());
        assert!(!camera.set_snap_nm(0) && !camera.set_snap_nm(-mil) && !camera.set_snap_nm(2000 * mil));
        assert_eq!(camera.snap_nm(), 100 * mil);
        // Far from the origin the result is still an exact multiple of the spacing.
        assert!(camera.set_snap_nm(30 * mil));
        camera.center_on(Point { x_nm: MAX_CENTER_NM - 7 * mil, y_nm: -MAX_CENTER_NM + 3 * mil });
        let far = camera.snap(at(&camera, MAX_CENTER_NM / mil - 7, -MAX_CENTER_NM / mil + 3), rect);
        assert_eq!((far.x_nm % (30 * mil), far.y_nm % (30 * mil)), (0, 0));
        assert!((far.x_nm - (MAX_CENTER_NM - 7 * mil)).abs() <= 15 * mil);
        // Fit, split and zoom keep the spacing.
        let mut session = pcbcraft_engine::Session::default();
        session.execute("project.new", &serde_json::json!({"name": "Grid"})).unwrap();
        camera.fit(&session.snapshot().unwrap(), rect);
        assert_eq!((camera.snap_nm(), camera.fork_navigation().snap_nm()), (30 * mil, 30 * mil));
    }

    /// Every sampled frame changes scale smoothly while keeping the anchor fixed. Motion
    /// finishes at the same scale at different refresh rates, including mirrored PCB views.
    #[test]
    fn wheel_motion_interpolates_and_preserves_its_anchor() {
        for fps in [30., 60., 144.] {
            for mirrored in [false, true] {
                for anchor in [Vec2::ZERO, vec2(63., -28.)] {
                    let mut camera = Camera { zoom: 0.6, offset: vec2(12., 30.), mirrored, ..Camera::default() };
                    let point = (anchor - camera.offset) / camera.zoom;
                    let target = camera.zoom * 0.32_f32.exp();
                    camera.wheel_target(0.32, anchor, Navigation::default(), MAX_ZOOM);
                    let mut frames = 0;
                    while camera.wheel_zoom.is_some() {
                        let before = camera.zoom;
                        camera.advance_zoom(1. / fps);
                        assert!(camera.zoom > before && camera.zoom <= target);
                        assert!(((anchor - camera.offset) / camera.zoom - point).length() < 0.001);
                        frames += 1;
                        assert!(frames <= 22, "motion did not settle at {fps} Hz");
                    }
                    assert!(frames > 1);
                    assert_eq!(camera.zoom, target);
                }
            }
        }
    }

    /// A new notch adds to the pending target, while reversing direction responds from
    /// the displayed scale instead of continuing the old zoom. Bounds cannot leave a tail.
    #[test]
    fn wheel_motion_accumulates_reverses_and_clamps() {
        let mut camera = Camera::default();
        camera.wheel_target(0.16, Vec2::ZERO, Navigation::default(), MAX_ZOOM);
        camera.advance_zoom(1. / 60.);
        camera.wheel_target(0.16, Vec2::ZERO, Navigation::default(), MAX_ZOOM);
        assert!((camera.wheel_zoom.as_ref().unwrap().target - 0.32_f32.exp()).abs() < 1e-6);
        let before = camera.zoom;
        camera.wheel_target(-0.16, Vec2::ZERO, Navigation::default(), MAX_ZOOM);
        camera.advance_zoom(1. / 60.);
        assert!(camera.zoom < before);
        for (zoom, delta) in [(MAX_ZOOM, 0.16), (MIN_ZOOM, -0.16)] {
            camera.zoom = zoom;
            camera.wheel_target(delta, Vec2::ZERO, Navigation::default(), MAX_ZOOM);
            assert!(camera.wheel_zoom.is_none());
        }
    }

    /// Splits and saved arrangements start from the visible transform, with no queued
    /// wheel movement. Explicit centering and Fit discard that transient authority too.
    #[test]
    fn wheel_motion_is_not_saved_or_inherited_by_another_view() {
        let mut camera = Camera::default();
        camera.wheel_target(0.32, vec2(50., 30.), Navigation::default(), MAX_ZOOM);
        camera.advance_zoom(1. / 60.);
        let restored = Camera::from_state(&camera.saved_state());
        assert_eq!(restored.zoom, camera.zoom);
        assert!(restored.wheel_zoom.is_none());
        let fork = camera.fork_navigation();
        assert_eq!(fork.zoom, camera.zoom);
        assert!(camera.wheel_zoom.is_none() && fork.wheel_zoom.is_none());
        camera.wheel_target(0.32, Vec2::ZERO, Navigation::default(), MAX_ZOOM);
        camera.request_fit();
        assert!(camera.wheel_zoom.is_none());
        camera.wheel_target(0.32, Vec2::ZERO, Navigation::default(), MAX_ZOOM);
        camera.center_on(Point { x_nm: 0, y_nm: 0 });
        assert!(camera.wheel_zoom.is_none());
    }

    fn scroll(y: f32, command: bool) -> Event {
        let modifiers = if command { egui::Modifiers::COMMAND } else { egui::Modifiers::NONE };
        Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: MouseWheelUnit::Point,
            delta: vec2(0.0, y),
            modifiers,
            phase: egui::TouchPhase::Move,
        }
    }

    #[test]
    fn default_navigation_zooms_with_the_wheel() {
        let input = wheel(&[scroll(50.0, false)], 500.0, Navigation::default());
        assert!(input.zoom > 0.0);
        assert_eq!(input.pan, Vec2::ZERO);
    }

    #[test]
    fn reverse_and_speed_change_wheel_zoom_but_not_pinch() {
        let base = wheel(&[scroll(50.0, false)], 500.0, Navigation::default()).zoom;
        let reversed = Navigation { reverse_zoom: true, ..Navigation::default() };
        assert_eq!(wheel(&[scroll(50.0, false)], 500.0, reversed).zoom, -base);
        let fast = Navigation { zoom_speed_percent: 200, ..Navigation::default() };
        assert!((wheel(&[scroll(50.0, false)], 500.0, fast).zoom - 2.0 * base).abs() < 1e-6);
        let pinch = [Event::Zoom(1.5)];
        assert_eq!(wheel(&pinch, 500.0, reversed).pinch, 1.5_f32.ln());
        assert_eq!(wheel(&pinch, 500.0, reversed).pinch, wheel(&pinch, 500.0, Navigation::default()).pinch);
    }

    #[test]
    fn panning_scroll_moves_the_view_and_ctrl_still_zooms() {
        let pan = Navigation { scroll: ScrollAction::Pan, ..Navigation::default() };
        let moved = wheel(&[scroll(-30.0, false)], 500.0, pan);
        assert_eq!((moved.zoom, moved.pan), (0.0, vec2(0.0, -30.0)));
        assert!(wheel(&[scroll(30.0, true)], 500.0, pan).zoom > 0.0);
    }
    /// Newly displayed panels can settle over several passes. Center once against stable
    /// geometry, while a subsequent explicit camera move cancels the deferred request.
    #[test]
    fn browser_center_waits_for_panel_geometry_and_expires_after_navigation() {
        let mut camera = Camera::default();
        let canvas = Rect::from_min_size(Pos2::ZERO, vec2(700., 800.));
        let first = canvas.shrink2(vec2(220., 100.));
        let settled = first.translate(vec2(0., -3.));
        let target = Point { x_nm: 127_000_000, y_nm: 25_400_000 };
        camera.request_center(target);
        assert!(camera.set_uncovered(canvas, first));
        assert!(camera.set_uncovered(canvas, settled));
        assert!(!camera.set_uncovered(canvas, settled));
        assert_eq!(camera.center(), target);
        camera.request_center(target);
        assert!(camera.set_uncovered(canvas, first));
        camera.center_on(Point { x_nm: 0, y_nm: 0 });
        let before = camera.screen(target, canvas);
        assert!(!camera.set_uncovered(canvas, settled));
        assert_eq!(camera.screen(target, canvas), before);
    }
}
