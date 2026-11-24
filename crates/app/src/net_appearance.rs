//! Net-wide drawing overrides edited in the selected wire's Properties panel.

use crate::{PcbApp, ViewInput, editor_view::DocumentTarget};
use pcbcraft_chrome::{controls, settings, shell, theme::Tokens};
use pcbcraft_engine::{
    nets::{Appearance, ClassTarget, DEFAULT_WIDTH_CP, MAX_PROPERTY_TARGETS, MAX_WIDTH_CP, MIN_WIDTH_CP, SET_APPEARANCE, SET_PROPERTIES},
    selection::Selection,
};
use serde_json::{Value, json};

mod bulk;

/// A wire selection or several net labels can address logical nets in Properties.
pub(super) fn has_net_selection(selection: &Selection) -> bool {
    !selection.wires.is_empty() || selection.net_labels.len() > 1
}

#[derive(Debug)]
enum Fields {
    Single(Edit),
    Bulk(bulk::Edit),
}

/// Resolve the net and its saved override once per selection and project revision.
#[derive(Debug)]
pub(super) struct Inspector {
    source: ViewInput,
    selection: Selection,
    edit: Result<Option<Fields>, String>,
}

#[derive(Debug)]
struct Edit {
    target: ClassTarget,
    saved: Option<Appearance>,
    use_color: bool,
    use_width: bool,
    color: [u8; 3],
    width_cp: u16,
}

impl Edit {
    /// Start with saved channel values or the current presentation defaults.
    fn new(target: ClassTarget, saved: Option<Appearance>) -> Self {
        let color = saved.and_then(|value| value.color);
        let width = saved.and_then(|value| value.width_cp);
        let ink = Tokens::get().schematic_wire;
        Self {
            target,
            saved,
            use_color: color.is_some(),
            use_width: width.is_some(),
            color: color.unwrap_or([ink.r(), ink.g(), ink.b()]),
            width_cp: width.unwrap_or(DEFAULT_WIDTH_CP),
        }
    }

    /// Disabled channels follow the theme without leaving stale overrides in the patch.
    fn appearance(&self) -> Option<Appearance> {
        (self.use_color || self.use_width)
            .then_some(Appearance { color: self.use_color.then_some(self.color), width_cp: self.use_width.then_some(self.width_cp) })
    }

    /// Keep channel clearing and setting in one command and one history step.
    fn patch(&self) -> Value {
        let Some(appearance) = self.appearance() else { return Value::Null };
        let mut patch = json!(appearance);
        let mut clear = Vec::new();
        if self.saved.is_some_and(|saved| saved.color.is_some()) && appearance.color.is_none() {
            clear.push("color");
        }
        if self.saved.is_some_and(|saved| saved.width_cp.is_some()) && appearance.width_cp.is_none() {
            clear.push("width_cp");
        }
        if !clear.is_empty() {
            patch["clear"] = json!(clear);
        }
        patch
    }

    /// Draw the appearance fields alongside the other wire properties.
    fn show(&mut self, ui: &mut egui::Ui) -> Option<Value> {
        ui.separator();
        ui.label("Net appearance");
        ui.horizontal(|ui| {
            controls::checkbox(ui, &mut self.use_color, "Set wire color");
            ui.add_enabled_ui(self.use_color, |ui| settings::color(ui, "Wire color", &mut self.color));
        });
        ui.horizontal(|ui| {
            controls::checkbox(ui, &mut self.use_width, "Set wire width");
            ui.add_enabled_ui(self.use_width, |ui| width_field(ui, &mut self.width_cp));
        });
        if self.appearance() != self.saved {
            let (apply, reset) =
                ui.horizontal(|ui| (controls::button(ui, "Apply changes").clicked(), controls::button(ui, "Reset changes").clicked())).inner;
            if apply {
                return Some(self.patch());
            }
            if reset {
                *self = Self::new(self.target.clone(), self.saved);
            }
        } else if self.saved.is_some() && controls::button(ui, "Use theme defaults").clicked() {
            return Some(Value::Null);
        }
        None
    }
}

