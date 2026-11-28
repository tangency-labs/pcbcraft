//! Import one self-contained KiCad schematic as a new native project (#60).
//!
//! The file is read by `pcbcraft-kicad` into a KiCad-shaped model. Its library symbols become
//! native `pcbcraft-symbol/4` documents in a disposable library source, and the project is built
//! in a scratch [`Session`] through the ordinary commands: placing symbols and units, setting
//! fields, drawing wires, placing labels and marking no-connects. The importing session only
//! ever receives the finished project, as one undoable step, or nothing.
//!
//! The import is exact or refused. Whatever would change which pins connect refuses the whole
//! file with a list of up to 20 reasons: sub-sheets, buses, hierarchical labels, diagonal wires, symbols
//! the native format can't represent, parts shown in another body style than the normal one,
//! and pins that touch without a wire. The finished project
//! is checked against KiCad's nets, worked out from the file alone. Text, drawings and other
//! appearance the native model doesn't hold are left out and listed in the [`Report`].

pub mod hierarchy;
mod symbols;
mod wiring;

#[cfg(test)]
mod tests;

use crate::library::sources::{Kind, LOAD};
use crate::schematic::fields::{Layout, Layouts};
use crate::schematic::{MAX_CENTER_NM, SymbolPlacement, Terminal, TextAlign, Wire, WireEndpoint, check_reference, check_value};
use crate::sheets::page::{Orientation, Page, Paper, SET_PAGE};
use crate::{CommandError, PLACE_SYMBOL, PLACE_UNIT, Project, Session, SheetTarget};
use pcbcraft_kicad::Point;
use pcbcraft_kicad::schematic::{LabelKind, Mirror, PlacedSymbol, Schematic};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Replace the open project with one imported from a KiCad schematic's text. Undoable.
pub const IMPORT: &str = "project.import_kicad_schematic";

/// The largest schematic text accepted, in bytes. Smaller than the reader's own bound so that
/// the escaped JSON parameter fits every transport's 16 MiB message limit.
pub const MAX_TEXT_BYTES: usize = 8 << 20;

/// The most placed symbols this slice imports. Each placement and wire runs through the ordinary
/// commands, which cost a few milliseconds each; the bound keeps an import within seconds in a
/// window that can't cancel it.
pub const MAX_SYMBOLS: usize = 1000;
/// The most bytes the converted symbol documents may take together: a library source's limit,
/// less room for its manifest.
const MAX_SYMBOL_BYTES: usize = crate::library::sources::MAX_SOURCE_BYTES - (64 << 10);
/// The most straight wire pieces this slice imports, for the same reason.
pub const MAX_WIRE_PIECES: usize = 4000;

/// The start of the library identity converted symbols are pinned under. The rest is a digest
/// of the converted documents, so one identity, item and revision always means one content,
/// as native sources require, and imports of different schematics never conflict.
const LIBRARY_PREFIX: &str = "kicad-";
/// The scratch session's source registration.
const SOURCE: &str = "kicad-import";
/// The report line for a paper the sheet can't take where the file puts it.
const PAPER_REPLACED: &str = "paper sizes other than A4 to A1 replaced by the smallest that holds the drawing";
/// The report line for a drawing that reaches past the paper the import could give it.
const PAPER_SHORT: &str = "drawings reaching past the replacement paper";

/// What an import left out or changed, by description, with how often.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    notes: BTreeMap<String, usize>,
}

impl Report {
    /// Count one more of `what`.
    fn note(&mut self, what: &str) {
        self.add(what, 1);
    }

    /// Count `count` more of `what`.
    fn add(&mut self, what: &str, count: usize) {
        if count > 0 {
            let total = self.notes.entry(what.to_owned()).or_default();
            *total = total.saturating_add(count);
        }
    }

    /// Each description with its count, ordered by description.
    pub fn notes(&self) -> impl Iterator<Item = (&str, usize)> {
        self.notes.iter().map(|(what, count)| (what.as_str(), *count))
    }

    /// The report as command output.
    pub fn to_json(&self) -> Value {
        Value::Array(self.notes().map(|(what, count)| json!({"what": what, "count": count})).collect())
    }
}

/// A finished import: the project and what it left out.
#[derive(Debug, Clone)]
pub struct Imported {
    /// The imported project, ready to adopt or save.
    pub project: Arc<Project>,
    /// What the import left out or changed.
    pub report: Report,
}

/// The reasons a file can't be imported exactly, gathered before any project is built.
#[derive(Default)]
struct Refusals(Vec<String>);

impl Refusals {
    /// Record a reason once; reasons past the twentieth are dropped to keep the message short.
    fn add(&mut self, reason: String) {
        if self.0.len() < 20 && !self.0.contains(&reason) {
            self.0.push(reason);
        }
    }

    /// An error listing every reason, or `Ok` when there is none.
    fn check(self) -> Result<(), CommandError> {
        if self.0.is_empty() {
            return Ok(());
        }
        Err(CommandError::InvalidParams(format!(
            "This KiCad schematic can't be imported exactly:\n{}",
            self.0.iter().map(|r| format!("- {r}")).collect::<Vec<_>>().join("\n")
        )))
    }
}

/// Name the import step in an error from a command run in the scratch session. A limit keeps
/// its `cap_exceeded` code; anything else is a refusal of the file, whatever the inner command
/// called it, since the scratch session's own state is never the caller's concern.
fn step(what: &str) -> impl Fn(CommandError) -> CommandError + '_ {
    move |error| {
        let message = format!("This KiCad schematic can't be imported exactly: {what}: {error}");
        match error {
            CommandError::CapExceeded(_) => CommandError::CapExceeded(message),
            _ => CommandError::InvalidParams(message),
        }
    }
}

/// A placed symbol's pose as native quarter turns and mirror, or `None` for a turn KiCad allows
/// and native symbols don't (other than a quarter turn).
///
/// KiCad turns a symbol counterclockwise and then mirrors it about its own axis; native symbols
/// are mirrored left to right first and then turned. Mirroring left to right after a turn of
/// `a` is mirroring first and turning by `-a`; mirroring top to bottom is that and half a turn.
fn pose(symbol: &PlacedSymbol) -> Option<(u8, bool)> {
    if symbol.rotation_mdeg % 90_000 != 0 {
        return None;
    }
    let turns = (symbol.rotation_mdeg / 90_000) as u8 % 4;
    Some(match symbol.mirror {
        Mirror::None => (turns, false),
        Mirror::Y => ((4 - turns) % 4, true),
        Mirror::X => ((4 - (turns + 2) % 4) % 4, true),
    })
}

/// Where a pin of a placed symbol connects on the sheet, by KiCad's own rules: the library
/// point (y upward) turned counterclockwise, mirrored about the symbol's axis, then set on the
/// y-down sheet at the symbol's origin.
fn kicad_pin_position(symbol: &PlacedSymbol, at: Point) -> Option<Point> {
    let turns = (symbol.rotation_mdeg / 90_000) % 4;
    let (mut x, mut y) = (at.x_nm, at.y_nm);
    for _ in 0..turns {
        (x, y) = (-y, x);
    }
    match symbol.mirror {
        Mirror::None => {}
        Mirror::X => y = -y,
        Mirror::Y => x = -x,
    }
    Some(Point { x_nm: symbol.at.x_nm.checked_add(x)?, y_nm: symbol.at.y_nm.checked_sub(y)? })
}

/// A sheet point as a command parameter.
fn at(p: Point) -> Value {
    json!({"x_nm": p.x_nm, "y_nm": p.y_nm})
}

/// Import `text` as a new project named `name`. Fails, with every reason found, when the file
/// can't be read or imported exactly; nothing outside the returned value changes either way.
pub fn import(text: &str, name: &str) -> Result<Imported, CommandError> {
    if text.len() > MAX_TEXT_BYTES {
        return Err(CommandError::CapExceeded(format!("A KiCad schematic may be at most {} MiB.", MAX_TEXT_BYTES >> 20)));
    }
    let schematic =
        pcbcraft_kicad::read_schematic(text).map_err(|e| CommandError::InvalidParams(format!("The KiCad schematic can't be read: {e}")))?;
    import_sheet(schematic, name, None).map(|(imported, _)| imported)
}

