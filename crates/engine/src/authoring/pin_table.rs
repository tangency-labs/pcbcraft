//! Pin-table parsing, column mapping, per-row validation and layout preview for symbol authoring.
//!
//! This module owns the shared domain model used by both the Symbol Editor's import dialog and
//! the read-only `library.preview_pin_table` command. It never creates pins, mutates a symbol
//! document or allocates undo history; confirming a valid preview produces an [`ImportPlan`] that
//! later symbol-generation and re-import-diff stages consume.

use crate::{
    library::MAX_TERMINALS,
    schematic::{ElectricalType, GRID_NM, PinNumber, PinOrientation},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// Maximum UTF-8 byte length accepted for a pasted or loaded pin table.
pub const MAX_BYTES: usize = 256 * 1024;
/// Hard parse ceiling on logical data rows before terminal-count validation runs.
pub const MAX_ROWS: usize = 4096;
/// Maximum number of columns tracked in one table.
pub const MAX_COLUMNS: usize = 64;
/// Maximum distinct units supported by native multi-unit symbol definitions.
pub const MAX_UNITS: usize = 32;
/// Maximum character count of a terminal display name, matching native symbol validation.
pub const MAX_PIN_NAME_CHARS: usize = 32;
/// Standard schematic pin pitch (2.54 mm = 2 x 1.27 mm grid) used for body-size estimation.
pub const PIN_PITCH_NM: i64 = GRID_NM * 2;
/// Minimum symbol body span along either axis (10.16 mm = 4 pin pitches).
pub const MIN_BODY_SPAN_NM: i64 = PIN_PITCH_NM * 4;
/// Native symbol drawing half-extent bound (±100 mm), matching `library::MAX_DRAWING_NM`.
pub const MAX_DRAWING_NM: i64 = 100_000_000;
/// Maximum symbol body span along either axis so leads still fit inside ±100 mm.
pub const MAX_BODY_SPAN_NM: i64 = (MAX_DRAWING_NM - PIN_PITCH_NM) * 2;
/// Maximum terminals one unit can hold on the native symbol grid (`library::MAX_TERMINALS` docs).
pub const MAX_PINS_PER_UNIT: usize = 441;
/// Default unit identifier when no unit column is mapped or a row leaves unit blank.
pub const DEFAULT_UNIT: &str = "main";

/// Field delimiter used to split a pin table into columns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Delimiter {
    #[default]
    Auto,
    Comma,
    Tab,
    Semicolon,
}

impl Delimiter {
    /// All selectable delimiter modes in display order.
    pub const ALL: [Self; 4] = [Self::Auto, Self::Comma, Self::Tab, Self::Semicolon];

    /// Stable wire spelling used by command parameters and inspection.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Comma => "comma",
            Self::Tab => "tab",
            Self::Semicolon => "semicolon",
        }
    }

    /// Human-readable label shown in the import dialog.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Comma => "Comma (,)",
            Self::Tab => "Tab",
            Self::Semicolon => "Semicolon (;)",
        }
    }

    /// Parse a wire delimiter spelling; unknown values return `None`.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "auto" => Some(Self::Auto),
            "comma" => Some(Self::Comma),
            "tab" => Some(Self::Tab),
            "semicolon" => Some(Self::Semicolon),
            _ => None,
        }
    }

    /// Resolve the concrete separator byte for this mode, detecting from `text` when `Auto`.
    pub fn resolve(self, text: &str) -> Self {
        match self {
            Self::Auto => detect_delimiter(text),
            concrete => concrete,
        }
    }

    /// Return the ASCII separator byte for a concrete delimiter mode.
    const fn byte(self) -> u8 {
        match self {
            Self::Auto | Self::Comma => b',',
            Self::Tab => b'\t',
            Self::Semicolon => b';',
        }
    }
}

/// Which edge of a symbol body a pin attaches to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinSide {
    #[default]
    Left,
    Right,
    Top,
    Bottom,
}

impl PinSide {
    /// Stable wire spelling used in previews and import plans.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Top => "top",
            Self::Bottom => "bottom",
        }
    }

    /// Human-readable label shown in the preview table.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Top => "Top",
            Self::Bottom => "Bottom",
        }
    }

    /// Parse a pin side case-insensitively from common spreadsheet spellings.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "left" | "l" => Some(Self::Left),
            "right" | "r" => Some(Self::Right),
            "top" | "t" | "up" => Some(Self::Top),
            "bottom" | "bot" | "b" | "down" => Some(Self::Bottom),
            _ => None,
        }
    }

    /// Direction from the wire connection point into the symbol body for symbol generation.
    pub const fn lead_orientation(self) -> PinOrientation {
        match self {
            Self::Left => PinOrientation::Right,
            Self::Right => PinOrientation::Left,
            Self::Top => PinOrientation::Down,
            Self::Bottom => PinOrientation::Up,
        }
    }
}

/// Logical role that a source table column can be mapped to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnRole {
    Number,
    Name,
    ElectricalType,
    Unit,
    Side,
}

impl ColumnRole {
    /// Human-readable field label for the mapping form and error text.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Number => "Pin number",
            Self::Name => "Pin name",
            Self::ElectricalType => "Electrical type",
            Self::Unit => "Unit",
            Self::Side => "Side",
        }
    }
}

/// Zero-based column indices assigned to each logical pin field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColumnMapping {
    pub number: Option<usize>,
    pub name: Option<usize>,
    pub electrical_type: Option<usize>,
    pub unit: Option<usize>,
    pub side: Option<usize>,
}

