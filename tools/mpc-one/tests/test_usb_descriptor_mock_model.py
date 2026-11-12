"""Host-side regression tests for synthetic QEMU USB descriptor model.

The model processes fixed synthetic bytes only; it does not access USB,
block devices, vendor binaries, or actual MPC hardware.
"""
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
SOURCE = ROOT / "tools/mpc-one/usb_descriptor_mock_model.c"


class SyntheticUsbDescriptorModelTests(unittest.TestCase):
    def test_static_source_guard(self):
        source = SOURCE.read_text()
        self.assertIn("b[off + 5] == 0xff", source)
        for code in ("0x42", "0x03", "0x61", "0x7a", "0x06", "0x05"):
            self.assertIn(code, source)
        for prohibited in ("libusb_open", "ioctl(", "open(", "/dev/mmcblk",
                           "/dev/bus/usb", "system("):
            self.assertNotIn(prohibited, source)

    def test_native_model_matches_synthetic_cases(self):
        compiler = shutil.which("cc") or shutil.which("gcc")
        if compiler is None:
            self.skipTest("No host C compiler")
        with tempfile.TemporaryDirectory(prefix="mpfree-usb-model-test-") as tmp:
            binary = Path(tmp) / "model"
            build = subprocess.run(
                [compiler, "-std=c11", "-O2", "-Wall", "-Wextra", "-Werror",
                 "-o", str(binary), str(SOURCE)],
                capture_output=True, text=True, timeout=60,
            )
            self.assertEqual(build.returncode, 0, build.stdout + build.stderr)
            run = subprocess.run([str(binary)], capture_output=True, text=True,
                                 timeout=20)
            self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
            rows = [json.loads(s) for s in run.stdout.splitlines()
                    if s.startswith('{"test":')]
            self.assertEqual(len(rows), 13)
            self.assertTrue(all(r["pass"] for r in rows))
            self.assertIn("MPFREE_USB_DESCRIPTOR_MODEL_TESTS 13/13", run.stdout)
            self.assertIn("MPFREE_USB_DESCRIPTOR_MODEL_PASS", run.stdout)


if __name__ == "__main__":
    unittest.main()
