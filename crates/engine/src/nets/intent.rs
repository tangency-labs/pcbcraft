//! One authored working-voltage requirement per net or net class. Ownership and transaction
//! policy stay separate from future rule resolution; neither class membership nor connectivity
//! derives values here, and no checker verifies them yet.

use super::{NetAddress, Retirement, State, graph::Graph};
use crate::{CommandError, Project, Session};
use pcbcraft_units::quantity::{Quantity, decimal};
use serde::de::{MapAccess, Visitor, value::MapAccessDeserializer};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{fmt, marker::PhantomData, sync::Arc};
use uuid::Uuid;

/// Replace the authored working voltage on one existing net or net-class record.
pub const SET: &str = "electrical_intent.set";
/// Read the authored working voltage with its source, verification and review state, not
/// effective or inherited values.
pub const INSPECT: &str = "electrical_intent.inspect";
/// Revision of the authored-value contract, not a native document format number.
pub const CONTRACT_VERSION: u8 = 1;

/// The authored requirement. Nothing here denotes a board default or an effective rule.
/// Saved records are read through `stored`, which accepts named fields only.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntentV1 {
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "present_named")]
    voltage: Option<WorkingVoltage>,
}

/// Validated exact decimal; the containing field supplies the dimension.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
struct ExactDecimal(String);

/// A voltage and its reference are one value: neither can be authored without the other.
/// Saved values are read through [`present_named`], which accepts named fields only.
///
/// A split can keep the owning or reference net's UUID on one branch, so a surviving active
/// identity does not prove the declaration still describes the same conductors. Each save
/// records how many split branches already named each net in the registry's lineage; a later
/// split adds a branch and flags the declaration for review until it is saved again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkingVoltage {
    magnitude_volts: ExactDecimal,
    basis: VoltageBasis,
    #[serde(deserialize_with = "named")]
    reference: Reference,
    /// Split branches of the owning net known when the declaration was last saved.
    #[serde(default, skip_serializing_if = "is_zero")]
    net_splits_seen: u32,
    /// Split branches of the reference net known when the declaration was last saved.
    #[serde(default, skip_serializing_if = "is_zero")]
    reference_splits_seen: u32,
}

/// Omit an empty split baseline so declarations on unsplit nets keep their saved shape.
fn is_zero(value: &u32) -> bool {
    *value == 0
}

/// Count registry records whose split or merge lineage names this net. Records are never
/// removed and lineage is immutable, so the count only grows as the net is split again.
fn splits_of(project: &Project, net: Uuid) -> u32 {
    let count = project.net_registry.records.iter().filter(|record| record.lineage.iter().any(|source| source.net == Some(net))).count();
    u32::try_from(count).unwrap_or(u32::MAX)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum VoltageBasis {
    Dc,
    PeakAc,
}

/// Store the net identity, not its current label or owner spelling. First-board adoption changes
/// the registry address, not the UUID; an unresolved or retired identity is not silently rebound.
/// Earth is an empty struct variant, so a saved Earth reference with extra fields is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Reference {
    Earth {},
    Net { net: Uuid },
}

#[derive(Clone, Copy, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Target {
    Net { address: NetAddress },
    NetClass { project: Uuid, class: Uuid },
}

/// Earth is an empty struct variant, so a leftover address is refused rather than ignored.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ReferenceInput {
    Earth {},
    Net { address: NetAddress },
}

/// Decoded only through [`named`]; positional arrays are refused.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QuantityInput {
    value: String,
    unit: String,
}

/// Decoded only through [`named_or_null`]; positional arrays are refused.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VoltageInput {
    #[serde(deserialize_with = "named")]
    magnitude: QuantityInput,
    basis: VoltageBasis,
    #[serde(deserialize_with = "named")]
    reference: ReferenceInput,
}

