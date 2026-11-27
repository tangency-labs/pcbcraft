# Align to a reference footprint

In MCP, `pcb.align_footprints` accepts `reference_footprint`, a placed footprint
ID on the same board. Use `left`, `right`, `top`, `bottom`, `center_x` or
`center_y` to match that footprint's physical bounds. Select at least one other
footprint. The reference stays fixed, even if it is locked or also listed in
`footprints`. A locked footprint other than the reference is named and the whole
edit is refused. This option does not apply to distribution or set pitch.
Attached track ends follow the moving footprints under the same placement
checks, in one undo step.
