# Netlist export format

`project.export_netlist` writes the open project's connectivity as one JSON
document, `<project name>-netlist.json`, for other tools to read. The project name is
made portable the same way as for the fabrication files: ASCII letters, digits and `-`
stay, every other character becomes `_`, and a name with nothing else left becomes
`board`. `pcb.export_netlist` writes what one board has accepted, as
`<project name>-<board name>-netlist.json`, with the board name made portable the same
way. The project netlist is format 1, described by
[`schemas/netlist-v1.schema.json`](schemas/netlist-v1.schema.json); the board netlist is
format 2, described by [`schemas/netlist-v2.schema.json`](schemas/netlist-v2.schema.json).
This page explains what the fields mean and which guarantees a reader can rely on.

## Source of the connections

The nets are the schematic's complete electrical connectivity, the same grouping
the Net Inspector shows: wires, net labels, power ports and the sheet hierarchy
across every sheet. Hierarchical labels join a child sheet to its parent only
through the matching sheet pin; global and power names join anywhere. This is
`basis: schematic`, the project export. Board copper is not consulted.

A board export has `basis: accepted`: its components, packages and nets are the
board's [accepted baseline](single-board-update.md), what the board took in at its
last reviewed update. Unapplied connectivity changes stay out of those connections;
findings report the pending update. Reference and canonical value text follow the
live component rather than waiting for that update; typed values may be formatted
differently in the editor. Comparing the two exports in sync still requires applying
the board's projection: excluded parts (including power ports by default) do not
contribute board terminals, and an unnamed schematic net that projects to a single
terminal with no multi-pad mapping is omitted. An unnamed one-pin wire stub is one
example. The board's `package` and `pads` fields describe the accepted mapping. An
unwired terminal mapped to several pads instead becomes a board-only `pad_net`,
moving that terminal from `unconnected` into `nets`. Board copper is not consulted.

## Document

| Field | Meaning |
| --- | --- |
| `format` | `pcbcraft-netlist/1` with `basis: schematic`, `pcbcraft-netlist/2` with `basis: accepted`. The reader refuses any other value and either tag on the other basis. |
| `project` | The project's UUID and display name. |
| `basis` | Which connectivity the nets describe: `schematic` or `accepted`. |
| `board` | Only with `accepted`: the board's UUID, name and the `source` capture (`capture_id`, `content_digest`) stored in its accepted baseline, including a baseline initialized with **Use current state**. `source` is null if there is no accepted baseline. |
| `components` | Every component, ordered by the number in its ID. In an `accepted` netlist, each component the board accepted, including one since deleted from the schematic. |
| `nets` | Every net with at least one part terminal, ordered by its first terminal. |
| `unconnected` | Terminals of placed units that are on no net. In an `accepted` netlist, the accepted terminals on no accepted net. |

Records are JSON objects, not positional arrays. A field that may be null is
still required; PCBcraft's reader refuses it when omitted, and refuses `board`,
`package` or `pads` in a `schematic` netlist. `Netlist::parse` also refuses
repeated non-null net IDs, repeated component IDs, repeated terminal numbers
within a component, and references in `nets` or `unconnected` to unknown or
multiply listed terminals. These relational checks supplement the schemas.
Exports omit empty nets; the reader permits them, and comparison counts them
as net rows.
UUIDs are written in the lowercase 36-character hyphenated form, and the reader
accepts only that hyphenated form.
Integers, such as a definition's or a package's `revision`, are written as plain digits.
The reader accepts integral decimal and exponent spellings too, including `1.0`,
`1e0` and zero as `-0`, within the schema's range of 0 through 4294967295.
Fractions and overflow are refused without rounding, even when a float would round
them to an integer.

### Components

A component is one logical part, however many units it has. `id` is the
component ID commands use (`component-3`); `reference` and `value` are its current
annotation. `kind` is `power` for a power port, whose terminal names the net it
sits on, and `part` for an ordinary part. In an `accepted` netlist, `kind` is
`unknown` when no live definition matches the accepted library, symbol ID,
revision and terminal table. The accepted baseline does not retain that role;
a changed live definition cannot supply it. This match uses the baseline's
available metadata, not a retained symbol asset or digest attestation.
The schematic format permits only `part` and `power`; both document readers and
writers refuse `unknown` outside accepted format 2. `definition` names the pinned
library symbol and its revision.