/// Decoded only through [`named`]; positional arrays are refused. An absent or null working
/// voltage is unset; any other field is refused.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IntentInput {
    #[serde(default, deserialize_with = "named_or_null")]
    working_voltage: Option<VoltageInput>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetInput {
    version: u8,
    expected_revision: u64,
    #[serde(deserialize_with = "named")]
    target: Target,
    #[serde(deserialize_with = "named")]
    intent: IntentInput,
    #[serde(default)]
    dry_run: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectInput {
    version: u8,
    #[serde(deserialize_with = "named")]
    target: Target,
}

/// Admits only a map, then applies the record's derived field rules (unknown, duplicate and
/// missing fields) to its entries.
struct NamedFields<T>(PhantomData<T>);

impl<'de, T: Deserialize<'de>> Visitor<'de> for NamedFields<T> {
    type Value = T;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("an object with named fields")
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<T, A::Error> {
        T::deserialize(MapAccessDeserializer::new(map))
    }
}

/// Decode a record from an object only. A positional array is refused rather than read in field
/// declaration order (or, for an internally tagged enum, with its first element as the tag),
/// which is not part of the command contract or the native format.
fn named<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(de: D) -> Result<T, D::Error> {
    de.deserialize_map(NamedFields(PhantomData))
}

/// A record decoded through [`named`], for use inside generic containers such as `Option`.
struct Named<T>(T);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Named<T> {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        named(de).map(Named)
    }
}

/// Decode an optional input record: null means unset, and any other value must be an object.
fn named_or_null<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(de: D) -> Result<Option<T>, D::Error> {
    Option::<Named<T>>::deserialize(de).map(|value| value.map(|Named(value)| value))
}

/// Decode a saved field that is present only when authored; null is refused and the value must be
/// an object.
fn present_named<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(de: D) -> Result<Option<T>, D::Error> {
    named(de).map(Some)
}

/// Describe the closed object shape shared by authoring and inspection targets.
fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}

/// Describe a project-qualified saved net; display names are never authority.
fn net_address_schema() -> Value {
    let uuid = json!({"type":"string","format":"uuid"});
    let scope = object(json!({"kind":{"enum":["schematic","board"]},"id":uuid}), &["kind", "id"]);
    object(json!({"project":uuid,"scope":scope,"net":uuid}), &["project", "scope", "net"])
}

/// Describe selectors for authored UUIDs, not derived nets or rule-resolution selectors.
pub(crate) fn target_schema() -> Value {
    let uuid = json!({"type":"string","format":"uuid"});
    json!({"oneOf":[
        object(json!({"kind":{"const":"net"},"address":net_address_schema()}), &["kind","address"]),
        object(json!({"kind":{"const":"net_class"},"project":uuid,"class":uuid}), &["kind","project","class"]),
    ]})
}

/// Publish the exact decimal working voltage with its explicit unit, basis and reference.
pub(crate) fn intent_schema() -> Value {
    let quantity = |unit| object(json!({"value":{"type":"string","maxLength":38},"unit":{"const":unit}}), &["value", "unit"]);
    let optional = |field| json!({"oneOf":[field,{"type":"null"}]});
    let reference = json!({"oneOf":[
        object(json!({"kind":{"const":"earth"}}), &["kind"]),
        object(json!({"kind":{"const":"net"},"address":net_address_schema()}), &["kind","address"]),
    ]});
    let voltage =
        object(json!({"magnitude":quantity("V"),"basis":{"enum":["dc","peak_ac"]},"reference":reference}), &["magnitude", "basis", "reference"]);
    object(json!({"working_voltage":optional(voltage)}), &[])
}

/// Validate the target shape independently of any current project.
pub(crate) fn validate_target(value: &Value) -> Result<(), &'static str> {
    named::<_, Target>(value.clone()).map(|_| ()).map_err(|_| "requires a scoped net or net-class target")
}

/// Validate the input shape. Units, canonical decimals and ownership are checked by the edit.
pub(crate) fn validate_intent(value: &Value) -> Result<(), &'static str> {
    named::<_, IntentInput>(value.clone()).map(|_| ()).map_err(|_| "requires an object with only an optional working_voltage")
}

