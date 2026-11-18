//! Authored electrical intent, independent of derived rules. The command prototype is linked
//! only into engine tests until the native codec can preserve every accepted edit.

use super::{NetAddress, State, graph::Graph};
use crate::{CommandError, Project, Session, named_catalog};
use pcbcraft_units::quantity::{Quantity, decimal};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

pub(crate) const SET: &str = "electrical_intent.set";
pub(crate) const INSPECT: &str = "electrical_intent.inspect";
/// Revision of the authored-value contract, not a native document format number.
const CONTRACT_VERSION: u8 = 1;

/// Sparse authored values. Nothing here denotes a board default or an effective rule.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct IntentV1 {
    voltage: Option<WorkingVoltage>,
    current_amperes: Option<ExactDecimal>,
    rise_kelvin: Option<ExactDecimal>,
    isolation_group: Option<String>,
}

/// Validated exact decimal; the containing field supplies the dimension.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ExactDecimal(String);

/// A voltage and its reference are one value: neither can be authored without the other.
#[derive(Clone, Debug, PartialEq, Eq)]
struct WorkingVoltage {
    magnitude_volts: ExactDecimal,
    basis: VoltageBasis,
    reference: Reference,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum VoltageBasis {
    Dc,
    PeakAc,
}

/// Store the net identity, not its current label or owner spelling. First-board adoption changes
/// the registry address, not the UUID; an unresolved or retired identity is not silently rebound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reference {
    Earth,
    Net(Uuid),
}

#[derive(Clone, Copy, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Target {
    Net { address: NetAddress },
    NetClass { project: Uuid, class: Uuid },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ReferenceInput {
    Earth,
    Net { address: NetAddress },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QuantityInput {
    value: String,
    unit: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VoltageInput {
    magnitude: QuantityInput,
    basis: VoltageBasis,
    reference: ReferenceInput,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IntentInput {
    working_voltage: Option<VoltageInput>,
    maximum_continuous_current: Option<QuantityInput>,
    allowed_temperature_rise: Option<QuantityInput>,
    isolation_group: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetInput {
    version: u8,
    expected_revision: u64,
    target: Target,
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

impl ExactDecimal {
    /// Accept only the field's explicit canonical unit and an exact, in-range decimal spelling.
    /// Current, temperature rise and peak magnitude cannot be negative; DC may be signed.
    fn checked(input: QuantityInput, quantity: Quantity, signed: bool) -> Result<Self, CommandError> {
        if input.unit != quantity.unit() || !decimal(&input.value) || (!signed && input.value.starts_with('-')) {
            return Err(CommandError::InvalidParams(format!(
                "Expected an exact {}canonical decimal in {}.",
                if signed { "" } else { "nonnegative " },
                quantity.unit()
            )));
        }
        Ok(Self(input.value))
    }

    /// Return the exact authored number with its explicit unit; no float conversion or rounding.
    fn inspect(&self, quantity: Quantity) -> Value {
        json!({"value": self.0, "unit": quantity.unit()})
    }
}

impl IntentV1 {
    /// Validate local values and foreign scopes before creating a candidate snapshot.
    fn checked(input: IntentInput, project: &Project) -> Result<Self, CommandError> {
        let voltage = input
            .working_voltage
            .map(|input| {
                let magnitude_volts = ExactDecimal::checked(input.magnitude, Quantity::Voltage, input.basis == VoltageBasis::Dc)?;
                let reference = match input.reference {
                    ReferenceInput::Earth => Reference::Earth,
                    ReferenceInput::Net { address } => Reference::Net(local_net(project, address)?),
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
                Reference::Earth => json!({"kind": "earth"}),
                Reference::Net(net) => json!({"kind": "net", "address": NetAddress { project: project.id(), scope: project.net_scope(), net }}),
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
    /// preserve the revision, history, observer and project Arc. Dry run uses shared preparation.
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
        if let Some(Reference::Net(net)) = intent.reference().filter(|reference| Some(*reference) != before.reference()) {
            check_new_reference(project, request.target, net)?;
        }
        let candidate = request.target.replace(project, intent)?;
        if request.dry_run {
            self.prepare_change(candidate)?;
            return Ok(());
        }
        self.change(candidate)
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

mod tests;
