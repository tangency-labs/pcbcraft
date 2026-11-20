//! Shared baseline codec for standalone files and embedded board history.

use super::{
    Baseline, DISPLAY_FORMAT, FORMAT, MAX_BASELINE_BYTES, MULTIPAD_FORMAT, NET_ORIGIN_FORMAT, NetState, Package, PartInstanceState, QUANTITY_FORMAT,
    TerminalKey, strictly_sorted,
};
use crate::{MAX_OBJECT_ID, MAX_SYMBOLS, library::parts::Value, schematic::PinNumber};
use std::{
    cmp::Ordering,
    collections::BTreeSet,
    io::{self, Write},
    ops::Bound::{Excluded, Unbounded},
};

#[derive(Default)]
pub(super) struct BaselineValidation {
    phase: u8,
    component: u64,
    net: usize,
    item: usize,
    needs_origin: bool,
    names: BTreeSet<String>,
    seen: BTreeSet<TerminalKey>,
}

impl BaselineValidation {
    /// Advance by one component, net header, ordered comparison, name, or terminal.
    pub(super) fn step(&mut self, baseline: &Baseline) -> Result<bool, String> {
        match self.phase {
            0 => {
                baseline.source.validate()?;
                if !matches!(baseline.format.as_str(), FORMAT | MULTIPAD_FORMAT | QUANTITY_FORMAT | DISPLAY_FORMAT | NET_ORIGIN_FORMAT)
                    || baseline.project.is_nil()
                    || baseline.board.is_nil()
                    || baseline.components.len() > MAX_SYMBOLS
                {
                    return Err("unsupported or invalid board baseline header".into());
                }
                self.phase = if baseline.format == FORMAT { 1 } else { 2 };
            }
            1 | 2 => {
                // Begin at BTreeMap's first key rather than assuming that input never contains 0.
                let next = if self.component == 0 && self.item == 0 {
                    baseline.components.first_key_value()
                } else {
                    baseline.components.range((Excluded(self.component), Unbounded)).next()
                };
                if let Some((&id, component)) = next {
                    if self.phase == 1 {
                        if component.package.as_ref().is_some_and(Package::extended) {
                            return Err("unsupported or invalid board baseline header".into());
                        }
                    } else {
                        validate_component(baseline, id, component)?;
                    }
                    self.component = id;
                    self.item = 1;
                } else {
                    self.component = 0;
                    self.item = 0;
                    self.phase += 1;
                    if self.phase == 3 && baseline.nets.len() > MAX_SYMBOLS * 64 {
                        return Err(noncanonical());
                    }
                }
            }
            3 => {
                // Lexicographic comparison, including when malformed nets share long prefixes.
                if let Some(net) = baseline.nets.get(self.net) {
                    self.needs_origin |= net.needs_origin();
                    let order = baseline
                        .nets
                        .get(self.net + 1)
                        .map_or(Ordering::Less, |next| net.terminals.get(self.item).cmp(&next.terminals.get(self.item)));
                    match order {
                        Ordering::Less => self.next_net(),
                        Ordering::Equal if self.item < net.terminals.len() => self.item += 1,
                        _ => return Err(noncanonical()),
                    }
                } else {
                    if baseline.format == NET_ORIGIN_FORMAT && !self.needs_origin {
                        return Err("board baseline format 5 requires an unnamed singleton net".into());
                    }
                    self.net = 0;
                    self.phase = 4;
                }
            }
            4 => {
                if let Some(net) = baseline.nets.get(self.net) {
                    validate_net_header(baseline, net)?;
                    self.phase = 5;
                } else {
                    self.phase = 9;
                }
            }
            5 => {
                let net = &baseline.nets[self.net];
                if let Some(terminal) = net.terminals.get(self.item) {
                    if self.item > 0 && net.terminals[self.item - 1] >= *terminal {
                        return Err("invalid accepted net membership".into());
                    }
                    self.item += 1;
                } else {
                    self.item = 0;
                    self.phase = 6;
                }
            }
            6 => {
                if let Some(name) = baseline.nets[self.net].names.get(self.item) {
                    if name.is_empty()
                        || name.chars().take(201).count() > 200
                        || name.trim() != name
                        || name.chars().any(char::is_control)
                        || !self.names.insert(name.clone())
                    {
                        return Err("invalid accepted net name".into());
                    }
                    self.item += 1;
                } else {
                    self.item = 0;
                    self.phase = 7;
                }
            }
            7 => {
                if self.names.pop_first().is_none() {
                    self.phase = 8;
                }
            }
            8 => {
                if let Some(terminal) = baseline.nets[self.net].terminals.get(self.item) {
                    if !baseline.components.get(&terminal.component).is_some_and(|part| part.terminals.binary_search(&terminal.terminal).is_ok())
                        || !self.seen.insert(terminal.clone())
                    {
                        return Err("accepted net has a duplicate or unknown component terminal".into());
                    }
                    self.item += 1;
                } else {
                    self.next_net();
                    self.phase = 4;
                }
            }
            9 => return Ok(self.seen.pop_first().is_none()),
            _ => return Err("invalid accepted baseline validation phase".into()),
        }
        Ok(false)
    }

