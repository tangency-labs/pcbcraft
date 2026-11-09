//! Explicit conversions from ordinary text into native portable fragments, never an EDA importer.
use crate::{
    CommandError, Session,
    board::{
        clipboard::{BoardFragment, BoardSelection},
        setup::LayerPurpose,
    },
    clipboard::Fragment,
    schematic::Point,
    selection::Selection,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// Prepare one-column local/global net names without editing the destination session.
pub const LABELS: &str = "schematic.prepare_text_paste";
/// Prepare a numeric CSV/TSV polyline on Board Outline, independently of a destination board.
pub const OUTLINE: &str = "pcb.prepare_coordinate_paste";
/// Source coordinate units are explicit and independent of the destination display preference.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CoordinateUnits {
    /// Millimeters, exactly 1,000,000 nanometers per unit.
    Mm,
    /// Thousandths of an inch, exactly 25,400 nanometers per unit.
    Mil,
}

/// Limit ordinary text before splitting or numeric parsing.
pub const MAX_TEXT_BYTES: usize = 65_536;

/// Keep preparation failures in the normal command error channel.
fn invalid(text: impl ToString) -> CommandError {
    CommandError::InvalidParams(text.to_string())
}
/// Ordinary text cannot silently reinterpret a native JSON or other structured EDA document.
fn plain(text: &str) -> Result<&str, CommandError> {
    if text.len() > MAX_TEXT_BYTES {
        return Err(invalid("external clipboard text exceeds 64 KiB"));
    }
    let text = text.trim_start_matches('\u{feff}').trim();
    if text.is_empty() || text.starts_with(['{', '[', '<', '(']) || text.contains("pcbcraft-clipboard/") {
        return Err(invalid("Choose ordinary net-name lines or a numeric coordinate table; native/structured EDA payloads are unsupported."));
    }
    Ok(text)
}
/// Build labels through ordinary native commands; a single invalid row rejects the whole fragment.
fn labels(params: &Value) -> Result<String, CommandError> {
    let text = plain(params["text"].as_str().unwrap_or_default())?;
    let names: Vec<_> = text.lines().map(str::trim).collect();
    if names.is_empty() || names.len() > crate::net_labels::MAX_LABELS || names.iter().any(|s| s.is_empty() || s.contains('\t')) {
        return Err(invalid("Use 1–256 nonempty net-name lines in one column."));
    }
    let spacing = params["spacing_nm"].as_i64().unwrap_or_default();
    if spacing <= 0 {
        return Err(invalid("Label spacing must be positive."));
    }
    let suffix = params["net_name_suffix"].as_str().unwrap_or_default();
    let mut source = Session::default();
    source.apply("project.new", &json!({"name":"Pasted net names"}))?;
    for (index, name) in names.iter().enumerate() {
        let y = (index as i64).checked_mul(spacing).ok_or_else(|| invalid("Label spacing exceeds coordinate bounds."))?;
        source.apply(
            crate::net_labels::ADD,
            &json!({"name":format!("{name}{suffix}"),"style":params["style"],"x_nm":0,"y_nm":y,"expected_revision":source.revision()}),
        )?;
    }
    let project = source.project().ok_or_else(|| invalid("temporary label fragment unavailable"))?;
    Fragment::capture(project, &Selection { net_labels: project.net_labels().iter().map(|l| l.object_id()).collect(), ..Default::default() })?
        .to_text()
}
/// Read a numeric cell, accepting spreadsheet quoting but no embedded delimiters or formulas.
fn cell(text: &str) -> Result<&str, CommandError> {
    let text = text.trim();
    let text = if text.starts_with('"') {
        text.strip_prefix('"').and_then(|s| s.strip_suffix('"')).ok_or_else(|| invalid("Unbalanced coordinate cell quotes."))?
    } else {
        text
    };
    if text.is_empty() || text.contains('"') {
        return Err(invalid("A coordinate cell must contain one finite number."));
    }
    Ok(text)
}
/// Convert once to whole nanometers and check native bounds before casting or constructing points.
fn coordinate(text: &str, per_unit: f64) -> Result<i64, CommandError> {
    let value: f64 = cell(text)?.parse().map_err(|_| invalid("Coordinate cells must contain numbers, not formulas or text."))?;
    let nm = (value * per_unit).round();
    if !nm.is_finite() || nm.abs() > crate::schematic::MAX_CENTER_NM as f64 {
        return Err(invalid("Coordinate exceeds native bounds or is not finite."));
    }
    Ok(nm as i64)
}
/// Parse one strict two-column table without allocating project identities. Optional headers may
/// state units but cannot disagree with the explicit choice. Bounds, finite values, row count and
/// distinct consecutive vertices are checked as a complete batch; a closed repeated endpoint is
/// normalized once. This supplies the conversion form's dimensions and authoritative preparation.
pub fn coordinate_points(text: &str, units: CoordinateUnits, closed: bool, invert_y: bool) -> Result<Vec<Point>, CommandError> {
    let text = plain(text)?;
    let unit = match units {
        CoordinateUnits::Mm => "mm",
        CoordinateUnits::Mil => "mil",
    };
    let per_unit = match units {
        CoordinateUnits::Mm => 1_000_000.,
        CoordinateUnits::Mil => 25_400.,
    };
    let delimiter = if text.lines().next().is_some_and(|s| s.contains('\t')) { '\t' } else { ',' };
    let mut points = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let cells: Vec<_> = line.split(delimiter).collect();
        if cells.len() != 2 {
            return Err(invalid(format!("Row {} needs exactly two coordinate columns.", index + 1)));
        }
        let header = [cell(cells[0])?.to_ascii_lowercase(), cell(cells[1])?.to_ascii_lowercase()];
        if index == 0 && matches!((header[0].as_str(), header[1].as_str()), ("x", "y") | ("x_mm", "y_mm") | ("x_mil", "y_mil")) {
            if (header[0] == "x_mm" && unit != "mm") || (header[0] == "x_mil" && unit != "mil") {
                return Err(invalid("Coordinate header units disagree with the source-unit choice."));
            }
            continue;
        }
        let mut point = Point {
            x_nm: coordinate(cells[0], per_unit).map_err(|e| invalid(format!("Row {}: {e}", index + 1)))?,
            y_nm: coordinate(cells[1], per_unit).map_err(|e| invalid(format!("Row {}: {e}", index + 1)))?,
        };
        if invert_y {
            point.y_nm = -point.y_nm;
        }
        points.push(point);
        if points.len() > 257 {
            return Err(invalid("Use at most 256 vertices (plus an optional repeated closing point)."));
        }
    }
    if closed && points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
    if !((if closed { 3 } else { 2 })..=256).contains(&points.len()) || points.windows(2).any(|p| p[0] == p[1]) {
        return Err(invalid("Use 2–256 distinct consecutive vertices, or at least 3 for a closed outline."));
    }
    Ok(points)
}
/// Prepare an outline through a temporary native project; destination setup/layers are never read
/// or overwritten. Drawing validation catches duplicate vertices and invalid open/closed counts.
fn outline(params: &Value) -> Result<String, CommandError> {
    let units = serde_json::from_value(params["units"].clone()).map_err(invalid)?;
    let points = coordinate_points(
        params["text"].as_str().unwrap_or_default(),
        units,
        params["closed"].as_bool().unwrap_or(false),
        params["invert_y"].as_bool().unwrap_or(false),
    )?;
    let mut source = Session::default();
    source.apply("project.new", &json!({"name":"Pasted outline","create_board":true}))?;
    let project = source.project().ok_or_else(|| invalid("temporary board unavailable"))?;
    let target = project.board_target().ok_or_else(|| invalid("temporary board unavailable"))?;
    let layer = project
        .board_view(target)
        .and_then(|b| b.layers().into_iter().find(|l| l.purpose == LayerPurpose::Outline))
        .ok_or_else(|| invalid("outline layer unavailable"))?
        .id;
    source.apply(
        crate::ADD_DRAWING,
        &json!({"board":target.board,"layer":layer,"points":points,"closed":params["closed"],"expected_revision":source.revision()}),
    )?;
    let project = source.project().ok_or_else(|| invalid("temporary board unavailable"))?;
    let drawings: BTreeSet<_> =
        project.board_view(target).ok_or_else(|| invalid("temporary board unavailable"))?.drawings().iter().map(|d| d.id).collect();
    BoardFragment::capture(project, &BoardSelection { drawings, ..Default::default() }, false)?.to_text()
}
impl Session {
    /// Prepare portable text with the shared validators while retaining this session's snapshot,
    /// revision, save state and history. Preparation grants no paste or operating-system authority.
    pub(crate) fn prepare_text_clipboard(&self, id: &str, params: &Value) -> Result<String, CommandError> {
        crate::check_params(crate::command(id).ok_or_else(|| invalid("preparation command unavailable"))?, params)?;
        self.check_revision(params)?;
        match id {
            LABELS => labels(params),
            OUTLINE => outline(params),
            _ => Err(invalid("unsupported clipboard conversion")),
        }
    }
}

#[cfg(test)]
mod tests;