type SheetNets = BTreeSet<BTreeSet<hierarchy::Contact>>;

fn import_sheet(mut schematic: Schematic, name: &str, scope: Option<(Arc<Project>, SheetTarget)>) -> Result<(Imported, SheetNets), CommandError> {
    let mut interface_labels = Vec::new();
    let hierarchical = scope.is_some();
    if hierarchical {
        // The tree builder has already created the native ports and their repeated anchors.
        schematic.sheets.clear();
        schematic.labels.retain(|label| {
            if label.kind == LabelKind::Hierarchical {
                interface_labels.push((label.name.clone(), label.at));
                false
            } else {
                true
            }
        });
    }
    let mut report = Report::default();
    for (what, count) in schematic.notes.iter() {
        report.add(what, count);
    }
    let mut refusals = Refusals::default();
    // Whatever the reader couldn't read and could carry a connection refuses the file.
    for (what, count) in schematic.lost.iter() {
        refusals.add(if count > 1 { format!("{what} ({count}×)") } else { what.to_owned() });
    }
    // Before power flags are set aside, since a flag's pins depend on its body style too.
    refuse_body_styles(&schematic, &mut refusals);
    // Power flags only tell KiCad's electrical rules check that a net is driven and have no part
    // of their own, so they are left out; flags that, alone or together, join wires that would
    // otherwise stay apart are refused below.
    // Each library symbol is classified once; derived symbols share their parent's answer.
    let mut by_drawing: BTreeMap<*const Vec<pcbcraft_kicad::schematic::SymbolUnit>, bool> = BTreeMap::new();
    let flag_symbols: BTreeSet<String> = schematic
        .lib_symbols
        .iter()
        .filter(|(_, lib)| lib.power && *by_drawing.entry(Arc::as_ptr(&lib.units)).or_insert_with(|| is_power_flag(lib)))
        .map(|(name, _)| name.clone())
        .collect();
    let flags: Vec<PlacedSymbol> = schematic.symbols.iter().filter(|s| flag_symbols.contains(&s.lib_symbol)).cloned().collect();
    schematic.symbols.retain(|s| !flag_symbols.contains(&s.lib_symbol));
    report.add("power flags (KiCad electrical-rules markers) left out", flags.len());
    for (count, limit, what) in
        [(schematic.symbols.len(), MAX_SYMBOLS, "placed symbols"), (schematic.labels.len(), crate::net_labels::MAX_LABELS, "labels")]
    {
        if count > limit {
            return Err(CommandError::CapExceeded(format!("This KiCad schematic has {count} {what}; an import takes at most {limit} for now.")));
        }
    }
    refuse_unsupported(&schematic, &mut refusals);
    report.add("sheet texts left out", schematic.texts.len());
    report.add("sheet drawings left out", schematic.graphics.len());

    // Convert each library symbol a placed symbol uses, under a distinct native ID.
    let mut converted: BTreeMap<&str, symbols::Converted> = BTreeMap::new();
    let mut ids: BTreeSet<String> = BTreeSet::new();
    let used: BTreeSet<&str> = schematic.symbols.iter().map(|s| s.lib_symbol.as_str()).collect();
    // One more than the limit, as the budget must stay above zero: a file costing exactly the
    // limit imports.
    let mut budget = symbols::MAX_DRAWING_WORK + 1;
    let mut bytes = 0usize;
    for name in used {
        let Some(symbol) = schematic.lib_symbols.get(name) else {
            refusals.add(format!("placed symbols use `{name}`, which the file's symbol library doesn't hold"));
            continue;
        };
        let base = symbols::native_id(name);
        let id = (1..).map(|n| if n == 1 { base.clone() } else { format!("{base}-{n}") }).find(|id| !ids.contains(id)).unwrap_or(base);
        ids.insert(id.clone());
        match symbols::convert(symbol, id, &mut budget, &mut report, hierarchical) {
            Ok(symbol) => {
                bytes = bytes.saturating_add(symbol.text.len());
                converted.insert(name, symbol);
            }
            Err(reason) => refusals.add(format!("symbol `{name}`: {reason}")),
        }
        if budget == 0 {
            return Err(CommandError::CapExceeded(
                "This KiCad schematic's symbols take more than 2,000,000 points of drawing, pins and units to convert, counting each unit; an import takes at most that for now."
                    .into(),
            ));
        }
        // The converted symbols load as one library source, which holds at most this much.
        if bytes > MAX_SYMBOL_BYTES {
            return Err(CommandError::CapExceeded(format!(
                "This KiCad schematic's symbols come to more than {} MiB as native documents; an import takes at most that for now.",
                MAX_SYMBOL_BYTES >> 20
            )));
        }
    }
    // Pins are read only for what conversion has bounded, and for the power flags, whose one
    // pin number is checked when they are recognized. A refused symbol has no pins, so the
    // power-flag reasons below are complete only when every placed symbol converted.
    let pins = LibraryPins::new(&schematic, converted.keys().copied().chain(flags.iter().map(|f| f.lib_symbol.as_str())));
    refuse_name_clashes(&schematic, &converted, &mut refusals);
    check_values(&schematic, &converted, &mut refusals, &mut report);
    // One sheet is the whole schematic here (sub-sheets are refused above), so a net that stays
    // on its sheet reaches the same pins as a global one. Importing sub-sheets must keep the
    // local scope or refuse these symbols.
    let local_power = schematic
        .symbols
        .iter()
        .filter(|s| converted.get(s.lib_symbol.as_str()).is_some_and(|c| c.power_terminal.is_some()))
        .filter(|s| schematic.lib_symbols.get(&s.lib_symbol).is_some_and(|lib| lib.power_local))
        .count();
    if hierarchical {
        report.add("local power symbols kept with sheet-scoped net anchors", local_power);
    } else {
        report.add("local power symbols imported as global ones (the same net on a single sheet)", local_power);
    }
    let poses: Vec<Option<(u8, bool)>> = schematic.symbols.iter().map(pose).collect();
    for (symbol, pose) in schematic.symbols.iter().zip(&poses) {
        if pose.is_none() {
            refusals.add(format!("{} is turned by other than a quarter turn", symbol.reference));
        }
        if let Some(c) = converted.get(symbol.lib_symbol.as_str())
            && !c.units.contains_key(&symbol.unit)
        {
            refusals.add(format!("{} places unit {}, which its symbol doesn't have", symbol.reference, symbol.unit));
        }
    }
    let segments = match wiring::segments(&schematic.wires.iter().map(|w| w.points.clone()).collect::<Vec<_>>()) {
        Ok(segments) => segments,
        Err(p) => {
            refusals.add(format!("a wire at ({:.2} mm, {:.2} mm) is diagonal", p.x_nm as f64 / 1e6, p.y_nm as f64 / 1e6));
            Vec::new()
        }
    };
    if !flags.is_empty() && segments.len() <= MAX_WIRE_PIECES {
        match flags_that_join(&schematic, &flags, &segments, &converted, &pins)? {
            Joining::None => {}
            Joining::Each(culprits) => {
                for flag in culprits {
                    refusals.add(format!("power flag {flag} sits on a crossing without a dot and joins its wires; left out, they would come apart"));
                }
            }
            Joining::Together { named, more } => {
                let (how, more) = if more > 0 { ("alone or together", format!(" and {more} more")) } else { ("together", String::new()) };
                refusals.add(format!(
                    "power flags on crossings without a dot join wires {how}; left out, they would come apart (flags that could: {}{more})",
                    named.join(", ")
                ));
            }
        }
    }
    if segments.len() > MAX_WIRE_PIECES {
        return Err(CommandError::CapExceeded(format!(
            "This KiCad schematic has {} wire pieces; an import takes at most {MAX_WIRE_PIECES} for now.",
            segments.len()
        )));
    }
    refusals.check()?;

    let mut builder = Builder::new(name, &converted, scope)?;
    let placed = builder.place(&schematic, &converted, &pins, &poses, &mut report)?;
    builder.wire(&schematic, &segments, &placed, &converted, &pins, &mut report)?;
    builder.page(schematic.paper.as_ref(), &mut report)?;
    let project = builder.session.snapshot().ok_or_else(|| CommandError::InvalidParams("the imported project is missing".into()))?;
    let groups = if hierarchical {
        let sheet = builder.project()?;
        hierarchy::source_nets(&schematic, &segments, &placed, &converted, &sheet, &interface_labels)?
    } else {
        check_nets(&schematic, &segments, &placed, &converted, &project)?;
        BTreeSet::new()
    };
    Ok((Imported { project, report }, groups))
}

