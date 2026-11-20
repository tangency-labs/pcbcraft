# Author electrical intent

Record working DC or peak AC voltage, maximum continuous current, allowed temperature rise or an isolation group on an existing persistent net or net class. These records describe what the designer entered; assigning a class does not copy values to a net or resolve precedence. The values do not change track width, vias, routing, DRC or insulation checks.

Use `electrical_intent.inspect` and `electrical_intent.set` through the shared command API, including headless and live MCP. The net-class editor does not expose these fields. Use `project.inspect` or `nets.inspect_classes` to find persistent net addresses, and `net_classes.inspect` for class UUIDs. A net selector is `{"kind":"net","address":{"project":"…","scope":{"kind":"schematic","id":"…"},"net":"…"}}`; use the returned `board` scope on a physical project. A class selector is `{"kind":"net_class","project":"…","class":"…"}`.

Inspect takes `version: 1` and `target`; it reports the authored values and current session revision. Set takes those parameters plus `expected_revision` and `intent`. The version inside `params` identifies this authored-data contract; it is separate from an optional [MCP command API pin](mcp.md#command-api-version), which sits beside `command`. Set replaces all four fields on the addressed record. Omitted or null fields are unset; an empty `intent` object clears them. The target, intent, voltage, reference and each quantity are objects with named fields; positional arrays are refused. The following intent explicitly specifies zero voltage, leaves current unset, and sets a temperature rise and isolation group:

```json
{
  "working_voltage": {
    "magnitude": {"value": "0", "unit": "V"},
    "basis": "dc",
    "reference": {"kind": "earth"}
  },
  "allowed_temperature_rise": {"value": "12.5", "unit": "K"},
  "isolation_group": "Chassis side"
}
```

`maximum_continuous_current` uses `{ "value": "2.5", "unit": "A" }`. Use canonical decimal text without rounding (at most 36 digits, including at most 18 fractional places) and the exact unit `V`, `A` or `K` for the field. DC can be signed. Peak AC (`peak_ac`), current and temperature rise are nonnegative. Isolation group names are trimmed, nonempty, at most 200 characters and contain no controls.

An Earth reference has no other fields; one that still carries an `address` is refused. A voltage reference may instead be `{"kind":"net","address":{…}}`, using the same scoped address as a net target. A newly selected reference must be an existing named active net in the same registry; a net cannot reference itself. Renaming a reference does not retarget it. A reference whose net was later retired or became ambiguous is retained for inspection, not redirected to a replacement. Newly allocated split or merge identities do not inherit net intent.

Set returns the session revision. A successful change is one Undo step. A stale or invalid request changes neither the document nor history. MCP reports a stale revision as `stale_revision`, with `current_revision` in the error details. `dry_run: true` performs the same validation but does not publish; an unchanged edit is a no-op. Intent is saved in [native projects and archives](native-project-storage.md#authored-electrical-intent). The versioned authored command contract does not specify future class or rule precedence.
