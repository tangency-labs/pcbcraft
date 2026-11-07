<h1 align="center">PCBCraft</h1>

<p align="center">
  <strong>Open-source electronics design for people and AI agents.</strong><br />
  Draw schematics, arrange footprints and work alongside an agent in the same editor,<br />
  with shared commands and undo history. Built in Rust for desktop and browser.
</p>

<p align="center">
  <a href="docs/quickstart.md">Quickstart</a> ·
  <a href="examples/README.md">Example project</a> ·
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

PCBCraft is in early development. Schematic editing runs on desktop and in the
browser, and the PCB workspace supports footprint inspection, placement moves
and rotation. Routing, full ERC/DRC and fabrication output are still planned.

- **Schematics:** place and edit symbols, draw and reshape orthogonal wires,
  move groups and organize flat or hierarchical sheets with shared undo/redo.
- **PCB workspace:** create a rectangular board outline, inspect physical starter footprints, move and rotate them,
  control layer visibility and highlight corresponding schematic components.
- **Views:** work with schematic and PCB panes side by side, or use multiple
  desktop windows with independent cameras and a shared project session.
- **Projects:** save a Git-friendly project folder or share a `.pc.zip` archive.
  Pinned symbol and footprint definitions travel with the project.
- **AI tools:** use the desktop Assistant with an installed agent or model
  endpoint, connect an MCP client to the open editor, or run a headless session.

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

The [quickstart](docs/quickstart.md) walks through opening a project, editing a
schematic, saving and inspecting its board. The [Dual LED example](examples/README.md)
includes a wired schematic and a shareable archive.

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
downloads `.pc.zip` archives; the Assistant and live MCP connection are desktop features.
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

The [AI and MCP guide](docs/mcp.md) covers agent setup, permissions, live
connections and command examples.

## Documentation

| Guide | Contents |
| --- | --- |
| [Quickstart](docs/quickstart.md) | First editing session and board walkthrough |
| [Editor guide](docs/editor-guide.md) | Tools, shortcuts, wiring, views and project files |
| [Capabilities](docs/capabilities.md) | Current support, limits and planned work |
| [AI assistants and MCP](docs/mcp.md) | Desktop Assistant, live editing and headless automation |
| [Symbols](docs/symbol-library.md) · [Footprints](docs/footprint-library.md) · [Library sources](docs/library-sources.md) | Definitions, mappings and source policy |
| [Native storage](docs/native-project-storage.md) | Folder/archive formats and save behavior |
| [PCB workspace](docs/board-workspace-plan.md) · [Representation](docs/pcb-representation.md) · [Materials](docs/board-materials.md) | Board architecture, implemented slices and planned extensions |

## Contributing

[CONTRIBUTING.md](CONTRIBUTING.md) covers the build, code map, required checks
and source rules. Find a bounded task in
[GitHub issues](https://github.com/tangency-labs/pcbcraft/issues) and check
[open pull requests](https://github.com/tangency-labs/pcbcraft/pulls) for work already underway.

PCBCraft is an independent application in the Craft family. Development follows
the [clean-room contribution rules](AGENTS.md): published specifications and
black-box observations are permitted; KiCad and other GPL EDA implementation
sources are excluded.

## License

MIT OR Apache-2.0, at your option. See [LICENSE-MIT](LICENSE-MIT),
[LICENSE-APACHE](LICENSE-APACHE) and [ATTRIBUTION.md](ATTRIBUTION.md) for the
project licenses and third-party notices. Library data, fonts and other assets
retain their documented licenses.
