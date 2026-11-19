"""Evidence-validation regressions, independent of machine-dependent timing thresholds."""
import copy
import gzip
import hashlib
import json
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("compare_performance", Path(__file__).with_name("compare-performance.py"))
comparison = importlib.util.module_from_spec(spec)
spec.loader.exec_module(comparison)


class ComparisonTests(unittest.TestCase):
    def setUp(self):
        self.report = dict(schema="pcbcraft-performance-report/1", target="native-cpu", debug_assertions=False,
                           artifact_sha256="artifact", project_sha256="project", recipe="reference-v1", trace_id="trace-v1",
                           iterations=3, warmup_frames=8, geometry={}, unmeasured=["presentation"],
                           environment=dict(revision="before", hardware="cpu", os="linux", runtime="native", compiler="rust", profile="release", power="AC", notes=""),
                           metrics=[dict(id="replay.move.ui", samples_ms=[1, 2, 5], p50_ms=2, p95_ms=5, p99_ms=5, max_ms=5)])

    def test_saved_evidence_retains_exact_fixture_and_sample_identity(self):
        root = Path(__file__).resolve().parents[1] / "tools" / "performance"
        for name in ["native-reference", "native-dense", "native-capacity-smoke", "browser-reference"]:
            with self.subTest(report=name):
                report = json.loads((root / "baselines" / f"{name}.json").read_text())
                comparison.validate(report)
                fixture = root / "fixtures" / f"{report['recipe']}.json"
                data = fixture.read_bytes() if fixture.exists() else gzip.decompress(fixture.with_suffix(".json.gz").read_bytes())
                self.assertEqual(hashlib.sha256(data).hexdigest(), report["artifact_sha256"])
                artifact = json.loads(data)
                self.assertEqual(hashlib.sha256(bytes(artifact["project"])).hexdigest(), artifact["project_sha256"])
                self.assertEqual(report["project_sha256"], artifact["project_sha256"])

    def test_rejects_malformed_report_shapes(self):
        for report in [None, [], {"schema": "pcbcraft-performance-report/1", "iterations": True}]:
            with self.subTest(report=report), self.assertRaises(ValueError):
                comparison.validate(report)

    def test_comparison_preserves_slow_tail_and_scope(self):
        after = copy.deepcopy(self.report)
        after["environment"]["revision"] = "after"
        result = comparison.compare(self.report, after)
        self.assertIn("before → after", result)
        self.assertIn("CPU frame p95 ≥ 4 ms: replay.move.ui", result)
        self.assertIn("cannot certify", result)

    def test_regenerated_bga_metrics_are_not_compared_as_artifact_metrics(self):
        before = copy.deepcopy(self.report)
        before["recipe"] = "bga-v1"
        before["metrics"].append(dict(before["metrics"][0], id="bga.library_load"))
        after = copy.deepcopy(before)
        after["metrics"][1].update(samples_ms=[10, 20, 50], p50_ms=20, p95_ms=50, p99_ms=50, max_ms=50)
        after["metrics"].append(dict(after["metrics"][0], id="bga.place_part"))
        result = comparison.compare(before, after)
        self.assertIn("Excluded bga.* boundaries", result)
        self.assertNotIn("| bga.library_load |", result)
        self.assertNotIn("| bga.place_part |", result)
        self.assertIn("| replay.move.ui |", result)
        after["metrics"][0]["id"] = "replay.other.ui"
        with self.assertRaisesRegex(ValueError, "measurement boundaries differ"):
            comparison.compare(before, after)
        del before["metrics"][0], after["metrics"][0]
        with self.assertRaisesRegex(ValueError, "no comparable fixed-artifact metrics"):
            comparison.compare(before, after)

    def test_rejects_regenerated_artifacts_or_environment_changes(self):
        for key in ["artifact_sha256", "target", "iterations", "warmup_frames"]:
            with self.subTest(key=key):
                other = copy.deepcopy(self.report)
                other[key] = 4 if isinstance(other[key], int) else "different"
                with self.assertRaises(ValueError):
                    comparison.compare(self.report, other)
        other = copy.deepcopy(self.report)
        other["environment"]["runtime"] = "different browser"
        with self.assertRaises(ValueError):
            comparison.compare(self.report, other)

    def test_rejects_missing_samples_bad_clocks_and_fabricated_quantiles(self):
        for samples in [[1, 2], [1, 2, float("nan")], [1, 2, -1], [1, 2, True], [1, 2, 3]]:
            with self.subTest(samples=samples):
                other = copy.deepcopy(self.report)
                other["metrics"][0]["samples_ms"] = samples
                with self.assertRaises(ValueError):
                    comparison.validate(other)


if __name__ == "__main__":
    unittest.main()
