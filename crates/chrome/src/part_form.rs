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
