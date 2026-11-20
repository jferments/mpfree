"""No-hardware tests for conservative ARM-QEMU lab preflight.

Never runs QEMU or touches real owner backups.
"""
import importlib.util
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[3]
SCRIPT=ROOT/"tools/mpc-one/qemu_arm_lab_preflight.py"
spec=importlib.util.spec_from_file_location("mpfree_arm_qemu_lab",SCRIPT)
lab=importlib.util.module_from_spec(spec)
spec.loader.exec_module(lab)

class MPFreeARMLabTests(unittest.TestCase):
    def fake_ext4(self,folder):
        image=folder/"rootfs.img"
        with image.open("wb") as f:
            f.truncate(25*1024*1024)
            f.seek(1024+56)
            f.write(bytes.fromhex("53ef"))
        return image

    def test_accepts_a_regular_raw_ext4_image(self):
        with tempfile.TemporaryDirectory() as td:
            image=self.fake_ext4(Path(td))
            self.assertEqual(lab.ext4_image(image),image)

    def test_rejects_partitioned_or_other_raw_image(self):
        with tempfile.TemporaryDirectory() as td:
            image=Path(td)/"non-ext4.img"
            with image.open("wb") as f:f.truncate(25*1024*1024)
            with self.assertRaisesRegex(ValueError,"ext4"):
                lab.ext4_image(image)

    def test_rejects_symlink_input(self):
        with tempfile.TemporaryDirectory() as td:
            image=self.fake_ext4(Path(td))
            link=Path(td)/"linked-rootfs.img"
            link.symlink_to(image)
            with self.assertRaises(ValueError):lab.ext4_image(link)

    def test_workspace_cannot_escape_mpfree_workdir(self):
        with tempfile.TemporaryDirectory() as td:
            home=Path(td)
            (home/"mpfree-work").mkdir()
            with patch.object(lab.Path,"home",return_value=home):
                allowed=home/"mpfree-work"/"qemu-lab"
                self.assertEqual(lab.safe_workspace(allowed),allowed)
                with self.assertRaises(ValueError):
                    lab.safe_workspace(home/"Downloads"/"qemu-lab")
                with self.assertRaises(ValueError):
                    lab.safe_workspace(home/"mpfree-work"/"../Downloads")

    def test_boot_command_cannot_execute_or_pass_physical_disk(self):
        with tempfile.TemporaryDirectory() as td:
            root=Path(td)
            kernel=root/"generic-zImage"; kernel.write_bytes(b"sample")
            overlay=root/"guest.qcow2";overlay.write_bytes(b"sample")
            line=lab.boot_command(kernel,overlay)
            self.assertEqual(line[0],"qemu-system-arm")
            self.assertIn("virt",line)
            self.assertIn("none",line)
            self.assertIn("virtio-blk-device,drive=rootdisk",line)
            self.assertNotIn("/dev/mmcblk0",str(line))
            self.assertNotIn("-device usb-host",str(line))

if __name__=="__main__":unittest.main()