impl ExactDecimal {
    /// Accept only the field's explicit canonical unit and an exact, in-range decimal spelling.
    /// A peak magnitude cannot be negative; DC may be signed.
    fn checked(input: QuantityInput, quantity: Quantity, signed: bool) -> Result<Self, CommandError> {
        let value = Self(input.value);
        if input.unit != quantity.unit() || !value.valid(signed) {
            return Err(CommandError::InvalidParams(format!(
                "Expected an exact {}canonical decimal in {}.",
                if signed { "" } else { "nonnegative " },
                quantity.unit()
            )));
        }
        Ok(value)
    }

    /// Check canonical precision and whether this field can hold signed values.
    fn valid(&self, signed: bool) -> bool {
        (signed || !self.0.starts_with('-')) && decimal(&self.0)
    }

    /// Return the exact authored number with its explicit unit; no float conversion or rounding.
    fn inspect(&self, quantity: Quantity) -> Value {
        json!({"value": self.0, "unit": quantity.unit()})
    }
}

impl IntentV1 {
    /// Empty intent selects the old native vocabulary; explicit zero is not empty.
    pub(crate) fn is_empty(&self) -> bool {
        self.voltage.is_none()
    }

    /// Validate saved references using the local registry, retaining historical IDs but refusing
    /// foreign/missing/self references instead of guessing from labels or descendants.
    pub(crate) fn validate_reference(&self, owner: Option<Uuid>, mut contains: impl FnMut(Uuid) -> bool) -> Result<(), String> {
        if let Some(Reference::Net { net }) = self.reference()
            && (net.is_nil() || owner == Some(net) || !contains(net))
        {
            return Err("Authored voltage requires a distinct reference in the local net registry.".into());
        }
        Ok(())
    }

    /// Validate local values and foreign scopes before creating a candidate snapshot. The split
    /// baselines come from the current registry, so saving marks the declaration as reviewed.
    fn checked(input: IntentInput, project: &Project, target: Target) -> Result<Self, CommandError> {
        let voltage = input
            .working_voltage
            .map(|input| {
                let magnitude_volts = ExactDecimal::checked(input.magnitude, Quantity::Voltage, input.basis == VoltageBasis::Dc)?;
                let reference = match input.reference {
                    ReferenceInput::Earth {} => Reference::Earth {},
                    ReferenceInput::Net { address } => Reference::Net { net: local_net(project, address)? },
                };
                let net_splits_seen = match target {
                    Target::Net { address } => splits_of(project, address.net),
                    Target::NetClass { .. } => 0,
                };
                let reference_splits_seen = match reference {
                    Reference::Net { net } => splits_of(project, net),
                    Reference::Earth {} => 0,
                };
                Ok::<_, CommandError>(WorkingVoltage { magnitude_volts, basis: input.basis, reference, net_splits_seen, reference_splits_seen })
            })
            .transpose()?;
        Ok(Self { voltage })
    }

    /// Read only authored intent. Null is explicitly unset, including on a historical net.
    /// The address of a reference is qualified for the current registry without following lineage.
    fn inspect(&self, project: &Project) -> Value {
        let voltage = self.voltage.as_ref().map(|voltage| {
            let reference = match voltage.reference {
                Reference::Earth {} => json!({"kind": "earth"}),
                Reference::Net { net } => json!({"kind": "net", "address": NetAddress { project: project.id(), scope: project.net_scope(), net }}),
            };
            json!({"magnitude": voltage.magnitude_volts.inspect(Quantity::Voltage), "basis": voltage.basis, "reference": reference})
        });
        json!({"working_voltage": voltage})
    }

