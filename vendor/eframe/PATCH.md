# Native root window lifetime

This is eframe 0.36.2 from crates.io, upstream egui commit
`49682f8baa058bf49e011035cfbd6e825f88a5ef`, with its MIT and Apache-2.0 notices.
The package manifest retains registry dependency versions; no dependency is fetched
from a moving Git branch. The local changes are in `src/epi.rs`,
`src/native/epi_integration.rs` and `src/native/wgpu_integration.rs`, plus local
license paths in `Cargo.toml`.

PCBcraft uses immediate viewports sharing one application/session. Upstream eframe
ties its root viewport to a native window, and winit cannot hide that window on
Wayland. Keeping an empty root canvas leaves a black window; moving a surviving
editor into that surface closes the wrong native window. The local
`Frame::retire_root_window` request releases the root's window, input adapter and
GPU surface after the frame, while retaining the logical root to drive immediate
children. Child native IDs, surfaces and input streams remain unchanged on every
desktop backend. No backend selection or Wayland fallback is added.

The request is explicit and opt-in. Ordinary eframe apps retain upstream behavior.
It is implemented for WGPU, the renderer PCBcraft uses, and requires an existing
immediate child. With no child it is ignored; normal root Close/CancelClose still
owns application shutdown. The retired root has no raw window handle. Root repaint
requests are scheduled through an existing child's window; a root Visible(true)
command recreates a fresh window for arrangement restoration. Deferred-only and
Glow integrations are outside this patch's contract.

Native tests must check destruction of the original OS window and retention of
the survivor's native ID, not merely that one editor remains visible. The isolated
Linux Wayland and X11 tests also exercise root recreation, replacement of all
children with a single root, cancelled close and shutdown requested from a child. Windows and
macOS use the same code path but require native qualification on those systems.

Keep this dependency patch separate from document and workspace models. Reassess
it when updating eframe, comparing these three files with the pinned upstream
package and retaining the native regression. Remove it when upstream provides an
equivalent native lifetime API. This is a local patch, not an upstream release or
published fork.

The failure-path review checks that no shared-state borrow is held while app or
child callbacks run, stale native IDs stop routing after destruction, and texture
updates are drained when no root surface is painted. Native tests exercise the
rootless redraw/quit path and arrangement restoration with no retained child;
application tests cover modal cancellation, failed saves, outstanding file jobs
and stale view addresses. Resource creation failure retains the native root when
no immediate child exists; the public request documents this precondition.
