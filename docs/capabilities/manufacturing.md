# Manufacturing capabilities

## Available

- [Read-only fabrication ZIP review](../editor-guide.md#pcb-layout) in an independent workspace, with
  manifest verification, Gerber layer visibility, plated and non-plated round drill tool counts, and
  basic file-presence and outline checks through shared GUI and live MCP.

- **Manufacture › Fabrication › Outputs** exports Gerber X2 copper, mask, paste, silkscreen and
  outline layers with aperture functions. The deterministic archive contains plated and, when
  present, [non-plated Excellon drill files](../manufacturing/drill-files.md) under portable names.
- The embedded manifest records member byte sizes and SHA-256. Desktop saves one ZIP and the browser
  requests one download. [Fabrication through MCP](../manufacturing/fabrication-mcp.md) returns file
  text, the resolved origin and member integrity metadata.
- A named placement origin can be board zero, the outline's lower-left corner or a fixed point.
- Immutable [assembly profiles](../automation/board-commands.md#assembly-output-profiles) pin
  bottom-side conventions and per-definition rotation and centroid corrections. The commands
  support a read-only correction preview, digest-checked board adoption, retirement and Undo/Redo.
  Project format 36 supports up to 64 profile identities, 128 retained revisions and 4,096 corrections.
  Adoptions require board format 22 or a later compatible format.
- Read-only [`pcb.check_assembly`](../automation/board-commands.md#assembly-orientation-check)
  warns about declared polarized packages with neither an orientation mark nor a stated zero
  orientation. Footprint editor authoring, placement export and design-check waivers are not part
  of this check.

## Remaining scope

- The manifest does not identify a board revision. Third-party archives, slot plotting, release
  comparison and assembly review are outside this slice. Placement file export and a profile
  management UI remain open.

## Implementation notes

PCB Draw tools operate on the active visible layer: copper linework uses the
validated conductive-segment path, while non-copper graphics preserve physical
strokes through saves and clipboard operations. Silkscreen, mask and paste
artwork is included in the matching fabrication layer; documentation-layer
graphics remain native board content. [Gerber X2 copper exports](../editor-guide.md#pcb-layout)
include net and component ownership attributes and identify pins on outer copper.
The Gerber profile plots each closed Board Outline path as a separate loop,
including cutouts. Export omits open outline paths.
For exported pads, Gerber retains C flashes for round pads. R/O flashes require individually quarter-turn library and placement rotations
and nondegenerate resolved axis rectangles; R also requires a midpoint at the integer pad
center, and O requires matching width/height parity for integral capsule endpoints. Other
rectangles retain resolved regions; other obrounds retain approximate round-ended strokes
or equal-sided C flashes. Rounded rectangles that qualify for aperture macros flash with
circular corners; others use polygon regions. If a macro core needs a fill but its outline
could self-intersect and no exact fill is supported, export stops without a file set.
A collapsed tiny rectangle can emit a retraced path with zero area. Mask uses outer-layer pads without process
expansion; paste excludes through-hole pads. Persistent layer/width/via controls stay visible across PCB tabs, with
routing in Layout and alignment in its Modify menu.