    /// Report where an authored requirement came from and whether it still describes current
    /// nets. Nothing verifies the value yet. An owner or reference that was split (even when
    /// one branch kept its UUID), merged, left ambiguous or removed keeps the original
    /// declaration and reference but flags it for review; nothing is redirected.
    fn requirement(&self, project: &Project, target: Target) -> Value {
        let Some(voltage) = &self.voltage else { return Value::Null };
        let mut reasons = Vec::new();
        let mut check = |net: Uuid, seen: u32, role: &str| match project.net_registry.record(net).map(|record| &record.state) {
            Some(State::Active {}) if splits_of(project, net) > seen => reasons.push(format!("{role}_split")),
            Some(State::Active {}) => {}
            Some(State::Ambiguous { .. }) => reasons.push(format!("{role}_ambiguous")),
            Some(State::Retired { reason: Retirement::Merged, .. }) => reasons.push(format!("{role}_merged")),
            _ => reasons.push(format!("{role}_retired")),
        };
        let source = match target {
            Target::Net { address } => {
                check(address.net, voltage.net_splits_seen, "net");
                "authored_on_net"
            }
            Target::NetClass { .. } => "authored_on_net_class",
        };
        if let Reference::Net { net } = voltage.reference {
            check(net, voltage.reference_splits_seen, "reference");
        }
        json!({
            "source": source,
            "verification": "unverified",
            "review": if reasons.is_empty() { "current" } else { "needs_review" },
            "reasons": reasons,
        })
    }

    /// A selected reference is not inherited or changed when other intent fields change.
    fn reference(&self) -> Option<Reference> {
        self.voltage.as_ref().map(|voltage| voltage.reference)
    }
}

/// Validate a presented net address against the project's current authority, without attaching it.
fn local_net(project: &Project, address: NetAddress) -> Result<Uuid, CommandError> {
    if address.project != project.id() || address.scope != project.net_scope() || address.net.is_nil() {
        return Err(CommandError::InvalidParams("Unknown or foreign scoped net identity.".into()));
    }
    Ok(address.net)
}

/// Refuse newer or unknown command interpretations rather than applying a partial edit.
fn check_version(version: u8) -> Result<(), CommandError> {
    if version != CONTRACT_VERSION {
        return Err(CommandError::InvalidParams("Unsupported authored electrical intent version.".into()));
    }
    Ok(())
}

impl Target {
    /// Load the exact authored record. A historical net's declaration can be inspected and
    /// cleared, but not replaced: `declaring` refuses a nonempty edit of an inactive identity.
    fn existing(self, project: &Project, declaring: bool) -> Result<&IntentV1, CommandError> {
        match self {
            Self::Net { address } => {
                let record = project
                    .net_registry
                    .record(local_net(project, address)?)
                    .ok_or_else(|| CommandError::InvalidParams("Unknown net identity.".into()))?;
                if declaring && !matches!(record.state, State::Active {}) {
                    return Err(CommandError::Unavailable("A net that no longer exists can only have its declaration cleared.".into()));
                }
                Ok(&record.authored_intent)
            }
            Self::NetClass { project: owner, class } => {
                if owner != project.id() {
                    return Err(CommandError::InvalidParams("Foreign net class owner.".into()));
                }
                let class = project.net_classes.get(class).ok_or_else(|| CommandError::InvalidParams("Unknown net class.".into()))?;
                Ok(&class.authored_intent)
            }
        }
    }

    /// Prepare one sparse replacement on a private snapshot; no session state is touched here.
    fn replace(self, project: &Project, intent: IntentV1) -> Result<Project, CommandError> {
        let mut candidate = project.clone();
        match self {
            Self::Net { address } => {
                let registry = Arc::make_mut(&mut candidate.net_registry);
                let record = registry
                    .records
                    .iter_mut()
                    .find(|record| record.id == address.net)
                    .ok_or_else(|| CommandError::InvalidParams("Unknown net identity.".into()))?;
                record.authored_intent = intent;
                registry.validate_encoded_size().map_err(CommandError::InvalidParams)?;
            }
            Self::NetClass { class, .. } => {
                let catalog = Arc::make_mut(&mut candidate.net_classes);
                let class = catalog
                    .definitions
                    .iter_mut()
                    .find(|entry| entry.id == class)
                    .ok_or_else(|| CommandError::InvalidParams("Unknown net class.".into()))?;
                class.revision = class.revision.checked_add(1).ok_or_else(|| CommandError::Unavailable("Class revision exhausted.".into()))?;
                class.authored_intent = intent;
                catalog.sync_format();
            }
        }
        Ok(candidate)
    }
}