/// The order to place symbols in: first every symbol that keeps its KiCad reference, then the
/// ones numbered automatically (unannotated, invalid, a copy of a unit already placed, or a
/// reference another library symbol claimed first). Automatic numbers then can't take a
/// reference a later symbol keeps. Each group stays in file order.
fn placement_order(schematic: &Schematic) -> Vec<usize> {
    let mut claimed: BTreeMap<&str, &str> = BTreeMap::new();
    let mut units: BTreeSet<(&str, u32)> = BTreeSet::new();
    let (mut keep, mut renumber) = (Vec::new(), Vec::new());
    for (index, symbol) in schematic.symbols.iter().enumerate() {
        let reference = symbol.reference.as_str();
        let keeps = !reference.ends_with('?')
            && check_reference(reference).is_ok()
            && *claimed.entry(reference).or_insert(symbol.lib_symbol.as_str()) == symbol.lib_symbol
            && units.insert((reference, symbol.unit));
        if keeps { keep.push(index) } else { renumber.push(index) }
    }
    keep.extend(renumber);
    keep
}

/// The file's nets as groups of `(symbol index, pin number)`, worked out from its geometry with
/// [`wiring::kicad_nets`], with `extra` pins (keyed past the placed symbols) joining in too.
fn file_nets(
    schematic: &Schematic,
    segments: &[wiring::Segment],
    converted: &BTreeMap<&str, symbols::Converted>,
    pins: &LibraryPins,
    extra: &[(usize, String, Point)],
) -> Result<BTreeSet<BTreeSet<(usize, String)>>, CommandError> {
    let mut contacts = wiring::Contacts { pins: Vec::new(), labels: Vec::new(), power: Vec::new() };
    let mut cuts: BTreeSet<Point> = schematic.junctions.iter().map(|m| m.at).collect();
    for (index, symbol) in schematic.symbols.iter().enumerate() {
        let power = converted.get(symbol.lib_symbol.as_str()).and_then(|c| c.power_terminal.as_deref());
        for (number, at) in pins_of(symbol, pins)? {
            if power == Some(number.as_str())
                && let Some(value) = symbol.property("Value")
            {
                contacts.power.push(((index, number.clone()), value.to_owned()));
            }
            cuts.insert(at);
            contacts.pins.push(((index, number), at));
        }
    }
    for (key, number, at) in extra {
        cuts.insert(*at);
        contacts.pins.push(((*key, number.clone()), *at));
    }
    for label in &schematic.labels {
        contacts.labels.push((if label.kind == LabelKind::Global { "global" } else { "local" }, label.name.clone(), label.at));
        cuts.insert(label.at);
    }
    Ok(wiring::kicad_nets(&wiring::split(segments, &cuts), &contacts))
}

/// Which power flags join pins that would otherwise stay apart, as a flag sitting on a crossing
/// without a dot does. Leaving such flags out would change the nets.
enum Joining {
    /// The flags join nothing; leaving them out keeps the nets.
    None,
    /// These flags (at most 20 checked) each join pins on their own; the file is refused
    /// whatever other flags do.
    Each(Vec<String>),
    /// The flags join pins, but none of the first 20 that could does it alone: `named` are
    /// those flags, `more` how many others could.
    Together { named: Vec<String>, more: usize },
}

/// Whether leaving out `flags` would change the file's nets, and which flags to name. One pass
/// with every flag's pins under keys of their own decides it: adding pins can only merge nets,
/// so the flags change the nets exactly when a group of that pass spans more than one net of
/// the pass without them. Labels take part as pins of their own in both passes, so a flag
/// joining a wire to a wire that only carries a label counts too.
///
/// Flags in such a group are then checked alone, so only culprits are named when there are any.
/// A flag whose pins all sit where wires already meet (a wire end, pin, label or dot) joins
/// nothing new alone and isn't checked; of the rest, at most 20 are.
fn flags_that_join(
    schematic: &Schematic,
    flags: &[PlacedSymbol],
    segments: &[wiring::Segment],
    converted: &BTreeMap<&str, symbols::Converted>,
    pins: &LibraryPins,
) -> Result<Joining, CommandError> {
    let first_extra = schematic.symbols.len();
    let first_label = first_extra + flags.len();
    let labels: Vec<(usize, String, Point)> =
        schematic.labels.iter().enumerate().map(|(j, label)| (first_label + j, String::new(), label.at)).collect();
    let without = file_nets(schematic, segments, converted, pins, &labels)?;
    let net_of: BTreeMap<&(usize, String), usize> = without.iter().enumerate().flat_map(|(i, group)| group.iter().map(move |pin| (pin, i))).collect();
    let mut extra = labels.clone();
    for (i, flag) in flags.iter().enumerate() {
        extra.extend(pins_of(flag, pins)?.into_iter().map(|(number, at)| (first_extra + i, number, at)));
    }
    let mut joining = BTreeSet::new();
    for group in file_nets(schematic, segments, converted, pins, &extra)? {
        let nets: BTreeSet<usize> = group.iter().filter_map(|pin| net_of.get(pin).copied()).collect();
        if nets.len() > 1 {
            joining.extend(group.iter().filter(|(index, _)| (first_extra..first_label).contains(index)).map(|(index, _)| index - first_extra));
        }
    }
    if joining.is_empty() {
        return Ok(Joining::None);
    }
    // Where wires already meet, a flag adds nothing; leave those out of the one-by-one checks.
    let mut met: BTreeSet<Point> = schematic.junctions.iter().map(|m| m.at).chain(schematic.labels.iter().map(|l| l.at)).collect();
    met.extend(segments.iter().flat_map(|(a, b)| [*a, *b]));
    for symbol in &schematic.symbols {
        met.extend(pins_of(symbol, pins)?.into_iter().map(|(_, at)| at));
    }
    let could_join: BTreeSet<usize> = extra
        .iter()
        .filter(|(index, _, at)| (first_extra..first_label).contains(index) && !met.contains(at))
        .map(|(index, _, _)| index - first_extra)
        .collect();
    let possible: Vec<usize> = joining.iter().copied().filter(|i| could_join.contains(i)).collect();
    let possible = if possible.is_empty() { joining.into_iter().collect() } else { possible };
    let more = possible.len().saturating_sub(20);
    let candidates: Vec<&PlacedSymbol> = possible.into_iter().take(20).filter_map(|i| flags.get(i)).collect();
    let mut culprits = Vec::new();
    for flag in &candidates {
        let mut alone = labels.clone();
        alone.extend(pins_of(flag, pins)?.into_iter().map(|(number, at)| (first_extra, number, at)));
        let joins = file_nets(schematic, segments, converted, pins, &alone)?
            .iter()
            .any(|group| group.iter().filter_map(|pin| net_of.get(pin)).collect::<BTreeSet<_>>().len() > 1);
        if joins {
            culprits.push(flag.reference.clone());
        }
    }
    Ok(if culprits.is_empty() {
        Joining::Together { named: candidates.iter().map(|flag| flag.reference.clone()).collect(), more }
    } else {
        Joining::Each(culprits)
    })
}

/// Whether a library symbol is a power flag: marked as a power symbol, with a single pin that
/// is a power output, as KiCad's `PWR_FLAG` is, and a pin number the native format holds (one
/// that doesn't is an ordinary symbol, which conversion refuses).
fn is_power_flag(symbol: &pcbcraft_kicad::schematic::LibSymbol) -> bool {
    let mut pins = symbol.units.iter().filter(|u| u.style <= 1).flat_map(|u| &u.pins);
    let only = pins.next().filter(|_| pins.next().is_none());
    symbol.power && only.is_some_and(|p| p.electrical == pcbcraft_kicad::schematic::Electrical::PowerOut && symbols::valid_number(&p.number))
}

