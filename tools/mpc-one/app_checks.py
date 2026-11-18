#!/usr/bin/env python3
"""The device checks: what a round asks on the MPC, by name, and the smoke tests behind them.

A device round tests only what changed. Everything the app does on its own
(its pages, buttons, drags, undo, songs and engines) is checked by the app's
own tests on every pull request: the UI smoke library in
rust/mpfree-app/src/smoke/ and the unit tests beside the code. What only the
MPC can show is checked here, by name:

  - that the MPC's own buttons, pads, wheel, Q-Links, touchscreen and MIDI IN
    reach the app (the tool reads the app's log and says what it saw),
  - what only a person can see or hear: lights, sound, a drag following a
    finger,
  - how the sound holds up on the MPC's own cores (the app's status lines),
  - what the stick kept from one boot to the next (partition 4).

A check is one of two kinds:

  hands  a few steps on the MPC. The tool shows them, waits while you do
         them, then reads the app's log for that time and says, line by
         line, what it saw. It asks at most one question, about what only
         you can see or hear.
  auto   no steps and no question: the tool reads the log and the stick and
         gives the result.

A round names its checks in MPFREE_APP_CHECKS (comma-separated, in the order
they run) for `collect-over-ssh.sh --app` with MPFREE_APP_STEPS=app. With no
checks named, the app step asks nothing: it reports the start-up and the
session from the log. The checks' smoke tests run on the workstation with
`app_checks.py smoke`, which runs the app's whole test suite (about 6
minutes) and reports it by check and by area.

  python3 tools/mpc-one/app_checks.py list               every check and area
  python3 tools/mpc-one/app_checks.py show NAME[,NAME]   a check's steps, log lines and tests
  python3 tools/mpc-one/app_checks.py smoke              the app's tests, by area (workstation)

How to add a check: docs/dev/hardware/device-checks.md. Keep this file ASCII.
"""
import os
import re
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.dirname(os.path.dirname(HERE))
RUST_DIR = os.path.join(REPO_ROOT, "rust")

# The app's log, as host_app_mpc.py reads it: MIDI lines from the controller end in these.
PAD_HIT = re.compile(r"= pad (\d+) down, velocity (\d+)$")
BUTTON = re.compile(r"= button (\d+) pressed$")
WHEEL = re.compile(r"= data wheel ([+-]\d+)$")
QLINK = re.compile(r"= .*Q-Link")
# The pad player's totals, every 30 s ("status: N hits (...), R repeats, U underruns in P periods, ...").
STATUS = re.compile(r"^status: (\d+) hits .*?, (\d+) underruns in (\d+) periods")
# #273's two underruns came at periods 524,292 (25.4 min) and 1,048,580 (50.7 min); an hour is 1,240,312.
HOUR_PERIODS = 1_240_312
PERIOD_S = 128 / 44100

# The MPC One's buttons as the app knows them from the start (mpfree_controller::mpc_one::button).
REC, OVERDUB, UNDO, PLAY, STOP, PLAY_START = 73, 80, 67, 82, 81, 83


class Window:
    """What a check is judged on: the app's log lines in its time, the whole session, and the reads beside it."""

    def __init__(self, lines, session=None, start=0.0, end=0.0, before=None, after=None, settings=None):
        self.lines = lines            # [(seconds since boot, message)] from the check's start to its end
        self.session = session if session is not None else lines
        self.start, self.end = start, end
        self.before, self.after = before or {}, after or {}   # touch interrupts and the like
        self.settings = settings      # partition 4's settings/app.conf, as text, or None (unread)

    def messages(self):
        return [m for _, m in self.lines]


# A note from a keyboard or controller on any port, as the app logs each message it reads: "+3.210 s MIDI In:
# 90 25 64 = note on ..." (MusicIn::bytes, rust/mpfree-app/src/midi_in.rs). The session's "MIDI in: ..." lines
# are summaries at the end, so they never fall in a check's time.
NOTE_IN = r"^\+\d+\.\d+ s .+: [0-9A-F]{2}( [0-9A-F]{2})* = note on "

# A control MIDI Learn took, as MusicIn::bytes logs it: "+3.210 s Keystep 37: MIDI Learn: CC 74 . ch 1 . Keystep 37
# takes t1.inst.12" (its target as a ParamAddr; the transport's are "transport.N").
LEARNED = r"^\+\d+\.\d+ s .+: MIDI Learn: "


def seen(*patterns):
    """Passes when each pattern matches a line of the check's time (any order)."""
    regexes = [re.compile(p) for p in patterns]

    def test(w):
        missing = [r.pattern for r in regexes if not any(r.search(m) for m in w.messages())]
        return (not missing, "" if not missing else "not in the log: " + "; ".join(missing))
    return test


def seen_times(pattern, least):
    """Passes when at least `least` lines of the check's time match `pattern`."""
    regex = re.compile(pattern)

    def test(w):
        n = sum(1 for m in w.messages() if regex.search(m))
        return n >= least, f"{n} time{'s' if n != 1 else ''}" + ("" if n >= least else f", {least} expected")
    return test


def not_seen(pattern):
    """Passes when no line of the check's time matches `pattern`; if one does, says it."""
    regex = re.compile(pattern)

    def test(w):
        bad = [m for m in w.messages() if regex.search(m)]
        return not bad, bad[0] if bad else "none"
    return test


def at_start(pattern, about):
    """Passes when a line since power-on matches `pattern`; if not, says the last line that matches `about`."""
    regex, near = re.compile(pattern), re.compile(about)

    def test(w):
        if any(regex.search(m) for _, m in w.session):
            return True, ""
        last = [m for _, m in w.session if near.search(m)]
        return False, last[-1] if last else f"no line like {about!r} since power-on"
    return test


def pressed(number, then=None):
    """Button `number` reached the app, and (if given) a line matching `then` came after it."""
    after = re.compile(then) if then else None

    def test(w):
        times = [t for t, m in w.lines if (b := BUTTON.search(m)) and int(b.group(1)) == number]
        if not times:
            return False, f"button {number} never reached the app"
        if after and not any(t >= times[0] and after.search(m) for t, m in w.lines):
            return False, f"button {number} reached the app, but no line like {then!r} followed"
        return True, f"button {number} x{len(times)}"
    return test


def pad_hits(least=8, soft=60, hard=100):
    """At least `least` pad hits, softer than `soft` and harder than `hard` among them."""
    def test(w):
        v = [int(h.group(2)) for m in w.messages() if (h := PAD_HIT.search(m))]
        pads = {int(h.group(1)) for m in w.messages() if (h := PAD_HIT.search(m))}
        if not v:
            return False, "no pad hits reached the app"
        text = f"{len(v)} hits on {len(pads)} pads, velocities {min(v)} to {max(v)}"
        return len(v) >= least and min(v) < soft and max(v) > hard, text
    return test


def wheel_turned():
    def test(w):
        turns = [int(t.group(1)) for m in w.messages() if (t := WHEEL.search(m))]
        return bool(turns), f"{len(turns)} wheel steps, net {sum(turns):+d}" if turns else "no wheel steps"
    return test


def qlinks_turned():
    def test(w):
        n = sum(1 for m in w.messages() if QLINK.search(m))
        return n > 0, f"{n} Q-Link messages" if n else "no Q-Link messages"
    return test


def touch_rose():
    """The touchscreen's interrupt count went up while you touched it (read before and after)."""
    def test(w):
        a, b = w.before.get("touch"), w.after.get("touch")
        if a is None or b is None:
            return None, "the touch interrupt count could not be read"
        return b > a, f"touch interrupts {a} -> {b}"
    return test


def statuses(lines):
    """[(t, hits, underruns, periods)] for every status line."""
    out = []
    for t, m in lines:
        s = STATUS.match(m)
        if s:
            out.append((t, int(s.group(1)), int(s.group(2)), int(s.group(3))))
    return out


def no_underruns_while():
    """No underrun between the last status line before the check and the first after it."""
    def test(w):
        st = statuses(w.session)
        before = [s for s in st if s[0] <= w.start]
        after = [s for s in st if s[0] >= w.end]
        if not after:
            return None, "no status line after the check yet (they come every 30 s)"
        a = before[-1] if before else (0.0, 0, 0, 0)
        b = after[0]
        if b[3] < a[3]:
            return False, "the period count went back: the pad player restarted during the check"
        new = b[2] - a[2]
        return new == 0, f"{new} underruns in {b[3] - a[3]:,} periods ({(b[3] - a[3]) * PERIOD_S:.0f} s around the check)"
    return test


# The time engine's costs on the status line, totals since the start: "compiles: N (slowest X ms, N notes and
# locks); step edits: N in place (slowest X ms), M compiled (slowest Y ms)".
TIME_COSTS = re.compile(r"compiles: (\d+) \(slowest ([\d.]+) ms, (\d+) notes and locks\); "
                        r"step edits: (\d+) in place \(slowest ([\d.]+) ms\), (\d+) compiled \(slowest ([\d.]+) ms\)")
# A step edit made in place may cost the audio thread a tenth of a 128-frame period (2.9 ms) at most.
IN_PLACE_MS = 0.29


def step_edits_in_place():
    """Step edits went into the playing clip in place, each within IN_PLACE_MS, by the first status line after
    the check against the last before it; says what they and a compile cost on the MPC (Gate 0's numbers)."""
    def test(w):
        costs = [(t, TIME_COSTS.search(m)) for t, m in w.session]
        before = [c for t, c in costs if c and t <= w.start]
        after = [c for t, c in costs if c and t >= w.end]
        if not after:
            return None, "no status line after the check yet (they come every 30 s)"
        a, b = (before[-1] if before else None), after[0]
        was = (lambda g: int(a.group(g)) if a else 0)
        in_place, compiled = int(b.group(4)) - was(4), int(b.group(6)) - was(6)
        text = (f"{in_place} step edits in place (slowest {b.group(5)} ms), {compiled} compiled (slowest "
                f"{b.group(7)} ms); the slowest compile {b.group(2)} ms, with {b.group(3)} notes and locks playing")
        return in_place > 0 and float(b.group(5)) <= IN_PLACE_MS, text
    return test


def hour_long():
    """The pad player has played for an hour with no underrun and no restart."""
    def test(w):
        st = statuses(w.session)
        if not st:
            return None, "no status line yet"
        last = st[-1]
        dropped = any(b[3] < a[3] for a, b in zip(st, st[1:]))
        marks = [m for _, m in w.session if m.startswith("underrun at period")]
        text = f"{last[2]} underruns in {last[3]:,} periods ({last[3] * PERIOD_S / 60:.0f} min)"
        if dropped:
            return False, text + "; the period count went back, so the pad player restarted"
        if last[3] < HOUR_PERIODS:
            left = (HOUR_PERIODS - last[3]) * PERIOD_S / 60
            return None, text + f"; NOT YET an hour: run this check again in {left + 1:.0f} min"
        if marks:
            text += "; " + "; ".join(marks[:4])
        return last[2] == 0 and not marks, text
    return test


def setting(text, key):
    """The value of `key` in a settings file's text (key = value lines), or None."""
    for line in (text or "").splitlines():
        m = re.match(rf"^\s*{re.escape(key)}\s*=\s*(.*?)\s*$", line)
        if m:
            return m.group(1)
    return None


def stayed_up():
    """No PANIC or safe start, and the pad player did not stop or restart during the check."""
    def test(w):
        bad = [m for m in w.messages() if m.startswith(("PANIC: ", "safe start: "))
               or (m.startswith("(systemd) ") and re.search(r"exited|failed|Started|Stopped", m))]
        return not bad, "; ".join(bad[:3]) if bad else "no PANIC, no restart"
    return test


def kept_settings():
    """What partition 4 keeps came back at this start: the app's "saved settings:" line agrees with the file."""
    def test(w):
        if w.settings is None:
            return None, "settings/app.conf could not be read from partition 4"
        starts = [m for _, m in w.session if m.startswith("saved settings: ")]
        if not starts:
            return False, "no 'saved settings:' line since power-on: the app started with its defaults"
        line = starts[-1]
        cores = setting(w.settings, "sound-cores") or "shared"
        minutes = setting(w.settings, "loop-memory")
        problems = []
        if f", sound cores {cores}," not in line:
            problems.append(f"the file says sound cores {cores}")
        if minutes and f", loop memory {minutes} min," not in line:
            problems.append(f"the file says loop memory {minutes} min")
        running = [m for _, m in w.session if m.startswith("sound cores: ") and ": the audio thread" in m]
        if running and not running[-1].startswith(f"sound cores: {cores}:"):
            problems.append(f"it runs as {running[-1][len('sound cores: '):].split(':')[0]}")
        shown = re.search(r"sound cores [^,]+, loop memory [^,]+", line)
        text = shown.group(0) if shown else line[:120]
        return not problems, text + ("; " + "; ".join(problems) if problems else "; the same in settings/app.conf")
    return test


class Check:
    def __init__(self, name, title, why, kind, steps=(), looks=(), ask=None, smoke=(), needs=(), probe=False,
                 wait_status=False):
        self.name, self.title, self.why, self.kind = name, title, why, kind
        self.steps = list(steps)
        self.looks = list(looks)      # [(what the tool looks for, test)]
        self.ask = ask                # the one question, or None
        self.smoke = list(smoke)      # the tests that check its software side
        self.needs = list(needs)      # buttons it uses that only Map controls finds (their names in its list)
        self.probe = probe            # read the cores and clock after it (CPU_PROBE)
        self.wait_status = wait_status  # wait for the app's next status line before judging


# What the mixer and the chain editor log when they refuse an effect (a full chain, a busy mixer, no maker,
# a mode switch that came too late).
CHAIN_REFUSED = (r"^(mixer|chain): (The chain is full|The mixer is busy"
                 r"|the chain editor asked for an effect that has no maker"
                 r"|an effect was switched after it was taken out)")

