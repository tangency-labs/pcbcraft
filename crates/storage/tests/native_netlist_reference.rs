//! Compare the app's export with the oracle for the independent reading example.

use pcbcraft_engine::{
    Session,
    netlist::{EXPORT_NETLIST, Netlist, TerminalRef},
};
use serde_json::json;
use std::path::Path;

type Terminal = (u64, String);

/// Translate exported command handles to the saved file's numeric part IDs.
fn part_id(id: &str) -> Result<u64, String> {
    id.strip_prefix("component-").and_then(|number| number.parse().ok()).ok_or_else(|| format!("unexpected part handle {id:?}"))
}

/// Ignore export presentation order while retaining every logical terminal address.
fn members(references: Vec<TerminalRef>) -> Result<Vec<Terminal>, String> {
    let mut members =
        references.into_iter().map(|reference| Ok((part_id(&reference.part)?, reference.terminal))).collect::<Result<Vec<_>, String>>()?;
    members.sort();
    Ok(members)
}

/// Open the exact archive used by the independent Python reader, then compare the actual
/// export command's parts, terminals and membership to the same committed projection.
#[test]
fn native_netlist_matches_the_independent_reader_oracle() -> Result<(), String> {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/native-open/dual-led-editing-demo-v3.pc");
    let mut session = Session::default();
    session.open_snapshot(pcbcraft_storage::open(&file)?);
    let reply = session.execute(EXPORT_NETLIST, &json!({})).map_err(|error| error.to_string())?;
    let text = reply["netlist"]["files"][0]["contents"].as_str().ok_or("expected netlist export text")?;
    let netlist = Netlist::parse(text)?;
    let mut parts = netlist
        .components
        .into_iter()
        .map(|part| {
            let mut pins: Vec<_> = part.terminals.into_iter().map(|pin| (pin.terminal, pin.name, pin.electrical_type)).collect();
            pins.sort_by(|a, b| a.0.cmp(&b.0));
            Ok((part_id(&part.id)?, part.reference, part.value, pins))
        })
        .collect::<Result<Vec<_>, String>>()?;
    parts.sort_by_key(|part| part.0);
    let mut nets = netlist.nets.into_iter().map(|net| members(net.terminals)).collect::<Result<Vec<_>, _>>()?;
    nets.sort();
    let actual = json!({"parts": parts, "nets": nets, "unconnected": members(netlist.unconnected)?});
    let expected: serde_json::Value = serde_json::from_str(include_str!("fixtures/native-open/dual-led-editing-demo-connectivity.json"))
        .map_err(|error| format!("invalid connectivity oracle: {error}"))?;
    assert_eq!(actual, expected);
    Ok(())
}