/// Refuse a net name used by more than one kind of named marker: local labels, global labels,
/// power symbols and local power symbols (KiCad 10's `(power local)`). PCBCraft joins every
/// marker of one name on a sheet; whether KiCad does across those kinds isn't confirmed, so
/// such files are refused rather than guessed at.
fn refuse_name_clashes(schematic: &Schematic, converted: &BTreeMap<&str, symbols::Converted>, refusals: &mut Refusals) {
    let mut kinds: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for label in &schematic.labels {
        kinds.entry(label.name.as_str()).or_default().insert(if label.kind == LabelKind::Global { "global label" } else { "local label" });
    }
    for symbol in &schematic.symbols {
        if converted.get(symbol.lib_symbol.as_str()).is_some_and(|c| c.power_terminal.is_some())
            && let Some(value) = symbol.property("Value")
        {
            let local = schematic.lib_symbols.get(&symbol.lib_symbol).is_some_and(|lib| lib.power_local);
            kinds.entry(value).or_default().insert(if local { "local power symbol" } else { "power symbol" });
        }
    }
    for (name, kinds) in kinds.iter().filter(|(_, kinds)| kinds.len() > 1) {
        let kinds: Vec<&str> = kinds.iter().copied().collect();
        refusals.add(format!("the name `{name}` is used by a {}; how KiCad joins those isn't confirmed", kinds.join(" and a ")));
    }
}

/// Refuse a power symbol whose Value, its net name, can't be kept exactly or differs from its
/// power pin's name; note other Values that had to be shortened or that fall back to the
/// symbol's default.
fn check_values(schematic: &Schematic, converted: &BTreeMap<&str, symbols::Converted>, refusals: &mut Refusals, report: &mut Report) {
    for symbol in &schematic.symbols {
        let raw = symbol.property("Value").unwrap_or_default();
        // KiCad 6 and 7 name a power symbol's net after its power pin; KiCad 8 and later after
        // its Value, as this import does. A file that doesn't say it comes from KiCad 8 or later
        // refuses where the two names differ.
        let pin_name = converted
            .get(symbol.lib_symbol.as_str())
            .and_then(|c| c.power_pin)
            .and_then(|(unit, pin)| Some(schematic.lib_symbols.get(&symbol.lib_symbol)?.units.get(unit)?.pins.get(pin)?.name.as_str()));
        if let Some(name) = pin_name.filter(|name| !value_names_power_nets(schematic) && !name.is_empty() && *name != raw) {
            refusals.add(format!(
                "power symbol {}'s Value `{}` differs from its power pin's name `{}`; KiCad 6–7 and KiCad 8 and later name the net differently",
                symbol.reference,
                symbols::shown(raw),
                symbols::shown(name)
            ));
            continue;
        }
        if native_value(symbol).as_deref() == Some(raw) {
            continue;
        }
        if converted.get(symbol.lib_symbol.as_str()).is_some_and(|c| c.power_terminal.is_some()) {
            refusals.add(format!("power symbol {}'s net name `{raw}` isn't a valid one-line Value of at most 64 characters", symbol.reference));
        } else {
            report.note("values that are blank, multi-line or longer than 64 characters shortened or set to the symbol's default");
        }
    }
}

/// Whether the file says it was written by KiCad 8 or later, whose power symbols take their net
/// name from their Value (KiCad 8.0 release notes). Files that don't say are treated as older.
fn value_names_power_nets(schematic: &Schematic) -> bool {
    let major = schematic.generator_version.as_deref().and_then(|v| v.split('.').next()).and_then(|m| m.trim().parse::<u32>().ok());
    major.is_some_and(|major| major >= 8)
}

/// A placed symbol's Value as a native Value: printable, trimmed, at most 64 characters, valid.
fn native_value(symbol: &PlacedSymbol) -> Option<String> {
    symbol.property("Value").and_then(|v| symbols::clip(v, 64)).filter(|v| check_value(v).is_ok())
}

/// Refuse every placed symbol, power flags included, that shows a body style other than the
/// normal one. The import reads and converts the normal style's pins only. KiCad 10.0.8 gives a
/// part shown in a style its symbol lacks none of the normal style's pins (#503), and how it
/// counts the styles of a symbol that has more isn't confirmed, so connecting the normal pins
/// could invent connections.
fn refuse_body_styles(schematic: &Schematic, refusals: &mut Refusals) {
    for symbol in schematic.symbols.iter().filter(|symbol| symbol.body_style != 1) {
        refusals.add(format!(
            "{} shows body style {} of `{}`; only parts shown in the normal body style (1) are imported",
            symbol.reference,
            symbol.body_style,
            symbol.lib_symbol.chars().take(80).collect::<String>()
        ));
    }
}

/// How many jumper pin groups a refusal names.
const NAMED_GROUPS: usize = 4;
/// How many pins of each jumper pin group a refusal names.
const NAMED_PINS: usize = 8;

/// The groups of `list` that join two or more different pins, each as its pin numbers joined
/// by `+`: at most one more than [`NAMED_GROUPS`], so the caller can tell there are more, and
/// at most [`NAMED_PINS`] pins of 16 characters each, with `+…` for the rest. A group of one
/// pin, or of one pin repeated, joins nothing.
fn joining_groups(list: &[Vec<String>]) -> Vec<String> {
    list.iter()
        .filter(|group| group.first().is_some_and(|first| group.iter().any(|pin| pin != first)))
        .take(NAMED_GROUPS + 1)
        .map(|group| {
            let mut pins: Vec<String> = Vec::new();
            for pin in group {
                if pins.len() == NAMED_PINS {
                    pins.push("…".into());
                    break;
                }
                let pin: String = pin.chars().take(16).collect();
                if !pins.contains(&pin) {
                    pins.push(pin);
                }
            }
            pins.join("+")
        })
        .collect()
}

/// Refuse what changes connectivity and has no native form in this slice: sub-sheets, buses,
/// hierarchical labels, labels the native model can't name, and parts that join their own pins
/// (KiCad 10 jumpers), named by reference and library symbol. Each list of jumper groups is
/// read once, however many symbols share it through `extends` and however often they are
/// placed; a placement then costs a few short strings.
fn refuse_unsupported(schematic: &Schematic, refusals: &mut Refusals) {
    if !schematic.sheets.is_empty() {
        refusals.add(format!("it has {} sub-sheet(s); only self-contained single-sheet schematics are supported", schematic.sheets.len()));
    }
    if !schematic.buses.is_empty() || !schematic.bus_entries.is_empty() {
        refusals.add("it uses buses, which PCBCraft doesn't have yet (#43)".into());
    }
    if schematic.labels.iter().any(|l| l.kind == LabelKind::Hierarchical) {
        refusals.add("it has hierarchical labels, which belong to sub-sheets".into());
    }
    for label in &schematic.labels {
        if label.name.chars().count() > 200 || label.name.chars().any(char::is_control) {
            refusals.add(format!("label `{}` is longer than 200 characters or spans lines", label.name.chars().take(40).collect::<String>()));
        }
    }
    let mut described: BTreeMap<*const Vec<Vec<String>>, Vec<String>> = BTreeMap::new();
    for symbol in &schematic.symbols {
        let Some(lib) = schematic.lib_symbols.get(&symbol.lib_symbol) else { continue };
        let name = symbol.lib_symbol.chars().take(80).collect::<String>();
        let mut joining: Vec<String> = Vec::new();
        for list in &lib.jumper_pin_groups {
            joining.extend(described.entry(Arc::as_ptr(list)).or_insert_with(|| joining_groups(list)).iter().cloned());
        }
        if !joining.is_empty() {
            let more = if joining.len() > NAMED_GROUPS { ", …" } else { "" };
            let groups = format!("{}{more}", joining[..joining.len().min(NAMED_GROUPS)].join(", "));
            refusals.add(format!(
                "{} (`{name}`) joins pins {groups} inside the part (a KiCad 10 jumper pin group), which PCBCraft doesn't have yet",
                symbol.reference
            ));
        }
        if lib.duplicate_pin_numbers_are_jumpers {
            refusals.add(format!(
                "{} (`{name}`) joins pins that share a number inside the part (KiCad 10 \"duplicate pin numbers are jumpers\"), which PCBCraft doesn't have yet",
                symbol.reference
            ));
        }
    }
}

