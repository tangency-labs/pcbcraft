# Failure-path review for #302

The separate review followed contact lookup and command/MCP dispatch through
immutable copper preparation, the existing native/browser scheduler, publication,
clearance probes/overlays and both DRC implementations. The change introduces no
worker, queue, transport, saved membership graph or shared mutable cache.

Class intent is projected from current stable registry terminals onto the
snapshot's accepted physical memberships. Equal displayed names cannot combine
nets. Direct pad contacts carry full island memberships to routed copper; pads
retain their own intended membership. Pending schematic edits do not accept a
board update, and an accepted reviewed update retains bindings and physical copper.
The shared #300 resolver handles every per-object hard value and preference;
pair spacing uses its symmetric maximum independently of preferences. Hard values
always have a baseline; only preferences can become null. Policy capture validates
clearance before consumers use the internal fallback accessors. Missing referenced
records/classes fail capture, and DRC emits an incomplete finding rather than a
successful board-default-only pass.

Preparation uses the existing try-lock pad cache and releases its guard within
the projection phase. Contention yields instead of waiting; poisoning returns a
request error. A separate unbounded pad-contact scan found during review was moved
into the existing budgeted IslandBuilder phase. Dropping partial work changes no
model/history. The existing source token checks session identity, opening,
revision and exact snapshot; class/binding changes therefore revoke queries,
checker results, GUI drafts and prepared publication. New contact lookup is at
pickup/click only. Explicit GUI pointer jobs skip duplicate source contact lookup,
although candidate membership and class metadata still add preparation cost.
Graph/policy projection and candidate copying remain phase-local synchronous work;
this is not a hard real-time guarantee.

Tests exercise distinct classified nets, command defaults and explicit choices,
weak-dimension refusal, unaffected nets and preferences, actual via contradictions,
old copper left unchanged, strict contour boundaries and tangency, differing layers,
unknown membership, equal-name conflicting islands, query/overlay generation,
class/binding removal and edits, stale completion/atomic publication, cancellation
at small budgets, pending review preservation, accepted reviewed updates, Undo/Redo,
folder/archive reopen, native and WASI predicates, real X/click input, visible error
routing, draft revocation and live/headless MCP parity. The added performance case
checks class defaults against committed width and exhaustive pair-specific probes.
Native GPU captures cover the reused width/halo presentation in both themes.

By code reading, shared queue capacity, stalled/disconnected-peer policy, shutdown,
file-publication workers and transport deadlines are unchanged. Their existing
regressions remain in the workspace suite. No new daemon/network permission or
worker lifetime is introduced. Native-window/browser rendered latency, large-class
capacity, allocation and overnight Wayland endurance remain unqualified; CPU
reports and headless GPU captures do not establish those boundaries.