impl ColumnMapping {
    /// Infer column assignments from header cells, falling back to positional defaults when
    /// `has_header` is false or no header names match known pin fields.
    pub fn auto_detect(headers: &[String], column_count: usize, has_header: bool) -> Self {
        let mut mapping = Self::default();
        if has_header {
            for (index, header) in headers.iter().enumerate() {
                let norm = normalize_header(header);
                if mapping.number.is_none()
                    && matches!(
                        norm.as_str(),
                        "number" | "pin" | "pin number" | "pin no" | "pin num" | "designator" | "num" | "no" | "#" | "terminal"
                    )
                {
                    mapping.number = Some(index);
                } else if mapping.name.is_none()
                    && matches!(norm.as_str(), "name" | "pin name" | "signal" | "signal name" | "label" | "display name" | "function")
                {
                    mapping.name = Some(index);
                } else if mapping.electrical_type.is_none()
                    && matches!(
                        norm.as_str(),
                        "type" | "electrical type" | "electrical" | "elec" | "elec type" | "pin type" | "direction" | "io" | "i/o"
                    )
                {
                    mapping.electrical_type = Some(index);
                } else if mapping.unit.is_none() && matches!(norm.as_str(), "unit" | "part" | "section" | "bank" | "subpart" | "group") {
                    mapping.unit = Some(index);
                } else if mapping.side.is_none() && matches!(norm.as_str(), "side" | "edge" | "position" | "orientation" | "location") {
                    mapping.side = Some(index);
                }
            }
        }
        if mapping.number.is_none() && mapping.name.is_none() && mapping.electrical_type.is_none() && mapping.unit.is_none() && mapping.side.is_none()
        {
            mapping = Self::positional(column_count);
        }
        mapping.clamped(column_count)
    }

    /// Assign columns in canonical order: number, name, electrical type, unit, side.
    pub fn positional(column_count: usize) -> Self {
        Self {
            number: (column_count > 0).then_some(0),
            name: (column_count > 1).then_some(1),
            electrical_type: (column_count > 2).then_some(2),
            unit: (column_count > 3).then_some(3),
            side: (column_count > 4).then_some(4),
        }
    }

    /// Drop any column index outside `0..column_count` so edits to source text never index out of bounds.
    pub fn clamped(self, column_count: usize) -> Self {
        let keep = |col: Option<usize>| col.filter(|&index| index < column_count);
        Self {
            number: keep(self.number),
            name: keep(self.name),
            electrical_type: keep(self.electrical_type),
            unit: keep(self.unit),
            side: keep(self.side),
        }
    }
}

/// Classification of a validation finding reported by pin-table preview.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    EmptyTable,
    InputTooLarge,
    UnclosedQuote,
    MissingNumberColumn,
    TooManyPins,
    TooManyUnits,
    UnitTooLarge,
    UnitNameCollision,
    MissingPinNumber,
    InvalidPinNumber,
    DuplicatePinNumber,
    InvalidPinName,
    UnknownElectricalType,
    InvalidUnitName,
    UnknownSide,
}

impl FindingKind {
    /// Short scannable badge label for compact preview-table status cells.
    pub const fn short_label(self) -> &'static str {
        match self {
            Self::EmptyTable => "Empty",
            Self::InputTooLarge => "Too large",
            Self::UnclosedQuote => "Quote error",
            Self::MissingNumberColumn => "No number col",
            Self::TooManyPins => "Too many pins",
            Self::TooManyUnits => "Too many units",
            Self::UnitTooLarge => "Unit too large",
            Self::UnitNameCollision => "Unit collision",
            Self::MissingPinNumber => "Missing number",
            Self::InvalidPinNumber => "Bad number",
            Self::DuplicatePinNumber => "Duplicate",
            Self::InvalidPinName => "Bad name",
            Self::UnknownElectricalType => "Bad type",
            Self::InvalidUnitName => "Bad unit",
            Self::UnknownSide => "Bad side",
        }
    }
}

/// One validation finding tied to either the whole table or a specific data row and source line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    /// 1-based data row index, or `None` for table-level findings.
    pub row: Option<usize>,
    /// 1-based line number in the source text, when available.
    pub line: Option<usize>,
    /// Logical column that triggered the finding, when applicable.
    pub column: Option<ColumnRole>,
    pub kind: FindingKind,
    pub message: String,
}

/// One parsed and validated row shown in the live preview table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewRow {
    /// 1-based data row number (excluding any header row).
    pub row_index: usize,
    /// 1-based starting line number in the source text.
    pub line_number: usize,
    pub raw_number: String,
    pub number: Option<String>,
    /// Pin name preserved verbatim without splitting at `/`.
    pub name: Option<String>,
    pub raw_type: String,
    pub electrical_type: Option<ElectricalType>,
    pub raw_unit: String,
    pub unit: String,
    pub raw_side: String,
    pub side: Option<PinSide>,
    /// Indices into [`Preview::findings`] belonging to this row.
    pub finding_indices: Vec<usize>,
}

impl PreviewRow {
    /// Whether this row has zero validation findings.
    pub fn is_valid(&self) -> bool {
        self.finding_indices.is_empty()
    }
}

/// Per-unit pin counts and grid-aligned body-size estimate for preview and P2 layout hand-off.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitSummary {
    pub unit: String,
    pub pin_count: usize,
    pub left: usize,
    pub right: usize,
    pub top: usize,
    pub bottom: usize,
    /// Minimum symbol body width on the 1.27 mm schematic grid to fit top and bottom pins.
    pub estimated_width_nm: i64,
    /// Minimum symbol body height on the 1.27 mm schematic grid to fit left and right pins.
    pub estimated_height_nm: i64,
}

/// Complete read-only preview returned by [`preview`] and `library.preview_pin_table`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preview {
    pub delimiter: Delimiter,
    pub has_header: bool,
    pub headers: Vec<String>,
    pub column_count: usize,
    pub mapping: ColumnMapping,
    pub rows: Vec<PreviewRow>,
    pub findings: Vec<Finding>,
    pub units: Vec<UnitSummary>,
    pub valid_row_count: usize,
}

impl Preview {
    /// Whether the table has at least one row and zero findings, permitting staging.
    pub fn can_accept(&self) -> bool {
        !self.rows.is_empty() && self.findings.is_empty()
    }

    /// Number of table-level findings not tied to a single row.
    pub fn table_finding_count(&self) -> usize {
        self.findings.iter().filter(|finding| finding.row.is_none()).count()
    }

    /// Number of parsed rows that carry at least one row-level finding.
    pub fn rows_with_findings_count(&self) -> usize {
        self.rows.iter().filter(|row| !row.is_valid()).count()
    }

