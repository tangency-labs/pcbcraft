# Named nets and power symbols

Power symbols are native library definitions placed as ordinary components.
**Capture → Ground** selects GND directly; **P** opens the Power library in the
Parts Catalog. **Value** names the global rail. GND, VCC, VCC (bar), +3V3, +5V,
+12V, VDD, VSS and GNDA are bundled choices, and custom libraries can declare the
same electrical role without depending on their names or drawings.

Power uses ordinary component properties, fields, library-copy authoring and
G/M/R tools. Dragging Value changes its text position; R over Value rotates its
text, while R over the body rotates the symbol. Double-click Value to edit it
in place. G keeps wire contacts and M leaves existing wiring behind. The
bundled definitions start excluded from BOM, board and position outputs through
editable instance defaults. See [bundled libraries](builtin-symbol-libraries.md).

## Text labels

**L** creates a local text label; **Ctrl+L** (⌘L on macOS) creates a global label.
Set the name, scope and rotation, choose Place and click its electrical anchor.
R rotates a preview and Escape cancels. E opens an existing label's properties;
M picks up the complete label, R rotates it and Delete removes it. A held drag
moves the complete label; cancellation leaves the document unchanged.

A text label or a definition-declared power terminal connects exact coincident
pins and every wire segment through its anchor, so either contact at a crossing
joins all routes there. Wire legs referencing the same electrical endpoint, such
as a junction or symbol terminal, also connect without a named contact; a geometric
crossing alone creates no connection.
Connected label movement retains touching wires, with fixed pins connected by
a wire leg. Local names join only within their sheet, including hierarchical
interfaces. Global labels and definition-declared power terminals join matching
global names across sheets. Names are case-sensitive, so GND and gnd are distinct.
A local label joins that connection when a matching global name exists on its sheet.

The Browser, Net Inspector, Rules chooser and PCB annotations use the same
readable names. A net with authored names uses the UTF-8 bytewise smallest name;
inspection retains all aliases. This presentation choice does not resolve an
electrical conflict or change which labels connect.

## Generated names and copper labels

Unnamed nets with component terminals use **REF.PIN**, such as `U3.OUT` or `R7.2`.
PIN is the trimmed pin name only when it uniquely identifies one electrical
terminal across every unit of the component; duplicate, blank or unavailable
names use the electrical terminal designator. The selected terminal has the
smallest [canonical terminal-anchor key](net-identity.md#canonical-anchor-key-encoding),
independent of reference sorting, geometry and wire traversal. A reference or pin
name edit updates the presentation without changing electrical identity.

Tokens preserve ASCII letters, digits, underscore, hyphen and plus. Other UTF-8
bytes use uppercase percent escapes, including periods inside tokens:
`OUT.A` becomes `OUT%2EA`, giving `U3.OUT%2EA`. Distinct nets with equal primary
names use their full encoded sheet-occurrence path, for example
`Main/Power::U3.OUT`. Explicit names use the canonical named anchor declaring
the primary name; generated names use the selected component terminal. A physical terminal drawn on multiple sheets uses the
lexically smallest encoded path among its appearances. Equal qualified names
append `~` and the shortest unique lowercase SHA-256 prefix, starting at eight
characters and extending by two. The scoped derived address hashed here is the
lowest canonical member-anchor key, which already includes the project and scope;
promotion does not change that display address. Orphan wire groups show
**Unconnected**; missing electrical targets show **Unresolved net**.

Generated names and qualification are read-only snapshot data. They neither
join equal-looking nets nor promote sparse registry records, allocate stable IDs,
change project bytes or supply command authority. UUIDs and typed anchors remain
available through explicit identity fields and Details. Shared headless/live
inspection includes `board.netlist` with its accepted/schematic basis, primary
names, electrical aliases and typed membership; pending schematic names remain
in the schematic netlist.

**Layers → Net names** controls the PCB overlay for one view, with
`view.set_net_names_visibility` providing the same live command. The setting
survives local arrangements without changing document history. Pads retain their
numbers and add a net name when both lines fit. Via names span the full diameter,
centered over the hole with a fine outline around the letters. Sufficiently wide,
long visible tracks repeat upright names along their axis. Narrow copper, short
segments and distant zoom omit text before it would overflow copper or become
unreadable. Hidden copper layers omit their track labels.

An accepted PCB keeps its reviewed name baseline while schematic changes are
pending. Properties and inspection report **Schematic update pending** for net
name or membership differences. Reviewed Update PCB accepts the new names with
connectivity; Cancel changes neither, and Undo/Redo restores the accepted state.
[Baseline format 4](native-project-storage.md) retains generated names across
native save/reopen. Older baselines preserve their stored authored names and
historical reference/designator fallback until an explicit reviewed update
adopts the complete presentation snapshot.

## Shared commands

Use `schematic.place_symbol` with an exact library/item/revision/digest selection
for power definitions. Optional `value` overrides the initial Value. Standard
component field, movement, deletion and history commands apply to these instances.
The definition's `behavior` explicitly identifies the globally named terminal.

`schematic.place_net_label` creates text labels with `name`, `style` (`local` or
`global`), `x_nm`, `y_nm`, optional quarter-turn `rotation`, optional `text` and
`expected_revision`. `schematic.set_net_label` accepts the same properties plus
`id`; `keep_connected: true` retains physical contacts when moving. The GUI uses
connected movement. Optional `auto_heal` controls route cleanup and defaults on.
The `text` layout uses the existing visibility, offset, size, alignment and angle
vocabulary; omitting it preserves an existing appearance. `schematic.delete_net_label`
removes the label and directly attached wires in one undoable transaction.

Supply the sheet UUID when more than one sheet exists. Inspect `net_labels` for
text-label records and endpoints such as `{"net_label":"net-label-7"}`. Power
instances instead have ordinary symbol/component IDs and terminal endpoints.
Both the live editor and headless MCP use these same schemas and transactions.

## Storage and limits

Each sheet supports 256 text labels, with names of 1–200 characters without
surrounding whitespace or control characters. Named labels require at least sheet
format 3. Symbol field layouts select format 4, including the defaults of a bundled
GND symbol on the same sheet; an explicit label text layout selects format 5.
Power uses the normal component budget
and project-pinned native definitions, including offline reopening and Home
previews. The former `power`, `power_bar` and `ground` net-label styles and their
separate editing tools have been removed. Opening an existing split project
migrates these records in memory to pinned revision-1 `power/vcc`, `power/vcc-bar`
and `power/gnd` native symbols. Their net names, positions, quarter-turn rotations,
authored junctions and wire routes survive; directly attached wire endpoints are
redirected to the new native terminals. New component identities are allocated
above the retained watermark through the shared placement command. First Save
persists the upgrade; previews and Open do not rewrite the source. Unaffected
documents retain their bytes. New editing commands still refuse retired styles.
