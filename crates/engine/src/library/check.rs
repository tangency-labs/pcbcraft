//! Read-only library checks: typed findings over exactly authored pad occurrences and package maps.
//!
//! Strict loaders, editors and publication keep refusing every defect reported here. The check
//! only explains one: it decodes the transported documents structurally, keeps every authored pad
//! in its authored order without deduplication, and reports each defect with its measured count,
//! limit and basis. It writes no file, changes no project and records no history step.
use super::{
    parse_manifest, parse_symbol,
    parts::{LEGACY_PART_FORMAT, PART_FORMAT, Package, PartDefinition, Reference},
    sources::{Catalog, Documents, MAX_FILE_BYTES, check_documents},
};
use crate::{
    authoring::{Item, Kind, Primitive},
    board::definition::{FootprintDefinition, RectPad},
    part_authoring::Record,
    persistence::reject_duplicate_json_keys,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// Report typed findings for transported documents and optional drafts without changing anything.
pub const CHECK: &str = "library.check";
/// Pads in one captured footprint, the native format's own limit; also the most pads per
/// terminal or unconnected pads in one captured package.
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
    pub footprint: Reference,
    pub mapping: BTreeMap<String, Vec<String>>,
    pub unconnected_pads: Vec<String>,
    /// Logical terminals in native symbol order; empty for a part without a symbol.
    pub terminals: Vec<String>,
}

/// An immutable capture of everything one check reads. Its digest binds a report to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capture {
    footprints: Vec<FootprintCapture>,
    packages: Vec<PackageCapture>,
    digest: String,
}

/// A capture that cannot be checked. There is no partial or clean report for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckError {
    /// Invalid JSON, an unknown or repeated field, a wrong library or a repeated identity.
    Malformed { file: String, reason: String },
    /// A format the check does not read, or one that does not match the authored content.
    UnsupportedFormat { file: String, format: String },
    /// An exact reference found neither in the transported documents nor in the catalog.
    Unresolved { kind: &'static str, reference: Reference },
    /// More data than the native formats allow.
    TooLarge { what: String, limit: usize },
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
    /// A footprint pad is neither mapped nor explicitly unconnected.
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
            Self::PadNumberRepeated => Basis::ProjectRule { doc: "docs/footprint-schema.md", section: "Semantic validation" },
            Self::PackageTerminalUnmapped | Self::PackagePadUnaccounted | Self::PackagePadNotInFootprint | Self::PackagePadRepeated => {
                Basis::ProjectRule { doc: "docs/part-schema.md", section: "Part fields" }
            }
        }
    }
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
    pub subject: Subject,
    /// Every footprint occurrence involved, in authored order; empty when none exists.
    pub occurrences: Vec<PadOccurrenceRef>,
    pub measured: Quantity,
    pub limit: Limit,
    pub basis: Basis,
}

