"""Independent analytic checks for the synthetic #244 stress generator."""
import importlib.util
import math
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("spatial_simulation", Path(__file__).with_name("simulate-spatial-obstacles.py"))
simulation = importlib.util.module_from_spec(spec)
spec.loader.exec_module(simulation)


class SpatialSimulationTests(unittest.TestCase):
    """Check the oracle and conservative subdivision without relying on a tree implementation."""

    def test_quarter_circle_includes_cardinals_and_excludes_empty_center(self):
        arc = ("arc", [0, 0, 10, 0, math.pi / 2], 1)
        self.assertEqual(simulation.envelope(arc), [-1, -1, 11, 11])
        self.assertEqual(simulation.distance(arc, 0, 0), 9)
        self.assertEqual(simulation.distance(arc, 10, 0), -1)
        self.assertAlmostEqual(simulation.distance(arc, -10, 0), math.sqrt(200) - 1)

    def test_split_bounds_cover_curve_points_across_wrap_and_cardinal_extrema(self):
        arc = ("arc", [123, -456, 5000, 7 * math.pi / 4, 7 * math.pi / 4], 1)
        for part in range(8):
            b = simulation.envelope(arc, part / 8, (part + 1) / 8)
            for step in range(65):
                x, y = simulation.point(arc, (part + step / 64) / 8)
                self.assertLessEqual(b[0], x - 1)
                self.assertLessEqual(b[1], y - 1)
                self.assertGreaterEqual(b[2], x + 1)
                self.assertGreaterEqual(b[3], y + 1)

    def test_diagonal_subdivision_removes_a_known_false_envelope_hit(self):
        diagonal = ("line", [0, 0, 100, 100], 1)
        self.assertEqual(simulation.envelope(diagonal), [-1, -1, 101, 101])
        self.assertGreater(simulation.distance(diagonal, 0, 100), 69)
        self.assertFalse(any(b[0] <= 0 <= b[2] and b[1] <= 100 <= b[3]
                             for b in (simulation.envelope(diagonal, n / 8, (n + 1) / 8) for n in range(8))))


if __name__ == "__main__":
    unittest.main()
