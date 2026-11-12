# Transport: the time engine's clock

**Status:** first crate (roadmap step T1), 2026-11-11. The code is [`rust/mpfree-transport`](../../../rust/mpfree-transport): host-only and tested, not yet wired to the audio thread or the pads. Nothing here has run on the MPC.

The transport is the one clock that the sequencer, looper, arpeggiator, sampler and metronome follow. The full time engine plan (research, the clip model, the UI and the roadmap T1 to T16) is the design document "MPFree time engine: transport, sequencer and looper"; mockups and the skill tree are in the mpfree-claude notes (`sequencer/`). This page covers only what T1 builds.

## The rules

1. **One frame counter.** The transport counts every frame the audio thread processes since start. It never resets, and an xrun loses wall-clock time but not musical time. It is the same count as the sampler's rolling recorder, so "frame N" means the same thing everywhere.
2. **Ticks.** Song positions are ticks, 960 per quarter note (the same as `mpfree-arp`), as `i64`. They are negative during a count-in before bar 1. On screen they read bar.beat.tick from 1.1.000; a one-bar count-in reads 0.1 to 0.4.
3. **One rule places every tick.** While rolling, tick `t` falls on frame `anchor_frame + round((t - anchor_tick) * 60 * rate / (bpm * 960))`. The anchor moves only when something happens: start, a loop jump, a locate, a tempo change. Positions are never found by adding up periods, so there is no drift.
4. **Same output for any period size.** Commands carry the absolute frame they apply on. Loop ends, tempo segments, the end of a count-in and live tempo changes are boundaries computed from absolute values. On one frame, commands apply first and then a boundary that falls there. Cutting a session into 1-frame or 8192-frame periods gives the same tick on the same frame and the same markers in the same order. Only a late command (its frame already played) moves, to the next period's first frame, and it is counted.
5. **No allocation, no locks, no panics** in `command` or `process`, or in reading a block. Everything has a fixed size.

## What one period gives the engines

`Transport::process(frames, &mut block)` fills a reusable `Block`:

- **Spans** cover the period exactly once, in order. Each has one state (stopped, count-in, playing, recording), one tempo, the overdub flag, the swing, the meter and its anchor. `span.ticks()` is the whole ticks whose frames fall in the span; consecutive spans give consecutive ranges, so each tick plays exactly once until a jump. `span.lines(step)` gives every grid line (beats, bars, 1/16s) with its offset in the period. `span.offset_of(tick)` places a fractional tick, for example a swung one. `span.tick_at(frame)` gives the playhead with its fraction, and `Transport::position_exact()` the song position at the start of the next period with its fraction (for following another device's clock).
- **Markers** say what happened and on which frame: `Started`, `Stopped`, `Panic`, `Rewound`, `CountIn`, `RecordOn`, `RecordOff`, `Overdub`, `Located`, `Looped`, `Tempo`, and the settings changes. A `Tempo` marker comes whenever the tempo changes, whatever caused it (a segment, a live change, a jump into another segment).

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

Swing is carried in every span but applied by whoever schedules notes (a sequencer track, the arp), never by the clock, so a track can have its own swing. `Swing::apply` warps ticks so that notes between grid lines keep their order and shape.

The tempo map (up to 32 segments, steps for now; ramps come later as another segment kind) and the meter are replaced only while stopped.

## Limits

- Sample rates 22,050 to 192,000 Hz, which keeps every tick on a frame of its own. Tempo 20 to 400 BPM (the same as the ClickEngine). Periods of 1 to 8192 frames.
- Loops and tempo segments are at least a 1/16 long. 64 queued commands, 16 applied per period (the rest wait one period and are counted). 64 spans and 128 markers per block, with room to spare: the limits above allow under 40 spans and under 90 markers.
- `Stats` counts late, refused and deferred commands, and block overflow (which should stay 0).

## Tests

- `tests/period_size.rs`: a busy 16-second session (count-in, tempo segments inside a loop, live tempo changes, punch in and out, locates, stop and panic, the shortest loop, and commands on the exact frame of a loop jump and of a tempo segment) cut into periods of 1, 2, 7, 64, 127, 333, 1024, 4096 and 8192 frames and three random sequences. Every tick's frame, every marker and the joined spans must be identical. It also checks that each tick plays once between jumps. Putting back the classic bugs (re-anchoring every period, or crossing a boundary before a command on the same frame) makes it fail.
- `tests/no_alloc.rs`: a counting allocator around 3000 periods of a busy session at sizes from 1 to 4096 frames, including reading ticks, lines and markers: no allocation or free.
- `tests/click.rs`: at a constant tempo the transport's beats land on exactly the frames where `mpfree-core`'s ClickEngine (the metronome that ran on the MPC) clicks, at seven rate and tempo pairs.
- Unit tests for positions, the tempo map, swing, count-in, loops, live tempo changes, stop and panic, punch in and out, locate, bad values and late commands.

On the build host, one 128-frame period (process plus reading every tick and 1/16 line) takes about 100 ns, against a 2.9 ms period. It has not been measured on the MPC yet.

## Next

T2, the playback engine, is [playback.md](playback.md). Device work waits until the pad player's round 5 (PR #31) is merged.
