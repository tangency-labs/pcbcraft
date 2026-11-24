## Unroute one connection

`pcb.unroute_connection` takes two pads on the same intended net. Choose `from`
on the arm to remove and `to` on the side to keep. For a T-shaped route, this
removes the chosen arm and leaves the trunk serving the other pads, including
when the arm meets the interior of a track. Floating tracks and automatic vias
severed by the cut are removed together, unless a locked or explicitly assigned
via anchors the group. An explicitly assigned via on the arm itself is kept
with the copper beyond it; the cut stops before it. Already isolated copper is
unchanged.

Use `dry_run` to preview the removal and its effect on ratsnest edges from
authored copper without making an edit. After a commit, zone fill can reconnect
those pads. Applying the removal is one Undo step and changes no storage format. If a locked object would have
to be removed, or there is no exclusive arm that safely separates the pads,
the command refuses the whole request. It also refuses when the pad to detach
overlaps another pad, or when overlapping pads elsewhere would keep the two
sides joined. This is a shared command available in
headless and live MCP, not a canvas tool or a net-wide removal.