/// One placed KiCad symbol in the native project.
struct Placed {
    /// The placed symbol's index in the file's [`Schematic::symbols`].
    source: usize,
    /// The native symbol placement's ordinal.
    symbol: u64,
    /// The native part instance's ordinal.
    component: u64,
    /// Each pin's number and where KiCad connects it on the sheet.
    pins: Vec<(String, Point)>,
}

/// The scratch session the project is built in, with the converted symbols' digests.
struct Builder {
    session: Session,
    sheet: Option<SheetTarget>,
    /// The library identity the converted symbols are pinned under.
    library: String,
    /// The project's highest allocated ordinal before the latest command, to find what it added.
    high: u64,
    /// `(revision, digest)` of each converted symbol, by native ID.
    digests: BTreeMap<String, Value>,
}

impl Builder {
    /// A scratch session with a new project and the converted symbols loaded as one source.
    /// Fails, naming the symbol and the loader's reason, when a converted document is refused.
    fn new(name: &str, converted: &BTreeMap<&str, symbols::Converted>, scope: Option<(Arc<Project>, SheetTarget)>) -> Result<Self, CommandError> {
        let mut session = Session::default();
        let sheet = if let Some((project, sheet)) = scope {
            session.open_snapshot(project);
            Some(sheet)
        } else {
            let name = symbols::clip(name, 64).unwrap_or_else(|| "Imported".to_owned());
            session.apply("project.new", &json!({"name": name})).map_err(step("creating the project"))?;
            None
        };
        let mut documents: BTreeMap<String, String> =
            converted.values().map(|symbol| (format!("symbols/{}.json", symbol.id), symbol.text.clone())).collect();
        let mut digest = <sha2::Sha256 as sha2::Digest>::new();
        for (path, text) in &documents {
            for part in [path.as_bytes(), &[0], text.as_bytes(), &[0]] {
                sha2::Digest::update(&mut digest, part);
            }
        }
        let library = format!("{LIBRARY_PREFIX}{}", &format!("{:x}", sha2::Digest::finalize(digest))[..16]);
        documents.insert(
            "library.json".to_owned(),
            json!({"format": "pcbcraft-library/1", "id": library, "name": "Imported from KiCad", "license": "NOASSERTION"}).to_string(),
        );
        session.execute(LOAD, &json!({"source": SOURCE, "documents": documents})).map_err(step("loading the converted symbols"))?;
        let digests: BTreeMap<String, Value> = session
            .libraries()
            .search("", Some(Kind::Symbol))
            .into_iter()
            .filter(|(source, _)| *source == SOURCE)
            .filter_map(|(_, entry)| {
                let inspected = entry.inspect();
                Some((inspected["key"]["id"].as_str()?.to_owned(), inspected["digest"].clone()))
            })
            .collect();
        let mut refusals = Refusals::default();
        let diagnostics = session.execute(crate::library::sources::LIST, &json!({})).unwrap_or(Value::Null);
        for (name, symbol) in converted {
            if !digests.contains_key(&symbol.id) {
                let reason =
                    diagnostic(&diagnostics, &format!("symbols/{}.json", symbol.id)).unwrap_or_else(|| "it was refused by the symbol loader".into());
                refusals.add(format!("symbol `{name}`: {reason}"));
            }
        }
        refusals.check()?;
        Ok(Self { session, sheet, library, high: 0, digests })
    }

    /// Run one project command at the current revision. Wire edits keep their authored routes.
    fn apply(&mut self, id: &str, mut params: Value, what: &str) -> Result<(), CommandError> {
        params["expected_revision"] = json!(self.session.revision());
        if let Some(sheet) = self.sheet
            && crate::sheets::scoped_command(id)
        {
            params["sheet"] = json!(sheet.id());
        }
        self.high = self.session.project().map_or(0, Project::highest_id);
        self.session.apply(id, &params).map(|_| ()).map_err(step(what))
    }

    /// Give the sheet the file's paper with its top-left corner at the origin, where KiCad
    /// measures every position from, so the drawing sits on the paper as in the file. Without a
    /// paper the sheet can take there (a size other than A4 to A1, which would reach past the
    /// schematic's range), fit a paper around the drawing and report the replacement if the file
    /// named one. Clamp the fallback corner to the range `sheet.set_page` accepts, then check if
    /// the paper holds the drawing. Keep the new sheet's paper if the fallback cannot be applied.
    fn page(&mut self, paper: Option<&pcbcraft_kicad::schematic::Paper>, report: &mut Report) -> Result<(), CommandError> {
        let project = self.project()?;
        let origin = crate::schematic::Point { x_nm: 0, y_nm: 0 };
        let orientation = |portrait: bool| if portrait { Orientation::Portrait } else { Orientation::Landscape };
        let page = match paper.and_then(|p| Page::new(Paper::parse(&p.name.to_ascii_lowercase())?, orientation(p.portrait), origin).ok()) {
            Some(page) => Some(page),
            None => {
                let bounds = project.drawing_bounds();
                let fitted = Page::around(bounds);
                let at = fitted.at();
                let corner = crate::schematic::Point {
                    x_nm: at.x_nm.clamp(-MAX_CENTER_NM, MAX_CENTER_NM),
                    y_nm: at.y_nm.clamp(-MAX_CENTER_NM, MAX_CENTER_NM),
                };
                let around = Page::new(fitted.paper(), fitted.orientation(), corner).ok();
                let holds = match (around.or(project.page()), bounds) {
                    (_, None) => true,
                    (Some(page), Some([min, max])) => {
                        let [low, high] = page.corners();
                        low.x_nm <= min.x_nm && low.y_nm <= min.y_nm && max.x_nm <= high.x_nm && max.y_nm <= high.y_nm
                    }
                    (None, Some(_)) => false,
                };
                if !holds {
                    report.note(PAPER_SHORT);
                } else if paper.is_some() {
                    report.note(PAPER_REPLACED);
                }
                around
            }
        };
        let Some(page) = page else { return Ok(()) };
        let params = json!({"sheet": project.sheet_target().id(), "paper": page.paper().as_str(), "orientation": page.orientation().as_str(), "at": page.at()});
        self.apply(SET_PAGE, params, "setting the paper")
    }

    /// The open project; always present once [`Builder::new`] succeeded.
    fn project(&self) -> Result<Arc<Project>, CommandError> {
        let project = self.session.snapshot().ok_or_else(|| CommandError::InvalidParams("the imported project is missing".into()))?;
        match self.sheet {
            Some(sheet) => project.for_sheet(sheet).map(Arc::new).ok_or_else(|| CommandError::InvalidParams("the imported sheet is missing".into())),
            None => Ok(project),
        }
    }

