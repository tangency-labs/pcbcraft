//! Read-only library checks: typed findings over exactly authored pad occurrences and package maps.
//!
//! Strict loaders, editors and publication keep refusing every defect reported here. The check
//! only explains one: it decodes the transported documents structurally, keeps every authored pad
//! in its authored order without deduplication, and reports each defect with its measured count,
//! limit and basis. It writes no file, changes no project and records no history step.
mod budget;

use super::{
    parse_manifest, parse_symbol,
    parts::{Package, PartDefinition, PartFormatError, Reference},
    sources::{Catalog, Documents, MAX_FILE_BYTES, MAX_FILES, check_documents},
};
use crate::{
    authoring::{Item, Kind, Primitive},
    board::definition::{FootprintDefinition, RectPad},
    part_authoring::Record,
    persistence::reject_duplicate_json_keys,
};
use budget::{Budget, MAX_REPORT_BYTES, MAX_WORK};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// Report typed findings for transported documents and optional drafts without changing anything.
pub const CHECK: &str = "library.check";
/// Pads in one captured footprint, the native format's own limit; also the most distinct pads,
/// pads per terminal, or unconnected pads in one captured package.
pub const MAX_PADS: usize = crate::board::definition::MAX_PADS;
/// Packages in one captured part, the part draft limit.
pub const MAX_PACKAGES: usize = 64;
/// Terminals mapped in one captured package, the symbol terminal limit.
pub const MAX_PACKAGE_TERMINALS: usize = super::MAX_TERMINALS;

/// Where a captured footprint or package came from. It never names a host path.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CaptureSource {
    /// A transported native document, by its portable relative path.
    Document { path: String },
    /// The captured authoring item or part draft.
    Draft,
    /// An exact definition resolved from the session catalog.
    Catalog,
}

/// One authored pad. Occurrences stay in authored order and repeated numbers are kept.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PadOccurrence {
    /// Position in the native `pads` array, or the authoring primitive ID for a draft.
    pub index: u64,
    /// 1-based count among the pads with the same number, in authored order.
    pub ordinal: u16,
    /// The authored pad number.
    pub number: String,
    /// The complete authored pad, so later rules read its other fields without a new capture.
    pub pad: RectPad,
}

/// The pads of one exact footprint revision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FootprintCapture {
    pub reference: Reference,
    pub source: CaptureSource,
    pub pads: Vec<PadOccurrence>,
}

/// One package of a part with the terminals of its exact resolved symbol, kept as authored.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PackageCapture {
    pub part: Reference,
    pub package: String,
    pub source: CaptureSource,
    pub symbol: Option<Reference>,
    pub footprint: Reference,
    pub mapping: BTreeMap<String, Vec<String>>,
    pub unconnected_pads: Vec<String>,
    /// Logical terminals in native symbol order; empty for a part without a symbol.
    pub terminals: Vec<String>,
}

/// An immutable capture of everything one check reads. Its digest binds a report to it.
/// Construction also completes and bounds the raw findings; reporting only selects and labels
/// those findings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capture {
    footprints: Vec<FootprintCapture>,
    packages: Vec<PackageCapture>,
    digest: String,
    findings: Vec<Finding>,
}

/// A capture that cannot be checked. There is no partial or clean report for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckError {
    /// Invalid JSON, fields, identities or package assignments.
    Malformed { file: String, reason: String },
    /// A format the check does not read, or one that does not match the authored content.
    UnsupportedFormat { file: String, format: String },
    /// An exact reference found neither in the transported documents nor in the catalog.
    Unresolved { kind: &'static str, reference: Reference },
    /// More data than the native formats or the aggregate check budget allow.
    TooLarge { what: String, limit: usize },
    /// A severity override names no registered rule.
    UnknownRule { rule: String },
    /// A severity override lowers a required rule below its floor.
    BelowFloor { rule: String, floor: Severity },
}

impl std::fmt::Display for CheckError {
    /// One line naming the file or reference, for command errors.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed { file, reason } => write!(f, "{file}: {reason}"),
            Self::UnsupportedFormat { file, format } => write!(f, "{file}: unsupported or mismatched format {format:?}"),
            Self::Unresolved { kind, reference } => {
                write!(f, "Unresolved {kind} {}/{} revision {}", reference.library, reference.id, reference.revision)
            }
            Self::TooLarge { what, limit } => write!(f, "{what} exceeds the limit of {limit}"),
            Self::UnknownRule { rule } => write!(f, "Unknown library check rule {rule:?}"),
            Self::BelowFloor { rule, floor } => write!(f, "Rule {rule} is required and cannot be set below {}", floor.id()),
        }
    }
}

