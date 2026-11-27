//! Vector bus labels: one label that names an ordered range of member nets.
//!
//! A bus name is written `PREFIX[FIRST..LAST]`, for example `D[0..7]`:
//! - `PREFIX` has at least one character, contains no brackets, whitespace or control
//!   characters, and does not end in a decimal digit (so `A1[0..3]` cannot be confused with
//!   `A[10..13]`);
//! - `FIRST` and `LAST` are whole numbers from 0 to 9999, written in decimal without a sign or
//!   leading zeros;
//! - the range runs in written order, ascending (`D[0..7]`) or descending (`D[7..0]`), and
//!   holds from 2 to [`MAX_WIDTH`] members;
//! - nothing follows the closing bracket.
//!
//! Member `i` is named `PREFIX` followed by `i` in decimal, so `D[0..7]` stands for D0 to D7.
//! The meaning comes from the label's `bus` style, never from its spelling: a local or global
//! label named `D[0..7]` still names one net with that literal name.
use super::*;

/// Most members one bus label may carry.
pub const MAX_WIDTH: usize = 64;
/// Largest range index; four decimal digits keep member names short.
const MAX_INDEX: u32 = 9999;

/// Expand a bus name into its member names in written order, or explain why it is not a bus.
pub fn members(name: &str) -> Result<Vec<String>, String> {
    let (prefix, rest) = name.split_once('[').ok_or("a bus name has the form PREFIX[FIRST..LAST], for example D[0..7]")?;
    let range = rest.strip_suffix(']').ok_or("a bus name ends with its closing bracket, as in D[0..7]")?;
    if prefix.is_empty() {
        return Err("a bus name needs a prefix before the bracket, as in D[0..7]".into());
    }
    if prefix.chars().any(|c| c == ']' || c.is_whitespace() || c.is_control()) {
        return Err("a bus prefix cannot contain brackets, whitespace or control characters".into());
    }
    if prefix.ends_with(|c: char| c.is_ascii_digit()) {
        return Err("a bus prefix cannot end in a digit, because its member names would be ambiguous".into());
    }
    let (first, last) = range.split_once("..").ok_or("a bus range is written FIRST..LAST, as in [0..7]")?;
    let (first, last) = (index(first)?, index(last)?);
    let span = first.abs_diff(last);
    if span < 1 {
        return Err("a bus needs at least two members".into());
    }
    if span as usize >= MAX_WIDTH {
        return Err(format!("a bus carries at most {MAX_WIDTH} members"));
    }
    Ok((0..=span).map(|step| if first <= last { first + step } else { first - step }).map(|i| format!("{prefix}{i}")).collect())
}

/// Parse one range bound: 1–4 decimal digits, no sign and no leading zero.
fn index(text: &str) -> Result<u32, String> {
    let error = || format!("a bus index is a whole number from 0 to {MAX_INDEX} without leading zeros");
    if text.is_empty() || text.len() > 4 || !text.bytes().all(|b| b.is_ascii_digit()) || (text.len() > 1 && text.starts_with('0')) {
        return Err(error());
    }
    text.parse().map_err(|_| error())
}

impl Project {
    /// Inspect a label; a bus label also reports each member with the net it joins on this
    /// sheet, or null when no ordinary label, hierarchical label or power terminal of that
    /// name exists here.
    pub(crate) fn inspect_net_label(&self, label: &Label) -> Value {
        let mut value = label.inspect();
        if label.style == Style::Bus {
            let graph = self.borrowed_electrical_connectivity();
            let resolved: Vec<_> = members(&label.name)
                .unwrap_or_default()
                .into_iter()
                .map(|name| {
                    let net = self.member_contact(&name).and_then(|end| graph.net_index_for_endpoint(end)).map(|index| graph.net_name(index));
                    json!({"name": name, "net": net})
                })
                .collect();
            value["members"] = json!(resolved);
        }
        value
    }

    /// The first same-sheet anchor that gives a member name its net. Every anchor of one name
    /// on a sheet is already joined, so any of them identifies the same net.
    fn member_contact(&self, name: &str) -> Option<WireEndpoint> {
        self.net_labels()
            .iter()
            .filter(|label| label.style.contact() && label.name == name)
            .map(|label| WireEndpoint::NetLabel(label.id))
            .chain(self.hierarchical_labels().iter().filter(|label| label.name == name).map(|label| WireEndpoint::Label(label.id)))
            .chain(self.power_terminals().filter(|(_, value, _)| *value == name).map(|(end, _, _)| end))
            .next()
    }
}
