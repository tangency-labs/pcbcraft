"""Rendered comparisons reject incomplete captures and changed rendering conditions."""
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("rendered", Path(__file__).with_name("compare-rendered-performance.py"))
rendered = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rendered)


class RenderedTests(unittest.TestCase):
    def setUp(self):
        self.report = dict(schema="pcbcraft-rendered-performance-report/1", target="native-rendered", renderer="test GPU",
                           input_policy="atomic pairs", idle_between_iterations_ms=0, iterations=2, captures=2, image_size=[1400, 900], last_image_sha256="a" * 64,
                           debug_assertions=False, artifact_sha256="fixture", project_sha256="project", recipe="reference-v1",
                           trace_id="trace-v1", warmup_frames=8, unmeasured=["physical presentation"],
                           environment=dict(revision="before", hardware="host", os="linux", runtime="native", compiler="rust", profile="release", power="unknown"),
                           metrics=[dict(id="rendered.reverse.input_to_gpu_readback", samples_ms=[10, 20], p50_ms=10, p95_ms=20, p99_ms=20, max_ms=20)])

    def test_scope_and_tail_are_preserved(self):
        after = copy.deepcopy(self.report)
        after["environment"]["revision"] = "after"
        text = rendered.compare(self.report, after)
        self.assertIn("before → after", text)
        self.assertIn("do not establish", text)
        self.assertIn("20.000000", text)

    def test_partial_or_unrendered_evidence_is_rejected(self):
        for key, value in [("captures", 1), ("image_size", [0, 900]), ("last_image_sha256", ""), ("target", "native-cpu")]:
            with self.subTest(key=key):
                other = copy.deepcopy(self.report)
                other[key] = value
                with self.assertRaises(ValueError):
                    rendered.validate(other)

    def test_window_occlusion_changes_are_not_silently_compared(self):
        after = copy.deepcopy(self.report)
        after["window_observations"] = [dict(at_ms=1, focused=False, visible=False, minimized=False, occluded=True)]
        with self.assertRaises(ValueError):
            rendered.compare(self.report, after)

    def test_gpu_viewport_and_clock_environment_changes_are_not_comparable(self):
        for key, value in [("renderer", "different GPU"), ("image_size", [900, 900]), ("artifact_sha256", "other fixture")]:
            other = copy.deepcopy(self.report)
            other[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                rendered.compare(self.report, other)


if __name__ == "__main__":
    unittest.main()