CHECKS = (
    Check(
        "transport", "REC, OVERDUB, UNDO and PLAY on a fresh start",
        "#387: these buttons are known from the start, with no Map controls", "hands",
        ["On Play, tap Loop in the view switch: it arms the loop (REC -> Loop 1). Turn the volume down a little.",
         "1. Press REC once: REC and PLAY blink.",
         "2. Press PLAY: PLAY stays lit, REC keeps blinking while the first pass records. Play a few pads.",
         "3. After about two bars, press REC: at the end of that bar REC goes dark and the loop plays back.",
         "4. Press OVERDUB: it blinks. Play a few more pads. 5. Press OVERDUB again: it goes dark.",
         "6. Press UNDO: the second layer goes. 7. Press STOP: PLAY goes dark.",
         "8. SHIFT + SAMPLER opens the Looper tab. Press REC: OVERDUB and PLAY blink (an overdub on",
         "   the loop waits for PLAY). Press STOP: both go dark."],
        [("REC (73) armed the loop", pressed(REC, r"^looper: Record")),
         ("PLAY (82) reached the app", pressed(PLAY)),
         ("OVERDUB (80) turned overdub on", pressed(OVERDUB, r"^looper: overdub on")),
         ("OVERDUB again turned it off", seen(r"^looper: overdub off")),
         ("UNDO (67) took the layer back", pressed(UNDO, r"^looper: Undo")),
         ("STOP (81) reached the app", pressed(STOP))],
        "Did the lights do what each step says? If not, which step?",
        ["rec_play_and_overdub_record_the_loop_and_light_on_a_fresh_mpc",
         "rec_on_the_looper_tab_arms_the_loop_on_a_fresh_mpc",
         "stop_lets_go_of_a_loop_waiting_for_play",
         "the_transport_buttons_work_with_no_saved_map_and_through_old_skips",
         "a_fresh_mpfree_knows_rec_overdub_undo_and_play_start",
         "the_transport_lights_follow_play_record_and_overdub_as_akais_do",
         "the_transport_buttons_drive_the_time_engine"],
        needs=['SAMPLER']),
    Check(
        "basics", "Pads, banks, Full Level, the wheel, Q-Links and touch reach the app",
        "a new MPC OS, kernel or boot route: the controller and the screen work on it", "hands",
        ["1. On Play, hit every pad, some softly and some hard. Press PAD BANK B, then A.",
         "2. Press FULL LEVEL (it lights), hit a pad softly, and press FULL LEVEL until its light goes out.",
         "3. Turn the data wheel a few steps and turn Q-Link 1.",
         "4. Press TRACK MIX for the Mix tab and turn Q-Links 1 to 4. Tap Effects, tap an effect in a chain",
         "   to open the chain editor and turn Q-Links 1 to 4 there too. Press MAIN to go back to Play.",
         "5. Tap a few things on the screen."],
        [("pad hits, soft and hard", pad_hits()),
         ("PAD BANK changed the bank", seen(r"\bbank:? [A-H]\b")),
         ("FULL LEVEL on and off", seen(r"^pad level: Full Level", r"^pad level: as played")),
         ("the data wheel", wheel_turned()),
         ("the Q-Links", qlinks_turned()),
         ("the touchscreen", touch_rose())],
        "Did every pad sound and light when you hit it, and did PAD BANK and FULL LEVEL light?",
        ["full_and_half_level_toggle_with_their_light",
         "pad_banks_pick_e_to_h_with_shift_or_a_second_press",
         "curves_keep_the_ends_and_bend_the_middle",
         "shift_and_the_wheel_on_play_change_the_kit",
         "track_mix_opens_the_mixer_and_the_q_links_turn_the_selected_strip",
         "the_chain_editor_takes_the_mixers_place_and_the_q_links_turn_its_knobs",
         "touches_become_actions_once_ready",
         "touch_reports_turn_into_screen_fingers"],
        needs=['PAD BANK A/E', 'PAD BANK B/F', 'TRACK MIX / PAD MIX', 'MAIN']),
    Check(
        "load", "The heaviest sounds on the MPC's own cores",
        "performance: underruns and render time under the worst case", "hands",
        ["1. Hold SHIFT and turn the wheel to the last kit, \"Synth: Poly\". Tap the Synth tab, pick the",
         "   Supersaw preset, and hold 8 pads for about 10 s.",
         "2. Hold SHIFT and turn the wheel to the kit called \"Grains: ...\". Hold NOTE REPEAT and 8 pads",
         "   together for about 10 s, and let go.",
         "3. Press STOP. Hold SHIFT and turn the wheel back to kit 1."],
        [("pads held", pad_hits(least=8, soft=128, hard=0)),
         ("no underrun while it played", no_underruns_while())],
        "Did you hear any crackles or dropouts?",
        ["the_sound_path_never_allocates_with_the_sampler",
         "the_sound_path_never_allocates_with_the_synth",
         "the_sound_path_never_allocates_on_two_cores",
         "the_sound_path_never_allocates_on_three_cores_with_the_synth"],
        probe=True, wait_status=True),
    Check(
        "track-mute", "TRACK MUTE: the pads as track mutes, with their colours",
        "#332: pad colours for tracks 1-16 and PAD BANK B to H past 16 tracks", "hands",
        ["1. If Arrange has no clip yet: on Play, tap Beats and light the kick on steps 1, 5, 9 and 13 (a clip",
         "   on track 1). Press PLAY.",
         "2. Press TRACK MUTE: it lights. Pad colours: bright green plays a clip, dim green has nothing launched,",
         "   red is muted, dark has no track. Hit pads 1 and 2 to mute them, then again to unmute.",
         "3. Press PAD BANK B: the pads show tracks 17-32, or stay dark if there are none. PAD BANK A goes back.",
         "4. Press SHIFT + TRACK MUTE: each pad mutes itself in the kit (red while muted). Press it again,",
         "   then TRACK MUTE to put the pads back. STOP."],
        [("TRACK MUTE muted and unmuted tracks", seen(r"^track mute: .* muted", r"^track mute: .* plays")),
         ("PAD BANK picked another 16 tracks", seen(r"^TRACK MUTE \S+ bank [B-H]: "))],
        "Were the pad colours right for each track, muted, playing and empty?",
        ["track_mute_turns_the_pads_into_mute_keys_over_any_page",
         "track_mute_banks_pick_which_16_tracks_the_pads_mute",
         "track_mute_turns_the_pads_into_track_mutes_and_back",
         "track_mute_takes_the_perc_out_of_the_hook_and_back",
         "track_mute_banks_reach_past_16_tracks_and_shift_mutes_the_kits_pads",
         "track_mute_writes_its_mutes_and_its_bank"],
        needs=['TRACK MUTE', 'PAD BANK A/E', 'PAD BANK B/F']),
    Check(
        "step-pads", "STEP SEQ: the pads as steps, with their lights",
        "S1: STEP SEQ turns the pads into the steps of one sound (sequencer decision 2)", "hands",
        ["1. On Play, hit pad 1 (the kick). Press STEP SEQ: it lights, Beats shows, and pads 1, 5, 9 and 13",
         "   glow faintly (the first step of each beat).",
         "2. Hit pads 1, 5, 9 and 13, one hard and one soft: they light in the kick's colour, harder brighter.",
         "   Press PLAY: a white light walks across the pads.",
         "3. Hit pad 5 again and let go: it goes dark. Hold ERASE and hit pad 9: it goes dark too.",
         "4. Press PAD BANK B: the pads go dark (page 2 is past the one-bar clip), B lit, A dim. PAD BANK A.",
         "5. Hold STEP SEQ, hit pad 2 (it sounds), let go: the pads show pad 2's steps. Press STEP SEQ: the",
         "   pads play again and STEP SEQ goes dark. STOP."],
        [("STEP SEQ made the pads steps", seen(r"^STEP SEQ: the pads are pad [A-H]?\d+'s steps")),
         ("a step pad put a note on", seen(r"^steps: .*step \d+ of pad \d+ on")),
         ("a step pad took one off", seen(r"^steps: step \d+ of pad \d+ off")),
         ("PAD BANK turned the page", seen(r"^STEP SEQ \S+ page 2 "))],
        "Did the pad lights (glow, velocity, white playhead) and the PAD BANK lights look as the steps say?",
        ["step_seq_turns_the_pads_into_steps_until_pressed_again",
         "step_seq_makes_the_pads_the_steps_of_the_sound_played_last",
         "step_seq_shows_its_chip_and_the_lane_on_the_pads",
         "presses_edit_the_clip_beats_shows",
         "the_pads_light_each_step_as_the_design_says",
         "the_page_shown_is_lit_and_the_page_playing_blinks"],
        needs=['STEP SEQ', 'ERASE', 'PAD BANK A/E', 'PAD BANK B/F']),
    Check(
        "step-hold", "STEP SEQ: a step pad held takes the Q-Links; ties and COPY",
        "S1: hold a step to set it, tap a later one for a tie, COPY steps (sequencer decision 2)", "hands",
        ["1. On Play, hit pad 1 (the kick) and press STEP SEQ. Hit pads 1 and 5 softly: they light dim.",
         "2. Hold pad 1: the Q-Link cards say Step 1. Turn Q-Link 1 right a few clicks: pad 1 gets brighter.",
         "   Press Q-LINK while still holding it: the cards show Condition and Ratchet. Let go: pad 1 stays lit.",
         "3. Hold pad 1 and hit pad 3, then let go of both: pads 2 and 3 glow dim (the note lasts to step 3).",
         "4. Hold COPY, hit pad 5, then pads 9 and 13: they light like pad 5. Let go of COPY. Press UNDO: pad",
         "   13 goes dark. Press STEP SEQ: the pads play again."],
        [("a held step took the Q-Links", seen(r"^steps held: Step \d+")),
         ("Q-LINK showed the Trig bank", seen(r"^q-link bank: Trig")),
         ("a later step made a tie", seen(r"^steps: step \d+ of pad \d+ lasts to the end of step \d+")),
         ("COPY copied a step", seen(r"^steps: step \d+ of pad \d+ to step \d+ copied"))],
        "Did the held step get brighter, did pads 2 and 3 glow for the tie, and did the copies light like pad 5?",
        ["held_step_pads_set_tie_and_copy_steps",
         "held_step_pads_take_the_q_links_and_q_link_steps_to_trig",
         "holding_steps_sets_them_ties_them_and_keeps_them",
         "copy_puts_the_step_picked_on_each_step_tapped",
         "held_steps_turn_as_one_undo_and_copy_is_one_take"],
        needs=['STEP SEQ', 'COPY', 'Q-LINK']),
    Check(
        "step-locks", "STEP SEQ: locks on held steps, heard and lit",
        "S1: a held step's Sound bank locks the kick's tune and decay; SHIFT + an empty step; ERASE takes locks off",
        "hands",
        ["1. On Play, hit pad 1 (the kick), press STEP SEQ, hold ERASE and hit each lit pad (it goes dark), let go.",
         "   Hit pads 1, 5, 9 and 13, then press PLAY: four kicks a bar.",
         "2. Hold pad 5, press Q-LINK twice (the cards say Sound) and turn Q-Link 1 right to +5 st: the second kick",
         "   plays higher, and pad 5 turns purple. Let go. 3. Hold pad 9, press Q-LINK twice and turn Q-Link 2 left",
         "   to about 100 ms: the third kick is cut short. Let go. 4. Hold SHIFT, hit pad 7 and keep it held, let go",
         "   of SHIFT: the cards say Sound. Turn Q-Link 3 left to about 500 Hz, let go: pad 7 glows dim purple.",
         "5. Hold pad 5 and press ERASE: the second kick is back to normal, pad 5 in the kick's colour. Let go.",
         "   Press UNDO: it plays higher again. Press STOP, then STEP SEQ: the pads play again."],
        [("Q-LINK showed the Sound bank", seen(r"^q-link bank: Sound")),
         ("a turn locked a held step", seen(r"^steps: step \d+ of pad \d+: lock pad\d+\.\d+ ")),
         ("SHIFT held an empty step", seen(r"^steps: step \d+ of pad \d+ held for a lock with no note")),
         ("ERASE took a step's locks off", seen(r"^ERASE: the locks of step \d+ are off"))],
        "Did the locked kicks sound higher and shorter on their own steps only, with no click, and did pads 5 and 7 turn purple?",
        ["held_step_pads_lock_the_kit_and_erase_takes_the_locks_off",
         "step_locks_write_their_lines",
         "held_step_pads_take_the_q_links_and_q_link_steps_to_trig",
         "held_steps_lock_the_synth_from_its_tab_and_the_sound_bank",
         "shift_holds_an_empty_step_and_erase_takes_the_held_steps_locks_off",
         "held_step_locks_reach_the_pads_before_their_note_and_end_at_the_next"],
        needs=['STEP SEQ', 'ERASE', 'Q-LINK']),
    Check(
        "step-synth", "STEP SEQ on the synth: the chord picked with STEP SEQ held goes on the steps",
        "S1: synth tracks on the step pads, the line and its pen", "hands",
        ["1. Tap the Synth tab (the pads play the synth), then press STEP SEQ: Beats shows a Line lane on top.",
         "2. Hold STEP SEQ, hold pads 1 and 5 together, let go of them, then let go of STEP SEQ: the chip at the",
         "   top names the two notes. 3. Hit pads 1, 5, 9 and 13 and let go: each plays the chord once.",
         "4. Press PLAY: the chord four times a bar. Hit pad 5 and let go: that chord goes.",
         "5. Press STOP, then STEP SEQ: the pads play again."],
        [("the pads were the line's steps", seen(r"^STEP SEQ: the pads are the line's steps")),
         ("a step took the chord", seen(r"step \d+ of the line: pads 1, 5 on$")),
         ("a lit step came off whole", seen(r"^steps: step 5 of the line erased$"))],
        "Did each step play both notes of the chord, and did the chord on pad 5 stop once you took it off?",
        ["step_seq_on_the_synth_puts_the_pen_on_the_line",
         "step_synth_writes_its_lines",
         "step_seq_on_the_synth_takes_the_line_and_the_pen_from_the_pads_held",
         "on_the_synth_a_step_takes_the_pen_and_comes_off_whole"],
        needs=['STEP SEQ']),
    Check(
        "step-notes", "STEP SEQ on the synth: Q-Link 1 moves a held step's note",
        "S1: a synth step's Note knob, a pad a click", "hands",
        ["1. Tap the Synth tab (the pads play the synth), then press STEP SEQ. Hold STEP SEQ, hit pad 1, and let",
         "   go of STEP SEQ. Hit pad 1 and let go: step 1 plays C3.",
         "2. Hold pad 1 and turn Q-Link 1 slowly to the right until its card says D3, then let go of pad 1.",
         "3. Press PLAY: step 1 plays D3. Press STOP, then STEP SEQ: the pads play again."],
        [("the pads were the line's steps", seen(r"^STEP SEQ: the pads are the line's steps")),
         ("Q-Link 1 moved the step's note", seen(r"^steps: step 1 of the line: lowest note on pad \d+$"))],
        "Did step 1 play a tone higher (D3, not C3) after the turn?",
        ["on_the_synth_the_note_knob_and_the_keys_set_a_steps_notes",
         "step_notes_and_keys_write_their_lines"],
        needs=['STEP SEQ']),
    Check(
        "step-keys", "STEP SEQ on the synth: a keyboard's keys on a held step (only with a keyboard)",
        "S1: hold a synth step and play a MIDI keyboard on MIDI IN or the USB socket: its notes go on the step",
        "hands",
        ["1. Plug a keyboard into MIDI IN or the USB socket. Tap the Synth tab, then press STEP SEQ. Hold STEP SEQ,",
         "   hit pad 1, and let go of STEP SEQ. Hit pads 1 and 5 and let go: steps 1 and 5 play C3.",
         "2. Hold pad 1. On the keyboard, the C an octave below middle C is pad 1: play the E and G above it",
         "   together, let go of the keys, then of pad 1.",
         "3. Press PLAY: step 1 plays E and G, step 5 still C. Press STOP, then STEP SEQ."],
        [("keys went on a held step", seen(r"^steps: step 1 of the line: pads [\d, ]+ from the keys on$"))],
        "Did step 1 play the two notes you played, and step 5 still C3?",
        ["on_the_synth_the_note_knob_and_the_keys_set_a_steps_notes",
         "step_notes_and_keys_write_their_lines",
         "keys_played_while_line_steps_are_held_go_on_them"],
        needs=['STEP SEQ']),
    Check(
        "rec-arm", "REC records what is armed: REC then PLAY records the pads; the Loop view arms the loop",
        "S2: sequencer decision 1, REC records what is armed and the transport bar names it", "hands",
        ["1. On Play, tap Beats, then All: the REC chip, between the tempo and the bar dots, says REC -> Track 1.",
         "2. Press REC: REC and PLAY blink, the chip says READY, and nothing plays yet.",
         "3. Press PLAY: the chip turns solid red as it records. Play pads 1 to 4 for about two bars, then STOP.",
         "4. Press PLAY: what you played plays back. Press STOP, then UNDO: the take goes.",
         "5. Press REC, then PLAY, and once the chip is solid red hit pad 1 a few times. Press UNDO while it",
         "   records: the chip turns grey and the music plays on. Press STOP.",
         "6. Tap Loop: the chip says REC -> Loop 1. Press REC: REC and PLAY blink (OVERDUB and PLAY if the loop",
         "   has a take already): it waits for PLAY. Press STOP: they go dark. Tap All."],
        [("REC (73) readied the notes", pressed(REC, r"^rec: ready, PLAY records ")),
         ("PLAY (82) started the take", pressed(PLAY, r"^time: transport: Record, ")),
         ("UNDO (67) took the take back", pressed(UNDO, r"^Undid take ")),
         ("UNDO while recording threw the take away", seen(r"^time: undo: take \d+ thrown away$")),
         ("the Loop view armed the loop", seen(r"^rec: the loop armed$")),
         ("REC then went to the loop", seen(r"^looper: (Record: armed|overdub on)"))],
        "Did PLAY play back the pads you recorded after REC and PLAY?",
        ["rec_arm_writes_its_lines",
         "rec_records_what_is_armed_and_play_starts_a_ready_take",
         "arranges_bar_rec_and_play_record_what_is_armed",
         "rec_on_beats_shows_hits_as_they_go_in_and_undo_takes_them_out",
         "undo_while_recording_throws_the_take_away_and_the_music_plays_on"]),
    Check(
        "loop-tracks", "Loop tracks: a recorded loop lands on Loops A and launches like a clip, on the bar line",
        "S2: sequencer decision 3, Loop tracks in the Clips grid (part 2: the model and launching)", "hands",
        ["1. Tap Looper at the top of the screen. If Loop 1's ring has a wave round it, tap Loop 1's name, then",
         "   Clear (outlined in red, at the right of the loop's header): the loop and its cell on Loops A go.",
         "   Then tap Play.",
         "2. On Play, tap Loop. Press REC, then PLAY, and play pads 1 to 4 for about two bars. Press REC:",
         "   the loop plays.",
         "3. Tap Arrange, then the > at the top right of the grid: the column Loops A has Loop 1 in row 1.",
         "4. Tap row 2's number: the loop stops on the next bar line.",
         "5. Tap Loop 1's cell: it comes back in on the next bar line. Press STOP."],
        [("the loop landed on Loops A", seen(r"^time: Loop 1 on Loops A, row 1, playing$")),
         ("row 2 stopped it on its bar line", seen(r"^looper: loop 1 off from bar \d+$")),
         ("its cell brought it back on a bar line", seen(r"^looper: loop 1 on from bar \d+$"))],
        "Did the loop stop and come back in exactly on the bar line, in time and with no click?",
        ["loop_tracks_write_their_lines",
         "a_recorded_loop_lands_on_a_loop_track_and_launches_like_a_clip",
         "a_loop_cell_launches_its_loop_on_the_bar_line_as_a_clip_does",
         "a_recorded_loop_lands_on_a_loop_track_and_its_cells_follow_it"]),
    Check(
        "loop-cell", "Loop tracks: SHIFT + + Track makes a Loop track, and a loop records into the cell tapped",
        "S2: sequencer decision 3, Loop tracks part 4: recording a loop from the grid", "hands",
        ["1. Tap Arrange. Hold SHIFT and tap + Track: a Loop track is added (Loops A, or Loops B after",
         "   loop-tracks). Tap the > at the top right of the grid until its column shows.",
         "2. Tap its row 2 cell: it turns red and says Armed.",
         "3. Press REC, then PLAY, and play pads 1 to 4 for about two bars. Press REC: the loop plays in",
         "   that cell. Press STOP."],
        [("SHIFT + + Track made a Loop track", seen(r"^time: Loops [A-Z] added, a Loop track")),
         ("its row 2 cell armed", seen(r"^time: loop cell: Loops [A-Z] row 2 armed")),
         ("the loop landed in it", seen(r"^time: Loop \d+ on Loops [A-Z], row 2, playing$"))],
        "Did the loop you played land in that row 2 cell and play back in time?",
        ["loop_cell_writes_its_lines",
         "a_loop_records_into_the_cell_tapped_on_the_grid",
         "an_armed_loop_cell_takes_the_next_loop_recorded",
         "a_new_loop_goes_into_a_free_one_or_says_why_not"],
        needs=['SHIFT']),
    Check(
        "loop-copy", "Loop tracks: SHIFT + a loop's cell dropped on another Loop track copies the loop there",
        "S2: sequencer decision 3, Loop tracks part 6: the same loop in more than one row, a copy as a new loop", "hands",
        ["1. After loop-cell: tap Arrange and the > at the top right of the grid until Loops A and the Loop",
         "   track loop-cell added both show, with Loop 1 playing (tap its cell if it isn't).",
         "2. Hold SHIFT and drag Loop 1's cell onto an empty row 3 cell of the other Loop track; let go of",
         "   the cell, then SHIFT. It says copying in, then shows the loop's wave.",
         "3. Tap that new cell: on the next bar line it plays along with Loop 1. Press STOP.",
         "4. Tap the square at the top of the row numbers (it stops every track, Loop tracks too): the loop",
         "   cells go dark, so the next checks start quiet."],
        [("the cell took a copy", seen(r"^clips: Loop \d+ copies into Loop \d+ on Loops [A-Z] / 3")),
         ("the copy's layers all went in", seen(r"^looper: loop \d+ is a copy of loop \d+, \d+ layers?$")),
         ("the copy came in on a bar line", seen(r"^looper: loop \d+ on from bar \d+$"))],
        "Did the copy come in on the bar line and play along with Loop 1 in time, with no click or dropout while it copied?",
        ["loop_copy_writes_its_lines",
         "a_loop_copied_onto_another_loop_track_is_a_new_loop_with_its_layers",
         "the_same_loop_in_two_rows_plays_on_and_a_copy_takes_a_new_loop",
         "a_whole_loop_copies_into_an_empty_one_a_layer_at_a_time"],
        needs=['SHIFT']),
    Check(
        "looper-deck", "The Looper tab's rings, speaker and Q-Links",
        "the Looper tab as an instrument, step 1 (#525)", "hands",
        ["1. Tap Looper at the top of the screen. If Loop 1's ring (the first) has a wave round it, tap Loop 1's",
         "   name, then Clear (outlined in red, at the right of the loop's header under the rings).",
         "2. If the song is stopped, press PLAY. Tap Loop 1's ring: it goes red and dashed until the next bar,",
         "   then red while it records. Play pads for about 2 bars, then tap the ring again: the loop plays.",
         "3. Tap the ring once more: it turns amber (an overdub). Play a few pads, then tap the ring again.",
         "4. Tap the speaker under Loop 1's ring: the ring goes grey and silent. Tap it again: it plays.",
         "5. Turn Q-Link 1: Loop 1 gets louder or quieter. Press Q-LINK twice, then turn Q-Link 2.",
         "6. Hold Loop 1's ring for two seconds: its arc drains and the loop fades out. Press STOP."],
        [("the ring armed the first pass", seen(r"^looper: Record: armed")),
         ("the second tap ended it", seen(r"^looper: Record during the first pass")),
         ("the ring overdubbed", seen(r"^looper: overdub on \(layer 2\)", r"^looper: overdub off")),
         ("the speaker muted Loop 1", seen(r"^looper: loop 1 muted", r"^looper: loop 1 unmuted")),
         ("Q-Link 1 set Loop 1's level", seen(r"^looper: loop 1 level")),
         ("Q-LINK reached the picked loop", seen(r"^q-link bank: the picked loop")),
         ("Q-Link 2 set a layer's level", seen(r"^looper: layer \d+ level")),
         ("the held ring cleared the loop", seen(r"^looper: Clear: the loop fades out"))],
        "Did each ring tap act at once and in time, Q-Link 1 change Loop 1's level, and the held ring clear the loop?",
        ["a_ring_acts_on_the_press_and_a_hold_clears_the_loop",
         "the_name_picks_the_speaker_mutes_and_the_level_bar_drags",
         "the_q_links_turn_the_loops_levels_then_the_picked_loop",
         "the_looper_tab_plays_loops_and_works_layers"],
        needs=['Q-LINK']),
    Check(
        "live-erase", "Live erase: ERASE held and a pad pressed while recording takes that pad's hits out",
        "S1: live erase, the MPC's ERASE + a pad while recording", "hands",
        ["1. On Play, tap All. Press REC, then PLAY, and once the chip is solid red play pads 1 and 3 for",
         "   about two bars. Press STOP.",
         "2. Press OVERDUB, then PLAY. Once the chip is solid red, hold ERASE and hold pad 1 for about two",
         "   bars: pad 1 makes no sound and its recorded hits go quiet as they come round, while pad 3 plays",
         "   on. Let go of both, then press STOP.",
         "3. Press PLAY: pad 3's hits play and pad 1's are gone. Press STOP, then UNDO twice (both takes go:",
         "   Track 1 is as it was before the check, for the checks after it)."],
        [("ERASE + pad 1 erased while recording", seen(r"^ERASE: pad [A-H]?\d+'s notes come out")),
         ("OVERDUB readied the take", seen(r"^rec: ready, PLAY adds to "))],
        "While you held ERASE and pad 1, did pad 1 stay silent and its recorded hits go, while pad 3 played on?",
        ["live_erase_writes_its_lines",
         "live_erase_takes_a_pads_notes_out_while_recording"],
        needs=['ERASE']),
    Check(
        "mute-queue", "Perform: TRACK MUTE's pads wait for the bar line, and TRACK MUTE held solos",
        "S4: sequencer decision 5, the mute queue and solo", "hands",
        ["1. On Play, tap Beats and light the kick on steps 1, 5, 9 and 13 if they are dark. Press PLAY.",
         "2. Press SHIFT + MAIN: the top bar's chip says PERFORM. Press MAIN: the Clips grid shows.",
         "3. Press TRACK MUTE, then hit pad 1: it blinks amber, Track 1's name is striped amber and the",
         "   transport bar says WAITING. The kick stops on the next bar line. Hit pad 1 again: it comes back.",
         "4. Hold TRACK MUTE, hit pad 2 and let go: pad 2 turns blue and on the next bar line the kick goes",
         "   quiet (Track 2 soloed). Hold TRACK MUTE and hit pad 2 again: the kick comes back on the line.",
         "5. Press TRACK MUTE, then SHIFT + MAIN (MAKE), then STOP."],
        [("Perform's tap waited for the line", seen(r"^track mute: .* muted on the next ")),
         ("the change landed on a bar line", seen(r"^time: track mute: at \d+\.1, .* muted")),
         ("TRACK MUTE held soloed a track", seen(r"^track mute: .* solo on the next ",
                                                r"^time: track mute: at \d+\.1, .* solo"))],
        "Did the kick stop and come back exactly on the bar lines, and go quiet while Track 2 was soloed, "
        "with no click?",
        ["mute_queue_writes_its_lines",
         "in_perform_a_track_mute_waits_for_the_bar_and_held_it_solos",
         "mutes_that_wait_land_together_on_the_launch_line",
         "track_mute_held_with_a_pad_solos_and_leaves_the_mode_as_it_was"],
        needs=['TRACK MUTE', 'SHIFT', 'MAIN']),
    Check(
        "reload-on-the-bar", "Reload clip brings a saved pattern back exactly on the bar line",
        "S4: pattern reload (Elektron's temp save), the clip card's Save state and the morph bar's Reload clip",
        "hands",
        ["1. On Play, tap Beats and light the kick on steps 1, 5, 9 and 13 if they are dark.",
         "2. Tap Arrange, then Clip on the Record panel's title line (right of Record), then Save state.",
         "3. Tap Play and light the kick on steps 3 and 7.",
         "4. Tap Arrange and press PLAY: the kick plays steps 3 and 7 too.",
         "5. Tap Reload clip on the morph bar under the grid: it turns amber, and on the next bar line steps",
         "   3 and 7 go quiet (the pattern is as saved). Press STOP."],
        [("Save state kept the clip", seen(r"^time: save state: .* kept")),
         ("the reload waited for its line", seen(r"^time: reload: .* reloads at ")),
         ("it landed on a bar line", seen(r"^time: (.*; )?reload: .* back as saved on bar \d+(;|$)"))],
        "Did steps 3 and 7 go quiet exactly on the bar line, with no flam, click or missed kick?",
        ["reload_on_the_bar_writes_its_lines",
         "reload_clip_brings_a_saved_pattern_back_on_the_bar",
         "reload_clip_lands_on_the_launch_line_and_undo_takes_it_back"]),
    Check(
        "live-transpose", "16 Levels' Transpose moves a playing synth line live, from its next note",
        "S4: live transpose (Jay's card, 17 Nov: 16 Levels gets a Transpose kind)", "hands",
        ["1. On Play, tap PADS PLAY (left of All | Loop) and pick Synth, the list's last row: Track 1 plays it.",
         "   Hit pad 1 and let go, press STEP SEQ, hit pads 1, 5, 9 and 13 and let go (four steps of C3),",
         "   press STEP SEQ again (the pads play), then press PLAY.",
         "2. Press 16 LEVEL: the 16 Levels panel shows Transpose, its heading names Track 1 and says as",
         "   written, and pad 9 is lit amber on the MPC.",
         "3. Hit pad 14: from the next step the line plays a fourth higher (F3), and pad 14 is lit.",
         "4. Hit pad 9: the line is back on C3. Press 16 LEVEL (the pads play again), then STOP."],
        [("pad 14 moved the track up a fourth", seen(r"^transpose: .* \+5 st$")),
         ("pad 9 put it back as written", seen(r"^transpose: .* as written$"))],
        "Did the line move a fourth up from the next step after pad 14, and back on pad 9, with no click?",
        ["live_transpose_writes_its_lines",
         "a_tracks_transpose_moves_its_notes_and_a_sounding_note_ends_where_it_went",
         "sixteen_level_on_the_synth_transposes_the_armed_track",
         "transpose_moves_the_track_and_the_tracks_that_follow_it"],
        needs=['16 LEVEL', 'STEP SEQ']),
    Check(
        "step-speed", "STEP SEQ: taps and turns go into the playing clip, and what they cost the MPC",
        "S1: step edits in place, no compile per tap or turn; Gate 0 for the time engine's own thread", "hands",
        ["1. On Play, hit pad 1 (the kick) and press STEP SEQ. Hit pads 1, 5, 9 and 13 if they are dark,",
         "   then press PLAY.",
         "2. For about 20 s, hit step pads on and off all over, as fast as you like; leave a few lit.",
         "3. Hold two lit pads, press Q-LINK twice (the cards say Sound) and turn Q-Link 1 back and forth",
         "   for about 10 s. Let go.",
         "4. Press STOP, then STEP SEQ."],
        [("step pads tapped", seen_times(r"^steps: step \d+ of pad \d+ (on|off)$", 10)),
         ("a turn locked held steps", seen(r"^steps: steps? .* of pad \d+: lock ")),
         ("the edits went in place, each quick", step_edits_in_place()),
         ("no slow period from the pads or the screen",
          not_seen(r"^control: slow period .* most in (screen actions|MIDI in and buttons)")),
         ("no underrun while it played", no_underruns_while())],
        "Did each step you lit play on the next pass, and the turn follow the knob, with no click?",
        ["step_edits_go_into_the_playing_view_in_place_as_a_compile_would",
         "step_edits_that_need_more_than_the_playing_view_compile",
         "a_step_edit_the_view_cannot_take_compiles_once_and_is_counted",
         "steps_tapped_on_beats_play_in_time"],
        needs=['STEP SEQ', 'Q-LINK'], wait_status=True),
    Check(
        "wheel", "The data wheel and its push on the real wheel",
        "#339 and #355: the outline, the push, and the Song view's picked block", "hands",
        ["1. On Play's All view, tap a lane's name: it mutes, and the outline goes on it. Push the wheel: it",
         "   unmutes. Turn the wheel: the outline moves on to the lane's steps.",
         "2. Tap a Q-Link card, push the wheel (the outline gets thicker, in the second colour) and turn: the",
         "   card changes. Push to let go.",
         "3. On Arrange, tap Song in the clips heading (Scenes -> song first if it is empty; with no clip yet,",
         "   light the kick on Play's Beats view first). Tap a block, push the wheel, turn it (a beat a step)",
         "   and push again: the block moves. UNDO takes it back."],
        [("the wheel turned", wheel_turned()),
         ("a block moved on the Song view", seen(r"^song: .* moved to "))],
        "Did the outline follow the wheel, and did each push do what the step says?",
        ["the_wheel_moves_an_outline_and_its_push_does_what_a_finger_does",
         "the_wheel_picks_rows_and_its_push_opens_one_on_every_page_with_rows",
         "the_wheels_push_opens_a_row_for_the_wheel_and_a_second_push_sets_it",
         "song_drags_copy_with_shift_and_the_wheel_moves_the_pick",
         "clip_and_block_moves_write_their_lines"],
        needs=['DATA DIAL PUSH']),
    Check(
        "touch", "Touch: Touch lab, Calibrate touch, two fingers and a real drag",
        "the real panel's jitter and report rate, which the smoke tests' exact taps do not have", "hands",
        ["1. SHIFT + MENU for Prefs, the last row Touch screen, +, then Touch lab, +. Follow its 9 steps",
         "   (about 3 minutes) and tap Exit.",
         "2. Calibrate touch, one row up: tap each corner's target. It only reports.",
         "3. Press MAIN for Play, press PLAY, and hold Delay throw and Reverb wash with two fingers. STOP.",
         "4. On Arrange (with no clip yet, light the kick on Play's Beats view first), drag the kick's clip to",
         "   an empty slot, then drag it again and hold SHIFT as you lift (a copy). UNDO twice."],
        [("Touch lab ran", seen(r"^touch-lab-open", r"^touch-lab-exit")),
         ("Calibrate touch gave a result", seen(r"^touch-calibrate: result=")),
         ("a clip moved and a copy dropped", seen(r"^clips: .* moved to ", r"^clips: a copy of "))],
        "Did the drags follow your finger without jumping, and did both throws light at once?",
        ["the_touch_lab_opens_from_prefs_and_exit_or_a_page_change_closes_it",
         "calibrate_touch_opens_from_prefs_and_takes_every_finger",
         "four_targets_on_the_built_in_mapping_match",
         "two_fingers_hold_two_throws_at_once",
         "a_dropped_clip_keeps_playing_where_it_lands_and_undo_takes_it_back",
         "clip_and_block_moves_write_their_lines"],
        needs=['MAIN']),
    Check(
        "note-repeat", "NOTE REPEAT, Latch and the arp",
        "the button and pads in real time, never reached on the device before", "hands",
        ["1. Hold NOTE REPEAT and a few pads: they repeat. While holding it, - / + switch Repeat, Arp and",
         "   Pattern, and the wheel changes the rate.",
         "2. SHIFT + NOTE REPEAT is Latch: the pads keep repeating. Press NOTE REPEAT to unlatch.",
         "3. NOTE REPEAT + MENU opens the Arp panel; the same again closes it. STOP."],
        [("NOTE REPEAT (11) reached the app", pressed(11)),
         ("the mode changed", seen(r"^arp: ")),
         ("Latch on and off", seen(r"^Latch on", r"^Latch off"))],
        "Did the repeats and the arp play in time?",
        ["pads_play_at_once_and_repeat_while_note_repeat_is_held",
         "latch_keeps_pads_repeating_until_it_goes_off_or_stop",
         "note_repeat_pressed_twice_latches_like_shift_and_note_repeat",
         "note_repeat_and_menu_open_the_arp_panel",
         "stop_silences_a_latched_arp_and_keeps_the_latch_until_stop_again",
         "note_repeat_latch_stop_and_the_looper_write_their_lines"]),
    Check(
        "looper", "REC + PLAY records until STOP, which keeps the take, with the pads' effects",
        "Jay's final-round looper notes (16 Nov): STOP threw the take away, the echo wasn't recorded", "hands",
        ["1. On the Play page's All view, turn Q-Link 4 (Delay send) up to about half. Play a pad: it echoes.",
         "2. SHIFT + SAMPLER opens the Looper tab. If Loop 1's ring (the first) has a wave round it, tap Loop 1's",
         "   name, then Clear (outlined in red, at the right of the loop's header under the rings): an earlier",
         "   check's loop goes. Press REC, then PLAY: the loop records from bar 1.",
         "3. Play a few pads, with gaps between them, for about a bar and a half. The wave draws as you play.",
         "4. Press STOP: everything stops at once.",
         "5. SHIFT + STOP (back to bar 1), then PLAY: your pads play back with their echoes, then",
         "   silence to the end of the bar, and round again. STOP."],
        [("the loop recorded", seen(r"^looper: Record: armed")),
         ("STOP kept the take", seen(r"^looper: loop of .* cut at STOP")),
         ("the pads were recorded with their effects", not_seen(r"^looper: records the pads dry"))],
        "Did the loop play back your pads with their echoes?",
        ["the_loop_strips_buttons_carry_their_symbols_and_play_and_stop_the_transport",
         "the_first_pass_shows_as_it_records",
         "presses_and_changes_log_and_the_wave_is_built",
         "the_looper_tab_plays_loops_and_works_layers",
         "rec_and_play_record_until_stop_which_keeps_the_take",
         "the_loop_records_the_pads_with_their_effects",
         "rec_closes_on_the_bar_line_and_stop_stops_the_loop_at_once",
         "capture_turns_what_you_played_into_a_loop",
         "when_i_play_starts_the_loop_and_the_song_on_the_first_pad",
         "tempo_from_the_first_loop_sets_the_tempo_and_starts_the_song",
         "note_repeat_latch_stop_and_the_looper_write_their_lines"],
        needs=['SAMPLER']),
    Check(
        "console", "The console chord, TAP TEMPO and STOP",
        "button chords on the real panel", "hands",
        ["1. Hold SHIFT, hold ERASE, press TAP TEMPO: the Console opens. The same again closes it.",
         "2. TAP TEMPO alone, four times in time: the tempo follows.",
         "3. SHIFT + NOTE REPEAT, hold pads 1 and 2, let go so they repeat, then press STOP twice."],
        [("the tempo was tapped", seen(r"^tempo [\d.]+ BPM \(tapped\)")),
         ("STOP stopped every sound", seen(r"^STOP: (all sounds stopped|arp silenced)"))],
        "Did the Console open and close, and did STOP silence everything?",
        ["shift_erase_tap_tempo_toggles_the_console_and_nothing_else_does",
         "tap_tempo_sets_the_tempo_and_blinks",
         "shift_stop_goes_back_to_bar_1_and_shift_rec_captures",
         "note_repeat_latch_stop_and_the_looper_write_their_lines"],
        needs=['ERASE']),
    Check(
        "midi-in", "A MIDI keyboard on MIDI IN (only with a keyboard and a 5-pin cable)",
        "the MPC's real MIDI ports", "hands",
        ["1. Plug the keyboard's MIDI OUT into the MPC's MIDI IN. On Play with kit 1, play keys from C2 up:",
         "   C#2 is pad 1, C2 is pad 2.",
         "2. Turn the wheel to \"Synth: Poly\" and play single keys and a chord, softly and loudly.",
         "3. Press TRACK MIX, tap I/O, then MIDI: the keyboard's port is listed. Change nothing. Back to kit 1."],
        [("notes came in", seen(NOTE_IN))],
        "Did the keys play in time, at the right loudness, with nothing stuck?",
        ["notes_on_any_channel_play_and_are_named",
         "keys_play_notes_and_share_holds_with_pads",
         "unplugging_lets_go_of_held_keys",
         "the_io_view_sees_ports_lights_and_clock"],
        needs=['TRACK MIX / PAD MIX']),
    Check(
        "sample-live-loop", "A loop's end moved while the pad is still sounding",
        "#455 (sampler R2): an edit on the Sample tab reaches a voice already playing; a jump past the playhead "
        "fades", "hands",
        ["1. On Play with kit 1, hit a pad that rings for a second or more (a cymbal or an open hi-hat), then",
         "   tap the Sample tab: its Wave page opens on that pad.",
         "2. Tap Mode's right arrow (the first row of the Region list) until it reads Loop: the loop shows shaded.",
         "3. Tap the wave: the pad keeps sounding, looping. Turn Q-Link 4 (Loop end) slowly back: the loop you",
         "   hear shortens as you turn, and when it passes the playhead the sound jumps into the loop.",
         "4. Tap the wave to stop it. Press UNDO until Mode reads as it did at the start and the shading is",
         "   gone, then press MAIN."],
        [("Mode went to Loop", seen(r"^sample: .*: mode .* -> Loop")),
         ("Loop end moved", seen(r"^sample: .*: Loop end [0-9.]+ s -> [0-9.]+ s")),
         ("UNDO (67) reached the app", pressed(UNDO)),
         ("the app kept running", stayed_up())],
        "Did the loop shorten while the pad kept sounding, and jump into the new loop without a click?",
        ["an_edit_reaches_the_sound_still_playing_and_another_kit_does_not"],
        needs=['UNDO', 'MAIN']),
    Check(
        "sample-r1-reach", "Normalize, New sound while pads play, a Chop kept, Export, and a layer",
        "#467 (sampler R1): Process runs on a worker at idle priority; Chop keeps its slices; Export and "
        "layers on the Play page", "hands",
        ["1. Hit the pad from the last check and tap the Sample tab. On the Wave page tap Normalize (the To 0 dB",
         "   row under Gain).",
         "2. On the Warp page set Warp to Stretch and Shift to +5 st, and tap New sound. While it runs (the button",
         "   shows Stop \u00b7 NN %), play other pads. When it ends, hit the pad: 5 semitones up. Press UNDO.",
         "3. On the Chop page set Method to Grid and Amount to 8 slices. Hit another pad, then this one again: the",
         "   8 slices still show. Set Put on to Files and tap Export 8 slices.",
         "4. On the Play page tap the Layers chip, then Add layer, then Remove layer. Press MAIN."],
        [("Normalize set the gain", seen(r"^sample: pad \S+ normalized: its peak")),
         ("New sound was made", seen(r"^sample: pad \S+ processed into a new sound")),
         ("UNDO (67) reached the app", pressed(UNDO)),
         ("the export was written", seen(r"^sample: pad \S+ exported as \d+ files")),
         ("a layer was added", seen(r"^sample: .*: layer 2 added, a copy of layer 1")),
         ("no underrun while New sound ran", no_underruns_while()),
         ("the app kept running", stayed_up())],
        "While New sound ran, did the pads keep playing without a click or a stall?",
        ["normalize_sets_the_gain_from_the_peak_in_one_undo_step",
         "process_makes_a_new_sound_on_a_worker_in_one_undo_step_and_can_be_stopped",
         "an_exported_pad_goes_to_samples_exports_and_loads_back_with_its_settings_and_slices",
         "layers_are_added_and_removed_on_the_play_tab_one_undo_step_each",
         "a_chop_is_kept_for_its_sound_and_shown_again_when_the_pad_comes_back"],
        needs=['UNDO', 'MAIN'], wait_status=True),
    Check(
        "sample-names", "A pad and its sound named on the text keyboard, taken back by UNDO",
        "#481 (sampler R1, names): a tap on the pad's name opens a text keyboard for the pad and its sound; "
        "each name is one UNDO step", "hands",
        ["1. Hit the pad from the last check and tap the Sample tab. On the Wave page tap the pad's name at the",
         "   top left (a dotted line under it): the text keyboard opens. Tap Delete twice, type x, tap Enter.",
         "2. Open the keyboard again, tap Sound at the top right, tap Clear, type Loop, and tap Enter: the",
         "   subtitle says Source Loop. Press UNDO: the sound has its old name. UNDO again: so does the pad.",
         "3. Open the keyboard again and turn the wheel: the cursor moves. Push the wheel: it closes, nothing",
         "   changed. Press MAIN."],
        [("the pad was named", seen(r"^sample: pad [A-H]\d+ named ")),
         ("its sound was named Loop", seen(r"^sample: pad [A-H]\d+'s sound named Loop$")),
         ("UNDO (67) reached the app", pressed(UNDO)),
         ("the app kept running", stayed_up())],
        "Did each new name show at once, did UNDO bring both old names back, and did the wheel move the cursor?",
        ["a_tap_on_the_pad_bars_title_names_the_pad_and_its_sound",
         "every_control_on_every_screen_does_something"],
        needs=['UNDO', 'MAIN']),
    Check(
        "sample-r2-16bit-cache", "A project saved and opened again: its sounds' analysis read back, no flat line",
        "#500 (sampler R2): 16-bit sounds held at half the memory, and a saved project keeps each sound's "
        "analysis so it opens without analysing; STATE-CHANGING: saves a new project on the stick's data "
        "partition (undo: Delete it in Projects)", "hands",
        ["1. Hit the pad from the last check and tap the Sample tab. Open Chop and wait until its hits show.",
         "2. Press BROWSE and tap Projects. Tap Save as: the keyboard shows the project's name. Type space,",
         "   r and 2 at its end and tap Done: it saves as a new project, so none of yours is written over.",
         "3. Tap that project (its name ends in r2) twice to open it again.",
         "4. On the Sample tab hit the same pad: the wave and its hits are there at once, with no flat line",
         "   first. Play the pad and its Chop slices: they sound as before. Press MAIN."],
        [("the project was saved", seen(r"^project: saved .* r2$")),
         ("it opened again", seen(r"^project: opening .* r2$")),
         ("the analysis was read, not made again", seen(r"^sample: the analysis of \d+ sounds? read from the project$")),
         ("the app kept running", stayed_up())],
        "When the project opened again, were the pad's wave and its hits there at once, with no flat line?",
        ["a_project_keeps_its_sounds_analysis_and_opens_without_analysing_them"],
        needs=['BROWSE', 'MAIN']),
    Check(
        "midi-monitor", "The MIDI monitor: what each message from a keyboard on MIDI IN did",
        "the app feeds the I/O view's Monitor now, and each line says where its message went", "hands",
        ["1. With a keyboard's MIDI OUT in the MPC's MIDI IN and kit 1 on Play, press TRACK MIX, tap I/O,",
         "   then Monitor. Play C#2, C2 and E3: lines appear, newest at the top.",
         "2. Turn a knob or the mod wheel on the keyboard.",
         "3. SHIFT + MENU for Prefs, turn Show developer tools on, open the Dev tab's MIDI page, hit pad 1,",
         "   press PLAY and STOP, and turn Q-Link 1. Turn Show developer tools off again. Change nothing else."],
        [("notes came in", seen(NOTE_IN))],
        "Did Went to name each key's pad (no pad for E3) and the knob, and Dev list your presses?",
        ["the_monitor_says_where_each_message_went",
         "the_monitor_gets_what_went_out_and_who_sent_it",
         "the_monitor_gets_each_value_sent"],
        needs=['TRACK MIX / PAD MIX']),
    Check(
        "midi-now", "The MIDI monitor's Now view and Save log",
        "new: Now says what a keyboard holds this moment, and Save log keeps the list on the stick", "hands",
        ["1. With a keyboard's MIDI OUT in the MPC's MIDI IN, press TRACK MIX, tap I/O, then Monitor, then Now.",
         "2. Hold a chord on the keyboard, move its mod wheel and its pitch bend, then let go.",
         "3. Tap List, then Save log: the line under the list says which file it wrote. Change nothing else."],
        [("Save log kept a file on the stick", seen(r"^midi log: Saved \d+ lines to files/midi-logs/midi-log-"))],
        "Did Now show the keys you held, the mod wheel and the bend while you played them?",
        ["a_saved_log_is_a_new_file_in_the_stores_folder",
         "without_a_data_folder_nothing_is_saved_and_the_monitor_hears_why",
         "the_monitors_note_offs_are_taken_once"],
        needs=['TRACK MIX / PAD MIX']),
    Check(
        "midi-port", "A port's page, and each MIDI port of a USB device",
        "new: a port's row opens its page (Open >), and a USB device with several MIDI ports lists each", "hands",
        ["1. With a keyboard's MIDI OUT in the MPC's MIDI IN, press TRACK MIX, tap I/O, then MIDI, then Open >",
         "   on the MIDI row. Play a key, read What it does, then tap < MIDI. Change nothing.",
         "2. Only with a USB MIDI interface that has two or more MIDI ports: plug it into the USB socket,",
         "   wait 5 s, look at the MIDI section's ports, and unplug it."],
        [("notes came in", seen(NOTE_IN))],
        "Did the page's Now card show the key you played as you played it?",
        ["each_port_of_a_usb_device_is_a_port_of_its_own",
         "a_port_that_ignores_clock_and_programs_follows_none",
         "every_sample_screen_draws"],
        needs=['TRACK MIX / PAD MIX']),
    Check(
        "midi-routing", "Routing: a keyboard's channel playing a track that isn't armed",
        "new (#449): a route sends one channel of a port to a track, whichever track is armed", "hands",
        ["1. With a keyboard's MIDI OUT in the MPC's MIDI IN, press TRACK MIX, tap I/O, then Routing.",
         "2. Tap the square in the MIDI in row under Tracks (by channel): the card shows the new route, ch 1 to",
         "   Track 1. Turn the Q-Link labelled Track one step right, to Track 2. If the keyboard sends on",
         "   another channel, turn the Q-Link labelled Channel to it.",
         "3. Tap Monitor and play C#2 and C2.",
         "4. Tap Routing, tap the same square, then Remove route. Change nothing else, and don't save."],
        [("notes came in", seen(NOTE_IN))],
        "Did Went to say Track 2, by its route, for the keys you played?",
        ["a_route_plays_its_track_whatever_the_ports_input_and_the_rest_play_the_armed_one",
         "a_routed_keys_note_off_follows_its_note_on_when_the_armed_track_changes",
         "routed_keys_play_their_track_where_its_clips_play"],
        needs=['TRACK MIX / PAD MIX']),
    Check(
        "midi-learn", "MIDI Learn: SHIFT + a touch, the Learn section, and PLAY and STOP from a key",
        "new (MIDI powerhouse PR 4): SHIFT + a touch learns the next control moved; a key can play and stop",
        "hands",
        ["1. With a keyboard or controller that has a knob in the MPC's MIDI IN, press TRACK MIX. On the Faders",
         "   look, hold SHIFT, touch the first strip's fader and let go of SHIFT: the hint line says LEARNING.",
         "   Turn the knob: the fader follows it.",
         "2. Tap I/O, then Learn: the knob's row says what it moves. Tap Transport and tempo, then PLAY and",
         "   STOP, and press a key on the keyboard. Press the key again: MPFree plays. Once more: it stops.",
         "3. Pick each row in the list and tap Remove, so the key plays notes again. Change nothing else."],
        [("Learn took the knob", seen(LEARNED + r"CC \d+ .* takes (?!transport\.)")),
         ("Learn took the key for PLAY and STOP", seen(LEARNED + r".+ takes transport\.\d+")),
         ("the key played and stopped MPFree", seen(r"^time: transport: Play\b", r"^time: transport: Stop\b"))],
        "Did the fader follow the knob, and did the key play and stop MPFree?",
        ["shift_and_a_touch_on_a_value_arms_midi_learn",
         "a_touch_arms_learn_until_a_control_takes_it",
         "a_key_on_play_and_stop_or_a_mute_flips_it_and_the_transport_is_the_rigs",
         "a_learned_control_presses_the_mpcs_transport_buttons",
         "shift_and_a_touch_on_the_tempo_or_a_mute_arms_midi_learn",
         "learn_again_replaces_a_control_and_the_rigs_work_in_every_song"],
        needs=['TRACK MIX / PAD MIX', 'SHIFT']),
    Check(
        "clip-learn", "A key on a keyboard launching and stopping a clip, learned by SHIFT + a tap on the Clips grid",
        "new: SHIFT + a tap on the Clips grid learns a clip, a scene, Stop all or a track's arm (sequencer S5)",
        "hands",
        ["Only with a keyboard or pad controller in the MPC's MIDI IN.",
         "1. On Play, tap Beats and light Track 1's first row on steps 1, 5, 9 and 13, then open Arrange.",
         "2. Hold SHIFT, tap Track 1's clip in row 1 and let go of SHIFT: the line under the screen says",
         "   LEARNING. Press a key on the keyboard.",
         "3. Press PLAY. Press the key: the clip stops on the next bar. Press it again: it starts on the next.",
         "4. STOP. TRACK MIX, I/O, Learn: pick the Launch row, tap Remove. Don't save."],
        [("Learn took the key for the clip", seen(LEARNED + r".+ takes clips\.\d+")),
         ("the key stopped the clip and launched it again",
          seen(r"^time: clips: StopClip\(0\)", r"^time: clips: Launch \{ track: 0, slot: 0 \}"))],
        "Did the key stop the clip on a bar line, then start it again on the next one?",
        ["shift_and_a_tap_on_the_grid_arms_midi_learn",
         "the_clips_grid_is_named_and_read_for_learn",
         "mutes_that_wait_land_together_on_the_launch_line"],
        needs=['SHIFT', 'TRACK MIX / PAD MIX']),
    Check(
        "midi-thru", "Thru: a keyboard on MIDI IN playing a synth on MIDI OUT (only with both)",
        "new: thru sends a port's channels straight out of another, set on Routing; STATE-CHANGING: notes go to the "
        "synth while Send MIDI and CV out is on", "hands",
        ["Only with a keyboard's MIDI OUT in the MPC's MIDI IN and a synth's MIDI IN on the MPC's MIDI OUT.",
         "1. SHIFT + MENU for Prefs, turn Send MIDI and CV out on.",
         "2. Press TRACK MIX, tap I/O, then Routing. Tap the square in the MIDI in row under MIDI out: the card",
         "   shows thru, every channel. Play a few keys and a chord, and let go: the synth plays them.",
         "3. Tap Remove route. SHIFT + MENU for Prefs, turn Send MIDI and CV out off again. Don't save."],
        [("sending was turned on", seen(r"^MIDI and CV out: on")),
         ("notes came in", seen(NOTE_IN)),
         ("thru sent them out of MIDI OUT", seen(r"^\+\d+\.\d+ s MIDI Out: [89][0-9A-F]( [0-9A-F]{2}){2} = note on")),
         ("sending was turned off again", seen(r"^MIDI and CV out: off"))],
        "Did the synth play each key as you played it, and stop when you let go?",
        ["thru_sends_a_channel_on_as_it_came_and_learn_keeps_what_it_learned",
         "thru_passes_a_keyboards_channel_out_as_it_came"],
        needs=['TRACK MIX / PAD MIX', 'SHIFT', 'MENU']),
    Check(
        "sample-gear", "Auto-sample of a synth on MIDI OUT through the line input, and Cancel (only with a synth)",
        "#481 (sampler R1, gear): auto-sample plays outside gear on MIDI OUT and records it from the line input; "
        "STATE-CHANGING: notes go to the synth while Send MIDI and CV out is on, a setting kept across "
        "restarts (the last step turns it off)", "hands",
        ["Only with a synth: its MIDI in on the MPC's MIDI OUT (on channel 1), its audio out in the line input.",
         "1. SHIFT + MENU for Prefs: turn Send MIDI and CV out on, and Line input on.",
         "2. Press MAIN, tap the Sample tab, open Rec, then Auto-sample. Step the Source row to Gear \u00b7 MIDI ch 1.",
         "   Set Lowest note C3, Highest note C4, Every 12, Layers 1. Tap Start: the synth plays the middle note",
         "   twice, then C3 and C4, and two pads get the notes.",
         "3. Tap Start again, and tap Cancel after the first note: the synth's note stops.",
         "4. SHIFT + MENU for Prefs: turn Send MIDI and CV out off, and Line input off. Press MAIN. Don't save."],
        [("sending was turned on", seen(r"^MIDI and CV out: on")),
         ("auto-sample found the gear's MIDI out", seen(r"^sample: auto-sample can play outside gear on ")),
         ("it started on channel 1, C3 to C4", seen(r"^sample: auto-sample Gear \(MIDI channel 1 on .+\) started: "
                                                   r"2 notes \(C3 to C4\)")),
         ("two pads got the notes", seen(r"^sample: auto-sampled Gear onto kit .+: 2 notes, ")),
         ("Cancel kept the notes done", seen(r"^sample: auto-sample cancelled, keeping the notes done$")),
         ("sending was turned off again", seen(r"^MIDI and CV out: off"))],
        "When you tapped Cancel, did the synth's note stop, with no note left hanging?",
        ["auto_sample_plays_external_gear_over_midi_and_records_it_from_the_line_input"],
        needs=['SHIFT', 'MENU', 'MAIN']),
    Check(
        "clip-program", "A clip's program change switching a synth on MIDI OUT as the clip starts (only with one)",
        "new: a clip's Program and Bank on the clip card (sequencer S5); STATE-CHANGING: a program change goes to the "
        "synth while Send MIDI and CV out is on (it picks the synth's sound, it writes nothing there)", "hands",
        ["Only with a synth on MIDI OUT, channel 1, whose program 5 you can tell apart. Note its program now.",
         "1. SHIFT + MENU for Prefs, turn Send MIDI and CV out on. TRACK MIX, I/O, Routing: tap Track 1's",
         "   square under MIDI out and pick channel 1 on the card.",
         "2. On Play, tap Beats and light Track 1's first row on steps 1, 5, 9 and 13. Open Arrange, tap Clip",
         "   on the Record panel, turn the wheel to Program and press + five times: it says 5.",
         "3. Press PLAY: the synth switches to its program 5 as the clip starts. Let it play two bars, STOP.",
         "4. Program back to off (- five times), Track 1's square under MIDI out off, Prefs' switch off.",
         "   Don't save. Set the synth back to its own program."],
        [("sending was turned on", seen(r"^MIDI and CV out: on")),
         ("the clip's Program was set to 5", seen(r"^time: clip: .+ \u00b7 Program 5 \(UNDO takes it back\)")),
         ("the program change went out of MIDI OUT", seen(r"^\+\d+\.\d+ s MIDI Out: C0 04 = program 5, ch 1")),
         ("sending was turned off again", seen(r"^MIDI and CV out: off"))],
        "Did the synth change to its program 5 as the clip started, and keep it while the clip looped?",
        ["the_clip_cards_program_goes_out_as_the_clip_starts",
         "a_clips_program_goes_after_the_tracks_own"],
        needs=['TRACK MIX / PAD MIX', 'SHIFT', 'MENU']),
    Check(
        "browser-pad-map", "BROWSE's pad map: a kit pad held plays, a tap picks the pad Load goes on",
        "The Play, Pads and Sequencer redesign's step 1 (board 8): under a sample BROWSE shows the kit's 16 pads; "
        "a held cell plays that pad and picks it (state-changing only in the app: the pad picked)", "hands",
        ["1. Press BROWSE and tap Samples. Tap a sample in the list: under its details the kit's pads show,",
         "   titled with the kit's name, bank A and \"tap a pad\".",
         "2. Hold the A1 cell: pad 1 plays while you hold it, and A1 lights. Let go.",
         "3. Tap \u203a beside the title: bank B shows. Tap \u2039 to come back to bank A.",
         "4. Tap the A16 cell: Load now reads Load on A16. Press BROWSE to close the browser without loading:",
         "   the Sample tab has pad A16 picked."],
        [("the A1 cell picked pad A1", seen(r"^browser: pad A1 picked \(")),
         ("the A16 cell picked pad A16", seen(r"^browser: pad A16 picked \(")),
         ("the touchscreen was used", touch_rose()),
         ("the app kept running", stayed_up())],
        "Did pad 1 sound for as long as you held its cell, and stop when you let go?",
        ["the_browsers_pad_map_picks_a_pad_and_plays_it_while_held"],
        needs=['BROWSE']),
    Check(
        "midi-file", "A MIDI file loaded from BROWSE as clips, on the pads that played it",
        "new: BROWSE loads a .mid as clips (MIDI powerhouse PR 6); an export's own MIDI file comes back", "hands",
        ["1. If Arrange has no clip yet: on Play, tap Beats and light the kick on steps 1, 5, 9 and 13.",
         "   Press PLAY, let it play a bar, then STOP.",
         "2. Press BROWSE, pick Projects, press Export. Untick Mixdown and Stems (MIDI file stays ticked),",
         "   press Start, and when it is done tap Show in BROWSE.",
         "3. Tap the .mid file, then tap it again: the line under the screen says how many clips came in",
         "   and where.",
         "4. Press BROWSE to close it and open Arrange: the kick's track has a new clip in its next slot.",
         "   Tap it to hear it, press STOP, then UNDO: the new clip goes."],
        [("the export wrote its files", seen(r"^export: Exported ")),
         ("the MIDI file went in as clips", seen(r"^MIDI file .+\.mid: \d+ clips? with \d+ notes, from slot \d+ of ")),
         ("UNDO took the file's clips away", seen(r"^Undid capture \d+ ")),
         ("it was never refused", not_seen(r"^MIDI file .+: (not loaded|nothing went in)"))],
        "Did the new clip play the kick on the same steps as the pattern?",
        ["an_exported_midi_file_loads_back_onto_the_pads_it_played",
         "a_midi_file_from_browse_lands_on_its_pads_and_undo_takes_it",
         "notes_go_on_the_pads_that_play_them_and_the_rest_are_counted"],
        needs=['BROWSE', 'UNDO']),
    Check(
        "sample-r3-kit-per-track", "Two tracks on two kits in a new project, each kept through a save and an open",
        "#509 (sampler R3 parts 1 and 2a): every track plays its own kit; STATE-CHANGING: saves a new project "
        "on the stick's data partition (undo: Delete it in Projects)", "hands",
        ["1. Press BROWSE, tap Projects, then New project, and Yes (the round's changes needn't be kept). Press",
         "   BROWSE. On Arrange the sound column says Kit for every track.",
         "2. Tap track 2's name to arm it, tap the kit chip at the top right and pick Kit 2 (second in its list):",
         "   the pads play that kit. Press REC and PLAY, play a bar of hits, and press STOP.",
         "3. Tap track 1's name: the pads play Kit 1 again. Press REC and PLAY, play a bar of other hits, STOP.",
         "4. Press PLAY: both clips play, each on its own kit. Press TRACK MIX, tap Faders, pull track 2's fader",
         "   down (only Kit 2's hits go quiet), then tap the number under it (0 dB again). Press STOP.",
         "5. Press SHIFT + BROWSE (it saves a new project), open it again from Projects: track 2 plays Kit 2."],
        [("a new project started", seen(r"^project: new project$")),
         ("REC readied a take", seen(r"^rec: ready, PLAY records ")),
         ("no underrun while both kits played", no_underruns_while()),
         ("it saved as a new project", seen(r"^project: saved Project \d{4}-\d\d-\d\d \d\d\.\d\d")),
         ("it opened again", seen(r"^project: opening Project \d{4}-\d\d-\d\d \d\d\.\d\d")),
         ("the app kept running", stayed_up())],
        "In step 4, did both tracks play together, each on its own kit, with nothing cut off or late?",
        ["each_track_keeps_the_kit_picked_for_it",
         "the_synth_plays_on_a_track_once_picked_until_a_kit_is"],
        needs=['BROWSE', 'SHIFT', 'TRACK MIX / PAD MIX'], wait_status=True),
    Check(
        "map-controls", "Map controls: find buttons the app does not know yet",
        "only when a round needs a button that has no number yet", "hands",
        ["1. SHIFT + MENU for Prefs, the wheel to Map controls, +. Tap a control still to find (or pick it",
         "   with the wheel and press +), then press that button on the MPC: its number appears.",
         "2. - skips one that is not on your MPC. STOP stops listening. MENU leaves."],
        [("buttons were found", seen(r"^map: .* = ")),
         ("Map controls was left (its summary)", seen(r"^map summary: "))],
        None,
        ["names_are_unique_and_measured_buttons_are_listed",
         "listening_takes_a_new_number_and_moves_to_the_next_control_left",
         "known_numbers_are_named_not_taken_and_stop_stops_listening",
         "skip_clear_and_measured_buttons"]),
    Check(
        "sample-warp", "The Warp crash retest on the shortest sound",
        "a crash on the device in live test 2", "hands",
        ["1. Hit the closed hi-hat, tap the Sample tab, then Warp. Turn the wheel through every row, push it on",
         "   one and turn, push again, and zoom in and out with SHIFT + - / +. UNDO."],
        [("the app kept running", stayed_up())],
        None,
        ["the_warp_tab_pins_the_hits_and_the_copy_goes_through_them",
         "a_warped_pad_follows_the_tempo_and_plays_its_stretched_copy_when_ready"]),
    Check(
        "fx-add", "Add an effect on the Mix tab and hear it",
        "live test 3's + Add effect did nothing (#275, #287, #353); never run on the MPC since", "hands",
        ["1. On Play with kit 1, hit a few pads. Press TRACK MIX, tap Effects, then the Drums row's + FX:",
         "   the effect picker opens.",
         "2. Tap Delay and reverb, then Reverb. \"Reverb \u00b7 Room\" goes in after \"Filter \u00b7 DJ\". Hit pads: they ring.",
         "3. Tap Reverb \u00b7 Room's ON pill: the pads go dry. Tap it again: the reverb comes back.",
         "4. Tap Reverb \u00b7 Room, then Remove (UNDO doesn't take back chain edits). Press MAIN."],
        [("the touchscreen was used", touch_rose()),
         ("no refusal", not_seen(CHAIN_REFUSED)),
         ("the app kept running", stayed_up())],
        "Did the pads ring once the reverb was in, go dry with its ON pill off, and ring again with it on?",
        ["the_chain_editors_taps_reach_the_app",
         "the_effects_look_adds_picks_turns_and_moves_an_effect",
         "an_insert_added_on_the_mixer_is_turned_bypassed_moved_and_removed",
         "the_effect_pickers_cancel_adds_nothing",
         "the_mixer_opens_the_editor_on_a_strip_and_adds_an_effect_from_the_picker",
         "room_to_plate_keeps_the_shared_knobs_and_learned_q_links_follow"],
        needs=['TRACK MIX / PAD MIX', 'MAIN']),
    Check(
        "eq", "The Mix tab's EQ: a dot follows a finger, UNDO and Solo band",
        "#388: the parametric EQ (the EQ look on the Mix tab, the EQ page in the chain editor)", "hands",
        ["1. Press TRACK MIX, tap EQ at the top right of the Mix tab, and tap \"+ EQ\" on the Drums row.",
         "   Play pads, drag the second dot of the new curve up, down and sideways, and let go.",
         "2. Press UNDO: the curve goes back and the hint line says \"Undid Drums EQ Band 2\".",
         "3. Tap the Drums curve twice to open the EQ page, tap Solo band, and play pads (you hear only",
         "   band 2's part of the sound). Press MAIN to go back to Play."],
        [("UNDO (67) reached the app", pressed(UNDO)),
         ("UNDO took back the EQ drag", seen(r"^Undid .+ EQ Band \d$")),
         ("no underrun while it played", no_underruns_while())],
        "Did the sound follow the dot while you dragged it, and did Solo band play only that band's part?",
        ["the_eq_look_adds_picks_and_shapes_an_eq", "a_drag_on_the_eq_look_is_what_you_hear",
         "a_drag_on_the_eq_page_is_what_you_hear"],
        needs=['TRACK MIX / PAD MIX', 'MAIN'], wait_status=True),
    Check(
        "eq-quality", "The EQ's Quality chip swapped while the sound plays",
        "#446 (FX quality, PR D): matched bells and shelves, and a Quality chip (Normal or HQ 2x) on the EQ page",
        "hands",
        ["1. On Play, press SHIFT + NOTE REPEAT (Latch), hold pad 2, let go: it repeats. Press TRACK MIX, tap EQ",
         "   at the top right of the Mix tab (tap \"+ EQ\" on the Drums row if it has none), then tap the Drums",
         "   curve twice: the EQ page opens.",
         "2. Tap the 4 chip (Band 4, a high shelf at 8 kHz) and turn Q-Link 2 (Gain) up to about +12 dB.",
         "3. Tap the chip under QUALITY at the right end of the type row: Normal becomes HQ 2x. Tap it four",
         "   more times, about a second apart, and leave it on HQ 2x for about 40 s.",
         "4. Tap it back to Normal, turn Q-Link 2 back to 0 dB, and press MAIN.",
         "   Press SHIFT + NOTE REPEAT (Latch goes off), then STOP twice: the pad stops."],
        [("Latch kept the pad repeating", seen(r"^Latch on")),
         ("the touchscreen was used", touch_rose()),
         ("no refusal", not_seen(CHAIN_REFUSED)),
         ("no underrun while it played", no_underruns_while()),
         ("Latch went off", seen(r"^Latch off")),
         ("the app kept running", stayed_up())],
        "Did every Quality tap swap without a click or a gap, with the pads otherwise sounding the same?",
        ["the_eq_look_adds_picks_and_shapes_an_eq", "a_drag_on_the_eq_page_is_what_you_hear"],
        needs=['SHIFT', 'NOTE REPEAT', 'TRACK MIX / PAD MIX', 'MAIN', 'STOP'], wait_status=True),
    Check(
        "return-wet", "A reverb added to a send return starts fully wet",
        "#422: a delay or reverb added to a send return (A to D) starts at Mix 100 %", "hands",
        ["1. Press TRACK MIX, tap Effects at the top right, then Submixes. Press > until Return B's row shows.",
         "   Tap its + FX, then Delay and reverb, then Reverb: Reverb \u00b7 Room opens with its Mix at 100 %.",
         "2. Tap \u2039 at the top left, then Tracks and Faders. Drag the Drums strip's B send half way up and",
         "   hit pads: the reverb rings alongside them.",
         "3. Drag the B send back down to the bottom and press MAIN."],
        [("the touchscreen was used", touch_rose()),
         ("no refusal", not_seen(CHAIN_REFUSED)),
         ("the app kept running", stayed_up())],
        "Did the reverb open at Mix 100 %, and did the pads ring through it with the B send up?",
        ["a_delay_or_reverb_on_a_return_starts_wet",
         "the_mixer_opens_the_editor_on_a_strip_and_adds_an_effect_from_the_picker"],
        needs=['TRACK MIX / PAD MIX', 'MAIN']),
    Check(
        "mix-curve", "A delay's or reverb's Mix curve: Equal holds the level, Linear dips",
        "#423 (FX quality, PR B): equal-power Mix and its Mix curve choice; the curves themselves are "
        "measured by mpfree-fx's mix_curve tests", "hands",
        ["1. On Play with kit 1, press SHIFT + NOTE REPEAT (Latch), hold pad 2 (the snare), let go: it repeats.",
         "2. Press TRACK MIX, tap Effects, then Tracks. On the Drums row tap + FX, then Delay and reverb, then",
         "   Reverb. In the chain editor, tap Hall in the mode chooser beside the effect's name.",
         "3. In the Output section the Mix curve reads Equal. Drag Mix down to 0, then slowly up to 100 %.",
         "4. Step the Mix curve's arrows to Linear, and drag Mix from 0 slowly up to 100 % again."],
        [("Latch kept the snare repeating", seen(r"^Latch on")),
         ("no refusal", not_seen(CHAIN_REFUSED)),
         ("no underrun while the Hall played", no_underruns_while()),
         ("the app kept running", stayed_up())],
        "As Mix went up, did Equal keep the level steady where Linear dipped half way?",
        ["a_mode_across_engines_swaps_in_place_and_switching_back_restores",
         "the_mixer_opens_the_editor_on_a_strip_and_adds_an_effect_from_the_picker"],
        needs=['TRACK MIX / PAD MIX', 'SHIFT', 'NOTE REPEAT'], wait_status=True),
    Check(
        "mix-curve-click", "Changing the Mix curve while the sound plays doesn't click",
        "#423 (FX quality, PR B): a change of Mix curve glides from one blend to the other", "hands",
        ["1. With the snare still repeating through the Hall, drag Mix to about 50 %.",
         "2. Step the Mix curve's arrows through Equal, Dry first and Linear, and back, a few times."],
        [("no refusal", not_seen(CHAIN_REFUSED)),
         ("the app kept running", stayed_up())],
        "Did each change of Mix curve glide, with no click?",
        ["fx_walks_keep_the_chain_editor_and_the_mixer_together"]),
    Check(
        "room-level", "The Room reverb is about as loud as the Plate",
        "#423 (FX quality, PR B): the Room was about 4 dB quieter than the other reverbs", "hands",
        ["1. Step the Mix curve back to Equal and leave Mix at about 50 %. Tap Room in the mode chooser,",
         "   listen, then Plate, and switch between them a few times.",
         "2. Tap Remove to take the reverb out, and press MAIN.",
         "   Press SHIFT + NOTE REPEAT (Latch goes off), then STOP twice: the snare stops."],
        [("no refusal", not_seen(CHAIN_REFUSED)),
         ("Latch went off", seen(r"^Latch off")),
         ("the app kept running", stayed_up())],
        "Were Room and Plate about as loud as each other?",
        ["room_to_plate_keeps_the_shared_knobs_and_learned_q_links_follow",
         "a_mode_across_engines_swaps_in_place_and_switching_back_restores"],
        needs=['MAIN', 'SHIFT', 'NOTE REPEAT']),
    Check(
        "room-rebuilt", "The rebuilt Room: each hit blooms at once, and Size and Pre-delay glide",
        "#462 (FX quality, PR E1): diffused input, early reflections that keep a source on its side, a 16-line "
        "network, and Size and Pre-delay that glide or crossfade", "hands",
        ["1. On Play with kit 1, press SHIFT + NOTE REPEAT (Latch), hold pad 2 (the snare), let go: it repeats.",
         "2. Press TRACK MIX, tap Effects, then Tracks. On the Drums row tap + FX, then Delay and reverb, then",
         "   Reverb: Reverb \u00b7 Room opens. Drag Mix to about 50 %. Listen to a few snares: each one should",
         "   bloom into a smooth tail at once, with no slap or flutter at its start.",
         "3. Drag Size slowly from one end to the other and back, then Pre-delay the same way: the room grows",
         "   and shrinks, and the tail moves later and earlier. Leave it playing for 40 s. The Plate is next."],
        [("Latch kept the snare repeating", seen(r"^Latch on")),
         ("the touchscreen was used", touch_rose()),
         ("no refusal", not_seen(CHAIN_REFUSED)),
         ("no underrun while the Room played", no_underruns_while()),
         ("the app kept running", stayed_up())],
        "Did each snare bloom smoothly with no slap at the start, and did Size and Pre-delay move with no zipper or click?",
        ["the_mixer_opens_the_editor_on_a_strip_and_adds_an_effect_from_the_picker",
         "room_to_plate_keeps_the_shared_knobs_and_learned_q_links_follow"],
        needs=['SHIFT', 'NOTE REPEAT', 'TRACK MIX / PAD MIX'], wait_status=True),
    Check(
        "plate-rebuilt", "The rebuilt Plate: dense from the first moment, with no ringing tone",
        "#466 (FX quality, PR E2): the Plate on the Room's network; before it a hit turned dense only after "
        "about 170 ms, and the tail rang at 3.7 kHz", "hands",
        ["1. With the snare still repeating through the Room, tap Plate in the mode chooser beside the effect's",
         "   name. Listen to a few snares: each one should be dense from its first moment, with no grainy",
         "   flutter, and the tail should not ring on one pitch.",
         "2. Drag Size slowly from one end to the other and back, then Pre-delay the same way.",
         "3. Tap the effect's name at the top of its page for its presets, and tap Endless plate: the tail runs",
         "   on for many seconds, about as loud as before (the preset keeps your Mix). Leave it for 40 s.",
         "   The Spring is next."],
        [("no refusal", not_seen(CHAIN_REFUSED)),
         ("no underrun while the Plate played", no_underruns_while()),
         ("the app kept running", stayed_up())],
        "Was the Plate dense from its first moment with no ringing tone, even on Endless plate, and no zipper or click?",
        ["a_mode_across_engines_swaps_in_place_and_switching_back_restores",
         "a_preset_from_the_header_loads_and_keeps_the_mix",
         "room_to_plate_keeps_the_shared_knobs_and_learned_q_links_follow"],
        wait_status=True),
    Check(
        "spring-rebuilt", "The rebuilt Spring: its boing, Drip that splashes without ringing longer",
        "#471 (FX quality, PR E3): the Spring rebuilt around its chirp; a source keeps its side, Decay rings "
        "longer not louder, and Dub splash no louder than the dry", "hands",
        ["1. With the snare still repeating, tap Spring in the mode chooser beside the effect's name. Each snare",
         "   should have the spring's boing (a quick falling chirp) and a tail that rings on, with no harsh whistle.",
         "2. Drag Drip slowly from one end to the other and back: the splash grows and shrinks, but the tail lasts",
         "   about as long. Then drag Decay end to end and back: the tail gets longer and shorter, not louder.",
         "3. Tap the effect's name at the top of its page for its presets, and tap Dub splash: about as loud as",
         "   a moment ago. Drag Drip and Tension a little. Leave it playing for 40 s. The Hall is next."],
        [("no refusal", not_seen(CHAIN_REFUSED)),
         ("no underrun while the Spring played", no_underruns_while()),
         ("the app kept running", stayed_up())],
        "Did the Spring keep its boing, did Drip leave the tail's length alone, and was Dub splash as loud, with no zipper?",
        ["a_mode_across_engines_swaps_in_place_and_switching_back_restores",
         "a_preset_from_the_header_loads_and_keeps_the_mix"],
        wait_status=True),
    Check(
        "cloud-rebuilt", "The rebuilt Hall, Cloud and Echoes: longer not louder, no pitch bend, no zipper",
        "#484 (FX quality, PR E4): before it a long Decay played 3.5 to 5 LU louder than a short one, Size and "
        "Pre-delay bent the tail's pitch, Echoes zipped as its knobs moved, and the Cloud's Decay ran long", "hands",
        ["1. With the snare still repeating through the Spring, tap Hall in the mode chooser beside the effect's",
         "   name. Drag Decay slowly from one end to the other and back: the tail gets longer, not louder.",
         "2. Drag Size from one end to the other and back, then Pre-delay the same way: the space grows and",
         "   shrinks, and the gap before it changes. The tail never bends in pitch (no whoop or wobble).",
         "3. Tap Cloud in the mode chooser: the wash rings on smoothly, holding its pitch. Tap Echoes, then the",
         "   effect's name for its presets, and tap Echo chamber. Drag Decay, Size and Diffusion a little each:",
         "   no zipper or click. Leave it playing for 40 s. Tap Remove to take the reverb out, and press MAIN.",
         "4. Press SHIFT + NOTE REPEAT (Latch goes off), then STOP twice: the snare stops."],
        [("no refusal", not_seen(CHAIN_REFUSED)),
         ("no underrun while the reverb played", no_underruns_while()),
         ("Latch went off", seen(r"^Latch off")),
         ("the app kept running", stayed_up())],
        "Did Decay ring longer not louder, did Size and Pre-delay move with no pitch bend, and Echo chamber with no zipper?",
        ["a_mode_across_engines_swaps_in_place_and_switching_back_restores",
         "a_preset_from_the_header_loads_and_keeps_the_mix"],
        needs=['MAIN', 'STOP', 'SHIFT', 'NOTE REPEAT'], wait_status=True),
    Check(
        "chorus-rebuilt", "The rebuilt Chorus: no dip as it comes in, wide at every voice count",
        "#477 (FX quality, PR M1): before it the Chorus went in about 3.5 dB quieter than the sound alone, four "
        "voices had no width, deep settings bent the pitch by semitones, and the highs fluttered", "hands",
        ["1. On Play with kit 1, press SHIFT + NOTE REPEAT (Latch), hold pad 2 (the snare), let go: it repeats.",
         "2. Press TRACK MIX, tap Effects, then Tracks. On the Drums row tap + FX, then Modulation, then Chorus:",
         "   Chorus opens at Mix 50 %. Drag Mix slowly between 0 and 50 % a few times: the snare stays about as",
         "   loud, only wider and thicker as the chorus comes in. It should not dip.",
         "3. Set Voices to 4: in headphones it sounds wide, not in the middle. Set 3, then 2: each wide, no click.",
         "4. Drag Depth from one end to the other and back, then Delay: no zipper, click or seasick pitch bend,",
         "   even at full Depth. Leave it playing for 40 s. Tap Remove to take the chorus out, and press MAIN.",
         "5. Press SHIFT + NOTE REPEAT (Latch goes off), then STOP twice: the snare stops."],
        [("Latch kept the snare repeating", seen(r"^Latch on")),
         ("no refusal", not_seen(CHAIN_REFUSED)),
         ("no underrun while the Chorus played", no_underruns_while()),
         ("Latch went off", seen(r"^Latch off")),
         ("the app kept running", stayed_up())],
        "Did the snare keep its level as the Chorus came in, sound wide at 2, 3 and 4 voices, and sweep with no zipper or bend?",
        ["the_mixer_opens_the_editor_on_a_strip_and_adds_an_effect_from_the_picker",
         "a_preset_from_the_header_loads_and_keeps_the_mix"],
        needs=['SHIFT', 'NOTE REPEAT', 'TRACK MIX / PAD MIX', 'MAIN', 'STOP'], wait_status=True),
    Check(
        "flanger-rebuilt", "The rebuilt Flanger: level kept at any Feedback, and Through zero with no click",
        "#490 (FX quality, PR M2): before it the Flanger went in up to 3 dB quieter and 6.5 dB louder at full "
        "Feedback, its high notches were shallow, and its sweep reached 40 ms; Through zero is new", "hands",
        ["1. On Play with kit 1, press SHIFT + NOTE REPEAT (Latch), hold pad 2 (the snare), let go: it repeats.",
         "2. Press TRACK MIX, tap Effects, then Tracks. On the Drums row tap + FX, then Modulation, then Flanger:",
         "   Flanger opens at Mix 50 %. Drag Feedback slowly end to end and back: more metallic, then hollow,",
         "   but the snare stays about as loud. Drag Delay the same way: the notches move, no zipper or click.",
         "3. Set Through zero (in Output) to On: it glides over, no click. Tap the effect's name for its presets",
         "   and tap Tape flange: over about 8 s the sweep thins to nearly nothing, twice a cycle, and comes back.",
         "4. Set Through zero back to Off: again it glides, no click. Leave it playing for 40 s. The LFO's Shape",
         "   is next, on the same flanger."],
        [("Latch kept the snare repeating", seen(r"^Latch on")),
         ("no refusal", not_seen(CHAIN_REFUSED)),
         ("no underrun while the Flanger played", no_underruns_while()),
         ("the app kept running", stayed_up())],
        "Did the snare keep its level through Feedback, Tape flange thin to nearly nothing and back, and Through zero not click?",
        ["the_mixer_opens_the_editor_on_a_strip_and_adds_an_effect_from_the_picker",
         "a_preset_from_the_header_loads_and_keeps_the_mix"],
        needs=['SHIFT', 'NOTE REPEAT', 'TRACK MIX / PAD MIX'], wait_status=True),
    Check(
        "lfo-shape", "The LFO's Triangle Shape on the Flanger: an even sweep, a smooth turn, no click",
        "#498 (FX quality, PR M4): a Triangle Shape for the chorus's, flanger's and phaser's sweep, with rounded "
        "corners so the turn at each end is quick but never a jump; a change of Shape glides", "hands",
        ["1. With the snare still repeating through the Flanger, drag Rate to about 1 Hz.",
         "2. In the Movement section set Shape to Triangle: the sweep moves up and down at an even pace and",
         "   turns round quickly at each end, with no click or jump at the turn. The switch glides, no click.",
         "3. Set Shape back to Sine: the sweep slows and lingers at each end again, no click. Leave it playing",
         "   for 40 s.",
         "4. Tap Remove to take the flanger out, and press MAIN.",
         "   Press SHIFT + NOTE REPEAT (Latch goes off), then STOP twice: the snare stops."],
        [("no refusal", not_seen(CHAIN_REFUSED)),
         ("no underrun while the Flanger played", no_underruns_while()),
         ("Latch went off", seen(r"^Latch off")),
         ("the app kept running", stayed_up())],
        "Did the Triangle sweep evenly and turn at each end without a click, and did each switch of Shape glide?",
        ["the_mixer_opens_the_editor_on_a_strip_and_adds_an_effect_from_the_picker"],
        needs=['MAIN', 'STOP', 'SHIFT', 'NOTE REPEAT'], wait_status=True),
    Check(
        "phaser-rebuilt", "The rebuilt Phaser: level held at any Feedback, no boom, and Opposite stereo",
        "#495 (FX quality, PR M3): before it the Phaser went in up to 1.9 LU quieter and 7.4 LU louder at high "
        "Feedback, and its lowest few hertz rang 26 dB up; Stereo's Opposite is new", "hands",
        ["1. On Play with kit 1, press SHIFT + NOTE REPEAT (Latch), hold pad 2 (the snare), let go: it repeats.",
         "2. Press TRACK MIX, tap Effects, then Tracks. On the Drums row tap + FX, then Modulation, then Phaser:",
         "   Phaser opens at Mix 50 %. Drag Feedback slowly end to end and back: sharper at the top, hollower at",
         "   the bottom, but the snare stays about as loud, and no boom or rumble builds up at the top.",
         "3. Set Stereo (in Movement) to Opposite: the notches rise on one side as they fall on the other, with",
         "   no click. Set it back to Quarter. Tap the effect's name for its presets and tap Liquid: a slow, deep",
         "   sweep, about as loud as before. Leave it playing for 40 s. Tap Remove to take the phaser out, and",
         "   press MAIN. Press SHIFT + NOTE REPEAT (Latch goes off), then STOP twice: the snare stops."],
        [("Latch kept the snare repeating", seen(r"^Latch on")),
         ("no refusal", not_seen(CHAIN_REFUSED)),
         ("no underrun while the Phaser played", no_underruns_while()),
         ("Latch went off", seen(r"^Latch off")),
         ("the app kept running", stayed_up())],
        "Did the snare keep its level through Feedback, with no boom at the top, and did Opposite sweep the sides apart?",
        ["the_mixer_opens_the_editor_on_a_strip_and_adds_an_effect_from_the_picker",
         "a_preset_from_the_header_loads_and_keeps_the_mix"],
        needs=['SHIFT', 'NOTE REPEAT', 'TRACK MIX / PAD MIX', 'MAIN', 'STOP'], wait_status=True),
    Check(
        "tape-first-echo", "The Tape delay's first echo already sounds like tape",
        "#430 (FX quality, PR C): the Tape's tone, head bump and saturation sit at the record head, so the "
        "first echo has been round the tape once; mpfree-fx's delay and echo_tails tests measure it", "hands",
        ["1. Press TRACK MIX, tap Effects, then Tracks. On the Drums row tap + FX, then Delay and reverb, then",
         "   Delay. In the chain editor, tap Tape in the mode chooser beside the effect's name.",
         "2. Drag Mix to about 50 %, so the echoes are easy to hear.",
         "3. Hit pad 2 (the snare) once every few seconds, and listen to the first echo and the ones after it."],
        [("no refusal", not_seen(CHAIN_REFUSED)),
         ("the app kept running", stayed_up())],
        "Was the first echo already a little darker and warmer than the dry snare, and each later one darker still?",
        ["the_mixer_opens_the_editor_on_a_strip_and_adds_an_effect_from_the_picker",
         "a_mode_across_engines_swaps_in_place_and_switching_back_restores"],
        needs=['TRACK MIX / PAD MIX']),
    Check(
        "dub-echo-dies", "The Tape's Dub echo dies away on its own",
        "#430 (FX quality, PR C): the Tape's saturation adds no gain, so Feedback is the loop's gain and "
        "Dub echo (80 %) decays instead of ringing on", "hands",
        ["1. Tap the effect's name at the top of its page for its presets, and tap Dub echo.",
         "2. Hit pad 2 once, then leave the pads alone and listen for up to half a minute."],
        [("no refusal", not_seen(CHAIN_REFUSED)),
         ("the app kept running", stayed_up())],
        "Did the repeats fade out on their own, well within half a minute?",
        ["a_preset_from_the_header_loads_and_keeps_the_mix"]),
    Check(
        "runaway-howls", "The Tape's Runaway preset still runs away",
        "#430 (FX quality, PR C): the Tape's Feedback goes to 150 %; its Runaway preset, at 130 %, grows into "
        "a soft ceiling and howls there", "hands",
        ["1. Turn the volume down a little: this one gets loud. Tap the name at the top of the page again",
         "   (it reads Dub echo now), and tap Runaway.",
         "2. Hit pad 2 once and wait about 10 seconds: the echoes grow on their own.",
         "3. Drag Feedback down below 100 %: the howl dies away. Tap Remove to take the delay out, and",
         "   press MAIN."],
        [("no refusal", not_seen(CHAIN_REFUSED)),
         ("no underrun while it howled", no_underruns_while()),
         ("the app kept running", stayed_up())],
        "Did the echoes grow into a howl on their own, and die away once Feedback came down?",
        ["a_preset_from_the_header_loads_and_keeps_the_mix"],
        needs=['MAIN'], wait_status=True),
    Check(
        "crush-rate", "Crush's Rate turned while the sound plays",
        "#450 (FX quality, round 2): Crush keeps exact time at every rate, with band-limited steps; its "
        "aliasing and levels are measured by mpfree-fx's crush tests", "hands",
        ["1. On Play with kit 1, press SHIFT + NOTE REPEAT (Latch), hold pad 2, let go: it repeats.",
         "2. Press TRACK MIX, tap Effects, then Tracks. On the Drums row tap + FX, then Drive and colour, then",
         "   Crush. Drag Bits to 10.",
         "3. Drag Rate up to about 22 kHz, then slowly down to about 1 kHz, and slowly back up to 22 kHz."],
        [("Latch kept the pad repeating", seen(r"^Latch on")),
         ("the touchscreen was used", touch_rose()),
         ("no refusal", not_seen(CHAIN_REFUSED)),
         ("the app kept running", stayed_up())],
        "Did Rate's turn down and back stay smooth, with no click or crackle beyond the crush's own grit?",
        ["the_mixer_opens_the_editor_on_a_strip_and_adds_an_effect_from_the_picker",
         "fx_walks_keep_the_chain_editor_and_the_mixer_together"],
        needs=['SHIFT', 'NOTE REPEAT', 'TRACK MIX / PAD MIX']),
    Check(
        "crush-filters", "Crush's Filters switched while the sound plays",
        "#450 (FX quality, round 2): the low-passes an old sampler had before and after its converter, "
        "switched with a crossfade", "hands",
        ["1. Tap the effect's name at the top of its page for its presets, and tap 12-bit classic.",
         "2. With the pad still repeating, switch Filters through Off, Before, After and Both, and round",
         "   again a few times.",
         "3. Tap Remove to take the Crush out, and press MAIN.",
         "   Press SHIFT + NOTE REPEAT (Latch goes off), then STOP twice: the pad stops."],
        [("no refusal", not_seen(CHAIN_REFUSED)),
         ("Latch went off", seen(r"^Latch off")),
         ("the app kept running", stayed_up())],
        "Did each Filters switch go through without a click?",
        ["a_preset_from_the_header_loads_and_keeps_the_mix",
         "fx_walks_keep_the_chain_editor_and_the_mixer_together"],
        needs=['MAIN', 'STOP', 'SHIFT', 'NOTE REPEAT']),
    Check(
        "comp-style", "The Compressor's Style switched while a drum loop plays",
        "#469 (FX quality, round 2): four styles (Clean, Opto, Bus, FET), switched with a crossfade", "hands",
        ["1. On Play with kit 1, press SHIFT + NOTE REPEAT (Latch), hold pad 2, let go: it repeats. Press PLAY:",
         "   the kick from the step checks plays with it.",
         "2. Press TRACK MIX, tap Effects, then Tracks. On the Drums row tap + FX, then Dynamics, then Compressor.",
         "   Tap the effect's name at the top of its page for its presets, and tap Drum punch.",
         "3. Switch Style through Clean, Opto, Bus and FET, and round again a few times."],
        [("Latch kept the pad repeating", seen(r"^Latch on")),
         ("the touchscreen was used", touch_rose()),
         ("no refusal", not_seen(CHAIN_REFUSED)),
         ("the app kept running", stayed_up())],
        "Did any Style switch click or jump in level?",
        ["a_preset_from_the_header_loads_and_keeps_the_mix",
         "fx_walks_keep_the_chain_editor_and_the_mixer_together"],
        needs=['SHIFT', 'NOTE REPEAT', 'PLAY', 'TRACK MIX / PAD MIX']),
    Check(
        "comp-moves", "The Compressor's Threshold, Ratio and Auto makeup moved while it plays",
        "#469 (FX quality, round 2): the Compressor's controls glide", "hands",
        ["1. Tap the effect's name at the top of its page for its presets again, and tap Gentle glue.",
         "2. Drag Threshold slowly down to -40 dB and back, then Ratio from 2 up to 20 and back.",
         "3. Switch Auto makeup on and off a few times.",
         "4. Tap Remove to take the Compressor out, and press MAIN.",
         "   Press SHIFT + NOTE REPEAT (Latch goes off), then STOP twice: all stops."],
        [("no refusal", not_seen(CHAIN_REFUSED)),
         ("no underrun while it played", no_underruns_while()),
         ("Latch went off", seen(r"^Latch off")),
         ("the app kept running", stayed_up())],
        "Did any move of Threshold, Ratio or Auto makeup zipper or click?",
        ["the_chain_editors_taps_reach_the_app",
         "a_preset_from_the_header_loads_and_keeps_the_mix"],
        needs=['MAIN', 'STOP', 'SHIFT', 'NOTE REPEAT'], wait_status=True),
    Check(
        "filter-switches", "The Multi filter's Type and Slope switched on a playing drum loop, with no click",
        "#482 (FX quality round 2, Filters): Type and Slope crossfade instead of jumping, so a switch while it "
        "plays doesn't click", "hands",
        ["1. On Play, press PLAY: the kick from the step checks plays. Press SHIFT + NOTE REPEAT (Latch), hold",
         "   pad 2 (the snare), let go: it repeats.",
         "2. Press TRACK MIX, tap Effects, then Tracks. On the Drums row tap + FX, then EQ and filter, then Multi",
         "   filter: its page opens. Drag Cutoff down to about 1 kHz.",
         "3. Switch Type between Low-pass and High-pass a few times, then Slope between 12 dB and 24 dB a few",
         "   times, listening at each switch.",
         "4. Tap Remove to take the filter out, and press MAIN.",
         "   Press SHIFT + NOTE REPEAT (Latch goes off), then STOP twice: all stops."],
        [("Latch kept the pad repeating", seen(r"^Latch on")),
         ("no refusal", not_seen(CHAIN_REFUSED)),
         ("Latch went off", seen(r"^Latch off")),
         ("the app kept running", stayed_up())],
        "Did any switch of Type or Slope click or pop?",
        ["the_mixer_opens_the_editor_on_a_strip_and_adds_an_effect_from_the_picker",
         "the_chain_editors_taps_reach_the_app"],
        needs=['SHIFT', 'NOTE REPEAT', 'TRACK MIX / PAD MIX', 'MAIN', 'STOP']),
    Check(
        "multiband-squash", "The Multiband comp's Drum crush at Mix 50 % on the drum loop, crossovers swept",
        "#521 (FX quality round 4): the Multiband comp, Low, mid and high compressed apart and summed flat, "
        "with a Mix for parallel squash", "hands",
        ["1. On Play with kit 1, press PLAY: the kick from the step checks plays. Press SHIFT + NOTE REPEAT",
         "   (Latch), hold pad 2 (the snare), let go: it repeats.",
         "2. Press TRACK MIX and tap Effects. On the Drums row tap + FX, then Dynamics, then Multiband comp.",
         "   Tap the effect's name for its presets, tap Drum crush, and drag Mix to 50 %.",
         "3. Drag Low | mid slowly from one end to the other and back, then Mid | high the same way.",
         "4. Tap Remove to take it out, and press MAIN.",
         "   Press SHIFT + NOTE REPEAT (Latch goes off), then STOP twice: all stops."],
        [("Latch kept the pad repeating", seen(r"^Latch on")),
         ("no refusal", not_seen(CHAIN_REFUSED)),
         ("no underrun while it played", no_underruns_while()),
         ("Latch went off", seen(r"^Latch off")),
         ("the app kept running", stayed_up())],
        "Did the blend sound full all through both sweeps, with no hollow or phasey spot and no click?",
        ["the_mixer_opens_the_editor_on_a_strip_and_adds_an_effect_from_the_picker",
         "a_preset_from_the_header_loads_and_keeps_the_mix"],
        needs=['SHIFT', 'NOTE REPEAT', 'TRACK MIX / PAD MIX', 'MAIN', 'STOP'], wait_status=True),
    Check(
        "gate-bass", "The Gate on the kick's tails with Hold at 0, as a Gate and as an Expander",
        "#527 (FX quality round 4): the Gate holds open at least 20 ms, so a low note no longer flutters as it "
        "shuts, and its Mode adds an Expander", "hands",
        ["1. On Play with kit 1, press PLAY: the kick from the step checks plays.",
         "2. Press TRACK MIX and tap Effects. On the Drums row tap + FX, then Dynamics, then Gate. Drag Hold to 0.",
         "3. Drag Threshold slowly up until each kick's tail is cut short, and listen to a few as they die away.",
         "4. Switch Mode to Expander and listen to a few more.",
         "5. Tap Remove to take the Gate out, press MAIN, and press STOP twice: all stops."],
        [("no refusal", not_seen(CHAIN_REFUSED)),
         ("no underrun while it played", no_underruns_while()),
         ("the app kept running", stayed_up())],
        "As each kick died away through the Gate or the Expander, did its tail ever flutter or buzz?",
        ["the_mixer_opens_the_editor_on_a_strip_and_adds_an_effect_from_the_picker",
         "the_chain_editors_taps_reach_the_app"],
        needs=['TRACK MIX / PAD MIX', 'MAIN'], wait_status=True),
    Check(
        "transient-snap", "The Transient shaper on soft and hard snare hits, on and bypassed",
        "#527 (FX quality round 4): the Transient shaper's Attack adds the same snap to a soft hit as to a "
        "hard one", "hands",
        ["1. Press TRACK MIX and tap Effects. On the Drums row tap + FX, then Dynamics, then Transient shaper:",
         "   it opens at its defaults (Attack 50).",
         "2. Hit pad 2 (the snare) a few times very softly, then a few times hard.",
         "3. Tap ON at the right end of its row in the chain on the left: it reads OFF. Hit pad 2 softly and",
         "   hard again, to hear the snare without it.",
         "4. Tap Remove to take it out, and press MAIN."],
        [("soft and hard hits", pad_hits()),
         ("the touchscreen was used", touch_rose()),
         ("no refusal", not_seen(CHAIN_REFUSED)),
         ("the app kept running", stayed_up())],
        "Did the shaper add about the same snap to the soft hits as to the hard ones?",
        ["the_mixer_opens_the_editor_on_a_strip_and_adds_an_effect_from_the_picker",
         "an_insert_added_on_the_mixer_is_turned_bypassed_moved_and_removed"],
        needs=['TRACK MIX / PAD MIX', 'MAIN']),
    Check(
        "deesser-voice", "The De-esser on a voice: its defaults, Range 24, Split and Wide (only with a voice)",
        "#527 (FX quality round 4): the De-esser catches each \"s\" from its start, turns it down as far as "
        "Range says, and in Split never touches the voice under Freq", "hands",
        ["Only with a sung or spoken sound with clear \"s\" sounds in the stick's Samples (s skips this check).",
         "1. Tap the Pads tab and tap pad 16 to pick it. Tap Browse for A16 (top right), then Samples, find the",
         "   voice, tap it, and tap Load on A16. Press BROWSE to close the browser: pad 16 plays the voice.",
         "2. Press TRACK MIX and tap Effects. On the Drums row tap + FX, then Dynamics, then De-esser. Hit pad 16",
         "   and listen at its defaults.",
         "3. Drag Range to 24 dB and hit pad 16 again. While it plays, switch Mode between Split and Wide a few",
         "   times.",
         "4. Tap Remove to take it out, and press MAIN."],
        [("the voice loaded onto pad 16", seen(r"^sample: loaded: .* on pad A16(,|$)")),
         ("no refusal", not_seen(CHAIN_REFUSED)),
         ("the app kept running", stayed_up())],
        "Did each \"s\" lose its edge from its start, the voice stay the same under it in Split, and no Mode switch click?",
        ["load_puts_a_sound_on_the_pad_shown_and_load_to_pad_on_the_next_hit",
         "the_mixer_opens_the_editor_on_a_strip_and_adds_an_effect_from_the_picker"],
        needs=['TRACK MIX / PAD MIX', 'MAIN']),
    Check(
        "drive-modes", "Drive's six modes stepped through while a chord plays",
        "#445 (FX quality, round 2): a change of Drive's mode or Quality fades over 10 ms", "hands",
        ["1. Hold SHIFT and turn the wheel to the last kit, \"Synth: Poly\". Tap the Synth tab and pick the",
         "   Supersaw preset.",
         "2. Press TRACK MIX, tap Effects, then Tracks. On the Synth row tap + FX, then Drive and colour, then",
         "   Drive: Drive \u00b7 Soft opens. Drag Drive up to about 18 dB.",
         "3. Hold pads 1, 5 and 8. With the other hand tap \u203a beside the mode's name through Hard, Tube,",
         "   Foldback, Fuzz and Rectify, then \u2039 back to Soft (past those two are Character's modes).",
         "   Let go of the pads.",
         "4. Tap Remove to take the Drive out, and press MAIN."],
        [("the touchscreen was used", touch_rose()),
         ("no refusal", not_seen(CHAIN_REFUSED)),
         ("no underrun while the chord played", no_underruns_while()),
         ("the app kept running", stayed_up())],
        "Did every change of mode go through without a click?",
        ["a_mode_of_the_same_engine_sets_its_value_and_one_of_another_comes_back",
         "the_mixer_opens_the_editor_on_a_strip_and_adds_an_effect_from_the_picker"],
        needs=['SHIFT', 'TRACK MIX / PAD MIX', 'MAIN'], wait_status=True),
    Check(
        "amp-turns", "The Amp's Gain and Treble turned while a note plays",
        "#452 (FX quality, round 2): Gain changes the grit, not the level, and the knobs glide", "hands",
        ["1. Press TRACK MIX, tap Effects, then Tracks. On the Synth row tap + FX, then Drive and colour, then",
         "   Amp. Tap the effect's name at the top of its page for its presets, and tap Modern lead.",
         "2. Hold pad 1. With the other hand drag Gain slowly down to 0, up to 10, and back to 8.",
         "3. Still holding pad 1, drag Treble from 0 to 10 and back quickly, a few times, and leave it at 5.",
         "   Let go of the pad."],
        [("the touchscreen was used", touch_rose()),
         ("no refusal", not_seen(CHAIN_REFUSED)),
         ("no underrun while the Amp played", no_underruns_while()),
         ("the app kept running", stayed_up())],
        "Did Gain change the grit with no jump in level, and did the turns stay free of clicks and zipper noise?",
        ["a_preset_from_the_header_loads_and_keeps_the_mix",
         "the_chain_editors_taps_reach_the_app"],
        needs=['TRACK MIX / PAD MIX'], wait_status=True),
    Check(
        "amp-switches", "The Amp's Model and Cabinet switched while a note plays",
        "#452 (FX quality, round 2): a change of model, boost or cabinet crossfades", "hands",
        ["1. Hold pad 1 again. Switch Model through Clean, Crunch, Lead and Bass, and round again a few times.",
         "2. Still holding it, switch Cabinet (in the Output section) through Off, 1x12 open, 2x12, 4x12 and",
         "   1x15 bass, and round again a few times. Let go of the pad."],
        [("no refusal", not_seen(CHAIN_REFUSED)),
         ("the app kept running", stayed_up())],
        "Did every Model and Cabinet switch go through without a click?",
        ["fx_walks_keep_the_chain_editor_and_the_mixer_together"]),
    Check(
        "amp-gate", "The Amp's gate in the rests",
        "#452 (FX quality, round 2): the Gate shuts hiss in the rests and opens without clipping the start",
        "hands",
        ["1. Tap the name at the top of the page for the presets again, and tap Tight rhythm.",
         "2. Hit pads 1, 5 and 8 together in short stabs, one every two seconds or so, for about 20 s.",
         "3. Tap Remove to take the Amp out, and press MAIN."],
        [("no refusal", not_seen(CHAIN_REFUSED)),
         ("the app kept running", stayed_up())],
        "In the rests, was it silent with no chatter, and did each chord start whole, not clipped by the gate?",
        ["a_preset_from_the_header_loads_and_keeps_the_mix"],
        needs=['MAIN']),
    Check(
        "fuzz-turns", "Fuzz's Fuzz, Tone and Gate turned while a note plays",
        "#453 (FX quality, round 2): a new Fuzz effect; its knob changes the fuzz, not the loudness", "hands",
        ["1. Press TRACK MIX, tap Effects, then Tracks. On the Synth row tap + FX, then Drive and colour, then",
         "   Fuzz (its own tile, not Drive). Tap the effect's name at the top of its page, and tap Vintage face.",
         "2. Hold pad 1. With the other hand drag Fuzz from 0 up to 100 and back, then Tone from one end to the",
         "   other and back. Let go of the pad.",
         "3. Drag Gate up to about 70. Hold pad 1 for a second, let go, and listen as it dies away.",
         "   Drag Gate back to 0."],
        [("the touchscreen was used", touch_rose()),
         ("no refusal", not_seen(CHAIN_REFUSED)),
         ("no underrun while the Fuzz played", no_underruns_while()),
         ("the app kept running", stayed_up())],
        "Did any turn click or jump in level, and did the note sputter out at the end with Gate up?",
        ["a_preset_from_the_header_loads_and_keeps_the_mix",
         "the_chain_editors_taps_reach_the_app"],
        needs=['TRACK MIX / PAD MIX'], wait_status=True),
    Check(
        "fuzz-switches", "Fuzz's Model and Low keep switched under a bass note",
        "#453 (FX quality, round 2): Low keep leaves the lows under a crossover clean, for 808s and bass",
        "hands",
        ["1. Press MAIN, tap the Synth tab and pick the Sub Bass preset, and press MAIN. Hold SHIFT and turn the",
         "   wheel two clicks left: pad 1 plays C1. Press TRACK MIX, tap Effects, then Tracks, and tap Fuzz on",
         "   the Synth row twice: its page opens.",
         "2. Hold pad 1. Switch Model through Face, Muff and Octave, and round again a few times.",
         "3. Still holding it, switch Low keep through Off, 60 Hz, 100 Hz, 150 Hz and 250 Hz, and round again.",
         "   Let go of the pad.",
         "4. Tap Remove to take the Fuzz out, press MAIN, hold SHIFT and turn the wheel two clicks right (pad 1",
         "   plays C3 again), then tap the Synth tab and pick Supersaw again."],
        [("no refusal", not_seen(CHAIN_REFUSED)),
         ("the app kept running", stayed_up())],
        "Did every Model and Low keep switch go through without a click?",
        ["fx_walks_keep_the_chain_editor_and_the_mixer_together"],
        needs=['MAIN', 'SHIFT', 'TRACK MIX / PAD MIX']),
    Check(
        "lofi-broken", "Lo-fi's Broken machine and Dropouts while a chord plays",
        "#476 (FX quality, round 2): wow and dropouts that dip and wander smoothly; the saturation at twice "
        "the rate", "hands",
        ["1. Press MAIN. Press SHIFT + NOTE REPEAT (Latch), hold pads 1, 5 and 8, let go: the chord repeats.",
         "2. Press TRACK MIX, tap Effects, then Tracks. On the Synth row tap + FX, then Drive and colour, then",
         "   Lo-fi. Tap the effect's name at the top of its page for its presets, and tap Broken machine.",
         "   Listen for about half a minute.",
         "3. Drag Dropouts down to 0, then slowly up to 100 and back."],
        [("Latch kept the chord repeating", seen(r"^Latch on")),
         ("the touchscreen was used", touch_rose()),
         ("no refusal", not_seen(CHAIN_REFUSED)),
         ("no underrun while Lo-fi played", no_underruns_while()),
         ("the app kept running", stayed_up())],
        "Did the dips and the wow dip and wander smoothly, never clicking, crackling or jumping?",
        ["a_preset_from_the_header_loads_and_keeps_the_mix",
         "the_chain_editors_taps_reach_the_app"],
        needs=['MAIN', 'SHIFT', 'NOTE REPEAT', 'TRACK MIX / PAD MIX'], wait_status=True),
    Check(
        "lofi-follow", "Lo-fi's noise bed after STOP: running on at Follow 0, fading at 100",
        "#476 (FX quality, round 2): with Follow at 100 the crackle, hiss and hum ride the music and fade "
        "about a second after it stops; at 0 they run on", "hands",
        ["1. Tap the effect's name at the top of its page for its presets again, and tap Vinyl bed: crackle",
         "   plays under the chord. Press SHIFT + NOTE REPEAT (Latch goes off), then STOP twice: the chord",
         "   stops, but the crackle runs on.",
         "2. Drag Hum up to about half: a mains hum joins the crackle. Then drag Follow up to 100: within about",
         "   a second the crackle, hiss and hum fade away to silence.",
         "3. Tap Remove to take the Lo-fi out, and press MAIN."],
        [("no refusal", not_seen(CHAIN_REFUSED)),
         ("Latch went off", seen(r"^Latch off")),
         ("the app kept running", stayed_up())],
        "Did the noise run on after STOP at Follow 0, and fade away within about a second at Follow 100?",
        ["a_preset_from_the_header_loads_and_keeps_the_mix"],
        needs=['STOP', 'MAIN', 'SHIFT', 'NOTE REPEAT']),
    Check(
        "filter-squelch", "The Multi filter's Acid squelch swept on a synth line: clean when loud, still singing",
        "#482 (FX quality round 2, Filters): the resonance is bounded, so a loud line no longer clips at its "
        "peak, and Drive runs at twice the rate with its curve antialiased", "hands",
        ["1. Press TRACK MIX, tap Effects, then Tracks. On the Synth row tap + FX, then EQ and filter, then Multi",
         "   filter: its page opens. Tap the effect's name at the top of its page for its presets, and tap",
         "   Acid squelch.",
         "2. With one hand play pads 1 to 4 as a line, some hits hard and some soft, without stopping. With the",
         "   other, drag Cutoff slowly from one end to the other and back, twice.",
         "   The Res mode is next, on the same filter."],
        [("the touchscreen was used", touch_rose()),
         ("no refusal", not_seen(CHAIN_REFUSED)),
         ("no underrun while it played", no_underruns_while()),
         ("the app kept running", stayed_up())],
        "Did loud notes stay clean at the top of the sweep, with no clipping or crackle, while soft ones still sang?",
        ["the_mixer_opens_the_editor_on_a_strip_and_adds_an_effect_from_the_picker",
         "a_preset_from_the_header_loads_and_keeps_the_mix"],
        needs=['TRACK MIX / PAD MIX'], wait_status=True),
    Check(
        "filter-resmode", "The Multi filter's Res mode switched while a loud line plays: Soft, Hard, Clean",
        "#489 (FX quality round 2, Filters part two): Res mode picks how the resonance is held back, Soft, Hard "
        "or Clean (the old, unbounded one); the switch crossfades", "hands",
        ["1. Turn the monitors down a little: on Clean a loud note at the cutoff can go over full scale.",
         "2. Tap the effect's name at the top of its page for its presets, and tap Screaming low-pass.",
         "3. With one hand play pads 1 to 4 as a loud line, hard hits, without stopping. With the other, switch",
         "   Res mode between Soft, Hard and Clean a few times, listening at each switch. The Ladder is next."],
        [("no refusal", not_seen(CHAIN_REFUSED)),
         ("the app kept running", stayed_up())],
        "Did Hard sound harder and buzzier than Soft, Clean ring deepest, and no switch of Res mode click?",
        ["the_chain_editors_taps_reach_the_app",
         "a_preset_from_the_header_loads_and_keeps_the_mix"]),
    Check(
        "filter-ladder", "The Multi filter's Ladder singing on its own as Cutoff turns",
        "#489 (FX quality round 2, Filters part two): the Ladder model, which self-oscillates at full "
        "Resonance and holds its level across Cutoff", "hands",
        ["1. Switch Model to Ladder and drag Resonance to 100. Play a few notes on pads 1 to 4, then stop",
         "   playing: the filter sings on its own.",
         "2. Drag Cutoff slowly from about 100 Hz up to 5 kHz and back: the tone follows it at a steady level.",
         "3. Drag Resonance down to 0: the singing stops.",
         "4. Tap Remove to take the filter out, press MAIN, and turn the monitors back up."],
        [("no refusal", not_seen(CHAIN_REFUSED)),
         ("no underrun while it sang", no_underruns_while()),
         ("the app kept running", stayed_up())],
        "Did the Ladder's tone follow Cutoff smoothly at a steady level, with no click, and stop as Resonance went down?",
        ["the_chain_editors_taps_reach_the_app",
         "the_mixer_opens_the_editor_on_a_strip_and_adds_an_effect_from_the_picker"],
        needs=['MAIN'], wait_status=True),
    Check(
        "limiter-hot-master", "Master safety as a Master insert at +8 dB: Safe limiter and Soft clip switched",
        "#511 (FX quality round 4): Master safety's Safe limiter on a true-peak core shared with the Limiter; "
        "the Master strip's + FX offers it as an insert, with Trim and Mode", "hands",
        ["1. Turn the monitors down a little. On Play, press PLAY: the kick plays. Press SHIFT + NOTE REPEAT",
         "   (Latch), hold pads 1, 5 and 8, let go: the Supersaw chord repeats.",
         "2. Press TRACK MIX and tap Effects. On the Master row tap + FX, then Utility, then Master safety.",
         "   Drag Trim to +8 dB (Mode stays Safe limiter). Listen for 20 s to the kick and the chord's low notes.",
         "3. Switch Mode to Soft clip and back to Safe limiter a few times, listening at each switch.",
         "4. Tap Remove to take this Master safety out. The Limiter is next."],
        [("Latch kept the chord repeating", seen(r"^Latch on")),
         ("no refusal", not_seen(CHAIN_REFUSED)),
         ("no underrun while the master was hot", no_underruns_while()),
         ("the app kept running", stayed_up())],
        "Did any switch of Mode click, or did either mode sound harsh or buzz on the kick and the chord's low notes?",
        ["the_mixer_opens_the_editor_on_a_strip_and_adds_an_effect_from_the_picker",
         "master_safety_catches_what_would_clip"],
        needs=['SHIFT', 'NOTE REPEAT', 'TRACK MIX / PAD MIX'], wait_status=True),
    Check(
        "limiter-loud-master", "The Limiter's Loud master on the Master at 8 dB of Gain, Auto release on and off",
        "#511 (FX quality round 4): the Limiter on a true-peak core, with Auto release", "hands",
        ["1. With the loop still playing, on the Master row tap + FX, then Dynamics, then Limiter.",
         "   Tap the effect's name at the top of its page for its presets, tap Loud master, and drag Gain to 8 dB.",
         "2. Listen for 20 s with Auto release on, then switch Auto release off and listen for 20 s more.",
         "3. Tap Remove to take the Limiter out, and press MAIN.",
         "   Press SHIFT + NOTE REPEAT (Latch goes off), then STOP twice: all stops.",
         "   Turn the monitors back up."],
        [("no refusal", not_seen(CHAIN_REFUSED)),
         ("no underrun while the Limiter played", no_underruns_while()),
         ("Latch went off", seen(r"^Latch off")),
         ("the app kept running", stayed_up())],
        "Did the Limiter at 8 dB stay clean, with no pumping or distortion on the kick, with Auto release on and off?",
        ["the_mixer_opens_the_editor_on_a_strip_and_adds_an_effect_from_the_picker",
         "a_preset_from_the_header_loads_and_keeps_the_mix"],
        needs=['MAIN', 'STOP', 'SHIFT', 'NOTE REPEAT'], wait_status=True),
    Check(
        "tremolo-lfo", "The Tremolo on the shared LFO: crisp chops, synced shapes, on the beat as the clip comes round",
        "#518 (FX quality round 5, PR W): the shared LFO behind the Tremolo and Auto-pan, with Ramp shapes and "
        "a sync that keeps its phase to the bar", "hands",
        ["1. On Play, press PLAY: the kick plays. Press SHIFT + NOTE REPEAT (Latch), hold pads 1, 5 and 8, let go.",
         "2. Press TRACK MIX and tap Effects. On the Synth row tap + FX, then Modulation, then Tremolo. Tap the",
         "   effect's name for its presets, tap Eighth chop, then set Sync to 1 bar: each chop is crisp.",
         "3. Set Shape to Ramp up, then Ramp down: no click as each cycle starts again. Set Sync to 1/4: the",
         "   pulse stays on the beat each time the kick's clip comes round.",
         "4. Switch Shape and Mode a few times while it plays: each change is smooth, with no click.",
         "5. Tap Remove to take it out, and press MAIN.",
         "   Press SHIFT + NOTE REPEAT (Latch goes off), then STOP twice: all stops."],
        [("Latch kept the chord repeating", seen(r"^Latch on")),
         ("no refusal", not_seen(CHAIN_REFUSED)),
         ("no underrun while it played", no_underruns_while()),
         ("Latch went off", seen(r"^Latch off")),
         ("the app kept running", stayed_up())],
        "Did the tremolo chop crisply, stay on the beat as the clip came round, and change Shape and Mode with no click?",
        ["the_mixer_opens_the_editor_on_a_strip_and_adds_an_effect_from_the_picker",
         "a_preset_from_the_header_loads_and_keeps_the_mix"],
        needs=['SHIFT', 'NOTE REPEAT', 'TRACK MIX / PAD MIX', 'MAIN', 'STOP'], wait_status=True),
    Check(
        "strip-switches", "The Channel strip's Gate, Comp and EQ switched, and its Order changed, while a chord plays",
        "#527 (FX quality round 4): the Channel strip's switches fade their stage in or out over 20 ms, and a "
        "change of Order dips the sound for about 20 ms", "hands",
        ["1. On Play, press SHIFT + NOTE REPEAT (Latch), hold pads 1, 5 and 8, let go: the chord repeats.",
         "2. Press TRACK MIX and tap Effects. On the Synth row tap + FX, then Dynamics, then Channel strip.",
         "   In the EQ section drag Mid gain up to +9 dB, so the EQ is heard.",
         "3. Switch the Gate, Compressor and EQ sections off and on a few times each (the switch on each",
         "   section's title), listening at each switch.",
         "4. Switch Order (under Signal path) back and forth a few times: a short, smooth dip is expected.",
         "5. Tap Remove to take it out. The Ducker is next, on the same chord."],
        [("Latch kept the chord repeating", seen(r"^Latch on")),
         ("the touchscreen was used", touch_rose()),
         ("no refusal", not_seen(CHAIN_REFUSED)),
         ("no underrun while it played", no_underruns_while()),
         ("the app kept running", stayed_up())],
        "Did any switch of Gate, Compressor or EQ click, or any change of Order do more than a short, smooth dip?",
        ["the_mixer_opens_the_editor_on_a_strip_and_adds_an_effect_from_the_picker",
         "the_chain_editors_taps_reach_the_app"],
        needs=['SHIFT', 'NOTE REPEAT', 'TRACK MIX / PAD MIX'], wait_status=True),
    Check(
        "ducker-pump", "The Ducker's Big pump on the beat, through stops, starts and jumps back to bar 1",
        "#527 (FX quality round 4): the Ducker's Tempo source ducks in time with the song, already down as each "
        "beat lands, and follows the transport", "hands",
        ["1. With the chord still repeating, on the Synth row tap + FX, then Dynamics, then Ducker. Tap the",
         "   effect's name for its presets and tap Big pump.",
         "2. Press PLAY: the kick from the step checks plays, and the chord pumps on each beat.",
         "3. Press STOP once and PLAY again, a few times (the chord plays on while it is stopped). Then, while",
         "   it plays, press PLAY START a few times: it jumps back to bar 1.",
         "4. Tap Remove to take it out, and press MAIN.",
         "   Press SHIFT + NOTE REPEAT (Latch goes off), then STOP twice: all stops."],
        [("PLAY START (83) reached the app", pressed(PLAY_START)),
         ("no refusal", not_seen(CHAIN_REFUSED)),
         ("no underrun while it pumped", no_underruns_while()),
         ("Latch went off", seen(r"^Latch off")),
         ("the app kept running", stayed_up())],
        "Did the pump stay on the kick through every stop, start and jump, with no drift or stutter?",
        ["a_preset_from_the_header_loads_and_keeps_the_mix",
         "the_context_follows_the_transport"],
        needs=['SHIFT', 'NOTE REPEAT', 'MAIN'], wait_status=True),
    Check(
        "midi-effects-heard", "A Chord, then an Echo, in the Synth track's Notes: each pad hit plays a chord that echoes",
        "#512, #517 and #526 (MIDI effects): a track's Notes change its notes before they reach its sound, top to "
        "bottom", "hands",
        ["1. Press TRACK MIX and tap Effects. On the Synth row tap + FX: the chain editor opens, with NOTES at the",
         "   top of the column on the left. Tap + MIDI effect, then Chord: Chord's page opens, on Major.",
         "2. Tap + MIDI effect again, then Echo: it goes in under Chord.",
         "3. Hit pad 1 a few times, about two seconds apart: each hit plays a chord, then repeats it three times,",
         "   softer each time.",
         "4. Tap Echo's row and tap Remove, then Chord's row and Remove, so the chord checks after this play",
         "   as before. Press MAIN."],
        [("the touchscreen was used", touch_rose()),
         ("the app kept running", stayed_up())],
        "Did each hit play a chord and then repeat that chord as softer echoes?",
        ["the_notes_stage_edits_the_tracks_midi_effects",
         "a_tracks_midi_effects_reach_the_pads_from_the_project"],
        needs=['TRACK MIX / PAD MIX', 'MAIN']),
    Check(
        "fx-busy", "Character on four tracks and Drive on two, for a minute",
        "#445 (FX quality, round 2): Drive and Character oversampled; a busy project on the MPC's cores",
        "hands",
        ["1. Press TRACK MIX, tap Effects, then Tracks. On the Drums row tap + FX, then Drive and colour, then",
         "   Drive's presets line, and tap Pushed tape: Drive \u00b7 Tape opens. Tap \u2039 at the top left. Do the same",
         "   on the Synth, Loop and Audio rows. Then on the Drums and Synth rows add Drive's Hard clip the same way.",
         "2. Press MAIN, then PLAY: the kick plays. Press SHIFT + NOTE REPEAT (Latch), hold pads 1, 5 and 8, let",
         "   go: the chord repeats. Listen for a minute. Press SHIFT + NOTE REPEAT (Latch off), then STOP twice.",
         "3. Press TRACK MIX, tap Effects, then Tracks. Tap each Drive \u00b7 Tape and Drive \u00b7 Hard twice (it",
         "   opens), tap Remove, then \u2039, until all six are out. Tap the Sample tab: the pads go back to kit 1.",
         "   Press MAIN."],
        [("Latch kept the chord repeating", seen(r"^Latch on")),
         ("Latch went off", seen(r"^Latch off")),
         ("no refusal", not_seen(CHAIN_REFUSED)),
         ("no underrun while it played", no_underruns_while()),
         ("the app kept running", stayed_up())],
        "Did the minute play through with no dropout or crackle?",
        ["the_picker_adds_with_a_preset_and_keeps_favourites_and_recents",
         "the_mixer_opens_the_editor_on_a_strip_and_adds_an_effect_from_the_picker"],
        needs=['TRACK MIX / PAD MIX', 'MAIN', 'PLAY', 'STOP', 'SHIFT', 'NOTE REPEAT'], wait_status=True),
    Check(
        "mpc-plugin", "An Open MPC plugin instrument plays on the MPC",
        "#385: MPC-format plugins in the plugin sandbox; CI runs them as ARM only under qemu", "hands",
        ["1. Open the Synth tab and press MENU. The top row is Track N instrument: press + until it reads",
         "   [SYN] Mono Voice (after the built-in synths). MENU closes the options.",
         "2. Play pads 1 to 8 for about 10 s, soft and hard.",
         "3. Hold a pad for 30 s, so a status line counts the plugin."],
        [("the app found the plugin at its start",
          at_start(r"^plugins: \d+ effect\(s\) and [1-9]\d* instrument\(s\) in \S*files/plugins/?; "
                   r"each runs in its own process", r"^plugins: \d+ effect")),
         ("a track plays it", seen(r"^track \d+: plays \[SYN\] Mono Voice")),
         ("it started in its own process", seen(r"^plugins: \[SYN\] Mono Voice started in its own process")),
         ("a status line counts it running, none down",
          seen(r"^status: plugins: \d+ found \u00b7 [1-9]\d* running \u00b7 0 down \u00b7 0 crashes")),
         ("it did not refuse to start", not_seen(r"^plugins: .* would not start")),
         ("no underrun while it played", no_underruns_while())],
        "Did the pads play a synth sound from the MPC's outputs, following how hard you hit?",
        ["a_plugin_made_with_the_app_as_its_process_plays",
         "plugins_live_in_the_data_folder_or_else_a_temp_folder_never_the_real_home"],
        wait_status=True),
    Check(
        "mpc-plugin-install", "Install a plugin from the stick and over Ethernet, and see its own screen",
        "#398: BROWSE's Plugins place (zips on the stick, the Open MPC catalog over HTTPS through the system's "
        "libcurl, by the mpfree-fetch helper beside the static pad player) and MPC plugin skins on the Synth tab",
        "hands",
        ["1. Press BROWSE. Hold SHIFT and turn the wheel to Plugins (it sits after Recordings).",
         "2. Tap On the stick, pick Tidal-Verb-1.6.0-mpc-armv7.zip, then tap Install from zip.",
         "3. Tap Catalog, then Instruments. Pick Modal Voice and tap Install: the bar shows the download.",
         "   If the catalog says there is no network, go on: its line in the log says why.",
         "4. Open the Synth tab and press MENU. On the top row, Track N instrument, press + to [SYN] Modal Voice,",
         "   or [SYN] Mono Voice if step 3 could not run. MENU closes the options: the plugin's own screen",
         "   fills the page under the top bar. Play a few pads."],
        [("the Plugins place says what is there and whether downloads can work",
          seen(r"^plugins: \d+ installed, [1-9]\d* zip\(s\) on the stick, .*, "
               r"downloads (possible \((libcurl|mpfree-fetch) found\)|off \(no (libcurl|mpfree-fetch)\))")),
         ("libcurl's OpenSSL can make the random numbers TLS needs (the final round's 'Insufficient randomness')",
          seen(r"^plugins: .*downloads possible \((libcurl|mpfree-fetch) found\), OpenSSL \S+ ready")),
         ("the catalog came over the network, or the line says why not",
          seen(r"^plugins: (The catalog lists \d+ plugins; \d+ run on this MPC|not done \(catalog\): )")),
         ("the zip on the stick installed", seen(r"^plugins: (Installed|Updated) .*Tidal")),
         ("the Synth tab offers the plugins again after the install", seen(r"^plugins: \d+ offered after an install")),
         ("Modal Voice installed from the catalog (needs a route out)", seen(r"^plugins: Installed .*Modal Voice")),
         ("a track plays the plugin", seen(r"^track \d+: plays \[SYN\] (Modal|Mono) Voice")),
         ("it started in its own process", seen(r"^plugins: \[SYN\] (Modal|Mono) Voice started in its own process")),
         ("its own screen is drawn on the Synth tab",
          seen(r"^screen: mpc:.*: its own screen, \d+ tab\(s\), on the Synth tab")),
         ("it did not refuse to start", not_seen(r"^plugins: .* would not start"))],
        "Did the plugin's own screen (its knobs and pictures, not MPFree's list) show, and did the pads play it?",
        ["the_plugins_place_lists_the_stick_and_deletes_a_zip_on_the_second_tap",
         "an_mpc_plugins_skin_fills_the_synth_tab_and_mpfree_page_is_one_tap_away",
         "lists_a_zip_on_the_stick_and_refuses_one_that_is_not_a_plugin",
         "installed_and_older_says_update_and_remove_asks_twice",
         "the_log_says_what_is_there_and_what_was_done",
         "the_log_says_what_libcurls_tls_said"],
        needs=['BROWSE']),
    Check(
        "power", "The power button opens the power card",
        "#389: a short press opens the card; Shut down and Restart end the boot, so the stick logs judge them",
        "hands",
        ["1. Press the power button briefly (don't hold it): \"Shut down the MPC?\" opens over the page.",
         "   Tap Cancel: back on the page.",
         "2. Press it briefly again, then briefly once more: the second press closes the card."],
        [("the app reads the power button",
          at_start(r"^power button: .*read-only, not grabbed; a short press opens the power card", r"^power button: ")),
         ("the card opened twice", seen_times(r"^power: the power card is open", 2))],
        "Did the card open over the page each time, and close with Cancel and with the second press?",
        ["a_short_press_of_the_power_button_opens_the_power_card",
         "a_press_is_short_only_when_the_button_comes_back_up_in_time",
         "the_power_key_is_found_by_name_and_key_not_by_number",
         "shut_down_and_restart_go_to_the_systems_file",
         "a_request_is_one_word_in_a_whole_file"]),
    Check(
        "first-start", "The USB creator's first-start setup: the backup, then the MPC's kits and samples",
        "#386: the read-only eMMC window was approved on condition that it runs on the reference MPC first",
        "auto",
        looks=[("the setup ran at this start",
                at_start(r"^first start: setup, try [1-3] of 3$", r"^first start: ")),
               ("the backup was copied, read back and checked, with no write to the eMMC",
                at_start(r"^first start: (backup done|using the backup made before): \S*/backup/"
                         r"mpc-one-emmc-\d{8}T\d{6}Z$", r"^first start: ")),
               ("the MPC's own kits and samples came in",
                at_start(r"^first start: done: [1-9]\d* Akai kits, [1-9]\d* samples; the backup is in ",
                         r"^first start: "))],
        smoke=["needed_only_with_the_service_and_until_done",
               "kits_and_samples_from_the_backup",
               "a_failed_or_missing_backup_is_tried_again_later",
               "one_bar_for_the_whole_first_start"]),
    Check(
        "kept", "The settings partition 4 kept came back at this start",
        "Prefs saved in one boot and used in the next (Sound cores, Loop memory)", "auto",
        looks=[("the start's settings agree with settings/app.conf", kept_settings())],
        smoke=["prefs_sound_cores_is_kept_and_used_from_the_next_start",
               "loop_memory_is_kept_and_used_from_the_next_start",
               "prefs_and_buttons_come_back_as_they_were"]),
    Check(
        "play-off-steps", "Play: a tap switches a step off and keeps it, ERASE + a tap deletes it",
        "The Play, Pads and Sequencer redesign's step 3b (Jay's default, 18 Nov): a lit step tapped on Play's "
        "rows is kept but not played", "hands",
        ["1. On Play's All view, tap the Kick row's steps 1, 5, 9 and 13 if they are dark. Press PLAY.",
         "2. Tap Kick's step 5: it turns into an outline with \"off\" in it, and the kick stops playing there.",
         "3. Tap step 5 again: it lights and the kick plays there again.",
         "4. Hold ERASE and tap step 9: it goes dark. Let go of ERASE and press UNDO: step 9 comes back. STOP."],
        [("a step switched off", seen(r"^steps: step \d+ of pad \d+: On\(false\)$")),
         ("and on again", seen(r"^steps: step \d+ of pad \d+: On\(true\)$")),
         ("ERASE + a tap deleted one", seen(r"^steps: step \d+ of pad \d+ erased$"))],
        "Did the kick drop out on step 5 only while it showed \"off\", and come back on the next tap, with no click?",
        ["a_tap_switches_a_step_off_and_keeps_it",
         "steps_tapped_on_beats_play_in_time"],
        needs=['ERASE', 'UNDO']),
    Check(
        "xy-pick", "The XY pad's chip picks what it plays: Wash, then Filter + echo again",
        "Jay, 18 Nov 07:34Z: pick what the XY pad plays from a list on its chip; an XY FX goes on the master",
        "hands",
        ["1. On Play's All view, tap the chip at the XY pad's top left (XY - Filter + echo): a list of ten",
         "   takes the pattern's place.",
         "2. Tap Wash: the list closes and the chip says XY - Wash. Press PLAY.",
         "3. Hold a finger on the XY pad and move it about for a few seconds, then lift it.",
         "4. Tap the chip again and tap Filter + echo. Move a finger on the pad: the filter and echo as before. STOP."],
        [("Wash picked", seen(r"^XY pad: plays Wash$")),
         ("Filter + echo picked again", seen(r"^XY pad: plays Filter \+ echo$")),
         ("no refusal", not_seen(CHAIN_REFUSED)),
         ("no underrun while it played", no_underruns_while()),
         ("the app kept running", stayed_up())],
        "Did the wash swell while your finger was down and ring out when you lifted it, with no click?",
        ["the_xy_pads_chip_picks_what_it_plays_from_a_list",
         "the_xy_pads_pick_puts_an_xy_fx_on_the_master_and_plays_it",
         "the_xy_list_offers_ten_choices_a_fingertip_each"]),
    Check(
        "hour", "An hour of play with no underrun",
        "#273's underruns at 25 and 51 minutes", "auto",
        looks=[("an hour, no underrun, no restart", hour_long())],
        smoke=["a_steady_period_never_allocates",
               "the_sound_path_never_allocates_on_two_cores"]),
)
BY_NAME = {c.name: c for c in CHECKS}