    /// Place every symbol, each component's further units, and set references, values and fields.
    fn place(
        &mut self,
        schematic: &Schematic,
        converted: &BTreeMap<&str, symbols::Converted>,
        pins: &LibraryPins,
        poses: &[Option<(u8, bool)>],
        report: &mut Report,
    ) -> Result<Vec<Placed>, CommandError> {
        let mut placed = Vec::new();
        // Annotated units of one component share a reference and a library symbol.
        let mut components: BTreeMap<(String, String), (u64, BTreeSet<u32>)> = BTreeMap::new();
        let mut references: BTreeSet<String> = BTreeSet::new();
        for source in placement_order(schematic) {
            let (Some(symbol), Some(pose)) = (schematic.symbols.get(source), poses.get(source)) else { continue };
            let (Some(c), Some((turns, mirrored))) = (converted.get(symbol.lib_symbol.as_str()), *pose) else { continue };
            let unit = c.units.get(&symbol.unit).cloned().unwrap_or_else(|| "main".into());
            // Place unturned when mirrored, then mirror left to right: that composes to `turns`.
            let rotation = u16::from(if mirrored { (4 - turns) % 4 } else { turns }) * 90;
            let annotated = !symbol.reference.ends_with('?');
            let key = (symbol.reference.clone(), symbol.lib_symbol.clone());
            // A unit already placed under this reference is a copy KiCad allows: a new part.
            let existing = annotated.then(|| components.get(&key).cloned()).flatten().filter(|(_, units)| !units.contains(&symbol.unit));
            match &existing {
                Some((component, _)) => {
                    let params = json!({"component": format!("component-{component}"), "unit": unit, "rotation": rotation,
                        "x_nm": symbol.at.x_nm, "y_nm": symbol.at.y_nm, "auto_heal": false});
                    self.apply(PLACE_UNIT, params, &format!("placing {} unit {}", symbol.reference, symbol.unit))?;
                }
                None => {
                    let digest = self.digests.get(&c.id).cloned().unwrap_or(Value::Null);
                    let mut params = json!({"library": self.library, "definition": c.id, "definition_revision": 1, "definition_digest": digest,
                        "unit": unit, "rotation": rotation, "x_nm": symbol.at.x_nm, "y_nm": symbol.at.y_nm, "auto_heal": false});
                    if let Some(value) = &native_value(symbol) {
                        params["value"] = json!(value);
                    }
                    self.apply(PLACE_SYMBOL, params, &format!("placing {}", symbol.reference))?;
                }
            }
            let project = self.project()?;
            let native = newest(project.symbols(), |s| s.ordinal, self.high)
                .ok_or_else(|| step("placing a symbol")(CommandError::NotFound("symbol".into())))?;
            let (ordinal, component) = (native.ordinal, native.component);
            if mirrored {
                self.apply(
                    crate::MIRROR_SYMBOL,
                    json!({"symbol": crate::schematic::symbol_id(ordinal), "axis": "horizontal"}),
                    "mirroring a symbol",
                )?;
            }
            if existing.is_none() {
                self.set_fields(symbol, ordinal, annotated, &mut references, report)?;
                if annotated {
                    components.entry(key).or_insert((component, BTreeSet::from([symbol.unit])));
                }
            } else if let Some((_, units)) = components.get_mut(&key) {
                units.insert(symbol.unit);
                let project = self.project()?;
                let id = crate::schematic::symbol_id(ordinal);
                let native = project.symbol(&id).ok_or_else(|| CommandError::NotFound("the imported unit is missing".into()))?;
                let layouts = field_layouts(symbol, native, report);
                if !layouts.is_empty() {
                    self.apply(
                        crate::SET_SYMBOL_FIELDS,
                        json!({"symbol":id,"reference":project.reference(native),"value":project.value(native),"field_layout":layouts}),
                        &format!("setting the field appearance of {} unit {}", symbol.reference, symbol.unit),
                    )?;
                }
            }
            let pins = pins_of(symbol, pins)?;
            placed.push(Placed { source, symbol: ordinal, component, pins });
        }
        Ok(placed)
    }

    /// Set a new part's reference, value and fields from KiCad's. An unannotated or clashing
    /// reference keeps the one placement allocated, with a note.
    fn set_fields(
        &mut self,
        symbol: &PlacedSymbol,
        ordinal: u64,
        annotated: bool,
        references: &mut BTreeSet<String>,
        report: &mut Report,
    ) -> Result<(), CommandError> {
        if symbol.reference_from == pcbcraft_kicad::schematic::ReferenceSource::OtherRecord {
            report.note("references taken from another project's instance record");
        }
        let project = self.project()?;
        let native =
            project.symbols().iter().find(|s| s.ordinal == ordinal).ok_or_else(|| step("setting fields")(CommandError::NotFound("symbol".into())))?;
        let part = project.component(native).ok_or_else(|| step("setting fields")(CommandError::NotFound("part".into())))?;
        let mut reference = part.reference().to_owned();
        if !annotated {
            report.note("unannotated references numbered automatically");
        } else if check_reference(&symbol.reference).is_err() {
            report.note("references longer than 32 characters or containing spaces numbered automatically");
        } else if !references.insert(symbol.reference.clone()) {
            report.note("references used by two different parts numbered automatically");
        } else {
            reference = symbol.reference.clone();
        }
        let value = part.value().to_owned();
        let metadata = metadata(symbol, report)?;
        let field_layout = field_layouts(symbol, native, report);
        let params = json!({"symbol": crate::schematic::symbol_id(ordinal), "reference": reference, "value": value,
            "metadata": metadata, "field_layout": field_layout});
        self.apply(crate::SET_SYMBOL_FIELDS, params, &format!("setting the fields of {}", symbol.reference))
    }