    /// Build a validated, immutable [`ImportPlan`] bound to `base_revision` for downstream symbol
    /// generation or diffing. Returns an error if any finding remains or no pins were parsed.
    pub fn into_import_plan(&self, base_revision: u64) -> Result<ImportPlan, String> {
        if !self.can_accept() {
            return Err(if self.rows.is_empty() {
                "Pin table has no rows to import.".into()
            } else {
                let count = self.findings.len();
                format!("Resolve {count} {} before staging the pin table.", plural(count, "finding", "findings"))
            });
        }
        let mut pins = Vec::with_capacity(self.rows.len());
        for row in &self.rows {
            let number = row.number.clone().ok_or("Missing validated pin number.")?;
            let electrical_type = row.electrical_type.ok_or("Missing validated electrical type.")?;
            let side = row.side.unwrap_or(PinSide::Left);
            pins.push(PlannedPin { number, name: row.name.clone(), electrical_type, unit: row.unit.clone(), side });
        }
        Ok(ImportPlan { base_revision, pins, units: self.units.clone() })
    }

    /// Serialize the preview as the canonical JSON reply for `library.preview_pin_table`.
    pub fn to_json(&self) -> Value {
        json!({
            "delimiter": self.delimiter.as_str(),
            "has_header": self.has_header,
            "headers": self.headers,
            "column_count": self.column_count,
            "mapping": self.mapping,
            "rows": self.rows,
            "findings": self.findings,
            "units": self.units,
            "valid_row_count": self.valid_row_count,
            "can_accept": self.can_accept(),
            "created": false,
            "saved": false,
        })
    }
}

/// One validated pin ready for multi-unit symbol generation (P2) or re-import diffing (P3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedPin {
    pub number: String,
    pub name: Option<String>,
    pub electrical_type: ElectricalType,
    pub unit: String,
    pub side: PinSide,
}

/// Validated, immutable hand-off produced when a user stages a clean pin-table preview.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportPlan {
    /// Owning symbol document revision when the plan was staged, for staleness checks in P2/P3.
    pub base_revision: u64,
    pub pins: Vec<PlannedPin>,
    pub units: Vec<UnitSummary>,
}

/// Match an electrical type string case-insensitively against the fixed spelling list.
/// `I/O`, `IO` and `bidirectional` map to [`ElectricalType::Bidirectional`]. Unknown spellings
/// return `None` so callers record a finding instead of guessing.
pub fn parse_electrical_type(raw: &str) -> Option<ElectricalType> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "input" | "in" | "i" => Some(ElectricalType::Input),
        "output" | "out" | "o" => Some(ElectricalType::Output),
        "bidirectional" | "bidir" | "bi" | "i/o" | "io" | "inout" | "in/out" | "input/output" | "input-output" | "input_output" => {
            Some(ElectricalType::Bidirectional)
        }
        "passive" | "pas" | "p" => Some(ElectricalType::Passive),
        "power" | "pwr" | "power_in" | "power in" | "power-in" | "pwr_in" | "pwr in" | "power input" | "power_input" | "supply" => {
            Some(ElectricalType::PowerIn)
        }
        "power_out" | "power out" | "power-out" | "pwr_out" | "pwr out" | "power output" | "power_output" => Some(ElectricalType::PowerOut),
        "unspecified" | "unspec" | "unknown" | "?" => Some(ElectricalType::Unspecified),
        "tri_state" | "tri-state" | "tristate" | "tri state" | "hi-z" | "hiz" | "high-z" | "high_z" | "high impedance" | "high-impedance"
        | "high_impedance" | "3-state" | "3state" => Some(ElectricalType::TriState),
        "open_collector" | "open collector" | "open-collector" | "oc" | "open_drain" | "open drain" | "open-drain" | "od" => {
            Some(ElectricalType::OpenCollector)
        }
        "open_emitter" | "open emitter" | "open-emitter" | "oe" | "open_source" | "open source" | "open-source" | "os" => {
            Some(ElectricalType::OpenEmitter)
        }
        "no_connect" | "no connect" | "no-connect" | "nc" | "dnc" | "not connected" | "not_connected" => Some(ElectricalType::NoConnect),
        _ => None,
    }
}

/// Human-readable display label for an [`ElectricalType`] in preview tables.
pub const fn electrical_type_label(kind: ElectricalType) -> &'static str {
    match kind {
        ElectricalType::Passive => "Passive",
        ElectricalType::Input => "Input",
        ElectricalType::Output => "Output",
        ElectricalType::Bidirectional => "Bidirectional",
        ElectricalType::PowerIn => "Power in",
        ElectricalType::PowerOut => "Power out",
        ElectricalType::Unspecified => "Unspecified",
        ElectricalType::TriState => "Tri-state",
        ElectricalType::OpenCollector => "Open collector",
        ElectricalType::OpenEmitter => "Open emitter",
        ElectricalType::NoConnect => "No connect",
    }
}