# What the device rounds used to ask by hand and the app's tests now check on every pull request,
# by area. `smoke` reports these after the checks' own tests.
AREAS = (
    ("Arrange: clips, drags, Capture, the song and the morph",
     ["a_dropped_clip_keeps_playing_where_it_lands_and_undo_takes_it_back",
      "song_drags_move_blocks_and_sections_and_undo_takes_them_back",
      "song_drags_copy_with_shift_and_the_wheel_moves_the_pick",
      "the_clips_page_shows_the_engine_and_turns_taps_into_actions",
      "rec_overdubs_a_clap_into_the_verse_and_it_plays_next_pass",
      "capture_keeps_a_jam_over_the_verse",
      "tidal_suite_becomes_a_song_from_its_scenes_and_keeps_it",
      "night_market_in_song_mode_follows_its_arrangement",
      "the_morph_crossfader_brings_the_lead_up_from_seed_to_bloom",
      "erase_held_deletes_a_clip_a_lanes_steps_or_a_layer"]),
    ("Play: views, steps, throws, XY pad and FILL",
     ["one_tap_switches_the_play_view_and_the_q_links_follow_it",
      "the_mode_buttons_pick_play_views_and_sample_editors",
      "the_mode_buttons_go_to_the_page_with_the_same_job",
      "steps_tapped_on_beats_play_in_time",
      "the_beats_view_shows_your_timing",
      "the_pattern_throws_and_xy_pad_reach_the_engines",
      "the_xy_pad_plays_the_dj_filter_and_echo_and_glides_home",
      "a_tap_switches_a_step_off_and_keeps_it",
      "the_xy_pads_chip_picks_what_it_plays_from_a_list",
      "play_held_while_rolling_is_a_fill_and_twice_latches_it",
      "fill_held_plays_the_fill_notes_and_only_then",
      "the_master_bar_and_the_mixer_master_are_one_level"]),
    ("Mix: strips, effect chains, the EQ, modes, learn, macros and I/O",
     ["the_mix_tab_holds_the_mixer_and_its_chains_and_track_mix_goes_back",
      "the_mixer_strip_controls_set_the_mixer",
      "the_effects_look_adds_picks_turns_and_moves_an_effect",
      "learn_puts_a_knob_on_the_next_qlink_turned",
      "a_learned_q_link_follows_its_control_to_a_new_engine",
      "a_mode_across_engines_swaps_in_place_and_switching_back_restores",
      "a_knob_given_to_a_macro_moves_with_the_macro",
      "q_link_steps_plays_column_through_the_macro_banks",
      "the_io_view_follows_its_section_and_says_why_a_setting_stays",
      "track_mix_comes_and_goes_and_undo_takes_back_strip_changes",
      "room_to_plate_keeps_the_shared_knobs_and_learned_q_links_follow",
      "the_eq_look_adds_picks_and_shapes_an_eq",
      "a_drag_on_the_eq_look_is_what_you_hear",
      "a_drag_on_the_eq_page_is_what_you_hear"]),
    ("Looper tab: layers, tools, undo history, trim and drags",
     ["the_looper_tab_plays_loops_and_works_layers",
      "a_layer_dragged_onto_a_loop_moves_or_copies_and_another_length_says_why",
      "a_dragged_layer_moves_between_loops_and_undo_says_so",
      "the_undo_history_steps_back_and_forward",
      "the_ruler_nudges_by_sixteenths_and_its_grips_trim",
      "records_from_the_loops_resamples_into_an_empty_loop",
      "the_last_layers_x_clears_and_a_loop_mutes_without_being_picked",
      "the_controls_ask_for_the_looper_edits_while_the_loop_plays",
      "the_tools_the_undo_history_and_trim_work_while_the_loop_plays",
      "layers_move_between_loops_and_speakers_clear_and_x_work",
      "a_ring_acts_on_the_press_and_a_hold_clears_the_loop",
      "the_q_links_turn_the_loops_levels_then_the_picked_loop",
      "the_pads_as_loop_keys_record_overdub_mute_and_clear",
      "shift_sampler_again_on_the_looper_tab_makes_the_pads_loop_keys",
      "once_solo_and_a_clear_that_undo_brings_back",
      "a_replace_overdub_takes_the_place_of_what_was_there",
      "half_speed_plays_the_loop_an_octave_down_and_an_overdub_fits_to_it"]),
    ("Pads tab and Sample tab: Pad, Chop, Stops, scenes and FX",
     ["the_pads_tab_picks_copies_and_clears_pads_and_undo_takes_it_back",
      "the_pads_tab_follows_play_and_takes_the_sample_tabs_controls",
      "map_to_keys_makes_the_bank_one_instrument_and_the_play_tab_plays_its_keys",
      "the_sample_page_edits_the_pad_hit_last_and_sends_one_view_at_a_time",
      "the_chop_tab_puts_slices_on_the_kits_pads_and_undo_takes_them_back",
      "live_chop_drops_cuts_where_the_sampler_marked_them_and_ends_with_the_sound",
      "hits_on_pads_edit_stops_and_also_plays_and_latch_is_a_row",
      "send_to_pad_copies_the_pad_onto_the_next_hit_and_chop_to_pads_opens_the_chop_tab",
      "scenes_morph_the_playing_kit_and_return_puts_the_pads_back",
      "the_sample_tabs_fx_shows_the_pad_then_the_track"]),
    ("Synth, Grains and Prefs",
     ["the_synth_options_pick_the_engine_the_scale_and_the_pad_layouts",
      "the_synth_options_pick_the_tuning_its_root_and_a4",
      "the_synth_page_shows_poly_and_sends_its_changes",
      "the_grain_tab_sets_the_grains_and_draws_the_cloud_the_audio_thread_fills",
      "menu_and_shift_menu_open_options_and_prefs",
      "the_wheels_push_opens_a_row_for_the_wheel_and_a_second_push_sets_it"]),
    ("Every control, random walks and the demo songs",
     ["every_control_on_every_screen_does_something",
      "every_swept_control_is_in_the_catalogue",
      "random_walks_keep_the_screen_and_the_engines_together",
      "fx_walks_keep_the_chain_editor_and_the_mixer_together",
      "every_song_opens_from_browse_as_it_was_written",
      "each_song_edited_saves_and_opens_again_as_edited"]),
)