    /// Draw the wires, place the labels and mark the no-connects.
    fn wire(
        &mut self,
        schematic: &Schematic,
        segments: &[wiring::Segment],
        placed: &[Placed],
        converted: &BTreeMap<&str, symbols::Converted>,
        pins: &LibraryPins,
        report: &mut Report,
    ) -> Result<(), CommandError> {
        // Where each pin connects, and the pins that share a point.
        let mut pins_at: BTreeMap<Point, Vec<(u64, u64, String)>> = BTreeMap::new();
        for p in placed {
            for (number, at) in &p.pins {
                pins_at.entry(*at).or_default().push((p.symbol, p.component, number.clone()));
            }
        }
        let labels: BTreeSet<Point> = schematic.labels.iter().map(|l| l.at).collect();
        let no_connects: BTreeSet<Point> = schematic.no_connects.iter().map(|m| m.at).collect();
        let dots: BTreeSet<Point> = schematic.junctions.iter().map(|m| m.at).collect();
        let project = self.project()?;
        let mut interfaces = BTreeMap::new();
        for (endpoint, point) in project.hierarchy_endpoints() {
            let point = Point { x_nm: point.x_nm, y_nm: point.y_nm };
            if interfaces.insert(point, endpoint).is_some() {
                return Err(CommandError::InvalidParams(format!(
                    "overlapping hierarchy anchors at ({:.3}, {:.3}) mm",
                    point.x_nm as f64 / 1e6,
                    point.y_nm as f64 / 1e6
                )));
            }
        }
        let mut cuts: BTreeSet<Point> = pins_at.keys().copied().collect();
        cuts.extend(interfaces.keys().copied());
        cuts.extend(labels.iter());
        cuts.extend(dots.iter());
        let pieces = wiring::split(segments, &cuts);
        let mut stops = cuts.clone();
        stops.extend(no_connects.iter());

        // Pins that touch another pin connect in KiCad; native pins only connect through wires.
        let mut refusals = Refusals::default();
        for (at, here) in &pins_at {
            let power = |(symbol, _, _): &(u64, u64, String)| {
                placed
                    .iter()
                    .find(|p| p.symbol == *symbol)
                    .and_then(|p| schematic.symbols.get(p.source))
                    .is_some_and(|s| converted.get(s.lib_symbol.as_str()).is_some_and(|c| c.power_terminal.is_some()))
            };
            let mut ordinary = here.iter().filter(|pin| !power(pin));
            if let Some((first, _, _)) = ordinary.next()
                && ordinary.any(|(symbol, _, _)| symbol != first)
            {
                refusals.add(format!("pins touch without a wire at ({:.2} mm, {:.2} mm)", at.x_nm as f64 / 1e6, at.y_nm as f64 / 1e6));
            }
        }
        refusals.check()?;

        // A wire end touching nothing is legal, but it is also what a pin put in the wrong place
        // would leave behind, so the report counts them.
        let mut degree: BTreeMap<Point, usize> = BTreeMap::new();
        for (a, b) in &pieces {
            *degree.entry(*a).or_default() += 1;
            *degree.entry(*b).or_default() += 1;
        }
        let loose: BTreeSet<Point> = degree
            .iter()
            .filter(|(p, n)| **n == 1 && !pins_at.contains_key(p) && !interfaces.contains_key(p) && !labels.contains(p) && !no_connects.contains(p))
            .map(|(p, _)| *p)
            .collect();
        report.add("wire ends that touch nothing", loose.len());
        // The order of turning and mirroring only matters for a mirrored symbol turned a quarter.
        // Were KiCad to mirror first, its pins would sit where the other mirror axis puts them
        // here; a loose wire end there means the pose is ambiguous, so it refuses.
        let mut refusals = Refusals::default();
        for p in placed {
            let Some(symbol) = schematic.symbols.get(p.source) else { continue };
            if symbol.mirror == Mirror::None || symbol.rotation_mdeg % 180_000 == 0 {
                continue;
            }
            let mut other = symbol.clone();
            other.mirror = if symbol.mirror == Mirror::X { Mirror::Y } else { Mirror::X };
            if pins_of(&other, pins)?.iter().any(|(_, at)| loose.contains(at)) {
                refusals.add(format!(
                    "{} is mirrored and turned a quarter, and a wire ends where its pins would be if KiCad mirrored before turning",
                    symbol.reference
                ));
            }
        }
        refusals.check()?;
        let routes = wiring::plan(&pieces, &stops);
        if routes.len() > crate::schematic::wiring::MAX_WIRES {
            return Err(CommandError::CapExceeded(format!(
                "This KiCad schematic needs {} native wires; a sheet holds at most {}.",
                routes.len(),
                crate::schematic::wiring::MAX_WIRES
            )));
        }
        let mut junctions: BTreeMap<Point, u64> = BTreeMap::new();
        for route in routes {
            let ends = [route.from, route.to];
            let anchors: Vec<Value> = ends
                .iter()
                .map(|end| match (interfaces.get(end), pins_at.get(end).and_then(|pins| pins.first()), junctions.get(end)) {
                    (Some(interface), _, _) => interface.inspect(),
                    (None, Some((symbol, _, number)), _) => json!({"symbol": crate::schematic::symbol_id(*symbol), "terminal": number}),
                    (None, None, Some(junction)) => json!({"junction": format!("junction-{junction}")}),
                    (None, None, None) => json!({"x_nm": end.x_nm, "y_nm": end.y_nm}),
                })
                .collect();
            let bends: Vec<Value> = route.bends.iter().map(|b| at(*b)).collect();
            self.apply(crate::DRAW_WIRE, json!({"from": anchors[0], "to": anchors[1], "bends": bends, "auto_heal": false}), "drawing a wire")?;
            let project = self.project()?;
            let wire =
                newest(project.wires(), Wire::ordinal, self.high).ok_or_else(|| step("drawing a wire")(CommandError::NotFound("wire".into())))?;
            for (end, endpoint) in ends.iter().zip(wire.endpoints) {
                if let WireEndpoint::Junction(id) = endpoint {
                    junctions.entry(*end).or_insert(id);
                }
            }
        }

        for label in &schematic.labels {
            let rotation = match label.rotation_mdeg {
                r if r % 90_000 == 0 => r / 1000,
                _ => {
                    report.note("labels at other than a quarter turn placed level");
                    0
                }
            };
            if label.shape.is_some() {
                report.note("global label shapes left out");
            }
            let style = if label.kind == LabelKind::Global { "global" } else { "local" };
            let text = label_layout(label, rotation as u16);
            let params =
                json!({"name": label.name, "style": style, "x_nm": label.at.x_nm, "y_nm": label.at.y_nm, "rotation": rotation, "text": text});
            self.apply(crate::net_labels::ADD, params, &format!("placing label {}", label.name))?;
        }

        if self.sheet.is_some() {
            for part in placed {
                let source = &schematic.symbols[part.source];
                if schematic.lib_symbols.get(&source.lib_symbol).is_some_and(|lib| lib.power_local)
                    && let Some(terminal) = converted.get(source.lib_symbol.as_str()).and_then(|lib| lib.power_terminal.as_deref())
                    && let Some((_, point)) = part.pins.iter().find(|(number, _)| number == terminal)
                    && let Some(name) = source.property("Value")
                {
                    self.apply(
                        crate::net_labels::ADD,
                        json!({"name":name,"style":"local","x_nm":point.x_nm,"y_nm":point.y_nm,"rotation":0}),
                        &format!("placing local power anchor for {}", source.reference),
                    )?;
                }
            }
        }

        for mark in &no_connects {
            if let Some(pins) = pins_at.get(mark) {
                for (_, component, number) in pins {
                    let target = json!({"part": format!("component-{component}"), "terminal": number});
                    self.apply(crate::no_connect::SET, json!({"target": target, "marked": true}), "marking a no-connect")?;
                }
            } else if let Some(junction) = junctions.get(mark) {
                let target = json!({"junction": format!("junction-{junction}")});
                self.apply(crate::no_connect::SET, json!({"target": target, "marked": true}), "marking a wire-end no-connect")?;
            } else {
                report.note("no-connect marks not on a pin or wire end left out");
            }
        }
        Ok(())
    }
}

/// The item a command just added, given the highest ordinal before it ran: allocation only
/// grows, so it is the one item above `before`. The engine appends, so the last item is checked
/// first; a full scan is the fallback. `None` when nothing new is there.
fn newest<T>(items: &[T], ordinal: impl Fn(&T) -> u64, before: u64) -> Option<&T> {
    match items.last() {
        Some(last) if ordinal(last) > before => Some(last),
        _ => items.iter().filter(|item| ordinal(item) > before).max_by_key(|item| ordinal(item)),
    }
}

/// The loader's message for one file, from `library.list_sources`.
fn diagnostic(list: &Value, file: &str) -> Option<String> {
    let mut found = None;
    let mut visit = |value: &Value| {
        if value.get("file").and_then(Value::as_str) == Some(file) {
            found = value.get("message").or_else(|| value.get("error")).and_then(Value::as_str).map(str::to_owned);
        }
    };
    let mut stack = vec![list];
    while let Some(value) = stack.pop() {
        visit(value);
        match value {
            Value::Array(items) => stack.extend(items),
            Value::Object(map) => stack.extend(map.values()),
            _ => {}
        }
    }
    found
}

/// One library symbol's normal-style pins, in its own frame, borrowed from the file: the
/// common unit's, and each numbered unit's own.
struct UnitPins<'a> {
    /// Whether the symbol has at most one numbered unit, which every placement then shows.
    single: bool,
    common: Vec<(&'a str, Point)>,
    units: BTreeMap<u32, Vec<(&'a str, Point)>>,
}

/// The pins of the library symbols an import places, read once each and borrowed from the
/// file: symbols extending one parent share its drawing and share one table, so the work
/// follows the file's own units and pins however many symbols or placements use them.
struct LibraryPins<'a> {
    tables: Vec<UnitPins<'a>>,
    by_name: BTreeMap<&'a str, usize>,
}

impl<'a> LibraryPins<'a> {
    /// Read the pins of each library symbol in `names` that the file holds; others have none.
    /// Callers pass symbols whose size conversion has bounded, and power flags.
    fn new(schematic: &'a Schematic, names: impl IntoIterator<Item = &'a str>) -> Self {
        let mut tables = Vec::new();
        let mut by_drawing: BTreeMap<*const Vec<pcbcraft_kicad::schematic::SymbolUnit>, usize> = BTreeMap::new();
        let mut by_name = BTreeMap::new();
        for name in names {
            let Some(library) = schematic.lib_symbols.get(name) else { continue };
            let index = *by_drawing.entry(Arc::as_ptr(&library.units)).or_insert_with(|| {
                let mut table = UnitPins { single: true, common: Vec::new(), units: BTreeMap::new() };
                for unit in library.units.iter().filter(|unit| unit.style <= 1) {
                    let pins = unit.pins.iter().map(|pin| (pin.number.as_str(), pin.at));
                    if unit.unit == 0 { table.common.extend(pins) } else { table.units.entry(unit.unit).or_default().extend(pins) }
                }
                table.single = table.units.len() <= 1;
                tables.push(table);
                tables.len() - 1
            });
            by_name.insert(name, index);
        }
        LibraryPins { tables, by_name }
    }

    /// The pins `symbol` shows, in its library's frame: the common ones, then the placed unit's,
    /// or for a single-unit symbol that unit's whatever is placed. None for a symbol not read.
    fn of(&self, symbol: &PlacedSymbol) -> impl Iterator<Item = &(&'a str, Point)> {
        let table = self.by_name.get(symbol.lib_symbol.as_str()).and_then(|&i| self.tables.get(i));
        table.into_iter().flat_map(move |table| {
            let own = if table.single { table.units.values().next() } else { table.units.get(&symbol.unit) };
            table.common.iter().chain(own.into_iter().flatten())
        })
    }
}