    fn next_net(&mut self) {
        self.net += 1;
        self.item = 0;
    }
}

fn noncanonical() -> String {
    "board baseline nets exceed bounds or are not canonical".into()
}

/// Check one historical component's exact identity, value, terminals and package choice.
fn validate_component(baseline: &Baseline, id: u64, component: &PartInstanceState) -> Result<(), String> {
    if !(1..=MAX_OBJECT_ID).contains(&id)
        || component.revision == 0
        || !super::super::definition::valid_id(&component.library)
        || !super::super::definition::valid_id(&component.definition)
        || component.terminals.is_empty()
        || component.terminals.len() > 128
        || !strictly_sorted(&component.terminals)
        || component.terminals.iter().any(|terminal| PinNumber::new(terminal).is_none())
    {
        return Err("invalid historical component identity or terminal set".into());
    }
    crate::schematic::check_reference(&component.reference)?;
    crate::schematic::check_value(&component.value)?;
    if let Some(quantity) = &component.primary_quantity {
        if !matches!(baseline.format.as_str(), QUANTITY_FORMAT | DISPLAY_FORMAT | NET_ORIGIN_FORMAT) {
            return Err("historical primary quantities require board baseline format 3".into());
        }
        quantity.validate()?;
        let Value::Quantity { value, unit } = quantity else {
            return Err("historical primary value must be a typed quantity".into());
        };
        if component.value != format!("{value} {unit}") {
            return Err("historical primary quantity disagrees with stored value text".into());
        }
    }
    if let Some(package) = &component.package {
        package.validate(&component.terminals)?;
    }
    Ok(())
}

/// Check net version, optional presentation and cardinality before reading either array.
fn validate_net_header(baseline: &Baseline, net: &NetState) -> Result<(), String> {
    let stores_origin = baseline.format == NET_ORIGIN_FORMAT;
    let stores_display = matches!(baseline.format.as_str(), DISPLAY_FORMAT | NET_ORIGIN_FORMAT);
    if net.origin.is_some() != (stores_origin && net.needs_origin()) {
        return Err("invalid accepted net origin or baseline version".into());
    }
    if net.display_name.is_some() != stores_display
        || net.display_name.as_ref().is_some_and(|name| name.is_empty() || name.len() > 65_536 || name.chars().any(char::is_control))
    {
        return Err("invalid accepted net display name or baseline version".into());
    }
    if net.terminals.is_empty()
        || (net.needs_origin()
            && !net.terminals.first().is_some_and(|terminal| {
                baseline
                    .components
                    .get(&terminal.component)
                    .and_then(|part| part.package.as_ref())
                    .and_then(|package| package.mapping.get(&terminal.terminal))
                    .is_some_and(|pads| pads.pads().len() > 1)
            }))
        || net.terminals.len() > MAX_SYMBOLS * 128
        || net.names.len() > crate::net_labels::MAX_LABELS * crate::sheets::MAX_SHEETS
    {
        return Err("invalid accepted net membership".into());
    }
    Ok(())
}

