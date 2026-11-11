# Native root window lifetime

This is eframe 0.37.0 from crates.io, upstream egui commit
`3c6eb8261c3293ad9c7ea1ed58487e038860ebef` (the package's `.cargo_vcs_info.json`),
with its MIT and Apache-2.0 notices. PCBcraft's existing 0.36.2 changes were
compared with the original crates.io package before being carried forward.
The package manifest retains registry dependency versions; no dependency is fetched
from a moving Git branch. The local changes are in `src/epi.rs`,
`src/native/epi_integration.rs`, `src/native/canvas_shortcut.rs`, `src/native/wgpu_integration.rs`,
`src/native/glow_integration.rs` and `src/web/events.rs`, plus local license
paths in `Cargo.toml`.

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
Glow integrations are outside the root-window lifetime contract; the clipboard
completion change below also applies to Glow.

If an app removes its last child while requesting root Close, that child's native
resources remain until the next app pass consumes the cancellable request.
Accepted shutdown releases them without recreating a root. If Close is cancelled
and the app supplies no replacement child, a visible root is recreated so the app
can continue. Root recreation flushes deferred visibility commands immediately;
the startup builder's hidden state must not strand a childless restored root.

Native tests must check destruction of the original OS window and retention of
the survivor's native ID, not merely that one editor remains visible. The native
regression also exercises root recreation, replacement of all
children with a single root, cancelled close, shutdown requested from a child,
and accepting/cancelling root Close when the last child is omitted in that same
frame. Late events for destroyed windows do not count as new windows in the test,
and a deadline bounds event-driver failures. This 0.37.0 rebase passed on an
isolated nested X11 display. Direct Wayland, Windows and macOS require native
qualification for this version.

Keep this dependency patch separate from document and workspace models. Reassess
it when updating eframe, comparing all files listed above with the pinned upstream
package, including the native `RequestPaste` and browser paste-event handlers,
and retaining the native regression. Remove the root-window lifetime changes when
upstream provides an equivalent native lifetime API. This is a local patch, not
an upstream release or published fork.

The failure-path review checks that no shared-state borrow is held while app or
child callbacks run, stale native IDs stop routing after destruction, and texture
updates are drained when no root surface is painted. Native tests exercise the
rootless redraw/quit path, last-driver removal and arrangement restoration with no retained child;
application tests cover modal cancellation, failed saves, outstanding file jobs
and stale view addresses. Resource creation failure retains the native root when
no immediate child exists; the public request documents this precondition.

## Clipboard read completion

The native WGPU and Glow menu `RequestPaste` handlers and browser paste-event
handler also publish `Event::Paste("")` when a clipboard read has no supported
text or image. Upstream 0.37 image paste remains available to widgets; PCBcraft
also consumes an image-only response to its captured canvas text read as an
unsupported completion. PCBcraft uses completion to retire the captured canvas destination;
otherwise an abandoned Paste Options read can affect a later paste. Nonempty
text and ordinary text-widget editing keep the existing event path. No extra
clipboard backend, polling, timeout, thread or platform API is introduced.
Empty/error reads, stale delayed responses and modal text ownership are covered
by application regressions. Reassess this change when updating eframe.

# Native canvas standalone-via shortcut

The local native input integration also preserves Command/Ctrl+Shift+X as a key
event when no widget owns keyboard input. egui-winit 0.37 otherwise converts
it to Cut and drops the key. Pending modifier events take precedence over the
previous frame. Unmapped non-Latin characters fall back to physical X before
Cut conversion, while mapped logical letters retain their layout meaning.
Text widgets and Windows Shift+Delete retain native clipboard handling. Plain V
while routing is unchanged. The shortcut predicate has Windows/macOS modifier,
non-Latin fallback and text-focus regression coverage. This original patch adds
no dependency or external source.
