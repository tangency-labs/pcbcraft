//! A compact, read-only schematic view for Home. It reuses the canvas symbol painter and
//! engine wire paths; egui remains the renderer, with no second rasterizer or graphics device.

use super::*;

/// Bound cached geometry and per-frame paint work independently of the full project limits.
const MAX_POINTS: usize = 100_000;

#[derive(Debug)]
struct Symbol {
    placement: SymbolPlacement,
    reference: String,
    value: String,
    metadata: pcbcraft_engine::schematic::fields::Metadata,
}

/// Disposable geometry from a saved snapshot, with no session, selection, camera or undo state.
#[derive(Debug)]
pub(crate) struct Thumbnail {
    symbols: Vec<Symbol>,
    wires: Vec<Vec<Pos2>>,
    blocks: Vec<(Rect, String)>,
    labels: Vec<(Pos2, String, pcbcraft_engine::hierarchy::Direction)>,
    bounds: Rect,
}

impl Thumbnail {
    /// Prepare on a worker, keeping pinned definitions and exact authored routes. Oversized
    /// drawings yield no preview instead of silently truncating or monopolizing the UI thread.
    pub(crate) fn new(project: &Project) -> Option<Self> {
        let mut points = 0;
        let mut bounds = Rect::NOTHING;
        let mut symbols = Vec::new();
        for symbol in project.symbols() {
            points += symbol
                .definition
                .graphics()
                .iter()
                .map(|shape| match shape {
                    Shape::Graphic(graphic) => graphic.paths().iter().map(Vec::len).sum(),
                    Shape::Polyline(points) => points.len(),
                    _ => 4,
                })
                .sum::<usize>();
            if points > MAX_POINTS {
                return None;
            }
            bounds = bounds.union(grid_bounds(&symbol.definition, symbol.turns()).translate(grid_pos(symbol.position).to_vec2()));
            let metadata = project.component(symbol)?.metadata.clone();
            points += metadata.fields.values().map(|f| f.value.len()).sum::<usize>();
            if points > MAX_POINTS {
                return None;
            }
            symbols.push(Symbol {
                placement: symbol.clone(),
                reference: project.reference(symbol),
                value: project.value(symbol).to_owned(),
                metadata,
            });
        }
        let mut wires = Vec::new();
        for wire in project.wires() {
            let path = project.wire_path(wire);
            points += path.len();
            if points > MAX_POINTS {
                return None;
            }
            let path: Vec<_> = path.into_iter().map(grid_pos).collect();
            for point in &path {
                bounds = bounds.union(Rect::from_pos(*point));
            }
            wires.push(path);
        }
        let blocks = project
            .sheet_symbols()
            .iter()
            .map(|block| {
                let size = project.sheet_symbol_size(block);
                let rect =
                    Rect::from_min_max(grid_pos(block.at), grid_pos(Point { x_nm: block.at.x_nm + size.x_nm, y_nm: block.at.y_nm + size.y_nm }));
                bounds = bounds.union(rect);
                (rect, project.child_name(block).to_owned())
            })
            .collect();
        let labels = project
            .hierarchical_labels()
            .iter()
            .map(|label| {
                let at = grid_pos(label.at);
                bounds = bounds.union(Rect::from_pos(at));
                (at, label.name.clone(), label.direction)
            })
            .collect();
        Some(Self { symbols, wires, blocks, labels, bounds })
    }

    /// An empty saved schematic has its own placeholder rather than pretending a load failed.
    pub(crate) fn is_empty(&self) -> bool {
        self.symbols.is_empty() && self.wires.is_empty() && self.blocks.is_empty() && self.labels.is_empty()
    }

