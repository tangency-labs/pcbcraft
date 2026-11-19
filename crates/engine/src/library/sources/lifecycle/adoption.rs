//! Project-owned exact pins and policy, adopted atomically through the shared command path.

use super::{Assertion, Disposition, Key, Policy, Reference};
use crate::{Command, CommandError, Param, ParamKind, Session};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

/// Apply a publisher assertion to the open project without updating the definition.
pub const ADOPT: &str = "project.adopt_lifecycle";
/// Inspect offline project lifecycle assessments and policy mappings.
pub const INSPECT: &str = "project.inspect_lifecycle";
/// Read an exact loaded publisher assertion and its content digest.
pub const LOOKUP: &str = "library.lifecycle_assertion";
const MAX_ASSESSMENTS: usize = 128;
const MAX_POLICIES: usize = 32;
const MAX_BYTES: usize = 4 * 1024 * 1024;
const ADDRESS: [Param; 3] = [
    Param { name: "library", required: true, summary: "Publishing library ID", kind: ParamKind::LibrarySource },
    Param { name: "assertion", required: true, summary: "Lifecycle assertion ID", kind: ParamKind::LibrarySource },
    Param { name: "assertion_revision", required: true, summary: "Exact assertion revision", kind: ParamKind::DefinitionRevision },
];
pub(crate) const COMMANDS: [Command; 3] = [
    Command {
        id: ADOPT,
        title: "Adopt lifecycle assertion",
        summary: "Pin an exact publisher assertion and optionally set its scheme's project policy. No library, definition or placement changes.",
        params: &[
            ADDRESS[0],
            ADDRESS[1],
            ADDRESS[2],
            crate::EXPECTED_REVISION,
            Param {
                name: "digest",
                required: true,
                summary: "Exact assertion digest from library.lifecycle_assertion",
                kind: ParamKind::LibraryDigest,
            },
            Param { name: "policy", required: false, summary: "Project policy for the assertion's exact scheme", kind: ParamKind::LifecyclePolicy },
            Param { name: "dry_run", required: false, summary: "Return the proposed change without editing", kind: ParamKind::Flag },
        ],
    },
    Command {
        id: INSPECT,
        title: "Inspect project lifecycle assessments",
        summary: "List exact adopted assertion revisions, policy mappings and dispositions. Works offline without changing the project.",
        params: &[],
    },
    Command {
        id: LOOKUP,
        title: "Inspect lifecycle assertion",
        summary: "Read one exact loaded publisher assertion and its digest. No latest-revision substitution or project change.",
        params: &ADDRESS,
    },
];

/// Bounded project-owned assessments. History shares the immutable payload of each assertion and policy.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Assessments {
    assertions: BTreeMap<Arc<Key>, Arc<Assertion>>,
    policies: BTreeMap<Arc<Reference>, Arc<Policy>>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    assertions: Vec<Arc<Assertion>>,
    policies: Vec<Arc<Policy>>,
}

impl Assessments {
    /// The optional assessment document is only written when a pin exists.
    pub fn is_empty(&self) -> bool {
        self.assertions.is_empty()
    }

    /// Read an exact adopted assertion, without consulting a loaded source or another revision.
    pub fn assertion(&self, key: &Key) -> Option<&Assertion> {
        self.assertions.get(key).map(Arc::as_ref)
    }

    /// Return the adopted disposition, or None for an absent pin, unknown scheme or unknown state.
    pub fn disposition(&self, key: &Key) -> Option<Disposition> {
        let assertion = self.assertion(key)?;
        self.policies.get(&assertion.scheme)?.disposition(&assertion.state)
    }

    /// Return the persisted payload in stable target and scheme order; no source access is needed.
    fn stored(&self) -> Stored {
        Stored { assertions: self.assertions.values().cloned().collect(), policies: self.policies.values().cloned().collect() }
    }

    /// Serialize attached assertion content and mappings in deterministic key order.
    pub(crate) fn encode(&self) -> Result<Vec<u8>, String> {
        let mut bytes = serde_json::to_vec_pretty(&self.stored()).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        if bytes.len() > MAX_BYTES {
            return Err("Project lifecycle data exceeds 4 MiB".into());
        }
        Ok(bytes)
    }

