//! Shared command composition and native output for the two distributed demonstrations.
use pcbcraft_engine::{DRAW_WIRE, PLACE_SYMBOL, SET_SYMBOL_FIELDS, Session, schematic::GRID_NM};
use serde_json::{Value, json};
pub(crate) mod copper;
use std::{collections::BTreeMap, error::Error};

/// The generated session and role-addressed symbol IDs keep board mapping independent of allocation order.
pub(crate) struct Demo {
    pub session: Session,
    pub symbols: BTreeMap<&'static str, String>,
}

impl Demo {
    /// Create a new project with one explicit board and stable role lookup.
    pub(crate) fn new(name: &str) -> Result<Self, Box<dyn Error>> {
        let mut demo = Self { session: Session::default(), symbols: BTreeMap::new() };
        demo.session.apply("project.new", &json!({"create_board":true,"name":name}))?;
        Ok(demo)
    }

    /// Place an exact catalog definition, then apply instance fields and pose through normal history.
    pub(crate) fn place(
        &mut self,
        reference: &'static str,
        library: &str,
        id: &str,
        value: &str,
        pose: (i64, i64, i64, bool),
    ) -> Result<(), Box<dyn Error>> {
        let (x, y, rotation, mirrored) = pose;
        let definition = self.session.libraries().latest_symbol(library, id).ok_or("catalog symbol missing")?;
        self.session.apply(PLACE_SYMBOL, &json!({"library":library,"definition":id,"definition_revision":definition.revision(),"definition_digest":pcbcraft_engine::library::sources::symbol_digest(&definition)?,"x_nm":x*GRID_NM,"y_nm":y*GRID_NM,"expected_revision":self.session.revision()}))?;
        let symbol = self.session.project().and_then(|p| p.symbols().last()).ok_or("placed symbol missing")?.id();
        self.session
            .apply(
                SET_SYMBOL_FIELDS,
                &json!({"symbol":symbol,"reference":reference,"value":value,"rotation":rotation,"expected_revision":self.session.revision()}),
            )
            .map_err(|e| format!("fields {reference}: {e}"))?;
        if mirrored {
            self.session
                .apply(pcbcraft_engine::MIRROR_SYMBOL, &json!({"symbol":symbol,"axis":"horizontal","expected_revision":self.session.revision()}))?;
        }
        self.symbols.insert(reference, symbol);
        Ok(())
    }
    /// Refer to a logical terminal by its generated stable identity and explicit terminal number.
    pub(crate) fn pin(&self, reference: &str, terminal: &str) -> Result<Value, Box<dyn Error>> {
        let symbol = self.symbols.get(reference).ok_or_else(|| format!("missing symbol {reference}"))?;
        Ok(json!({"symbol":symbol,"terminal":terminal}))
    }
    /// Commit one orthogonal route and retain its logical destination for subsequent contacts.
    pub(crate) fn wire(&mut self, from: Value, to: Value, bends: &[(i64, i64)]) -> Result<Value, Box<dyn Error>> {
        let destination = to.clone();
        let from = self.endpoint(from)?;
        let to = self.endpoint(to)?;
        let bends: Vec<_> = bends.iter().map(|&(x, y)| point(x, y)).collect();
        self.session
            .apply(DRAW_WIRE, &json!({"from":from,"to":to,"bends":bends,"expected_revision":self.session.revision()}))
            .map_err(|error| format!("wire {from} to {to}: {error}"))?;
        Ok(destination)
    }

    /// Resolve a planned coordinate against the current snapshot, since ordinary wire cleanup
    /// may merge intermediate routes. Ambiguous crossings require an explicit authored contact.
    fn endpoint(&self, value: Value) -> Result<Value, Box<dyn Error>> {
        let (Some(x_nm), Some(y_nm)) = (value["x_nm"].as_i64(), value["y_nm"].as_i64()) else {
            return Ok(value);
        };
        let at = pcbcraft_engine::schematic::Point { x_nm, y_nm };
        let project = self.session.project().ok_or("missing project")?;
        if let Some((id, _)) = project.junctions().find(|(_, p)| *p == at) {
            return Ok(json!({"junction":format!("junction-{id}")}));
        }
        let wires: Vec<_> = project.wires().iter().filter(|wire| pcbcraft_engine::wire::path_contains_point(&project.wire_path(wire), at)).collect();
        match wires.as_slice() {
            [] => Ok(value),
            [wire] => Ok(json!({"wire":wire.id(),"at":at})),
            _ => Err(format!("ambiguous planned wire contact at {at:?}").into()),
        }
    }
    /// Place a local label at an existing route or junction; matching probe labels share its net.
    pub(crate) fn label(&mut self, name: &str, x: i64, y: i64) -> Result<(), Box<dyn Error>> {
        self.session
            .apply(
                pcbcraft_engine::net_labels::ADD,
                &json!({"name":name,"style":"local","x_nm":x*GRID_NM,"y_nm":y*GRID_NM,"rotation":0,"expected_revision":self.session.revision()}),
            )
            .map_err(|e| format!("label {name} at {x},{y}: {e}"))?;
        Ok(())
    }
}
/// Represent a coordinate on the schematic's 1.27 mm grid, never a saved-file edit.
pub(crate) fn point(x: i64, y: i64) -> Value {
    json!({"x_nm":x*GRID_NM,"y_nm":y*GRID_NM})
}

/// Validate accepted electrical/physical connectivity and encode both native forms before writing.
/// Refuse an existing output directory; a filesystem error reports any partial destination.
pub(crate) fn publish(
    session: Session,
    destination: std::path::PathBuf,
    stem: &str,
    expected_nets: usize,
    expected_terminals: usize,
) -> Result<(), Box<dyn Error>> {
    use pcbcraft_engine::persistence::{encode_documents, encode_packed};
    use std::fs;
    let project = session.snapshot().ok_or("demo has no project")?;
    let connectivity = project.electrical_connectivity();
    if connectivity.nets.len() != expected_nets || !connectivity.unconnected.is_empty() {
        return Err("demo circuit has unexpected or unconnected nets".into());
    }
    let target = project.board_target().ok_or("demo has no board target")?;
    let ratsnest = project.board_view(target).and_then(|board| board.ratsnest()).ok_or("demo has no accepted board connectivity")?;
    if ratsnest.terminal_count != expected_terminals || !ratsnest.missing.is_empty() || !ratsnest.lines.is_empty() {
        return Err(format!("demo is not completely routed: {} airwires, {} missing terminals", ratsnest.lines.len(), ratsnest.missing.len()).into());
    }
    let report = session.check_design(&serde_json::json!({}))?;
    if report["total"].as_u64() != Some(0) {
        return Err(format!("demo board failed its design check: {report}").into());
    }
    let documents = encode_documents(project.clone())?;
    let archive = encode_packed(project)?;
    fs::create_dir(&destination).map_err(|error| format!("cannot create {}: {error}", destination.display()))?;
    let publish = || -> Result<(), Box<dyn Error>> {
        for (relative, bytes) in documents {
            let path = destination.join(stem).join(relative);
            fs::create_dir_all(path.parent().ok_or("document has no parent")?)?;
            fs::write(path, bytes)?;
        }
        fs::write(destination.join(format!("{stem}.pc")), archive)?;
        Ok(())
    };
    publish().map_err(|error| format!("incomplete output at {}: {error}", destination.display()))?;
    println!("Open {}", destination.join(stem).join("project.json").display());
    Ok(())
}