/// Parse and validate a CSV or TSV pin table without mutating any symbol or project state.
/// When `mapping_override` is `None`, columns are auto-mapped from headers (or positionally).
pub fn preview(text: &str, delimiter: Delimiter, has_header: bool, mapping_override: Option<ColumnMapping>) -> Preview {
    let resolved = delimiter.resolve(text);
    if text.len() > MAX_BYTES {
        return Preview {
            delimiter: resolved,
            has_header,
            headers: Vec::new(),
            column_count: 0,
            mapping: ColumnMapping::default(),
            rows: Vec::new(),
            findings: vec![Finding {
                row: None,
                line: None,
                column: None,
                kind: FindingKind::InputTooLarge,
                message: format!("Pin table exceeds the {} KiB limit ({} bytes).", MAX_BYTES / 1024, text.len()),
            }],
            units: Vec::new(),
            valid_row_count: 0,
        };
    }

    let cleaned = text.strip_prefix('\u{feff}').unwrap_or(text);
    let parsed = parse_records(cleaned, resolved.byte());
    let mut findings = Vec::new();
    if let Some(line) = parsed.unclosed_quote_line {
        findings.push(Finding {
            row: None,
            line: Some(line),
            column: None,
            kind: FindingKind::UnclosedQuote,
            message: format!("Line {line}: unclosed quoted field."),
        });
    }

    let column_count = parsed.records.iter().map(|record| record.fields.len()).max().unwrap_or(0).min(MAX_COLUMNS);
    let (headers, data_records) = if has_header {
        match parsed.records.split_first() {
            Some((first, rest)) => (first.fields.iter().take(MAX_COLUMNS).map(|s| s.trim().to_owned()).collect(), rest),
            None => (Vec::new(), [].as_slice()),
        }
    } else {
        (Vec::new(), parsed.records.as_slice())
    };

    let mapping = match mapping_override {
        Some(custom) => custom.clamped(column_count),
        None => ColumnMapping::auto_detect(&headers, column_count, has_header),
    };

    if data_records.is_empty() {
        if findings.is_empty() {
            findings.push(Finding {
                row: None,
                line: None,
                column: None,
                kind: FindingKind::EmptyTable,
                message: "Pin table has no data rows. Paste or load at least one pin.".into(),
            });
        }
        return Preview {
            delimiter: resolved,
            has_header,
            headers,
            column_count,
            mapping,
            rows: Vec::new(),
            findings,
            units: Vec::new(),
            valid_row_count: 0,
        };
    }

    if mapping.number.is_none() {
        findings.push(Finding {
            row: None,
            line: None,
            column: Some(ColumnRole::Number),
            kind: FindingKind::MissingNumberColumn,
            message: "Map a Pin number column before staging.".into(),
        });
    }

    if parsed.truncated || data_records.len() > MAX_TERMINALS {
        let count_text = if parsed.truncated { format!("more than {MAX_ROWS} rows") } else { format!("{} rows", data_records.len()) };
        findings.push(Finding {
            row: None,
            line: None,
            column: None,
            kind: FindingKind::TooManyPins,
            message: format!("Pin table has {count_text}; a symbol supports at most {MAX_TERMINALS} pins."),
        });
    }

    #[derive(Default)]
    struct SideCounts {
        left: usize,
        right: usize,
        top: usize,
        bottom: usize,
    }

    let mut seen_numbers: BTreeMap<String, usize> = BTreeMap::new();
    let mut unit_spellings: BTreeMap<String, (String, usize)> = BTreeMap::new();
    let mut unit_counts: BTreeMap<String, SideCounts> = BTreeMap::new();
    let mut distinct_units: BTreeSet<String> = BTreeSet::new();
    let mut rows = Vec::with_capacity(data_records.len().min(MAX_ROWS));

    for (offset, record) in data_records.iter().take(MAX_ROWS).enumerate() {
        let row_index = offset + 1;
        let line_number = record.line_number;
        let cell = |col: Option<usize>| -> &str { col.and_then(|index| record.fields.get(index)).map(|value| value.trim()).unwrap_or("") };
        let mut row_findings = Vec::new();

        // Pin number
        let raw_number = cell(mapping.number).to_owned();
        let number = if mapping.number.is_none() {
            None
        } else if raw_number.is_empty() {
            let idx = findings.len();
            findings.push(Finding {
                row: Some(row_index),
                line: Some(line_number),
                column: Some(ColumnRole::Number),
                kind: FindingKind::MissingPinNumber,
                message: format!("Row {row_index} (line {line_number}): missing pin number."),
            });
            row_findings.push(idx);
            None
        } else if PinNumber::new(&raw_number).is_none() {
            let idx = findings.len();
            findings.push(Finding {
                row: Some(row_index),
                line: Some(line_number),
                column: Some(ColumnRole::Number),
                kind: FindingKind::InvalidPinNumber,
                message: format!(
                    "Row {row_index} (line {line_number}): pin number '{raw_number}' must be 1 to 8 ASCII letters, digits, '+', '-' or '_'."
                ),
            });
            row_findings.push(idx);
            None
        } else if let Some(&first_row) = seen_numbers.get(&raw_number) {
            let idx = findings.len();
            findings.push(Finding {
                row: Some(row_index),
                line: Some(line_number),
                column: Some(ColumnRole::Number),
                kind: FindingKind::DuplicatePinNumber,
                message: format!("Row {row_index} (line {line_number}): duplicate pin number '{raw_number}' (first seen on row {first_row})."),
            });
            row_findings.push(idx);
            Some(raw_number.clone())
        } else {
            seen_numbers.insert(raw_number.clone(), row_index);
            Some(raw_number.clone())
        };

        // Pin name (never split at '/'; native symbol validation counts characters, max 32)
        let raw_name = cell(mapping.name);
        let name = if raw_name.is_empty() {
            None
        } else if raw_name.chars().count() > MAX_PIN_NAME_CHARS || raw_name.chars().any(char::is_control) {
            let idx = findings.len();
            findings.push(Finding {
                row: Some(row_index),
                line: Some(line_number),
                column: Some(ColumnRole::Name),
                kind: FindingKind::InvalidPinName,
                message: format!(
                    "Row {row_index} (line {line_number}): pin name must be at most {MAX_PIN_NAME_CHARS} characters with no control characters."
                ),
            });
            row_findings.push(idx);
            None
        } else {
            Some(raw_name.to_owned())
        };

        // Electrical type (fixed case-insensitive spelling list; unmapped value is a finding)
        let raw_type = cell(mapping.electrical_type).to_owned();
        let electrical_type = if mapping.electrical_type.is_none() {
            Some(ElectricalType::Unspecified)
        } else if raw_type.is_empty() {
            let idx = findings.len();
            findings.push(Finding {
                row: Some(row_index),
                line: Some(line_number),
                column: Some(ColumnRole::ElectricalType),
                kind: FindingKind::UnknownElectricalType,
                message: format!(
                    "Row {row_index} (line {line_number}): missing electrical type (expected e.g. input, output, I/O, passive, power_in)."
                ),
            });
            row_findings.push(idx);
            None
        } else if let Some(parsed_type) = parse_electrical_type(&raw_type) {
            Some(parsed_type)
        } else {
            let idx = findings.len();
            findings.push(Finding {
                row: Some(row_index),
                line: Some(line_number),
                column: Some(ColumnRole::ElectricalType),
                kind: FindingKind::UnknownElectricalType,
                message: format!(
                    "Row {row_index} (line {line_number}): unknown electrical type '{raw_type}' (expected e.g. input, output, I/O, passive, power_in)."
                ),
            });
            row_findings.push(idx);
            None
        };

        // Unit
        let raw_unit = cell(mapping.unit).to_owned();
        let unit = if raw_unit.is_empty() {
            DEFAULT_UNIT.to_owned()
        } else if let Some(normalized) = normalize_unit_id(&raw_unit) {
            if let Some((first_spelling, first_row)) = unit_spellings.get(&normalized) {
                if first_spelling != &raw_unit {
                    let idx = findings.len();
                    findings.push(Finding {
                        row: Some(row_index),
                        line: Some(line_number),
                        column: Some(ColumnRole::Unit),
                        kind: FindingKind::UnitNameCollision,
                        message: format!(
                            "Row {row_index} (line {line_number}): unit '{raw_unit}' normalizes to '{normalized}', colliding with '{first_spelling}' on row {first_row}."
                        ),
                    });
                    row_findings.push(idx);
                }
            } else {
                unit_spellings.insert(normalized.clone(), (raw_unit.clone(), row_index));
            }
            normalized
        } else {
            let idx = findings.len();
            findings.push(Finding {
                row: Some(row_index),
                line: Some(line_number),
                column: Some(ColumnRole::Unit),
                kind: FindingKind::InvalidUnitName,
                message: format!(
                    "Row {row_index} (line {line_number}): unit '{raw_unit}' is not a valid unit ID (use 1 to 64 ASCII letters, digits or hyphens)."
                ),
            });
            row_findings.push(idx);
            DEFAULT_UNIT.to_owned()
        };
        distinct_units.insert(unit.clone());

        // Side
        let raw_side = cell(mapping.side).to_owned();
        let side = if raw_side.is_empty() {
            Some(PinSide::Left)
        } else if let Some(parsed_side) = PinSide::parse(&raw_side) {
            Some(parsed_side)
        } else {
            let idx = findings.len();
            findings.push(Finding {
                row: Some(row_index),
                line: Some(line_number),
                column: Some(ColumnRole::Side),
                kind: FindingKind::UnknownSide,
                message: format!("Row {row_index} (line {line_number}): unknown pin side '{raw_side}' (expected left, right, top or bottom)."),
            });
            row_findings.push(idx);
            None
        };

        let counts = unit_counts.entry(unit.clone()).or_default();
        match side.unwrap_or(PinSide::Left) {
            PinSide::Left => counts.left = counts.left.saturating_add(1),
            PinSide::Right => counts.right = counts.right.saturating_add(1),
            PinSide::Top => counts.top = counts.top.saturating_add(1),
            PinSide::Bottom => counts.bottom = counts.bottom.saturating_add(1),
        }

        rows.push(PreviewRow {
            row_index,
            line_number,
            raw_number,
            number,
            name,
            raw_type,
            electrical_type,
            raw_unit,
            unit,
            raw_side,
            side,
            finding_indices: row_findings,
        });
    }

    if distinct_units.len() > MAX_UNITS {
        findings.push(Finding {
            row: None,
            line: None,
            column: Some(ColumnRole::Unit),
            kind: FindingKind::TooManyUnits,
            message: format!("Pin table defines {} units; a symbol supports at most {MAX_UNITS} units.", distinct_units.len()),
        });
    }

    let units: Vec<UnitSummary> = unit_counts
        .into_iter()
        .map(|(unit, counts)| {
            let pin_count = counts.left.saturating_add(counts.right).saturating_add(counts.top).saturating_add(counts.bottom);
            let (estimated_width_nm, estimated_height_nm) = estimate_unit_body_nm(counts.left, counts.right, counts.top, counts.bottom);
            UnitSummary {
                unit,
                pin_count,
                left: counts.left,
                right: counts.right,
                top: counts.top,
                bottom: counts.bottom,
                estimated_width_nm,
                estimated_height_nm,
            }
        })
        .collect();

    for unit in &units {
        if unit.pin_count > MAX_PINS_PER_UNIT || unit.estimated_width_nm > MAX_BODY_SPAN_NM || unit.estimated_height_nm > MAX_BODY_SPAN_NM {
            findings.push(Finding {
                row: None,
                line: None,
                column: Some(ColumnRole::Unit),
                kind: FindingKind::UnitTooLarge,
                message: format!(
                    "Unit '{}' ({} pins, {:.1} × {:.1} mm) exceeds the native ±100 mm symbol drawing bound (max {MAX_PINS_PER_UNIT} pins per unit, max {:.1} mm per side); split pins across multiple units or sides.",
                    unit.unit,
                    unit.pin_count,
                    unit.estimated_width_nm as f64 / 1_000_000.0,
                    unit.estimated_height_nm as f64 / 1_000_000.0,
                    MAX_BODY_SPAN_NM as f64 / 1_000_000.0,
                ),
            });
        }
    }

    let valid_row_count = rows.iter().filter(|row| row.is_valid()).count();

    Preview { delimiter: resolved, has_header, headers, column_count, mapping, rows, findings, units, valid_row_count }
}