    /// Refuse duplicate targets/addresses, orphaned or duplicate policies and unbounded data before adoption.
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_BYTES {
            return Err("Project lifecycle data exceeds 4 MiB".into());
        }
        let stored: Stored = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        if !(1..=MAX_ASSESSMENTS).contains(&stored.assertions.len()) || stored.policies.len() > MAX_POLICIES {
            return Err("Invalid project lifecycle assessment or policy count".into());
        }
        let mut result = Self::default();
        let mut addresses = std::collections::BTreeSet::new();
        for assertion in stored.assertions {
            assertion.validate()?;
            if !addresses.insert(assertion.address.clone()) || result.assertions.insert(Arc::new(assertion.target.clone()), assertion).is_some() {
                return Err("Duplicate lifecycle assertion address or target".into());
            }
        }
        for policy in stored.policies {
            policy.validate()?;
            if !result.assertions.values().any(|assertion| assertion.scheme == policy.scheme)
                || result.policies.insert(Arc::new(policy.scheme.clone()), policy).is_some()
            {
                return Err("An adopted lifecycle policy is duplicated or has no pinned assertion".into());
            }
        }
        // Check the same bound the writer uses, even when the input was compact.
        result.encode()?;
        Ok(result)
    }

    /// Build a candidate pin and optional mapping. Keep unknown states unknown and fail without touching history.
    fn adopting(&self, assertion: Arc<Assertion>, policy: Option<Policy>) -> Result<Self, String> {
        if self.assertion(&assertion.target).is_none() && self.assertions.len() == MAX_ASSESSMENTS {
            return Err(format!("A project supports up to {MAX_ASSESSMENTS} lifecycle assessments"));
        }
        if self.assertions.values().any(|old| old.address == assertion.address && **old != *assertion) {
            return Err("The same immutable lifecycle assertion address already has different pinned content".into());
        }
        let mut candidate = self.clone();
        let standard = Policy::standard();
        let policy = match policy {
            Some(policy) if policy.scheme != assertion.scheme => return Err("Policy must name the assertion's exact scheme revision".into()),
            Some(policy) => Some(policy),
            None if standard.scheme == assertion.scheme && !self.policies.contains_key(&assertion.scheme) => Some(standard),
            None => None,
        };
        if let Some(policy) = policy {
            policy.validate()?;
            candidate.policies.insert(Arc::new(policy.scheme.clone()), Arc::new(policy));
        }
        if let Some(previous) = candidate.assertions.insert(Arc::new(assertion.target.clone()), assertion)
            && !candidate.assertions.values().any(|pin| pin.scheme == previous.scheme)
        {
            candidate.policies.remove(&previous.scheme);
        }
        if candidate.policies.len() > MAX_POLICIES {
            return Err(format!("A project supports up to {MAX_POLICIES} lifecycle policies"));
        }
        // The validated per-field and record-count bounds fit the encoded 4 MiB limit,
        // including JSON escaping and indentation. The writer also checks that limit.
        Ok(candidate)
    }
}

/// Parse the catalog-validated inputs without substituting a revision or source registration.
fn address(params: &Value) -> Result<Reference, CommandError> {
    let fail = CommandError::InvalidParams;
    Ok(Reference {
        library: params["library"].as_str().ok_or_else(|| fail("Missing publishing library".into()))?.into(),
        id: params["assertion"].as_str().ok_or_else(|| fail("Missing lifecycle assertion ID".into()))?.into(),
        revision: params["assertion_revision"]
            .as_u64()
            .and_then(|r| u32::try_from(r).ok())
            .ok_or_else(|| fail("Invalid assertion revision".into()))?,
    })
}

impl Session {
    /// Decode and adopt through the revision-checked edit path, or return the same proposed change without committing.
    pub(crate) fn adopt_lifecycle(&mut self, params: &Value) -> Result<Value, CommandError> {
        crate::check_params(&COMMANDS[0], params)?;
        self.check_revision(params)?;
        let fail = CommandError::InvalidParams;
        let assertion = self.libraries().lifecycle_assertion(&address(params)?).map_err(fail)?.clone();
        if params["digest"].as_str() != Some(assertion.digest().map_err(fail)?.as_str()) {
            return Err(fail("Lifecycle assertion content changed; inspect the exact revision and retry".into()));
        }
        self.libraries().item(&assertion.target).map_err(fail)?;
        let policy = params.get("policy").map(|value| serde_json::from_value(value.clone())).transpose().map_err(|e| fail(e.to_string()))?;
        let old = self.open_project()?;
        let candidate = old.lifecycle.adopting(assertion.clone(), policy).map_err(fail)?;
        let changed = candidate != *old.lifecycle;
        let policy_changed = candidate.policies != old.lifecycle.policies;
        let dry_run = params["dry_run"] == true;
        let mut report = json!({"target":assertion.target,"before":old.lifecycle.assertion(&assertion.target).map(|old| &old.address),
            "after":assertion.address,"disposition_before":old.lifecycle.disposition(&assertion.target),
            "disposition_after":candidate.disposition(&assertion.target),"changed":changed,"policy_changed":policy_changed,"dry_run":dry_run});
        if changed {
            let mut project = old.clone();
            project.lifecycle = Arc::new(candidate);
            let (prepared, highest) = self.prepare_change(project)?;
            let prepared = self.retain_allocation_watermark(Arc::new(prepared));
            // Refuse this explicit adoption before publishing if it would make the project
            // exceed native storage limits; ordinary Save does not run an adoption check.
            crate::persistence::encode_documents(prepared.clone()).map_err(fail)?;
            if !dry_run {
                self.publish_change(prepared, highest);
            }
        }
        report["revision"] = json!(self.revision());
        Ok(report)
    }

    /// Report adopted publisher content and the project mapping without requiring live sources.
    pub(crate) fn inspect_lifecycle(&self, params: &Value) -> Result<Value, CommandError> {
        crate::check_params(&COMMANDS[1], params)?;
        let project = self.open_project()?;
        let assessments = project
            .lifecycle
            .assertions
            .values()
            .map(|assertion| json!({"assertion":assertion,"disposition":project.lifecycle.disposition(&assertion.target)}))
            .collect::<Vec<_>>();
        Ok(json!({"project":project.id(),"revision":self.revision(),"assessments":assessments,"policies":project.lifecycle.stored().policies}))
    }

    /// Query the exact assertion, returning its full immutable content and selection digest.
    pub(crate) fn lookup_lifecycle(&self, params: &Value) -> Result<Value, CommandError> {
        let assertion = self.libraries().lifecycle_assertion(&address(params)?).map_err(CommandError::InvalidParams)?;
        Ok(json!({"assertion":assertion,"digest":assertion.digest().map_err(CommandError::InvalidParams)?}))
    }
}