class Unknown(Exception):
    pass


def chosen(text):
    """The checks MPFREE_APP_CHECKS names, in its order, without repeats. Raises Unknown for a name not here."""
    names = []
    for part in (text or "").split(","):
        name = part.strip()
        if name and name not in names:
            names.append(name)
    unknown = [n for n in names if n not in BY_NAME]
    if unknown:
        raise Unknown(f"unknown check(s) {', '.join(unknown)}; the checks are {', '.join(BY_NAME)} "
                      "(python3 tools/mpc-one/app_checks.py list)")
    return [BY_NAME[n] for n in names]


def control_key(name):
    """A control's key in partition 4's settings/map.conf, as the app writes it ("PAD BANK A/E" is pad-bank-a-e)."""
    return {"-": "minus", "+": "plus"}.get(name) or "-".join(w for w in re.split(r"[^a-z0-9]+", name.lower()) if w)


# Every button the app knows from the start (`ButtonMap::measured` and the measured constants in
# rust/mpfree-app/src/app/mapper.rs). Since the creator's fifth stick (2026-11-16) that is every button on the
# panel, so no check needs Map controls first on a build from then on. test_app_checks keeps it in step with
# the app's list.
BUILT_IN = frozenset([
    "REC", "OVERDUB", "STOP", "PLAY", "PLAY START", "UNDO", "TAP TEMPO", "PAD BANK A/E", "PAD BANK B/F",
    "PAD BANK C/G", "PAD BANK D/H", "FULL LEVEL", "16 LEVEL", "NOTE REPEAT", "ERASE", "COPY", "TC", "SHIFT",
    "MENU", "MAIN", "BROWSE", "TRACK MIX / PAD MIX", "-", "+", "Q-LINK", "TRACK MUTE", "NEXT SEQ", "STEP SEQ",
    "SAMPLER", "SAMPLE EDIT", "PROG EDIT", "DATA DIAL PUSH",
])


