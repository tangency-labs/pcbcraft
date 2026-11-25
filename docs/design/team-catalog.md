# Read-only team catalog and provider records

November 23, 2026. A proposal for
[#94](https://github.com/tangency-labs/pcbcraft/issues/94). It builds on the
[library sources model](library-sources.md) and the
[native library sources](../libraries/native-library-sources.md) guide. The team catalog
and its provider records are not implemented yet.

The first remote source should be one controlled, read-only team catalog. It is
small enough to review end to end, and it shows how search, details,
authentication and offline reopen work for every later remote source. Supplier
data stays separate from it: projects pin the engineering definitions they use,
while price and stock live in dated provider records that refresh on their own.

## Scope

In scope:

- One team catalog, registered per person or per project, that PCBcraft only
  reads.
- Search, details, sign-in and offline reopen for that catalog, through the same
  shared commands in the GUI and MCP.
- Dated price and stock records, kept apart from library definitions and
  project files.

Out of scope for this slice:

- Writing, publishing or approving items in the catalog from PCBcraft.
- Distributor or manufacturer connectors, carts and purchasing.
- Merging several remote catalogs, and Git sources
  ([#93](https://github.com/tangency-labs/pcbcraft/issues/93)).
- The wire protocol and on-disk cache format. Both need their own storage review.

## The catalog

A team catalog is a [source](library-sources.md#sources-and-identity) that a
team controls and serves from one location. It holds ordinary native records:
symbols, footprints, category schemas, generic parts and manufacturer parts.

- **Read-only.** PCBcraft never edits, uploads or publishes to it. Items are
  authored and published by the team's own process, outside this slice.
- **Same identity rules.** Items are addressed by library ID, kind, item ID and
  revision. A published revision never changes. Two copies of one revision with
  different content are a conflict that is reported, never chosen.
- **Same validation.** Fetched items go through the native codecs used for local
  folders. An invalid item is reported on that item, and the rest of the catalog
  stays usable.
- **Registration.** The catalog is registered like other sources. A project
  registration stores its identity and location only, never credentials or
  machine-specific paths.

## Search

The catalog answers the same listing shape as every other source in the
[Parts Catalog](library-sources.md#the-parts-catalog), so its results sit beside
bundled and local ones.

- The catalog declares which filters it supports. A filter it cannot apply is
  reported, and a partial or paged result is never shown as complete.
- When the catalog is unreachable or needs sign-in, the source shows that status
  and its reason. Other sources keep searching normally.
- Search is a shared engine command, so MCP sees the same results as the GUI.
  Searching never changes the open project.

## Details

Selecting a result fetches the full definition of that exact revision.

- Details show the same content as for local items: drawing and footprint
  previews, terminals, pads and pin map, license, provenance, available revisions
  and validation errors.
- Price and stock may appear in the same view, but labeled as provider data
  with its date (see [below](#price-and-stock-in-dated-provider-records)). They
  are never part of the definition.
- Fetching details is read-only. Only **Place**, through the existing undoable
  placement command, changes the project. **Add to project** (pin without
  placing) is the planned action from the
  [Parts Catalog](library-sources.md#the-parts-catalog) design and would also be
  one undoable command.

## Authentication

- **Per person.** Credentials belong to the person, not the project. On desktop
  they are kept in the system keychain.
- **Never saved with data.** Credentials are never written to project files,
  library files, bundles, logs, diagnostics or MCP results.
- **Offline first.** Offline mode is checked before any connection is made.
- **Clear failures.** Missing or rejected credentials put the source in a
  *needs sign-in* state. They do not hide pinned items, change the project or
  block editing.
- **One session for GUI and MCP.** MCP commands use the person's existing
  sign-in. They never take or return credentials.

## Offline reopen

[Pinning](library-sources.md#pinning-into-a-project) already copies each exact
definition into the project with its provenance. A catalog item is pinned the
same way when it is placed, or when it is added through the planned **Add to
project** action.

- A project opens, edits and saves with the catalog unreachable, signed out or
  unregistered. Its pinned items keep their previews and are marked as from an
  unavailable source.
- A no-op save keeps the same bytes, whether or not the catalog is reachable.
- A local cache of fetched items may speed up browsing. The cache never changes
  the project, never marks it dirty and is never needed to open it.
- A newer catalog revision never replaces a pinned one by itself. Discovering
  newer revisions (`library.available_revisions`) and a read-only footprint pad
  preview exist today; see
  [Query available revisions](../libraries/native-library-sources.md#query-available-revisions).
  [Reviewed text-only adoption](https://github.com/tangency-labs/pcbcraft/pull/526)
  lets users preview a loaded symbol or generic-part revision and apply the
  reviewed changes to selected instances in one undoable edit. Broader
  compatible revision adoption remains tracked in
  [#312](https://github.com/tangency-labs/pcbcraft/issues/312).

## Pinned engineering definitions

Projects pin the engineering definitions they use: symbols, footprints, generic
parts and their package variants, manufacturer parts and category schemas. These
define the design. A catalog change, a sign-out or a removed registration never
changes what a saved project means.

## Price and stock in dated provider records

Price, stock, lead time, packaging, minimum order quantity and lifecycle status
are provider records, not library definitions. A provider record can come from
the team catalog or from a separate provider; the rules are the same.

Lifecycle status here is the provider's dated report, such as active or end of
life. It is distinct from a lifecycle assessment that a project or library
publisher deliberately adopts. An adopted assessment is not a provider record,
and a provider refresh never changes it.

- **Dated.** Each record names its provider, the manufacturer part it describes
  and when it was retrieved. Prices carry their currency.
- **Independent refresh.** A refresh replaces provider records only. It never
  makes a new definition revision, never changes a chosen MPN and never marks
  the project dirty.
- **Not saved by default.** Projects do not store provider records. Keeping a
  dated quote is a separate, deliberate action, stored apart from the pinned
  definitions.
- **Shown with their date.** Older records are shown as old, never as current. A
  missing record means unknown, never zero stock or a zero price.

## Acceptance for the first slice

- **Search:** search the catalog from the Parts Catalog and MCP and get the same
  results. A filter the catalog does not support is reported, not ignored.
- **Details:** open one exact revision and see its previews, pin map, license
  and provenance. An invalid item is reported, and its neighbors stay listed.
- **Authentication:** sign in, sign out and use a rejected credential. The
  source shows *needs sign-in*, and no credential appears in the project, logs
  or MCP output.
- **Offline reopen:** place a catalog part, save, then reopen with the catalog
  unreachable. The project opens, edits and saves, and a no-op save keeps its
  bytes.
- **Provider records:** refresh price and stock for a placed part. The project
  stays clean, the pinned definitions are unchanged, and the shown values carry
  their retrieval date.

## Open questions

- **Transport.** Should the catalog be a static, signed bundle served over HTTPS,
  or a query service that searches on the server?
- **Browser sign-in.** Where, if anywhere, should the browser build keep a
  credential between sessions?
- **Staleness.** When should a provider record be shown as old, and should the
  provider be able to say so?
