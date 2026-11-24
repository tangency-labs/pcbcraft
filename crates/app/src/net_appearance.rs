//! Net-wide drawing overrides edited in the selected wire's Properties panel.

use crate::{PcbApp, ViewInput};
use pcbcraft_chrome::{controls, settings, shell, theme::Tokens};
use pcbcraft_engine::{
    nets::{Appearance, ClassTarget, DEFAULT_WIDTH_CP, INSPECT_APPEARANCE, MAX_WIDTH_CP, MIN_WIDTH_CP},
    selection::Selection,
};
use serde_json::{Value, json};

/// Resolve the net and its saved override once per selection and project revision.
#[derive(Debug)]
pub(super) struct Inspector {
    source: ViewInput,
    selection: Selection,
    edit: Result<Option<Edit>, String>,
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
            ui.add_enabled_ui(self.use_width, |ui| {
                let percent_per_cp = 100.0 / f64::from(DEFAULT_WIDTH_CP);
                let mut percent = f64::from(self.width_cp) * percent_per_cp;
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
                    self.width_cp = (percent / percent_per_cp).round() as u16;
                }
            });
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

impl Inspector {
    /// Present cached data and carry the captured revision with any requested edit.
    pub(super) fn show(&mut self, ui: &mut egui::Ui) -> Option<Value> {
        match &mut self.edit {
            Ok(Some(edit)) => {
                edit.show(ui).map(|appearance| json!({"target":edit.target,"appearance":appearance,"expected_revision":self.source.revision}))
            }
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
        let Some(source) = view.input(&self.session).filter(|_| !selection.wires.is_empty()) else {
            view.schematic.properties.wire_appearance = None;
            return;
        };
        if view.schematic.properties.wire_appearance.as_ref().is_some_and(|draft| draft.source == source && &draft.selection == selection) {
            return;
        }
        let edit = (|| {
            let project = self.session.project().ok_or("Project unavailable.")?;
            let target = crate::net_class_editor::selected_target(project, view.target(), selection, &view.board.tracks.selected)?;
            let Some(target) = target else { return Ok(None) };
            let report = self.session.execute(INSPECT_APPEARANCE, &json!({"target":target})).map_err(|error| error.to_string())?;
            let saved = serde_json::from_value(report["appearance"].clone()).map_err(|error| error.to_string())?;
            Ok(Some(Edit::new(target, saved)))
        })();
        view.schematic.properties.wire_appearance = Some(Inspector { source, selection: selection.clone(), edit });
    }
}