/// A placed symbol's pins and where KiCad connects them, from its library symbol's normal body
/// style: the common unit's pins and the placed unit's. Fails when a pin lies out of range.
fn pins_of(symbol: &PlacedSymbol, pins: &LibraryPins) -> Result<Vec<(String, Point)>, CommandError> {
    pins.of(symbol)
        .map(|(number, at)| {
            let at = kicad_pin_position(symbol, *at).ok_or_else(|| CommandError::InvalidParams(format!("{} lies out of range", symbol.reference)))?;
            Ok(((*number).to_owned(), at))
        })
        .collect()
}

/// A part's native metadata from KiCad's fields: the datasheet and description by their native
/// identities, the footprint link as an ordinary `KiCad footprint` field (assigning a footprint
/// is separate work), the other fields by new identities, and the fitting attributes. A kept
/// value holds at most 2,048 characters and no control characters; one changed to fit is noted
/// in `report`.
fn metadata(symbol: &PlacedSymbol, report: &mut Report) -> Result<Value, CommandError> {
    let mut fields = serde_json::Map::new();
    let mut names: BTreeSet<String> = BTreeSet::new();
    for property in &symbol.properties {
        let value = property.value.trim();
        if property.key.starts_with("ki_") || matches!(property.key.as_str(), "Reference" | "Value") || value.is_empty() || value == "~" {
            continue;
        }
        let (id, name) = match property.key.as_str() {
            "Datasheet" => ("datasheet".to_owned(), "Datasheet".to_owned()),
            "Description" => ("description".to_owned(), "Description".to_owned()),
            "Footprint" => (crate::schematic::fields::new_id().map_err(CommandError::InvalidParams)?, "KiCad footprint".to_owned()),
            key => match symbols::clip(key, 64) {
                Some(name) if !matches!(name.to_lowercase().as_str(), "reference" | "value" | "footprint") => {
                    (crate::schematic::fields::new_id().map_err(CommandError::InvalidParams)?, name)
                }
                _ => {
                    report.note("fields with reserved or blank names left out");
                    continue;
                }
            },
        };
        let kept: String = value.chars().filter(|c| !c.is_control()).take(2048).collect();
        if fields.len() >= 64 || !names.insert(name.to_lowercase()) {
            report.note("fields beyond 64 per part, or repeated, left out");
            continue;
        }
        if kept != value {
            report.note("field values shortened to 2,048 characters or stripped of control characters");
        }
        let value = kept;
        fields.insert(id, json!({"name": name, "value": value}));
    }
    Ok(json!({"fields": fields, "attributes": {
        "do_not_populate": symbol.dnp,
        "exclude_from_bom": !symbol.in_bom,
        "exclude_from_board": !symbol.on_board,
        "exclude_from_simulation": symbol.exclude_from_sim,
        "exclude_from_position_files": !symbol.in_pos_files,
    }}))
}

/// Placed field anchors are in KiCad's sheet frame. The native editor stores their offsets
/// relative to the symbol's unturned drawing so they follow later placement edits.
fn field_layouts(symbol: &PlacedSymbol, native: &SymbolPlacement, report: &mut Report) -> Layouts {
    [("Reference", "reference"), ("Value", "value")]
        .into_iter()
        .filter_map(|(key, id)| {
            let property = symbol.properties.iter().find(|property| property.key == key)?;
            let offset = property.at.and_then(|at| {
                let (x_nm, y_nm) =
                    crate::schematic::unplace(at.x_nm - symbol.at.x_nm, at.y_nm - symbol.at.y_nm, native.turns(), native.is_mirrored());
                if [x_nm, y_nm].iter().any(|value| value.unsigned_abs() > MAX_CENTER_NM as u64) {
                    report.note("field positions outside the native range reset to defaults");
                    return None;
                }
                Some(crate::Point { x_nm, y_nm })
            });
            let rotation = if property.rotation_mdeg % 90_000 == 0 {
                (property.rotation_mdeg / 1_000) as u16
            } else {
                report.note("fields at other than a quarter turn placed level");
                0
            };
            let align = match property.justify {
                pcbcraft_kicad::schematic::Justify::Left => TextAlign::Start,
                pcbcraft_kicad::schematic::Justify::Center => TextAlign::Middle,
                pcbcraft_kicad::schematic::Justify::Right => TextAlign::End,
            };
            Some((
                id.to_owned(),
                Layout {
                    offset,
                    visible: !property.hidden,
                    rotation,
                    align,
                    height_nm: property.size_nm.clamp(100_000, 10_000_000),
                    ..Layout::default()
                },
            ))
        })
        .collect()
}

/// Keep an imported net label's authored text size on its existing electrical anchor.
fn label_layout(label: &pcbcraft_kicad::schematic::Label, rotation: u16) -> Layout {
    let align = if label.kind == LabelKind::Global {
        TextAlign::Middle
    } else if rotation == 180 {
        TextAlign::End
    } else {
        TextAlign::Start
    };
    Layout { rotation: rotation % 180, align, height_nm: label.size_nm.clamp(100_000, 10_000_000), ..Layout::default() }
}

/// Check that the native project connects exactly the pins KiCad does. Both sides are grouped by
/// part and pin number, so a pin every unit shares is one pin wherever it is wired; any
/// difference refuses the import.
fn check_nets(
    schematic: &Schematic,
    segments: &[wiring::Segment],
    placed: &[Placed],
    converted: &BTreeMap<&str, symbols::Converted>,
    project: &Project,
) -> Result<(), CommandError> {
    let mut contacts = wiring::Contacts { pins: Vec::new(), labels: Vec::new(), power: Vec::new() };
    let mut cuts: BTreeSet<Point> = BTreeSet::new();
    for p in placed {
        let (part, Some(symbol)) = (p.component, schematic.symbols.get(p.source)) else { continue };
        let power = converted.get(symbol.lib_symbol.as_str()).and_then(|c| c.power_terminal.clone());
        for (number, at) in &p.pins {
            contacts.pins.push(((part, number.clone()), *at));
            cuts.insert(*at);
            if power.as_deref() == Some(number.as_str())
                && let Some(value) = symbol.property("Value")
            {
                contacts.power.push(((part, number.clone()), value.to_owned()));
            }
        }
    }
    for label in &schematic.labels {
        let scope = if label.kind == LabelKind::Global { "global" } else { "local" };
        contacts.labels.push((scope, label.name.clone(), label.at));
        cuts.insert(label.at);
    }
    cuts.extend(schematic.junctions.iter().map(|m| m.at));
    let expected = wiring::kicad_nets(&wiring::split(segments, &cuts), &contacts);

    let connectivity = project.connectivity();
    let mut groups: BTreeMap<usize, BTreeSet<(u64, String)>> = BTreeMap::new();
    let mut unconnected: BTreeSet<(u64, String)> = BTreeSet::new();
    for p in placed {
        for (number, _) in &p.pins {
            let terminal = Terminal::parse(&json!({"symbol": crate::schematic::symbol_id(p.symbol), "terminal": number}));
            match terminal.and_then(|terminal| connectivity.net_index_for(terminal)) {
                Some(net) => {
                    groups.entry(net).or_default().insert((p.component, number.clone()));
                }
                None => {
                    unconnected.insert((p.component, number.clone()));
                }
            }
        }
    }
    // A shared pin left unwired on one unit is still connected through another unit's copy.
    let connected: BTreeSet<&(u64, String)> = groups.values().flatten().collect();
    let singles: Vec<BTreeSet<(u64, String)>> =
        unconnected.iter().filter(|pin| !connected.contains(pin)).map(|pin| BTreeSet::from([pin.clone()])).collect();
    let found: BTreeSet<BTreeSet<(u64, String)>> = groups.into_values().chain(singles).collect();
    if found != expected {
        let mismatched = expected.symmetric_difference(&found).flatten().next().map(|(part, n)| {
            let reference =
                placed.iter().find(|p| p.component == *part).and_then(|p| schematic.symbols.get(p.source)).map_or("?", |s| s.reference.as_str());
            format!("{reference} pin {n}")
        });
        return Err(CommandError::InvalidParams(format!(
            "This KiCad schematic can't be imported exactly: the imported connections differ from KiCad's{}",
            mismatched.map(|m| format!(", starting at {m}")).unwrap_or_default()
        )));
    }
    Ok(())
}
