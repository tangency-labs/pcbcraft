"""Exercise source-count comparison through the standalone reader executable."""

import json
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory
import unittest
from zipfile import ZipFile


SCRIPTS = Path(__file__).resolve().parent
FIXTURES = SCRIPTS.parent / "crates/storage/tests/fixtures/native-open"
SCRIPT = SCRIPTS / "read-flat-project-netlist.py"
ARCHIVE = FIXTURES / "dual-led-editing-demo-v3.pc"
COUNTS = FIXTURES / "dual-led-editing-demo-source-counts.json"


def run_reader(folder, counts):
    """Run the documented CLI against the unpacked fixture and separate source oracle."""
    return subprocess.run(
        [sys.executable, "-B", str(SCRIPT), str(folder), "--expected-counts", str(counts)],
        capture_output=True, text=True, check=False, timeout=10,
    )


class FlatProjectSourceCounts(unittest.TestCase):
    def test_matches_source_counts_without_changing_inputs(self):
        with TemporaryDirectory(prefix="pcbcraft source counts ") as directory:
            folder = Path(directory)
            with ZipFile(ARCHIVE) as archive:
                archive.extractall(folder)
            before = {path: path.read_bytes() for path in folder.rglob("*") if path.is_file()}
            counts_before = COUNTS.read_bytes()
            result = run_reader(folder, COUNTS)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stderr, "")
            self.assertEqual(json.loads(result.stdout), json.loads(counts_before))
            self.assertEqual({path: path.read_bytes() for path in folder.rglob("*") if path.is_file()}, before)
            self.assertEqual(COUNTS.read_bytes(), counts_before)

    def test_reports_each_mismatch_and_rejects_invalid_count_files(self):
        with TemporaryDirectory(prefix="pcbcraft source counts ") as directory:
            root = Path(directory)
            folder = root / "project"
            with ZipFile(ARCHIVE) as archive:
                archive.extractall(folder)
            source_counts = root / "counts.json"
            expected = json.loads(COUNTS.read_bytes())
            for key, count in expected.items():
                with self.subTest(mismatch=key):
                    source_counts.write_text(json.dumps(dict(expected, **{key: count + 1})), encoding="utf-8")
                    result = run_reader(folder, source_counts)
                    self.assertEqual(result.returncode, 1)
                    self.assertEqual(result.stdout, "")
                    self.assertIn(f"count mismatch: {key}: expected {count + 1}, got {count}", result.stderr)
            invalid = (
                "[]", "{}", '{"parts":7,"pins":15}', '{"parts":7,"pins":15,"nets":4,"extra":0}',
                json.dumps(dict(expected, pins=True)), json.dumps(dict(expected, pins=-1)),
                json.dumps(dict(expected, pins=15.0)), json.dumps(dict(expected, pins="15")),
                '{"parts":6,"parts":7,"pins":15,"nets":4}',
            )
            for text in invalid:
                with self.subTest(invalid=text):
                    source_counts.write_text(text, encoding="utf-8")
                    result = run_reader(folder, source_counts)
                    self.assertEqual(result.returncode, 1)
                    self.assertEqual(result.stdout, "")
                    self.assertIn("expected counts", result.stderr)
            source_counts.unlink()
            result = run_reader(folder, source_counts)
            self.assertEqual(result.returncode, 1)
            self.assertEqual(result.stdout, "")
            self.assertIn(source_counts.name, result.stderr)
