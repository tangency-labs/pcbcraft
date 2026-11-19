//! Accepted-connectivity display geometry, independent of the proposed schematic.
//!
//! Each net uses a lexicographically spatially sorted spanning chain. Terminals already
//! joined by copper tracks and vias share an island, and the chain skips pairs whose islands are
//! already joined, so a fully routed net shows no airwires. This bounded approximation is a
//! display aid, not a minimum spanning tree, router, clearance or fabrication-readiness check.

use super::{
    Pad,
    update::SourceCapture,
    view::{BoardTarget, BoardView},
    work::yield_unit,
};
use crate::schematic::Point;
use std::{
    collections::BTreeMap,
    future::Future,
    task::{Context, Poll, Waker},
};

/// A logical terminal qualified by its project and physical board, including retained removals.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalAddress {
    pub board: BoardTarget,
    pub component: u64,
    pub terminal: String,
}

/// One accepted terminal resolved through its actual footprint's pin-to-pad mapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub terminal: TerminalAddress,
    pub pad: Pad,
    pub footprint: String,
    pub position: Point,
}

/// An unavailable accepted terminal has no invented board coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissingReason {
    Unplaced,
    MissingMapping,
    MissingPad,
}

/// An unavailable terminal remains attributable even when its schematic source was removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissingEndpoint {
    pub terminal: TerminalAddress,
    pub reason: MissingReason,
}

/// One edge of a deterministic display tree. `net` is an index within `source`, not a persisted ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Airwire {
    pub net: usize,
    pub from: Endpoint,
    pub to: Endpoint,
}

/// Transient display geometry derived from accepted historical membership and current placement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ratsnest {
    pub board: BoardTarget,
    pub source: SourceCapture,
    pub lines: Vec<Airwire>,
    pub missing: Vec<MissingEndpoint>,
    pub net_count: usize,
    pub terminal_count: usize,
}

impl BoardView<'_> {
    /// Whether this board has accepted historical inputs, independently of its net count.
    pub fn has_accepted_connectivity(self) -> bool {
        self.project().board.baseline.as_ref().is_some_and(|baseline| baseline.project == self.target.project && baseline.board == self.target.board)
    }

    /// Resolve accepted connectivity only. Missing baselines and unsupported circular copper
    /// return errors instead of an apparently complete empty overlay;
    /// pending schematic edits never alter these connections before reviewed acceptance.
    /// Every resolvable net forms a spatial chain with one display edge per copper island
    /// beyond the first: N-1 edges with no tracks, none once copper joins every terminal.
    /// Unplaced or unmapped terminals are reported separately, without phantom endpoints.
    pub fn ratsnest(self) -> Result<Ratsnest, String> {
        self.require_linear_copper()?;
        if !self.has_accepted_connectivity() {
            return Err("board has no accepted connectivity".into());
        }
        let copper = if self.tracks().is_empty() && self.vias().is_empty() { super::track::Copper::default() } else { self.copper()? };
        self.ratsnest_with_copper(&copper)
    }

    /// Reuse completed copper connectivity when checking the same immutable snapshot.
    pub(super) fn ratsnest_with_copper(self, copper: &super::track::Copper) -> Result<Ratsnest, String> {
        let mut work = std::pin::pin!(self.build_ratsnest::<false>(copper));
        match work.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(result) => result,
            Poll::Pending => unreachable!("synchronous ratsnest has no suspension points"),
        }
    }

    /// Share display ordering and membership, yielding during published unroute preparation.
    pub(super) async fn build_ratsnest<const COOPERATIVE: bool>(self, copper: &super::track::Copper) -> Result<Ratsnest, String> {
        self.require_linear_copper()?;
        let board = &self.project().board;
        let baseline = board
            .baseline
            .as_ref()
            .filter(|baseline| baseline.project == self.target.project && baseline.board == self.target.board)
            .ok_or_else(|| "board has no accepted connectivity".to_owned())?;
        let mut result = Ratsnest {
            board: self.target,
            source: baseline.source.clone(),
            lines: Vec::new(),
            missing: Vec::new(),
            net_count: baseline.nets.len(),
            terminal_count: 0,
        };
        for (net, state) in baseline.nets.iter().enumerate() {
            if COOPERATIVE {
                yield_unit().await;
            }
            let mut endpoints = Vec::with_capacity(state.terminals.len());
            let mut ordered = BTreeMap::new();
            result.terminal_count += state.terminals.len();
            for key in &state.terminals {
                if COOPERATIVE {
                    yield_unit().await;
                }
                let terminal = TerminalAddress { board: self.target, component: key.component, terminal: key.terminal.clone() };
                let resolution = (|| {
                    let footprint =
                        board.by_component.get(&key.component).and_then(|index| board.footprints.get(*index)).ok_or(MissingReason::Unplaced)?;
                    let numbers = footprint.pads_for(&key.terminal);
                    if numbers.is_empty() {
                        return Err(MissingReason::MissingMapping);
                    }
                    numbers
                        .into_iter()
                        .map(|number| {
                            let position = footprint.pad_position(number, board).ok_or(MissingReason::MissingPad)?;
                            let pad = Pad { board: Some(self.target.board), footprint: footprint.id, number };
                            Ok(Endpoint { terminal: terminal.clone(), pad, footprint: format!("footprint-{}", footprint.id), position })
                        })
                        .collect::<Result<Vec<_>, _>>()
                })();
                match resolution {
                    Ok(resolved) if COOPERATIVE => {
                        for endpoint in resolved {
                            yield_unit().await;
                            let Point { x_nm, y_nm } = endpoint.position;
                            // The ordinal preserves the synchronous stable sort's tie order.
                            ordered.insert((x_nm, y_nm, endpoint.terminal.component, endpoint.terminal.terminal.clone(), ordered.len()), endpoint);
                        }
                    }
                    Ok(resolved) => endpoints.extend(resolved),
                    Err(reason) => result.missing.push(MissingEndpoint { terminal, reason }),
                }
            }
            if COOPERATIVE {
                for endpoint in ordered.into_values() {
                    yield_unit().await;
                    endpoints.push(endpoint);
                }
            } else {
                endpoints.sort_by(|a, b| {
                    (a.position.x_nm, a.position.y_nm, a.terminal.component, &a.terminal.terminal).cmp(&(
                        b.position.x_nm,
                        b.position.y_nm,
                        b.terminal.component,
                        &b.terminal.terminal,
                    ))
                });
            }
            // Untracked terminals get their own island past every copper island index.
            let mut islands = Vec::with_capacity(endpoints.len());
            for (index, endpoint) in endpoints.iter().enumerate() {
                if COOPERATIVE {
                    yield_unit().await;
                }
                islands.push(
                    copper.pads.get(&(endpoint.footprint.clone(), endpoint.pad.number.as_str().to_owned())).copied().unwrap_or(usize::MAX - index),
                );
            }
            for (index, pair) in endpoints.windows(2).enumerate() {
                if COOPERATIVE {
                    yield_unit().await;
                }
                let (a, b) = (islands[index], islands[index + 1]);
                if a != b {
                    result.lines.push(Airwire { net, from: pair[0].clone(), to: pair[1].clone() });
                    for batch in islands.chunks_mut(32) {
                        if COOPERATIVE {
                            yield_unit().await;
                        }
                        for island in batch {
                            if *island == b {
                                *island = a;
                            }
                        }
                    }
                }
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
pub(crate) mod tests;