/// A newly selected reference must be a different, current named net. An unchanged historical
/// reference is kept as authored (even after its label is removed); no descendant is guessed.
fn check_new_reference(project: &Project, target: Target, net: Uuid) -> Result<(), CommandError> {
    if matches!(target, Target::Net { address } if address.net == net) {
        return Err(CommandError::InvalidParams("A net cannot be its own voltage reference.".into()));
    }
    let record = project
        .net_registry
        .record(net)
        .filter(|r| matches!(r.state, State::Active {}))
        .ok_or_else(|| CommandError::InvalidParams("Choose an active reference net.".into()))?;
    let graph = Graph::build(project).map_err(CommandError::InvalidParams)?;
    if graph.network(&record.primary).is_none_or(|i| graph.networks[i].names.is_empty()) {
        return Err(CommandError::InvalidParams("Choose a named reference net.".into()));
    }
    Ok(())
}

impl Session {
    /// Replace the addressed authored values as one engine transaction. No-ops and dry runs
    /// preserve the revision, history, observer and project Arc. Dry run validates the same
    /// metadata candidate. The edit cannot change topology or adopt generated net numbers.
    pub(crate) fn set_electrical_intent(&mut self, params: &Value) -> Result<(), CommandError> {
        let request: SetInput = serde_json::from_value(params.clone()).map_err(|error| CommandError::InvalidParams(error.to_string()))?;
        check_version(request.version)?;
        self.check_revision(&json!({"expected_revision": request.expected_revision}))?;
        let project = self.open_project()?;
        let intent = IntentV1::checked(request.intent, project, request.target)?;
        let before = request.target.existing(project, !intent.is_empty())?;
        if before == &intent {
            return Ok(());
        }
        if let Some(Reference::Net { net }) = intent.reference().filter(|reference| Some(*reference) != before.reference()) {
            check_new_reference(project, request.target, net)?;
        }
        let candidate = request.target.replace(project, intent)?;
        if !request.dry_run {
            self.publish_prepared_change(candidate, 0);
        }
        Ok(())
    }

    /// Inspect authored values and the current revision, without resolving class or board policy.
    pub(crate) fn inspect_electrical_intent(&self, params: &Value) -> Result<Value, CommandError> {
        let request: InspectInput = serde_json::from_value(params.clone()).map_err(|error| CommandError::InvalidParams(error.to_string()))?;
        check_version(request.version)?;
        let project = self.open_project()?;
        let mut result = project.electrical_intent(request.target)?;
        result["version"] = json!(CONTRACT_VERSION);
        result["revision"] = json!(self.revision());
        Ok(result)
    }
}

impl Project {
    /// Read one record's authored working voltage and requirement state exactly as
    /// `electrical_intent.inspect` reports them, for in-process editors that hold a snapshot.
    pub fn inspect_electrical_intent(&self, target: &Value) -> Result<Value, CommandError> {
        let target: Target = named(target.clone()).map_err(|error: serde_json::Error| CommandError::InvalidParams(error.to_string()))?;
        self.electrical_intent(target)
    }

    /// Share one read path between the command adapter and in-process editors.
    fn electrical_intent(&self, target: Target) -> Result<Value, CommandError> {
        let intent = target.existing(self, false)?;
        Ok(json!({"intent": intent.inspect(self), "requirement": intent.requirement(self, target)}))
    }
}

/// Decode only a nonempty canonical payload with named fields. Older native envelopes
/// additionally refuse its presence, regardless of whether a newer reader knows its vocabulary.
pub(crate) fn stored<'de, D: serde::Deserializer<'de>>(de: D) -> Result<IntentV1, D::Error> {
    let intent: IntentV1 = named(de)?;
    if intent.is_empty() || intent.voltage.as_ref().is_some_and(|v| !v.magnitude_volts.valid(v.basis == VoltageBasis::Dc)) {
        return Err(serde::de::Error::custom("Saved electrical intent must be nonempty and contain canonical values."));
    }
    Ok(intent)
}

#[cfg(test)]
mod tests;
