# What PCBCraft Should Be

November 4, 2026

## Why PCBCraft should exist

PCB design is ripe for what we did with SolidCraft: software that feels like the tools you already know, with modern guts that humans and agents can both work in.

Today you mostly get two options:

- A legacy WYSIWYG tool with AI bolted on top that barely works.
- Python, JS or some random DSL that generates the board, but you can't WYSIWYG edit the result.

I've used KiCad for a while. With my setup, agents have to edit schematic files directly. There's some integration on the PCB side, but it isn't the complete workflow we want. Nobody should need 1,000 tools just to edit s-expressions.

I don't want either of those options. I want one tool where a person and an agent can work on the same board, at the same time, with the same commands.

## It has to feel like KiCad

Muscle memory is the name of the game. If an engineer can't sit down and move at full speed on day one, they won't switch, however clever the new design is.

This is what all the Craft apps are: they look and feel like existing software, with modern, interoperable internals underneath. KiCad has pretty good usability. It's the guts that are rigid and hard to work with. So we keep the part people already know and replace the part that's holding them back.

- **Same controls.** The same controls as KiCad, so your hands already know what to do.
- **Familiar canvas.** The schematic and PCB canvas should look like KiCad, or close to it, and always be themeable.
- **Family feel.** It should still feel like part of the Craft family, on the UI we're unifying across the apps.

New ideas belong on top of that base, not in place of it. Reinventing the workflow is not the goal.

## Built for humans and agents from day one

What interests me is building this from the ground up for humans and agents to work alongside each other, with agent support designed in from the beginning, not bolted on later.

- **One set of commands.** MCP should be able to do everything using the same internal commands humans use. That's how SolidCraft works.
- **No file surgery.** Agents shouldn't be hand-editing project files to get things done.
- **Still a real editor.** You can always grab the board and edit it directly. Agents don't replace the WYSIWYG experience.
- **Open to contributors.** Every PR is written by an agent. If you have a Claude account, you can contribute, whether or not you know how to code.

AI is still pretty bad at the spatial part of PCBs, especially the way KiCad does it. But LLMs can somehow make coherent things in SolidCraft, and I want that for PCBs.

### First live editing milestone

An agent can move a wired symbol in the schematic the user has open. The
symbol and attached wire update on the canvas, and the edit stays unsaved
until the user saves. Undo and redo from the editor, then inspect through MCP
after each step to verify both see the same state. The editor refuses a move
based on a revision that has since changed.

## What we fix

Keep what works, then go after the things that have annoyed everyone for years.

- **Library management.** I had to write my own KiCad plugin just to pull Digi-Key, Ultra Librarian and SnapMagic files into a project library, and it only works from the PCB layout page. This should be built in. It's also the most common request in our community discussion.
- **The schematic side.** KiCad still has no proper API for schematics. Here, schematic and PCB get the same treatment.
- **SolidCraft link.** PCBCraft is its own app, not a tab inside SolidCraft, but the two should definitely interconnect.
- **Everything else.** Once that base feels right, we can build what people want on top of it, without gatekeeping.

## Clean-room, MIT/Apache

Our rule is clean-room, and the license is MIT/Apache, like the rest of the Craft family. We can study how KiCad behaves, but we don't read or port its code. That's slower than porting, and it's the right trade: code can move freely between the Craft apps.

## How our files should work

Files are hard to change once people have real boards saved in them, so we discuss this in the open first. Everything in the table below is a proposal, so push back on it. One thing is already firm: PCBCraft files must work well with version control tools like Git. Whether the app ever includes version control itself isn't part of this discussion.

"Version-controllable" means:

- **No-op saves.** Opening and saving without changes produces zero diff.
- **Small diffs.** Moving one part changes only that part's lines, because everything saves in a stable order with stable IDs.
- **No noise.** Timestamps, caches and thumbnails stay out of tracked files.
- **Separate binaries.** 3D models and other large files live on their own, referenced by hash.
- **Mergeable.** Two people editing different parts of a board merge cleanly, and a real conflict is caught on load.

| Topic | Proposal |
|---|---|
| Storage | A project folder is the working format for Git, with a plain JSON `project.pc` entry point and separate schematic, board and pinned library documents. A `.pc.zip` archive packages the project for sharing. The [storage spec](docs/native-project-storage.md#file-encodings-and-save-behavior) defines both forms, including archives without a manifest. |
| Links between things | A link pins a permanent ID and a version. The path is only a hint, and taking the latest version is a deliberate step. |
| One file or many | Start in one project, then promote a block or part to its own linked item. |
| Parts and libraries | A part links a symbol, footprint, 3D model and supplier data, each optional until needed. Projects keep a pinned copy, so they open without the library. |
| Reusable circuits | A reusable block works like a linked part, with the same ID and version rules. |

## What's still open

PCBCraft is still an idea we're exploring with the community. The direction above is firm, but these questions aren't settled:

- **Routing.** Can agents do real routing work through MCP tools? We're building this ourselves, and it will likely take several iterations to find out.
- **Who leads.** Who leads the work day to day?

These get decided in #pcbcraft, with the engineers who actually design boards.