/// Stable rule identities. Their declaration order is the report order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum RuleId {
    /// One footprint has more than one pad with the same number.
    #[serde(rename = "pad.number.repeated")]
    PadNumberRepeated,
    /// A symbol terminal has no pad in a package map.
    #[serde(rename = "package.terminal.unmapped")]
    PackageTerminalUnmapped,
    /// A footprint pad on a package with a symbol is neither mapped nor explicitly unconnected.
    #[serde(rename = "package.pad.unaccounted")]
    PackagePadUnaccounted,
    /// A package names a pad number its footprint does not have.
    #[serde(rename = "package.pad.not_in_footprint")]
    PackagePadNotInFootprint,
    /// A package assigns the same pad number more than once.
    #[serde(rename = "package.pad.repeated")]
    PackagePadRepeated,
}

impl RuleId {
    /// The stable text identity used in reports.
    pub fn id(self) -> &'static str {
        match self {
            Self::PadNumberRepeated => "pad.number.repeated",
            Self::PackageTerminalUnmapped => "package.terminal.unmapped",
            Self::PackagePadUnaccounted => "package.pad.unaccounted",
            Self::PackagePadNotInFootprint => "package.pad.not_in_footprint",
            Self::PackagePadRepeated => "package.pad.repeated",
        }
    }

    /// Repetition rules allow at most one; presence rules require at least one.
    fn limit(self) -> Limit {
        let comparator = match self {
            Self::PadNumberRepeated | Self::PackagePadRepeated => Comparator::Max,
            Self::PackageTerminalUnmapped | Self::PackagePadUnaccounted | Self::PackagePadNotInFootprint => Comparator::Min,
        };
        Limit { value: 1, unit: Unit::Count, comparator }
    }

    /// The project rule each check enforces; standard clauses arrive with numeric rules.
    fn basis(self) -> Basis {
        match self {
            Self::PadNumberRepeated => Basis::ProjectRule { doc: "docs/libraries/footprint-schema.md", section: "Semantic validation" },
            Self::PackageTerminalUnmapped | Self::PackagePadUnaccounted | Self::PackagePadNotInFootprint | Self::PackagePadRepeated => {
                Basis::ProjectRule { doc: "docs/libraries/part-schema.md", section: "Package maps" }
            }
        }
    }
}

/// How much a finding matters. Declared from least to most severe, so severities compare.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Worth knowing; never blocks anything.
    Info,
    /// Worth fixing; does not block publication.
    Warning,
    /// Blocks publication of the item.
    Error,
}

impl Severity {
    /// The stable text identity used in reports and parameters.
    pub fn id(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

/// Where in an item's life a check runs: while editing, before publication or before placement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    #[default]
    Edit,
    Publish,
    Place,
}

/// Every stage, for rules that always run.
const ALL_STAGES: &[Stage] = &[Stage::Edit, Stage::Publish, Stage::Place];

/// The lowest severity an override may give a required rule.
const REQUIRED_FLOOR: Severity = Severity::Warning;

/// One registered rule: its identity, its default severity, the stages it runs at and whether an
/// override may lower it below [`REQUIRED_FLOOR`]. Rules are found by their text identity, so a
/// rule set can grow without changing how overrides or reports name them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule {
    pub id: RuleId,
    pub severity: Severity,
    pub stages: &'static [Stage],
    pub required: bool,
}

/// The built-in rules, in report order. Strict loading and publication refuse each of these
/// defects, so every one is a required error that runs at every stage.
pub const RULES: &[Rule] = &[
    Rule { id: RuleId::PadNumberRepeated, severity: Severity::Error, stages: ALL_STAGES, required: true },
    Rule { id: RuleId::PackageTerminalUnmapped, severity: Severity::Error, stages: ALL_STAGES, required: true },
    Rule { id: RuleId::PackagePadUnaccounted, severity: Severity::Error, stages: ALL_STAGES, required: true },
    Rule { id: RuleId::PackagePadNotInFootprint, severity: Severity::Error, stages: ALL_STAGES, required: true },
    Rule { id: RuleId::PackagePadRepeated, severity: Severity::Error, stages: ALL_STAGES, required: true },
];

/// The choices of one check run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// Only rules that run at this stage report findings.
    pub stage: Stage,
    /// A severity per rule identity, replacing that rule's default for this run.
    pub severity: BTreeMap<String, Severity>,
}