/// Compute the minimum grid-aligned body rectangle (width, height) in nanometres for one unit.
/// Each side reserves one pin pitch per pin plus one pitch margin at both ends, clamped to
/// [`MIN_BODY_SPAN_NM`] so small units stay readable on the 1.27 mm schematic grid.
pub fn estimate_unit_body_nm(left: usize, right: usize, top: usize, bottom: usize) -> (i64, i64) {
    let span = |max_pins: usize| -> i64 {
        let slots = i64::try_from(max_pins).unwrap_or(i64::MAX).saturating_add(1);
        slots.saturating_mul(PIN_PITCH_NM).max(MIN_BODY_SPAN_NM)
    };
    (span(top.max(bottom)), span(left.max(right)))
}

/// Choose singular or plural noun form for a count.
fn plural<'a>(count: usize, singular: &'a str, plural: &'a str) -> &'a str {
    if count == 1 { singular } else { plural }
}

/// JSON Schema for the optional `delimiter` parameter of `library.preview_pin_table`.
pub fn delimiter_schema() -> Value {
    json!({"type": "string", "enum": ["auto", "comma", "tab", "semicolon"]})
}

/// JSON Schema for the optional `mapping` parameter of `library.preview_pin_table`.
pub fn mapping_schema() -> Value {
    let col = json!({"type": "integer", "minimum": 0, "maximum": MAX_COLUMNS - 1});
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "number": col,
            "name": col,
            "electrical_type": col,
            "unit": col,
            "side": col,
        }
    })
}

