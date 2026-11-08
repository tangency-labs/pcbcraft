# Named nets and power symbols

Use **L** for a local label or **Ctrl+L** (⌘L on macOS) for a global label.
Their form has a net name, scope and rotation. **Capture → Ground** starts placing
GND directly, with no catalog or property form. **P** opens
the **Parts Catalog** with the Power collection selected. Choose a drawing such
as GND, VCC, VCC (bar), +3V3, +5V, +12V, VDD, VSS or GNDA; its preview has an editable net
name, so supplies do not depend on a fixed list of rail names.

**VCC** keeps the arrow-shaped supply drawing, while **VCC (bar)** uses a flat
horizontal bar. Both default to the electrical name `VCC`; the descriptive
catalog name does not create a separate net. Each drawing retains its style when
its net name is edited.

Choose **Place**, then click to place the anchor. **R** turns its preview
counterclockwise in quarter turns, and **Esc** or right-click cancels. Hover a
placed marker and use **E** for properties, **M** to pick it up, **R** to rotate or
**Delete** to remove it. Drag its drawing or name to move it directly, releasing
to finish; Escape or releasing outside the canvas cancels the preview. Its context menu offers the same actions; **Apply**
confirms property changes. Each completed placement, move, property change or
deletion is one shared undo step. Cancelled previews and forms leave project data
unchanged; stale forms cannot replace newer work.

The catalog's **Repeat placement** keeps new markers active after each placement,
with fresh IDs and the same name/rotation until **Esc**. Moving an existing marker
remains one edit regardless of that setting.

A marker connects where its small anchor meets a pin or wire exactly. A physically
connected anchor omits the open-pin ring and a contacted wire end's open square;
these indicators return when that physical contact is released. It can
also stand alone and serve as the endpoint of a later wire. Drawing with **W**
uses that anchor like an ordinary pin. A label placed at a crossing connects the
routes it contacts; crossing wires elsewhere still remain separate. Moving a
marker away from a contacted pin or wire releases that geometric contact, while
wires explicitly drawn to its endpoint follow the marker's identity.

Names are case-sensitive: GND and gnd are distinct. Local names connect only
within their sheet, including matching hierarchical labels. Global labels,
power and ground connect matching global names anywhere in the project; a local
label shares that connection when a matching global name appears on its sheet.
A ground drawing with the name GNDA therefore joins GNDA, not GND. Changing a
power symbol's name changes its electrical connection, without changing any
component value or creating a PCB footprint.

The Browser and Net Inspector show named groups instead of Net 1, and PCB pad
annotations use the same names. Inspection retains every alias when a wire joins
several named nets. Global names take display priority, with lexical ordering
within each scope; this ordering does not imply that joining different supply
names is electrically correct. ERC and supply-conflict diagnostics are separate
work. Unnamed groups keep their existing view-local numbering.

## Shared commands

`schematic.place_net_label` takes `name`, `style`, `x_nm`, `y_nm`, optional
`rotation` and `expected_revision`. `style` is `local`, `global`, `power`,
`power_bar` or `ground`; rotation defaults to zero and accepts 0, 90, 180 or 270 degrees.
`schematic.set_net_label` takes the same properties plus the stable `id` from
inspection. `schematic.delete_net_label` takes `id` and `expected_revision`.
Supply the `sheet` UUID when more than one sheet exists. The live editor and
headless MCP expose the same command schemas and transactions as the GUI.

Inspect `project.schematic.net_labels` or the addressed sheet's schematic to
find records and endpoint objects such as `{"net_label":"net-label-7"}`.
Use that endpoint with `schematic.draw_wire`. Project `netlist` resolves all
sheets; sheet netlists retain only that sheet's endpoints after resolving remote
connections. Named groups include `name` and a `names` array of all aliases.
Deleting a label removes wires explicitly attached to its endpoint, while
unrelated contacted wires remain. Undo restores the complete edit.

## Storage and limits

Each sheet supports 256 named anchors and names up to 200 characters, without
surrounding whitespace or control characters. Folder and `.pc.zip` saves retain
names, scope, placement, rotation and endpoint identities in
[sheet format 3](native-project-storage.md#named-net-records). Existing unnamed
projects retain their previous sheet encodings; legacy single-file export
refuses named anchors rather than silently discarding them.

Custom power drawings, library-authored power-symbol definitions, electrical
rules checks, configurable label fields/formatting, inter-sheet reference lists
and box/group editing of labels remain outside this slice.