impl Options {
    /// The rules of `rules` with this run's severities applied.
    ///
    /// # Errors
    /// Refuses an override naming no rule in `rules`, and one that sets a required rule below
    /// [`REQUIRED_FLOOR`]; a refused run has no report.
    fn resolve(&self, rules: &[Rule]) -> Result<Vec<Rule>, CheckError> {
        for (id, severity) in &self.severity {
            let rule = rules.iter().find(|rule| rule.id.id() == id).ok_or_else(|| CheckError::UnknownRule { rule: id.clone() })?;
            if rule.required && *severity < REQUIRED_FLOOR {
                return Err(CheckError::BelowFloor { rule: id.clone(), floor: REQUIRED_FLOOR });
            }
        }
        Ok(rules.iter().map(|rule| Rule { severity: self.severity.get(rule.id.id()).copied().unwrap_or(rule.severity), ..*rule }).collect())
    }
}

/// Findings per severity in one report.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub error: usize,
    pub warning: usize,
    pub info: usize,
}

/// What a finding is about. Fields are ordered so reports sort by identity before source.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Subject {
    Pad { footprint: Reference, number: String, source: CaptureSource },
    Terminal { part: Reference, package: String, terminal: String, source: CaptureSource },
    PackagePad { part: Reference, package: String, number: String, source: CaptureSource },
}

/// One authored pad occurrence named by a finding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct PadOccurrenceRef {
    pub index: u64,
    pub ordinal: u16,
}

/// Units of measured values. Current rules count occurrences.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Count,
}

/// A measured value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Quantity {
    pub value: i64,
    pub unit: Unit,
}

/// Whether the limit is an upper or a lower bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Comparator {
    Max,
    Min,
}

/// The required value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Limit {
    pub value: i64,
    pub unit: Unit,
    pub comparator: Comparator,
}

/// Where a rule comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Basis {
    ProjectRule { doc: &'static str, section: &'static str },
}

/// One defect with its measured value, limit and basis.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub rule: RuleId,
    /// The rule's severity in the run that produced this finding.
    pub severity: Severity,
    pub subject: Subject,
    /// Every footprint occurrence involved, in authored order; empty when none exists.
    pub occurrences: Vec<PadOccurrenceRef>,
    pub measured: Quantity,
    pub limit: Limit,
    pub basis: Basis,
}

impl Finding {
    /// Fill the limit and basis from the rule, so one rule cannot report two limits. The severity
    /// starts as an error and the run sets it from its resolved rule.
    fn new(rule: RuleId, subject: Subject, measured: usize, occurrences: Vec<PadOccurrenceRef>) -> Self {
        Self {
            rule,
            severity: Severity::Error,
            subject,
            occurrences,
            measured: Quantity { value: i64::try_from(measured).unwrap_or(i64::MAX), unit: Unit::Count },
            limit: rule.limit(),
            basis: rule.basis(),
        }
    }
}

/// The findings for one exact capture, in deterministic order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Report {
    capture_digest: String,
    stage: Stage,
    findings: Vec<Finding>,
    counts: Counts,
    blocks_publish: bool,
}

impl Report {
    /// SHA-256 of the canonical capture; a changed capture needs a new report.
    pub fn capture_digest(&self) -> &str {
        &self.capture_digest
    }
    /// Findings sorted by rule, subject and first occurrence.
    pub fn findings(&self) -> &[Finding] {
        &self.findings
    }
    /// Clean only when no rule reports a finding; errors never produce a report.
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }
    /// The stage whose rules ran.
    pub fn stage(&self) -> Stage {
        self.stage
    }
    /// Findings per severity.
    pub fn counts(&self) -> Counts {
        self.counts
    }
    /// True when a finding is an error from a rule that runs before publication.
    pub fn blocks_publish(&self) -> bool {
        self.blocks_publish
    }
}

/// The only field read before the full structural decode, so a format is named before its fields.
#[derive(Deserialize)]
struct Header {
    format: String,
}

/// The transported documents, decoded structurally and bound to their manifest.
#[derive(Default)]
struct Transported {
    footprints: BTreeMap<Reference, FootprintCapture>,
    symbols: BTreeMap<Reference, Vec<String>>,
    parts: Vec<(String, PartDefinition)>,
}

