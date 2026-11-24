# Declare a working voltage

Record the working DC or peak AC voltage of an existing persistent net or net class, with the Earth or net it is measured against. The declaration describes what the designer entered: assigning a class does not copy it to a net or resolve precedence, and no ERC, DRC or solver checks or uses it yet, so every report calls it unverified. Requirements for current, temperature rise and isolation are outside this first slice.

## In Rules

**Analyze → Rules → Working voltage** authors and inspects the requirement from the schematic or PCB; see [net classes in the editors](../pcb/net-class-editor.md). The page uses the commands below, so GUI and MCP edits share revision checks, Undo/Redo and native save/reopen.

## Commands

Use `electrical_intent.inspect` and `electrical_intent.set` through the shared command API, including headless and live MCP. Use `project.inspect` or `nets.inspect_classes` to find persistent net addresses, and `net_classes.inspect` for class UUIDs. A net selector is `{"kind":"net","address":{"project":"…","scope":{"kind":"schematic","id":"…"},"net":"…"}}`; use the returned `board` scope on a physical project. A class selector is `{"kind":"net_class","project":"…","class":"…"}`.

Inspect takes `version: 1` and `target`. Set takes those parameters plus `expected_revision` and `intent`. The version inside `params` identifies this authored-data contract; it is separate from an optional [MCP command API pin](../automation/command-api.md#command-api-version), which sits beside `command`. The intent object has one field, `working_voltage`; an omitted or null value, or an empty `intent`, clears the requirement, and any other field is refused. The target, intent, voltage, reference, magnitude, net address and its scope are objects with named fields; positional arrays are refused. Over MCP, a member repeated anywhere in the parameters is refused with `invalid_params`, rather than keeping only its last copy. This intent declares an explicit zero:

```json
{
  "working_voltage": {
    "magnitude": {"value": "0", "unit": "V"},
    "basis": "dc",
    "reference": {"kind": "earth"}
  }
}
```

Use canonical decimal text without rounding (at most 36 digits, including at most 18 fractional places) and the unit `V`. DC can be signed; peak AC (`peak_ac`) is nonnegative. An Earth reference has no other fields; one that still carries an `address` is refused. A reference may instead be `{"kind":"net","address":{…}}`, using the same scoped address as a net target. A newly selected reference must be an existing named active net in the same registry; a net cannot reference itself. Renaming a reference does not retarget it. Newly allocated split or merge identities do not inherit a declaration.

## Requirement status

Inspect returns the session `revision`, the authored `intent` and a `requirement` report, which is null when nothing is declared:

```json
{
  "source": "authored_on_net",
  "verification": "unverified",
  "review": "needs_review",
  "reasons": ["reference_retired"]
}
```

`source` is `authored_on_net` or `authored_on_net_class`. `verification` is always `unverified` in this slice. `review` is `current`, or `needs_review` with a reason for the owning net (`net_…`) or the reference net (`reference_…`):

- `_split`: the net was split after the declaration was last saved, and it kept its identity on one branch while another branch received a new one;
- `_ambiguous`: the net was split and no branch kept its identity;
- `_merged`: the net was merged into another net;
- `_retired`: the net was deleted.

A flagged declaration keeps its original value and reference; it is not redirected to a replacement net. A split that kept both identities is cleared by setting the declaration again after review, even unchanged; that save is one Undo step. For a retired, merged or ambiguous reference, setting a new value with the same retained reference keeps the flag, and choosing a current reference clears it. The declaration on a net that was deleted, merged or left ambiguous can be inspected and cleared, but not given a new value.

Set returns the session revision. A successful change is one Undo step, and Undo restores the earlier report. A stale or invalid request changes neither the document nor history. MCP reports a stale revision as `stale_revision`, with `current_revision` in the error details. `dry_run: true` performs the same validation but does not publish; an unchanged edit is a no-op. Declarations are saved in [native projects and archives](native-project-storage.md#authored-electrical-intent).
