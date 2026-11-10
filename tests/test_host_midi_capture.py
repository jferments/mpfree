"""Mock-only checks for the passive usbmon MIDI capture; NEVER connects to an MPC."""
from pathlib import Path
import importlib.util
import unittest

BASE = Path(__file__).resolve().parents[1] / "tools" / "mpc-one"
spec = importlib.util.spec_from_file_location("mpfree_midi_capture", BASE / "host_midi_capture.py")
cap = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cap)


class UsbmonParseTests(unittest.TestCase):
    def test_in_completion_with_data(self):
        r = cap.parse_usbmon_line("d6b3e5c0 1873512690 C Bi:1:002:1 0 4 = 19903c64", 1, 2)
        self.assertEqual(r, ("IN", bytes.fromhex("19903c64"), 4))

    def test_out_submission_with_data(self):
        r = cap.parse_usbmon_line("d6b3e5c0 1873512690 S Bo:1:002:1 -115 8 = 0b904c7f 00000000", 1, 2)
        self.assertEqual(r[0], "OUT")
        self.assertEqual(r[1], bytes.fromhex("0b904c7f00000000"))

    def test_ignores_other_devices_events_and_control(self):
        self.assertIsNone(cap.parse_usbmon_line("x 1 C Bi:1:003:1 0 4 = 19903c64", 1, 2))
        self.assertIsNone(cap.parse_usbmon_line("x 1 S Bi:1:002:1 -115 64 <", 1, 2))
        self.assertIsNone(cap.parse_usbmon_line("x 1 C Bo:1:002:1 0 8 >", 1, 2))
        self.assertIsNone(cap.parse_usbmon_line("x 1 C Ci:1:002:0 0 4 = 01050000", 1, 2))
        self.assertIsNone(cap.parse_usbmon_line("@@CAPTURE START", 1, 2))

    def test_truncated_transfer_detected(self):
        words = " ".join(["0f0f0f0f"] * 8)  # usbmon text shows at most 32 bytes
        r = cap.parse_usbmon_line(f"x 1 S Bo:1:002:1 -115 64 = {words}", 1, 2)
        self.assertEqual(r[2], 64)
        self.assertEqual(len(r[1]), 32)


class DecoderTests(unittest.TestCase):
    def test_note_on_cable1(self):
        d = cap.MidiDecoder()
        msgs = d.feed("IN", bytes.fromhex("19903c64" "00000000"))
        self.assertEqual(msgs, [("IN", 1, bytes([0x90, 0x3C, 0x64]))])
        self.assertIn("note_on ch=1 60 100", cap.describe(msgs[0][2]))

    def test_sysex_reassembly_across_packets(self):
        d = cap.MidiDecoder()
        # F0 47 7F 46 65 00 04 00 7F 00 00 F7 = 12 bytes: 4,4,4(start/cont) ... end
        data = bytes.fromhex("04f0477f" "04466500" "0404007f" "070000f7")
        msgs = d.feed("OUT", data)
        self.assertEqual(len(msgs), 1)
        self.assertEqual(msgs[0][2], bytes.fromhex("f0477f46650004007f0000f7"))
        self.assertTrue(cap.describe(msgs[0][2]).startswith("sysex len=12"))

    def test_sysex_split_across_transfers(self):
        d = cap.MidiDecoder()
        self.assertEqual(d.feed("OUT", bytes.fromhex("04f0477f")), [])
        msgs = d.feed("OUT", bytes.fromhex("0646f700"))
        self.assertEqual(msgs[0][2], bytes.fromhex("f0477f46f7"))

    def test_analyze_groups_by_phase(self):
        lines = [("gap", "@@CAPTURE START"),
                 ("pads", "x 1 C Bi:1:002:1 0 4 = 19902564"),
                 ("pads", "x 1 C Bi:1:002:1 0 4 = 19802500")]
        text = cap.analyze(lines, 1, 2)
        self.assertIn("== pads: 2 messages", text)
        self.assertIn("IN  cable1 90 37", text)


class RemoteScriptTests(unittest.TestCase):
    def test_capture_script_is_bounded_and_passive(self):
        text = (BASE / "passive-usbmon-capture.sh").read_text()
        code = "\n".join(l for l in text.splitlines() if not l.lstrip().startswith("#"))
        self.assertIn("CAPTURE_SECONDS=240", code)
        self.assertIn('sleep "$CAPTURE_SECONDS"', code)
        self.assertIn('kill "$pid"', code)
        self.assertIn('"09e8:0046"', code)
        for word in ("mount ", "modprobe", "insmod", "/dev/snd", "systemctl", "echo 1", "amidi"):
            self.assertNotIn(word, code, msg=word)
        cleaned = code.replace("2>/dev/null", "").replace(">/dev/null", "").replace("2>&1", "")
        self.assertNotRegex(cleaned, r"(?<![0-9-])>\s*[/$]", "no output redirection to files")


if __name__ == "__main__":
    unittest.main()