impl Transported {
    /// Decode every footprint, symbol and part document. Categories and manufacturer parts own no
    /// pads or package maps and are not read.
    fn read(documents: &Documents, work: &mut Budget) -> Result<Self, CheckError> {
        if documents.len() <= MAX_FILES {
            for (path, text) in documents {
                if text.len() > MAX_FILE_BYTES {
                    return Err(CheckError::TooLarge { what: path.clone(), limit: MAX_FILE_BYTES });
                }
            }
        }
        check_documents(documents).map_err(|reason| CheckError::Malformed { file: "documents".into(), reason })?;
        let library = documents
            .get("library.json")
            .map(|text| {
                reject_duplicate_json_keys(text)?;
                parse_manifest(text).map(|manifest| manifest.id)
            })
            .transpose()
            .map_err(|reason| CheckError::Malformed { file: "library.json".into(), reason })?;
        let mut transported = Self::default();
        let mut part_refs = BTreeSet::new();
        for (path, text) in documents.iter().filter(|(path, _)| path.as_str() != "library.json") {
            let malformed = |reason: &str| CheckError::Malformed { file: path.clone(), reason: reason.into() };
            let library = library.as_deref().ok_or_else(|| malformed("library.json is required to bind document identity"))?;
            if path.starts_with("footprints/") {
                let definition = decode_footprint(path, text)?;
                if definition.library != library {
                    return Err(malformed("Footprint library does not match library.json"));
                }
                let reference = Reference { library: definition.library.clone(), id: definition.id.clone(), revision: definition.revision };
                if transported.footprints.contains_key(&reference) {
                    return Err(malformed("Another document defines the same footprint revision"));
                }
                let pads = definition.pads.iter().enumerate().map(|(index, pad)| (u64::try_from(index).unwrap_or(u64::MAX), pad));
                let footprint = capture_footprint(reference.clone(), CaptureSource::Document { path: path.clone() }, pads, work)?;
                transported.footprints.insert(reference, footprint);
            } else if path.starts_with("symbols/") {
                reject_duplicate_json_keys(text).map_err(|reason| malformed(&reason))?;
                let symbol = parse_symbol(text).map_err(|reason| malformed(&reason))?;
                let reference = Reference { library: library.into(), id: symbol.id.clone(), revision: symbol.revision() };
                work.reference(&reference)?;
                let terminals = copy_terminals(symbol.pins().iter().map(|pin| &pin.number), work)?;
                if transported.symbols.insert(reference, terminals).is_some() {
                    return Err(malformed("Another document defines the same symbol revision"));
                }
            } else if path.starts_with("parts/") {
                let part = decode_part(path, text)?;
                if part.library != library {
                    return Err(malformed("Part library does not match library.json"));
                }
                let reference = part.reference();
                work.reference(&reference)?;
                if !part_refs.insert(reference) {
                    return Err(malformed("Another document defines the same part revision"));
                }
                transported.parts.push((path.clone(), part));
            }
        }
        Ok(transported)
    }

    /// Terminals of an exact symbol, from the documents first and then the catalog.
    fn terminals(&self, symbol: Option<&Reference>, catalog: &Catalog, work: &mut Budget) -> Result<Vec<String>, CheckError> {
        let Some(reference) = symbol else {
            return Ok(Vec::new());
        };
        work.reference(reference)?;
        if let Some(terminals) = self.symbols.get(reference) {
            return copy_terminals(terminals, work);
        }
        let symbol = catalog.indexed_symbol(reference).ok_or_else(|| CheckError::Unresolved { kind: "symbol", reference: reference.clone() })?;
        copy_terminals(symbol.pins().iter().map(|pin| &pin.number), work)
    }
}

fn copy_terminals<'a>(terminals: impl IntoIterator<Item = &'a String>, work: &mut Budget) -> Result<Vec<String>, CheckError> {
    let mut copy = Vec::new();
    for terminal in terminals {
        work.text(terminal)?;
        copy.push(terminal.clone());
    }
    Ok(copy)
}

/// Read the format first, then decode and validate identity without checking pads or publication readiness.
fn decode_footprint(path: &str, text: &str) -> Result<FootprintDefinition, CheckError> {
    let malformed = |error: serde_json::Error| CheckError::Malformed { file: path.into(), reason: error.to_string() };
    let header: Header = serde_json::from_str(text).map_err(malformed)?;
    if !crate::board::definition::FORMATS.contains(&header.format.as_str()) {
        return Err(CheckError::UnsupportedFormat { file: path.into(), format: header.format });
    }
    let definition: FootprintDefinition = serde_json::from_str(text).map_err(malformed)?;
    if definition.validate_format().is_err() {
        return Err(CheckError::UnsupportedFormat { file: path.into(), format: definition.format });
    }
    if !definition.has_valid_identity() {
        return Err(CheckError::Malformed { file: path.into(), reason: "invalid footprint identity".into() });
    }
    Ok(definition)
}

