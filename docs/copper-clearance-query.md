# Snapshot copper queries

The engine's `board::geometry::Contour` represents current supported copper as
an integer convex core expanded by a floating radius. Rectangular and rounded
rectangular pads retain their resolved cores; circles, ovals, round-ended tracks
and through vias use point or segment cores. Connectivity, DRC copper separation,
disposable query probes and DRC board-edge distances share this representation.
Native circular arcs remain the separate representation and integration work in
[#259](https://github.com/tangency-labs/pcbcraft/issues/259).

`Session::clearance_query(BoardTarget, clearance_nm)` captures an immutable
project snapshot, explicit board, session identity, opening, revision and effective
clearance. It validates the target and existing coordinate-range rule bounds
without editing project state. `Results::authority.is_current(session)` rejects
results after an edit, Undo/Redo, reopen or snapshot replacement, including a
second session opened on the same `Arc<Project>`. A changed request-local rule
requires a new query; its captured clearance is available on the authority.
Callers must check that authority before adopting a result into a current preview
or proposed edit. The ordinary command's revision and short validation still
apply at commit. No new worker, command, persisted net binding or saved format is
introduced by this engine API.

`objects()` supplies source copper in pad/track/via order with resolved copper
layers, contours, item references and intended membership. A mapped pad retains
its own intended membership; routed copper inherits the sorted membership set of
its connectivity island. Membership comes from the accepted board baseline when
present, otherwise from snapshot electrical connectivity. Empty membership stays
unknown, equal display names never establish identity, and a conflicting island
retains multiple net indices. These indices and object indices are meaningful
only within their captured query and must never become saved bindings.

`pairs()` evaluates a conservative bounds sweep. `exhaustive_pairs()` bypasses
that spatial pruning and provides a small-fixture reference for later indexes.
Both evaluate actual contours on common copper layers. `candidates()` separately
returns a conservative linear obstacle set for a disposable contour on explicit
board copper layers. Extra candidates are permitted; the final gap/contact
predicate decides the result. DRC streams its candidates and skips its existing
same-island and known same-net clearance exemptions before evaluating distances,
so it does not allocate an intermediate all-pairs decision table.

`probe()` replaces one source object when `intended_from` is given, or starts
with unknown membership otherwise. It combines the intended memberships of all
contacted obstacles before evaluating the result, so an initially unnamed probe
bridging two nets still reports conflicting contact at zero required clearance.
A probe retests contacts without assuming its old physical island remains joined.
This single-contour operation does not implement a multi-object routing overlay,
connected-move solver or routing gesture authority; those remain later slices.

Copper contact includes exact tangency (`gap <= 0`), independently of a strict
clearance violation (`gap < clearance`). Equality with required clearance passes.
Containment may return zero rather than penetration depth. Degenerate query
segments represent round points; authored zero-length tracks remain rejected.
Rotated pad cores retain their existing nearest-nanometer projection, while odd
track widths and circle/via diameters retain half-nanometer radii. The existing
floating distance and integer orientation predicates are retained without a new
epsilon, exact-integer rewrite or coordinate restriction. DRC continues to report
conflicting intended nets per physical island independently of clearance, and
its board-edge check distinguishes tangency from interior penetration.

The query regressions compare sweep/probe candidates and decisions with
exhaustive shared-layer pairs on original command-built fixtures. They cover all
supported pad shapes, rotations/reflections, fractional radii, equality and nearby
values, through vias, differing layers, equal-name distinct nets, connected copper,
conflicting islands, unset membership, long diagonals, coordinate limits and stale
source authority. Existing DRC contour and scoped short-rejection regressions
remain part of the same board suite. The tests run natively and as WebAssembly
under WASI; the browser target is checked separately. This does not establish
browser interaction performance or native-window endurance.

To repeat WebAssembly execution from the repository root with Node 22 or later:

```sh
rustup target add wasm32-wasip1
CARGO_TARGET_WASM32_WASIP1_RUNNER="node --no-warnings $PWD/scripts/wasi-test-runner.cjs" \
  cargo test --locked -p pcbcraft-engine --target wasm32-wasip1 --lib board:: -- --test-threads=1
```

CI runs this board suite in addition to the ordinary native suite and packaged
browser build. WASI exercises the engine predicates and query behavior; it does
not substitute for the browser host or WebGL/WebGPU verification.
