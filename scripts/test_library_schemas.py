"""Published library schemas cover real native records and reject lossy/ambiguous encodings."""
import copy
import importlib.util
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("library_schema_checker", ROOT / "scripts/check-library-schemas.py")
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


class LibrarySchemaTests(unittest.TestCase):
    """Exercise actual fixtures, version boundaries and canonical numeric values."""

    @classmethod
    def setUpClass(cls):
        cls.checks = checker.validators()
        cls.common = json.loads((ROOT / "docs/schemas/library-common-v1.schema.json").read_text())
        cls.part = checker.read_json(ROOT / "examples/native-parts/parts/resistor.json")
        cls.category = checker.read_json(ROOT / "examples/native-parts/categories/resistor.json")
        cls.symbol = checker.read_json(ROOT / "examples/native-parts/symbols/resistor.json")

    def test_bundled_and_example_records_validate_offline(self):
        checker.validate([], [])

    def test_part_keeps_drawing_manufacturer_and_instance_ownership_separate(self):
        validator = self.checks[checker.FORMATS["pcbcraft-part/1"]]
        validator.validate(self.part)
        for key, value in [("manufacturer", "Vendor"), ("mpn", "ABC"), ("reference", "R1"),
                           ("pads", []), ("drawing", []), ("stock", 100), ("price", 1)]:
            with self.subTest(key=key):
                changed = copy.deepcopy(self.part)
                changed[key] = value
                self.assertFalse(validator.is_valid(changed))
        uncategorized = copy.deepcopy(self.part)
        del uncategorized["category"]
        uncategorized["packages"] = {}
        validator.validate(uncategorized)
        uncategorized["fixed_parameters"] = {"pcbcraft:resistance": {"type": "quantity", "value": "0.1", "unit": "Ohm"}}
        self.assertFalse(validator.is_valid(uncategorized))

    def test_manufacturer_metadata_and_instance_requirements(self):
        manufacturer = checker.read_json(ROOT / "examples/native-parts/manufacturer-parts/test-resistor-100m.json")
        validator = self.checks[checker.FORMATS["pcbcraft-manufacturer-part/1"]]
        validator.validate(manufacturer)
        for field, value in [("stock", 10), ("price", 1), ("datasheet", "file:///tmp/test.pdf")]:
            changed = copy.deepcopy(manufacturer)
            changed[field] = value
            self.assertFalse(validator.is_valid(changed))
        selection = {"definition": manufacturer["part"], "package": "0603", "requirements": {
            "pcbcraft:resistance": {"operator": "range", "minimum": {"type": "quantity", "value": "0.1", "unit": "Ohm"},
                                    "maximum": {"type": "quantity", "value": "100", "unit": "Ohm"}}}}
        validator = self.checks["urn:pcbcraft:schema:part-selection:15"]
        validator.validate(selection)
        selection["requirements"]["pcbcraft:resistance"]["minimum"]["unit"] = "mOhm"
        self.assertFalse(validator.is_valid(selection))

    def test_multipad_placement_requires_extended_vocabulary(self):
        placement = {"id": 1, "component": 2, "library": "parts-demo", "definition": "test", "definition_revision": 1,
                     "x_nm": 0, "y_nm": 0, "mapping": {"1": ["1", "EP"], "2": "2"}, "unconnected_pads": ["MH"]}
        self.checks["urn:pcbcraft:schema:footprint-placement:17"].validate(placement)
        self.assertFalse(self.checks["urn:pcbcraft:schema:footprint-placement:15"].is_valid(placement))
        placement["mapping"]["1"] = ["1", "1"]
        self.assertFalse(self.checks["urn:pcbcraft:schema:footprint-placement:17"].is_valid(placement))

    def test_discovered_records_must_match_their_directory(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary)
            path = source / "parts" / "wrong-kind.json"
            path.parent.mkdir()
            path.write_text(json.dumps(self.category))
            with self.assertRaisesRegex(ValueError, "does not belong in parts/"):
                checker.validate([], [source])
            checker.validate([path], [])

    def test_symbol_binding_is_explicit_and_versioned(self):
        validator = self.checks[checker.FORMATS["pcbcraft-symbol/3"]]
        validator.validate(self.symbol)
        missing = copy.deepcopy(self.symbol)
        del missing["category"]
        self.assertFalse(validator.is_valid(missing))
        for version in (1, 2):
            old = copy.deepcopy(self.symbol)
            old["format"] = f"pcbcraft-symbol/{version}"
            self.assertFalse(self.checks[checker.FORMATS[old["format"]]].is_valid(old))
            del old["category"]
            self.checks[checker.FORMATS[old["format"]]].validate(old)

    def test_multiunit_extension_is_versioned_and_bounded(self):
        symbol = checker.read_json(ROOT / "examples/native-parts/symbols/dual-amplifier.json")
        self.checks[checker.FORMATS["pcbcraft-symbol/4"]].validate(symbol)
        self.checks["urn:pcbcraft:schema:symbol"].validate(symbol)
        for version in (1, 2, 3):
            changed = copy.deepcopy(symbol)
            changed["format"] = f"pcbcraft-symbol/{version}"
            self.assertFalse(self.checks[checker.FORMATS[changed["format"]]].is_valid(changed))
        for count in (0, 33):
            changed = copy.deepcopy(symbol)
            changed["units"] = [symbol["units"][0]] * count
            self.assertFalse(self.checks[checker.FORMATS["pcbcraft-symbol/4"]].is_valid(changed))

    def test_alternate_pin_functions_are_versioned_and_bounded(self):
        symbol = checker.read_json(ROOT / "examples/native-parts/symbols/dual-amplifier.json")
        symbol["format"] = "pcbcraft-symbol/5"
        validator = self.checks["urn:pcbcraft:schema:symbol"]
        terminal = symbol["terminals"][0]
        validator.validate(symbol)
        terminal["alternates"] = [{"name": "ADC_IN", "electrical_type": "input"}]
        validator.validate(symbol)
        for invalid in ([], None, [{"name": "ADC_IN", "electrical_type": "unknown"}],
                        [{"name": str(i), "electrical_type": "input"} for i in range(17)]):
            broken = copy.deepcopy(symbol)
            broken["terminals"][0]["alternates"] = invalid
            self.assertFalse(validator.is_valid(broken))
        large = copy.deepcopy(symbol)
        count = len(large["terminals"])
        large["terminals"].extend({"id": f"T{i}", "electrical_type": "input"} for i in range(count, 1024))
        validator.validate(large)
        large["terminals"].append({"id": "T1024", "electrical_type": "input"})
        self.assertFalse(validator.is_valid(large))
        symbol["format"] = "pcbcraft-symbol/4"
        self.assertFalse(validator.is_valid(symbol))

    def test_units_are_canonical_and_bound_to_the_declared_quantity(self):
        validator = self.checks[checker.FORMATS["pcbcraft-category/1"]]
        for unit in ["mOhm", "Ω", "V", "ohms"]:
            changed = copy.deepcopy(self.category)
            changed["fields"]["pcbcraft:resistance"]["value_type"]["unit"] = unit
            self.assertFalse(validator.is_valid(changed))
        common = self.checks["urn:pcbcraft:schema:library-common:1"]
        value_validator = common.evolve(schema={"$ref": "urn:pcbcraft:schema:library-common:1#/$defs/parameter_value"})
        value_validator.validate({"type": "quantity", "value": "0.1", "unit": "Ohm"})
        for value in [{"type": "quantity", "value": 0.1, "unit": "Ohm"},
                      {"type": "quantity", "value": "100", "unit": "mOhm"},
                      {"type": "quantity", "value": "0.10", "unit": "Ohm"},
                      {"type": "quantity", "value": "1e-1", "unit": "Ohm"},
                      {"type": "quantity", "value": "-0", "unit": "Ohm"}]:
            self.assertFalse(value_validator.is_valid(value), value)

    def test_optional_drawings_are_versioned_and_pin_maps_require_a_symbol(self):
        """Format 2 permits independent drawings without weakening the retained format 1."""
        validator = self.checks[checker.FORMATS["pcbcraft-part/2"]]
        part = copy.deepcopy(self.part)
        part["format"] = "pcbcraft-part/2"
        validator.validate(part)
        del part["symbol"]
        self.assertFalse(validator.is_valid(part))
        for package in part["packages"].values():
            package["mapping"] = {}
            package.pop("unconnected_pads", None)
        validator.validate(part)
        part["packages"] = {}
        validator.validate(part)
        part["symbol"] = None
        self.assertFalse(validator.is_valid(part))
        del part["symbol"]
        part["format"] = "pcbcraft-part/1"
        self.assertFalse(self.checks[checker.FORMATS["pcbcraft-part/1"]].is_valid(part))

    def test_pin_identifiers_and_mapping_are_preserved(self):
        validator = self.checks[checker.FORMATS["pcbcraft-part/1"]]
        value = copy.deepcopy(self.part)
        package = value["packages"]["0603"]
        package["mapping"]["1"] = ["01", "EP"]
        package["unconnected_pads"] = ["SHIELD"]
        validator.validate(value)
        for pads in [[], ["01", "01"], ["01\n"]]:
            package["mapping"]["1"] = pads
            self.assertFalse(validator.is_valid(value))

    def test_duplicate_keys_are_rejected_before_validation(self):
        with self.assertRaisesRegex(ValueError, "Duplicate JSON key"):
            json.loads('{"field":1,"field":2}', object_pairs_hook=checker.distinct_object)

    @unittest.skipUnless(shutil.which("node"), "Node.js is needed for cross-language regex validation")
    def test_decimal_patterns_agree_in_python_and_javascript(self):
        common = self.checks["urn:pcbcraft:schema:library-common:1"]
        validator = common.evolve(schema=self.common["$defs"]["decimal"])
        pattern = self.common["$defs"]["decimal"]["pattern"]
        cases = []
        for value in ["0", "0.1", "-0.1", "1", "1" * 36, "0." + "0" * 17 + "1",
                      "00", "1.0", "1e3", "1\n", "1" * 37, "0." + "0" * 18 + "1"]:
            cases.append({"pattern": pattern, "text": value, "expected": validator.is_valid(value)})
        program = "const cases=JSON.parse(require('fs').readFileSync(0,'utf8'));for(const c of cases){if(new RegExp(c.pattern).test(c.text)!==c.expected)throw new Error(JSON.stringify(c));}"
        subprocess.run(["node", "-e", program], input=json.dumps(cases), text=True, check=True, capture_output=True)

    def test_missing_checker_dependency_has_an_installation_hint(self):
        result = subprocess.run([sys.executable, "-S", str(ROOT / "scripts/check-library-schemas.py")], capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        self.assertIn('pip install "jsonschema>=4.18"', result.stderr)

    def part5(self):
        """A valid /5 record built from the original /1 example, with one of each metadata field."""
        value = copy.deepcopy(self.part)
        value["format"] = "pcbcraft-part/5"
        value["description"] = "Precision thin-film resistor"
        value["keywords"] = ["precision", "thin film"]
        value["links"] = [{"label": "Datasheet", "url": "https://example.invalid/ds.pdf"}]
        return value

    def test_part5_accepts_metadata_with_and_without_ratings(self):
        self.assertIn("pcbcraft-part/5", checker.FORMATS)
        part = self.part5()
        validator = self.checks[checker.FORMATS["pcbcraft-part/5"]]
        validator.validate(part)
        part["terminal_ratings"] = {"1": {"supply_min": {"type": "quantity", "value": "4.5", "unit": "V"}}}
        validator.validate(part)
        # A metadata-only part needs no symbol, but its packages then carry no terminal map.
        del part["terminal_ratings"]
        del part["symbol"]
        for package in part["packages"].values():
            package["mapping"] = {}
            package.pop("unconnected_pads", None)
        validator.validate(part)
        # Ratings still require the exact symbol they belong to.
        part["terminal_ratings"] = {"1": {"supply_max": {"type": "quantity", "value": "36", "unit": "V"}}}
        self.assertFalse(validator.is_valid(part))

    def test_current_part_accepts_omitted_ratings_and_metadata(self):
        validator = self.checks[checker.FORMATS["pcbcraft-part/6"]]
        part = self.part5()
        part["format"] = "pcbcraft-part/6"
        part["terminal_ratings"] = {"1": {"supply_min": {"type": "quantity", "value": "4.5", "unit": "V"}}}
        validator.validate(part)
        for field in ("description", "keywords", "links"):
            del part[field]
            validator.validate(part)
        invalid = copy.deepcopy(part)
        del invalid["symbol"]
        self.assertFalse(validator.is_valid(invalid))
        invalid["packages"] = {}
        self.assertFalse(validator.is_valid(invalid))
        del part["terminal_ratings"]
        validator.validate(part)
        del part["symbol"]
        self.assertFalse(validator.is_valid(part))
        part["packages"] = {}
        validator.validate(part)
        part["keywords"] = []
        self.assertFalse(validator.is_valid(part))

    def test_legacy_part_schemas_still_reject_all_metadata(self):
        for version in (1, 2, 4):
            old = copy.deepcopy(self.part5())
            old["format"] = f"pcbcraft-part/{version}"
            if version == 4:
                old["terminal_ratings"] = {"1": {"supply_min": {"type": "quantity", "value": "4.5", "unit": "V"}}}
            validator = self.checks[checker.FORMATS[old["format"]]]
            self.assertFalse(validator.is_valid(old), f"format {version} accepted metadata")
            for field in ("description", "keywords", "links"):
                del old[field]
            validator.validate(old)
            schema = checker.read_json(ROOT / f"docs/schemas/part-v{version}.schema.json")
            self.assertFalse(schema["additionalProperties"])
            for field in ("description", "keywords", "links"):
                self.assertNotIn(field, schema["properties"])

    def test_part5_bounds_and_metadata_ownership(self):
        part = self.part5()
        validator = self.checks[checker.FORMATS["pcbcraft-part/5"]]
        for field in ("description", "keywords", "links"):
            alone = copy.deepcopy(part)
            for other in ("description", "keywords", "links"):
                if other != field:
                    del alone[other]
            validator.validate(alone)
        empty = copy.deepcopy(part)
        for field in ("description", "keywords", "links"):
            del empty[field]
        self.assertFalse(validator.is_valid(empty))
        for field, value in [
            ("description", None), ("description", ""), ("description", "x" * 513),
            ("description", "line\nbreak"),
            ("keywords", None), ("keywords", []), ("keywords", [""]),
            ("keywords", ["x" * 65]), ("keywords", ["k" for _ in range(17)]),
            ("links", None), ("links", []),
            ("links", [{"label": "", "url": "https://example.invalid"}]),
            ("links", [{"label": "x" * 65, "url": "https://example.invalid"}]),
            ("links", [{"label": f"L{i}", "url": "https://example.invalid"} for i in range(9)]),
            ("links", [{"label": "Datasheet", "url": "https://example.invalid", "fetched": True}]),
        ]:
            with self.subTest(field=field, value=value):
                changed = copy.deepcopy(part)
                changed[field] = value
                self.assertFalse(validator.is_valid(changed))
        valid = copy.deepcopy(part)
        valid["description"] = "x" * 512
        valid["keywords"] = [f"k{i}" for i in range(16)]
        valid["links"] = [{"label": f"L{i}", "url": "https://example.invalid"} for i in range(8)]
        validator.validate(valid)

    def test_part5_link_urls_are_http_only_and_whitespace_free(self):
        part = self.part5()
        validator = self.checks[checker.FORMATS["pcbcraft-part/5"]]
        for url in ["ftp://example.invalid/x", "file:///tmp/x", "//example.invalid/x", "http://",
                    "https://?x", "https://example.invalid/a b", "https://exa mple.invalid",
                    "HTTP://example.invalid", "example.invalid", "x" * 2049]:
            with self.subTest(url=url):
                changed = copy.deepcopy(part)
                changed["links"] = [{"label": "Datasheet", "url": url}]
                self.assertFalse(validator.is_valid(changed))
        for url in ["http://example.invalid", "https://example.invalid/a?b=1#c"]:
            changed = copy.deepcopy(part)
            changed["links"] = [{"label": "Datasheet", "url": url}]
            validator.validate(changed)


if __name__ == "__main__":
    unittest.main()
