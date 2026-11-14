//! Part documents compose existing Craft fields and editable table primitives.
use egui::Ui;
use solidcraft_ui_kit as kit;

/// Keep pad identities and terminal choices aligned with the app's shared selectable tables.
/// The host owns mapping semantics, exact identities and selection.
pub fn mapping(ui: &mut Ui, pads: &[String], selected: &mut Option<String>, mut edit: impl FnMut(&mut Ui, &str)) {
    let columns =
        [kit::Column::new("Footprint pad", 1.).flex().min_content_width(90.), kit::Column::new("Symbol terminal", 2.).flex().min_content_width(160.)];
    kit::table_custom(ui, "part_pin_map", &columns, "No footprint pads", |ui, table| {
        table.headers(ui, |_, _| false);
        for pad in pads {
            if table
                .row_visible(ui, pad, selected.as_ref() == Some(pad), false, |row| {
                    row.cell(|ui| {
                        ui.label(pad);
                    });
                    row.cell(|ui| edit(ui, pad));
                })
                .clicked()
            {
                *selected = Some(pad.clone());
            }
        }
    });
}

/// Reference rows show the current association; the shared Browse action opens a searchable
/// chooser. Optional removal is explicit and never inferred from an empty search result.
pub fn reference(ui: &mut Ui, label: &str, value: &str, clear: bool) -> ReferenceAction {
    let mut action = ReferenceAction::None;
    crate::properties::field(ui, label, |ui| {
        let reserved = if clear { 162. } else { 98. };
        let width = (ui.available_width() - reserved).max(0.);
        ui.add_sized(egui::vec2(width, crate::Tokens::get().control_height), egui::Label::new(value).truncate()).on_hover_text(value);
        let response = crate::controls::bounded_button(ui, ("browse", label), "Browse…", 86., true);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), format!("Choose {}", label.to_lowercase())));
        if response.clicked() {
            action = ReferenceAction::Browse;
        }
        if clear {
            let response = crate::controls::bounded_button(ui, ("clear", label), "Clear", 54., true);
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), format!("Clear {}", label.to_lowercase())));
            if response.clicked() {
                action = ReferenceAction::Clear;
            }
        }
    });
    action
}
/// Only user actions leave the reference row; data and validation stay in the host.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ReferenceAction {
    None,
    Browse,
    Clear,
}

/// Use the same search field as catalog browsing, with a kind-specific accessible name.
pub fn search(ui: &mut Ui, query: &mut String, placeholder: &str) -> egui::Response {
    let response = kit::search_field(ui, ui.make_persistent_id("part_reference_search"), query, placeholder);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, ui.is_enabled(), "Search library items"));
    response
}
/// A chooser retains the shared modal frame and fixed Choose/Cancel footer around bounded content.
pub fn chooser(ctx: &egui::Context, title: &str, enabled: bool, add: impl FnOnce(&mut Ui, f32)) -> crate::catalog::FormAction {
    crate::catalog::property_form_enabled(ctx, "part_reference_picker", title, 1080., "Choose", enabled, |ui| {
        let t = crate::Tokens::get();
        let reserved = t.control_height * 8. + t.space_16 * 6. + t.form_footer_height;
        let height = (ctx.content_rect().height() - reserved).clamp(100., 560.);
        add(ui, height);
    })
}
/// Results and preview share two columns when space allows, otherwise use the same compact
/// region choice as Parts Library. Both regions clip and scroll within the chooser's fixed body.
pub fn chooser_body(ui: &mut Ui, height: f32, compact: &mut usize, mut draw: impl FnMut(&mut Ui, bool)) {
    let width = ui.available_width();
    if width < 760. {
        kit::segmented(ui, compact, &["Results", "Preview"], kit::Segments { small: true, ..Default::default() });
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
        ui.scope_builder(egui::UiBuilder::new().id_salt("reference_compact").max_rect(rect), |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(rect));
            egui::ScrollArea::vertical().id_salt("reference_compact_scroll").auto_shrink([false, false]).show(ui, |ui| draw(ui, *compact == 1));
        });
    } else {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
        let gap = crate::Tokens::get().space_16;
        let split = rect.left() + rect.width() * 0.57;
        for (preview, region) in [
            (false, egui::Rect::from_min_max(rect.min, egui::pos2(split - gap / 2., rect.bottom()))),
            (true, egui::Rect::from_min_max(egui::pos2(split + gap / 2., rect.top()), rect.max)),
        ] {
            ui.scope_builder(egui::UiBuilder::new().id_salt(("reference_region", preview)).max_rect(region), |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(region));
                egui::ScrollArea::vertical().id_salt(("reference_scroll", preview)).auto_shrink([false, false]).show(ui, |ui| draw(ui, preview));
            });
        }
    }
}

/// Compose the shared selectable table, revealing keyboard selections through the kit's
/// explicit row-reveal contract instead of constructing an unbounded list of widgets.
pub fn reference_results(ui: &mut Ui, rows: &[crate::library_workspace::TableEntry], reveal: bool) -> Option<usize> {
    let columns = [
        kit::Column::new("Name", 2.).flex().min_content_width(160.),
        kit::Column::new("Library", 1.).flex().min_content_width(95.),
        kit::Column::new("Rev", 0.5).flex().min_content_width(48.),
    ];
    let mut picked = None;
    kit::table_custom(ui, "reference_results", &columns, "No matching items", |ui, table| {
        table.headers(ui, |_, _| false);
        for (index, entry) in rows.iter().enumerate() {
            let response = table.row_visible(ui, &entry.key, entry.selected, entry.selected && reveal, |row| {
                for cell in &entry.cells {
                    row.cell(|ui| {
                        ui.add(egui::Label::new(cell).selectable(false).truncate()).on_hover_text(cell);
                    });
                }
            });
            if response.clicked() {
                picked = Some(index);
            }
            if entry.selected && reveal {
                table.reveal_row(response.rect);
            }
        }
    });
    picked
}
