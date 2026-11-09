"""M10 mock-only checks; NEVER connects to MPC or opens input devices."""
from pathlib import Path
import importlib.util
import sys
import unittest

BASE = Path(__file__).resolve().parents[1] / "tools" / "mpc-one"
sys.path.insert(0, str(BASE))
spec = importlib.util.spec_from_file_location("mpfree_m10_touch_test", BASE / "host_touch_axes.py")
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)

FIELDS = {
    "driver": "ili2117",
    "device": "event0",
    "bus": "i2c-4-0026",
    "access": "evdev_RDONLY",
    "events_consumed": "0",
    "exclusive_grab": "no",
    "multitouch_slots": "10",
    "status": "PASS",
}
for name, lo, hi in (
    ("abs_x", 0, 2047),
    ("abs_y", 0, 2047),
    ("abs_mt_slot", 0, 9),
    ("abs_mt_x", 0, 2047),
    ("abs_mt_y", 0, 2047),
    ("abs_mt_tracking_id", -1, 65535),
):
    for prop, val in (
        ("min", lo), ("max", hi), ("fuzz", 0), ("flat", 0), ("resolution", 1)
    ):
        FIELDS[f"{name}_{prop}"] = str(val)

# Synthetic fixture only: none of these axis numbers or 10 slots are known
# to be the actual user's ILI2117 settings.
def example_output(fields=None):
    fields = FIELDS if fields is None else fields
    return "\n".join([
        "STAGE=CHECK_RUN_TMPFS",
        "STAGE=CHECK_STOCK_AND_TOUCH",
        "STAGE=CREATE_TMPFS_DIR",
        "STAGE=RECEIVE_PAYLOAD",
        "STAGE=VERIFY_SHA256",
        "MPFREE_TOUCH_STAGE_SHA256_OK",
        "STAGE=TOUCH_METADATA_IOCTL",
        "MPFREE_TOUCH_ABS_METADATA_V1",
        *(f"{k}={v}" for k, v in fields.items()),
        "STAGE=RUST_EXECUTED",
        "",
    ])


class TouchSafetyTests(unittest.TestCase):
    def test_fixture_valid_without_hardware(self):
        received = probe.parse_touch_output(
            example_output(), probe.WATCHDOG + "\n", 0
        )
        self.assertEqual(int(received["multitouch_slots"]), 10)
        self.assertEqual(received["abs_mt_tracking_id_min"], "-1")

    def test_all_identity_and_structural_fields_are_mandatory(self):
        for key in FIELDS:
            wrong = dict(FIELDS)
            del wrong[key]
            with self.subTest(key=key), self.assertRaises(RuntimeError):
                probe.parse_touch_output(example_output(wrong), probe.WATCHDOG + "\n", 0)

    def test_refuse_bad_report_even_if_it_says_pass(self):
        mutations = (
            ("device", "event1"),
            ("driver", "fake"),
            ("bus", "i2c-2-0026"),
            ("access", "evdev_RDWR"),
            ("events_consumed", "1"),
            ("exclusive_grab", "yes"),
            ("abs_x_min", "9999999999999"),
            ("abs_x_max", "0"),
            ("abs_y_max", "0"),
            ("abs_mt_slot_min", "1"),
            ("abs_mt_slot_max", "64"),
            ("abs_mt_tracking_id_min", "not-an-int"),
            ("abs_mt_x_fuzz", "-1"),
            ("multitouch_slots", "11"),
            ("status", "FAIL"),
        )
        for key, value in mutations:
            wrong = dict(FIELDS)
            wrong[key] = value
            with self.subTest(key=key), self.assertRaises(RuntimeError):
                probe.parse_touch_output(example_output(wrong), probe.WATCHDOG + "\n", 0)

    def test_fails_on_duplicate_marker_missing_watchdog_or_error(self):
        output = example_output()
        for bad in (output + "\n" + probe.SENTINEL, output + "\nabs_x_min=0",
                    output.replace("STAGE=RUST_EXECUTED", "")):
            with self.assertRaises(RuntimeError):
                probe.parse_touch_output(bad, probe.WATCHDOG + "\n", 0)
        for stderr in ("", probe.WATCHDOG + "\n" + probe.WATCHDOG + "\n"):
            with self.assertRaises(RuntimeError):
                probe.parse_touch_output(output, stderr, 0)
        with self.assertRaises(RuntimeError):
            probe.parse_touch_output(output, probe.WATCHDOG + "\n", 127)
        with self.assertRaises(RuntimeError):
            probe.parse_touch_output(output + "\nfoo=bar", probe.WATCHDOG + "\n", 0)

    def test_preflight_expected_live_device_markers(self):
        preflight = "\n".join([
            "MPFREE_TOUCH_AXES_PREFLIGHT_V1",
            "touch_driver=ili2117",
            "touch_event=event0",
            "touch_bus=i2c-4-0026",
            "factory_service_active=active",
            "ioctl_performed=no",
            "END MPFREE_TOUCH_AXES_PREFLIGHT",
        ])
        probe.verify_touch_preflight(preflight)
        with self.assertRaises(RuntimeError):
            probe.verify_touch_preflight(preflight.replace("ioctl_performed=no", "ioctl_performed=yes"))

    def test_fixed_remote_script_and_hash_rejection(self):
        digest = "a" * 64
        script = probe.remote_script(digest)
        self.assertIn('test "$observed" = "' + digest + '"', script)
        self.assertIn('MPFREE_ACCEPT_TOUCH_IOCTL=YES "$stage/probe" --inspect', script)
        self.assertIn("ulimit -t 3", script)
        self.assertIn("mpfree-touch-axes.", script)
        self.assertIn("trap cleanup 0", script)
        self.assertIn("systemctl is-active acvs.service", script)
        self.assertNotIn("systemctl stop", script)
        self.assertNotIn("EVIOCGRAB", script)
        for bad in ("", "not-a-hash", "a" * 63, "a" * 64 + "x"):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                probe.remote_script(bad)


if __name__ == "__main__":
    unittest.main()
