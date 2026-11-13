# Index/cache/overlay failure review

The review followed captured board/rule validation through the session cache,
layer grouping, indexed enumeration, exact predicates and overlay results. The
existing revision-checked commands retain ownership of authored transactions.

Tests exercise exhaustive candidate completeness at 64 layers and 14,336 objects,
spanning-object storage, fractional bounds, tangency, long diagonals, ordering,
source replacement, Undo/Redo/reopen, cache contention, overlay additions/removals,
suppression of moved originals, invalid inputs, duplicate IDs, cancellation by
drop, and stale overlay generations. Existing native and WASI contour regressions
continue to check represented geometry, net membership and saved-state isolation.

Code inspection additionally covered poisoned caches, simultaneous cache misses,
slot replacement, overlay capacity/generation exhaustion and dependency-builder
preconditions. Cache lookup/publication uses try_lock; construction occurs outside
the guard. A miss may duplicate bounded preparation, rather than waiting on another
builder. Only one geometry/index is retained per session and its source reference
is weak; history snapshots do not accumulate cache entries. Outstanding queries
may retain their own older captured geometry. Layer spans are checked after shared
candidate lookup, and source order is restored before consumption.

The index is immutable during a gesture. Overlay validation and generation checks
precede mutation, rejected changes preserve state, and a replacement overlay has a
new identity. Source/overlay authority is required before reusing results, but it
never authorizes a model mutation. Publication still uses existing command
validation and transaction guards. No worker, socket or file I/O is introduced in
the production query path. Native cold preparation remains synchronous through
this engine API; callers needing responsiveness use the existing captured-job
scheduling boundary. There is no new real-time routing or endurance certification.
