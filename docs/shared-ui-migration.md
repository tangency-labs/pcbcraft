# Shared Craft UI migration

This local branch starts adoption of SolidCraft's extracted interface crates at
`f56114219b2d7f6d16de8dd58700479646948f8a`. It is based on PCBcraft
`16e7bab`. The first slice establishes dependencies, fonts and toolkit
compatibility. The shared kit also draws menus and Preferences choices through
`pcbcraft-chrome`.

## Dependency boundary

The workspace pins `solidcraft-ui-kit`, `solidcraft-theme` and `solidcraft-look`
to the same Git revision. Consuming crates opt in with workspace dependencies.
The application/session, electronics engine, document commands, saved project
formats and window/view ownership stay in PCBcraft. The shared universal panel
runtime in SolidCraft's application/engine is not part of this migration.

The current kit's blur support also reaches the shared viewport renderer and
SolidCore's record/tessellation libraries, including the independent SolidCore
kernel. Those are ordinary library dependencies; SolidCraft's executable,
document orchestration and mechanical application engine are absent from this
dependency graph. This is a larger dependency closure than the form controls
alone. A future upstream feature split could make renderer-dependent effects
optional, but this branch uses the existing published crate boundary.

Cargo fetches the source repository and builds the selected dependency graph.
It does not make SolidCraft a member of PCBcraft's workspace. The consumer's root
manifest explicitly selects the pinned `egui-wgpu` fixes required by the kit's
renderer code. It also selects SolidCraft's optional winit patch for Wayland
trackpad gestures. Cargo does not inherit patch settings from a Git
dependency's workspace root.

## Compatibility slice

PCBcraft's egui, eframe, egui_extras and egui_kittest versions move together to
0.37.0. The local eframe patch is rebased onto the original crates.io 0.37.0
package, retaining root-window retirement/recreation, native canvas shortcut
routing and clipboard read completion. Its source boundary is recorded in
[the patch notes](../vendor/eframe/PATCH.md).

The font facade now uses `solidcraft-look`'s Plex definitions and the UI kit's
interface weight names. The Plex source bytes match PCBcraft's existing faces.
The OFL Noto Emoji fallback previously bundled by egui 0.36.2 is retained as
unmodified font data, preserving portable glyph coverage for project names and
metadata in headless tests as well as desktop/browser hosts.

PCBcraft's theme facade synchronizes the shared dark/light state while retaining
its existing schematic, board and material tokens. Control adapters migrate
inside that facade rather than putting toolkit-specific state in document
models. Accessibility role/identity APIs and scroll fixtures follow egui 0.37;
previously unnamed controls receive explicit accessible names.

Workspace picker identity includes its native viewport. A pane or window keeps
its own navigation, camera, selection and pending input authority. Clipboard
image events complete captured text reads without creating project data or
leaving a delayed paste destination active.

## Shared menus

The chrome menu adapter uses the kit's popup frame and menu row widget in
File, Help, context menus, submenus and ribbon dropdowns. The kit measures rows,
paints labels, shortcuts and check marks, and reports their accessible state.
PCBcraft supplies its command glyphs, popup placement and action handlers.
The application clears the shared widget geometry registry at the start of
each frame before drawing the primary and detached views.

## Shared Preferences

Appearance and Navigation use the kit's segmented choices, Views uses its
dropdown, and Schematic uses its checkbox for Auto-heal wires. The chrome adapter
maps widget indices back to PCBcraft's typed settings. Segmented choices use the
toolkit's named dropdown when the setting column is too narrow. The app installs
the shared icon set so those dropdowns can draw their indicators. PCBcraft's
Preferences window continues to own page search, Apply, Cancel and defaults.

The modal, title bar, search field and action footer now come from the kit's
modal and window components. The page list and search results remain inside
PCBcraft's bounded page layout, with the search field above the page content.
This composition preserves backdrop dismissal and the per-page availability of
Restore Defaults. The pinned standalone Preferences wrapper opens a movable
window and always offers Restore Defaults. Narrow pages stack their labels
above controls so a wide field cannot expand the modal past the viewport.
Interface scale previews its value in the slider while the pointer is held.
The app applies the scale on release or when a typed value is confirmed, so
the slider stays in place while it is being edited.

## Shared Home components

Home uses the kit's file page, navigation rail, headings and action tiles.
PCBcraft supplies the project actions, enabled states and schematic and PCB
glyphs. The page retains its compact card sizes and puts navigation above the
tiles at narrow widths. Recent projects still use PCBcraft's vector preview
cards, whose painters receive the current theme and card bounds. The kit's
recent list accepts texture previews.

## Browser and Selection filters

