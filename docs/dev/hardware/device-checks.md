# Device checks: targeted rounds on the MPC

**As of 2026-11-15** · **Asked for by** the owner, 15 Nov 22:35Z: a device round should ask only about what it tests, automate as much as it can with the UI smoke tests, and use a modular runbook format for targeted tests. · **Code:** [`tools/mpc-one/app_checks.py`](../../../tools/mpc-one/app_checks.py), used by [`host_app_mpc.py`](../../../tools/mpc-one/host_app_mpc.py) (`collect-over-ssh.sh --app`).

Until this change the app step walked the same 19 numbered phases every round. Each phase asked its own question, and the start-up asked about the startup screen every time. A round picked phases by number, so a round about one feature still showed "Phase 1 of 19" and its text, and most phases asked a person to tap through things the app's own tests already check. Now a round names its **checks**, and nothing else is asked.

## The rule

- **A round tests only what changed.** Each check in a round's runbook says why it's in this round: a pull request, a new system, a fault from an earlier round.
- **What the app does on its own is checked by its tests, not by hand.** The UI smoke library (`rust/mpfree-app/src/smoke/`) drives the screen and the engines together, as a finger, the buttons and the wheel would. With the unit tests beside the code, it checks pages, drags, undo, songs, mixer and effects on every pull request, and under ARM in CI. A round runs the app's whole suite once on the workstation (`app_checks.py smoke`), so its zip records the result for the commit it tests.
- **On the MPC, only what the MPC alone can show:** that its buttons, pads, wheel, Q-Links, touchscreen and MIDI IN reach the app; lights and sound; a drag on the real panel; the load on its own cores; what the stick keeps from one boot to the next.
- **The tool reads the log, not the person.** After each check's steps it reads the app's log for that time and says, line by line, what it saw. A check asks **at most one question**, and only about something the log can't show, such as a light, a sound, or a drag following a finger.
- **No question without a check.** The start-up is reported from the log and the services' state. With no checks named, the app step asks nothing.

## A round's shape

1. **Builds** on the workstation, with the **smoke block** (`app_checks.py smoke`, about 6 minutes).
2. **The boot blocks** for the route the round uses (for the soft-boot stick, [how MPFree boots from the stick](soft-reboot-boot.md)).
3. **The app step** with this round's checks: `MPFREE_APP_STEPS=app MPFREE_APP_CHECKS=name,name,…`, in the order they run.
4. **Other tools' blocks** the round needs: the benchmark, the GPU's page flips, FXProbe.
5. **Back to Akai's system**, the stick logs and the zip.

