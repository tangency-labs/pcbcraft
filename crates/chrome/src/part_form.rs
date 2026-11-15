//! Part documents compose existing Craft fields and editable table primitives.
use egui::Ui;
use solidcraft_ui_kit as kit;

/// Bound the document's reading width inside a large workspace or maximized Items region.
/// Calls the host for its scrolling body (`false`) and then fixed footer (`true`), using
/// the kit's measured catalog layout. Field/table adapters retain their narrow layout.
pub fn document(ui: &mut Ui, mut add: impl FnMut(&mut Ui, bool)) {
    let width = ui.available_width().min(760.);
    ui.allocate_ui_with_layout(egui::vec2(width, ui.available_height()), egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.set_min_width(width);
        ui.set_max_width(width);
        let layout = kit::CatalogLayout::new(ui, "part_document_fields", 1);
        layout.body(ui, |ui| add(ui, false));
        layout.footer(ui, |ui| add(ui, true));
    });
}

/// Use the shared primary action band after the fields; it stays outside the scrolling body.
/// An empty Cancel label omits that action because document closing belongs to the workspace.
pub fn publication(ui: &mut Ui, status: &str, enabled: bool) -> bool {
    kit::footer(ui, "Publish…", "", enabled, kit::Footer { status: Some(status), confirm: true, ..Default::default() }) == kit::Action::Ok
}

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
        ui.allocate_ui_with_layout(egui::vec2(width, crate::Tokens::get().control_height), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add(egui::Label::new(value).truncate()).on_hover_text(value);
        });
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
pub fn reference_results(ui: &mut Ui, rows: &[crate::library_workspace::TableEntry], reveal: bool, empty: &str) -> Option<usize> {
    let columns = [
        kit::Column::new("Name", 2.).flex().min_content_width(160.),
        kit::Column::new("Library", 1.).flex().min_content_width(95.),
        kit::Column::new("Rev", 0.5).flex().min_content_width(48.),
    ];
    let mut picked = None;
    kit::table_custom(ui, "reference_results", &columns, empty, |ui, table| {
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

/// Library browsing stays available beside the pinned kit's document tabs. The host maps
/// tab indices to stable document IDs and owns selection, closing, creation and ordering.
pub fn documents(ui: &mut Ui, documents: &[(String, bool, String)], selected: Option<usize>) -> Vec<DocumentAction> {
    use kit::app_bar::{self, DocTab, TabAction, TabHandles, TabStrip};
    let t = crate::Tokens::get();
    // These document tabs are embedded in a workspace, whose background differs from the
    // title bar. Bind the kit's tab ink/surfaces to that host palette without changing menus.
    let mut palette = solidcraft_theme::Tokens::get();
    palette.app_bar_text = t.text;
    palette.app_bar_tab_active = t.selected_bg;
    palette.app_bar_hover = t.hover;
    palette.app_bar_hover_strong = t.hover;
    let _palette = solidcraft_theme::scoped_panel_tokens(palette);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), t.control_height + t.space_8), egui::Sense::hover());
    let browse = egui::Rect::from_min_size(rect.min, egui::vec2(94., rect.height()));
    let mut actions = Vec::new();
    if kit::button_at(ui, "browse_library", browse, "Browse", if selected.is_none() { kit::Button::SmallSelected } else { kit::Button::Ghost })
        .accessible_name("Browse library")
        .clicked()
    {
        actions.push(DocumentAction::Browse);
    }
    let mut strip = TabStrip {
        tabs: documents.iter().map(|(name, dirty, place)| DocTab { name, list_name: name, dirty: *dirty, place }).collect(),
        selected: selected.unwrap_or(0),
        shown_selected: selected,
        title_bar: false,
        double_click_renames: false,
        rename: None,
        menu: &[app_bar::TabMenuRow { id: "close", label: "Close", key_on_selected: "", needs_others: false }],
        close_name: "Close document",
        close_tip: "Close document",
        more_tip: "Open part documents",
        new_tip: "New part document",
        id: ui.make_persistent_id("part_document_tabs"),
        handles: TabHandles {
            tab: "part-tab:",
            close: "part-close:",
            more: "part-tabs-more",
            new: "part-tab-new",
            rename: "part-tab-rename",
            menu_row: "part-tab-menu:",
            more_row: "part-tab-more:",
        },
    };
    let events = app_bar::tabs(ui, &mut strip, rect, browse.right() + t.space_8, rect.right());
    for (index, (name, _, _)) in documents.iter().enumerate() {
        if let Some(response) = ui.ctx().read_response(ui.scope_id().with(("doc_tab", index))) {
            response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, ui.is_enabled(), selected == Some(index), name));
            crate::automation::publish(ui, &response, format!("part-tab:{index}"));
        }
    }
    for action in events {
        match action {
            TabAction::Select(index) => actions.push(DocumentAction::Select(index)),
            TabAction::Close(index) => actions.push(DocumentAction::Close(index)),
            TabAction::Menu { tab, id } if id == "close" => actions.push(DocumentAction::Close(tab)),
            TabAction::Reorder { from, to } => actions.push(DocumentAction::Reorder { from, to }),
            TabAction::New => actions.push(DocumentAction::New),
            _ => {}
        }
    }
    actions
}
/// Presentation events retain no document state and never discard an edit themselves.
pub enum DocumentAction {
    Browse,
    Select(usize),
    Close(usize),
    Reorder { from: usize, to: usize },
    New,
}
