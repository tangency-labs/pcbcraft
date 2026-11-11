//! Sourced Tg/Dk/Df editing retains immutable material revisions and report conditions.
use super::*;
use pcbcraft_engine::board::materials::{DielectricReport, DielectricValueKind, MeasurementAxis, ReportBasis, Source, TgMethod, TgReport};

/// One new material or revision; existing reports survive while new characterization is appended.
#[derive(Debug)]
pub(super) struct Edit {
    material: Material,
    name: String,
    family: String,
    manufacturer: String,
    product: String,
    tg: String,
    dk: String,
    df: String,
    frequency: String,
    temperature: String,
    method: String,
    document: String,
    revision: String,
    locator: String,
    specimen_glass: String,
    resin: String,
    specimen_thickness: String,
    tg_method: Option<TgMethod>,
    basis: Option<ReportBasis>,
    value_kind: Option<DielectricValueKind>,
    axis: Option<MeasurementAxis>,
}

/// Blank optional text remains absent rather than becoming a misleading empty specification.
fn text(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

/// Convert exact authored decimals to integer fixed-point units without rounding away digits.
pub(super) fn decimal(value: &str, scale: u32) -> Result<Option<i64>, String> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    let value = value.trim();
    let negative = value.starts_with('-');
    let value = value.strip_prefix('-').or_else(|| value.strip_prefix('+')).unwrap_or(value);
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if (whole.is_empty() && fraction.is_empty())
        || !whole.chars().all(|c| c.is_ascii_digit())
        || !fraction.chars().all(|c| c.is_ascii_digit())
        || fraction.len() > scale as usize
    {
        return Err(format!("Enter a decimal with at most {scale} fractional digits"));
    }
    let multiplier = 10_i64.pow(scale);
    let whole = if whole.is_empty() { 0 } else { whole.parse::<i64>().map_err(|_| "Value is out of range")? };
    let fraction = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<i64>().map_err(|_| "Value is out of range")? * 10_i64.pow(scale - fraction.len() as u32)
    };
    let value = whole.checked_mul(multiplier).and_then(|n| n.checked_add(fraction)).ok_or("Value is out of range")?;
    Ok(Some(if negative { -value } else { value }))
}

impl Edit {
    /// A new revision retains the old identity and reports; adoption never rewrites a pinned key.
    fn new(mut material: Material, revise: bool) -> Result<Self, String> {
        if revise {
            material.revision = material.revision.checked_add(1).ok_or("Material revision limit reached")?;
        }
        Ok(Self {
            name: material.name.clone(),
            family: material.family.clone().unwrap_or_default(),
            manufacturer: material.manufacturer.clone().unwrap_or_default(),
            product: material.product.clone().unwrap_or_default(),
            material,
            tg: String::new(),
            dk: String::new(),
            df: String::new(),
            frequency: String::new(),
            temperature: String::new(),
            method: String::new(),
            document: String::new(),
            revision: String::new(),
            locator: String::new(),
            specimen_glass: String::new(),
            resin: String::new(),
            specimen_thickness: String::new(),
            tg_method: None,
            basis: None,
            value_kind: None,
            axis: None,
        })
    }

    /// Reports require their actual source; no frequency, Dk/Df, Tg or qualification is guessed.
    fn value(&self) -> Result<Material, String> {
        let mut material = self.material.clone();
        material.name = self.name.clone();
        material.family = text(&self.family);
        material.manufacturer = text(&self.manufacturer);
        material.product = text(&self.product);
        let source = Source { document: self.document.clone(), revision: text(&self.revision), locator: text(&self.locator) };
        let source = if self.document.is_empty() { None } else { Some(source) };
        if source.is_some() {
            material.source = source.clone();
        }
        if let Some(value) = decimal(&self.tg, 3)? {
            material.tg.push(TgReport {
                temperature_milli_c: value,
                method: self.tg_method.ok_or("Choose the reported Tg method")?,
                basis: self.basis.ok_or("Choose the report basis")?,
                test_standard: text(&self.method),
                source: source.clone().ok_or("A Tg report requires a source document")?,
            });
        }
        let dk = decimal(&self.dk, 6)?.map(u32::try_from).transpose().map_err(|_| "Dk is out of range")?;
        let df = decimal(&self.df, 6)?.map(u32::try_from).transpose().map_err(|_| "Df is out of range")?;
        if dk.is_some() || df.is_some() {
            let frequency = if self.frequency.trim().is_empty() {
                None
            } else {
                Some(self.frequency.trim().parse::<u64>().map_err(|_| "Frequency must be a whole number of Hz")?)
            };
            material.dielectric.push(DielectricReport {
                dk_millionths: dk,
                df_millionths: df,
                frequency_hz: frequency,
                method: text(&self.method),
                value_kind: self.value_kind,
                basis: self.basis.ok_or("Choose the report basis")?,
                source: source.ok_or("A Dk/Df report requires a source document")?,
                temperature_milli_c: decimal(&self.temperature, 3)?,
                axis: self.axis,
                glass_construction: text(&self.specimen_glass),
                resin_content_ppm: decimal(&self.resin, 4)?.map(u32::try_from).transpose().map_err(|_| "Resin content is out of range")?,
                specimen_thickness_nm: if self.specimen_thickness.trim().is_empty() {
                    None
                } else {
                    Some(units::parse_length(&self.specimen_thickness, units::Unit::Mm).map_err(|e| e.to_string())?)
                },
            });
        }
        Ok(material)
    }