A round's runbook has a table of its checks: the check, why it's in this round, what you do, and about how long it takes. The [3.10.1 device round](firmware-3.10.1-qualification.md#this-rounds-checks) is the first.

## The checks

`python3 tools/mpc-one/app_checks.py list` prints them, and `show NAME` prints one check's steps, what the log must show, its question and its tests. A check is **hands** (a few steps on the MPC) or **auto** (no steps and no question).

| Check | Kind | What only the MPC shows | The log must show |
|---|---|---|---|
| `transport` | hands | REC, OVERDUB, UNDO and PLAY on a fresh start (#387); their lights | buttons 73, 82, 80, 67 and 81, and the looper's Record, overdub on and off, and Undo |
| `basics` | hands | Pads, PAD BANK, FULL LEVEL, the wheel, the Q-Links (also on the Mix tab and the chain editor) and touch, on a new system or boot route | soft and hard hits, a bank change, Full Level on and off, wheel steps, Q-Link messages, touch interrupts rising |
| `load` | hands | Supersaw and the Grains kit with 8 pads: the worst case for the cores | no underrun between the status lines around it; then the cores, clock and threads |
| `track-mute` | hands | The pad colours as track mutes, and PAD BANK past 16 tracks (#332) | tracks muted and playing, another bank of 16 |
| `wheel` | hands | The outline and the push on the real wheel (#339), and moving a Song block by wheel (#355) | wheel steps, a block moved |
| `touch` | hands | Touch lab, Calibrate touch, two fingers, and a real drag past the 12 px lift | the Touch lab's start and end, Calibrate's result, a clip moved and a copy dropped |
| `note-repeat` | hands | NOTE REPEAT, Latch and the arp in real time | button 11, an arp mode, Latch on and off |
| `looper` | hands | The wave drawing as you play, and STOP stopping the loop at once | the loop armed, STOP |
| `console` | hands | The console chord, TAP TEMPO and STOP on the real panel | a tapped tempo, STOP's silence |
| `midi-in` | hands | A keyboard on the MPC's MIDI IN, and its port in the I/O view | `MIDI in:` lines |
| `map-controls` | hands | Buttons the app has no number for yet | `map:` lines and the summary |
| `sample-warp` | hands | The Warp crash retest | no PANIC and no restart |
| `kept` | auto | Prefs saved in one boot come back in the next (Sound cores, Loop memory) | the start's `saved settings:` line and the running Sound cores agree with `settings/app.conf` |
| `hour` | auto | An hour of play with no underrun (#273) | more than 1,240,312 periods, 0 underruns, no restart |

The areas the device rounds used to tap through, and that now only the tests check, are listed in `app_checks.py` (`AREAS`): Arrange, Play's views and throws, Mix, the Looper tab, the Pads and Sample tabs, the Synth, Grains and Prefs, and the sweeps over every control and the demo songs. `smoke` reports them by area.

## What a check looks like in the terminal

```
  Check 1 of 3: REC, OVERDUB, UNDO and PLAY on a fresh start (transport)
    for: #387: these buttons are known from the start, with no Map controls
    1. Press REC once: REC and PLAY blink.
    …
  Ready? Press Enter to start (s skips this step, q stops here):
  Go ahead. Press Enter when you are done (q stops here):
    the app's log: 10 lines (all in the report)
    …
    what the log showed:
      seen      REC (73) armed the loop (button 73 x1)
      NOT SEEN  OVERDUB (80) turned overdub on (button 80 never reached the app)
      …
  Did the lights do what each step says? If not, which step? Type a short answer and press Enter:
```

At the end the app step lists each check with how many of its lines were seen and your answer, then the session summary from the log.

- **seen:** the log has it.
- **NOT SEEN:** the log doesn't. The detail says what is missing, for example a button that never reached the app, or a press with nothing after it. Write down what happened on the MPC.
- **unclear:** the tool couldn't tell, for example no status line yet, or the touch count couldn't be read. An `hour` check that is short of the hour says `NOT YET` and how many minutes are left; run it again then.

Some steps use buttons that only Map controls gives a number, such as PAD BANK, TRACK MUTE, TRACK MIX, SAMPLER and ERASE. Before the first check the tool reads `settings/map.conf` on the stick's partition 4 (read-only) and names any that a named check uses and Map controls hasn't found yet. Then it runs `map-controls` first, unless the round named it already; `s` skips it, and the steps that use those buttons then show `NOT SEEN`. Partition 4 keeps what Map controls finds, so the next round doesn't ask again.

## Adding a check

1. Add a `Check` to `CHECKS` in `app_checks.py`: a short name, a title, why it exists (the PR or fault), the kind, a few numbered steps that each say what to expect, what the log must show, at most one question, the tests that check its software side, and the buttons it needs from Map controls. Keep the file ASCII.
2. **Prefer the log to a question.** If a step's result isn't in the app's log, add the log line in the app, with a test, rather than asking. The judges in `app_checks.py` (`seen`, `pressed`, `pad_hits`, `no_underruns_while`, …) cover most cases.
3. **Name the tests.** `test_app_checks.py` fails if a test named in a check, or in `AREAS`, isn't in the Rust sources, so a renamed test can't leave a silent gap.
4. If a check's steps end the boot (a power-off or a restart), the tool can't see the end. Put the part before it in the check, and the rest in a runbook block that the stick logs judge.
5. When the app gains something that only tests need to check, add its tests to an area in `AREAS`, not a check.
