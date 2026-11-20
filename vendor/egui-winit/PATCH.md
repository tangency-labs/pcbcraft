# Clipboard paste chord completion

This is egui-winit 0.37.0 from crates.io, upstream egui commit
`7c0b40402271e8d40cda69f87cf3c2617c2e4382` (the package's `.cargo_vcs_info.json`),
with its MIT and Apache-2.0 notices (LICENSE-APACHE / LICENSE-MIT, carried over —
the published package references them from the repository root). The package
manifest retains registry dependency versions; no dependency is fetched from a
moving Git branch. The local changes are in `src/lib.rs` and `src/clipboard.rs`,
plus this file.

## What changed and why

Upstream translates an ordinary Cmd/Ctrl+V press into `Event::Paste` only when
the OS clipboard read returns non-empty text, into `Event::PasteImage` when it
returns an image — and into **nothing at all** when the read comes back empty or
unsupported (the `is_paste_command` branch returns early with no event). The raw
`Event::Key` is swallowed too, so application-level paste handlers (PCBcraft's
library editor canvases, issue #417) can never see the chord on an empty or
unsupported clipboard.

The vendored branch now emits `Event::Paste("")` in that last case: the chord is
a *completed clipboard read* even when the read yields no usable payload. This
mirrors the local eframe patch that publishes `Event::Paste("")` for empty
menu-triggered `RequestPaste` reads (`vendor/eframe/PATCH.md`, "Clipboard read
completion"). `Event::Paste("")` is a no-op for `egui::TextEdit` (empty inserts
are ignored), so native text-widget editing is unchanged; text and image reads
keep the upstream `Event::Paste(text)` / `Event::PasteImage` events.

## Testing seams

winit's `KeyEvent` cannot be constructed outside the winit crate
(`platform_specific` is `pub(crate)`), which makes the translation boundary
unreachable for tests. Three `#[doc(hidden)]` methods on `State` close that gap
without changing behaviour:

- `translate_keyboard_input_for_testing(physical_key, logical_key, text, state)`
  runs the same body as `on_keyboard_input` (extracted, unchanged, as
  `translate_keyboard_input`).
- `set_modifiers_for_testing(modifiers)` sets the state normally updated by
  `WindowEvent::ModifiersChanged`.
- `set_test_clipboard(text, image)` seeds `Clipboard.test_content`, consulted
  ahead of the real backends by `get()`/`get_image()`: `Text`/`Image`/`Empty`
  payloads reproduce the three OS clipboard states deterministically.

`clipboard_translation_tests` in `src/lib.rs` covers all three clipboard states
plus the copy chord and the shifted-chord raw-key path. Application tests drive
the same seams to feed real translated events into the editor harness; they do
not need a `Window`, an event loop or an OS clipboard.

## Maintenance

Keep this dependency patch separate from document and workspace models. Reassess
it when updating egui-winit, comparing `src/lib.rs` and `src/clipboard.rs` with
the pinned upstream package and retaining the regression tests. Remove the
emission change if upstream publishes a completion event for empty paste reads
itself. This is a local patch, not an upstream release or published fork.