/// The native baseline is validated independently, then emitted with its original field
/// order and whitespace. A net's terminal and name arrays are individually resumable.
#[derive(Default)]
pub(crate) struct BaselineEncoding {
    validation: BaselineValidation,
    phase: u8,
    component: u64,
    net: usize,
    item: usize,
    raw_bytes: usize,
}

impl BaselineEncoding {
    /// Count the standalone bytes even in board mode; the destination enforces its own cap.
    pub(crate) fn step(&mut self, baseline: &Baseline, dest: &mut impl Write, embedded: bool) -> Result<bool, String> {
        if self.phase == 0 {
            if self.validation.step(baseline)? {
                self.phase = 1;
            }
            return Ok(false);
        }
        let mut bytes = BaselineStream { dest, raw_bytes: &mut self.raw_bytes, embedded };
        let result = match self.phase {
            1 => {
                write!(
                    bytes,
                    "{{\n  \"format\": \"{}\",\n  \"project\": \"{}\",\n  \"board\": \"{}\",\n  \"source\": ",
                    baseline.format, baseline.project, baseline.board
                )
                .map_err(|error| error.to_string())?;
                json(&mut bytes, &baseline.source)?;
                self.phase = 2;
                bytes.write_all(b",\n  \"components\": {\n")
            }
            2 => {
                if let Some((&id, component)) = baseline.components.range((Excluded(self.component), Unbounded)).next() {
                    if self.component != 0 {
                        bytes.write_all(b"    ,\n").map_err(|error| error.to_string())?;
                    }
                    write!(bytes, "    \"{id}\":").map_err(|error| error.to_string())?;
                    self.component = id;
                    json(&mut bytes, component)?;
                    bytes.write_all(b"\n")
                } else {
                    self.phase = 3;
                    bytes.write_all(b"  },\n  \"nets\": [\n")
                }
            }
            3 => {
                let Some(net) = baseline.nets.get(self.net) else {
                    bytes.write_all(b"  ]\n}").map_err(|error| error.to_string())?;
                    return Ok(true);
                };
                if self.net != 0 {
                    bytes.write_all(b"    ,\n").map_err(|error| error.to_string())?;
                }
                bytes.write_all(b"    {").map_err(|error| error.to_string())?;
                if let Some(name) = &net.display_name {
                    bytes.write_all(b"\"display_name\":").map_err(|error| error.to_string())?;
                    json(&mut bytes, name)?;
                    bytes.write_all(b",").map_err(|error| error.to_string())?;
                }
                self.phase = 4;
                bytes.write_all(b"\"terminals\":[")
            }
            4 => {
                if let Some(terminal) = baseline.nets[self.net].terminals.get(self.item) {
                    array_item(&mut bytes, self.item, terminal)?;
                    self.item += 1;
                    Ok(())
                } else {
                    self.item = 0;
                    self.phase = 5;
                    bytes.write_all(b"],\"names\":[")
                }
            }
            5 => {
                let net = &baseline.nets[self.net];
                if let Some(name) = net.names.get(self.item) {
                    array_item(&mut bytes, self.item, name)?;
                    self.item += 1;
                    Ok(())
                } else {
                    bytes.write_all(b"]").map_err(|error| error.to_string())?;
                    if let Some(origin) = net.origin {
                        bytes.write_all(b",\"origin\":").map_err(|error| error.to_string())?;
                        json(&mut bytes, &origin)?;
                    }
                    self.net += 1;
                    self.item = 0;
                    self.phase = 3;
                    bytes.write_all(b"}\n")
                }
            }
            _ => return Err("invalid accepted baseline encoding phase".into()),
        };
        result.map_err(|error| error.to_string())?;
        Ok(false)
    }
}

