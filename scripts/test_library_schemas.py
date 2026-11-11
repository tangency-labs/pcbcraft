"""Published library schemas cover real native records and reject lossy/ambiguous encodings."""
import copy
import importlib.util
import json
import shutil
import subprocess
import sys
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


if __name__ == "__main__":
    unittest.main()
