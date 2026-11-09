# Accepted board connections

The PCB canvas can display a ratsnest after the board has an accepted update
baseline. Before the first accepted update, schematic connections do not produce
airwires or a zero-connection status that could imply the board is synchronized.
The Layers card's **Ratsnest** checkbox controls the overlay for that view;
`view.set_ratsnest_visibility` exposes the same presentation change to live MCP.
**Show all** and **Isolate active** preserve that overlay choice while changing physical layers.
Visibility survives local arrangement saves and leaves project bytes, electrical
revision and document Undo unchanged. Existing arrangements show it by default.

`BoardView::ratsnest()` resolves the accepted baseline's component/terminal groups
through the actual placed footprints' pin-to-pad mappings and pinned geometry.
Its endpoint addresses include the project, board, component and terminal, along
with the board-qualified physical pad address and world coordinates. A component
whose schematic source is pending removal keeps its accepted connections until a
reviewed update accepts that removal. Proposed wire, name and package edits do
not change accepted membership before Apply.

For each accepted net, the current display approximation sorts available endpoints
by world X, world Y, component ID and terminal token, then joins consecutive
endpoints into a spanning chain. There are N−1 lines for N available endpoints,
including separate endpoint identities at coincident coordinates. Copper tracks
join pads into islands: a pad and a track connect when they share a copper layer
and the track's end lies on the land or its centerline passes within half its width
of the land center; two tracks on one layer connect when their copper overlaps. A
through via connects, on every layer, to tracks and vias its pad overlaps and to pads
whose land contains its center or whose center lies within its pad. The
chain then skips consecutive endpoints whose islands are already joined, so a net
shows one line per island beyond the first and none once fully routed. Sorting costs
O(T log T) over the accepted terminals rather than constructing every endpoint
pair. This approximation is deterministic and can be refined independently; it
is neither a minimum spanning tree nor a clearance or DRC result. Move previews
keep the committed copper islands until the move is applied.
Airwire counts do not establish fabrication readiness.

Unplaced terminals, missing mappings and unavailable physical pads are reported
separately with their logical addresses. They receive no invented coordinates and
are excluded from lines, so missing endpoints can reduce the displayed line count.
The board status reports both airwires and unavailable endpoints when present.

The app caches the engine result by view input authority, board target and document
revision. Placement, accepted updates and Undo invalidate it; panning, zooming and
visibility changes preserve authored state. Disposable move/rotation previews
transform the affected line endpoints with the same transform as the footprint.
Thin chrome-owned strokes are painted behind pads, independently of physical
copper and technical-layer visibility.
