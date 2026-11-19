"""Check the independent reader against the projection used by the engine test."""

import hashlib
import json
from pathlib import Path
import runpy
import unittest
from zipfile import ZipFile


SCRIPTS = Path(__file__).resolve().parent
FIXTURES = SCRIPTS.parent / "crates/storage/tests/fixtures/native-open"
READER = runpy.run_path(str(SCRIPTS / "read-flat-project-netlist.py"))["read_netlist"]


class FlatProjectNetlist(unittest.TestCase):
    def test_frozen_connectivity_matches_engine_oracle(self):
        archive_path = FIXTURES / "dual-led-editing-demo-v3.pc"
        self.assertEqual(hashlib.sha256(archive_path.read_bytes()).hexdigest(),
                         "20761598da99b3cb688aa57d16cc6f2be71a06869be5446b05f7a26325ceb46c")
        expected = json.loads((FIXTURES / "dual-led-editing-demo-connectivity.json").read_text(encoding="utf-8"))
        with ZipFile(archive_path) as archive:
            actual = READER(lambda path: json.loads(archive.read(path)))
        self.assertEqual(json.loads(json.dumps(actual)), expected)

    def test_refuses_other_project_envelopes(self):
        with self.assertRaisesRegex(ValueError, "expected project/6 with exactly one sheet"):
            READER(lambda _: {"format": "pcbcraft-project/7", "sheets": [{}]})

    def test_refuses_an_endpoint_it_cannot_interpret(self):
        with ZipFile(FIXTURES / "dual-led-editing-demo-v3.pc") as archive:
            def read(path):
                document = json.loads(archive.read(path))
                if path.startswith("sheets/"):
                    document["wires"][0]["to"] = {"junction": -1}
                return document

            with self.assertRaisesRegex(ValueError, "unsupported or unresolved wire endpoint"):
                READER(read)

    def test_refuses_duplicate_logical_terminal_and_unit_pin(self):
        for field in ("terminals", "pins"):
            with self.subTest(field=field), ZipFile(FIXTURES / "dual-led-editing-demo-v3.pc") as archive:
                def read(path):
                    document = json.loads(archive.read(path))
                    if path.startswith("library/symbols/"):
                        records = document["terminals"] if field == "terminals" else document["units"][0]["pins"]
                        records.append(records[0])
                    return document

                with self.assertRaisesRegex(ValueError, "expected each logical terminal to appear once"):
                    READER(read)

    def test_refuses_duplicate_component_and_junction_identities(self):
        cases = (("components", "project.json", "duplicate component ID"),
                 ("junctions", "sheets/", "duplicate junction ID"))
        for field, prefix, message in cases:
            with self.subTest(field=field), ZipFile(FIXTURES / "dual-led-editing-demo-v3.pc") as archive:
                def read(path):
                    document = json.loads(archive.read(path))
                    if path.startswith(prefix):
                        document[field].append(document[field][0])
                    return document

                with self.assertRaisesRegex(ValueError, message):
                    READER(read)
