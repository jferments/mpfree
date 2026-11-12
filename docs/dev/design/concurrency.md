# Concurrency: using every core

Jay, 2026-11-11: using all of the MPC One's cores wherever it pays is a top
priority, across the whole app, and the audio graph is split into parts
(render partitions) that run on different cores. The full audit is the
"Concurrency audit" tab of the performance plan
(https://claude.ai/artifact/Su4KF6BVbzkm9d5NfnWQMT). This page is the short
version for the code.

## Where things run today

The MPC One has four Cortex-A17 cores. The test build's kernel adds
`isolcpus=2-3`, so Linux places a thread on cores 2 or 3 only if the thread
asks for them. By default the pad player asks for no core anywhere, so the
inference is that all its threads, the audio thread included, share cores 0
and 1 while cores 2 and 3 sit idle. Round 8 measures this with a read-only
check (step 4 of the round 8 profiling asks). `--sound-cores` (below) puts
the sound on cores 3 and 2 instead, for the device rounds to compare.

| Thread | What it does |
| --- | --- |
| `mpfree-app` (main) | The audio loop, SCHED_FIFO 70: the screen's actions, controller MIDI, lights, the transport and arp, the sampler, the synth, the looper, the mixer, the ALSA write |
| `mpfree-screen` | Touch, the raster, the framebuffer |
| `mpfree-store` | Saved settings and patterns |
| `mpfree-journal` | The console's log |
| `mpfree-stretch`, the Sample tab's worker | Time-stretch jobs |

## The plan, in order

1. **Pin the audio thread to core 3.** No parallel code needed. Built:
   `--sound-cores core3` (below), off by default until a device round has
   measured it.
2. **A control thread on core 1** (Hello thread): MIDI in, the screen's
   actions, lights and the session ping leave the timed period; the audio
   thread gets a ready list of timed events through a lock-free queue.
3. **A second audio core** with lanes (`rust/mpfree-lanes`, below): half the
   sampler's voices, the synth, mixer stages, then pipelined returns. Built
   for the sampler and the mixer: `--sound-cores two` (below), off by default
   until a device round has measured it.
4. **Background work** on cores 0 and 1: loading at boot in parallel, looper
   bounce and Sample tab analysis on workers.

| Core | Runs |
| --- | --- |
| 0 | The screen and touch, the store, the journal, loading |
| 1 | The control thread, background jobs, offload sockets later |
| 2 | Audio lane B, SCHED_FIFO 69 |
| 3 | The audio thread (lane A), SCHED_FIFO 70 |

## Lanes (`rust/mpfree-lanes`)

A `Lane<J>` runs a `Job` (anything with `run(&mut self, block: u64)`) on a
worker thread, one block per period:

A sketch (`part`, `take_events`, `mix_into` and `render_my_part` stand for
the caller's own code):

```rust
let mut lane = Lane::new(part, Config { core: Some(2), fifo: Some(69), ..Config::default() });
// every period:
lane.job().map(|j| j.take_events(&events)); // the job is ours between periods
let posted = lane.post(block).is_ok();     // the worker renders its part...
render_my_part(&mut l, &mut r);            // ...while this thread renders its own
match lane.join(deadline) {
    Ok(j) if posted => j.mix_into(&mut l, &mut r),
    _ => {} // late or busy: this part is silent this period (or reuse the last block)
}
```

- **The job moves, nothing is shared.** The job owns its buffers. Between
  `post` and the worker's "done" it belongs to the worker; after `join` it
  is back. One atomic word says who holds it. No lock, no copy, no
  allocation per period (a test proves it with the counting allocator).
- **Deadlines, like the plugin sandbox.** `join` never waits past its
  deadline. Late, it says `Late`; the job comes back by itself, and until it
  does `post` says `Busy`. The caller picks the fallback. The plugin sandbox
  and the remote offload server ([Offload](offload.md)) keep the same
  contract.
- **Waiting.** The joiner spins, then sleeps on the word (a futex on Linux)
  until the deadline; by default it spins all the way, since the audio
  thread has nothing else to do. The worker sleeps between blocks
  (`worker_spin` zero, a wake-up each period) or spins for a while after
  each block (the lowest handoff, a busy core). Each side wakes the other
  only if it is asleep.
- **Never fails.** If the thread cannot start, or `Config::threaded` is off,
  the job runs on the calling thread inside `post`, with the same calls and
  the same output. So the audio loop is written once and a setting compares
  one core against two.
- **Pinning and priority** are asked for by the worker when it starts;
  `Lane::setup` says what took. Neither failing stops the lane. They change
  only the program's own thread and end with it.
- **Linux first.** Other systems poll with short sleeps instead of a futex,
  and report that pinning and SCHED_FIFO are not supported yet.

## The mixer's inserts on a second core

`Mixer::set_lane` (mpfree-fx, see [fx.md](fx.md), "The mixer") is the first
user in the audio graph: pad channels' and tracks' inserts are split between
the audio thread and a lane, stage by stage, with the same output bit for
bit. Strips with sidechain keys and frozen strips stay on the audio thread;
returns stay too, since they need every send first. The app turns it on with
`--sound-cores two` (core 2, SCHED_FIFO 69, a wait of a period; below).

## In the app: Prefs "Sound cores" and `--sound-cores`

Prefs has a "Sound cores" row (Shared, Core 3, Cores 3 and 2), kept in the
"app" settings on the stick's data partition as `sound-cores =
shared|core3|two` and used from the next start; while it differs from what
runs, the row says "(next start)". `mpfree play` and `mpfree app` also take
`--sound-cores shared|core3|two` for one run, which is not kept
([`cores.rs`](../../../rust/mpfree-app/src/cores.rs)). The default is
`shared`, which is how the app always ran, so nothing changes until a device
round has compared the three and Jay picks the default. The device round
runs Shared, sets Cores 3 and 2 in Prefs, and runs again after the second
boot it already makes to check saved settings.

| Mode | The audio thread | Core 2 |
| --- | --- | --- |
| `shared` | SCHED_FIFO 70, wherever the kernel puts it (cores 0 and 1 on the test stick) | idle |
| `core3` | pinned to core 3, SCHED_FIFO 70 | idle |
| `two` | pinned to core 3, SCHED_FIFO 70 | two lanes at SCHED_FIFO 69: the sampler's second voice pool (`Engine::split`, half the voices) and about half the pad channels' and tracks' inserts (`Mixer::set_lane`, a wait of one period) |

- **When.** The audio thread pins itself and splits the sampler just before
  the sound card opens, and the mixer's lane starts right after the mixer is
  made, all before the first period. Nothing changes while the sound plays.
  An audio buffer change in Prefs reopens the sound card on the same thread,
  which keeps its core.
- **One core for both lanes.** Each lane has its own worker thread (the
  sampler and the mixer each own theirs), both on core 2. They never run at
  once: within a period the sampler's lane is joined before the mixer posts.
  A late sampler lane can delay the mixer's, and both are counted.
- **The sampler's wait.** The audio loop still calls `render_routed`, so each
  part waits for the lane no longer than that part of the period lasts.
- **Late is a fallback, and never silent** (Jay, 12 Nov): a late sampler
  part leaves half the voices out of that part of the period; a late mixer
  stage leaves those strips silent for the block. Both counts are on the Dev
  tab's CPU by engine page (red once one is late), in the 30 s `status:`
  line (`sound cores two: drum voices N parts on core 2, N late; mixer N
  stages on core 2, N late`), and in a log line within a second of a late
  one. A late part is a bug or an over-budget design to fix, not a setting
  to tune away.
- **What took.** The start lines (`sound cores: ...`) and the Dev page say
  whether the pin and each lane's core and priority took. A refused pin or
  priority is reported in red or amber and the sound plays on.
- **Safety.** Only the program's own threads change: their allowed cores
  (`sched_setaffinity`) and priority (`sched_setscheduler`), in RAM. The
  only thing written is the Prefs row's line in the app's settings on the
  stick's data partition (never the MPC's own storage). Setting the row
  back to Shared undoes it from the next start; on the workstation,
  removing that line or `write-test-stick.py --fresh-data` does too.
- **Tests.** `cores.rs` checks the modes, the rows and the late lines;
  `rt_safety.rs` runs the sound path for 35 s on two cores (unpinned) with
  no allocation on the audio thread.

The device round that compares the modes is the app thread's live test
runbook. Its app check picks up the `sound cores: ` and `status: sound
cores ` lines.

`Mixer::set_pipelined_returns` is the returns' turn, as a prototype for Jay's
call: the returns' inserts run on a lane one block behind, during the next
block's pad channels, tracks and submixes, so the reverb leaves the audio
core at the cost of a block of timing on every return. Its output is the
reference a block late, bit for bit. It has its own lane; with both on, put
the two lanes on different cores or expect them to queue.

## Measuring it: `mpfree-bench --lanes`

```text
cargo run --release -p mpfree-bench -- --lanes [--lane-cores A,B] [--quick]
```

Runs only the lane cases: the handoff alone (an empty job, the worker
sleeping or spinning), the sampler case as one engine, as two engines of 12
voices on one core, and as two engines on two cores, the bench song's mixer
with every insert on one core, with its track inserts split over two, and with its returns a block late on a second core (Full and Eco reverb),
and the sampler with Poly Init's 8-voice chords on one core and on two. On the MPC,
`--lane-cores 3,2` pins the bench's own thread to core 3 and the workers to
core 2. A test checks that the two-core cases make exactly the same sound as
the one-core ones.

On a shared desktop VM (2026-11-11, noisy) the handoff cost about 1 to 17 µs
on average with a sleeping worker and under 1 µs with a spinning one. The
sampler's mean block went from about 90 µs on one engine to 66 to 79 µs split
over two cores with a sleeping worker, and 52 to 70 µs with a spinning one. On the MPC the
engines are about 6.6 times slower and the handoff is unmeasured, so the
real gain is for the device run to say.
