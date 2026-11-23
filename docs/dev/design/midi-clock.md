# MIDI clock (`rust/mpfree-midi-clock`)

MIDI clock in and out for MPFree, as bytes. It opens no port itself: the app's MIDI out sends its clock, and the app's MIDI in hands the followed port's bytes to the time engine (see [MIDI I/O](midi-io.md), M4). Sending on the MPC needs the owner's go. Like the transport, it never allocates while running, so it can sit in the audio thread.

Ticks are the transport's: 960 per quarter note. A clock pulse (`F8`, 24 per quarter note) is 40 ticks, and a Song Position step (a sixteenth) is 240.

## Out: `ClockOut`

Each period, in time order, the time engine calls:

- `started(offset, tick, out)` on the transport's Started mark;
- `stopped(offset, out)` on Stopped;
- `located(offset, tick, out)` on a locate or a loop wrap;
- `pulse(tick, offset, out)` for each line of `Span::lines(PULSE_TICKS)`.

`moved(offset, tick, playing, out)` is for a host that puts the song somewhere new on a frame without the transport's marks saying so, such as a song switch in a live show: it takes the position and whether the song plays from there, whatever played before. Playing from the top (tick 0, or a count-in), Stop if the clock ran, then Start; playing from further in, Song Position and Continue (after Stop if it ran); stopped, Stop if it ran and Song Position.

| Transport | MIDI out |
|---|---|
| Starts from tick 0 | Start (`FA`), then a pulse at tick 0 |
| Starts before tick 0 (a count-in) | Nothing until tick 0, then Start and the first pulse |
| Starts anywhere else | Song Position for the next sixteenth (`F2`), Continue (`FB`), pulses from that sixteenth |
| Plays | A pulse every 40 ticks, on the transport's own frames |
| Stops | Stop (`FC`); no pulses while stopped |
| Jumps while playing (locate, loop wrap) | Stop, Song Position, Continue at the jump's frame, so the other device follows it |
| Moves while stopped | Song Position |

Messages go into a fixed `Messages` buffer of 32 a period; any that do not fit are counted in `dropped`.

Not yet: sending pulses while stopped (some drum machines like that, to show the tempo before Start).

## In: `ClockIn`

Give it every incoming byte with the frame it arrived at (`byte` or `bytes`); other messages are skipped, and real-time bytes inside a Song Position are handled. Each byte that means something returns an `InEvent`: `Pulse { tick }`, `Started`, `Continued { tick }`, `Stopped` or `Located { tick }`. `state(now)` gives:

- `bpm`: averaged over the last beat of pulses (25 pulses), so USB timing jitter of about 1 ms moves it by under 0.5 BPM. It is None until two pulses came, and again after 0.5 s without one.
- `playing`, and `tick`: the position of the last pulse, from the top.
- `pulse_frame`: when the last pulse came.

