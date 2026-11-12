"""Regression tests for integrated read-only bootchain QEMU transcript parsing."""
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[3]
PATH = ROOT / "tools/mpc-one/run_bootchain_integrated_lab.py"
SPEC = importlib.util.spec_from_file_location("mpfree_integrated_lab_tests", PATH)
LAB = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(LAB)


class SerialCompletionTests(unittest.TestCase):
    def test_crcrlf_from_arm_pl011_serial(self):
        marker = "MPFREE_VM_DONE_123X456"
        transcript = "~ # echo " + marker + "\r\r\n" + marker + "\r\r\n~ # "
        self.assertTrue(LAB.guest_marker_seen(transcript, marker))

    def test_echoed_command_is_not_completion(self):
        marker = "MPFREE_VM_DONE_123X456"
        self.assertFalse(LAB.guest_marker_seen("~ # echo " + marker + "\r\r\n", marker))

    def test_other_marker_not_completion(self):
        self.assertFalse(LAB.guest_marker_seen("MPFREE_VM_DONE_different\r\n", "MPFREE_VM_DONE_expected"))

    def test_lf_and_optional_trailing_spaces(self):
        self.assertTrue(LAB.guest_marker_seen("MPFREE_VM_DONE_expected  \n", "MPFREE_VM_DONE_expected"))

    def test_section_marker_not_done(self):
        self.assertFalse(LAB.guest_marker_seen("MPFREE_VM_SECTION_completed\r\r\n",
                                               "MPFREE_VM_DONE_expected"))


if __name__ == "__main__":
    unittest.main()
