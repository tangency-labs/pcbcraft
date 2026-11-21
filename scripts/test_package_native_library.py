"""The transfer packager keeps every native source directory, including lifecycle assertions."""
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("native_library_packager", ROOT / "scripts/package-native-library.py")
packager = importlib.util.module_from_spec(spec)
spec.loader.exec_module(packager)


class PackageNativeLibraryTests(unittest.TestCase):
    """Package a small flat source folder and read back the bundle's exact document set."""

    def test_lifecycle_assertions_are_packaged_with_items(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "library.json").write_text('{"format":"pcbcraft-library/1"}\n')
            for path, text in [("symbols/resistor.json", '{"symbol":1}\n'),
                               ("lifecycle-assertions/review.r1.json", '{"assertion":1}\n')]:
                (root / path).parent.mkdir(exist_ok=True)
                (root / path).write_text(text)
            bundle = json.loads(packager.bundle(root))
            self.assertEqual(bundle["format"], "pcbcraft-library-bundle/1")
            self.assertEqual(sorted(bundle["documents"]),
                             ["library.json", "lifecycle-assertions/review.r1.json", "symbols/resistor.json"])
            self.assertEqual(bundle["documents"]["lifecycle-assertions/review.r1.json"], '{"assertion":1}\n')

    def test_packaged_directories_match_the_schema_checker(self):
        spec = importlib.util.spec_from_file_location("library_schema_directories", ROOT / "scripts/check-library-schemas.py")
        checker = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(checker)
        self.assertEqual(set(packager.DIRECTORIES), set(checker.DIRECTORIES))


if __name__ == "__main__":
    unittest.main()
