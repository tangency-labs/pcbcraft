# STEP placement composition

This is the `solidcore-ops` source from `solidcore/crates/solidcore-ops` in
[SolidCraft](https://github.com/tangency-labs/solidcraft) at
`73bdb6cdd39efc3899fb54e7217289a630a89efe`, under Apache-2.0. Its source files,
Clippy configuration and ban-list generator are retained, together with the
solidcore `LICENSE` and `NOTICE`. The external kernel test corpus and integration
tests are not copied. PCBcraft's generated STEP fixtures exercise the patched
traversal through the public geometry boundary.

The functional changes are the three matrix products in
`src/step_assembly.rs`, backported from the already-merged
[SolidCraft #17776](https://github.com/tangency-labs/solidcraft/pull/17776),
commit `7dc89ee26e4d5f20c3bb93137fc35fb430e34245`:

- Child occurrences compose `local` before inherited `world`.
- Mapped items compose the map-origin inverse before the target.
- Mapped descendants compose that local transform before inherited `world`.

The file also identifies the local modification and documents the two changed
branching helpers. [The exact source diff](composition.patch) and
[source hashes](source-manifest.json) record the boundary; every other copied
source file is byte-identical to the pin.

The standalone manifest expands the original workspace metadata and lints,
declares Apache-2.0 explicitly, and addresses its three kernel dependencies at
the same original Git revision. Cargo's package patch supplies this one ops
implementation to both geometry and STEP without introducing a second kernel
or moving the toolkit pin. The lockfile changes only the ops package's source.
No modelling algorithm, traversal bound, source reader or UI component changes.

When the workspace moves to a qualified SolidCraft revision containing the
upstream correction (merged at `253c8a136d211c4d8f115249e5a47a3836e4839f`), remove
this package patch after running the geometry qualification. The downstream
fixtures should remain: they independently state the expected nested placement,
nonzero map-origin result and placed-body bounds.
