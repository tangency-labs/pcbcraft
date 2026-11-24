# Set-pitch distribution

In MCP, call `pcb.align_footprints` with `distribute_x` or `distribute_y`,
`pitch_nm` (a positive integer number of nanometers), and at least two distinct
placed footprint IDs. For example, use `pitch_nm: 5080000` for a 5.08 mm pitch.
Pitch is measured between footprint origins, not pad centers or physical bounds.
The smallest X (left) or Y (top) origin stays fixed, and the other origins are
spaced in increasing coordinate order. Request order breaks ties. The other
coordinate, rotation and board side do not change. The original span may grow or
shrink. A locked footprint anywhere in the selection refuses the whole request
and is named, even if it would be the fixed anchor. An out-of-range destination,
a new short or a move that would change locked copper also refuses the request.
A successful move and its attached track changes are one undo step; repeating
an accepted pitch that already matches creates no history. In a live editor the
copper checks are prepared before the edit is applied. If the board changes
in the meantime, retry with the current revision.

Omit `pitch_nm` for the existing equal-gap distribution: it requires at least
three footprints, uses physical bounds and preserves the outer footprints and
span. The Modify menu continues to use equal gaps.
