# Hierarchical clipboard contract

The engine/MCP clipboard supports one selected root and complete reachable child
subtrees, alongside flat symbols, wire routes, net labels and selected root
hierarchical labels. This extends the portable schematic contract; canvas controls
are a separate slice. It creates independent sheet content under the existing
single-parent model, without introducing repeated instances or changing saved
project formats.

`schematic.copy` and `schematic.cut` retain their required `items` object with
symbol/wire arrays and accept optional `objects`, a bounded unique array of
`sheet-symbol-N` and `label-N` IDs on the explicit source `sheet`. Both arrays in
`items` may be empty for a hierarchy-only selection. Copy includes each selected
body's entire child subtree, internal wiring, interfaces and pinned package
intent, excluding unrelated root documents and siblings. Root wires enter the
payload only when selected; endpoints whose owners are omitted become shared
free endpoints. Copy/Cut return portable text through `Session::execute` and MCP,
including with inspection suppressed.

Flat data stays `pcbcraft-clipboard/1`; a rooted hierarchy/interface payload uses
`pcbcraft-clipboard/2`, with `kind: "schematic"`, an anchor, explicit `root_sheet` UUID, native
`documents` and optional `cut_sheets` metadata. Native document display order
never determines the clipboard root. Native codecs qualify records, definitions,
geometry, endpoint references and topology. Additional scope checks reject board
owners, orphan sheets/components, empty roots and a hierarchy disguised as a
legacy flat payload. The 8 MiB bound applies before JSON parsing. Older clients
reject version 2; source project paths and runtime providers are not required.

Default paste creates fresh child UUIDs and numeric object/component/interface
IDs. Existing names are retained when free, otherwise bounded copy names are
allocated; names never determine ownership. References renumber by default and
preservation rejects collisions. A suffix changes copied net labels, interface
names and power values. Child coordinates remain local; root placement moves
only selected root geometry. Sheet symbols and hierarchical interfaces retain
their model-supported orientation, so nonzero group rotation is refused.

Cut detaches selected bodies while retaining their child documents/content.
Unselected incident routes remain as dangling routes at their old coordinates,
including both sides of a removed interface. Paste with `reuse_sheets: true`
reparents eligible original unparented children from a same-project Cut, retaining
their current content and identities. It rejects foreign, missing, occupied or
cyclic ownership and cannot rename current child nets. An earlier Cut flag grants
no authority to bypass live checks. All modes use explicit destination sheet and
revision guards and publish one validated history step; failures leave the
snapshot/history intact.

`SchematicPastePreview` caches child creation and reference/package remapping
once, with stable transient IDs. Its `at` method transforms only introduced root
geometry and invalidates the appropriate connectivity caches. These snapshots
carry no commit authority: the host submits original portable text through the
paste command again. Board circuit fragments remain flat and reject embedded
hierarchy. PCB export also compares selected terminals' original connectivity
with copied flat islands and carried global names, refusing selections that
would silently lose an internal connection. A complete schematic subtree is the
supported structural transfer path for those cases.

Regression data is built through shared native commands. Tests cover nested
connectivity and pinned mappings, sibling exclusion, independent duplication,
current-content Cut relocation, parent/child dangling routes, port slots,
malformed scope/versions, resource/ref/revision/rotation/ownership refusals,
preview stability and native persistence/undo. No external EDA implementation,
source-derived fixture, asset or dependency is introduced.
