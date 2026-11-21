"""Check the independent reader against the projection used by the engine test."""

import hashlib
import json
from pathlib import Path
import runpy
import subprocess
import sys
from tempfile import TemporaryDirectory
import unittest
from zipfile import ZipFile


SCRIPTS = Path(__file__).resolve().parent
FIXTURES = SCRIPTS.parent / "crates/storage/tests/fixtures/native-open"
SCRIPT = SCRIPTS / "read-flat-project-netlist.py"
READER = runpy.run_path(str(SCRIPT))["read_netlist"]
ARCHIVE = FIXTURES / "dual-led-editing-demo-v3.pc"
ORACLE = FIXTURES / "dual-led-editing-demo-connectivity.json"


class FlatProjectNetlist(unittest.TestCase):
    def test_frozen_connectivity_matches_engine_oracle(self):
        self.assertEqual(hashlib.sha256(ARCHIVE.read_bytes()).hexdigest(),
                         "20761598da99b3cb688aa57d16cc6f2be71a06869be5446b05f7a26325ceb46c")
        expected = json.loads(ORACLE.read_text(encoding="utf-8"))
        with ZipFile(ARCHIVE) as archive:
            actual = READER(lambda path: json.loads(archive.read(path)))
        self.assertEqual(json.loads(json.dumps(actual)), expected)

    def test_cli_reads_unpacked_project_and_refuses_a_missing_sheet(self):
        expected = json.loads(ORACLE.read_text(encoding="utf-8"))
        with TemporaryDirectory(prefix="pcbcraft flat project ") as directory:
            folder = Path(directory)
            with ZipFile(ARCHIVE) as archive:
                archive.extractall(folder)
            command = [sys.executable, "-B", str(SCRIPT), str(folder)]
            result = subprocess.run(command, capture_output=True, text=True, check=False, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stderr, "")
            self.assertEqual(json.loads(result.stdout), expected)

            project = json.loads((folder / "project.json").read_text(encoding="utf-8"))
            missing = folder / project["sheets"][0]["file"]
            missing.unlink()
            result = subprocess.run(command, capture_output=True, text=True, check=False, timeout=10)
            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertEqual(result.stdout, "")
            self.assertIn(SCRIPT.name, result.stderr)
            self.assertIn(missing.name, result.stderr)
            self.assertNotIn("Traceback", result.stderr)

    def test_refuses_unsupported_documents_and_addresses(self):
        with ZipFile(ARCHIVE) as archive:
            project_path = "project.json"
            project = json.loads(archive.read(project_path))
            sheet_path = project["sheets"][0]["file"]
            sheet = json.loads(archive.read(sheet_path))
            component = project["components"][0]
            library, definition = component["library"], component["definition"]
            revision = component["definition_revision"]
            symbol_path = f"library/symbols/{library}/{definition}.r{revision}.json"
            symbol = json.loads(archive.read(symbol_path))
            project_error = "expected project/6 with exactly one sheet"
            reference_error = "invalid sheet reference"
            sheet_error = "expected the referenced sheet/1"
            address_error = "invalid pinned symbol address"
            definition_error = "expected a pinned, ordinary single-unit symbol/1"
            placement_error = "expected exactly one placement per part"
            cases = (
                ("project format", project_path, {"format": "pcbcraft-project/7"}, project_error),
                ("no sheets", project_path, {"sheets": []}, project_error),
                ("two sheets", project_path, {"sheets": project["sheets"] * 2}, project_error),
                ("wrong sheet path", project_path,
                 {("sheets", 0, "file"): "sheets/other.json"}, reference_error),
                ("non-UUID sheet with matching path", project_path,
                 {("sheets", 0, "id"): "other", ("sheets", 0, "file"): "sheets/other.json"}, reference_error),
                ("sheet format", sheet_path, {"format": "pcbcraft-sheet/2"}, sheet_error),
                ("sheet identity", sheet_path, {"id": "00000000-0000-0000-0000-000000000001"}, sheet_error),
                ("library segment", project_path, {("components", 0, "library"): "bad library"}, address_error),
                ("definition segment", project_path, {("components", 0, "definition"): "bad symbol"}, address_error),
                ("boolean revision", project_path, {("components", 0, "definition_revision"): True}, address_error),
                ("zero revision", project_path, {("components", 0, "definition_revision"): 0}, address_error),
                ("symbol format", symbol_path, {"format": "pcbcraft-symbol/2"}, definition_error),
                ("symbol library", symbol_path, {"library": "other"}, definition_error),
                ("symbol identity", symbol_path, {"id": "other"}, definition_error),
                ("symbol revision", symbol_path, {"revision": revision + 1}, definition_error),
                ("multi-unit symbol", symbol_path, {"units": symbol["units"] * 2}, definition_error),
                ("electrical behavior", symbol_path, {"behavior": {}}, definition_error),
                ("absent unit pin", symbol_path,
                 {("units", 0, "pins"): symbol["units"][0]["pins"][1:]},
                 "expected each logical terminal to appear once"),
                ("duplicate placement identity", sheet_path,
                 {("symbols", 0, "id"): sheet["symbols"][1]["id"]}, placement_error),
                ("unknown owner", sheet_path, {("symbols", 0, "component"): -1}, placement_error),
                ("duplicate owner", sheet_path,
                 {("symbols", 0, "component"): sheet["symbols"][1]["component"]}, placement_error),
                ("unknown unit", sheet_path, {("symbols", 0, "unit"): "unknown"}, "placement names an unknown unit"),
            )
            for label, target, changes, message in cases:
                with self.subTest(case=label):
                    def read(path):
                        document = json.loads(archive.read(path))
                        if path == target:
                            for keys, value in changes.items():
                                keys = (keys,) if isinstance(keys, str) else keys
                                record = document
                                for key in keys[:-1]:
                                    record = record[key]
                                record[keys[-1]] = value
                        return document

                    with self.assertRaisesRegex(ValueError, f"^{message}"):
                        READER(read)

    def test_refuses_an_endpoint_it_cannot_interpret(self):
        cases = (
            ("unknown junction", {"junction": -1}),
            ("null endpoint", None),
            ("list endpoint", [1]),
            ("float symbol endpoint", {"symbol": 2.0, "pin": "1"}),
            ("bool symbol endpoint", {"symbol": True, "pin": "1"}),
        )
        for label, bad_endpoint in cases:
            with self.subTest(case=label), ZipFile(ARCHIVE) as archive:
                def read(path):
                    document = json.loads(archive.read(path))
                    if path.startswith("sheets/"):
                        document["wires"][0]["to"] = bad_endpoint
                    return document

                with self.assertRaisesRegex(ValueError, "unsupported or unresolved wire endpoint"):
                    READER(read)

    def test_cli_refuses_null_endpoint_and_duplicate_keys_without_traceback(self):
        with TemporaryDirectory(prefix="pcbcraft flat project ") as directory:
            folder = Path(directory)
            with ZipFile(ARCHIVE) as archive:
                archive.extractall(folder)
            project = json.loads((folder / "project.json").read_text(encoding="utf-8"))
            sheet_file = folder / project["sheets"][0]["file"]
            original_sheet = sheet_file.read_text(encoding="utf-8")
            command = [sys.executable, "-B", str(SCRIPT), str(folder)]

            sheet = json.loads(original_sheet)
            sheet["wires"][0]["to"] = None
            sheet_file.write_text(json.dumps(sheet), encoding="utf-8")
            result = subprocess.run(command, capture_output=True, text=True, check=False, timeout=10)
            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertEqual(result.stdout, "")
            self.assertIn("unsupported or unresolved wire endpoint", result.stderr)
            self.assertNotIn("Traceback", result.stderr)

            sheet_file.write_text(original_sheet, encoding="utf-8")
            project_file = folder / "project.json"
            project_file.write_text(
                project_file.read_text(encoding="utf-8").replace('"format":', '"format":"pcbcraft-project/6","format":', 1),
                encoding="utf-8",
            )
            result = subprocess.run(command, capture_output=True, text=True, check=False, timeout=10)
            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertEqual(result.stdout, "")
            self.assertIn("duplicate keys", result.stderr)
            self.assertNotIn("Traceback", result.stderr)

    def test_refuses_non_integer_counter_ids(self):
        with ZipFile(ARCHIVE) as archive:
            project = json.loads(archive.read("project.json"))
            sheet_path = project["sheets"][0]["file"]
            sheet = json.loads(archive.read(sheet_path))
            cases = (
                ("float component id", "project.json", ("components", 0, "id"), float(project["components"][0]["id"]),
                 "expected integer component ID"),
                ("bool component id", "project.json", ("components", 0, "id"), True,
                 "expected integer component ID"),
                ("float symbol id", sheet_path, ("symbols", 0, "id"), float(sheet["symbols"][0]["id"]),
                 "expected integer symbol ID"),
                ("float symbol component id", sheet_path, ("symbols", 0, "component"), float(sheet["symbols"][0]["component"]),
                 "expected integer symbol component ID"),
                ("float junction id", sheet_path, ("junctions", 0, "id"), float(sheet["junctions"][0]["id"]),
                 "expected integer junction ID"),
            )
            for label, target, keys, value, message in cases:
                with self.subTest(case=label):
                    def read(path):
                        document = json.loads(archive.read(path))
                        if path == target:
                            record = document
                            for key in keys[:-1]:
                                record = record[key]
                            record[keys[-1]] = value
                        return document

                    with self.assertRaisesRegex(ValueError, f"^{message}"):
                        READER(read)

    def test_refuses_duplicate_logical_terminal_and_unit_pin(self):
        for field in ("terminals", "pins"):
            with self.subTest(field=field), ZipFile(ARCHIVE) as archive:
                def read(path):
                    document = json.loads(archive.read(path))
                    if path.startswith("library/symbols/"):
                        records = document["terminals"] if field == "terminals" else document["units"][0]["pins"]
                        records.append(records[0])
                    return document

                with self.assertRaisesRegex(ValueError, "expected each logical terminal to appear once"):
                    READER(read)

    def test_refuses_duplicate_part_and_junction_identities(self):
        cases = (("components", "project.json", "duplicate component ID"),
                 ("junctions", "sheets/", "duplicate junction ID"))
        for field, prefix, message in cases:
            with self.subTest(field=field), ZipFile(ARCHIVE) as archive:
                def read(path):
                    document = json.loads(archive.read(path))
                    if path.startswith(prefix):
                        document[field].append(document[field][0])
                    return document

                with self.assertRaisesRegex(ValueError, message):
                    READER(read)
