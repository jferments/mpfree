#!/usr/bin/env python3
"""The device checks (app_checks.py): the catalogue stays true to the app, and each check judges the log right."""
import importlib.util
import os
import re
import subprocess
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOLS = os.path.dirname(HERE)
REPO = os.path.dirname(os.path.dirname(TOOLS))
_spec = importlib.util.spec_from_file_location("app_checks", os.path.join(TOOLS, "app_checks.py"))
checks = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(checks)


def lines(*messages, start=100.0):
    return [(start + k, m) for k, m in enumerate(messages)]


def midi(text):
    return f"+1.000 s Private: 90 00 7F = {text}"


class CatalogueTests(unittest.TestCase):
    def test_names_kinds_and_texts(self):
        names = [c.name for c in checks.CHECKS]
        self.assertEqual(len(names), len(set(names)))
        for c in checks.CHECKS:
            self.assertRegex(c.name, r"^[a-z][a-z0-9-]*$")
            self.assertIn(c.kind, ("hands", "auto"))
            self.assertTrue(c.title and c.why and c.looks, c.name)
            self.assertTrue(c.smoke, f"{c.name}: name the tests that check its software side")
            if c.kind == "hands":
                self.assertTrue(c.steps, c.name)
                self.assertLessEqual(len(c.steps), 8, f"{c.name}: a check is a few steps, not a phase")
            else:
                self.assertFalse(c.steps or c.ask, f"{c.name}: an auto check has no steps and no question")
            if c.ask:
                self.assertTrue(c.ask.endswith("?") and len(c.ask) <= 120, f"{c.name}: one short question")

    def test_every_named_test_is_in_the_rust_sources(self):
        """A test renamed or removed in the app shows up here, not as a silent hole in a round."""
        names = checks.all_test_names()
        self.assertGreater(len(names), 1000)
        missing = [(c.name, t) for c in checks.CHECKS for t in c.smoke if t not in names]
        missing += [(title, t) for title, tests in checks.AREAS for t in tests if t not in names]
        self.assertEqual(missing, [])

    def test_needed_buttons_are_in_map_controls_list(self):
        with open(os.path.join(REPO, "rust/mpfree-app/src/app/mapper.rs")) as fh:
            listed = set(re.findall(r'\bc\("([^"]+)", Job::', fh.read()))
        for c in checks.CHECKS:
            for name in c.needs:
                self.assertIn(name, listed, f"{c.name} needs {name}")
        self.assertEqual(checks.control_key("PAD BANK A/E"), "pad-bank-a-e")
        self.assertEqual(checks.control_key("TRACK MIX / PAD MIX"), "track-mix-pad-mix")
        self.assertEqual(checks.control_key("-"), "minus")

    def test_the_file_is_ascii(self):
        with open(os.path.join(TOOLS, "app_checks.py"), "rb") as fh:
            self.assertTrue(all(b < 128 for b in fh.read()))

    def test_chosen_keeps_the_order_and_refuses_unknown_names(self):
        self.assertEqual([c.name for c in checks.chosen(" load, transport,load ,")], ["load", "transport"])
        self.assertEqual(checks.chosen(""), [])
        with self.assertRaises(checks.Unknown):
            checks.chosen("transport,phase1")

    def test_list_and_show(self):
        tool = os.path.join(TOOLS, "app_checks.py")
        out = subprocess.run([sys.executable, "-I", tool, "list"], capture_output=True, text=True).stdout
        for c in checks.CHECKS:
            self.assertIn(f"  {c.name} ", out)
        out = subprocess.run([sys.executable, "-I", tool, "show", "transport"], capture_output=True, text=True).stdout
        self.assertIn("the log must show: REC (73) armed the loop", out)
        self.assertIn("its software side: rec_play_and_overdub_record_the_loop_and_light_on_a_fresh_mpc", out)
        res = subprocess.run([sys.executable, "-I", tool, "show", "nope"], capture_output=True, text=True)
        self.assertEqual(res.returncode, 1)
        self.assertIn("unknown check(s) nope", res.stdout)

    def test_smoke_without_cargo_says_so(self):
        tool = os.path.join(TOOLS, "app_checks.py")
        env = dict(os.environ, PATH="/nonexistent")
        res = subprocess.run([sys.executable, "-I", tool, "smoke"], capture_output=True, text=True, env=env)
        self.assertEqual(res.returncode, 1, res.stdout + res.stderr)
        self.assertIn("RESULT: the tests did not run: cargo is not on PATH", res.stdout)
        self.assertNotIn("Traceback", res.stderr)