/// Decode a transported part with native format and header checks but without strict package-map validation.
fn decode_part(path: &str, text: &str) -> Result<PartDefinition, CheckError> {
    let malformed = |error: serde_json::Error| CheckError::Malformed { file: path.into(), reason: error.to_string() };
    let header: Header = serde_json::from_str(text).map_err(malformed)?;
    if PartDefinition::supported_format(&header.format).is_none() {
        return Err(CheckError::UnsupportedFormat { file: path.into(), format: header.format });
    }
    let part: PartDefinition = serde_json::from_str(text).map_err(malformed)?;
    validate_part_format(path, &part)?;
    part.validate_header().map_err(|reason| CheckError::Malformed { file: path.into(), reason })?;
    Ok(part)
}

/// Apply the same format/content checks to a transported definition and a typed draft.
fn validate_part_format(file: &str, part: &PartDefinition) -> Result<(), CheckError> {
    part.validate_format().map_err(|error| match error {
        PartFormatError::Incompatible(_) => CheckError::UnsupportedFormat { file: file.into(), format: part.format.clone() },
        PartFormatError::InvalidContent(reason) => CheckError::Malformed { file: file.into(), reason },
    })
}

/// Keep every pad in authored order and number its same-number occurrences.
fn capture_footprint<'a>(
    reference: Reference,
    source: CaptureSource,
    pads: impl Iterator<Item = (u64, &'a RectPad)>,
    work: &mut Budget,
) -> Result<FootprintCapture, CheckError> {
    work.charge(128)?;
    work.reference(&reference)?;
    work.source(&source)?;
    let mut ordinals: BTreeMap<&str, u16> = BTreeMap::new();
    let mut occurrences = Vec::new();
    for (index, pad) in pads {
        if occurrences.len() == MAX_PADS {
            let what = format!("Pads of footprint {}/{} revision {}", reference.library, reference.id, reference.revision);
            return Err(CheckError::TooLarge { what, limit: MAX_PADS });
        }
        work.charge(256)?;
        work.text(&pad.number)?;
        let ordinal = ordinals.entry(pad.number.as_str()).or_insert(0);
        *ordinal = ordinal.saturating_add(1);
        occurrences.push(PadOccurrence { index, ordinal: *ordinal, number: pad.number.clone(), pad: pad.clone() });
    }
    Ok(FootprintCapture { reference, source, pads: occurrences })
}

/// Capture the pads of a footprint authoring draft. Non-plated holes are not pads.
fn draft_footprint(item: &Item, work: &mut Budget) -> Result<FootprintCapture, CheckError> {
    if item.kind != Kind::Footprint {
        return Err(CheckError::Malformed { file: "item".into(), reason: "Only footprint drafts carry pad occurrences".into() });
    }
    work.charge(item.objects.len())?;
    work.text(&item.library)?;
    work.text(&item.id)?;
    let pads = item.objects.iter().filter_map(|(id, primitive)| match primitive {
        Primitive::Pad { pad } => Some((*id, pad)),
        _ => None,
    });
    capture_footprint(Reference { library: item.library.clone(), id: item.id.clone(), revision: item.revision }, CaptureSource::Draft, pads, work)
}

/// Refuse an oversized package, and charge all map entries before copying the map. Repeated
/// assignments count toward aggregate work even if they add no distinct pad to the set.
fn bounded_package(part: &Reference, name: &str, package: &Package, work: &mut Budget) -> Result<(), CheckError> {
    let what = || format!("Package {name} of part {}/{} revision {}", part.library, part.id, part.revision);
    if package.mapping.len() > MAX_PACKAGE_TERMINALS {
        return Err(CheckError::TooLarge { what: what(), limit: MAX_PACKAGE_TERMINALS });
    }
    if package.unconnected_pads.len() > MAX_PADS || package.mapping.values().any(|pads| pads.len() > MAX_PADS) {
        return Err(CheckError::TooLarge { what: what(), limit: MAX_PADS });
    }
    for terminal in package.mapping.keys() {
        work.charge(64)?;
        work.text(terminal)?;
    }
    let mut distinct_pads = BTreeSet::new();
    for pad in package.mapping.values().flatten().chain(&package.unconnected_pads) {
        work.text(pad)?;
        if distinct_pads.insert(pad.as_str()) && distinct_pads.len() > MAX_PADS {
            return Err(CheckError::TooLarge { what: what(), limit: MAX_PADS });
        }
    }
    Ok(())
}