def not_found(checks, map_text, built_in=BUILT_IN):
    """[(check, control)] for each button a check uses that the app doesn't have built in and settings/map.conf
    has no number for (`built_in` empty: as on a build from before every button was built in)."""
    out = []
    for c in checks:
        for name in c.needs:
            if name in built_in:
                continue
            value = setting(map_text, control_key(name)) or ""
            if not value.startswith("button "):
                out.append((c.name, name))
    return out


def judge(check, window):
    """[(ok, what, detail)] for each thing the check looks for; ok is True, False or None (could not tell)."""
    out = []
    for what, test in check.looks:
        ok, detail = test(window)
        out.append((ok, what, detail))
    return out


def mark(ok):
    return {True: "seen    ", False: "NOT SEEN", None: "unclear "}[ok]


def all_test_names(top=None):
    """Every #[test] function name under `top` (default: the app's crate, the one `smoke` runs), to keep the
    lists here true: a test named here must be one that `smoke` runs."""
    names = set()
    for root, dirs, files in os.walk(top or os.path.join(RUST_DIR, "mpfree-app")):
        dirs[:] = [d for d in dirs if d != "target" and not d.startswith(".")]
        for f in files:
            if not f.endswith(".rs"):
                continue
            with open(os.path.join(root, f), encoding="utf-8", errors="replace") as fh:
                text = fh.read()
            names.update(re.findall(r"#\[test\]\s*(?:#\[[^\]]*\]\s*)*fn\s+([a-z0-9_]+)", text))
    return names