/// Edit a displayed percentage while preserving the native stroke-width representation.
fn width_field(ui: &mut egui::Ui, width_cp: &mut u16) {
    let percent_per_cp = 100.0 / f64::from(DEFAULT_WIDTH_CP);
    let mut percent = f64::from(*width_cp) * percent_per_cp;
    let response = ui
        .add(
            egui::DragValue::new(&mut percent)
                .range(f64::from(MIN_WIDTH_CP) * percent_per_cp..=f64::from(MAX_WIDTH_CP) * percent_per_cp)
                .speed(1.0)
                .max_decimals(2)
                .suffix("%"),
        )
        .on_hover_text("100% is the default wire width");
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::SpinButton, ui.is_enabled(), "Wire stroke width"));
    if response.changed() {
        *width_cp = (percent / percent_per_cp).round() as u16;
    }
}

impl Inspector {
    /// Bulk selections edit assignments here and use the Rules window to manage class definitions.
    pub(super) fn is_bulk(&self) -> bool {
        matches!(self.edit, Ok(Some(Fields::Bulk(_))))
    }

    /// Present cached data and carry the captured revision with any requested edit.
    pub(super) fn show(&mut self, ui: &mut egui::Ui) -> Option<(&'static str, Value)> {
        match &mut self.edit {
            Ok(Some(Fields::Single(edit))) => edit
                .show(ui)
                .map(|appearance| (SET_APPEARANCE, json!({"target":edit.target,"appearance":appearance,"expected_revision":self.source.revision}))),
            Ok(Some(Fields::Bulk(edit))) => edit
                .show(ui)
                .map(|changes| (SET_PROPERTIES, json!({"targets":edit.targets,"changes":changes,"expected_revision":self.source.revision}))),
            Ok(None) => None,
            Err(error) => {
                ui.label(shell::dim(error.as_str()));
                None
            }
        }
    }
}

impl PcbApp {
    /// Refresh the inline draft only when its view, revision or actual selection changes.
    pub(super) fn sync_wire_appearance(&mut self) {
        let view = &mut self.views.active;
        let selection = &view.schematic.selection;
        let Some(source) = view.input(&self.session).filter(|_| has_net_selection(selection)) else {
            view.schematic.properties.wire_appearance = None;
            return;
        };
        if view.schematic.properties.wire_appearance.as_ref().is_some_and(|draft| draft.source == source && &draft.selection == selection) {
            return;
        }
        let edit = (|| {
            let project = self.session.project().ok_or("Project unavailable.")?;
            let Some(DocumentTarget::Sheet(sheet)) = view.target() else { return Ok(None) };
            let own = project.for_sheet(sheet).ok_or("Sheet unavailable.")?;
            let wires = own.wires().iter().filter(|wire| selection.wires.contains(&wire.id())).map(|wire| wire.endpoints()[0]);
            let labels = own
                .net_labels()
                .iter()
                .filter(|label| selection.net_labels.contains(&label.object_id()))
                .map(|label| pcbcraft_engine::schematic::WireEndpoint::NetLabel(label.id));
            let mut targets = Vec::new();
            let mut unsupported = false;
            for target in project.net_class_targets_for_endpoints(sheet, wires.chain(labels)).map_err(|error| error.to_string())? {
                match target {
                    Some(target) if !targets.contains(&target) => targets.push(target),
                    Some(_) => {}
                    None => unsupported = true,
                }
                if targets.len() > MAX_PROPERTY_TARGETS {
                    return Err(format!("Select up to {MAX_PROPERTY_TARGETS} nets to edit their properties."));
                }
            }
            if targets.is_empty() {
                return Ok(None);
            }
            if unsupported {
                return Err("Selection includes wires without a supported current net.".to_owned());
            }
            let values = project.net_property_values(&targets).map_err(|error| error.to_string())?;
            if targets.len() == 1 {
                return Ok(Some(Fields::Single(Edit::new(targets.remove(0), values[0].appearance))));
            }
            Ok(Some(Fields::Bulk(bulk::Edit::new(targets, &values, project.net_classes()))))
        })();
        view.schematic.properties.wire_appearance = Some(Inspector { source, selection: selection.clone(), edit });
    }
}