/// Validate a JSON `mapping` parameter value against [`ColumnMapping`] bounds.
pub fn validate_mapping_value(value: &Value) -> Result<ColumnMapping, &'static str> {
    let Some(object) = value.as_object() else {
        return Err("must be an object mapping pin fields to zero-based column indices");
    };
    let mut mapping = ColumnMapping::default();
    for (key, raw) in object {
        let Some(index) = raw.as_u64().and_then(|n| usize::try_from(n).ok()).filter(|&n| n < MAX_COLUMNS) else {
            return Err("column index must be an integer from 0 to 63");
        };
        match key.as_str() {
            "number" => mapping.number = Some(index),
            "name" => mapping.name = Some(index),
            "electrical_type" => mapping.electrical_type = Some(index),
            "unit" => mapping.unit = Some(index),
            "side" => mapping.side = Some(index),
            _ => return Err("mapping keys must be number, name, electrical_type, unit or side"),
        }
    }
    Ok(mapping)
}

/// Execute `library.preview_pin_table` from validated command parameters without mutating state.
pub fn preview_from_params(params: &Value) -> Result<Value, String> {
    let text = params.get("text").and_then(Value::as_str).ok_or("Pin table `text` is required.")?;
    let delimiter = match params.get("delimiter").and_then(Value::as_str) {
        Some(raw) => Delimiter::parse(raw).ok_or("Delimiter must be auto, comma, tab or semicolon.")?,
        None => Delimiter::Auto,
    };
    let has_header = params.get("has_header").and_then(Value::as_bool).unwrap_or(true);
    let mapping = match params.get("mapping") {
        Some(value) if !value.is_null() => Some(validate_mapping_value(value).map_err(str::to_owned)?),
        _ => None,
    };
    Ok(preview(text, delimiter, has_header, mapping).to_json())
}

/// Normalize a unit cell to a portable lowercase identifier matching native symbol rules, or
/// return `None` if it contains disallowed punctuation or exceeds 64 bytes.
fn normalize_unit_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(trimmed.len());
    let mut prev_hyphen = false;
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_hyphen = false;
        } else if ch == '-' || ch == '_' || ch == ' ' {
            if !out.is_empty() && !prev_hyphen {
                out.push('-');
                prev_hyphen = true;
            }
        } else {
            return None;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    crate::board::definition::valid_id(&out).then_some(out)
}

/// Normalize a column header for case-insensitive matching against known pin field aliases.
fn normalize_header(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut prev_space = false;
    for ch in raw.trim().chars() {
        if ch == '_' || ch == '-' || ch.is_whitespace() {
            if !out.is_empty() && !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch.to_ascii_lowercase());
            prev_space = false;
        }
    }
    out.trim().to_owned()
}

/// Detect the most likely field delimiter from the first non-empty lines of `text`, ignoring
/// characters inside double-quoted strings. Prefers Tab whenever every sampled non-empty line
/// contains at least one unquoted tab, so TSV tables whose names contain commas stay Tab-delimited.
fn detect_delimiter(text: &str) -> Delimiter {
    let mut tab_count: usize = 0;
    let mut comma_count: usize = 0;
    let mut semi_count: usize = 0;
    let mut lines_with_tab: usize = 0;
    let mut in_quotes = false;
    let mut lines_checked: usize = 0;

    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut line_has_tab = false;
        for byte in line.bytes() {
            match byte {
                b'"' => in_quotes = !in_quotes,
                b'\t' if !in_quotes => {
                    tab_count = tab_count.saturating_add(1);
                    line_has_tab = true;
                }
                b',' if !in_quotes => comma_count = comma_count.saturating_add(1),
                b';' if !in_quotes => semi_count = semi_count.saturating_add(1),
                _ => {}
            }
        }
        if line_has_tab {
            lines_with_tab = lines_with_tab.saturating_add(1);
        }
        lines_checked = lines_checked.saturating_add(1);
        if lines_checked >= 8 {
            break;
        }
    }

    if lines_checked > 0 && lines_with_tab == lines_checked {
        Delimiter::Tab
    } else if tab_count >= comma_count && tab_count >= semi_count && tab_count > 0 {
        Delimiter::Tab
    } else if semi_count > comma_count {
        Delimiter::Semicolon
    } else {
        Delimiter::Comma
    }
}

struct ParsedRecord {
    line_number: usize,
    fields: Vec<String>,
}

struct ParsedTable {
    records: Vec<ParsedRecord>,
    unclosed_quote_line: Option<usize>,
    truncated: bool,
}