In a `schematic` netlist, `units` lists each placed unit with its symbol ID, the
sheet it is drawn on and `occurrence`, the sheet occurrence UUIDs from the root
sheet down to that sheet.
A unit that is not placed is absent and reported as an `unplaced_unit` finding.

In a `schematic` netlist, `terminals` is the definition's complete logical terminal
table in native order, with each terminal's name and electrical type. A terminal
drawn on several units, such as a shared supply pin, appears once.

In an `accepted` netlist the components are those the board accepted. `units`
is empty, and `package` names the accepted footprint (`library`, `id`,
`revision`), the placement as `footprint` (`footprint-2`, or null before it is
placed) and `unconnected_pads`, the footprint's pads no terminal uses. Each
terminal adds `pads`, the footprint pads it maps to. `reference` and `value`
follow the live component when present; `value` is canonical text, not the board
view's formatted typed value. When the component's current
symbol is not the accepted revision with the same terminals (the part was
deleted, or its symbol changed and the update is pending), terminals use the
accepted baseline's lexical order with `name` and `electrical_type` null.
Otherwise they follow the current symbol's native terminal order.

### Nets

`id` is the net's persistent UUID from the [net registry](net-registry-storage.md)
and stays the same across edits that keep the net. A net without a persistent
record has `id: null` and an `unregistered_net` finding; the export never
substitutes the Net Inspector's row number, which changes as the design changes.
With `basis: schematic`, `name` is the current Net Inspector display name and
`names` every authored name on the net, sorted. With `basis: accepted`, these are
the display name and sorted authored names captured in the board's accepted
baseline. Older baselines without a stored display name derive it from the
smallest accepted authored name or a historical reference and terminal designator.
Pending schematic renames do not supply accepted net names.

`terminals` lists the joined part terminals as `{part, terminal}`, the same
logical address that commands such as `schematic.set_no_connect` accept. Each
terminal of a placed unit appears in exactly one net or once in `unconnected` in a
`schematic` netlist. Terminals belonging only to unplaced units appear in neither;
a shared terminal drawn on a placed unit is still included. In an `accepted`
netlist, every accepted terminal appears in one accepted net or in `unconnected`,
whether its schematic unit is currently placed or not.

An accepted schematic net keeps the persistent `id` of the current schematic net
with exactly the same board terminals. If there is no matching membership, it has
`id: null` and a `pending_net` finding. A net accepted solely to join pads of one
unwired terminal has `id: null` and a `pad_net` finding even if a pending edit now
wires that terminal. Older baseline formats do not distinguish the two origins for
unnamed one-terminal nets. Those rows have `id: null` and a `net_origin_unknown`
finding rather than borrowing a current UUID.

## Board findings

Besides `pending_net`, `pad_net`, `net_origin_unknown` and `unregistered_net`, a board export reports
`pending_removal` for an accepted component that was deleted or excluded from the
board in the schematic, and one `pending_update` with the counts
`pcb.preview_update` would show (`component_changes`, `removed_nets`,
`added_nets`, `blockers`) whenever the schematic differs from the accepted state.
A board with placed footprints but no accepted baseline has no known accepted
state, and the export fails until the baseline is initialized. When the schematic
cannot be compared with the accepted state at all, the export still succeeds and
reports one `update_unavailable` finding with the reason instead of
`pending_update`.

## Determinism

Components are ordered by the number in their ID, so `component-9` comes before
`component-10`. Units follow their unit ID. Terminals follow the definition's
native table except for the accepted fallback described above; nets follow their
first terminal in that same exported component and terminal order.
Nothing depends on traversal or editing order, and the text is two-space indented
JSON ending in a newline. Exporting an unchanged project, including after save
and reopen, gives identical bytes.

## Format versions

Format 2 widens format 1: it adds `board`, `package`, terminal `pads`, a null
`electrical_type` and the `accepted` basis, and changes nothing format 1 already
has. The writer uses the lowest format that carries the content, so the project
netlist stays format 1 and is byte-identical to what a format 1 writer produces.
The new fields take a new number rather than widening format 1 in place because
every object is closed: a format 1 reader refuses fields it does not know, so a
board netlist tagged format 1 would break it under its own tag. This follows the
other documents PCBcraft exchanges, which step their number for each addition:
the board clipboard became `pcbcraft-clipboard/2` for stroked drawings and `/3`
for locked tracks and vias. PCBcraft's reader reads the tag before anything
else, so a later format is refused by its number, not by its first unknown field.

