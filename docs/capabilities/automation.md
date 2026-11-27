# Automation capabilities

## Automation

### Available

- [Read-only headless MCP sessions](../mcp.md) and the headless command catalog with a command API
  version and a catalog revision in discovery, and a published error code on failed project commands
  and live requests.
- The desktop live connection runs every project command except headless-only ones (the KiCad
  import) with shared undo, plus window commands for
  schematic and PCB views, workspaces, footprint selection, named arrangements under File → Layouts,
  windows, panels, preferences, Home, in-place Save, Save As/Open to an explicit path without the
  picker, and `inspect_editor` with file location and job results.
- Opt-in Assistant chat with installed ACP agents (Claude Code, Codex and others) or
  OpenAI-/Anthropic-compatible model endpoints, running the same commands with SolidCraft's
  permission modes and approvals.
- Endpoint keys in the system keychain.
- Read-only `project.export_netlist` writes the schematic's components, logical terminals and nets
  across all sheets as a deterministic [JSON netlist](../formats/netlist-format.md), `project.compare_netlist`
  reads an export back to compare counts and connectivity, with notes for annotation or net-metadata
  changes, and `pcb.export_netlist` writes one board's accepted components, pads and nets as JSON
  format 2.

### Remaining scope

- Headless sessions are separate from the open GUI.

## Implementation notes

The engine's [object-query parser](../design/object-queries.md) provides bounded boolean
syntax, text selectors and exact numeric property comparisons with byte-offset
diagnostics. It is a Rust API only; query evaluation, object selection, paging,
area predicates and search UI are not implemented by this parser.

Native live MCP also provides [named UI targets and queued input](../automation/ui-automation.md)
for demonstrations and interaction automation. Common chrome adapters and project
canvases publish targets; specialized widgets may still require coordinates.
`ui.project` turns a model point in a schematic, PCB or definition view into
window coordinates for a click. `ui.resize` requests a reproducible native window
size before a recording, and `ui.window` reports the measured result.
Pointer, key, text and scroll operations use normal UI handling and report
completion or cancellation through a bounded ticket. Input supports the original
native window, its split panes and detached editor windows. This does not record
video.