    /// Basic identity remains concise; sourced characterization and test conditions expand only on request.
    fn show(&mut self, ui: &mut Ui) {
        form::text(ui, "Material name", &mut self.name);
        form::text(ui, "Family", &mut self.family);
        form::text(ui, "Manufacturer", &mut self.manufacturer);
        form::text(ui, "Product", &mut self.product);
        ui.collapsing("Technical properties", |ui| {
            form::text(ui, "Tg (°C)", &mut self.tg);
            form::field(ui, "Tg method", |ui| {
                egui::ComboBox::from_id_salt("tg_method")
                    .selected_text(match self.tg_method {
                        Some(TgMethod::Dsc) => "DSC",
                        Some(TgMethod::Tma) => "TMA",
                        Some(TgMethod::Dma) => "DMA",
                        None => "Choose method",
                    })
                    .show_ui(ui, |ui| {
                        for (value, name) in [(TgMethod::Dsc, "DSC"), (TgMethod::Tma, "TMA"), (TgMethod::Dma, "DMA")] {
                            ui.selectable_value(&mut self.tg_method, Some(value), name);
                        }
                    })
                    .response
                    .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Tg method"));
            });
            form::text(ui, "Dk", &mut self.dk);
            form::text(ui, "Df", &mut self.df);
            form::text(ui, "Frequency (Hz)", &mut self.frequency);
            form::field(ui, "Basis", |ui| {
                egui::ComboBox::from_id_salt("report_basis")
                    .selected_text(self.basis.map(|b| format!("{b:?}")).unwrap_or_else(|| "Choose basis".into()))
                    .show_ui(ui, |ui| {
                        for value in [ReportBasis::Typical, ReportBasis::Minimum, ReportBasis::Maximum, ReportBasis::Measured] {
                            ui.selectable_value(&mut self.basis, Some(value), format!("{value:?}"));
                        }
                    })
                    .response
                    .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Basis"));
            });
            form::text(ui, "Source document", &mut self.document);
            form::text(ui, "Source revision", &mut self.revision);
            form::text(ui, "Source locator", &mut self.locator);
            ui.collapsing("Measurement conditions", |ui| {
                form::text(ui, "Test method / standard", &mut self.method);
                form::text(ui, "Temperature (°C)", &mut self.temperature);
                form::text(ui, "Specimen glass construction", &mut self.specimen_glass);
                form::text(ui, "Specimen resin (%)", &mut self.resin);
                form::text(ui, "Specimen thickness (mm)", &mut self.specimen_thickness);
                form::field(ui, "Dk/Df value", |ui| {
                    egui::ComboBox::from_id_salt("value_kind")
                        .selected_text(match self.value_kind {
                            Some(DielectricValueKind::Design) => "Design",
                            Some(DielectricValueKind::Test) => "Test",
                            None => "Unspecified",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.value_kind, None, "Unspecified");
                            ui.selectable_value(&mut self.value_kind, Some(DielectricValueKind::Design), "Design");
                            ui.selectable_value(&mut self.value_kind, Some(DielectricValueKind::Test), "Test");
                        });
                });
                form::field(ui, "Axis", |ui| {
                    egui::ComboBox::from_id_salt("report_axis")
                        .selected_text(self.axis.map(|a| format!("{a:?}")).unwrap_or_else(|| "Unspecified".into()))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.axis, None, "Unspecified");
                            for axis in [MeasurementAxis::X, MeasurementAxis::Y, MeasurementAxis::Z] {
                                ui.selectable_value(&mut self.axis, Some(axis), format!("{axis:?}"));
                            }
                        });
                });
            });
        });
    }
}

impl super::Edit {
    /// Adopt a fully validated material into the disposable board draft, retaining the form on
    /// error. The project/history are unchanged until the later Apply board setup action.
    pub(super) fn save_material(&mut self) -> Result<(), String> {
        let candidate = &self.candidate;
        let material = self.material_edit.as_ref().ok_or("No material is being edited")?.value()?;
        let mut value = serde_json::to_value(candidate.value(self.unit)?).map_err(|e| e.to_string())?;
        let mut materials = candidate.materials.clone();
        materials.push(material);
        value["materials"] = json!(materials);
        let setup = serde_json::from_value::<BoardSetup>(value).map_err(|e| e.to_string())?;
        self.candidate = Candidate::new(setup, self.unit);
        self.material_edit = None;
        Ok(())
    }

