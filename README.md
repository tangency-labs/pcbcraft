# PCBCraft

**A Rust EDA application, MCP-first, with one window and tabbed workspaces.**

GPL-3.0-or-later. Inspired by KiCad workflows and the SolidCraft / Craft application
architecture. Uses the pinned `craft-ui` library from `storytold/craft-libs`.

**Status: first working foundation, not a KiCad replacement yet.** The current
vertical slice is a flat schematic → generic PCB → basic routing → checks → KiCad
board export. The complete EDA roadmap is in [ROADMAP.md](ROADMAP.md).

## What works

- One native Rust window with Schematic, PCB Layout, Libraries and Checks tabs;
  project browser and properties remain docked in the same window.
- Dark default, light theme, persistent accent and grid preferences, zoom around
  the cursor, middle-button pan, symbol previews and drag-to-move.
- Periodic project recovery and recovery on normal close, using application storage.
- Original resistor, capacitor, LED and two-pin connector symbols; explicit
  logical nets and pin-to-pin orthogonal wire presentations.
- Schematic-to-PCB sync retains existing placements and reconciles pad nets.
- Generic front-layer circular SMD pads; straight or two-segment orthogonal tracks;
  airwires from actual same-layer copper connectivity.
- Editable rectangular board width/height and dimensioned circular cutouts,
  exported as Edge.Cuts. Constrained/parametric outline sketches are planned.
- Initial pin assignment/net-population checks and copper short, clearance,
  board-edge, cutout and unrouted checks with affected object IDs.
- 17 engine commands, discoverable schemas, atomic batches, stable IDs,
  revision guards, 32-step undo/redo and validated atomic project saves.
- 30 MCP tools: every engine command plus inspection, checks, persistence,
  export and PNG previews. Headless or bridged into the actual running desktop.
- Rust browser entry point; the shared UI and engine compile to WebAssembly.
  Browser host-file open/save/export are not implemented yet.

## Run on Linux / WSL

Install Rust 1.95+ and window-system development libraries. On Ubuntu:

```sh
sudo apt install build-essential pkg-config libxkbcommon-dev libwayland-dev \
  libx11-dev libxi-dev libxrandr-dev libxcursor-dev libgl1-mesa-dev
rustup target add wasm32-unknown-unknown
cargo run -p pcbcraft -- --sample --control 127.0.0.1:7991
cargo run -p pcbcraft -- --project led.pcbcraft --tab pcb
```

WSLg can display the Linux app on Windows. For automated UI capture without a
desktop, use Xvfb with `--capture`:

```sh
xvfb-run -a -s '-screen 0 1600x1100x24' \
  env -u WAYLAND_DISPLAY WGPU_BACKEND=vulkan \
  cargo run -p pcbcraft -- --sample --capture /tmp/pcbcraft.png
```

Windows can also build natively with `cargo run -p pcbcraft -- --sample`.

## First circuit

Load **LED example**, or press **A** to select a symbol and click to place it.
Press **W**, then click two pin circles to connect them. A new net is created
atomically if neither pin is assigned; conflicting existing nets are rejected.
Click **Sync to PCB** to generate footprints. Press **X** and click two same-net
pads to route between them. Open **Checks** to inspect remaining findings. Save
the native `.pcbcraft` project or export its board to `.kicad_pcb`.

The route tool is a simple orthogonal pad-to-pad operation. It does not avoid
obstacles; inspect its clearance findings. Footprint movement does not drag tracks.
Wire crossings do not implicitly join logical nets.

## Keyboard defaults

| Shortcut | Action |
|---|---|
| A | Inline add-symbol chooser (schematic) |
| W | Wire between pins (schematic) |
| X | Route between pads (PCB) |
| M | Move selection, then click destination |
| R | Rotate selection 90° |
| E | Show properties |
| Delete | Delete selection |
| Escape | Cancel tool / placement preview |
| Ctrl+Z / Ctrl+Y | Undo / redo (Ctrl+Shift+Z also redoes) |
| Ctrl+S | Save to the inline project path |
| Ctrl+K | Inline command panel |
| Home | Fit the active canvas |

These are the initial KiCad-style defaults, not a complete configurable KiCad keymap.

## CLI and MCP

```sh
cargo run -p pcbcraft-cli -- commands
cargo run -p pcbcraft-cli -- sample --out led.pcbcraft
cargo run -p pcbcraft-cli -- run examples/led-indicator.json --save led.pcbcraft --export led.kicad_pcb
cargo run -p pcbcraft-cli -- checks --project led.pcbcraft
cargo run -p pcbcraft-cli -- snapshot --sample --view pcb --out led.png
cargo run -p pcbcraft-cli -- mcp
cargo run -p pcbcraft-cli -- mcp --connect 127.0.0.1:7991
```

See [docs/mcp.md](docs/mcp.md) for client configuration and request examples.
MCP uses JSON-RPC over newline-delimited stdio. Logs/errors go to stderr.

## Development

```sh
cargo xtask ci  # formatting, clippy, tests, UI/engine/browser wasm checks
cargo build -p pcbcraft -p pcbcraft-cli
# Browser: install Trunk, then:
cd apps/pcbcraft-web && trunk serve
```

Keep build products outside the mounted Windows tree in WSL for better filesystem
performance, e.g. `CARGO_TARGET_DIR=$HOME/.cache/pcbcraft-target`.

Architecture: [docs/architecture.md](docs/architecture.md). Agent instructions:
[AGENTS.md](AGENTS.md). Provenance: [ATTRIBUTION.md](ATTRIBUTION.md).

## License

PCBCraft is licensed under GNU GPL version 3 or later; see [LICENSE](LICENSE).
Third-party dependencies retain their own licenses and notices. No KiCad source
implementation has been copied or ported in this initial version. KiCad is an
independent project; PCBCraft is not affiliated with or endorsed by the KiCad team.