/// Refuse assignments that cannot belong to this package. Missing real terminals and
/// missing or repeated pads still reach the checker as findings.
fn validate_package_assignments(name: &str, package: &Package, has_symbol: bool, terminals: &BTreeSet<&str>, file: &str) -> Result<(), CheckError> {
    let malformed = |reason| CheckError::Malformed { file: file.into(), reason };
    if !has_symbol && (!package.mapping.is_empty() || !package.unconnected_pads.is_empty()) {
        return Err(malformed(format!("Package {name:?} has a mapping or unconnected pads but the part has no symbol.")));
    }
    if let Some(terminal) = package.mapping.keys().find(|terminal| !terminals.contains(terminal.as_str())) {
        return Err(malformed(format!("Package {name:?} maps unknown symbol terminal {terminal:?}.")));
    }
    Ok(())
}

impl Capture {
    /// Capture a whole transported library: its footprints and the packages of its parts.
    pub fn from_documents(documents: &Documents, catalog: &Catalog) -> Result<Self, CheckError> {
        Self::new(documents, None, None, catalog)
    }

    /// Capture the pads of one footprint authoring draft.
    pub fn from_item(item: &Item) -> Result<Self, CheckError> {
        Self::new(&Documents::new(), Some(item), None, &Catalog::default())
    }

    /// Capture a generic part draft, resolving its references in the documents and then the catalog.
    pub fn from_part_record(record: &Record, documents: &Documents, catalog: &Catalog) -> Result<Self, CheckError> {
        Self::new(documents, None, Some(record), catalog)
    }

    /// Capture documents and optional drafts once. Exact references resolve in the transported
    /// documents first and then in the catalog; an unresolved reference is an error.
    /// Refuses the whole capture if its work or unfiltered findings exceed the aggregate budgets.
    pub fn new(documents: &Documents, item: Option<&Item>, record: Option<&Record>, catalog: &Catalog) -> Result<Self, CheckError> {
        Self::new_bounded(documents, item, record, catalog, Budget::work(MAX_WORK), Budget::report(MAX_REPORT_BYTES))
    }

    fn new_bounded(
        documents: &Documents,
        item: Option<&Item>,
        record: Option<&Record>,
        catalog: &Catalog,
        mut work: Budget,
        mut output: Budget,
    ) -> Result<Self, CheckError> {
        let mut transported = Transported::read(documents, &mut work)?;
        let mut footprints = std::mem::take(&mut transported.footprints);
        let draft = item.map(|item| draft_footprint(item, &mut work)).transpose()?;
        let mut parts: Vec<(CaptureSource, &PartDefinition)> =
            transported.parts.iter().map(|(path, part)| (CaptureSource::Document { path: path.clone() }, part)).collect();
        match record {
            Some(Record::Part(part)) => {
                validate_part_format("record", part)?;
                parts.push((CaptureSource::Draft, part));
            }
            Some(Record::Orderable(_)) => {
                return Err(CheckError::Malformed { file: "record".into(), reason: "Only generic part drafts own package maps".into() });
            }
            None => {}
        }
        let mut packages = Vec::new();
        for (source, part) in parts {
            work.text(&part.library)?;
            work.text(&part.id)?;
            let reference = part.reference();
            if part.packages.len() > MAX_PACKAGES {
                let what = format!("Packages of part {}/{} revision {}", reference.library, reference.id, reference.revision);
                return Err(CheckError::TooLarge { what, limit: MAX_PACKAGES });
            }
            let terminals = transported.terminals(part.symbol.as_ref(), catalog, &mut work)?;
            let terminal_ids = terminals.iter().map(String::as_str).collect();
            let file = match &source {
                CaptureSource::Document { path } => path.as_str(),
                _ => "record",
            };
            for (name, package) in &part.packages {
                work.charge(256)?;
                work.text(name)?;
                work.reference(&reference)?;
                work.source(&source)?;
                if let Some(symbol) = &part.symbol {
                    work.reference(symbol)?;
                }
                let target = &package.footprint;
                work.reference(target)?;
                bounded_package(&reference, name, package, &mut work)?;
                validate_package_assignments(name, package, part.symbol.is_some(), &terminal_ids, file)?;
                if let std::collections::btree_map::Entry::Vacant(entry) = footprints.entry(target.clone()) {
                    let definition = catalog
                        .footprint(&target.library, &target.id, target.revision)
                        .ok_or_else(|| CheckError::Unresolved { kind: "footprint", reference: target.clone() })?;
                    let pads = definition.pads.iter().enumerate().map(|(index, pad)| (u64::try_from(index).unwrap_or(u64::MAX), pad));
                    let footprint = capture_footprint(target.clone(), CaptureSource::Catalog, pads, &mut work)?;
                    entry.insert(footprint);
                }
                let package_terminals = copy_terminals(&terminals, &mut work)?;
                packages.push(PackageCapture {
                    part: reference.clone(),
                    package: name.clone(),
                    source: source.clone(),
                    symbol: part.symbol.clone(),
                    footprint: target.clone(),
                    mapping: package.mapping.clone(),
                    unconnected_pads: package.unconnected_pads.clone(),
                    terminals: package_terminals,
                });
            }
        }
        let mut footprints: Vec<_> = footprints.into_values().chain(draft).collect();
        footprints.sort_by(|a, b| (&a.reference, &a.source).cmp(&(&b.reference, &b.source)));
        packages.sort_by(|a, b| (&a.part, &a.package, &a.source).cmp(&(&b.part, &b.package, &b.source)));
        // Complete the bounded evaluation before serializing anything. An immutable capture
        // retains those findings so both public check entry points only produce bounded reports.
        let findings = evaluate(&footprints, &packages, &mut work, &mut output)?;
        let mut hash = CaptureHash(Sha256::new());
        serde_json::to_writer(&mut hash, &(&footprints, &packages))
            .map_err(|error| CheckError::Malformed { file: "capture".into(), reason: error.to_string() })?;
        let digest = format!("{:x}", hash.0.finalize());
        Ok(Self { footprints, packages, digest, findings })
    }

