# Netlist export format

`project.export_netlist` writes the open project's connectivity as one JSON
document, `<project name>-netlist.json`, for other tools to read. The machine-readable
contract is [`schemas/netlist-v1.schema.json`](schemas/netlist-v1.schema.json); this
page explains what the fields mean and which guarantees a reader can rely on.

## Source of the connections

The nets are the schematic's complete electrical connectivity, the same grouping
the Net Inspector shows: wires, net labels, power ports and the sheet hierarchy
across every sheet. Hierarchical labels join a child sheet to its parent only
through the matching sheet pin; global and power names join anywhere. `basis` is
always `schematic` in format 1. Board copper is not consulted.

## Document

| Field | Meaning |
| --- | --- |
| `format` | Always `pcbcraft-netlist/1`. Readers refuse other values. |
| `project` | The project's UUID and display name. |
| `basis` | Which connectivity the nets describe; `schematic`. |
| `components` | Every component, ordered by component ID. |
| `nets` | Every net with at least one part terminal, ordered by its first terminal. |
| `unconnected` | Terminals of placed units that are on no net. |

Every object is closed: a reader may reject unknown fields.

### Components

A component is one logical part, however many units it has. `id` is the
component ID commands use (`component-3`); `reference` and `value` are its current
annotation. `kind` is `power` for a power port, whose terminal names the net it
sits on, and `part` otherwise. `definition` names the pinned library symbol and
its revision.

`units` lists each placed unit with its symbol ID, the sheet it is drawn on and
`occurrence`, the sheet occurrence UUIDs from the root sheet down to that sheet.
A unit that is not placed is absent and reported as an `unplaced_unit` finding.

`terminals` is the definition's complete logical terminal table in native order,
with each terminal's name and electrical type. A terminal drawn on several units,
such as a shared supply pin, appears once.

### Nets

`id` is the net's persistent UUID from the [net registry](net-registry-storage.md)
and stays the same across edits that keep the net. A net without a persistent
record has `id: null` and an `unregistered_net` finding; the export never
substitutes the Net Inspector's row number, which changes as the design changes.
`name` is the name the Net Inspector shows and `names` every authored name on the
net, sorted.

`terminals` lists the joined part terminals as `{part, terminal}`, the same
logical address that commands such as `schematic.set_no_connect` accept. Each
part terminal appears in exactly one net or once in `unconnected`; a terminal of an
unplaced unit appears in neither.

## Determinism

Components, units, terminals and nets are ordered by IDs and terminal-table
order, never by traversal or editing order, and the text is two-space indented
JSON ending in a newline. Exporting an unchanged project, including after save
and reopen, gives identical bytes.

## Command

`project.export_netlist` reads the open project and edits nothing. `execute`
returns the current `revision` and `netlist` with `files` (one entry with `name`
and text `contents`) and `findings`. With `dry_run: true` each file has only its
`name`. `apply` validates the request without changing the project, revision or
undo history.

## Not yet covered

Footprint pads per terminal, a board-side netlist after a board update, and other
formats are not part of format 1.