/// Serialize a compact array member, inserting a comma only after the first member.
fn array_item(bytes: &mut impl Write, index: usize, value: &impl serde::Serialize) -> Result<(), String> {
    if index != 0 {
        bytes.write_all(b",").map_err(|error| error.to_string())?;
    }
    json(bytes, value)
}

fn json(bytes: &mut impl Write, value: &impl serde::Serialize) -> Result<(), String> {
    serde_json::to_writer(bytes, value).map_err(|error| error.to_string())
}

struct BaselineStream<'a, W> {
    dest: &'a mut W,
    raw_bytes: &'a mut usize,
    embedded: bool,
}

impl<W: Write> Write for BaselineStream<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let length = self
            .raw_bytes
            .checked_add(bytes.len())
            .filter(|length| *length <= MAX_BASELINE_BYTES)
            .ok_or_else(|| io::Error::other("board baseline exceeds its byte limit"))?;
        if self.embedded && bytes.contains(&b'\n') {
            for (index, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
                if index != 0 {
                    self.dest.write_all(b"\n  ")?;
                }
                self.dest.write_all(line)?;
            }
        } else {
            self.dest.write_all(bytes)?;
        }
        *self.raw_bytes = length;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.dest.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::update::SourceCapture;
    use uuid::Uuid;

    /// A large accepted net emits one terminal at a time without changing compact JSON or indentation.
    #[test]
    fn large_accepted_net_keeps_native_bytes_and_resumes_between_terminals() {
        const TERMINALS: u64 = 1024;
        let part = PartInstanceState {
            library: "basic".into(),
            definition: "resistor".into(),
            revision: 1,
            reference: "R1".into(),
            value: "10k".into(),
            primary_quantity: None,
            terminals: vec!["1".into()],
            package: None,
        };
        let mut baseline = Baseline {
            format: DISPLAY_FORMAT.into(),
            project: Uuid::from_u128(1),
            board: Uuid::from_u128(2),
            source: SourceCapture { capture_id: Uuid::from_u128(3), content_digest: "a".repeat(64) },
            components: (1..=TERMINALS).map(|id| (id, part.clone())).collect(),
            nets: vec![NetState {
                display_name: Some("Accepted \"signal\"".into()),
                terminals: (1..=TERMINALS).map(|component| TerminalKey { component, terminal: "1".into() }).collect(),
                names: vec!["Shared".into(), "Clock".into()],
                origin: None,
            }],
        };
        let standalone = baseline.encode().unwrap();
        let old_compact_net_row = format!("    {}\n", serde_json::to_string(&baseline.nets[0]).unwrap());
        assert!(std::str::from_utf8(&standalone).unwrap().contains(old_compact_net_row.as_str()));
        assert_eq!(Baseline::decode(&standalone).unwrap(), baseline);
        let mut writer = BaselineEncoding::default();
        let mut embedded = Vec::new();
        let mut terminal_steps = 0;
        loop {
            let before = (writer.phase, writer.item);
            if writer.step(&baseline, &mut embedded, true).unwrap() {
                break;
            }
            if before.0 == 4 && writer.phase == 4 {
                assert_eq!(writer.item, before.1 + 1);
                terminal_steps += 1;
            }
        }
        assert_eq!(terminal_steps, TERMINALS);
        assert_eq!(embedded, std::str::from_utf8(&standalone).unwrap().replace('\n', "\n  ").into_bytes());

        let reused_terminal = baseline.nets[0].terminals[0].clone();
        baseline
            .nets
            .insert(0, NetState { display_name: Some("Other".into()), terminals: vec![reused_terminal], names: vec!["Other".into()], origin: None });
        assert!(baseline.encode().unwrap_err().contains("duplicate or unknown component terminal"));
    }
}
