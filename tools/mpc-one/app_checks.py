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


CHECKS = (
    Check(
        "transport", "REC, OVERDUB, UNDO and PLAY on a fresh start",
        "#387: these buttons are known from the start, with no Map controls", "hands",
        ["Stay on the Play page, All view, where MPFree starts. Turn the volume down a little.",
         "1. Press REC once: REC and PLAY blink.",
         "2. Press PLAY: PLAY stays lit, REC keeps blinking while the first pass records. Play a few pads.",
         "3. Wait about 4 seconds (two bars): REC goes dark and the loop plays back.",
         "4. Press OVERDUB: it blinks. Play a few more pads. 5. Press OVERDUB again: it goes dark.",
         "6. Press UNDO: the second layer goes. 7. Press STOP: PLAY goes dark.",
         "8. SHIFT + SAMPLER opens the Looper tab. Press REC: REC blinks with PLAY. Press STOP twice."],
        [("REC (73) armed the loop", pressed(REC, r"^looper: Record")),
         ("PLAY (82) reached the app", pressed(PLAY)),
         ("OVERDUB (80) turned overdub on", pressed(OVERDUB, r"^looper: overdub on")),
         ("OVERDUB again turned it off", seen(r"^looper: overdub off")),
         ("UNDO (67) took the layer back", pressed(UNDO, r"^looper: Undo")),
         ("STOP (81) reached the app", pressed(STOP))],
        "Did the lights do what each step says? If not, which step?",
        ["rec_play_and_overdub_record_the_loop_and_light_on_a_fresh_mpc",
         "rec_on_the_looper_tab_arms_the_loop_on_a_fresh_mpc",
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
        "looper", "The Looper tab records and STOP stops it at once",
        "live test 3's loop that kept playing after STOP", "hands",
        ["1. SHIFT + SAMPLER opens the Looper tab. Press PLAY, then REC: 2 bars from the next bar line.",
         "   Play a beat on the pads: the wave draws as you play.",
         "2. When it loops, press STOP: the loop and its playhead stop at once. PLAY starts it again. STOP."],
        [("the loop recorded", seen(r"^looper: Record: armed")),
         ("STOP reached the app", pressed(STOP))],
        "Did the wave draw as you played, and did STOP stop the loop at once?",
        ["the_loop_strips_buttons_carry_their_symbols_and_play_and_stop_the_transport",
         "the_first_pass_shows_as_it_records",
         "presses_and_changes_log_and_the_wave_is_built",
         "the_looper_tab_picks_loops_and_works_layers",
         "rec_records_two_bars_and_stop_stops_the_loop_at_once",
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
        [("notes came in", seen(r"^MIDI in: "))],
        "Did the keys play in time, at the right loudness, with nothing stuck?",
        ["notes_on_any_channel_play_and_are_named",
         "keys_play_notes_and_share_holds_with_pads",
         "unplugging_lets_go_of_held_keys",
         "the_io_view_sees_ports_lights_and_clock"],
        needs=['TRACK MIX / PAD MIX']),
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
         ("no refusal", not_seen(r"^(mixer|chain): (The chain is full|The mixer is busy"
                                 r"|the chain editor asked for an effect that has no maker)")),
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
        "eq", "The Mix tab's EQ: a dot follows a finger, UNDO and Listen",
        "#388: the parametric EQ (the EQ look on the Mix tab, the EQ page in the chain editor)", "hands",
        ["1. Press TRACK MIX, tap EQ at the top right of the Mix tab, and tap \"+ EQ\" on the Drums row.",
         "   Play pads, drag the second dot of the new curve up, down and sideways, and let go.",
         "2. Press UNDO: the curve goes back and the hint line says \"Undid Drums EQ Band 2\".",
         "3. Tap the Drums curve twice to open the EQ page, tap Listen, and play pads (you hear only",
         "   band 2's slice). Press MAIN to go back to Play."],
        [("UNDO (67) reached the app", pressed(UNDO)),
         ("UNDO took back the EQ drag", seen(r"^Undid .+ EQ Band \d$")),
         ("no underrun while it played", no_underruns_while())],
        "Did the sound follow the dot while you dragged it, and did Listen play only a thin slice of the sound?",
        ["the_eq_look_adds_picks_and_shapes_an_eq"],
        needs=['TRACK MIX / PAD MIX', 'MAIN'], wait_status=True),
    Check(
        "mpc-plugin", "An Open MPC plugin instrument plays on the MPC",
        "#385: MPC-format plugins in the plugin sandbox; CI runs them as ARM only under qemu", "hands",
        ["1. Open the Synth tab. On the Instrument row, step past the built-in engines to [SYN] Mono Voice.",
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
        "kept", "The settings partition 4 kept came back at this start",
        "Prefs saved in one boot and used in the next (Sound cores, Loop memory)", "auto",
        looks=[("the start's settings agree with settings/app.conf", kept_settings())],
        smoke=["prefs_sound_cores_is_kept_and_used_from_the_next_start",
               "loop_memory_is_kept_and_used_from_the_next_start",
               "prefs_and_buttons_come_back_as_they_were"]),
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
      "the_eq_look_adds_picks_and_shapes_an_eq"]),
    ("Looper tab: layers, tools, undo history, trim and drags",
     ["the_looper_tab_picks_loops_and_works_layers",
      "a_layer_dragged_onto_a_loop_moves_or_copies_and_another_length_says_why",
      "a_dragged_layer_moves_between_loops_and_undo_says_so",
      "the_undo_history_steps_back_and_forward",
      "the_trim_handles_keep_a_range_by_sixteenths",
      "resample_goes_into_an_empty_loop_and_picks_it",
      "the_last_layers_x_clears_and_a_loop_mutes_without_being_picked",
      "working_tools_act_and_faint_ones_say_why",
      "the_tools_the_undo_history_and_trim_work_while_the_loop_plays",
      "layers_move_between_loops_and_speakers_clear_and_x_work"]),
    ("Pads tab and Sample tab: Browse, Pad, Chop, Stops, scenes and FX",
     ["the_pads_tab_picks_copies_and_clears_pads_and_undo_takes_it_back",
      "the_pads_tab_follows_play_and_its_browser_takes_the_wheel_push_minus_plus_and_play",
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


def not_found(checks, map_text):
    """[(check, control)] for each button a check uses that settings/map.conf has no number for."""
    out = []
    for c in checks:
        for name in c.needs:
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


def cmd_smoke():
    """Runs the app's whole test suite on the workstation and reports it by check and by area."""
    print("The app's tests on this workstation (read-only: it builds into rust/target)")
    try:
        head = subprocess.run(["git", "-C", REPO_ROOT, "rev-parse", "HEAD"], capture_output=True, text=True).stdout
    except OSError:
        head = ""
    print(f"commit: {head.strip() or '(not a git checkout)'}")
    # Release: it builds for longer but runs the smoke workflows several times faster (about 6 minutes in all).
    cmd = ["cargo", "test", "-p", "mpfree-app", "--release", "--locked"]
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
        for n in failed:
            print(f"  {n}")
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
