# MIDI clock (`rust/mpfree-midi-clock`)

MIDI clock in and out for MPFree, as bytes. It opens no port itself: the app's MIDI out sends its clock, and the app's MIDI in hands the followed port's bytes to the time engine (see [MIDI I/O](midi-io.md), M4). Sending on the MPC needs the owner's go. Like the transport, it never allocates while running, so it can sit in the audio thread.

Ticks are the transport's: 960 per quarter note. A clock pulse (`F8`, 24 per quarter note) is 40 ticks, and a Song Position step (a sixteenth) is 240.

## Out: `ClockOut`

Each period, in time order, the time engine calls:

- `started(offset, tick, out)` on the transport's Started mark;
- `stopped(offset, out)` on Stopped;
- `located(offset, tick, out)` on a locate or a loop wrap;
- `pulse(tick, offset, out)` for each line of `Span::lines(PULSE_TICKS)`.

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
| Song Position, both stopped | either | on | `Locate` there |
| Playing, the transport playing | either | on | keeps the leader's position; more than a sixteenth away, it jumps there (counted) |
| Pulses | any | off, or either stopped | keeps its own 40-tick grid on the leader's pulses, however many pulses apart they started |

The tempo is a phase-locked loop. At every pulse the follower measures the phase error and averages it (weight 0.25). The loop's tempo for the leader starts from the leader's measured tempo (the pulses' average over 8 beats, or over the last beat until then, from the third pulse on) and moves with the error (`KI` 0.05 per pulse for an error of a beat). The transport plays it nudged by the error (`KP` 2: 10 ticks off plays 2 % faster or slower, at most 3 %). The tempo goes to the transport at every pulse, and takes effect on its next sixteenth line. A change of more than 3 % in the measured tempo starts the loop again from it. The gains came from a simulation and are checked by the tests.

A clock that stops arriving for 0.5 s lets go of the tempo. The transport plays on, and the app's tempo applies again; it is the followed one if the app kept it. If the leader was playing, the loss is counted.

The status line gets a "following a clock" part once a count isn't 0: jumps to the leader's position, clocks lost while playing, and commands the transport's queue refused.

**Timing.** Starting takes the pulse's own tick, so the first note plays, a period late at most, and the loop closes that. Bytes that are read once a period seem to come at the period's start, up to a period after they did. Followed that way, the transport plays about half a period behind the leader (1.5 ms at 128 frames), within a period. Stamping the bytes half a period earlier would take that off.

**Measured** (the tests, 44.1 kHz):

| Case | Result |
|---|---|
| Steady 128 BPM, clock sent before Start, exact stamps, periods of 16 to 1024 frames | within a tick of the leader after four beats |
| No clock before Start (100 BPM, the transport at 120) | within a tick after six beats |
| Bytes read once a period, 64 to 256 frames | within a period; 128 frames: about 3 ticks behind |
| The leader's tempo steps by +1.5 %, +10 % and -25 % | at most about 7, 50 and 170 ticks off, with no jump; within a tick again after 4, 5 and 8 beats |

**Not yet.** The app's wiring (the MIDI thread's M4). Measuring it on the MPC with a real leader. A leader that changes tempo in big steps while playing is off for a few beats; a quicker change detector would need to know the period's jitter.

## Tests

`cargo test -p mpfree-midi-clock`. They cover Start and pulse timing at 120 BPM, Continue from the middle of a bar, a count-in, a loop wrap, Stop and a locate while stopped, a full buffer, reading the tempo with about 1 ms of jitter, Start, Continue, Stop and Song Position in, and a round trip from `ClockOut` back through `ClockIn` at 140 BPM.