Browser branches use the kit's disclosure button as a separate keyboard and
pointer target. PCBcraft's row still handles navigation, context menus and the
PCB layer color control. Selection filters use the kit's checkboxes, including
the mixed state for All items. The filters stay local to each editor view, and
All items applies only to categories supported by that view.

## Shared property dialogs

Symbol Properties and the net label forms use the kit's modal host, window
header, scrolling body and fixed action footer. PCBcraft handles field-level
Enter actions, validation and document changes. Escape, Cancel and backdrop
clicks dismiss the draft. An open dropdown handles Escape before the dialog
does.

Board Setup uses the shared modal host, header and action footer around a
bounded page editor. Its navigation and validation message sit outside the
page's scrolling fields. The primary action follows the draft's validation and
profile-loading state, and dropdowns handle Escape before the enclosing modal.

Assign Footprint uses the shared modal frame, title text, close control and
footer actions around its responsive browser panes. Place on board occupies
the footer's left action slot and has its own enabled state. The header close
control is named Close Assign Footprint so it remains distinguishable from the
footer's Close button. Each pane controls its scrolling and preview input.

## Source and licence record

The shared UI kit, look and theme retain the SolidCraft Brand and Interface
Licence. The local migration was explicitly authorized by Travis Lowe on
November 10, 2026, identifying PCBcraft as an official Tangency Labs subproject
and directing inclusion of these crates. The request for an upstream written
record remains [SolidCraft #14690](https://github.com/tangency-labs/solidcraft/issues/14690).
This branch records that distinction rather than describing the shared material
as MIT/Apache-licensed PCBcraft code.

`deny.toml` limits the custom licence exceptions to the three named 0.40.0-rc.3
packages and fingerprints their licence text. The look's fonts retain OFL-1.1.
Git sources are limited to the SolidCraft repository and require explicit
revisions. The retained licence texts are available in About; exact provenance
and hashes are in [ATTRIBUTION.md](../ATTRIBUTION.md).

## Validation

On November 10, 2026, this slice passed `cargo fmt --all --check`,
`cargo clippy --locked --workspace --all-targets -- -D warnings`,
`cargo test --locked --workspace`,
`cargo check --locked -p pcbcraft-web --target wasm32-unknown-unknown` and
`cargo deny --locked check licenses sources bans`. The workspace run passed
1,240 tests, with eight opt-in or helper tests ignored. The split-editor,
detached-editor, clipboard and library-editor regressions exercise shared
history and per-view input; the image-only clipboard regression covers the new
egui event path. Detached menu tests deliver the child's requested paint after
egui 0.37's hidden sizing pass, because the headless harness settles root
repaints only.

The ignored `native_window_lifecycle` regression also passed separately on an
isolated nested X11 display. It destroyed the original OS window while retaining
the child's identity, recreated a root, cancelled close, and exited after
removing the final child. Creation/destruction notifications may trail a
software-rendered frame, so the driver waits for them while retaining its exact
identity/count assertions and deadline. The actual desktop app was rendered and
inspected at 1280×800 in dark mode and 800×600 in light mode, including Home,
the schematic editor and the wide workspace picker. These runs used temporary
settings and an isolated display.

The failure-path review followed root retirement, deferred commands, texture
updates without a root surface, stale window events and clipboard completion
through their callers. Tests cover root recreation, cancelled shutdown, final
child removal, stale view input and unsupported clipboard images. The eframe
comparison against the original 0.37.0 package confirmed that only the recorded
local patch files and manifest differ. Direct Wayland, Windows, macOS and a
packaged browser interaction run remain outside this slice's verification;
the wasm result establishes compilation only.

After migrating the menus, the workspace suite, Clippy, WebAssembly compilation
and dependency policy checks passed again. File menus were inspected on the
desktop at 1280×800 in dark mode and 800×600 in light mode. The ribbon body,
dialog layouts and panels still use PCBcraft's own presentation code.

After migrating the Browser disclosures and Selection filters, the workspace
suite passed 1,279 tests with eight ignored. Formatting, Clippy and WebAssembly
compilation also passed. An isolated X11 run verified the mixed checkbox and
branch folding at 1280×800 in dark mode, with a layout check at 800×650 in
light mode.

The property dialog and Board Setup migrations passed the same workspace,
formatting, Clippy and WebAssembly checks. The net label regression exercises
Escape with a dropdown open, Escape from the draft, and backdrop dismissal over
Home at both widths. Board Setup's compact regression checks that Escape closes
the page chooser before the draft. Its desktop layout was inspected at
1280×800 in dark mode and 760×520 in light mode, with the action footer visible
below the scrolling page editor. Dependency license, source and ban checks
also passed.
