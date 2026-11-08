# Native library sources

This slice of #92 loads native symbols and footprints from personal sources.
Symbols can be searched and placed through the Parts Catalog. Footprints can be searched and
inspected in Library sources; connecting external footprints to the assignment
chooser remains separate work. Registering, refreshing or removing a source
does not change the project, its revision, dirty state or undo history.

Open **Libraries → Sources**. On desktop, **Add folder…** selects a native
library containing `library.json` and optional flat `symbols/` and `footprints/`
directories. Registrations are saved in personal application settings, including
their machine-specific folder paths. They are read in the background on startup.
Each source card's **⋯** menu contains **Refresh**, **Relocate…** and **Remove**
(uploaded sources offer only Remove). **Refresh** reloads a source; **Relocate…**
points its registration at a moved folder. Library and item identities come from their documents, so moving a folder
does not change them. A missing or unreadable source remains listed with its
error and stops supplying items until a successful refresh.

**Upload bundle…** works on desktop and in the browser. Uploaded sources last
for the current session; the browser does not get access to arbitrary folders.
To try the original, MIT OR Apache-2.0 example, select `examples/native-library`
on desktop, or create a bundle for upload:

```sh
python3 scripts/package-native-library.py examples/native-library /tmp/workshop.pclib.json
```

The packager preserves an existing output file. Its JSON envelope has format
`pcbcraft-library-bundle/1` and a `documents` object mapping relative paths to
UTF-8 file contents. The engine validates the same native documents on both
platforms. An uploaded source must be uploaded again after a restart, while any
symbols already placed in a saved project reopen from their pinned copies.

After loading the example, press **A**, search for **Workshop**, choose
**Workshop resistor**, and place it. Search results retain library/item IDs,
revision and license in their tooltips. Library sources can search both symbols
and footprints and filter by kind. Sources and the catalog appear side by side
on wide windows and as **Sources** / **Catalog** tabs on compact windows, with
independent scrolling and a fixed Close action. A bad item reports its filename
and validation error without hiding other valid items; an invalid manifest prevents the source
from loading. The built-in `starter` library identity is reserved.

The source loader accepts at most 16 external registrations, 256 files per
source, 1 MiB per file and 8 MiB of decoded UTF-8 content per source. Bundles are
also bounded to 16 MiB before decoding. Folder reads reject symlinks, special
files and nested item directories. Other root files, such as a library's license
text, are not treated as native item documents; retain those notices with the
source and comply with the asset's redistribution terms when sharing it.

Definitions with the same library, kind, item ID and revision must have identical
content. Conflicting copies are reported and excluded from search and placement.
A conflicting latest revision does not silently fall back to an older one.
Refreshing a registration also detects content changes under an existing
revision. Correct the source or give a changed definition a new revision.
Removing a registration forgets its disposable source cache, but a project still
refuses a new placement that conflicts with its already pinned revision.

External symbols retain portable library-name and license attribution with their
pinned definition, alongside the existing library/item/revision identity.
Personal source IDs and machine paths are not copied into project documents.
Previously saved symbols without attribution retain their original encoding.
Opening a project never searches or refreshes a library to replace its geometry.

## Shared commands

GUI source loading and MCP use the same validated catalog transaction. Native
folder reads and desktop bundle parsing run on a single background worker;
MCP supplies already transported UTF-8 document text rather than asking the
engine to read a path.

| Command | Parameters and result |
| --- | --- |
| `library.load_source` | `source` is a personal portable ID; `documents` maps supported relative paths to file text. Returns source inspection. |
| `library.list_sources` | No parameters. Returns source identities, licenses, counts and file/conflict diagnostics. |
| `library.search` | Optional `query` and `kind` (`symbol` or `footprint`). Returns current unambiguous items, their exact keys, formats, provenance and content digests. |
| `library.remove_source` | `source`. Removes that session source without changing pinned definitions. Desktop personal folder registrations are managed in the Sources dialog. |

Use `execute` with normal inspection enabled to receive these results;
`inspect: false` intentionally returns only the unchanged project revision.
For an external `schematic.place_symbol`, pass `library`, `definition`,
`definition_revision` and `definition_digest` from search, plus the usual sheet,
coordinates and `expected_revision`. Placement checks both project revision and
the exact definition content, so a source refresh during a preview cannot
substitute another symbol. Existing starter-only calls remain valid.

A removed source cannot be restored by a late worker result. A source change
through MCP while a worker is loading invalidates that delayed result. A blocked
filesystem read retains the single worker slot until it returns; removing its
registration discards the result, and application shutdown does not wait for the
reader. Project commands and editing remain independent of the reader.

Native parts linking symbols to footprints, project source tables, external
footprint assignment, library authoring, Git/providers and external-format
imports are outside this slice. The remaining acceptance in #92 stays open.
