"""Retained extended evidence must keep its sources, artifacts and raw quantiles intact."""
import gzip
import hashlib
import importlib.util
import json
from pathlib import Path
import unittest
import zipfile


def load_module(filename):
    """Load the actual diagnostic validators without requiring an installed Python package."""
    spec = importlib.util.spec_from_file_location(filename, Path(__file__).with_name(filename))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


cpu = load_module("compare-performance.py")
rendered = load_module("compare-rendered-performance.py")
resources = load_module("collect-performance-resources.py")
root = Path(__file__).resolve().parents[1] / "tools" / "performance"
evidence = root / "baselines" / "extended"


def read_report(path):
    """Decode the saved gzip without changing report bytes or dropping raw samples."""
    return json.loads(gzip.decompress(path.read_bytes()))


class EvidenceTests(unittest.TestCase):
    def test_part_group_history_evidence_preserves_inputs_samples_and_summaries(self):
        """Retain exact checkout bytes and tie reported savings to the saved raw observations."""
        path = root / "baselines" / "part-groups"
        manifest = dict(line.split("  ", 1)[::-1] for line in (path / "SHA256SUMS").read_text().splitlines())
        self.assertEqual(set(manifest), {entry.name for entry in path.iterdir() if entry.name != "SHA256SUMS"})
        for name, digest in manifest.items():
            self.assertEqual(hashlib.sha256((path / name).read_bytes()).hexdigest(), digest, name)
        metadata = json.loads((path / "source-metadata.json").read_text())
        self.assertEqual(hashlib.sha256((path / "membership-memory.rs").read_bytes()).hexdigest(), metadata["runner_sha256"])
        self.assertEqual(hashlib.sha256(gzip.decompress((path / "fixture.pc.gz").read_bytes())).hexdigest(), metadata["fixture_sha256"])
        comparison = json.loads((path / "comparison.json").read_text())
        for label in ("before", "after"):
            with self.subTest(revision=label):
                report = read_report(path / f"{label}-resources.json.gz")
                timings = read_report(path / f"{label}-timings.json.gz")
                self.assertEqual(report["artifact_sha256"], metadata["fixture_sha256"])
                self.assertEqual(report["executable_sha256"], metadata[label]["executable_sha256"])
                values = sorted(timings["edit_ms"])
                self.assertEqual(len(values), timings["workload"]["edits"])
                self.assertEqual(len(values), 1000)
                self.assertTrue(all(value >= 0 for value in values))
                self.assertTrue(all(timings["correctness"].values()))
                summary = comparison["runs"][label]
                self.assertEqual(summary["correctness"], timings["correctness"])
                self.assertEqual(summary["edit_p50_ms"], values[(len(values) * 50 + 99) // 100 - 1])
                self.assertEqual(summary["edit_p95_ms"], values[(len(values) * 95 + 99) // 100 - 1])
                for field, source in (("observed_peak_rss_bytes", "VmRSS"), ("observed_high_water_bytes", "VmHWM")):
                    peak = max(sample[source] for sample in report["uninstrumented"]["rss_samples"])
                    self.assertEqual(report["uninstrumented"][field], peak)
                    self.assertEqual(summary[field], peak)

    def test_manifest_and_source_archives_match_the_recorded_bytes(self):
        manifest = json.loads((evidence / "manifest.json").read_text())
        self.assertEqual(set(manifest["files"]), {p.name for p in evidence.iterdir() if p.name != "manifest.json"})
        for name, expected in manifest["files"].items():
            with self.subTest(file=name):
                self.assertEqual(hashlib.sha256((evidence / name).read_bytes()).hexdigest(), expected)
        for source in evidence.glob("*-source.json"):
            stem = source.stem
            record = json.loads((evidence / f"{stem}.json").read_text())
            archive = evidence / f"{stem}.zip"
            self.assertEqual(hashlib.sha256(archive.read_bytes()).hexdigest(), record["source_archive_sha256"])
            with zipfile.ZipFile(archive) as files:
                self.assertEqual(set(files.namelist()), set(record["files"]))
                for name, expected in record["files"].items():
                    self.assertEqual(hashlib.sha256(files.read(name)).hexdigest(), expected)

    def test_raw_reports_match_the_canonical_inputs_and_quantiles(self):
        for path in evidence.glob("*.json.gz"):
            report = read_report(path)
            if report["schema"] == "pcbcraft-performance-resources/1":
                continue
            with self.subTest(report=path.name):
                validator = rendered if report["schema"] == "pcbcraft-rendered-performance-report/1" else cpu
                validator.validate(report)
                fixture = root / "fixtures" / f"{report['recipe']}.json"
                data = fixture.read_bytes() if fixture.exists() else gzip.decompress(fixture.with_suffix(".json.gz").read_bytes())
                self.assertEqual(hashlib.sha256(data).hexdigest(), report["artifact_sha256"])
                artifact = json.loads(data)
                self.assertEqual(hashlib.sha256(bytes(artifact["project"])).hexdigest(), report["project_sha256"])

    def test_capacity_counts_and_process_totals_have_real_evidence(self):
        board = read_report(evidence / "cpu-board-limits-smoke.json.gz")
        schematic = read_report(evidence / "cpu-schematic-limits.json.gz")
        self.assertEqual((board["geometry"]["tracks"], board["geometry"]["vias"]), (4096, 2048))
        self.assertEqual((schematic["geometry"]["symbols"], schematic["geometry"]["sheets"]), (4096, 64))
        self.assertEqual(board["iterations"], 1)
        self.assertEqual(schematic["iterations"], 100)
        report = read_report(evidence / "resources.json.gz")
        self.assertEqual(resources.allocation_totals(gzip.decompress((evidence / "valgrind.log.gz").read_bytes()).decode()), report["allocations"]["totals"])
        samples = report["uninstrumented"]["rss_samples"]
        self.assertEqual(max(x["VmRSS"] for x in samples), report["uninstrumented"]["observed_peak_rss_bytes"])
        self.assertEqual(max(x["VmHWM"] for x in samples), report["uninstrumented"]["observed_high_water_bytes"])
        browser = read_report(evidence / "browser-gl-reference.json.gz")
        self.assertEqual(browser["captures"], 4400)
        self.assertEqual(browser["iterations"], 100)
        native = read_report(evidence / "native-wayland-reference.json.gz")
        self.assertEqual((native["captures"], native["iterations"]), (4400, 100))
        transport = next(metric for metric in native["metrics"] if metric["id"] == "mcp.roundtrip")
        self.assertEqual(len(transport["samples_ms"]), 100)


if __name__ == "__main__":
    unittest.main()
