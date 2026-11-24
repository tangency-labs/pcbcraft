# Prepared operand failure review

The region cache contains only a prepared kernel body, protected by a mutex and
owned by the immutable derived value. It has no session, project or history
reference and no process-global registry. Clone/equality/worker transport retain
the same analytic profiles; the cache is excluded from serialization and debug
output. Completed Boolean/union bodies are retained only after profile validation.
A single material island reuses its owning value's prepared body.

Each Boolean/mesher starts a fresh kernel and obtains an independent body through
SolidCore's documented `brep_copy` API. Only copying holds the source mutex;
subsequent Boolean or meshing work consumes the owned copy. The same operand can
be used on both sides without holding two locks. Initial preparation uses the
previous material-minus-holes recipe, including balanced unions, rather than a
new clipping or loop-classification algorithm. Empty material returns without
trying to copy a faceless body.

Native regressions exercise concurrent operations, nested material inside a hole,
unchanged source bytes after multiple operations, worker round trips, and an
injected unwind while holding the copy lock. That last case must refuse the
poisoned operand while an independently decoded source continues to work. It is
a simulated interrupted copy, not a reproduced kernel panic. The WASI case covers
nested material and transport; native thread/unwind tests are excluded there.
Existing fill tests retain clearance, edge, thermal, island, short, stale-source,
removal, cancellation and unchanged native-byte/history assertions.

Review traced worker capture/publication and host transport: the cache initializes
inside geometry operations on workers, not in paint or profile serialization.
Existing cancellation checkpoints and queue/source authority remain in place.
Copying is an indivisible kernel call, so cancellation can still wait for it;
this change makes no new cancellation deadline claim. Prepared topology adds
source-owned derived memory; allocation and realistic-board memory growth were
not measured. Timing results concern the supplied demo's headless calculation,
not input-to-visible latency, endurance or a vendor comparison.

All unit/integration tests passed (2,210, with 12 ignored). The first workspace
run's documentation stage could not load shared-cache dependency artifacts;
the documentation-only rerun passed its one executable test. Both logs remain
in the local verification record. Workspace all-target Clippy passed. Platform
builds and browser evidence are recorded separately in this directory's guide.