class JudgeTests(unittest.TestCase):
    def judge(self, name, window):
        return [(ok, what) for ok, what, _ in checks.judge(checks.BY_NAME[name], window)]

    def test_transport_needs_each_button_and_what_it_did(self):
        w = checks.Window(lines(midi("button 73 pressed"), "looper: Record: armed, the first pass ...",
                                midi("button 82 pressed"), midi("button 80 pressed"), "looper: overdub on (layer 2)",
                                "looper: overdub off", midi("button 81 pressed")))
        got = dict((what, ok) for ok, what in self.judge("transport", w))
        self.assertTrue(got["REC (73) armed the loop"])
        self.assertTrue(got["OVERDUB (80) turned overdub on"])
        self.assertFalse(got["UNDO (67) took the layer back"], "UNDO never pressed")
        # A press with nothing after it is not enough.
        w = checks.Window(lines(midi("button 73 pressed")))
        ok, _, detail = checks.judge(checks.BY_NAME["transport"], w)[0]
        self.assertFalse(ok)
        self.assertIn("no line like '^looper: Record' followed", detail)

    def test_load_counts_underruns_between_status_lines(self):
        session = [(90.0, "status: 1 hits (0 from the screen), 0 repeats, 3 underruns in 1000 periods, x"),
                   (130.0, "status: 9 hits (0 from the screen), 0 repeats, 3 underruns in 11000 periods, x")]
        test = checks.no_underruns_while()
        self.assertEqual(test(checks.Window([], session, 100.0, 120.0))[0], True)
        session[1] = (130.0, "status: 9 hits (0 from the screen), 0 repeats, 5 underruns in 11000 periods, x")
        ok, detail = test(checks.Window([], session, 100.0, 120.0))
        self.assertFalse(ok)
        self.assertIn("2 underruns in 10,000 periods", detail)
        self.assertIsNone(test(checks.Window([], session[:1], 100.0, 120.0))[0], "no status after it yet")

    def test_kept_reads_the_running_value_too(self):
        session = lines("saved settings: pad curve Hard, sound cores two, loop memory 10 min, x",
                        "sound cores: shared: the audio thread runs where the kernel puts it")
        ok, detail = checks.kept_settings()(checks.Window([], session, settings="sound-cores = two\n"))
        self.assertFalse(ok)
        self.assertIn("it runs as shared", detail)
        ok, _ = checks.kept_settings()(checks.Window([], session[:1], settings="sound-cores = two\n"))
        self.assertTrue(ok)
        ok, detail = checks.kept_settings()(checks.Window([], [], settings=""))
        self.assertFalse(ok)
        self.assertIn("started with its defaults", detail)
        self.assertIsNone(checks.kept_settings()(checks.Window([], session, settings=None))[0])

    def test_hour_lists_underrun_marks(self):
        session = lines("status: 9 hits (0 from the screen), 0 repeats, 0 underruns in 1300000 periods, x",
                        "underrun at period 524292")
        ok, detail = checks.hour_long()(checks.Window([], session))
        self.assertFalse(ok)
        self.assertIn("underrun at period 524292", detail)

    def test_stayed_up_sees_a_panic_or_a_restart(self):
        self.assertTrue(checks.stayed_up()(checks.Window(lines("sample: warp 2 hits")))[0])
        self.assertFalse(checks.stayed_up()(checks.Window(lines("PANIC: index out of bounds")))[0])
        self.assertFalse(checks.stayed_up()(checks.Window(lines("(systemd) mpfree-test-app.service: Main process "
                                                                "exited, code=killed")))[0])

    def test_not_found_names_buttons_without_a_number(self):
        chosen = checks.chosen("transport,track-mute")
        missing = checks.not_found(chosen, "track-mute = skipped\npad-bank-a-e = button 35\n")
        self.assertEqual(missing, [("transport", "SAMPLER"), ("track-mute", "TRACK MUTE"),
                                   ("track-mute", "PAD BANK B/F")])


if __name__ == "__main__":
    unittest.main()