    /// Captured footprints sorted by exact reference and source.
    pub fn footprints(&self) -> &[FootprintCapture] {
        &self.footprints
    }
    /// Captured packages sorted by part, package and source.
    pub fn packages(&self) -> &[PackageCapture] {
        &self.packages
    }
    /// SHA-256 of the canonical capture.
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

/// Stream the original compact canonical JSON into SHA-256, without a second capture-sized buffer.
struct CaptureHash(Sha256);

impl std::io::Write for CaptureHash {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Footprint occurrences grouped by pad number, each group in authored order. Charge the full
/// scan on every call, even when several packages resolve to the exact same footprint.
fn by_number<'a>(pads: &'a [PadOccurrence], work: &mut Budget) -> Result<BTreeMap<&'a str, Vec<PadOccurrenceRef>>, CheckError> {
    let mut groups: BTreeMap<&str, Vec<PadOccurrenceRef>> = BTreeMap::new();
    for pad in pads {
        work.text(&pad.number)?;
        groups.entry(pad.number.as_str()).or_default().push(PadOccurrenceRef { index: pad.index, ordinal: pad.ordinal });
    }
    Ok(groups)
}

/// The one checker with the built-in rules at the edit stage and their default severities.
pub fn check(capture: &Capture) -> Report {
    report(capture, RULES, Stage::Edit)
}

/// The one checker. GUI, MCP and publication gates call it on the same capture with their stage
/// and severity overrides; `rules` is the registered rule set, normally [`RULES`].
///
/// # Errors
/// Refuses an override of an unknown rule or one below a required rule's floor.
pub fn check_with(capture: &Capture, options: &Options, rules: &[Rule]) -> Result<Report, CheckError> {
    let rules = options.resolve(rules)?;
    Ok(report(capture, &rules, options.stage))
}

/// Keep the findings of rules that run at `stage`, give each its rule's severity and count them.
/// A rule missing from `rules` reports nothing.
fn report(capture: &Capture, rules: &[Rule], stage: Stage) -> Report {
    let mut counts = Counts::default();
    let mut blocks_publish = false;
    let findings = capture
        .findings
        .iter()
        .filter_map(|finding| {
            let rule = rules.iter().find(|rule| rule.id == finding.rule && rule.stages.contains(&stage))?;
            match rule.severity {
                Severity::Error => counts.error += 1,
                Severity::Warning => counts.warning += 1,
                Severity::Info => counts.info += 1,
            }
            blocks_publish |= rule.severity == Severity::Error && rule.stages.contains(&Stage::Publish);
            Some(Finding { severity: rule.severity, ..finding.clone() })
        })
        .collect();
    Report { capture_digest: capture.digest.clone(), stage, findings, counts, blocks_publish }
}

