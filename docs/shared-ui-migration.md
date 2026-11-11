# Shared Craft UI migration

This local branch adopts SolidCraft's extracted interface crates at
`f56114219b2d7f6d16de8dd58700479646948f8a`. The migration began at PCBcraft
`16e7bab` and was rebased onto upstream main `6895130` on November 11, 2026.
The first slice establishes dependencies, fonts and toolkit
compatibility. The shared kit also draws menus and Preferences choices through
`pcbcraft-chrome`.

The component migration is complete at this pinned revision: shared chrome
adapters cover dialogs, command controls, choices, common text fields,
disclosures, ribbon groups, panel surfaces and Assistant presentation. The
adoption boundary below records the native editing and host responsibilities
that remain in PCBcraft.

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

## Parts Catalog controls

The expanded chooser and compact dock share the toolkit's search field and
Repeat placement checkbox. Density choices use its segmented control, while
short result panes use List and retain the selected expanded density. The Library
selector uses the shared dropdown through PCBcraft's labelled choice adapter,
which also serves property forms. The app owns query changes, library keys,
selected definitions, focus and placement authority.

The expanded catalog uses the shared modal frame, named close control, Dock
button and action footer. Manage libraries occupies the footer's independent
left action slot, including when Place is disabled. The bounded browsing area
hosts the catalog's toolbar and responsive panes, and the app handles Enter
confirmation and switching between expanded and docked presentations.

Library Sources uses the shared modal frame, named title close, footer, search
field and import buttons. Its source cards, category filters and responsive
panes carry the existing source diagnostics and catalog metadata. The manager
handles an open source-action menu's Escape before dismissing the dialog.

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

Update PCB from Schematic uses the shared modal frame and action footer, with
the same title and close adapter as Assign Footprint. Its report uses the
toolkit's bounded list, checkboxes and action buttons. The All filter displays
a mixed state when some categories are hidden, while warning and error totals
count the complete report. The application controls review validity, update
approval and placement handoffs.

## Unsaved changes

The project save prompt uses the shared modal frame, title close control and
confirmation footer. Don't Save remains available when Save is disabled during
a file operation or omitted in the browser. Header close, Cancel, Escape and
backdrop dismissal retain the pending project. The app continues to own save
completion, failures, edits made during a save and subsequent close/open actions.

## Other dialogs

The common chrome dialog adapter uses the shared modal frame and named title
close control. Naming, hierarchy, arrangement, file-location, clipboard, layer
colour, library-editor and Assistant configuration dialogs inherit this host.
Long bodies scroll within the viewport below the fixed title. Each caller
retains its validation, focus, actions and document or connection lifecycle.

Ordinary command buttons and checkboxes use the shared toolkit through the
chrome control adapter, including Assistant permissions/configuration, library
authoring, Board Setup and clipboard options. Enabled and sized action layouts
retain their caller-owned availability. Net Inspector uses the shared search
field while keeping connectivity resolution and filtering in the app.

The remaining choice fields use the shared bounded dropdown, including board
materials/stackup, pin mapping, field appearance, library authoring and Assistant
model/mode selection. Typed option values remain with their existing callers.
The active-layer selector retains its colour swatch and full accessible value
while using shared dropdown chrome. Escape closes a dropdown before its modal.

## Workspace panels and Assistant

Floating workspace cards and ribbon cards use shared container frames. Panel
titles, grips, disclosure controls and catalog header actions use toolkit
presentation with PCBcraft-owned placement and drag state. Editor tabs use the
shared tab surface and close control while retaining pane focus and native
window commands. Assistant uses the shared drawer frame, status badges,
composer and message bubbles. Its protocol, permissions, input limits and
save/close lifecycle remain in PCBcraft.

Ribbon command groups use the shared ribbon renderer and its button metrics;
PCBcraft supplies qualified electronics glyphs, enabled/selected state and menu
actions. The host retains horizontal scrolling and per-workspace tab contents.
Inline ribbon fields align with the shared group height. App-bar quick actions
and arrangement tabs use shared button presentation within PCBcraft's existing
native title-bar placement.

Common labelled plain-text fields and property tabs use shared field and
segmented controls. Layer groups, text appearance and Assistant tool details
use shared disclosures over native persisted collapse state. Compact command
actions and source-category choices also use shared controls.

## Adoption boundary

Reusable UI presentation is supplied through the shared kit and chrome adapters.
PCBcraft retains its electrical canvases, previews, window/view identities,
pane and panel placement, input routing and document/connection lifecycles.
The app bar and ribbon compose shared controls around those host contracts.
They do not adopt SolidCraft's application-level panel runtime.

Native egui primitives remain where PCBcraft needs builder features absent from
the pinned standalone kit: password/length-limited text, cursor-controlled
inline edits, multiline clipboard input, coordinate tables, radio choices and
the opaque colour picker. Numeric parsing, snapping and delayed scale application
remain PCBcraft behavior. These are intentional adapter boundaries rather than
replacement copies of shared dialog or command-control components.

Status-bar unit/grid actions, filter reset, setup page navigation and bounded
footprint actions also use shared controls. Native selection rows remain inside
the shared menus and domain-specific preview lists.
The status bar accommodates the shared controls' minimum height so its grid
and unit targets stay below the canvas, including at larger interface scales.

## Ribbon background

The outer ribbon strip is transparent, exposing each top pane's grid through
the margins around the workspace and ribbon cards. A deferred paint slot uses
the cameras after this frame's navigation, including independent split and
definition views. The background uses the same spacing, origin, opacity and
board grid style as the corresponding canvas. Drawing and selection operate
within the editor panes below the ribbon.

This follows the visual composition of SolidCraft's floating shell at the
pinned revision, whose viewport is painted before its ribbon cards. This slice
extends PCBcraft's grid background into those margins.

Both project workspaces expose a Grid toggle on the View ribbon and its Display
menu. Schematic and PCB visibility are saved independently, including through
the live preference interface, while opacity and the PCB's dot/line style are
retained when hidden. Showing a grid from older opacity-zero settings restores
default opacity for that workspace; showing a PCB grid set to Off starts with
dots. The renderer applies these choices to the canvas and ribbon margins.
Preferences shows Off when the PCB grid is hidden; selecting Dots or Lines also
turns its visibility back on, including when it was hidden from the ribbon.

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

After rebasing onto `6895130`, the same formatting, Clippy, WebAssembly and
dependency checks passed again, and the workspace suite passed 1,286 tests with
eight ignored. All 18 migration patches replayed unchanged, preserving their
author and co-author records.

The completion pass on November 11, 2026 passed formatting, workspace Clippy
with warnings denied, all 1,284 workspace tests (eight ignored), WebAssembly
compilation and dependency license/source/ban checks. The opt-in native window
lifecycle regression also passed on isolated X11, covering root retirement,
recreation, cancelled close and final-child shutdown. Desktop screenshots cover
wide/compact schematic and PCB ribbons, Assistant and property controls in both
themes. A packaged development web build from `trunk build --locked` was tested
in isolated headless Chrome: Home, both workspaces, grid toggles, compact layout
and Preferences theme changes rendered without observed JavaScript exceptions
or console errors. Windows, macOS and direct Wayland remain untested here.

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
