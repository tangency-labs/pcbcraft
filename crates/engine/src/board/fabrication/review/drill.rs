use super::{CoordinateFrame, Layer, LayerRole, Mark, Tool, add, decimal, xy};
use std::collections::BTreeMap;

/// Read the writer's absolute metric Excellon drill hits and tool declarations.
/// Require one plating declaration and reject motion modes that could imply routing or slots.
/// A legacy drill file has no coordinate frame identifier.
pub(super) fn read(text: &str, budget: &mut usize) -> Result<(Layer, CoordinateFrame), String> {
    let mut lines = text.lines();
    if lines.next() != Some("M48") {
        return Err("Expected an Excellon header".into());
    }
    let (mut role, mut format, mut metric) = (None, false, false);
    let mut frame = None;
    let mut tools = BTreeMap::<u32, Tool>::new();
    loop {
        let record = lines.next().ok_or("Incomplete Excellon header")?;
        if let Some(value) = record.strip_prefix("; #@! TF.SameCoordinates,") {
            if frame.replace(CoordinateFrame::parse(value)?).is_some() {
                return Err("Duplicate drill coordinate frame declaration".into());
            }
            continue;
        }
        match record {
            "%" => break,
            "FMAT,2" => format = true,
            "METRIC" => metric = true,
            "; TYPE=PLATED" if role.is_none() => role = Some(LayerRole::PlatedDrill),
            "; TYPE=NON_PLATED" if role.is_none() => role = Some(LayerRole::NonPlatedDrill),
            c if c.starts_with("; TYPE=") => return Err("Invalid or duplicate drill plating declaration".into()),
            c if c.starts_with(';') || c.is_empty() => {}
            c => {
                let (number, diameter) = c.strip_prefix('T').and_then(|s| s.split_once('C')).ok_or("Unsupported Excellon header record")?;
                let number = number.parse::<u32>().map_err(|_| "Invalid drill tool number")?;
                let diameter_nm = decimal(diameter)?;
                if number == 0 || diameter_nm <= 0 || tools.len() >= 65_536 || tools.insert(number, Tool { number, diameter_nm, hits: 0 }).is_some() {
                    return Err("Invalid or duplicate drill tool".into());
                }
            }
        }
    }
    if !format || !metric || lines.next() != Some("G90") || lines.next() != Some("G05") {
        return Err("Expected absolute metric coordinates in drill mode".into());
    }
    let mut active = None;
    let mut marks = Vec::new();
    loop {
        let record = lines.next().ok_or("Missing Excellon end record")?;
        if record == "M30" {
            break;
        }
        if let Some(t) = record.strip_prefix('T') {
            let number = t.parse::<u32>().map_err(|_| "Invalid drill tool selection")?;
            if !tools.contains_key(&number) {
                return Err("Undefined drill tool".into());
            }
            active = Some(number);
        } else {
            let center = xy(record, decimal)?;
            let tool = active.and_then(|id| tools.get_mut(&id)).ok_or("No active drill tool")?;
            add(&mut marks, Mark::Stroke { ends: [center; 2], diameter_nm: tool.diameter_nm }, budget)?;
            tool.hits += 1;
        }
    }
    if lines.any(|line| !line.trim().is_empty()) {
        return Err("Unexpected records after Excellon end".into());
    }
    let layer = Layer { name: String::new(), role: role.ok_or("Drill plating is unspecified")?, marks, tools: tools.into_values().collect() };
    Ok((layer, frame.unwrap_or(CoordinateFrame::Legacy)))
}
