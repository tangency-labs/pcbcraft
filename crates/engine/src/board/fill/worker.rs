//! Ephemeral, same-build worker transport; native document codecs retain model authority.
//! The host keeps session/view/revision authority separately from this byte exchange.
use super::*;
use crate::persistence::{Documents, decode_documents, encode_documents};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u32,
    target: BoardTarget,
    documents: BTreeMap<String, String>,
    source: String,
    diagnostics: Option<super::super::rules::Values>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    version: u32,
    source: String,
    result: Result<WireFill, String>,
    diagnostics: Option<Vec<Problem>>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireFill {
    board: BoardTarget,
    zones: Vec<WireZone>,
    problems: Vec<Problem>,
    pads: Vec<((String, String), usize)>,
    tracks: BTreeMap<Uuid, usize>,
    vias: BTreeMap<Uuid, usize>,
    members: Vec<Vec<usize>>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireZone {
    id: Uuid,
    net: Uuid,
    layer: Uuid,
    islands: Vec<WireIsland>,
    dropped: usize,
    clearance_nm: i64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireIsland {
    region: Vec<u8>,
    mesh: DisplayMesh,
    contacts: Vec<usize>,
}

/// Bind a request to the full canonical native source bundle and explicit physical board.
fn digest(target: BoardTarget, documents: &BTreeMap<String, String>, diagnostics: Option<super::super::rules::Values>) -> Result<String, String> {
    let encoded = serde_json::to_vec(&(target, documents, diagnostics)).map_err(|e| e.to_string())?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}

/// Validated same-source worker material plus optional checker preparation.
struct Accepted {
    fill: Arc<Fill>,
    diagnostics: Option<Vec<Problem>>,
}

impl FillJob {
    /// Capture one source bundle for an isolated browser worker. This occurs once per started
    /// committed-source job, never once per pointer sample or queued/coalesced replacement.
    pub fn worker_request(&mut self) -> Result<String, String> {
        self.worker_request_with_diagnostics(None)
    }

    /// Checkers request zone diagnostics in the same isolated execution as fill geometry.
    pub(crate) fn worker_request_with_diagnostics(&mut self, diagnostics: Option<super::super::rules::Values>) -> Result<String, String> {
        if !self.source.fill_current() {
            return Err("Obsolete pour request.".into());
        }
        let documents = encode_documents(self.source.snapshot.clone())?
            .into_iter()
            .map(|(path, bytes)| String::from_utf8(bytes).map(|text| (path, text)).map_err(|e| e.to_string()))
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let source = digest(self.source.target, &documents, diagnostics)?;
        self.worker_hash = Some(source.clone());
        serde_json::to_string(&Request { version: 1, target: self.source.target, documents, source, diagnostics }).map_err(|e| e.to_string())
    }

    /// Accept only the identically addressed request, validating all region profiles and mesh
    /// indices before the captured slot can become ready. The host still checks live authority.
    pub fn accept_worker_response(&mut self, response: &str) -> Result<Option<Arc<Fill>>, String> {
        self.accept_response(response).map(|accepted| Some(accepted.fill))
    }

    /// Publish a complete fill and return its same-source analytic diagnostic findings.
    pub(crate) fn accept_worker_diagnostics(&mut self, response: &str) -> Result<Vec<Problem>, String> {
        self.accept_response(response)?.diagnostics.ok_or_else(|| "Fill worker omitted checker diagnostics.".into())
    }

    /// Decode once, validate source/geometry, and publish only to this captured source slot.
    fn accept_response(&mut self, response: &str) -> Result<Accepted, String> {
        if response.len() > 128 * 1024 * 1024 {
            return Err("Fill worker response exceeds its bound.".into());
        }
        super::check_cancel(Some(&self.cancellation()))?;
        if !self.source.fill_current() {
            return Err("Obsolete pour response.".into());
        }
        let response: Response = serde_json::from_str(response).map_err(|e| e.to_string())?;
        if response.version != 1 || self.worker_hash.as_ref() != Some(&response.source) {
            return Err("Obsolete or foreign fill worker response.".into());
        }
        let result = response
            .result
            .and_then(|wire| decode_fill(wire, self.source.snapshot.board_view(self.source.target).ok_or("Missing fill source board.")?))
            .map(Arc::new);
        let _ = self.slot.set(result.clone());
        result.map(|fill| Accepted { fill, diagnostics: response.diagnostics })
    }
}

/// Run entirely inside an isolated worker/module. Malformed requests return a response error;
/// a WASM abort/trap is caught by the JavaScript host, which retires that worker instance.
pub fn execute(request: &str) -> String {
    let run = || -> Result<Response, String> {
        if request.len() > 128 * 1024 * 1024 {
            return Err("Fill worker request exceeds its bound.".into());
        }
        let request: Request = serde_json::from_str(request).map_err(|e| e.to_string())?;
        if request.version != 1 || digest(request.target, &request.documents, request.diagnostics)? != request.source {
            return Err("Incompatible fill worker source.".into());
        }
        let documents: Documents = request.documents.into_iter().map(|(path, text)| (path, text.into_bytes())).collect();
        let mut diagnostics = None;
        let result = decode_documents(&documents).and_then(|project| {
            let view = project.board_view(request.target).ok_or("Unknown fill worker board.")?;
            let filled = view.fill_for_check()?;
            if let Some(rules) = request.diagnostics {
                diagnostics = Some(super::diagnostics(view, &BoardGeometry::new(view), rules));
            }
            encode_fill(filled.as_ref())
        });
        Ok(Response { version: 1, source: request.source, result, diagnostics })
    };
    let response = run().unwrap_or_else(|error| Response { version: 1, source: String::new(), result: Err(error), diagnostics: None });
    serde_json::to_string(&response)
        .unwrap_or_else(|_| "{\"version\":1,\"source\":\"\",\"result\":{\"Err\":\"Fill response encoding failed\"}}".into())
}

/// Keep transport independent of JSON's object-key limitations for tuple-addressed pads.
fn encode_fill(fill: &Fill) -> Result<WireFill, String> {
    let zones = fill
        .zones
        .iter()
        .map(|zone| {
            let islands = zone
                .islands
                .iter()
                .map(|island| {
                    Ok(WireIsland {
                        region: island.region.worker_bytes().map_err(|e| e.to_string())?,
                        mesh: island.mesh.clone(),
                        contacts: island.contacts.clone(),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(WireZone { id: zone.id, net: zone.net, layer: zone.layer, islands, dropped: zone.dropped, clearance_nm: zone.clearance_nm })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(WireFill {
        board: fill.board,
        zones,
        problems: fill.problems.clone(),
        pads: fill.copper.pads.iter().map(|(key, value)| (key.clone(), *value)).collect(),
        tracks: fill.copper.tracks.clone(),
        vias: fill.copper.vias.clone(),
        members: fill.members.clone(),
    })
}

/// Refuse malformed geometry or connectivity indices before they reach a canvas/query caller.
fn decode_fill(wire: WireFill, view: BoardView<'_>) -> Result<Fill, String> {
    if wire.board != view.target || wire.zones.len() > super::super::zones::MAX_ZONES || wire.members.len() > 1_000_000 {
        return Err("Invalid fill worker result owner or resource bound.".into());
    }
    let mut seen = BTreeSet::new();
    let mut zones = Vec::new();
    for zone in wire.zones {
        if !seen.insert(zone.id)
            || !view.zones().iter().any(|authored| {
                authored.id == zone.id && authored.net == zone.net && authored.layer == zone.layer && view.zone_net_state(authored.net) == "active"
            })
        {
            return Err("Invalid fill worker zone identity.".into());
        }
        let mut islands = Vec::new();
        for island in zone.islands {
            if island.mesh.vertices_nm.len() > 1_000_000
                || island.mesh.triangles.len() > 1_000_000
                || island.mesh.vertices_nm.iter().flatten().any(|v| !v.is_finite() || v.abs() > pcbcraft_units::MAX_LENGTH_NM as f64)
                || island.mesh.triangles.iter().flatten().any(|i| *i as usize >= island.mesh.vertices_nm.len())
            {
                return Err("Invalid fill worker display mesh.".into());
            }
            islands.push(Island {
                region: Regions::from_worker_bytes(&island.region).map_err(|e| e.to_string())?,
                mesh: island.mesh,
                contacts: island.contacts,
            });
        }
        zones.push(ZoneFill { id: zone.id, net: zone.net, layer: zone.layer, islands, dropped: zone.dropped, clearance_nm: zone.clearance_nm });
    }
    if seen.len() != view.zones().iter().filter(|zone| view.zone_net_state(zone.net) == "active").count() {
        return Err("Fill worker omitted an active zone.".into());
    }
    if wire.tracks.keys().copied().collect::<BTreeSet<_>>() != view.tracks().iter().map(|track| track.id).collect()
        || wire.vias.keys().copied().collect::<BTreeSet<_>>() != view.vias().iter().map(|via| via.id).collect()
    {
        return Err("Invalid fill worker copper addresses.".into());
    }
    let copper = Copper { pads: wire.pads.into_iter().collect(), tracks: wire.tracks, vias: wire.vias };
    if copper.pads.values().chain(copper.tracks.values()).chain(copper.vias.values()).any(|index| *index >= wire.members.len()) {
        return Err("Invalid fill worker copper membership.".into());
    }
    Ok(Fill { board: wire.board, zones, problems: wire.problems, copper, members: wire.members })
}
