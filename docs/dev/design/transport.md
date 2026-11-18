# Transport: the time engine's clock

**Status:** first crate (roadmap step T1), 2026-11-11; on main and in use, 2026-11-12. The code is [`rust/mpfree-transport`](../../../rust/mpfree-transport). The pad player runs it on its audio thread through the time engine's audio half (`TimeHost`, [time-threads.md](time-threads.md)), with the transport bar, PLAY, STOP, REC, OVERDUB and TAP TEMPO. Round 7 ran it on the MPC One. It follows another device's MIDI clock ([midi-clock.md](midi-clock.md#following-the-time-engine), #239), which has not run on the MPC yet. Tempo ramps in the tempo map, 2026-11-18 (sequencer roadmap): the engine and its tests; the song's tempo points and the Song view's tempo lane come next.

The transport is the one clock that the sequencer, looper, arpeggiator, sampler and metronome follow. The full time engine plan (research, the clip model, the UI and the roadmap T1 to T16) is the design document "MPFree time engine: transport, sequencer and looper"; mockups and the skill tree are in the mpfree-claude notes (`sequencer/`). This page covers only what T1 builds.

## The rules

1. **One frame counter.** The transport counts every frame the audio thread processes since start. It never resets, and an xrun loses wall-clock time but not musical time. It is the same count as the sampler's rolling recorder, so "frame N" means the same thing everywhere.
2. **Ticks.** Song positions are ticks, 960 per quarter note (the same as `mpfree-arp`), as `i64`. They are negative during a count-in before bar 1. On screen they read bar.beat.tick from 1.1.000; a one-bar count-in reads 0.1 to 0.4.
3. **One rule places every tick.** While rolling, tick `t` falls on frame `anchor_frame + round((t - anchor_tick) * 60 * rate / (bpm * 960))`. The anchor moves only when something happens: start, a loop jump, a locate, a tempo change (on a tempo ramp, every 1/16 line is one; see [Tempo ramps](#tempo-ramps)). Positions are never found by adding up periods, so there is no drift.
4. **Same output for any period size.** Commands carry the absolute frame they apply on. Loop ends, tempo segments, the end of a count-in and live tempo changes are boundaries computed from absolute values. On one frame, commands apply first and then a boundary that falls there. Cutting a session into 1-frame or 8192-frame periods gives the same tick on the same frame and the same markers in the same order. Only a late command (its frame already played) moves, to the next period's first frame, and it is counted.
5. **No allocation, no locks, no panics** in `command` or `process`, or in reading a block. Everything has a fixed size.

## What one period gives the engines

`Transport::process(frames, &mut block)` fills a reusable `Block`:

- **Spans** cover the period exactly once, in order. Each has one state (stopped, count-in, playing, recording), one tempo, the overdub flag, the swing, the meter and its anchor. `span.ticks()` is the whole ticks whose frames fall in the span; consecutive spans give consecutive ranges, so each tick plays exactly once until a jump. `span.lines(step)` gives every grid line (beats, bars, 1/16s) with its offset in the period. `span.offset_of(tick)` places a fractional tick, for example a swung one. `span.tick_at(frame)` gives the playhead with its fraction, and `Transport::position_exact()` the song position at the start of the next period with its fraction (for following another device's clock).
- **Markers** say what happened and on which frame: `Started`, `Stopped`, `Panic`, `Rewound`, `CountIn`, `RecordOn`, `RecordOff`, `Overdub`, `Located`, `Looped`, `Tempo`, and the settings changes. A `Tempo` marker comes whenever the tempo changes, whatever caused it (a segment, each step of a ramp, a live change, a jump into another segment).

An engine walks the spans in order and schedules its events at the offsets the span gives. The arp gets one `mpfree_arp::Clock` per span, built from the span's anchor and tempo, and `Engine::sync` runs before `process(span.len)` (agreed with the arp thread; the hook is in PR #42). The looper (T4) takes its phase from the span's tick, not from a free-running frame count, so it stays on the transport's loop.

A host whose bookkeeping runs later on another thread can rebuild a period's block from the spans the audio thread sends it: `Block::start_copy(start_frame, frames)`, then `Block::copy_span` for each span in order. The markers are not copied (the time engine's control half reads only spans; see [time-threads.md](time-threads.md)).

## Commands

| Command | Stopped | Rolling |
|---|---|---|
| `Play` | play from the position | nothing |
| `PlayStart` | play from bar 1 | jump to bar 1 |
| `Stop` | back to bar 1 and `Panic` (cut all sound) | stop where it is; sounds keep their tails |
| `StopAndRewind` | back to bar 1 | stop, then back to bar 1 |
| `Record` | count in (if set), then record | punch in; while recording, punch out |
| `ToggleOverdub` | toggles | toggles |
| `Locate(t)` | move the position | jump at once |
| `SetTempo(bpm)` | the whole song takes it | on the next tempo-grid line (a 1/16), or at the loop jump if that comes first |
| `SetSwing`, `SetLoop`, `SetCountIn` | set | set; a loop that ends before the playhead does not pull it back |
| `change_tempo_map(at, map)` | the song takes the map | as `SetTempo`: on the next tempo-grid line, or at a loop jump, locate or stop if one comes first |

Swing is carried in every span but applied by whoever schedules notes (a sequencer track, the arp), never by the clock, so a track can have its own swing. `Swing::apply` warps ticks so that notes between grid lines keep their order and shape.

The meter is replaced only while stopped. The tempo map (up to 32 segments, each a step or a ramp) is replaced with `set_tempo_map` while stopped, or queued with `Transport::change_tempo_map(at, map)` at any time, so song mode can change a tempo point while the song plays. The map is not a `Command` (it is about 520 bytes, and every queue slot would carry it): it waits in one of `MAX_STAGED_MAPS` (4) slots, and the queue carries the slot, in frame order with the commands. Rolling, it takes over the way a live tempo does, on the next tempo-grid line, and the clock enters the new map there with `play_bpm`, so inside a ramp it lands on the ramp's staircase and every later line keeps the map's spacing exactly. A later `SetTempo` or map replaces one still waiting for its line.

## Tempo ramps

`TempoMap::ramp(i)` makes segment `i` glide from its tempo to the next segment's, arriving on the next segment's start (the last segment has nothing to ramp to: `MapError::RampAtEnd`). The glide is linear in ticks, so halfway through the ramp the tempo is halfway; `bpm_at` gives the glide's tempo at a tick, for the screen.

The transport plays a ramp as a staircase, so every span keeps one tempo and every engine that reads spans follows the ramp with no change:

- **Steps.** The ramp changes tempo on each line of its own 1/16 grid, counted from its start (`RAMP_STEP`). The last step takes any remainder, so no step is shorter than a 1/16 and a period crosses no more boundaries than before. `next_change_after` gives the next line, and the transport crosses it as it crosses a segment.
- **Whole frames.** Each line sits on the whole frame nearest to where the true glide puts it, counted from the ramp's start: `round(k L / (b1 - b0) ln(bpm(t) / b0))` frames, with `k = 60 * rate / 960`. Between lines, frames follow ticks in a straight line. The transport starts a ramp on a whole frame and takes, at each line, the step's tempo that lands the next line exactly on its frame (`TempoMap::play_bpm`), so rounding never adds up along a ramp. Entered in the middle of a step (a locate, a loop jump, PLAY), the first step runs to the nearest whole frame and every later line keeps the map's spacing exactly.
- **One sum for everyone.** `TempoMap::frames_between` and `TempoMap::tick_after` follow the same staircase, so audio tracks (`mpfree_audiotracks::frames_from`, `tick_after`) and the screen agree with the transport to the frame, at any period size. A ramp whose two tempi are equal plays as that tempo (to half a frame when a 1/16 isn't whole frames).
- **MIDI files.** `TempoMap::changes` lists every segment start and every ramp step with the step's average tempo (the true glide's, at any rate). Export writes those as the MIDI file's tempo track, a tempo every 1/16 through a ramp.

How close the staircase is: a step's tempo is the glide's average over that 1/16, so each line is where the glide puts it (to the frame). In between, a tick strays from the true glide most in the middle of a step: at most 0.3 ms on 90 to 180 BPM over four bars, 0.5 ms on 100 to 140 over one bar, 1.8 ms on a tempo that doubles or halves in one bar (48 kHz). A ramp far steeper than music asks for (20 to 400 BPM in a bar) strays tens of milliseconds inside its first slow steps. Halving the step would cut these by four, at the cost of twice the spans in every period on a ramp.

## Limits

- Sample rates 22,050 to 192,000 Hz, which keeps every tick on a frame of its own. Tempo 20 to 400 BPM (the same as the ClickEngine). Periods of 1 to 8192 frames.
- Loops, tempo segments and ramp steps are at least a 1/16 long (the first step after a jump into a ramp can be shorter, as a segment that starts just after a locate can). 64 queued commands, 16 applied per period (the rest wait one period and are counted); `queue_room` says how many more fit, for a caller whose commands must all go in or none (the looper's start on a note). 64 spans and 128 markers per block, with room to spare: the limits above allow under 40 spans and under 90 markers.
- `Stats` counts late, refused and deferred commands, and block overflow (which should stay 0).

## Tests

- `tests/period_size.rs`: a busy 16-second session (count-in, tempo segments inside a loop, live tempo changes, punch in and out, locates, stop and panic, the shortest loop, and commands on the exact frame of a loop jump and of a tempo segment) cut into periods of 1, 2, 7, 64, 127, 333, 1024, 4096 and 8192 frames and three random sequences. Every tick's frame, every marker and the joined spans must be identical. It also checks that each tick plays once between jumps. A second session plays two ramps (90 to 160 BPM over four bars, then 160 to 70 over 1000 ticks, whose last step is 280 ticks) and locates into the middle of a step: one `Tempo` marker per step, rising then falling; every line on exactly the frame `frames_between` gives, counted from the ramp's start, and every other tick within a frame of it; after the locate, the next line on the nearest whole frame and every later line spaced exactly as the map says; and the same log at every period size. A third session plays at 120 BPM, queues that ramp map while playing (it takes over on the next 1/16, inside the first ramp) and later a flat 100 BPM map: the same checks, at every period size. Putting back the classic bugs (re-anchoring every period, or crossing a boundary before a command on the same frame) makes it fail.
- `tests/no_alloc.rs`: a counting allocator around 3000 periods of a busy session (new tempo maps queued among the commands) at sizes from 1 to 4096 frames, including reading ticks, lines and markers: no allocation or free.
- `tests/click.rs`: at a constant tempo the transport's beats land on exactly the frames where `mpfree-core`'s ClickEngine (the metronome that ran on the MPC) clicks, at seven rate and tempo pairs.
- Unit tests for positions, the tempo map and its ramps (the glide's tempo, the 1/16 lines and a ramp's last longer step, the whole ramp within half a frame of the true glide's time, every line on a whole frame, a ramp with one tempo, `tick_after` undoing `frames_between` both ways at four rates, `play_bpm` landing the next line from a line or from inside a step, `changes`), swing, count-in, loops, live tempo changes, a new map while stopped or rolling (on the next 1/16, at a loop jump, locate or stop, replaced by a later tempo, four waiting at most), stop and panic, punch in and out, locate, bad values and late commands.

On the build host, one 128-frame period (process plus reading every tick and 1/16 line) takes about 100 ns, against a 2.9 ms period. It has not been measured on the MPC yet.

## Next

- Tempo ramps in the song: song mode hands the transport `Arrangement::tempo_map` and changes it while playing when a tempo point changes (sequencer roadmap). The song's tempo points and the Song view's tempo lane are in; until the app wires them, nothing in the app plays a ramp.
- Following a clock: the MIDI thread wires the followed port's bytes to `TimeHost::clock_in`, then a device test with a real leader. A quicker answer to a leader whose tempo jumps is in [midi-clock.md](midi-clock.md#following-the-time-engine); the bytes are stamped half a period earlier (`TimeHost::clock_read`), so the transport sits on the leader's beats.
- Measuring the transport's cost on the MPC.

The playback engine (T2) is [playback.md](playback.md).
