# PCBCraft

Electronics design that feels like the tools you already know, built so a person and an agent can work on the same board, at the same time, with the same commands.

**Status: just started.** Schematic editing works on desktop and in the browser.
Use **File → New Project…** to start. The app bar shows the project name and an
unsaved marker beside **Undo**, **Redo**, and **Save** (**Download** in the
browser). The gear opens **Preferences**. See [VISION.md](VISION.md) for the
project's direction.

Choose **Capture → Symbol** or press **A** to pick a resistor or LED. Click to
place it on the schematic grid or press **Esc** to cancel. Choose **Capture → Wire**
or press **W**, then click two terminal markers to connect them. Press **Esc** or
right-click and choose **Cancel** to discard an in-progress wire.

Scroll to zoom or drag with the middle mouse button to pan. Use **Fit** to bring
the circuit into view and **Nets** to open the Net Inspector.

Open a native `.pc` project from **File → Open…**. On desktop, **Save** keeps the
open file's format, and **File → Save As…** offers a choice. In the browser, use
**Download Project…** for the same choice. Both plain JSON and ZIP use `.pc`.
The [native storage notes](docs/native-project-storage.md) describe the formats,
file safety and performance checks.

## Run it

Use Rust 1.95 or later and start in the repository root. To launch the desktop app:

```sh
cargo run --locked -p pcbcraft
```

For the browser, install [Trunk](https://trunkrs.dev) and run:

```sh
rustup target add wasm32-unknown-unknown
cd apps/pcbcraft-web
trunk serve --locked
```

Then open [the local web app](http://127.0.0.1:8781).

To start the MCP server from the repository root, run
`cargo run --locked -p pcbcraft-cli -- mcp`. To connect an MCP client, point it
at `pcbcraft-cli mcp`. Pass
`--project path/to/project.pc` after `mcp` to start with a saved snapshot. The
headless session is separate from the app window. Send `inspect: false` with
`execute` to return just the resulting revision. Use `inspect_project` to read
the project, or `cross_probe` to look up a linked terminal or pad.

An MCP client can use `schematic.connect_pins` to join two terminals from
`inspect_project`. Describe the command for its input shape, and send the current
project revision with the two endpoints. Project inspection returns the wires,
connected terminal groups and unconnected pins.

## Where things go

| Path | What it is |
|---|---|
| `crates/engine` | The session and the command set. The schematic and board model goes here. No UI. |
| `crates/mcp` | The MCP server, a thin layer over the engine's commands. |
| `crates/app` | The window, shared by desktop and browser. Menus run engine commands. |
| `crates/chrome` | Theme, fonts, menus, bars and dialogs in the Craft style. Stands in for the shared Craft UI crate. |
| `apps/` | The desktop, command line and browser programs. They only start things. |

To add a feature: add a command to `crates/engine/src/lib.rs`, then call it from a menu or tool. MCP's `list_commands` and `execute` pick it up on their own.

## Checks

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo deny --locked check licenses sources bans
```

## License

MIT OR Apache-2.0, at your option. See [LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE) and [ATTRIBUTION.md](ATTRIBUTION.md).
