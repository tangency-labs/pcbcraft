"""Resource evidence must come from actual profiler output, not absent-data zeros."""
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch
import tempfile
import sys

spec = importlib.util.spec_from_file_location("resources", Path(__file__).with_name("collect-performance-resources.py"))
resources = importlib.util.module_from_spec(spec)
spec.loader.exec_module(resources)


class ResourcesTests(unittest.TestCase):
    def test_allocation_totals_parse_thousands_and_reject_missing_evidence(self):
        self.assertEqual(resources.allocation_totals("==42== total heap usage: 1,024 allocs, 999 frees, 123,456 bytes allocated"),
                         dict(allocations=1024, frees=999, bytes_allocated=123456))
        with self.assertRaises(ValueError):
            resources.allocation_totals("profiling stopped before statistics")

    def test_missing_high_water_counter_does_not_become_zero(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(resources, "resident_memory", return_value={"VmRSS": 123}):
            result = resources.execute([sys.executable, "-c", "import time; time.sleep(0.05)"], Path(directory) / "run.log", 5)
        self.assertEqual(result["observed_peak_rss_bytes"], 123)
        self.assertIsNone(result["observed_high_water_bytes"])

    def test_missing_process_is_unavailable(self):
        self.assertEqual(resources.resident_memory(999999999), {})


if __name__ == "__main__":
    unittest.main()