    /// Browse all pinned reports, or stage a new material/revision. Committing the overall form
    /// is still the sole model transaction; report validation cannot partially adopt material data.
    pub(super) fn materials(&mut self, ui: &mut Ui) {
        form::heading(ui, "Materials");
        let candidate = &mut self.candidate;
        if let Some(edit) = &mut self.material_edit {
            form::card(ui, |ui| edit.show(ui));
            if ui.button("Discard material edits").clicked() {
                self.material_edit = None;
            }
            return;
        }
        properties::section(ui, "Material families");
        let mut starter = None;
        let can_use = candidate.setup.stackup().strata.len() == 3;
        ui.columns(2, |columns| {
            for (i, family) in pcbcraft_engine::board::materials::StarterMaterial::ALL.into_iter().enumerate() {
                if form::material_choice(&mut columns[i % 2], family.name(), can_use) {
                    starter = Some(family);
                }
            }
        });
        if let Some(family) = starter {
            match candidate.value(self.unit).and_then(|setup| setup.with_starting_material(family).map_err(|e| e.to_string())) {
                Ok(setup) => {
                    *candidate = Candidate::new(setup, self.unit);
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
        properties::section(ui, "Board materials");
        if ui.button("Add material").clicked() {
            match Material::new("New material".into()).map_err(|e| e.to_string()).and_then(|m| Edit::new(m, false)) {
                Ok(edit) => self.material_edit = Some(edit),
                Err(error) => self.error = Some(error),
            }
        }
        if candidate.materials.is_empty() {
            ui.label("No material specifications pinned to this board.");
        }
        let mut use_material = None;
        for material in &candidate.materials {
            ui.push_id((material.id, material.revision), |ui| {
                form::card(ui, |ui| {
                    let used = candidate.constructions.iter().any(|construction| match construction {
                        Some(Construction::Core { material: Some(reference), .. }) => *reference == material.reference(),
                        Some(Construction::Prepreg { plies }) => plies.iter().any(|ply| ply.material == Some(material.reference())),
                        _ => false,
                    });
                    form::material_heading(ui, &material.name, material.family.as_deref(), used);
                    if !used && can_use && ui.button("Use for stackup").clicked() {
                        use_material = Some(material.reference());
                    }
                    ui.collapsing("Technical details", |ui| {
                        if let Some(source) = &material.source {
                            ui.label(&source.document);
                        }
                        for report in &material.tg {
                            ui.label(format!(
                                "Tg  {} °C · {} · {:?}",
                                report.temperature_milli_c as f64 / 1000.,
                                match report.method {
                                    TgMethod::Dsc => "DSC",
                                    TgMethod::Tma => "TMA",
                                    TgMethod::Dma => "DMA",
                                },
                                report.basis
                            ));
                            ui.label(&report.source.document);
                        }
                        for report in &material.dielectric {
                            ui.label(format!(
                                "Dk {}  ·  Df {}  ·  {}",
                                report.dk_millionths.map(|n| (n as f64 / 1_000_000.).to_string()).unwrap_or_else(|| "—".into()),
                                report.df_millionths.map(|n| (n as f64 / 1_000_000.).to_string()).unwrap_or_else(|| "—".into()),
                                report.frequency_hz.map(|f| format!("{f} Hz")).unwrap_or_else(|| "Frequency unspecified".into())
                            ));
                            ui.label(&report.source.document);
                        }
                        if ui.button("Add specification…").clicked() {
                            match Edit::new(material.clone(), true) {
                                Ok(edit) => self.material_edit = Some(edit),
                                Err(error) => self.error = Some(error),
                            }
                        }
                    });
                })
            });
        }
        if let Some(reference) = use_material {
            match candidate.value(self.unit).and_then(|setup| setup.with_core_material(reference).map_err(|e| e.to_string())) {
                Ok(setup) => {
                    *candidate = Candidate::new(setup, self.unit);
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Missing characterization conditions stay absent, and material revisions append reports
    /// without rewriting the original pin. Fixed-point parsing rejects malformed/overprecise text.
    #[test]
    fn material_reports_preserve_unknowns_sources_and_pinned_revisions() {
        let original = Material::new("Original laminate".into()).unwrap();
        let mut edit = Edit::new(original.clone(), true).unwrap();
        edit.dk = "4.200001".into();
        edit.df = "0".into();
        edit.basis = Some(ReportBasis::Typical);
        assert!(edit.value().is_err());
        edit.document = "Synthetic characterization source".into();
        let revised = edit.value().unwrap();
        assert_eq!(original.revision, 1);
        assert_eq!(revised.revision, 2);
        assert_eq!(revised.id, original.id);
        assert!(original.dielectric.is_empty());
        let report = &revised.dielectric[0];
        assert_eq!(report.dk_millionths, Some(4200001));
        assert_eq!(report.df_millionths, Some(0));
        assert_eq!(report.frequency_hz, None);
        assert_eq!(report.axis, None);
        assert_eq!(report.value_kind, None);
        for invalid in ["-", ".", "--2", "++2", "4.2000001", "1e3"] {
            assert!(decimal(invalid, 6).is_err(), "{invalid}");
        }
        assert_eq!(decimal("-40.125", 3).unwrap(), Some(-40125));
    }
}