## Commands

`project.export_netlist` reads the open project and `pcb.export_netlist` one board
(the `board` UUID, optional with a single board); both edit nothing. `execute`
returns the current `revision` and `netlist` with `files` (one entry with `name`
and text `contents`) and `findings`. With `dry_run: true` each file has only its
`name`. `apply` validates the request without changing the project, revision or
undo history.

## Comparing a file with the design

`project.compare_netlist` reads a netlist file back, given as its text in
`netlist` (at most 8 MiB of UTF-8 bytes), and compares it with what the open project exports
now: the project netlist for a `schematic` file, or the accepted netlist of the
board a file with `basis: accepted` names. A board the project does not have is
refused. The command edits nothing; `execute` returns the current `revision` and
`comparison`, and `apply` validates the request without changing the project,
revision or undo history. The byte limit is checked before parsing. The discovery schema's
`maxLength` counts Unicode characters and is only a coarse upper bound; the command also
checks the encoded byte length, so multibyte text can reach the limit sooner.
This is the engine's limit, not an MCP transfer limit. Transport frame limits
include the envelope and JSON escaping; the live editor refuses encoded
comparison replies over 16 MiB.

Terminals are matched by their `{part, terminal}` address, never by net name or
position in a list. Nets pair up by persistent `id` first, then by an identical
terminal set. Remaining pairs are selected greedily by most shared terminals,
then fewest unshared terminals, then design order and file order to break ties.
Persistent IDs let the comparison identify a terminal moved between those nets.
A board net whose export has a null ID, such as a pad-only net, pairs by terminal
membership. Without persistent IDs, ambiguous edits need not identify which net
or terminal originally moved.

| Field | Meaning |
| --- | --- |
| `matches` | True when `differences` is empty. |
| `identical` | True when the file equals the design's export in every field. |
| `expected`, `found` | Counts for the design and the file: `components`, `terminals` in the components' tables, `nets`, `connections` (terminals listed across all nets) and `unconnected`. |
| `differences` | Connectivity differences: project, basis and board first; then, for each design component in order, the component missing, or its missing and re-padded terminals in its terminal order, then its extra terminals, then its moved ones; then components only the file has; then design nets with no counterpart, in design order; then nets only the file has. |
| `notes` | Differences that leave connectivity unchanged. |

Each entry has a `kind` and a `message`, plus `part`, `terminal`, `field`,
`expected` and `found` where they apply. A terminal's place is `"unconnected"`,
`"unlisted"` (on no net and not unconnected, as for a unit that is not placed)
or a net: `{"net": {"id", "name"}}` in `expected` and `{"net": {"id", "index"}}`
in `found`, where `index` is the net's position in the file's `nets`. The
message names the design net the file's net pairs with, if any. The file's
net display names are not copied into each moved-terminal entry; changed or
extra names are reported in `net_renamed` or `net_extra`. Move entries can repeat
fixed-size UUIDs and design-side names. The engine's input limit does not bound
the serialized result size.

The difference kinds are `project` (another project ID), `basis`, `board`
(another board, or a board on one side only), `part_missing`,
`part_extra`, `terminal_missing`, `terminal_extra`, `terminal_moved` (between
nets, `unconnected` or `unlisted`), `pads_changed` (the pad list changed,
including its order), `net_missing`
(a design net with no counterpart in the file) and `net_extra`. The note kinds
are `net_renamed` (`field` is `name` or `names`), `net_id_changed`, `header`
(`project.name`, `board.name` or `board.source`), `part_changed`
(`reference`, `value`, `kind`, `definition`, `units` or `package`) and
`terminal_changed` (`name` or `electrical_type`). A file whose nets were only
renamed or reordered therefore matches, with the renames as notes. Reordering the
component, terminal or net lists alone can leave both result lists empty while
`identical` is false.

## Not yet covered

Board copper connectivity and other file formats are not part of format 1 or format 2.

`project.compare_netlist` always takes the open design as the expected side. Comparing
two files with each other, or a project file with a board file, has no command.

Each sheet appears in the hierarchy once, because the editor refuses repeated sheet
instances, so each unit has one `occurrence` path. When repeated instances arrive, a
unit drawn on a reused sheet will need one entry per occurrence.
