# Native part definitions

This original MIT OR Apache-2.0 library contains resistor, connector and IC part definitions with typed
categories and explicit symbol/category bindings. The resistor has a 0603
package; the connector and IC demonstrate valid schematic-stage definitions
without assigned packages. It derives its drawings and land pattern from this repository's
starter library, retaining the footprint's recorded provenance.

Open **Libraries → Sources → Add folder…** and select this directory. The
catalog's Parts and Categories filters show the new records. This demonstrates
library ingestion and validation; ordinary schematic placement still uses symbols.
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
unpersisted catalog definitions unavailable; this example does not claim that
part records have already been pinned into a project. Existing placed symbols
continue to use their ordinary pinned copies.
