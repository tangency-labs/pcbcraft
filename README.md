# PCBCraft

Electronics design that feels like the tools you already know, built so a person and an agent can work on the same board, at the same time, with the same commands.

**Status: just started.** Schematic editing works on desktop and in the browser.
Use **File → New Project…** to start. The app bar shows the project name and an
unsaved marker beside **Undo**, **Redo**, and **Save** (**Download** in the
browser). The gear (or **Ctrl+,**) opens **Preferences**, with
pages for Appearance (theme, interface scale), Navigation (whether the wheel
zooms or pans, zoom direction, speed and anchor), a list of Shortcuts and, on
the desktop, the AI page for the Assistant.
Search finds any setting; changes show at once, and **Cancel** puts them back. The house button beside the brand opens
**Home**, with New Project, Open, Save, Preferences and About in its rail. Press
**Esc** or choose **Back to** the project to return to the schematic, which stays
open with its history while Home is shown. See [VISION.md](VISION.md) for the
project's direction.

Choose **Capture → Symbol** or press **A** to pick a resistor, LED, capacitor,
two-pin connector, or a module: controller, 4-wire fan connector, regulator or
analog temperature sensor. Click to
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

## Assistant in the desktop editor

Open **Assistant** from the chat icon next to Preferences or press
**Ctrl+Shift+A** (**Cmd+Shift+A** on Mac). Drag the title to switch sides,
drag the inside edge to resize, or use **Pop out Assistant** to move the chat
into its own window. **Dock Assistant** returns it to the workspace.
Minimize folds the panel into the bottom-right strip. Close hides the panel
and keeps the conversation available when you reopen it.

Open **Preferences → AI** from Home or the gear in the app bar to test
sign-in, choose the default agent, and set where and when Assistant opens.
**Add agent…** saves a named Codex
or Claude Code profile. Choose **Installed agent** to use the standard
executable on PATH or **Custom command** to use a compatible executable at
another location. Both **Add agent…** and **AI settings…** are available
from the Assistant's agent dropdown.

For an installed agent, put the Codex CLI or Claude Code on your PATH. For a
custom command, set the executable path in its profile. Choose the agent and
select **Connect**.
The header shows **Ready** when PCBcraft can use the agent's saved login.
If the agent asks you to sign in, choose **Sign in** and complete the browser
flow. You can also sign in with `codex login` or `claude auth login --claudeai`
in a terminal, then choose **I've signed in** or **Retry** in PCBcraft.

Open a schematic and ask a question or request a wire or a symbol move.
**Send** shares the schematic and recent conversation with the selected
agent. Review the specific edit, then choose **Apply** or **Discard**.
**Undo** reverts an applied edit. **Stop** cancels a request in progress.
If the schematic changed since the request, ask again before applying.

PCBcraft remembers the agent choice and reuses the CLI sign-in across
projects. Switching projects or agents starts a fresh conversation.

## Live MCP in the desktop editor

Build the desktop and CLI binaries with `cargo build --locked -p pcbcraft -p pcbcraft-cli`.
Open a project in the desktop app and choose **File → Local MCP… → Enable local
connection**. Copy the configuration into your MCP client. If `pcbcraft-cli`
isn't on your PATH, set its command to the full path of the binary in
`target/debug`. Close the setup dialog before editing. The copied configuration
grants access to this window until you disable the connection or close the app.

For a quick check, open `crates/app/tests/fixtures/two-resistors.pc`. Use
`inspect_project` to get R1's symbol ID and the current revision. Call `execute`
with `schematic.move_symbol` and pass the ID as `symbol`, coordinates as integer
`x_nm` and `y_nm`, and the inspected revision as `expected_revision`. Move R1 to
`(0, 5_080_000)` and watch the attached wire follow it. Undo and redo in the
editor, then inspect again to confirm the position. The edit stays unsaved
until you save the project.

## Headless MCP

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
| `crates/craft-control` | Bounded line framing and authenticated local transport. |
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
