"""Host-only tests: interpreting CPU tick differences from synthetic read-only surveys."""
import importlib.util
from pathlib import Path
import unittest

source = Path(__file__).resolve().parents[1] / "tools/mpc-one/analyze-capacity.py"
spec = importlib.util.spec_from_file_location("mpfree_capacity", source)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

REPORT = """MPFREE STOCK RESOURCE BUDGET SURVEY - READ ONLY
=== CPU counters snapshot A (system-wide + ALL factory threads) ===
cpu0 total_ticks=100 idle_iowait_ticks=60
cpu1 total_ticks=100 idle_iowait_ticks=50
cpu2 total_ticks=100 idle_iowait_ticks=70
cpu3 total_ticks=100 idle_iowait_ticks=90
factory_thread_ticks=500 sampled_threads=38
=== Two-second sampling interval (no device operations) ===
=== CPU counters snapshot B ===
cpu0 total_ticks=200 idle_iowait_ticks=80
cpu1 total_ticks=200 idle_iowait_ticks=140
cpu2 total_ticks=200 idle_iowait_ticks=100
cpu3 total_ticks=200 idle_iowait_ticks=170
factory_thread_ticks=650 sampled_threads=38
END MPFREE STOCK RESOURCE BUDGET SURVEY
"""

class CapacityTests(unittest.TestCase):
    def test_busy_fraction_and_process_total(self):
        out = module.parse(REPORT)
        self.assertEqual(out["cores"],
                         {"cpu0": 80.0, "cpu1": 10.0, "cpu2": 70.0, "cpu3": 20.0})
        self.assertAlmostEqual(out["all_core_factory_percent"], 37.5)
        self.assertAlmostEqual(out["single_core_equivalent_percent"], 150.0)
        self.assertEqual(out["thread_count_after"], 38)

    def test_missing_second_snapshot_is_rejected(self):
        with self.assertRaises(ValueError):
            module.parse(REPORT.split("=== CPU counters snapshot B")[0])

    def test_counter_reset_is_rejected(self):
        with self.assertRaises(ValueError):
            module.parse(REPORT.replace("factory_thread_ticks=650", "factory_thread_ticks=100"))
        with self.assertRaises(ValueError):
            module.parse(REPORT.replace("cpu3 total_ticks=200", "cpu3 total_ticks=50"))

    def test_duplicate_core_or_missing_factory_is_rejected(self):
        with self.assertRaises(ValueError):
            module.parse(REPORT.replace(
                "cpu0 total_ticks=100 idle_iowait_ticks=60",
                "cpu0 total_ticks=100 idle_iowait_ticks=60\n"
                "cpu0 total_ticks=100 idle_iowait_ticks=60",
            ))
        with self.assertRaises(ValueError):
            module.parse(REPORT.replace("factory_thread_ticks=650 sampled_threads=38", ""))

if __name__ == "__main__":
    unittest.main()
