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



class SyntheticDriveRoutingTests(unittest.TestCase):
    def sample_command(self):
        return [
            "qemu-system-arm", "-snapshot", "-machine", "virt",
            "-kernel", "/tmp/kernel-zImage",
            "-append", "console=ttyAMA0 root=/dev/vda rw rootwait init=/bin/sh",
            "-drive", "if=none,id=mpfree0,format=qcow2,file=/tmp/root.qcow2",
            "-device", "virtio-blk-device,drive=mpfree0",
        ]

    def test_second_virtio_mmio_disk_moves_root_to_vdb(self):
        """Regression for private 2026-11-12 log: 24MiB vda, 431MiB vdb."""
        original = self.sample_command()
        updated = LAB.qemu_storage_for_synthetic_model(original, "/tmp/model.ext4")
        self.assertEqual(original[original.index("-append") + 1].split()[1],
                         "root=/dev/vda")
        self.assertIn("root=/dev/vdb", updated[updated.index("-append") + 1])
        self.assertEqual(
            [v for v in updated if v.startswith("virtio-blk-device,drive=")],
            ["virtio-blk-device,drive=mpfree0",
             "virtio-blk-device,drive=mpfreemock"],
        )
        self.assertIn("if=none,id=mpfreemock,format=raw,readonly=on,file=/tmp/model.ext4",
                      updated)

    def test_refuses_unknown_existing_root_mapping(self):
        cmd = self.sample_command()
        cmd[cmd.index("-append") + 1] = "root=/dev/vdb init=/bin/sh"
        with self.assertRaises(ValueError):
            LAB.qemu_storage_for_synthetic_model(cmd, "/tmp/model.ext4")

    def test_refuses_duplicate_mock_disk(self):
        cmd = LAB.qemu_storage_for_synthetic_model(self.sample_command(),
                                                    "/tmp/model.ext4")
        with self.assertRaises(ValueError):
            LAB.qemu_storage_for_synthetic_model(cmd, "/tmp/model.ext4")


if __name__ == "__main__":
    unittest.main()
