# MIDI clock (`rust/mpfree-midi-clock`)

MIDI clock in and out for MPFree, as bytes. It opens no port. Which MIDI port the MPC sends and listens on, and whether it may, goes through the Hello MPC thread and needs the owner's go. Like the transport, it never allocates while running, so it can sit in the audio thread.

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

How the transport follows another device's clock (tempo, locate, play and stop) is the time engine's call.

## Tests

`cargo test -p mpfree-midi-clock`. They cover Start and pulse timing at 120 BPM, Continue from the middle of a bar, a count-in, a loop wrap, Stop and a locate while stopped, a full buffer, reading the tempo with about 1 ms of jitter, Start, Continue, Stop and Song Position in, and a round trip from `ClockOut` back through `ClockIn` at 140 BPM.