    /// Fit the saved geometry in the card and paint with the active theme. Canvas labels follow
    /// the existing zoom threshold; library strokes, rotations and wire corners use the same data.
    pub(crate) fn paint(&self, painter: &Painter, rect: Rect) {
        if self.is_empty() || !rect.is_positive() {
            return;
        }
        let t = Tokens::get();
        let target = rect.shrink(t.space_8);
        let bounds = self.bounds.expand(theme::SCHEMATIC_FIT_MARGIN / theme::SCHEMATIC_GRID_SPACING);
        let step = (target.width() / bounds.width()).min(target.height() / bounds.height()).min(theme::SCHEMATIC_PREVIEW_MAX_STEP);
        let screen = |point: Pos2| target.center() + (point - bounds.center()) * step;
        let painter = painter.with_clip_rect(painter.clip_rect().intersect(rect));
        for path in &self.wires {
            painter
                .add(egui::Shape::line(path.iter().copied().map(screen).collect(), Stroke::new(theme::SCHEMATIC_PREVIEW_STROKE, t.schematic_wire)));
        }
        for (bounds, name) in &self.blocks {
            let bounds = Rect::from_min_max(screen(bounds.min), screen(bounds.max));
            painter.rect_stroke(bounds, 0, Stroke::new(theme::SCHEMATIC_PREVIEW_STROKE, t.text), egui::StrokeKind::Middle);
            if step >= theme::SCHEMATIC_MIN_GRID_SPACING {
                painter.text(bounds.left_top() - egui::vec2(0., t.space_4), Align2::LEFT_BOTTOM, name, FontId::proportional(t.font_small), t.text);
            }
        }
        if step >= theme::SCHEMATIC_MIN_GRID_SPACING {
            for (at, name, direction) in &self.labels {
                crate::hierarchy::paint_label(&painter, screen(*at), name, *direction, t.schematic_wire);
            }
        }
        for symbol in &self.symbols {
            let placement = &symbol.placement;
            paint_symbol(
                &painter,
                &placement.definition,
                screen(grid_pos(placement.position)),
                step,
                placement.turns(),
                Stroke::new(theme::SCHEMATIC_PREVIEW_STROKE, t.text),
            );
            fields::paint_labels(
                &painter,
                placement,
                fields::Content { reference: &symbol.reference, value: &symbol.value, metadata: &symbol.metadata },
                screen(grid_pos(placement.position)),
                step,
                t.text,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pcbcraft_engine::Session;
    use serde_json::json;

    /// Snapshot preparation preserves every authored route and rotated symbol without touching
    /// the project; an empty document remains a valid, distinguishable empty preview.
    #[test]
    fn thumbnail_uses_saved_geometry_without_editor_state() {
        let project = pcbcraft_engine::persistence::decode(include_bytes!("../../tests/fixtures/two-resistors.pc")).unwrap();
        let preview = Thumbnail::new(&project).unwrap();
        assert_eq!(preview.symbols.len(), project.symbols().len());
        assert_eq!(preview.wires[0], project.wire_path(&project.wires()[0]).into_iter().map(grid_pos).collect::<Vec<_>>());
        assert!(preview.bounds.is_finite());
        let mut session = Session::default();
        session.apply("project.new", &json!({"name":"Empty"})).unwrap();
        assert!(Thumbnail::new(session.project().unwrap()).unwrap().is_empty());
        session
            .apply(pcbcraft_engine::hierarchy::CREATE_CHILD, &json!({"name":"Child","x_nm":0,"y_nm":0,"expected_revision":session.revision()}))
            .unwrap();
        let preview = Thumbnail::new(session.project().unwrap()).unwrap();
        assert!(!preview.is_empty());
        assert!(preview.bounds.is_finite());
        assert_eq!(preview.blocks[0].1, "Child");
    }

    /// Home previews use saved visibility and optional field content rather than reconstructing
    /// reference/value labels from the library's automatic layout.
    #[test]
    fn thumbnail_respects_saved_field_visibility_and_metadata() {
        use pcbcraft_engine::schematic::fields::{Field, Layout, Metadata};
        let project = pcbcraft_engine::persistence::decode(include_bytes!("../../tests/fixtures/two-resistors.pc")).unwrap();
        let mut session = Session::default();
        session.open_snapshot(project);
        let mut metadata = Metadata::default();
        metadata.fields.insert("manufacturer".into(), Field { name: "Manufacturer".into(), value: "ACME".into() });
        session.apply(pcbcraft_engine::SET_SYMBOL_FIELDS, &json!({"symbol":"symbol-2","reference":"R1","value":"1k","metadata":metadata,
            "field_layout":{"reference":Layout{visible:false,..Layout::default()},"manufacturer":Layout{offset:Some(Point{x_nm:0,y_nm:5_080_000}),..Layout::default()}},"expected_revision":session.revision()})).unwrap();
        let preview = Thumbnail::new(session.project().unwrap()).unwrap();
        let ctx = egui::Context::default();
        pcbcraft_chrome::fonts::install(&ctx);
        ctx.begin_pass(egui::RawInput::default());
        preview.paint(&ctx.layer_painter(egui::LayerId::background()), Rect::from_min_size(Pos2::ZERO, vec2(1200., 800.)));
        let mut output = ctx.end_pass();
        output.textures_delta.clear();
        let text: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.job.text.as_str()),
                _ => None,
            })
            .collect();
        assert!(!text.contains(&"R1"));
        assert!(text.contains(&"R2"));
        assert!(text.contains(&"ACME"));
    }
}

#[cfg(test)]
mod native_power_tests {
    use super::*;
    use pcbcraft_engine::{
        PLACE_SYMBOL, Session,
        persistence::{decode_documents, encode_documents},
    };
    use serde_json::json;

    /// Recent previews render pinned native power bodies and Value fields through the same
    /// symbol painter as other components, including when the source catalog is absent.
    #[test]
    fn saved_native_power_symbols_render_in_the_common_thumbnail_path() {
        let mut s = Session::default();
        s.apply("project.new", &json!({})).unwrap();
        for (id, x) in [("gnd", 0), ("vcc-bar", 8 * GRID_NM)] {
            let d = s.libraries().latest_symbol("power", id).unwrap();
            s.apply(PLACE_SYMBOL,&json!({"library":"power","definition":id,"definition_revision":d.revision(),"definition_digest":pcbcraft_engine::library::sources::symbol_digest(&d).unwrap(),"x_nm":x,"y_nm":0,"expected_revision":s.revision()})).unwrap();
        }
        let docs = encode_documents(s.snapshot().unwrap()).unwrap();
        let pinned = decode_documents(&docs).unwrap();
        let preview = Thumbnail::new(&pinned).unwrap();
        assert_eq!(preview.symbols.len(), 2);
        assert!(pinned.net_labels().is_empty());
        let ctx = egui::Context::default();
        pcbcraft_chrome::fonts::install(&ctx);
        ctx.begin_pass(egui::RawInput::default());
        preview.paint(&ctx.layer_painter(egui::LayerId::background()), Rect::from_min_size(Pos2::ZERO, vec2(300., 180.)));
        let mut out = ctx.end_pass();
        out.textures_delta.clear();
        let text: Vec<_> = out
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.text()),
                _ => None,
            })
            .collect();
        assert!(text.contains(&"GND"));
        assert!(text.contains(&"VCC"));
        assert!(!text.iter().any(|t| t.starts_with("PWR")));
        assert_eq!(out.shapes.iter().filter(|s| matches!(s.shape, egui::Shape::LineSegment { .. })).count(), 6);
        assert_eq!(encode_documents(pinned).unwrap(), docs);
    }
}
