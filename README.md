<h1 align="center">PCBCraft</h1>

<p align="center">
  <strong>Open-source electronics design for people and AI agents.</strong><br />
  Draw schematics, edit PCB layouts and work alongside an agent in the same editor,<br />
  with shared commands and undo history. Built in Rust for desktop and browser.
</p>

<p align="center">
  <a href="docs/quickstart.md">Quickstart</a> ·
  <a href="docs/README.md">Documentation</a> ·
  <a href="docs/capabilities.md">Capabilities</a> ·
  <a href="CONTRIBUTING.md">Contributing</a> ·
  <a href="VISION.md">Vision</a>
</p>

<p align="center">
  <a href="https://github.com/tangency-labs/pcbcraft/actions/workflows/ci.yml"><img src="https://github.com/tangency-labs/pcbcraft/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI" /></a>
  <a href="https://github.com/tangency-labs/pcbcraft/milestone/1"><img src="https://img.shields.io/badge/status-early_development-d97706" alt="Status: early development" /></a>
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue" alt="License: MIT OR Apache-2.0" /></a>
</p>

## What works

PCBCraft is in early development, with schematic and PCB editing on desktop and
in the browser. You can draw a circuit, assign footprints, review changes before
applying them to the board, place and edit copper tracks and through vias, and
save the schematic and board together with shared undo/redo.

- **Schematics:** place and edit symbols, draw and reshape orthogonal wires,
  move or grab groups, mark unused pins and wire ends, name nets with local/global
  labels and power/ground symbols, and organize flat or hierarchical sheets.
- **PCB layout:** assign pinned footprints, review package and connection changes
  before applying them, draw a Board Outline, move/rotate/flip footprints, and
  place, move and resize tracks and through vias, and route around fixed copper. Layer controls, net highlighting
  and schematic/PCB component highlights help navigate the board; see the
  [layout guide](docs/pcb-layout-workflow.md).
- **Checks:** review DRC findings for shorts, copper clearance,
  track width, via annular rings, unrouted connections, board edges and courtyard
  overlap, using saved board rules and supported dimensional net-class rules.
- **Outputs:** export supported board geometry as Gerber X2 copper, mask, paste,
  silkscreen and outline layers, with circular plated holes in an Excellon drill
  file, together in one archive.
- **Views:** work with schematic and PCB panes side by side, or use multiple
  desktop windows with independent cameras and a shared project session.
- **Libraries:** browse built-in, project-pinned and registered native libraries
  in Parts Library. Place generic or orderable parts, choose packages and assign
  footprints. Author symbols and footprints in their own workspaces, then publish
  revisions to writable native folders on desktop.
- **Projects:** save a Git-friendly project folder or share a `.pc` archive.
  Pinned symbol and footprint definitions travel with the project. The CLI can
  explicitly convert the previous split layout into a new copy.
- **AI tools:** use the desktop Assistant with an installed agent or model
  endpoint, connect an MCP client to the open editor, or run a headless session.

Full ERC, broader DRC coverage, advanced routing, copper-zone fill
and 3D/STEP workflows remain in development. Footprint slots, non-plated drills
and independent footprint mask/paste controls are not yet supported. The current
checks and exporters do not establish manufacturing readiness.

The [capability table](docs/capabilities.md) separates available features from
remaining work. The [MVP milestone](https://github.com/tangency-labs/pcbcraft/milestone/1)
tracks the first collaboration-ready preview; [VISION.md](VISION.md) explains
the longer-term direction, including familiar KiCad controls and links with SolidCraft.

## Get started

Use **Rust 1.95 or later**. From the repository root, build the desktop app and
its companion CLI, then launch the editor:

```sh
cargo build --locked -p pcbcraft -p pcbcraft-cli
cargo run --locked -p pcbcraft
```

The [quickstart](docs/quickstart.md) covers opening a project, schematic editing
and saving, with PCB tools described in the [editor guide](docs/editor-guide.md).
The [sensor node example](examples/README.md) includes a small MCU, an I²C sensor,
status LEDs and a routed board; a transistor flasher is available as a second
demo. Both have shareable archives.

<details>
<summary><strong>Run the browser app locally</strong></summary>

Install the WebAssembly target and [Trunk](https://trunkrs.dev), then serve the app:

```sh
rustup target add wasm32-unknown-unknown
cargo install --locked trunk --version 0.21.14
cd apps/pcbcraft-web
trunk serve --locked
```

Open [localhost:8781](http://127.0.0.1:8781). The browser uploads projects and
downloads `.pc` archives; the Assistant and live MCP connection are desktop features.
Build prerequisites and troubleshooting are in [CONTRIBUTING.md](CONTRIBUTING.md#build-and-find-your-way-around).

</details>

## AI assistants and MCP

Open **Assistant** beside Preferences in the desktop app to connect an installed
coding agent or a compatible model endpoint. Edits run through the editor's
commands, appear on the canvas and share its undo history.

For an external MCP client, choose **Help (?) → Connect AI assistants… → Enable
local connection** and copy the configuration. For a separate headless session:

```sh
cargo run --locked -p pcbcraft-cli -- mcp
```

Native live MCP also supports [UI automation](docs/ui-automation.md): locate named
controls and send clicks, drags, keys, text and scrolling through ordinary editor
input. Input currently targets the original native window and its split panes;
headless sessions operate separately from the open editor.

The [AI and MCP guide](docs/mcp.md) covers agent setup, permissions, live
connections and command examples.

## Documentation

The [documentation map](docs/README.md) groups user guides, implementation
references, architecture plans and verification records, with reading paths for
contributors and agents.

| Guide | Contents |
| --- | --- |
| [Quickstart](docs/quickstart.md) · [Example project](examples/README.md) | First editing session |
| [Editor guide](docs/editor-guide.md) | Tools, shortcuts, wiring, views and project files |
| [Capabilities](docs/capabilities.md) | Current support, limits and planned work |
| [AI assistants and MCP](docs/mcp.md) · [UI automation](docs/ui-automation.md) | Agent connections, shared commands and native UI input |
| [Contributing](CONTRIBUTING.md) · [Agent instructions](AGENTS.md) | Repository workflow, architecture and checks |
| [Reference and formats](docs/README.md#reference-and-formats) | Project storage, library schemas, nets, rules and shared geometry |

## Contributing

[CONTRIBUTING.md](CONTRIBUTING.md) covers the build, code map, required checks
and source rules. Find a bounded task in
[GitHub issues](https://github.com/tangency-labs/pcbcraft/issues), check for
existing claims and open PRs, and leave a **`claimed`** comment on the issue
before starting. The first claimant gets merge priority under the
[issue claim policy](CONTRIBUTING.md#claim-the-issue-before-starting).

PCBCraft is an independent application in the Craft family. Development follows
the [clean-room contribution rules](AGENTS.md): published specifications and
black-box observations are permitted; KiCad and other GPL EDA implementation
sources are excluded.

## License

MIT OR Apache-2.0, at your option. See [LICENSE-MIT](LICENSE-MIT),
[LICENSE-APACHE](LICENSE-APACHE) and [ATTRIBUTION.md](ATTRIBUTION.md) for the
project licenses and third-party notices. Library data, fonts and other assets
retain their documented licenses.
