use super::*;
use egui::{Pos2, Sense};
use pcbcraft_doc::{Component, Layer};
use pcbcraft_geom::point_segment;

fn snap(p: Point) -> Point {
    Point::new((p.x * 2.0).round() / 2.0, (p.y * 2.0).round() / 2.0)
}
impl App {
    pub(super) fn canvas(&mut self, ui: &mut egui::Ui) {
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = response.rect;
        let mut camera = if self.tab == Tab::Pcb {
            self.pcb
        } else {
            self.schematic
        };
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.01
                && let Some(mouse) = ui.input(|i| i.pointer.hover_pos())
            {
                let before = camera.world(mouse, rect);
                camera.zoom = (camera.zoom * (scroll * 0.003).exp()).clamp(1.0, 100.0);
                let after = camera.world(mouse, rect);
                camera.center = camera
                    .center
                    .offset(Point::new(before.x - after.x, before.y - after.y));
            }
        }
        if response.dragged_by(egui::PointerButton::Middle) {
            let d = ui.input(|i| i.pointer.delta());
            camera.center = camera.center.offset(Point::new(
                -d.x as f64 / camera.zoom as f64,
                -d.y as f64 / camera.zoom as f64,
            ));
        }
        if self.tab == Tab::Pcb {
            self.pcb = camera;
        } else {
            self.schematic = camera;
        }
        let cursor = response.hover_pos().map(|p| camera.world(p, rect));
        self.cursor = cursor;
        let mut visible = self.project.clone();
        if let Some((id, pos)) = self.drag {
            if let Some(c) = visible.components.iter_mut().find(|c| c.id == id) {
                c.position = pos;
            }
            if let Some(f) = visible.board.footprints.iter_mut().find(|f| f.id == id) {
                f.position = pos;
            }
        }
        painter.rect_filled(rect, 0.0, self.surface());
        if self.preferences.grid {
            let grid = if camera.zoom < 5.0 { 5.0 } else { 1.0 };
            let step = grid * camera.zoom;
            let zero = camera.screen(Point::default(), rect);
            let x0 = rect.left() + (zero.x - rect.left()).rem_euclid(step);
            let y0 = rect.top() + (zero.y - rect.top()).rem_euclid(step);
            let color = self.muted().gamma_multiply(0.13);
            for i in 0..((rect.width() / step) as usize + 1).min(1600) {
                for j in 0..((rect.height() / step) as usize + 1).min(1000) {
                    painter.circle_filled(
                        Pos2::new(x0 + i as f32 * step, y0 + j as f32 * step),
                        0.65,
                        color,
                    );
                }
            }
        }
        if self.tab == Tab::Schematic {
            self.paint_schematic(&painter, rect, camera, &visible);
        } else {
            self.paint_board(&painter, rect, camera, &visible);
        }
        let text = match &self.tool {
            Tool::Select => "SELECT",
            Tool::Place(_) => "PLACE SYMBOL · CLICK TO COMMIT",
            Tool::Wire(None) => "WIRE · PICK FIRST PIN",
            Tool::Wire(Some(_)) => "WIRE · PICK SECOND PIN",
            Tool::Route(None) => "ROUTE · PICK FIRST PAD",
            Tool::Route(Some(_)) => "ROUTE · PICK SECOND PAD",
            Tool::Move => "MOVE · CLICK TO PLACE SELECTION",
            Tool::Cutout => "CUTOUT · CLICK TO PLACE",
        };
        painter.text(
            rect.left_top() + Vec2::new(20.0, 20.0),
            Align2::LEFT_TOP,
            text,
            FontId::monospace(10.0),
            self.muted(),
        );
        if let Some(pos) = cursor {
            let snapped = snap(pos);
            match &self.tool {
                Tool::Place(kind) => {
                    if let Some(c) =
                        pcbcraft_doc::component_template(kind, 1, "preview".into(), snapped)
                    {
                        self.symbol(&painter, rect, camera, &c, false, true);
                    }
                }
                Tool::Cutout if pcbcraft_geom::positive(self.cutout_radius) => {
                    painter.circle_stroke(
                        camera.screen(snapped, rect),
                        self.cutout_radius as f32 * camera.zoom,
                        Stroke::new(1.5, self.accent()),
                    );
                }
                Tool::Wire(Some(r)) => {
                    if let Some(start) = self.project.pin_position(r) {
                        self.path(
                            &painter,
                            rect,
                            camera,
                            start,
                            snapped,
                            Stroke::new(1.5, self.accent()),
                        );
                    }
                }
                Tool::Route(Some((id, pad))) => {
                    if let Some(start) = self
                        .project
                        .footprint(*id)
                        .and_then(|f| f.pad_position(pad))
                    {
                        self.path(
                            &painter,
                            rect,
                            camera,
                            start,
                            snapped,
                            Stroke::new(3.0, Color32::from_rgb(245, 141, 121)),
                        );
                    }
                }
                _ => {}
            }
        }
        if response.drag_started_by(egui::PointerButton::Primary)
            && matches!(self.tool, Tool::Select)
            && let Some(pos) = cursor
        {
            let id = self.pick_object(pos, camera.zoom);
            self.select(id);
            if let Some(id) = id
                && (self.project.component(id).is_some() || self.project.footprint(id).is_some())
            {
                let original = self
                    .project
                    .component(id)
                    .map(|c| c.position)
                    .or_else(|| self.project.footprint(id).map(|f| f.position))
                    .unwrap_or(pos);
                self.drag_offset = Point::new(original.x - pos.x, original.y - pos.y);
                self.drag = Some((id, original));
            }
        }
        if response.dragged_by(egui::PointerButton::Primary)
            && let (Some((id, _)), Some(pos)) = (self.drag, cursor)
        {
            self.drag = Some((id, snap(pos.offset(self.drag_offset))));
        }
        if response.drag_stopped_by(egui::PointerButton::Primary)
            && let Some((id, pos)) = self.drag.take()
        {
            self.run(
                if self.project.footprint(id).is_some() {
                    "footprint.move"
                } else {
                    "component.move"
                },
                json!({"id":id,"position":pos}),
            );
        }
        if response.clicked()
            && let Some(pos) = cursor
        {
            self.canvas_click(pos, camera.zoom);
        }
    }
    fn pick_object(&self, pos: Point, zoom: f32) -> Option<Id> {
        if self.tab == Tab::Schematic {
            self.project
                .components
                .iter()
                .min_by(|a, b| {
                    a.position
                        .distance(pos)
                        .total_cmp(&b.position.distance(pos))
                })
                .filter(|c| c.position.distance(pos) * (zoom as f64) < 35.0)
                .map(|c| c.id)
        } else {
            if let Some(f) = self
                .project
                .board
                .footprints
                .iter()
                .find(|f| f.position.distance(pos) * (zoom as f64) < 45.0)
            {
                return Some(f.id);
            }
            if let Some(c) = self
                .project
                .board
                .cutouts
                .iter()
                .find(|c| (c.center.distance(pos) - c.radius).abs() * (zoom as f64) < 10.0)
            {
                return Some(c.id);
            }
            self.project
                .board
                .tracks
                .iter()
                .find(|t| point_segment(pos, t.start, t.end) * (zoom as f64) < 8.0)
                .map(|t| t.id)
        }
    }
    pub(super) fn select_hover(&mut self) {
        if let Some(pos) = self.cursor {
            let zoom = if self.tab == Tab::Pcb {
                self.pcb.zoom
            } else {
                self.schematic.zoom
            };
            if let Some(id) = self.pick_object(pos, zoom) {
                self.select(Some(id));
            }
        }
    }
    fn pick_pin(&self, pos: Point, zoom: f32) -> Option<PinRef> {
        for c in &self.project.components {
            for pin in &c.pins {
                if c.pin_position(&pin.number)
                    .is_some_and(|p| p.distance(pos) * (zoom as f64) < 12.0)
                {
                    return Some(PinRef {
                        component: c.id,
                        pin: pin.number.clone(),
                    });
                }
            }
        }
        None
    }
    fn pick_pad(&self, pos: Point, zoom: f32) -> Option<(Id, String)> {
        for f in &self.project.board.footprints {
            for pad in &f.pads {
                if f.pad_position(&pad.number).is_some_and(|p| {
                    p.distance(pos) * (zoom as f64)
                        < (pad.diameter as f32 * zoom / 2.0 + 8.0) as f64
                }) {
                    return Some((f.id, pad.number.clone()));
                }
            }
        }
        None
    }
    fn canvas_click(&mut self, pos: Point, zoom: f32) {
        match self.tool.clone() {
            Tool::Select => self.select(self.pick_object(pos, zoom)),
            Tool::Place(kind) => {
                self.run("component.add", json!({"kind":kind,"position":snap(pos)}))
            }
            Tool::Move => {
                if let Some(id) = self.selected {
                    self.run(
                        if self.project.footprint(id).is_some() {
                            "footprint.move"
                        } else {
                            "component.move"
                        },
                        json!({"id":id,"position":snap(pos)}),
                    );
                    self.tool = Tool::Select;
                }
            }
            Tool::Wire(start) => {
                if let Some(pin) = self.pick_pin(pos, zoom) {
                    if let Some(start) = start {
                        self.run("wire.connect", json!({"from":start,"to":pin}));
                        self.tool = Tool::Wire(None);
                    } else {
                        self.tool = Tool::Wire(Some(pin));
                    }
                } else {
                    self.message = "Wire endpoints must be symbol pins".into();
                }
            }
            Tool::Route(start) => {
                if let Some((id, pad)) = self.pick_pad(pos, zoom) {
                    if let Some((from, from_pad)) = start {
                        self.run("track.route",json!({"from":{"footprint":from,"pad":from_pad},"to":{"footprint":id,"pad":pad}}));
                        self.tool = Tool::Route(None);
                    } else {
                        self.tool = Tool::Route(Some((id, pad)));
                    }
                } else {
                    self.message = "Pick a pad to start or finish a route".into();
                }
            }
            Tool::Cutout => self.run(
                "board.cutout.add",
                json!({"center":snap(pos),"radius":self.cutout_radius}),
            ),
        }
    }
    fn path(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        camera: Camera,
        a: Point,
        b: Point,
        stroke: Stroke,
    ) {
        let mid = (a.x + b.x) / 2.0;
        painter.add(egui::Shape::line(
            vec![
                camera.screen(a, rect),
                camera.screen(Point::new(mid, a.y), rect),
                camera.screen(Point::new(mid, b.y), rect),
                camera.screen(b, rect),
            ],
            stroke,
        ));
    }
    fn paint_schematic(&self, painter: &egui::Painter, rect: Rect, camera: Camera, p: &Project) {
        for w in &p.wires {
            if let Some(path) = p.wire_path(w) {
                painter.add(egui::Shape::line(
                    path.iter()
                        .map(|point| camera.screen(*point, rect))
                        .collect(),
                    Stroke::new(1.8, self.accent()),
                ));
                let pair = path.windows(2).max_by(|a, b| {
                    let length = |p: &[Point]| {
                        p.first()
                            .zip(p.get(1))
                            .map(|(a, b)| a.distance(*b))
                            .unwrap_or(0.0)
                    };
                    length(a).total_cmp(&length(b))
                });
                let Some((a, b)) = pair.and_then(|pair| pair.first().zip(pair.get(1))) else {
                    continue;
                };
                let mid = camera.screen(Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0), rect);
                painter.text(
                    mid + Vec2::new(6.0, -7.0),
                    Align2::LEFT_BOTTOM,
                    p.net_name(w.net),
                    FontId::monospace(10.0),
                    self.accent(),
                );
            }
        }
        for c in &p.components {
            self.symbol(painter, rect, camera, c, self.selected == Some(c.id), false);
        }
        if p.components.is_empty() {
            painter.text(
                rect.center() - Vec2::new(0.0, 25.0),
                Align2::CENTER_CENTER,
                "Start your circuit",
                FontId::proportional(28.0),
                self.ink(),
            );
            painter.text(
                rect.center() + Vec2::new(0.0, 20.0),
                Align2::CENTER_CENTER,
                "Press A to add a symbol, or load the LED example below",
                FontId::proportional(14.0),
                self.muted(),
            );
        }
    }
    fn symbol(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        camera: Camera,
        c: &Component,
        selected: bool,
        ghost: bool,
    ) {
        let at = |p: Point| camera.screen(c.position.offset(p.rotate(-c.rotation)), rect);
        let color = if selected || ghost {
            self.accent()
        } else {
            self.ink()
        };
        let stroke = Stroke::new(1.8, color);
        if selected {
            painter.rect_filled(
                Rect::from_center_size(
                    camera.screen(c.position, rect),
                    Vec2::new(13.0 * camera.zoom, 8.0 * camera.zoom),
                ),
                6.0,
                self.accent().gamma_multiply(0.10),
            );
        }
        match c.kind.as_str() {
            "capacitor" => {
                for x in [-0.7, 0.7] {
                    painter.line_segment([at(Point::new(x, -2.0)), at(Point::new(x, 2.0))], stroke);
                }
                for (a, b) in [(-5.0, -0.7), (0.7, 5.0)] {
                    painter.line_segment([at(Point::new(a, 0.0)), at(Point::new(b, 0.0))], stroke);
                }
            }
            "led" => {
                painter.add(egui::Shape::closed_line(
                    vec![
                        at(Point::new(-2.0, -2.0)),
                        at(Point::new(-2.0, 2.0)),
                        at(Point::new(2.0, 0.0)),
                    ],
                    stroke,
                ));
                painter.line_segment(
                    [at(Point::new(2.0, -2.0)), at(Point::new(2.0, 2.0))],
                    stroke,
                );
                for (a, b) in [(-5.0, -2.0), (2.0, 5.0)] {
                    painter.line_segment([at(Point::new(a, 0.0)), at(Point::new(b, 0.0))], stroke);
                }
                for x in [0.0, 1.8] {
                    painter.line_segment(
                        [at(Point::new(x, -2.5)), at(Point::new(x + 1.3, -3.8))],
                        Stroke::new(1.2, color),
                    );
                }
            }
            _ => {
                painter.add(egui::Shape::closed_line(
                    vec![
                        at(Point::new(-3.0, -1.5)),
                        at(Point::new(3.0, -1.5)),
                        at(Point::new(3.0, 1.5)),
                        at(Point::new(-3.0, 1.5)),
                    ],
                    stroke,
                ));
                for (a, b) in [(-5.0, -3.0), (3.0, 5.0)] {
                    painter.line_segment([at(Point::new(a, 0.0)), at(Point::new(b, 0.0))], stroke);
                }
            }
        }
        for pin in &c.pins {
            let position = at(pin.offset);
            painter.circle_filled(position, 3.0, self.surface());
            painter.circle_stroke(
                position,
                3.0,
                Stroke::new(
                    1.2,
                    if pin.net.is_some() {
                        self.accent()
                    } else {
                        color
                    },
                ),
            );
            painter.text(
                position + Vec2::new(0.0, 10.0),
                Align2::CENTER_TOP,
                &pin.number,
                FontId::monospace(9.0),
                self.muted(),
            );
        }
        let center = camera.screen(c.position, rect);
        painter.text(
            center - Vec2::new(0.0, 3.5 * camera.zoom),
            Align2::CENTER_BOTTOM,
            &c.reference,
            FontId::proportional(14.0),
            color,
        );
        if !ghost {
            painter.text(
                center + Vec2::new(0.0, 3.4 * camera.zoom),
                Align2::CENTER_TOP,
                &c.value,
                FontId::proportional(12.0),
                self.muted(),
            );
        }
    }
    fn paint_board(&self, painter: &egui::Painter, rect: Rect, camera: Camera, p: &Project) {
        let board = Rect::from_two_pos(
            camera.screen(Point::default(), rect),
            camera.screen(Point::new(p.board.width, p.board.height), rect),
        );
        painter.rect_filled(
            board,
            0.0,
            if self.preferences.dark {
                Color32::from_rgb(21, 54, 49)
            } else {
                Color32::from_rgb(205, 233, 222)
            },
        );
        painter.rect_stroke(
            board,
            0.0,
            Stroke::new(1.5, self.accent()),
            egui::StrokeKind::Inside,
        );
        for c in &p.board.cutouts {
            let center = camera.screen(c.center, rect);
            painter.circle_filled(center, c.radius as f32 * camera.zoom, self.surface());
            painter.circle_stroke(
                center,
                c.radius as f32 * camera.zoom,
                Stroke::new(1.0, self.accent()),
            );
        }
        if let Some(airwires) = self.checks.get("airwires").and_then(Value::as_array) {
            for wire in airwires {
                if let (Some(a), Some(b)) = (
                    wire.get("from")
                        .and_then(|v| serde_json::from_value::<Point>(v.clone()).ok()),
                    wire.get("to")
                        .and_then(|v| serde_json::from_value::<Point>(v.clone()).ok()),
                ) {
                    painter.line_segment(
                        [camera.screen(a, rect), camera.screen(b, rect)],
                        Stroke::new(1.0, Color32::from_rgb(126, 153, 161)),
                    );
                }
            }
        }
        for t in &p.board.tracks {
            painter.line_segment(
                [camera.screen(t.start, rect), camera.screen(t.end, rect)],
                Stroke::new(
                    (t.width as f32 * camera.zoom).max(1.0),
                    if self.selected == Some(t.id) {
                        self.accent()
                    } else if t.layer == Layer::Front {
                        Color32::from_rgb(245, 141, 121)
                    } else {
                        Color32::from_rgb(116, 167, 236)
                    },
                ),
            );
        }
        for f in &p.board.footprints {
            let center = camera.screen(f.position, rect);
            let color = if self.selected == Some(f.id) {
                self.accent()
            } else {
                self.ink().gamma_multiply(0.7)
            };
            painter.rect_stroke(
                Rect::from_center_size(center, Vec2::new(7.5 * camera.zoom, 3.0 * camera.zoom)),
                3.0,
                Stroke::new(1.0, color),
                egui::StrokeKind::Inside,
            );
            for pad in &f.pads {
                if let Some(pos) = f.pad_position(&pad.number) {
                    let center = camera.screen(pos, rect);
                    painter.circle_filled(
                        center,
                        pad.diameter as f32 / 2.0 * camera.zoom,
                        Color32::from_rgb(230, 194, 125),
                    );
                    painter.text(
                        center,
                        Align2::CENTER_CENTER,
                        &pad.number,
                        FontId::monospace(10.0),
                        Color32::from_rgb(48, 44, 32),
                    );
                }
            }
            if let Some(c) = p.component(f.component) {
                painter.text(
                    center - Vec2::new(0.0, 2.6 * camera.zoom),
                    Align2::CENTER_BOTTOM,
                    &c.reference,
                    FontId::proportional(14.0),
                    color,
                );
                painter.text(
                    center + Vec2::new(0.0, 2.6 * camera.zoom),
                    Align2::CENTER_TOP,
                    &c.value,
                    FontId::proportional(11.0),
                    color,
                );
            }
        }
        painter.text(
            board.left_bottom() + Vec2::new(0.0, 18.0),
            Align2::LEFT_TOP,
            format!(
                "{} × {} mm    F.Cu / B.Cu    clearance {} mm",
                p.board.width, p.board.height, p.board.clearance
            ),
            FontId::monospace(11.0),
            self.muted(),
        );
    }
}