impl Finding {
    /// Fill the limit and basis from the rule, so one rule cannot report two limits.
    fn new(rule: RuleId, subject: Subject, measured: usize, occurrences: Vec<PadOccurrenceRef>) -> Self {
        Self {
            rule,
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
    findings: Vec<Finding>,
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
}

/// The only field read before the full structural decode, so a format is named before its fields.
#[derive(Deserialize)]
struct Header {
    format: String,
}

/// The transported documents, decoded structurally and bound to their manifest.
#[derive(Default)]
struct Transported {
    footprints: Vec<FootprintCapture>,
    symbols: BTreeMap<Reference, Vec<String>>,
    parts: Vec<(String, PartDefinition)>,
}

impl Transported {
    /// Decode every footprint, symbol and part document. Categories and manufacturer parts own no
    /// pads or package maps and are not read.
    fn read(documents: &Documents) -> Result<Self, CheckError> {
        for (path, text) in documents {
            if text.len() > MAX_FILE_BYTES {
                return Err(CheckError::TooLarge { what: path.clone(), limit: MAX_FILE_BYTES });
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
        for (path, text) in documents.iter().filter(|(path, _)| path.as_str() != "library.json") {
            let malformed = |reason: &str| CheckError::Malformed { file: path.clone(), reason: reason.into() };
            let library = library.as_deref().ok_or_else(|| malformed("library.json is required to bind document identity"))?;
            if path.starts_with("footprints/") {
                let definition = decode_footprint(path, text)?;
                if definition.library != library {
                    return Err(malformed("Footprint library does not match library.json"));
                }
                let reference = Reference { library: definition.library.clone(), id: definition.id.clone(), revision: definition.revision };
                if transported.footprints.iter().any(|footprint| footprint.reference == reference) {
                    return Err(malformed("Another document defines the same footprint revision"));
                }
                let pads = definition.pads.iter().enumerate().map(|(index, pad)| (u64::try_from(index).unwrap_or(u64::MAX), pad));
                transported.footprints.push(capture_footprint(reference, CaptureSource::Document { path: path.clone() }, pads)?);
            } else if path.starts_with("symbols/") {
                reject_duplicate_json_keys(text).map_err(|reason| malformed(&reason))?;
                let symbol = parse_symbol(text).map_err(|reason| malformed(&reason))?;
                let reference = Reference { library: library.into(), id: symbol.id.clone(), revision: symbol.revision() };
                let terminals = symbol.pins().iter().map(|pin| pin.number.clone()).collect();
                if transported.symbols.insert(reference, terminals).is_some() {
                    return Err(malformed("Another document defines the same symbol revision"));
                }
            } else if path.starts_with("parts/") {
                let part = decode_part(path, text)?;
                if part.library != library {
                    return Err(malformed("Part library does not match library.json"));
                }
                transported.parts.push((path.clone(), part));
            }
        }
        Ok(transported)
    }

    /// Terminals of an exact symbol, from the documents first and then the catalog.
    fn terminals(&self, symbol: Option<&Reference>, catalog: &Catalog) -> Result<Vec<String>, CheckError> {
        let Some(reference) = symbol else {
            return Ok(Vec::new());
        };
        if let Some(terminals) = self.symbols.get(reference) {
            return Ok(terminals.clone());
        }
        catalog
            .symbol(&reference.library, &reference.id, reference.revision)
            .map(|symbol| symbol.pins().iter().map(|pin| pin.number.clone()).collect())
            .ok_or_else(|| CheckError::Unresolved { kind: "symbol", reference: reference.clone() })
    }
}

/// Read the format first, then decode every field without semantic validation.
fn decode_footprint(path: &str, text: &str) -> Result<FootprintDefinition, CheckError> {
    let malformed = |error: serde_json::Error| CheckError::Malformed { file: path.into(), reason: error.to_string() };
    let header: Header = serde_json::from_str(text).map_err(malformed)?;
    if header.format != FootprintDefinition::format_for(false) && header.format != FootprintDefinition::format_for(true) {
        return Err(CheckError::UnsupportedFormat { file: path.into(), format: header.format });
    }
    let definition: FootprintDefinition = serde_json::from_str(text).map_err(malformed)?;
    if definition.format != FootprintDefinition::format_for(!definition.non_plated_holes.is_empty()) {
        return Err(CheckError::UnsupportedFormat { file: path.into(), format: definition.format });
    }
    Ok(definition)
}

/// Read the format first, then decode the part without its local map validation.
fn decode_part(path: &str, text: &str) -> Result<PartDefinition, CheckError> {
    let malformed = |error: serde_json::Error| CheckError::Malformed { file: path.into(), reason: error.to_string() };
    let header: Header = serde_json::from_str(text).map_err(malformed)?;
    if header.format != PART_FORMAT && header.format != LEGACY_PART_FORMAT {
        return Err(CheckError::UnsupportedFormat { file: path.into(), format: header.format });
    }
    serde_json::from_str(text).map_err(malformed)
}

/// Keep every pad in authored order and number its same-number occurrences.
fn capture_footprint<'a>(
    reference: Reference,
    source: CaptureSource,
    pads: impl Iterator<Item = (u64, &'a RectPad)>,
) -> Result<FootprintCapture, CheckError> {
    let mut ordinals: BTreeMap<String, u16> = BTreeMap::new();
    let mut occurrences = Vec::new();
    for (index, pad) in pads {
        if occurrences.len() == MAX_PADS {
            let what = format!("Pads of footprint {}/{} revision {}", reference.library, reference.id, reference.revision);
            return Err(CheckError::TooLarge { what, limit: MAX_PADS });
        }
        let ordinal = ordinals.entry(pad.number.clone()).or_insert(0);
        *ordinal = ordinal.saturating_add(1);
        occurrences.push(PadOccurrence { index, ordinal: *ordinal, number: pad.number.clone(), pad: pad.clone() });
    }
    Ok(FootprintCapture { reference, source, pads: occurrences })
}

/// Capture the pads of a footprint authoring draft. Non-plated holes are not pads.
fn draft_footprint(item: &Item) -> Result<FootprintCapture, CheckError> {
    if item.kind != Kind::Footprint {
        return Err(CheckError::Malformed { file: "item".into(), reason: "Only footprint drafts carry pad occurrences".into() });
    }
    let pads = item.objects.iter().filter_map(|(id, primitive)| match primitive {
        Primitive::Pad { pad } => Some((*id, pad)),
        _ => None,
    });
    capture_footprint(Reference { library: item.library.clone(), id: item.id.clone(), revision: item.revision }, CaptureSource::Draft, pads)
}

/// Refuse a package larger than the part formats allow before it is captured.
fn bounded_package(part: &Reference, name: &str, package: &Package) -> Result<(), CheckError> {
    let what = || format!("Package {name} of part {}/{} revision {}", part.library, part.id, part.revision);
    if package.mapping.len() > MAX_PACKAGE_TERMINALS {
        return Err(CheckError::TooLarge { what: what(), limit: MAX_PACKAGE_TERMINALS });
    }
    if package.unconnected_pads.len() > MAX_PADS || package.mapping.values().any(|pads| pads.len() > MAX_PADS) {
        return Err(CheckError::TooLarge { what: what(), limit: MAX_PADS });
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
    pub fn new(documents: &Documents, item: Option<&Item>, record: Option<&Record>, catalog: &Catalog) -> Result<Self, CheckError> {
        let transported = Transported::read(documents)?;
        let mut footprints = transported.footprints.clone();
        if let Some(item) = item {
            footprints.push(draft_footprint(item)?);
        }
        let mut parts: Vec<(CaptureSource, &PartDefinition)> =
            transported.parts.iter().map(|(path, part)| (CaptureSource::Document { path: path.clone() }, part)).collect();
        match record {
            Some(Record::Part(part)) => parts.push((CaptureSource::Draft, part)),
            Some(Record::Orderable(_)) => {
                return Err(CheckError::Malformed { file: "record".into(), reason: "Only generic part drafts own package maps".into() });
            }
            None => {}
        }
        let mut packages = Vec::new();
        for (source, part) in parts {
            let reference = part.reference();
            if part.packages.len() > MAX_PACKAGES {
                let what = format!("Packages of part {}/{} revision {}", reference.library, reference.id, reference.revision);
                return Err(CheckError::TooLarge { what, limit: MAX_PACKAGES });
            }
            let terminals = transported.terminals(part.symbol.as_ref(), catalog)?;
            for (name, package) in &part.packages {
                bounded_package(&reference, name, package)?;
                let target = &package.footprint;
                let known = footprints.iter().any(|footprint| footprint.reference == *target && footprint.source != CaptureSource::Draft);
                if !known {
                    let definition = catalog
                        .footprint(&target.library, &target.id, target.revision)
                        .ok_or_else(|| CheckError::Unresolved { kind: "footprint", reference: target.clone() })?;
                    let pads = definition.pads.iter().enumerate().map(|(index, pad)| (u64::try_from(index).unwrap_or(u64::MAX), pad));
                    footprints.push(capture_footprint(target.clone(), CaptureSource::Catalog, pads)?);
                }
                packages.push(PackageCapture {
                    part: reference.clone(),
                    package: name.clone(),
                    source: source.clone(),
                    footprint: target.clone(),
                    mapping: package.mapping.clone(),
                    unconnected_pads: package.unconnected_pads.clone(),
                    terminals: terminals.clone(),
                });
            }
        }
        footprints.sort_by(|a, b| (&a.reference, &a.source).cmp(&(&b.reference, &b.source)));
        packages.sort_by(|a, b| (&a.part, &a.package, &a.source).cmp(&(&b.part, &b.package, &b.source)));
        let canonical = serde_json::to_vec(&(&footprints, &packages))
            .map_err(|error| CheckError::Malformed { file: "capture".into(), reason: error.to_string() })?;
        let digest = format!("{:x}", Sha256::digest(&canonical));
        Ok(Self { footprints, packages, digest })
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

/// Footprint occurrences grouped by pad number, each group in authored order.
fn by_number(pads: &[PadOccurrence]) -> BTreeMap<&str, Vec<PadOccurrenceRef>> {
    let mut groups: BTreeMap<&str, Vec<PadOccurrenceRef>> = BTreeMap::new();
    for pad in pads {
        groups.entry(pad.number.as_str()).or_default().push(PadOccurrenceRef { index: pad.index, ordinal: pad.ordinal });
    }
    groups
}

/// The one checker. GUI, MCP and publication gates call it on the same capture.
pub fn check(capture: &Capture) -> Report {
    let mut findings = Vec::new();
    for footprint in &capture.footprints {
        for (number, occurrences) in by_number(&footprint.pads).into_iter().filter(|(_, occurrences)| occurrences.len() > 1) {
            let subject = Subject::Pad { footprint: footprint.reference.clone(), number: number.into(), source: footprint.source.clone() };
            findings.push(Finding::new(RuleId::PadNumberRepeated, subject, occurrences.len(), occurrences));
        }
    }
    for package in &capture.packages {
        // Package maps name published footprints, so a draft never stands in for one.
        let pads = capture
            .footprints
            .iter()
            .find(|footprint| footprint.reference == package.footprint && footprint.source != CaptureSource::Draft)
            .map(|footprint| footprint.pads.as_slice())
            .unwrap_or_default();
        let footprint = by_number(pads);
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
        for (number, occurrences) in &footprint {
            if !assigned.contains_key(number) {
                findings.push(Finding::new(RuleId::PackagePadUnaccounted, pad(number), 0, occurrences.clone()));
            }
        }
        for (number, count) in &assigned {
            let occurrences = footprint.get(number).cloned().unwrap_or_default();
            if occurrences.is_empty() {
                findings.push(Finding::new(RuleId::PackagePadNotInFootprint, pad(number), 0, Vec::new()));
            }
            if *count > 1 {
                findings.push(Finding::new(RuleId::PackagePadRepeated, pad(number), *count, occurrences));
            }
        }
    }
    findings.sort_by(|a, b| (a.rule, &a.subject, a.occurrences.first()).cmp(&(b.rule, &b.subject, b.occurrences.first())));
    Report { capture_digest: capture.digest.clone(), findings }
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
    let capture = Capture::new(&documents, item.as_ref(), record.as_ref(), catalog).map_err(|error| error.to_string())?;
    serde_json::to_value(check(&capture)).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests;