def cmd_list():
    print("Device checks (MPFREE_APP_CHECKS):")
    for c in CHECKS:
        print(f"  {c.name:13} {c.kind:5}  {c.title}")
        print(f"  {'':13}        for: {c.why}")
    print("\nChecked by the app's tests instead of on the MPC (app_checks.py smoke):")
    for title, tests in AREAS:
        print(f"  {title} ({len(tests)} tests)")
    return 0


def cmd_show(text):
    for c in chosen(text):
        print(f"{c.name} ({c.kind}): {c.title}")
        print(f"  for: {c.why}")
        for line in c.steps:
            print(f"  {line}")
        for what, _ in c.looks:
            print(f"  the log must show: {what}")
        if c.ask:
            print(f"  question: {c.ask}")
        print(f"  its software side: {', '.join(c.smoke)}")
        print()
    return 0


TEST_LINE = re.compile(r"^test (\S+) \.\.\. (ok|FAILED|ignored)")
FAILURE_HEAD = re.compile(r"^---- (\S+) stdout ----$")


def failure_lines(output, most=6):
    """{test name: the first lines of what it printed when it failed (the panic and its values)}."""
    out, name = {}, None
    for line in output.splitlines():
        m = FAILURE_HEAD.match(line)
        if m:
            name = m.group(1).rsplit("::", 1)[-1]
            out[name] = []
        elif name is not None:
            if line.startswith("stack backtrace:") or line in ("failures:", "successes:"):
                name = None
            elif line.strip() and len(out[name]) < most:
                out[name].append(line.strip())
    return out


