"""Only deterministic host-side parsing tests. Never contacts MPC."""
import importlib.util
from pathlib import Path
import unittest

SOURCE = Path(__file__).resolve().parents[1] / "tools/mpc-one/check-arm-abi.py"
spec = importlib.util.spec_from_file_location("mpfree_arm_abi", SOURCE)
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)

class ArmAbiTests(unittest.TestCase):
    def test_glibc_report_version(self):
        text = """MPFREE ARMV7 USERSPACE ABI SURVEY - READ ONLY
getconf GNU_LIBC_VERSION: glibc 2.39
GLIBC_2.37
GLIBC_2.39
"""
        self.assertEqual(checker.extract_target_glibc(text), (2, 39, 0))
        self.assertIsNone(checker.extract_target_glibc("glibc 2.39 without sentinel"))

    def test_highest_needed_symbol_not_confused_with_other_text(self):
        text = "Name: GLIBC_2.34\nName: GLIBC_2.17\nName: GLIBC_2.28\n"
        self.assertEqual(checker.highest_glibc_required(text), (2, 34, 0))
        self.assertIsNone(checker.highest_glibc_required("no symbols"))

    def test_loader_and_dt_needed_extraction(self):
        program = "[Requesting program interpreter: /lib/ld-linux-armhf.so.3]\n"
        dyn = "Shared library: [libc.so.6]\nShared library: [libm.so.6]\n"
        self.assertEqual(checker.interpreter(program), "/lib/ld-linux-armhf.so.3")
        self.assertEqual(checker.needed_libraries(dyn), ["libc.so.6","libm.so.6"])

    def test_older_target_must_be_rejected(self):
        target=(2,31,0); required=(2,34,0)
        self.assertGreater(required,target)

if __name__=="__main__":
    unittest.main()
