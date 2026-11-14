# Issue 301 failure-path review

Scope: original PCBcraft engine/shared-command changes on travle1/net-class-assignments, based on merged #300 at 3ba1e4120b66d4e21ec01adad1f8cee1adada722. This review adds no workers, transport, runtime queues or blocking I/O. Existing native storage publication and immutable background-source authority are reused.

## Exercised paths

- Shared engine commands reject stale revisions, unknown classes, foreign owners, retired/ambiguous targets and bare pins without changing the current snapshot. Empty derived assignments and identical class sets retain pointer identity/history. Promotion and assignment Undo together.
- Topology tests cover bare-pin/stub continuation, primary/fallback loss, all eleven dimensional properties on meaningful splits, compatible/conflicting merges, retained origins, ambiguity with later branch edits, and single-use child sheets. Native folder/packed reopen and exact Undo/Redo preserve decisions.
- Codec tests refuse bindings under old project/instance headers, absent binding vocabulary under the new header, null/empty/duplicate/unknown bindings, missing class IDs and dangling origin IDs. Existing registry-limit regressions still cover atomic exhaustion.
- Class removal refuses retained references; explicit replacement rewrites current and historical references in one transaction. Undo restores the definition and origins.
- First-board creation requalifies live anchors while retaining registry/net/origin IDs. Assignments preserve physical board bytes and accepted baseline; invalid assignment preserves a pending update, and later valid edits/reopen can create a fresh update review.
- Live/headless parity tests exercise discovery before promotion, returned query data with inspect:false, one-step assignment/promotion, modal refusal, stale refusal and shared Undo.
- Rule-dependent Session DRC (sync/job), clearance query and copper preparation reject required dimensional bindings. Direct BoardView DRC returns an explicit unsupported finding. Class-aware evaluation remains #302.
- Storage interruption tests exercise every existing forward/inverse journal checkpoint for the first class assignment in schematic/board homes and first-board transfer; recovered state is exactly the before or after document set, with physical board bytes retained.

## Reviewed by reading

- nets.set_classes validates input and constructs a private Project/Registry candidate. Promotion allocation and complete-registry validation occur before Session.change publishes anything. Entropy failure, record/binding/serialized-size limits and reconciliation errors return through the shared command error without revision/history/update mutation.
- Session.change reconciles the complete candidate before replacing the current Arc, pushing history, clearing redo or invalidating update authority. Sheet-local transactions defer reconciliation until the complete project is available. No shared lock or external peer is introduced by assignment bookkeeping.
- Reconciliation uses frozen derived before/after graphs; origin union is canonical and bounded, and unknown inherited class/property policy refuses instead of discarding intent. Historical source records remain reserved, so origins are checked in the same owner registry. There is no persisted membership index or name lookup.
- Decode validates class vocabulary before installing instance records, allowing binding checks to resolve the exact catalog. Project/21 and board instances/3 must agree and require actual bindings; untouched no-binding projects retain prior encodings. Encoding cannot silently downgrade historical assignments.
- Class updates/removal validate the resulting catalog/registry before publication. Rebind sorts/deduplicates pairs while preserving origin IDs. Historical references deliberately prevent definition deletion unless a replacement is supplied.
- Existing copper/checker source tokens compare session identity, opening, revision, board and immutable snapshot; assignment/class edits invalidate old prepared results under those existing checks. Current required dimensions are refused before dependent work. Cancellation/disconnect/drop resource behavior is unchanged. Deleting tracks is allowed; empty-rule classifications require no unsupported dimensions.
- General project inspection remains the existing raw net-registry projection. Only explicit per-target nets.inspect_classes calls invoke the class resolver. Definition-only inspection does not build connectivity. Naming is not classification.

## Limits

No UI controls, class-aware routing/checking, repeated-use circuits or multi-board ownership transfer are supplied; these remain #302/#303/#307. No app was launched. The scoped CPU fixture has 64 logical terminals and four promoted unnamed nets, with two class definitions; it is not a named-net rendering, large-project, allocation, compositor or endurance qualification. Full Wayland endurance remains unresolved in the workspace record.

Final post-check review confirmed the candidate/publication, rebind, source-token and journal paths above against commit 7ac9df2c4ef726b62640df40d059184e9814684e. All required checks have passed; there are no unresolved correctness failures.