def cmd_smoke():
    """Runs the app's whole test suite on the workstation and reports it by check and by area."""
    print("The app's tests on this workstation (read-only: it builds into rust/target)")
    try:
        head = subprocess.run(["git", "-C", REPO_ROOT, "rev-parse", "HEAD"], capture_output=True, text=True).stdout
    except OSError:
        head = ""
    print(f"commit: {head.strip() or '(not a git checkout)'}")
    # Release: it builds for longer but runs the smoke workflows several times faster (about 6 minutes in all).
    # --no-fail-fast: one failing test program doesn't keep the others (tests/*.rs) from running.
    cmd = ["cargo", "test", "-p", "mpfree-app", "--release", "--locked", "--no-fail-fast"]
    print(f"running: (cd rust && {' '.join(cmd)})", flush=True)
    if not shutil.which("cargo"):
        print("RESULT: the tests did not run: cargo is not on PATH (the Rust toolchain that builds the stick's programs)")
        return 1
    env = dict(os.environ, CARGO_INCREMENTAL="0")
    proc = subprocess.run(cmd, cwd=RUST_DIR, env=env, capture_output=True, text=True)
    results = {}
    for line in (proc.stdout + "\n" + proc.stderr).splitlines():
        m = TEST_LINE.match(line)
        if m:
            results[m.group(1).rsplit("::", 1)[-1]] = m.group(2)
    if not results:
        print((proc.stdout + proc.stderr)[-4000:])
        print("RESULT: the tests did not run (see above)")
        return 1
    failed = sorted(n for n, r in results.items() if r == "FAILED")
    print(f"\n{len(results)} tests ran: {sum(r == 'ok' for r in results.values())} passed, "
          f"{len(failed)} failed, {sum(r == 'ignored' for r in results.values())} ignored")

    def report(title, tests):
        got = [results.get(t, "did not run") for t in tests]
        ok = sum(r == "ok" for r in got)
        print(f"  {'ok  ' if ok == len(tests) else 'FAIL'} {title}: {ok} of {len(tests)} passed")
        for t, r in zip(tests, got):
            if r != "ok":
                print(f"         {t}: {r}")
        return ok == len(tests)

    print("\nThe software side of each device check:")
    good = all([report(c.name, c.smoke) for c in CHECKS])
    print("\nWhat the device rounds used to ask by hand:")
    good = all([report(title, tests) for title, tests in AREAS]) and good
    if failed:
        print("\nFailed:")
        why = failure_lines(proc.stdout + "\n" + proc.stderr)
        for n in failed:
            print(f"  {n}")
            for line in why.get(n, []):
                print(f"      {line}")
    print("\nRESULT: " + ("every test passed." if proc.returncode == 0 and good else "some tests failed (above)."))
    return 0 if proc.returncode == 0 and good else 1


def main(argv):
    if not argv or argv[0] not in ("list", "show", "smoke"):
        print(__doc__.split("\n\n")[-2])
        return 2
    try:
        if argv[0] == "list":
            return cmd_list()
        if argv[0] == "show":
            return cmd_show(argv[1] if len(argv) > 1 else "")
        return cmd_smoke()
    except Unknown as err:
        print(f"STOP: {err}")
        return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