/// Parse RFC 4180 delimited records with `""` escape handling and 1-based line tracking.
/// Completely blank lines are skipped so trailing newlines never emit phantom error rows.
fn parse_records(text: &str, separator: u8) -> ParsedTable {
    let bytes = text.as_bytes();
    let mut records = Vec::new();
    let mut fields = Vec::new();
    let mut field = Vec::new();
    let mut in_quotes = false;
    let mut quote_start_line = 1usize;
    let mut line_number = 1usize;
    let mut record_start_line = 1usize;
    let mut index = 0usize;
    let mut truncated = false;

    while let Some(&byte) = bytes.get(index) {
        if in_quotes {
            if byte == b'"' {
                if bytes.get(index + 1) == Some(&b'"') {
                    field.push(b'"');
                    index += 2;
                    continue;
                }
                in_quotes = false;
                index += 1;
                continue;
            }
            if byte == b'\n' {
                line_number = line_number.saturating_add(1);
            }
            field.push(byte);
            index += 1;
            continue;
        }

        if byte == b'"' && field.is_empty() {
            in_quotes = true;
            quote_start_line = line_number;
            index += 1;
            continue;
        }

        if byte == separator {
            fields.push(String::from_utf8_lossy(&field).into_owned());
            field.clear();
            index += 1;
            continue;
        }

        if byte == b'\r' || byte == b'\n' {
            if byte == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
                index += 2;
            } else {
                index += 1;
            }
            fields.push(String::from_utf8_lossy(&field).into_owned());
            field.clear();
            if fields.iter().any(|cell| !cell.trim().is_empty()) {
                records.push(ParsedRecord { line_number: record_start_line, fields: std::mem::take(&mut fields) });
            } else {
                fields.clear();
            }
            line_number = line_number.saturating_add(1);
            record_start_line = line_number;
            if records.len() > MAX_ROWS {
                if bytes.get(index..).is_some_and(|rest| rest.iter().any(|b| !b.is_ascii_whitespace())) {
                    truncated = true;
                }
                break;
            }
            continue;
        }

        field.push(byte);
        index += 1;
    }

    if !truncated && (!field.is_empty() || !fields.is_empty()) {
        fields.push(String::from_utf8_lossy(&field).into_owned());
        if fields.iter().any(|cell| !cell.trim().is_empty()) {
            records.push(ParsedRecord { line_number: record_start_line, fields });
        }
    }

    ParsedTable { records, unclosed_quote_line: in_quotes.then_some(quote_start_line), truncated }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Session, library::sources};

    /// Done test: a synthetic 256-pin CSV with 2 duplicate pin numbers previews 256 rows and 2
    /// findings through the shared engine command, and creates nothing.
    #[test]
    fn synthetic_256_pin_csv_with_two_duplicates_previews_256_rows_and_2_findings() {
        let mut csv = String::from("Number,Name,Type,Unit,Side\n");
        for i in 1..=256 {
            let number = match i {
                100 => 10,
                200 => 20,
                _ => i,
            };
            let unit = format!("bank-{}", ((i - 1) / 64) + 1);
            let side = if i % 2 == 1 { "left" } else { "right" };
            let etype = match i % 4 {
                0 => "power_in",
                1 => "input",
                2 => "output",
                _ => "I/O",
            };
            csv.push_str(&format!("{number},SIG_{i}/ALT_{i},{etype},{unit},{side}\n"));
        }

        let mut session = Session::default();
        let before = session.inspect();
        let reply = session.execute(sources::PREVIEW_PIN_TABLE, &json!({"text": csv})).unwrap();

        assert_eq!(session.inspect(), before);
        assert_eq!(reply["rows"].as_array().unwrap().len(), 256);
        assert_eq!(reply["findings"].as_array().unwrap().len(), 2);
        assert_eq!(reply["valid_row_count"], 254);
        assert_eq!(reply["can_accept"], false);
        assert_eq!(reply["created"], false);
        assert_eq!(reply["saved"], false);
        assert_eq!(reply["units"].as_array().unwrap().len(), 4);
        assert_eq!(reply["findings"][0]["kind"], "duplicate_pin_number");
        assert_eq!(reply["findings"][0]["row"], 100);
        assert_eq!(reply["findings"][1]["kind"], "duplicate_pin_number");
        assert_eq!(reply["findings"][1]["row"], 200);
        assert_eq!(reply["rows"][0]["name"], "SIG_1/ALT_1");
        assert_eq!(reply["rows"][2]["electrical_type"], "bidirectional");
    }

    /// Electrical type spellings are matched case-insensitively and unmapped values produce
    /// clear findings while blocking staging. Pin names containing `/` stay intact.
    #[test]
    fn electrical_type_matching_and_slash_pin_names() {
        for spelling in ["I/O", "IO", "io", "Bidirectional", "bidir", "input-output", "IN/OUT"] {
            assert_eq!(parse_electrical_type(spelling), Some(ElectricalType::Bidirectional), "{spelling}");
        }
        assert_eq!(parse_electrical_type("Open Drain"), Some(ElectricalType::OpenCollector));
        assert_eq!(parse_electrical_type("Hi-Z"), Some(ElectricalType::TriState));
        assert_eq!(parse_electrical_type("NC"), Some(ElectricalType::NoConnect));
        assert_eq!(parse_electrical_type("analog_magic"), None);

        let csv = "Pin,Signal,Electrical Type,Unit,Side\n1,SDA/GPIO2,I/O,main,left\n2,SCL/GPIO3,analog_magic,main,right\n";
        let result = preview(csv, Delimiter::Auto, true, None);
        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.rows[0].name.as_deref(), Some("SDA/GPIO2"));
        assert_eq!(result.rows[0].electrical_type, Some(ElectricalType::Bidirectional));
        assert!(result.rows[0].is_valid());
        assert_eq!(result.findings.len(), 1);
        assert_eq!(result.findings[0].kind, FindingKind::UnknownElectricalType);
        assert!(result.findings[0].message.contains("analog_magic"));
        assert!(!result.can_accept());
        assert!(result.into_import_plan(0).is_err());
    }

    /// TSV input (including commas inside unquoted names), quoted fields, BOM and manual column
    /// remapping produce exact rows and plans.
    #[test]
    fn tsv_auto_detect_quoted_fields_and_manual_column_mapping() {
        let tsv = "\u{feff}Signal\tPin\tDir\tEdge\n\"TXD, \"\"alt\"\"\"\tA1\tOutput\tRight\nRXD, A, B, C\tA2\tInput\tLeft\n";
        let auto = preview(tsv, Delimiter::Auto, true, None);
        assert_eq!(auto.delimiter, Delimiter::Tab);
        assert_eq!(auto.mapping.number, Some(1));
        assert_eq!(auto.mapping.name, Some(0));
        assert_eq!(auto.mapping.electrical_type, Some(2));
        assert_eq!(auto.mapping.side, Some(3));
        assert!(auto.can_accept());
        assert_eq!(auto.rows[0].number.as_deref(), Some("A1"));
        assert_eq!(auto.rows[0].name.as_deref(), Some("TXD, \"alt\""));
        assert_eq!(auto.rows[0].side, Some(PinSide::Right));
        assert_eq!(auto.rows[1].name.as_deref(), Some("RXD, A, B, C"));

        // Overriding mapping to drop the number column produces MissingNumberColumn.
        let no_num = preview(tsv, Delimiter::Tab, true, Some(ColumnMapping { number: None, ..auto.mapping }));
        assert!(!no_num.can_accept());
        assert_eq!(no_num.table_finding_count(), 1);
        assert!(no_num.findings.iter().any(|finding| finding.kind == FindingKind::MissingNumberColumn));
    }

    /// Unit body geometry estimates stay aligned to the 1.27 mm schematic grid, enforce native
    /// ±100 mm / 441-pin unit bounds, and record `base_revision` in the staged `ImportPlan`.
    #[test]
    fn unit_geometry_estimation_and_import_plan_seam() {
        let mut csv = String::from("Number,Name,Type,Unit,Side\n");
        for i in 1..=256 {
            let unit = format!("Unit {}", ((i - 1) / 64) + 1);
            let side = if i % 2 == 1 { "left" } else { "right" };
            csv.push_str(&format!("{i},P{i},passive,{unit},{side}\n"));
        }
        let result = preview(&csv, Delimiter::Comma, true, None);
        assert!(result.can_accept());
        assert_eq!(result.units.len(), 4);
        for unit in &result.units {
            assert_eq!(unit.pin_count, 64);
            assert_eq!(unit.left, 32);
            assert_eq!(unit.right, 32);
            assert_eq!(unit.estimated_width_nm, MIN_BODY_SPAN_NM);
            assert_eq!(unit.estimated_height_nm, 33 * PIN_PITCH_NM);
            assert_eq!(unit.estimated_width_nm % GRID_NM, 0);
            assert_eq!(unit.estimated_height_nm % GRID_NM, 0);
        }
        let plan = result.into_import_plan(7).unwrap();
        assert_eq!(plan.base_revision, 7);
        assert_eq!(plan.pins.len(), 256);
        assert_eq!(plan.units.len(), 4);
        assert_eq!(plan.pins[0].unit, "unit-1");
        assert_eq!(plan.pins[0].side.lead_orientation(), PinOrientation::Right);
        assert_eq!(plan.pins[1].side.lead_orientation(), PinOrientation::Left);

        // A single unit with 1024 pins on the Left side exceeds native ±100 mm drawing bounds and
        // 441 pins per unit, so it must be refused with UnitTooLarge.
        let mut single_unit = String::from("Number,Name,Type,Unit,Side\n");
        for i in 1..=1024 {
            single_unit.push_str(&format!("{i},P{i},passive,main,left\n"));
        }
        let oversized = preview(&single_unit, Delimiter::Comma, true, None);
        assert!(!oversized.can_accept());
        assert!(oversized.findings.iter().any(|finding| finding.kind == FindingKind::UnitTooLarge));
    }

    /// Validation catches empty tables, missing pin numbers, invalid pin names (counting Unicode
    /// characters rather than bytes), unit spelling collisions, >1024 pins, >32 units, unclosed
    /// quotes, oversized inputs, and command parameter validation errors.
    #[test]
    fn validation_catches_malformed_rows_and_limits() {
        let empty = preview("   \n\n", Delimiter::Auto, true, None);
        assert!(!empty.can_accept());
        assert_eq!(empty.findings[0].kind, FindingKind::EmptyTable);

        // 20 non-ASCII characters (40 UTF-8 bytes) is valid; 33 characters is refused.
        let valid_utf8_name = "Ω".repeat(20);
        let long_name = "A".repeat(33);
        let rows = format!(
            "Number,Name,Type,Unit,Side\n\
             ,{valid_utf8_name},input,main,left\n\
             2,{long_name},input,Bank 1,top\n\
             3,OK,input,bank_1,bottom\n\
             bad pin!,OK,input,bad/unit,diagonal\n"
        );
        let result = preview(&rows, Delimiter::Comma, true, None);
        assert!(!result.can_accept());
        let kinds: Vec<_> = result.findings.iter().map(|finding| finding.kind).collect();
        assert!(kinds.contains(&FindingKind::MissingPinNumber));
        assert!(kinds.contains(&FindingKind::InvalidPinName));
        assert!(kinds.contains(&FindingKind::UnitNameCollision));
        assert!(kinds.contains(&FindingKind::InvalidPinNumber));
        assert!(kinds.contains(&FindingKind::InvalidUnitName));
        assert!(kinds.contains(&FindingKind::UnknownSide));
        assert_eq!(result.rows[0].name.as_deref(), Some(valid_utf8_name.as_str()));

        let unclosed = "Number,Name\n1,\"unclosed\n";
        let unclosed_preview = preview(unclosed, Delimiter::Comma, true, None);
        assert!(unclosed_preview.findings.iter().any(|finding| finding.kind == FindingKind::UnclosedQuote));

        let mut too_many_pins = String::from("Number,Name,Type\n");
        for i in 1..=1025 {
            too_many_pins.push_str(&format!("{i},P{i},passive\n"));
        }
        let pins_preview = preview(&too_many_pins, Delimiter::Comma, true, None);
        assert!(pins_preview.findings.iter().any(|finding| finding.kind == FindingKind::TooManyPins));

        let mut too_many_units = String::from("Number,Name,Type,Unit\n");
        for i in 1..=33 {
            too_many_units.push_str(&format!("{i},P{i},passive,u{i}\n"));
        }
        let units_preview = preview(&too_many_units, Delimiter::Comma, true, None);
        assert!(units_preview.findings.iter().any(|finding| finding.kind == FindingKind::TooManyUnits));

        let huge = "a".repeat(MAX_BYTES + 1);
        let huge_preview = preview(&huge, Delimiter::Comma, true, None);
        assert_eq!(huge_preview.findings.len(), 1);
        assert_eq!(huge_preview.findings[0].kind, FindingKind::InputTooLarge);

        // Command parameter handling: has_header: false, explicit mapping, bad delimiter, bad mapping.
        let mut session = Session::default();
        let custom = session
            .execute(
                sources::PREVIEW_PIN_TABLE,
                &json!({
                    "text": "VCC;1;power_in\nGND;2;power_in\n",
                    "delimiter": "semicolon",
                    "has_header": false,
                    "mapping": {"name": 0, "number": 1, "electrical_type": 2}
                }),
            )
            .unwrap();
        assert_eq!(custom["can_accept"], true);
        assert_eq!(custom["rows"][0]["number"], "1");
        assert_eq!(custom["rows"][0]["name"], "VCC");
        assert!(session.execute(sources::PREVIEW_PIN_TABLE, &json!({"text": "1,A\n", "delimiter": "pipe"})).is_err());
        assert!(session.execute(sources::PREVIEW_PIN_TABLE, &json!({"text": "1,A\n", "mapping": {"unknown": 0}})).is_err());
        assert!(session.execute(sources::PREVIEW_PIN_TABLE, &json!({"text": "1,A\n", "mapping": {"number": 99}})).is_err());
    }
}
