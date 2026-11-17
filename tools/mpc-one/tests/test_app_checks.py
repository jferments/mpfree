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
        """A test renamed or removed in the app shows up here, not as a silent hole in a round. Only the app's
        crate counts: `smoke` runs `cargo test -p mpfree-app`, so a test in another crate would never run."""
        names = checks.all_test_names()
        self.assertGreater(len(names), 500)
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

    def test_a_failed_tests_panic_is_shown_with_its_name(self):
        out = ("test smoke::phases::looper::layers_x ... FAILED\n\nfailures:\n\n"
               "---- smoke::phases::looper::layers_x stdout ----\n\n"
               "thread '<unnamed>' (15472) panicked at mpfree-app/src/smoke/phases.rs:382:13:\n"
               "assertion `left == right` failed: x deleted layer 2\n  left: 2\n right: 1\n"
               "stack backtrace:\n   0: __rustc::rust_begin_unwind\n\nfailures:\n    smoke::phases::looper::layers_x\n")
        why = checks.failure_lines(out)
        self.assertEqual(why["layers_x"], ["thread '<unnamed>' (15472) panicked at mpfree-app/src/smoke/phases.rs:382:13:",
                                           "assertion `left == right` failed: x deleted layer 2",
                                           "left: 2", "right: 1"])


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

    def test_rec_arm_needs_the_ready_the_take_and_the_loop_armed(self):
        w = checks.Window(lines(midi("button 73 pressed"), "rec: ready, PLAY records Track 1",
                                midi("button 82 pressed"), "time: transport: Record, new clip in slot 1 of track 1",
                                midi("button 67 pressed"), "Undid take 1 \u00b7 Track 1 / 1",
                                "time: undo: take 2 thrown away",
                                "rec: the loop armed", "looper: Record: armed, the first pass starts on the next bar"))
        self.assertTrue(all(ok for ok, _ in self.judge("rec-arm", w)))
        # UNDO after the take (the page's) is not UNDO while it records.
        w = checks.Window(lines(midi("button 67 pressed"), "Undid take 1 \u00b7 Track 1 / 1"))
        got = dict((what, ok) for ok, what in self.judge("rec-arm", w))
        self.assertFalse(got["UNDO while recording threw the take away"])
        # REC on the loop (the old way) is not the notes' ready.
        w = checks.Window(lines(midi("button 73 pressed"), "looper: Record: armed, the first pass ..."))
        got = dict((what, ok) for ok, what in self.judge("rec-arm", w))
        self.assertFalse(got["REC (73) readied the notes"])
        self.assertFalse(got["the Loop view armed the loop"])
        # A loop with a take already: REC's overdub waits for PLAY.
        w = checks.Window(lines("rec: the loop armed", "looper: overdub on (layer 2)"))
        got = dict((what, ok) for ok, what in self.judge("rec-arm", w))
        self.assertTrue(got["REC then went to the loop"])

    def test_loop_tracks_need_the_landing_and_both_launches(self):
        w = checks.Window(lines("time: Loop 1 on Loops A, row 1, playing", "looper: loop 1 off from bar 4",
                                "looper: loop 1 on from bar 6"))
        self.assertTrue(all(ok for ok, _ in self.judge("loop-tracks", w)))
        # Another loop's line, or one on a later Loop track, is not this one.
        w = checks.Window(lines("time: Loop 2 on Loops B, row 1, playing", "looper: loop 1 off"))
        got = dict((what, ok) for ok, what in self.judge("loop-tracks", w))
        self.assertFalse(got["the loop landed on Loops A"])
        self.assertFalse(got["row 2 stopped it on its bar line"], "off at once is not on the bar line")
        self.assertFalse(got["its cell brought it back on a bar line"])

    def test_loop_cell_needs_the_track_the_arm_and_the_landing(self):
        w = checks.Window(lines("time: Loops B added, a Loop track: tap a cell of it to record a loop there",
                                "time: loop cell: Loops B row 2 armed, the next loop recorded lands there",
                                "time: Loop 2 on Loops B, row 2, playing"))
        self.assertTrue(all(ok for ok, _ in self.judge("loop-cell", w)))
        # A loop landing in row 1 (no cell armed) is not this one.
        w = checks.Window(lines("time: Track 9 added and armed (UNDO takes it away)",
                                "time: Loop 1 on Loops A, row 1, playing"))
        got = dict((what, ok) for ok, what in self.judge("loop-cell", w))
        self.assertFalse(got["SHIFT + + Track made a Loop track"])
        self.assertFalse(got["its row 2 cell armed"])
        self.assertFalse(got["the loop landed in it"])

    def test_loop_copy_needs_the_copy_all_in_and_its_launch(self):
        w = checks.Window(lines("clips: Loop 1 copies into Loop 3 on Loops B / 3: launch it to hear it",
                                "looper: loop 3 is a copy of loop 1, 2 layers",
                                "looper: loop 3 on from bar 9"))
        self.assertTrue(all(ok for ok, _ in self.judge("loop-copy", w)))
        # A copy into another row of its own track, a copy that stopped, a launch at once: not this.
        w = checks.Window(lines("clips: a copy of Loop 1 in Loops A / 3 (UNDO takes it back)",
                                "Loop 3 has 1 of Loop 1's 2 layers: the copy stopped",
                                "looper: loop 3 on"))
        got = dict((what, ok) for ok, what in self.judge("loop-copy", w))
        self.assertFalse(got["the cell took a copy"])
        self.assertFalse(got["the copy's layers all went in"])
        self.assertFalse(got["the copy came in on a bar line"])

    def test_live_erase_needs_the_erase_line(self):
        w = checks.Window(lines("rec: ready, PLAY adds to Track 1", "ERASE: pad 1's notes come out of the armed "
                                "track's clip as the playhead passes them; UNDO brings them back"))
        self.assertTrue(all(ok for ok, _ in self.judge("live-erase", w)))
        # ERASE + a step pad (STEP SEQ) is not this one.
        w = checks.Window(lines("rec: ready, PLAY adds to Track 1", "ERASE: the locks of step 5 are off"))
        got = dict((what, ok) for ok, what in self.judge("live-erase", w))
        self.assertFalse(got["ERASE + pad 1 erased while recording"])

    def test_power_reads_the_start_and_counts_the_card(self):
        start = (5.0, "power button: /dev/input/event1; read-only, not grabbed; a short press opens the power card")
        window = [(100.0, "power: the power card is open"), (110.0, "power: the power card is open")]
        got = checks.judge(checks.BY_NAME["power"], checks.Window(window, [start] + window, 100.0, 120.0))
        self.assertEqual([ok for ok, _, _ in got], [True, True])
        self.assertEqual(got[1][2], "2 times")
        # Not found at the start: it says the line the app wrote instead; one press is not two.
        start = (5.0, "power button: not found yet (no gpio-keys device with KEY_POWER)")
        got = checks.judge(checks.BY_NAME["power"], checks.Window(window[:1], [start] + window[:1], 100.0, 120.0))
        self.assertEqual([ok for ok, _, _ in got], [False, False])
        self.assertEqual(got[0][2], "power button: not found yet (no gpio-keys device with KEY_POWER)")
        self.assertEqual(got[1][2], "1 time, 2 expected")
        got = checks.judge(checks.BY_NAME["power"], checks.Window([], [], 100.0, 120.0))
        self.assertEqual(got[0][2], "no line like '^power button: ' since power-on")

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

    def test_step_speed_reads_the_step_edits_between_status_lines(self):
        def status(t, in_place, slowest, compiled):
            return (t, "status: 1 hits (0 from the screen), 0 repeats, 0 underruns in 1000 periods, x, time: "
                       f"Playing at 1.1.1, 120.0 BPM, 0 takes; compiles: 9 (slowest 0.80 ms, 120 notes and "
                       f"locks); step edits: {in_place} in place (slowest {slowest} ms), {compiled} compiled "
                       f"(slowest 0.90 ms)")
        test = checks.step_edits_in_place()
        session = [status(90.0, 2, "0.01", 1), status(130.0, 40, "0.05", 2)]
        ok, detail = test(checks.Window([], session, 100.0, 120.0))
        self.assertTrue(ok, detail)
        self.assertEqual(detail, "38 step edits in place (slowest 0.05 ms), 1 compiled (slowest 0.90 ms); "
                                 "the slowest compile 0.80 ms, with 120 notes and locks playing")
        session[1] = status(130.0, 40, "0.40", 2)
        self.assertFalse(test(checks.Window([], session, 100.0, 120.0))[0], "past a tenth of a period")
        session[1] = status(130.0, 2, "0.01", 30)
        self.assertFalse(test(checks.Window([], session, 100.0, 120.0))[0], "none in place")
        self.assertIsNone(test(checks.Window([], session[:1], 100.0, 120.0))[0], "no status after it yet")

    def test_the_mix_checks_see_a_refused_effect_and_the_latch(self):
        """The effect checks (return-wet, the Mix curve and Room checks, the Tape delay's three, Crush's,
        Drive's and the Amp's, the busy project): any refusal from the mixer or the chain editor fails them, a
        late mode switch among them, and the latched ones start and end on Latch."""
        latched = lines("Latch on", "Latch off")
        for name in ("return-wet", "mix-curve", "mix-curve-click", "room-level",
                     "tape-first-echo", "dub-echo-dies", "runaway-howls", "crush-rate", "crush-filters",
                     "drive-modes", "amp-turns", "amp-switches", "amp-gate", "fx-busy"):
            got = dict((what, ok) for ok, what in self.judge(name, checks.Window(latched)))
            self.assertTrue(got["no refusal"], name)
            self.assertTrue(got["the app kept running"], name)
            for refusal in ("mixer: The chain is full", "chain: The mixer is busy",
                            "mixer: an effect was switched after it was taken out"):
                got = dict((what, ok) for ok, what in self.judge(name, checks.Window(lines(refusal))))
                self.assertFalse(got["no refusal"], f"{name}: {refusal}")
        got = dict((what, ok) for ok, what in self.judge("mix-curve", checks.Window(lines("Latch off"))))
        self.assertFalse(got["Latch kept the snare repeating"])
        got = dict((what, ok) for ok, what in self.judge("room-level", checks.Window(lines("Latch on"))))
        self.assertFalse(got["Latch went off"])
        got = dict((what, ok) for ok, what in self.judge("crush-rate", checks.Window(lines("Latch off"))))
        self.assertFalse(got["Latch kept the pad repeating"])
        got = dict((what, ok) for ok, what in self.judge("crush-filters", checks.Window(lines("Latch on"))))
        self.assertFalse(got["Latch went off"])
        got = dict((what, ok) for ok, what in self.judge("fx-busy", checks.Window(lines("Latch on"))))
        self.assertTrue(got["Latch kept the chord repeating"])
        self.assertFalse(got["Latch went off"])

    def test_the_midi_checks_see_a_note_read_from_any_port(self):
        """midi-in, midi-monitor, midi-port and midi-routing pass on a note the app read (MusicIn::bytes' line), from
        the jacks or a USB port; the session's summary at the end and a knob alone don't count."""
        notes = ("+3.210 s MIDI In: 90 25 64 = note on · ch 1 · C#2 · vel 100",
                 "+4.002 s Keystep 37: 99 24 40 = note on · ch 10 · C2 · vel 64")
        others = ("MIDI in: 3 keys had no pad in the kit",
                  "+5.000 s MIDI In: B0 4A 40 = CC 74 · ch 1 · 64")
        for name in ("midi-in", "midi-monitor", "midi-port", "midi-routing"):
            for line in notes:
                got = dict((what, ok) for ok, what in self.judge(name, checks.Window(lines(line))))
                self.assertTrue(got["notes came in"], f"{name}: {line}")
            for line in others:
                got = dict((what, ok) for ok, what in self.judge(name, checks.Window(lines(line))))
                self.assertFalse(got["notes came in"], f"{name}: {line}")

    def test_midi_learn_needs_the_knob_the_key_and_both_presses(self):
        """midi-learn passes on a knob learned on a value, a key learned on the transport (MusicIn::bytes' lines)
        and the key's PLAY and STOP reaching the time engine; a knob alone doesn't."""
        knob = "+3.210 s MIDI In: MIDI Learn: CC 74 \u00b7 ch 1 \u00b7 MIDI In takes t1.vol.0"
        key = "+6.500 s MIDI In: MIDI Learn: note C3 \u00b7 ch 1 \u00b7 MIDI In takes transport.0"
        both = checks.Window(lines(knob, key, "time: transport: Play", "time: transport: Stop"))
        self.assertTrue(all(ok for ok, _ in self.judge("midi-learn", both)))
        got = dict((what, ok) for ok, what in self.judge("midi-learn", checks.Window(lines(knob))))
        self.assertTrue(got["Learn took the knob"])
        self.assertFalse(got["Learn took the key for PLAY and STOP"])
        self.assertFalse(got["the key played and stopped MPFree"])
        got = dict((what, ok) for ok, what in self.judge("midi-learn", checks.Window(lines(key))))
        self.assertFalse(got["Learn took the knob"])

    def test_midi_thru_needs_sending_on_a_note_in_and_out_and_sending_off(self):
        """midi-thru passes on Prefs' switch on, a note read, the same note out of MIDI OUT (the writer's line) and
        the switch off again; without the note out it fails."""
        note_in = "+3.210 s MIDI In: 90 3C 64 = note on \u00b7 ch 1 \u00b7 C4 \u00b7 vel 100"
        note_out = "+3.219 s MIDI Out: 90 3C 64 = note on, ch 1, C4, velocity 100"
        on, off = "MIDI and CV out: on (the MIDI Out jack and the CV jacks send)", "MIDI and CV out: off (nothing is sent)"
        self.assertTrue(all(ok for ok, _ in self.judge("midi-thru", checks.Window(lines(on, note_in, note_out, off)))))
        got = dict((what, ok) for ok, what in self.judge("midi-thru", checks.Window(lines(on, note_in, off))))
        self.assertFalse(got["thru sent them out of MIDI OUT"])
        self.assertTrue(got["notes came in"])

    def test_first_start_reads_the_setup_since_power_on(self):
        start = [(6.0, "first start: setup, try 1 of 3"),
                 (6.1, "first start: asked for the backup (/run/mpfree-app/setup-backup.request)"),
                 (900.0, "first start: backup done: /run/mpfree-data/backup/mpc-one-emmc-20261116T031500Z"),
                 (1500.0, "first start: done: 52 Akai kits, 1630 samples; the backup is in "
                          "/run/mpfree-data/backup/mpc-one-emmc-20261116T031500Z")]
        got = checks.judge(checks.BY_NAME["first-start"], checks.Window([], start, 1600.0, 1600.0))
        self.assertEqual([ok for ok, _, _ in got], [True, True, True])
        failed = start[:2] + [(700.0, "first start: the backup did not work: the eMMC's write or discard "
                                      "counters moved during the copy (see /run/mpfree-setup/counters-*.txt)")]
        got = checks.judge(checks.BY_NAME["first-start"], checks.Window([], failed, 800.0, 800.0))
        self.assertEqual([ok for ok, _, _ in got], [True, False, False])
        self.assertIn("counters moved", got[1][2])
        got = checks.judge(checks.BY_NAME["first-start"], checks.Window([], [], 10.0, 10.0))
        self.assertEqual([ok for ok, _, _ in got], [False, False, False])
        self.assertIn("no line like", got[0][2])

    def test_mpc_plugin_install_reads_the_plugins_place_and_the_skin(self):
        """Lines as the app writes them (plugin_page.rs, main.rs, gui.rs); a direct cable with no route out."""
        window = lines(
            "plugins: 1 installed, 1 zip(s) on the stick, no catalog kept yet, downloads possible (libcurl found), "
            "OpenSSL 3.5.4 ready",
            "plugins: not done (catalog): The catalog didn't download: no network",
            "plugins: Installed Tidal Verb 1.6.0: Tidal Verb ready in + Add effect, under Plugins",
            "plugins: 2 offered after an install",
            "track 7: plays [SYN] Mono Voice",
            "plugins: [SYN] Mono Voice started in its own process",
            "screen: mpc:timncox.monovoice: its own screen, 2 tab(s), on the Synth tab")
        got = checks.judge(checks.BY_NAME["mpc-plugin-install"], checks.Window(window, window))
        self.assertEqual([ok for ok, _, _ in got], [True, True, True, True, True, False, True, True, True, True])
        self.assertEqual(got[5][1], "Modal Voice installed from the catalog (needs a route out)")
        # The final round's failure, as the app now says it: OpenSSL still can't make random numbers.
        unseeded = lines("plugins: 1 installed, 1 zip(s) on the stick, no catalog kept yet, downloads possible "
                         "(libcurl found), OpenSSL 3.5.4 not ready: its random numbers can't start (...)") + window[1:]
        got = checks.judge(checks.BY_NAME["mpc-plugin-install"], checks.Window(unseeded, unseeded))
        self.assertEqual([ok for ok, _, _ in got][:2], [True, False])
        refused = window + lines("plugins: [SYN] Modal Voice would not start: failed to map segment")
        got = checks.judge(checks.BY_NAME["mpc-plugin-install"], checks.Window(refused, refused))
        self.assertFalse(got[-1][0])
        # The static pad player on the MPC downloads through mpfree-fetch, and says whether it has it.
        for means in ("downloads possible (mpfree-fetch found)", "downloads off (no mpfree-fetch)"):
            helper = lines(f"plugins: 1 installed, 1 zip(s) on the stick, no catalog kept yet, {means}") + window[1:]
            got = checks.judge(checks.BY_NAME["mpc-plugin-install"], checks.Window(helper, helper))
            self.assertTrue(got[0][0], means)
            self.assertFalse(got[1][0], means)
        # The helper reports its libcurl's OpenSSL (mpfree-fetch --tls), or why it couldn't.
        for tls, ready in (("OpenSSL 3.5.4 ready", True),
                           ("OpenSSL 3.5.4 ready (its random numbers didn't start: x; MPFree loaded its default "
                            "provider)", True),
                           ("TLS not checked: mpfree-fetch gave no answer in 10 s", False)):
            helper = lines("plugins: 1 installed, 1 zip(s) on the stick, no catalog kept yet, downloads possible "
                           f"(mpfree-fetch found), {tls}") + window[1:]
            got = checks.judge(checks.BY_NAME["mpc-plugin-install"], checks.Window(helper, helper))
            self.assertEqual([ok for ok, _, _ in got][:2], [True, ready], tls)

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
        # Every button is built in: nothing to find, even with no map.conf.
        self.assertEqual(checks.not_found(chosen, ""), [])
        self.assertEqual(checks.not_found(checks.CHECKS, ""), [])
        # As on a build from before every button was built in.
        missing = checks.not_found(chosen, "track-mute = skipped\npad-bank-a-e = button 35\n", frozenset())
        self.assertEqual(missing, [("transport", "SAMPLER"), ("track-mute", "TRACK MUTE"),
                                   ("track-mute", "PAD BANK B/F")])

    def test_built_in_is_every_button_in_the_apps_list(self):
        """BUILT_IN names every control of the app's Map controls list but the spare Other rows."""
        path = os.path.join(checks.RUST_DIR, "mpfree-app", "src", "app", "mapper.rs")
        with open(path, encoding="utf-8") as fh:
            names = re.findall(r'^\s*c\("([^"]+)", Job::', fh.read(), re.M)
        self.assertEqual(len(names), 42)
        self.assertEqual(checks.BUILT_IN, frozenset(n for n in names if not n.startswith("Other ")))
        for c in checks.CHECKS:
            for name in c.needs:
                self.assertIn(name, names, c.name)


if __name__ == "__main__":
    unittest.main()
