"""Mock-only checks for the guided MIDI activity map; NEVER connects to an MPC."""
from pathlib import Path
import importlib.util
import subprocess
import unittest

BASE = Path(__file__).resolve().parents[1] / "tools" / "mpc-one"
spec = importlib.util.spec_from_file_location("mpfree_midi_activity", BASE / "host_midi_activity.py")
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)

SNAP_A = """MPFREE MIDI COUNTERS V1
@@FILE /proc/asound/card0/midi0
MPC One MIDI

Output 0
  Tx bytes     : 6036856
  Owner PID    : 311
  Mode         : direct
  Buffer size  : 4096
  Avail        : 4096
Output 1
  Tx bytes     : 29775
Input 0
  Rx bytes     : 0
Input 1
  Rx bytes     : 49837
  Owner PID    : 311
  Overruns     : 0
@@FILE /proc/asound/card0/usbmidi
@@UNAVAILABLE
@@END
"""
SNAP_B = SNAP_A.replace("49837", "50237").replace("6036856", "6037000")


class ParseTests(unittest.TestCase):
    def test_parses_traffic_counters_only(self):
        v = mod.parse_counters(SNAP_A)
        f = "/proc/asound/card0/midi0"
        self.assertEqual(v[(f, "Output 0", "Tx bytes")], 6036856)
        self.assertEqual(v[(f, "Input 1", "Rx bytes")], 49837)
        self.assertEqual(v[(f, "Input 1", "Overruns")], 0)
        self.assertNotIn((f, "Output 0", "Owner PID"), v)
        self.assertNotIn((f, "Output 0", "Buffer size"), v)

    def test_delta_and_format(self):
        d = mod.deltas(mod.parse_counters(SNAP_A), mod.parse_counters(SNAP_B))
        f = "/proc/asound/card0/midi0"
        self.assertEqual(d[(f, "Input 1", "Rx bytes")], 400)
        self.assertEqual(d[(f, "Output 0", "Tx bytes")], 144)
        self.assertEqual(d[(f, "Output 1", "Tx bytes")], 0)
        text = mod.format_phase("pads", 8.0, d)
        self.assertIn("Input 1", text)
        self.assertIn("+400", text)
        self.assertNotIn("Output 1", text)

    def test_rejects_bad_snapshots(self):
        with self.assertRaises(ValueError):
            mod.parse_counters("garbage")
        with self.assertRaises(ValueError):
            mod.parse_counters(SNAP_A.replace("@@END\n", ""))
        with self.assertRaises(ValueError):
            mod.parse_counters("MPFREE MIDI COUNTERS V1\n@@END\n")

    def test_ssh_command_is_strict_and_fixed(self):
        cmd = mod.ssh_command("10.42.0.141", 2222, "root", "/k")
        self.assertIn("StrictHostKeyChecking=yes", cmd)
        self.assertIn("BatchMode=yes", cmd)
        self.assertEqual(cmd[-1], "sh -s")


class RemoteScriptTests(unittest.TestCase):
    def test_counter_script_is_read_only(self):
        text = (BASE / "read-only-midi-counters.sh").read_text()
        code = "\n".join(l for l in text.splitlines() if not l.lstrip().startswith("#"))
        self.assertNotIn("/dev/snd", code)
        self.assertNotIn(">", code.replace("2>/dev/null", ""))
        for word in ("amidi", "aconnect", "systemctl", "mount", "echo "):
            self.assertNotIn(word, code)

    def test_counter_script_output_parses(self):
        out = subprocess.run(["sh", str(BASE / "read-only-midi-counters.sh")],
                             stdout=subprocess.PIPE, check=True).stdout.decode()
        self.assertIn(mod.SENTINEL, out)
        self.assertIn("@@END", out)

    def test_survey_script_never_touches_devices_or_mounts(self):
        text = (BASE / "read-only-audio-midi.sh").read_text()
        code = "\n".join(l for l in text.splitlines() if not l.lstrip().startswith("#"))
        self.assertNotIn("/dev/snd", code)
        self.assertIn("*serial*|*mac-address*|*uuid*) printf '  %s = [redacted]", code)
        for word in ("mount -", "modprobe", "insmod", "systemctl stop", "systemctl restart",
                     "systemctl start", "chrt", "taskset", "renice", "ioctl"):
            self.assertNotIn(word, code, msg=word)
        # ALSA/MIDI tools may only be looked up with `command -v`, never run.
        for line in code.splitlines():
            if line.strip().startswith("for c in "):
                continue
            for tool in ("aplay", "arecord", "amixer", "alsactl", "amidi", "aconnect",
                         "aseqdump", "speaker-test", "evtest"):
                self.assertNotRegex(line, rf"(^|[;|&(]\s*){tool}\b", msg=line)
        cleaned = code.replace("2>/dev/null", "").replace(">/dev/null", "").replace("2>&1", "").replace("< \"$p\"", "").replace("< \"$c\"", "")
        self.assertNotRegex(cleaned, r"(?<![0-9-])>\s*[/$]", "no output redirection to files")


if __name__ == "__main__":
    unittest.main()
