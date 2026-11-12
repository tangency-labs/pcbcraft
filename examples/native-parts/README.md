# Native part definitions

This original MIT OR Apache-2.0 library contains resistor, connector and IC part definitions with typed
categories and explicit symbol/category bindings. The resistor has a 0603
package; the connector and IC demonstrate valid schematic-stage definitions
without assigned packages. It derives its drawings and land pattern from this repository's
starter library, retaining the footprint's recorded provenance.

Open **Libraries → Sources → Add folder…** and select this directory. The
catalog's Parts and Categories filters show the new records. This demonstrates
library ingestion, validation and part placement, including optional package and
manufacturer selection. The Manufacturer parts filter exposes a synthetic
manufacturer/MPN record; it represents no real manufacturer product.
To upload the same library in the browser:

```sh
python3 scripts/package-native-library.py examples/native-parts /tmp/native-parts.pclib.json
```

For shared-command inspection, load that bundle's `documents` map with
`library.load_source`, then call:

```json
{"command":"library.resolve_part","params":{"library":"parts-demo","definition":"resistor","definition_revision":1}}
```

The result contains the exact part, symbol, category and package footprints.
It validates the complete pin map without placing or updating a project. The
same session can normalize input with:

```json
{"command":"library.parse_quantity","params":{"quantity":"resistance","value":"100 mΩ"}}
```

The canonical result is `{"type":"quantity","value":"0.1","unit":"Ohm"}`.
Moving the library folder retains identities. Removing the source makes its
unpersisted catalog definitions unavailable. Placed native parts retain their
part, symbol, category, all package choices and selected manufacturer record in
the project. Reopen the project with no source registrations to verify offline
editing. The resistor Value accepts `100 mΩ`, and part properties can set a
maximum tolerance requirement against the synthetic 1% specification.

For command placement, use the returned part `digest` as `definition_digest`
with `schematic.place_part`, the exact library/id/revision, coordinates and
`expected_revision`. Choosing the optional synthetic MPN also requires its exact
reference and digest from `library.search` with `kind: "manufacturer_part"`.
The same selection is used by the catalog's click, keyboard and drag actions.