How the transport follows another device's clock is the time engine's: see [Following](#following-the-time-engine).

## Following: the time engine

The time engine's audio half follows another device's clock (`rust/mpfree-app/src/time/follow.rs`). The app gives it three things, from the port the project's **Tempo from** names (`MusicIn::leader_clock` in `midi_in.rs`, see [M4: clock in](midi-io.md#m4-clock-in)):

- `TimeHost::follow(Option<Follow>)`: `Follow { start_stop, position }` from the project's MIDI setup, or None for MPFree's own tempo. It is cheap to call every period; only a change starts again from nothing known.
- `TimeHost::clock_in(frame, bytes)`: every byte read from the followed port, before `advance`, with `frame` = `TimeHost::next_frame()`. The time engine runs its own `ClockIn` on them and skips anything else. The options decide what follows, so the app doesn't filter.
- `TimeHost::followed_bpm()`: the leader's tempo while its clock arrives, for the app's tempo and the tempo chip. While it is Some, `follow_tempo` does nothing, and the screens' `TransportInfo` shows it.

What the transport does:

| The leader | Follow start/stop | Follow position | The transport |
|---|---|---|---|
| Start | on | on | at the first pulse after it, `Locate(0)` and `Play` |
| Start | on | off | `Play` from where the song stands |
| Continue | on | on | at the first pulse after it, `Locate` to the Song Position and `Play` |
| Continue | on | off | `Play` from where the song stands |
| Stop | on | either | `Stop` |
| Start or Continue, a take ready | on | either | as above, with `Record` on the same frame (a punch-in, no count-in) |
| Stopped, PLAY pressed | on | either | nothing: PLAY waits for the leader's Start |
| Anything | either | on | the transport's loop is off, held until position isn't followed |
| Song Position, both stopped | either | on | `Locate` there |
| Playing, the transport playing | either | on | keeps the leader's position; more than a sixteenth away, it jumps there (counted) |
| Pulses | any | off, or either stopped | keeps its own 40-tick grid on the leader's pulses, however many pulses apart they started |

The tempo is a phase-locked loop. At every pulse the follower measures the phase error and averages it (weight 0.25). The loop's tempo for the leader starts from the leader's measured tempo (the pulses' average over 8 beats, or over the last beat until then, from the third pulse on) and moves with the error (`KI` 0.05 per pulse for an error of a beat). The transport plays it nudged by the error (`KP` 2: 10 ticks off plays 2 % faster or slower, at most 3 %). The tempo goes to the transport at every pulse, and takes effect on its next sixteenth line. A change of more than 3 % in the measured tempo starts the loop again from it. The gains came from a simulation and are checked by the tests.

**Waiting for the leader** (sequencer S5, the review's "Following another sequencer"). Following start and stop, `Follower::waits` is true while the leader is stopped: before its Start and after its Stop. `TimeHost::act` passes that to the control half (`TimeControl::clock_waits`) before each action. PLAY and PLAY START from stopped make ready what they would start (`ready_to_play`, the Play page's pattern; PLAY START also moves to bar 1) and send the transport nothing, and the app's `time_action` says "Waiting for clock: PLAY starts when MIDI In starts" (the name from `MusicIn::leader_name`, given with `TimeHost::set_leader`, read with `waiting_for`). A transport `Record` from stopped is refused while it waits. The app tells the follower every period whether a take is ready (`TimeHost::take_waits`, from `RecArm::ready`, adding or replacing), and PLAY keeps the take ready while it waits (`rec_arm::Now::clock`). At the leader's start, on the pulse that plays, the follower sends `ToggleOverdub` if the transport's mode isn't the take's, then `Play` and `Record` on one frame: a punch-in with no count-in, since the leader counts. `TimeHost::clock_in` sees `Follower::took` and readies the clip the take records into as REC's punch-in does (`TimeControl::take_started`, logged "rec: the take started with the clock, new clip in slot 2 of track 1"), before the transport runs. Once the transport records, the app lets go of its ready.

**The leader owns the timeline.** While following position the transport's loop is off. `Follower::period` holds it (`SetLoop(None)`) when position is followed and gives it back when that stops (Tempo from set to MPFree, or Position off). A `SetLoop` from the control half meanwhile takes the held loop's place (`TimeAudio::command` asks `Follower::keep_loop`), so the Record tab's Loop row still sets the loop that comes back. The screens' `TransportInfo` then has the held loop as its `loop_range` with `follows_position` set, so they show it greyed ("1–4, off while following"), the Record tab steps it and an export puts it back; what the transport plays is `TransportInfo::playing_loop`, none while following (a cue's line, CAPTURE, Play's bar count). It adds one flag, so a period's message stays within 128 bytes. Launches and scenes keep MPFree's bar lines, and with the leader's position those are the leader's bars.

A clock that stops arriving for 0.5 s lets go of the tempo. The transport plays on, and the app's tempo applies again; it is the followed one if the app kept it. If the leader was playing, the loss is counted. It is also System status's yellow "The clock MPFree follows stopped" (`midi/clock-stopped`, [status.md](status.md)): `Follower::lost` holds from the loss until a pulse or a Stop comes, or following changes, and the audio loop's timing carries it to the screen's watcher.

The status line gets a "following a clock" part once a count isn't 0: jumps to the leader's position, clocks lost while playing, and commands the transport's queue refused.

**Timing.** Starting takes the pulse's own tick, so the first note plays, a period late at most, and the loop closes that. The app reads the bytes once a period, at its top, so they came during the period before, up to a period before it read them. Stamped with the period's start, the transport played about half a period behind the leader (1.5 ms at 128 frames). The app stamps them half a period earlier (`follow::read_stamp`, through `TimeHost::clock_read`), when they came on average, so the transport sits on the leader's beats, within half a period (18 Nov 2026).

**Measured** (the tests, 44.1 kHz):

| Case | Result |
|---|---|
| Steady 128 BPM, clock sent before Start, exact stamps, periods of 16 to 1024 frames | within a tick of the leader after four beats |
| No clock before Start (100 BPM, the transport at 120) | within a tick after six beats |
| Bytes read once a period, 64 to 256 frames, stamped half a period back | within half a period, centred (0.02 ticks behind on average; stamped at the period's start: 1.5 ticks behind at 64 frames, half a period) |
| The leader's tempo steps by +1.5 %, +10 % and -25 % | at most about 7, 50 and 170 ticks off, with no jump; within a tick again after 4, 5 and 8 beats |

**Not yet.** The app's wiring (the MIDI thread's M4). Measuring it on the MPC with a real leader. A leader that changes tempo in big steps while playing is off for a few beats; a quicker change detector would need to know the period's jitter.

## Tests

`cargo test -p mpfree-midi-clock`. They cover Start and pulse timing at 120 BPM, Continue from the middle of a bar, a count-in, a loop wrap, Stop and a locate while stopped, `moved` from the top, further in and stopped, a full buffer, reading the tempo with about 1 ms of jitter, Start, Continue, Stop and Song Position in, and a round trip from `ClockOut` back through `ClockIn` at 140 BPM.

Following (`cargo test -p mpfree-app --bin mpfree follow`): `time::follow::tests` measure the cases above, and `a_ready_take_records_from_the_leaders_first_pulse` and `following_position_holds_the_loop_off_and_gives_it_back` the waiting and the held loop. `rec_arm::tests::play_keeps_a_take_ready_while_it_waits_for_the_clock` and the smoke test `following_a_clock_play_waits_and_a_ready_take_records_from_its_start` (PLAY's line, REC and PLAY, the leader's Start recording, its Stop) go through the app; `mpfree-time-ui`'s `record_settings_step_within_their_range` shows and steps the held loop.
