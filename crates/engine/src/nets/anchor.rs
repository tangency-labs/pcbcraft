//! Canonical typed anchor addresses, independent of names, geometry and graph traversal.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use uuid::Uuid;

/// Fixed scope tags from the persistent-net key contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum ScopeKind {
    /// A registered physical board, tag 1.
    Board = 1,
    /// Explicit system connectivity, tag 2; codec-only in this slice.
    System = 2,
    /// Project-owned schematic connectivity, tag 3.
    Schematic = 3,
}

/// A typed owner; schematic/system scopes use the project UUID rather than a fake board.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    /// Owner kind; its tag distinguishes schematic and system addresses.
    pub kind: ScopeKind,
    /// Registered board UUID, or project UUID for the other scope kinds.
    pub id: Uuid,
}

/// Fixed anchor-kind priority and tags, independent of displayed object names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum AnchorKind {
    /// Authored local/global named label, tag 1.
    Label = 1,
    /// Hierarchical interface, tag 2.
    Port = 2,
    /// Physical component terminal shared across symbol-unit appearances, tag 3.
    Terminal = 3,
    /// Authored junction, tag 4; codec-only in this slice.
    Junction = 4,
}

/// A stable typed electrical anchor, with a validated canonical binary key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Anchor {
    /// Fixed priority kind, never inferred from an ID prefix.
    pub kind: AnchorKind,
    /// Project UUID qualifying all local identities.
    pub project: Uuid,
    /// Current or snapshot-qualified electrical owner.
    pub scope: Scope,
    /// Sheet occurrence UUID for local objects; zero for component terminals.
    pub occurrence: Uuid,
    /// Numeric component or local label/port/junction identity.
    pub object: u64,
    /// Exact opaque terminal token; empty for sheet-owned objects.
    pub terminal: String,
}

impl Anchor {
    /// Reject nil owners, noncanonical scope IDs and kind-incompatible field forms.
    /// This validates the key vocabulary; live-object resolution belongs to the project graph.
    pub fn validate_form(&self) -> Result<(), String> {
        if self.project.is_nil()
            || self.scope.id.is_nil()
            || self.object == 0
            || self.object > crate::MAX_OBJECT_ID
            || (self.scope.kind != ScopeKind::Board && self.scope.id != self.project)
        {
            return Err("Invalid anchor project, owner or object identity.".into());
        }
        let terminal = self.kind == AnchorKind::Terminal;
        if terminal != self.occurrence.is_nil() || terminal != !self.terminal.is_empty() || self.terminal.len() > 256 {
            return Err("Anchor kind does not match its occurrence or terminal token.".into());
        }
        Ok(())
    }

    /// Encode validated tags, raw RFC UUID bytes, big-endian IDs/length and exact UTF-8 tokens.
    /// No names, text-form numeric sorting, locale rules or normalization participate.
    pub fn canonical_key(&self) -> Result<Vec<u8>, String> {
        self.validate_form()?;
        let mut bytes = Vec::with_capacity(62 + self.terminal.len());
        bytes.push(self.kind as u8);
        bytes.extend_from_slice(self.project.as_bytes());
        bytes.push(self.scope.kind as u8);
        bytes.extend_from_slice(self.scope.id.as_bytes());
        bytes.extend_from_slice(self.occurrence.as_bytes());
        bytes.extend_from_slice(&self.object.to_be_bytes());
        bytes.extend_from_slice(&(self.terminal.len() as u32).to_be_bytes());
        bytes.extend_from_slice(self.terminal.as_bytes());
        Ok(bytes)
    }
}

impl Ord for Anchor {
    /// Match the canonical byte order, including token length before token bytes.
    fn cmp(&self, other: &Self) -> Ordering {
        self.kind
            .cmp(&other.kind)
            .then_with(|| self.project.cmp(&other.project))
            .then_with(|| self.scope.cmp(&other.scope))
            .then_with(|| self.occurrence.cmp(&other.occurrence))
            .then_with(|| self.object.cmp(&other.object))
            .then_with(|| self.terminal.len().cmp(&other.terminal.len()))
            .then_with(|| self.terminal.as_bytes().cmp(other.terminal.as_bytes()))
    }
}

impl PartialOrd for Anchor {
    /// Use the same total order for partial comparisons and canonical fallback selection.
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Publish the exact typed-key vocabulary alongside ordinary terminal/label command targets.
pub(crate) fn schema() -> serde_json::Value {
    serde_json::json!({"type":"object","properties":{
        "kind":{"type":"string","enum":["label","port","terminal"]},
        "project":{"type":"string","format":"uuid"},
        "scope":{"type":"object","properties":{"kind":{"type":"string","enum":["board","schematic"]},"id":{"type":"string","format":"uuid"}},"required":["kind","id"],"additionalProperties":false},
        "occurrence":{"type":"string","format":"uuid"},"object":{"type":"integer","minimum":1,"maximum":crate::MAX_OBJECT_ID},"terminal":{"type":"string","maxLength":256}
    },"required":["kind","project","scope","occurrence","object","terminal"],"additionalProperties":false})
}
