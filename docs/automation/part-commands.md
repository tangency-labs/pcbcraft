# Native part definitions and quantities

`library.search` accepts `kind: "part"` or `"category"` alongside symbols and
footprints. `library.resolve_part` takes an exact `library`, `definition` and
`definition_revision`, validates category values and every package map, and
returns native dependency documents. It never fetches or substitutes definitions.
`library.parse_quantity` takes a declared `quantity` and input `value`; for
example, resistance `100 mΩ` produces
`{"type":"quantity","value":"0.1","unit":"Ohm"}`. Both operations are read-only
and work without an open project. [The native contracts](../libraries/part-schema.md)
distinguish part definitions from placed part instances. `schematic.place_part`
pins the full native dependency set; `part.set_selection` edits package, MPN and
typed requirements through the same revision-checked command path as the GUI.

`schematic.place_symbol` accepts an optional `unit` for a new part, while
`schematic.place_unit` adds an unplaced unit to an existing `component-…` part
using its project-pinned definition. Supply `component`, `unit`, `x_nm`, `y_nm`,
`expected_revision` and `sheet` when the project contains multiple sheets.
Schematic inspection reports `available_units` and `display_reference`; the
base `reference` stays shared by the part. The library authoring transaction
accepts `select_unit` and `add_unit` actions. See the
[multi-unit contract](../libraries/symbol-schema.md#multiple-schematic-units).

`schematic.complete_packages` requires `expected_revision`. Set `scope` to
`"sheet"` (include the addressed `sheet` UUID when several sheets exist) or
`"project"` to opt in; omission or `"none"` does nothing. Set `dry_run: true` to
validate the same plan without editing. Both modes return `{revision, changes,
skipped}`, even with `inspect: false`. Each change names `sheet`, `symbol`,
`from_part`, `from_unit`, `to_part`, `to_unit` and `remapped_terminals` (changed
pin numbers as an old-to-new map). A source part in `changes` is retired; its
symbol is retained. Skipped entries name `part` and `reason`. Authored
no-connect markers or selected alternate pin functions report `terminal_intent`.
A read-only session refuses the command with `out_of_scope`, even for a dry run, because the
same command applies the transfer. In the live editor, Preview and Apply prepare
the plan off the editor thread, so the canvas stays responsive; just before
publishing, the editor re-checks the revision, open dialogs, the request deadline
and the connection. If the report would exceed the 16 MiB live reply limit,
Preview and Apply both return `cap_exceeded` and nothing is edited; use `"sheet"`
scope or headless MCP for very large projects. For a live Apply, the editor
checks its request deadline after preparing the transfer and does not publish if
the deadline or an observed disconnect wins. Once publication is authorized, that server
deadline no longer ends the reply wait. A client timeout or transport failure can
still lose the reply to a committed edit; inspect before retrying. See [package
completion](../libraries/symbol-schema.md#complete-loose-packages) for the matching and
electrical restrictions.