/// Every built-in rule's findings, sorted by rule, subject and first occurrence.
fn evaluate(
    footprints: &[FootprintCapture],
    packages: &[PackageCapture],
    work: &mut Budget,
    output: &mut Budget,
) -> Result<Vec<Finding>, CheckError> {
    output.envelope()?;
    let mut findings = Vec::new();
    let mut published = BTreeMap::new();
    for footprint in footprints {
        // Package maps name published footprints, so a draft never stands in for one.
        if footprint.source != CaptureSource::Draft {
            published.insert(&footprint.reference, footprint.pads.as_slice());
        }
        for (number, occurrences) in by_number(&footprint.pads, work)?.into_iter().filter(|(_, occurrences)| occurrences.len() > 1) {
            output.finding(&footprint.reference, &footprint.source, None, number, occurrences.len())?;
            let subject = Subject::Pad { footprint: footprint.reference.clone(), number: number.into(), source: footprint.source.clone() };
            findings.push(Finding::new(RuleId::PadNumberRepeated, subject, occurrences.len(), occurrences));
        }
    }
    for package in packages {
        let pads = published.get(&package.footprint).copied().unwrap_or_default();
        let footprint = by_number(pads, work)?;
        // Capture already charged this package's materialized terminals and every map
        // entry, paying for the bounded passes over those inputs here as well.
        let pad = |number: &str| Subject::PackagePad {
            part: package.part.clone(),
            package: package.package.clone(),
            number: number.into(),
            source: package.source.clone(),
        };
        let terminals: BTreeSet<&str> = package.terminals.iter().map(String::as_str).collect();
        for terminal in terminals {
            let mapped = package.mapping.get(terminal).map_or(0, Vec::len);
            if mapped == 0 {
                output.finding(&package.part, &package.source, Some(&package.package), terminal, 0)?;
                let subject = Subject::Terminal {
                    part: package.part.clone(),
                    package: package.package.clone(),
                    terminal: terminal.into(),
                    source: package.source.clone(),
                };
                findings.push(Finding::new(RuleId::PackageTerminalUnmapped, subject, mapped, Vec::new()));
            }
        }
        let mut assigned: BTreeMap<&str, usize> = BTreeMap::new();
        for number in package.mapping.values().flatten().chain(&package.unconnected_pads) {
            *assigned.entry(number.as_str()).or_default() += 1;
        }
        if package.symbol.is_some() {
            for (number, occurrences) in &footprint {
                if !assigned.contains_key(number) {
                    output.finding(&package.part, &package.source, Some(&package.package), number, occurrences.len())?;
                    findings.push(Finding::new(RuleId::PackagePadUnaccounted, pad(number), 0, occurrences.clone()));
                }
            }
        }
        for (number, count) in &assigned {
            let occurrences = footprint.get(number).map_or(&[][..], Vec::as_slice);
            if occurrences.is_empty() {
                output.finding(&package.part, &package.source, Some(&package.package), number, 0)?;
                findings.push(Finding::new(RuleId::PackagePadNotInFootprint, pad(number), 0, Vec::new()));
            }
            if *count > 1 {
                output.finding(&package.part, &package.source, Some(&package.package), number, occurrences.len())?;
                findings.push(Finding::new(RuleId::PackagePadRepeated, pad(number), *count, occurrences.to_vec()));
            }
        }
    }
    findings.sort_by(|a, b| (a.rule, &a.subject, a.occurrences.first()).cmp(&(b.rule, &b.subject, b.occurrences.first())));
    Ok(findings)
}

/// Decode `library.check` parameters, capture once and return the serialized report.
pub(crate) fn execute(params: &Value, catalog: &Catalog) -> Result<Value, String> {
    let documents: Documents = serde_json::from_value(params["documents"].clone()).map_err(|error| error.to_string())?;
    let item = params
        .get("item")
        .map(|item| serde_json::to_vec(item).map_err(|error| error.to_string()).and_then(|bytes| Item::decode(&bytes)))
        .transpose()?;
    let record =
        params.get("record").map(|record| serde_json::from_value::<Record>(record.clone()).map_err(|error| error.to_string())).transpose()?;
    let options = Options {
        stage: params
            .get("stage")
            .map(|stage| serde_json::from_value(stage.clone()))
            .transpose()
            .map_err(|error| error.to_string())?
            .unwrap_or_default(),
        severity: params
            .get("severity")
            .map(|severity| serde_json::from_value(severity.clone()))
            .transpose()
            .map_err(|error| error.to_string())?
            .unwrap_or_default(),
    };
    let capture = Capture::new(&documents, item.as_ref(), record.as_ref(), catalog).map_err(|error| error.to_string())?;
    let report = check_with(&capture, &options, RULES).map_err(|error| error.to_string())?;
    serde_json::to_value(report).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests;
