//! Sparse authored electrical values. Ownership and transaction policy stay separate from
//! future rule resolution; neither class membership nor connectivity derives values here.

use super::{NetAddress, State, graph::Graph};
use crate::{CommandError, Project, Session, named_catalog};
use pcbcraft_units::quantity::{Quantity, decimal};
use serde::de::{MapAccess, Visitor, value::MapAccessDeserializer};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{fmt, marker::PhantomData, sync::Arc};
use uuid::Uuid;

/// Replace authored electrical intent on one existing net or net-class record.
pub const SET: &str = "electrical_intent.set";
/// Read only the authored electrical intent, not effective or inherited values.
pub const INSPECT: &str = "electrical_intent.inspect";
/// Revision of the authored-value contract, not a native document format number.
pub(crate) const CONTRACT_VERSION: u8 = 1;

/// Sparse authored values. Nothing here denotes a board default or an effective rule.
/// Saved records are read through `stored`, which accepts named fields only.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntentV1 {
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "present_named")]
    voltage: Option<WorkingVoltage>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::persistence::present")]
    current_amperes: Option<ExactDecimal>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::persistence::present")]
    rise_kelvin: Option<ExactDecimal>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::persistence::present")]
    isolation_group: Option<String>,
}

/// Validated exact decimal; the containing field supplies the dimension.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
struct ExactDecimal(String);

/// A voltage and its reference are one value: neither can be authored without the other.
/// Saved values are read through [`present_named`], which accepts named fields only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkingVoltage {
    magnitude_volts: ExactDecimal,
    basis: VoltageBasis,
    reference: Reference,
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

/// Decoded only through [`named`] or [`named_or_null`]; positional arrays are refused.
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
    reference: ReferenceInput,
}

/// Decoded only through [`named`]; positional arrays are refused. Absent and null fields are unset.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IntentInput {
    #[serde(default, deserialize_with = "named_or_null")]
    working_voltage: Option<VoltageInput>,
    #[serde(default, deserialize_with = "named_or_null")]
    maximum_continuous_current: Option<QuantityInput>,
    #[serde(default, deserialize_with = "named_or_null")]
    allowed_temperature_rise: Option<QuantityInput>,
    isolation_group: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetInput {
    version: u8,
    expected_revision: u64,
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
/// declaration order, which is not part of the command contract or the native format.
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

/// Publish exact decimal inputs with explicit units and nullable sparse fields.
pub(crate) fn intent_schema() -> Value {
    let quantity = |unit| object(json!({"value":{"type":"string","maxLength":38},"unit":{"const":unit}}), &["value", "unit"]);
    let optional = |field| json!({"oneOf":[field,{"type":"null"}]});
    let reference = json!({"oneOf":[
        object(json!({"kind":{"const":"earth"}}), &["kind"]),
        object(json!({"kind":{"const":"net"},"address":net_address_schema()}), &["kind","address"]),
    ]});
    let voltage =
        object(json!({"magnitude":quantity("V"),"basis":{"enum":["dc","peak_ac"]},"reference":reference}), &["magnitude", "basis", "reference"]);
    object(
        json!({"working_voltage":optional(voltage),"maximum_continuous_current":optional(quantity("A")),
        "allowed_temperature_rise":optional(quantity("K")),"isolation_group":{"type":["string","null"],"minLength":1,"maxLength":200}}),
        &[],
    )
}

/// Validate the target shape independently of any current project.
pub(crate) fn validate_target(value: &Value) -> Result<(), &'static str> {
    serde_json::from_value::<Target>(value.clone()).map(|_| ()).map_err(|_| "requires a scoped net or net-class target")
}

/// Validate the sparse input shape. Units, canonical decimals and ownership are checked by the edit.
pub(crate) fn validate_intent(value: &Value) -> Result<(), &'static str> {
    named::<_, IntentInput>(value.clone()).map(|_| ()).map_err(|_| "requires a sparse authored electrical intent object")
}

impl ExactDecimal {
    /// Accept only the field's explicit canonical unit and an exact, in-range decimal spelling.
    /// Current, temperature rise and peak magnitude cannot be negative; DC may be signed.
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
        self.voltage.is_none() && self.current_amperes.is_none() && self.rise_kelvin.is_none() && self.isolation_group.is_none()
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

    /// Validate local values and foreign scopes before creating a candidate snapshot.
    fn checked(input: IntentInput, project: &Project) -> Result<Self, CommandError> {
        let voltage = input
            .working_voltage
            .map(|input| {
                let magnitude_volts = ExactDecimal::checked(input.magnitude, Quantity::Voltage, input.basis == VoltageBasis::Dc)?;
                let reference = match input.reference {
                    ReferenceInput::Earth {} => Reference::Earth {},
                    ReferenceInput::Net { address } => Reference::Net { net: local_net(project, address)? },
                };
                Ok::<_, CommandError>(WorkingVoltage { magnitude_volts, basis: input.basis, reference })
            })
            .transpose()?;
        if let Some(name) = &input.isolation_group {
            named_catalog::validate_name("isolation group", name).map_err(CommandError::InvalidParams)?;
        }
        Ok(Self {
            voltage,
            current_amperes: input.maximum_continuous_current.map(|q| ExactDecimal::checked(q, Quantity::Current, false)).transpose()?,
            rise_kelvin: input.allowed_temperature_rise.map(|q| ExactDecimal::checked(q, Quantity::Temperature, false)).transpose()?,
            isolation_group: input.isolation_group,
        })
    }

    /// Read only authored intent. Null fields are explicitly unset, including on a historical net.
    /// The address of a reference is qualified for the current registry without following lineage.
    fn inspect(&self, project: &Project) -> Value {
        let voltage = self.voltage.as_ref().map(|voltage| {
            let reference = match voltage.reference {
                Reference::Earth {} => json!({"kind": "earth"}),
                Reference::Net { net } => json!({"kind": "net", "address": NetAddress { project: project.id(), scope: project.net_scope(), net }}),
            };
            json!({"magnitude": voltage.magnitude_volts.inspect(Quantity::Voltage), "basis": voltage.basis, "reference": reference})
        });
        json!({
            "working_voltage": voltage,
            "maximum_continuous_current": self.current_amperes.as_ref().map(|q| q.inspect(Quantity::Current)),
            "allowed_temperature_rise": self.rise_kelvin.as_ref().map(|q| q.inspect(Quantity::Temperature)),
            "isolation_group": self.isolation_group,
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
    /// Load the exact authored record. Historical values are available for inspection only.
    fn existing(self, project: &Project, editing: bool) -> Result<&IntentV1, CommandError> {
        match self {
            Self::Net { address } => {
                let record = project
                    .net_registry
                    .record(local_net(project, address)?)
                    .ok_or_else(|| CommandError::InvalidParams("Unknown net identity.".into()))?;
                if editing && !matches!(record.state, State::Active {}) {
                    return Err(CommandError::Unavailable("Only an active net identity can be edited.".into()));
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
        let before = request.target.existing(project, true)?;
        let intent = IntentV1::checked(request.intent, project)?;
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
        let intent = request.target.existing(project, false)?;
        Ok(json!({"version": CONTRACT_VERSION, "revision": self.revision(), "intent": intent.inspect(project)}))
    }
}

/// Decode only a nonempty canonical payload with named fields. Older native envelopes
/// additionally refuse its presence, regardless of whether a newer reader knows its vocabulary.
pub(crate) fn stored<'de, D: serde::Deserializer<'de>>(de: D) -> Result<IntentV1, D::Error> {
    let intent: IntentV1 = named(de)?;
    if intent.is_empty()
        || intent.voltage.as_ref().is_some_and(|v| !v.magnitude_volts.valid(v.basis == VoltageBasis::Dc))
        || intent.current_amperes.as_ref().is_some_and(|v| !v.valid(false))
        || intent.rise_kelvin.as_ref().is_some_and(|v| !v.valid(false))
    {
        return Err(serde::de::Error::custom("Saved electrical intent must be nonempty and contain canonical values."));
    }
    if let Some(group) = &intent.isolation_group {
        named_catalog::validate_name("isolation group", group).map_err(serde::de::Error::custom)?;
    }
    Ok(intent)
}

#[cfg(test)]
mod tests;
