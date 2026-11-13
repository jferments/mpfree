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
| `mpfree-app` (main) | The audio loop, SCHED_FIFO 70: the screen's actions, controller MIDI in, the transport and arp, the sampler, the synth, the looper, the mixer, the ALSA write |
| `mpfree-lights` | The controller's lights and the session ping, queued by the audio loop ([below](#the-control-side-and-helper-threads)) |
| `mpfree-report` | The report's lines (`/run/mpfree-app/report.txt`), queued by the audio loop |
| `mpfree-build` | Builds the chain editor's effects, and drops the ones edits take out |
| `midi-out`, `cv-out` | MIDI Out and CV out to other gear (`port_writer.rs`), SCHED_FIFO 60 on the MPC |
| `usb-midi-scan`, `sample-open`, `sample-load`, `preview-load`, `mpfree-print` | Helper threads: USB keyboards, loading sounds, the looper's buffers |
| `mpfree-screen` | Touch, the raster, the framebuffer. Ticks the pages every 33 ms and sleeps between ([profiling.md](profiling.md#the-screens-cost)) |
| `mpfree-touch-bell` | The touchscreen's doorbell: sleeps in read(2) on a second read-only handle on the panel and wakes `mpfree-screen` when it sends something |
| `mpfree-store` | Saved settings and patterns |
| `mpfree-journal` | The console's log |
| `mpfree-stretch`, the Sample tab's worker | Time-stretch jobs |

## The plan, in order

1. **Pin the audio thread to core 3.** No parallel code needed. Built:
   `--sound-cores core3` (below), off by default until a device round has
   measured it.
2. **The control side**: work that is not sound leaves the audio thread.
   Built in two parts (below), and measured first, as Jay chose ("measure,
   then move", 12 Nov). Part 1 (#261): the lights, the report's lines and
   effect building run on threads of their own, and helper threads keep off
   the sound's cores. Part 2 (#267): the loop times its own parts outside
   the engines and names any slow period
   ([profiling.md](profiling.md#dev-tools-pages)). Live test 2 (13 Nov)
   found no slow period: the worst part took 0.37 ms, 13 % of a period. So
   MIDI in, the screen's actions and the app logic stay on the audio thread
   for now. Any of them moves (through `TimeHost::split`) only when a device
   run's `control: slow period` lines name it.
3. **A second audio core** with lanes (`rust/mpfree-lanes`, below): half the
   sampler's voices, the synth, mixer stages, then pipelined returns. Built
   for the sampler, the mixer and the reverb returns: `--sound-cores two`
   (below), off by default until a device round has measured it.
4. **Background work** on cores 0 and 1: loading at boot in parallel, looper
   bounce and Sample tab analysis on workers.

| Core | Runs |
| --- | --- |
| 0 | The screen and touch, the store, the journal; helper threads at normal priority |
| 1 | Helper threads at normal priority (lights, report, effect building, loading, MIDI and CV out at FIFO 60), background jobs, offload sockets later |
| 2 | Audio lane B, SCHED_FIFO 69; the reverb returns' lane at SCHED_FIFO 68 |
| 3 | The audio thread (lane A), SCHED_FIFO 70 |

## The control side and helper threads

The outside review of 12 Nov (performance plan, "Outside review" tab) found
work on the audio thread that can take longer than a period by itself,
outside every engine. [`control.rs`](../../../rust/mpfree-app/src/control.rs)
moves it to threads of its own, each fed by a bounded queue made before the
sound starts:

- **Lights** (`control::Lights`, thread `mpfree-lights`). The audio loop
  queues each controller message (pad colours, button lights, the session
  ping) and never waits. The thread writes it, waiting up to half a second
  for room in the MIDI port's buffer; before, the audio thread slept there
  1 ms at a time. A full queue drops the message and counts it (a log line
  at most once a second); a message the port refuses is logged and the
  sound plays on (a lost controller is still found by the port's poll). A
  watchdog says when the thread has written nothing for 2 s with messages
  waiting (`controller output: the lights' thread has written nothing ...`)
  and when it writes again: the session ping waits behind the lights, and
  the controller drops the session after about 10 s without one. Refused,
  dropped and stalled messages are counted on the Dev tab (CPU by engine,
  under the budgets). The end of the session says how many were written,
  refused, dropped and stalled.
- **The report's lines** (`control::Report`, thread `mpfree-report`). Each
  log line was a write to `/run/mpfree-app/report.txt` by the audio thread;
  now it is queued, and a buffered writer writes it. Lines a full queue
  drops are counted in the report when the session ends, and on the Dev
  tab. One cost: if the program aborts (release builds abort on a panic),
  the lines still queued or buffered are lost, and those are the last
  lines before the crash. The screen thread prints each line to the
  system journal (`journalctl`) as soon as it gets it, so read the end of
  a crashed run there rather than in report.txt.
- **Effects** (`control::Builder`, thread `mpfree-build`). Building an
  effect allocates all it will use (the Cloud reverb about 9 MB, every page
  locked under `mlockall`). The chain editor's Add now asks the builder,
  and `MixHost::poll_built` puts the effect at the end of its chain a few
  periods later (`inserts_building` counts it against the chain's room).
  The effects edits take out go back there to be freed. This also fixes a
  stall: the app never took retired effects back from the mixer, which
  stops applying edits once 32 are waiting, so the chain editor froze after
  32 removes. `poll_built` takes them back every period.

**Priority and cores of helper threads.** A Linux thread starts with the
policy and the cores of the thread that started it, so a thread started by
the audio thread after it went real-time used to run at SCHED_FIFO 70 on
the audio core (the USB MIDI scan, MIDI Out's writer, the loaders), where
it could hold the core in the middle of a period. Two fixes:

- `sys::realtime` asks for SCHED_FIFO with `SCHED_RESET_ON_FORK`, so every
  thread started from the audio thread starts at the normal policy. A lane
  asks for its own priority (`Config::fifo`, 69), so lanes are unchanged.
- Every helper thread calls `cores::background()` first
  (`mpfree_lanes::os::background`, which the sampler's `mpfree-stretch`
  worker calls too): the normal policy, and once the audio thread is pinned
  (`core3`, `two`, `three`), cores 0 and 1 only. The start lines say
  `sound cores: helper threads on cores 0 and 1, normal priority`, and the
  Dev tab's Sound cores page has a "Helper threads" row. In `shared` they
  stay where the kernel puts them. MIDI Out's and CV out's writers
  (`port_writer.rs`) then ask for SCHED_FIFO 60 on the MPC (below the lanes
  and the audio thread), so each message still leaves on its moment; if
  that is refused the log says so and they run at the normal priority.

Tests: control.rs (lights written in order without waiting; a port that
stays full is counted and logged; a full queue drops and counts; report
lines written and the file handed back; effects built and dropped on the
builder), mixing.rs (effects built off the audio thread land in their
chain; 40 adds and removes, past the old stall at 32), mpfree-lanes (a
helper thread goes to the background cores at the normal priority).

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
  `Lane::setup` says what took. They change only the program's own thread
  and end with it. Neither failing stops the lane: by default the worker
  runs where and how it can. With `Config::strict` it stops before it
  touches the job, and the lane runs its jobs on the calling thread instead
  (`Setup::threaded` says why). The app's lanes are strict: a worker that
  could not be pinned keeps the core set of the thread that made it, which
  is the audio thread's own core 3, and would fight the audio thread for it
  while the audio thread spins in `join`.
- **Linux first.** Other systems poll with short sleeps instead of a futex,
  and report that pinning and SCHED_FIFO are not supported yet.

## The mixer's inserts on a second core

`Mixer::set_lane` (mpfree-fx, see [fx.md](fx.md), "The mixer") is the first
user in the audio graph: pad channels' and tracks' inserts are split between
the audio thread and a lane, stage by stage, with the same output bit for
bit. Strips with sidechain keys and frozen strips stay on the audio thread;
returns stay too, since they need every send first. The app turns it on with
`--sound-cores two` (core 2, SCHED_FIFO 69; below). `Mixer::set_lane_deadline`
gives the next block one deadline for every join in it (each stage, and the
returns a block late); the app sets it from the sound card's queue, and
without one each join waits the lane's own wait from when it starts.

## In the app: Prefs "Sound cores" and `--sound-cores`

Prefs has a "Sound cores" row (Shared, Core 3, Cores 3 and 2, Cores 3, 2
and 1), kept in the "app" settings on the stick's data partition as
`sound-cores = shared|core3|two|three` and used from the next start; while
it differs from what runs, the row says "(next start)". `mpfree play` and
`mpfree app` also take `--sound-cores shared|core3|two|three` for one run,
which is not kept ([`cores.rs`](../../../rust/mpfree-app/src/cores.rs)).
The default is `shared`, which is how the app always ran, so nothing
changes until a device round has compared the modes and Jay picks the
default. The device round runs Shared, sets Cores 3 and 2 in Prefs, and
runs again after the second boot it already makes to check saved settings;
`three` is a test Jay asked for (12 Nov), run the same way.

| Mode | The audio thread | Core 2 | Core 1 |
| --- | --- | --- | --- |
| `shared` | SCHED_FIFO 70, wherever the kernel puts it (cores 0 and 1 on the test stick) | idle | shared with the rest |
| `core3` | pinned to core 3, SCHED_FIFO 70 | idle | the rest |
| `two` | pinned to core 3, SCHED_FIFO 70 | two lanes at SCHED_FIFO 69: the sampler's second voice pool (`Engine::split`, half the voices) and about half the pad channels' and tracks' inserts (`Mixer::set_lane`); and the reverb returns a block late at SCHED_FIFO 68 (`Mixer::set_pipelined_returns`) | the rest |
| `three` (a test) | pinned to core 3, SCHED_FIFO 70 | as `two` | the synth's lane at SCHED_FIFO 69 (`SynthSlot`), beside the rest |

Core 0 is never asked for: Linux on ARM sends hardware interrupts to core 0
unless something moves them (an assumption for the MPC until a read-only
look at `/proc/interrupts` on the unit), and the screen and the rest live
there. Jay (12 Nov): keep one core for interrupts.

- **`three`: the synth beside the drums.** The app's synth is a separate
  engine from the sampler, so it is the one part that can run at the same
  time as both halves of the drums. Each period the audio thread posts the
  synth to its lane on core 1, renders its half of the drums while the
  lane on core 2 renders the other half, then joins the synth and adds its
  output, bit for bit what the audio thread would have made. Core 1 is not
  kept free by the kernel (only 2 and 3 are), so the synth runs ahead of
  the ordinary threads there and shares the core with any interrupts sent
  to it; what that costs is what the test measures. A late synth is left
  out of that period (Jay, 12 Nov 20:51Z), like the other fallbacks: the
  lane finishes the period on its own while the sound card takes this one,
  and `SynthSlot::settle`, right after the write, takes the synth back and
  drops what it made. Only if it is still not back by the next period's
  deadline does the audio thread wait for it, because the rest of the loop
  plays the synth; that wait is counted too (`synth N periods on core 1, N
  late (left out), N waited for`). Before 12 Nov a late synth was waited
  for with no limit, so one slow synth period could run the card dry. With
  the synth on its lane, the CPU by engine page's synth row is only the
  audio thread's wait for it.

- **When.** The audio thread pins itself and splits the sampler just before
  the sound card opens, and the mixer's lane starts right after the mixer is
  made, all before the first period. Nothing changes while the sound plays.
  An audio buffer change in Prefs reopens the sound card on the same thread,
  which keeps its core.
- **One core for three lanes.** Each lane has its own worker thread (the
  sampler, the mixer's inserts and the reverb returns each own theirs), all
  on core 2. The sampler's lane never runs at the same time as the mixer's
  two: within a period it is joined before the mixer posts. The returns'
  lane is posted at the start of the mixer's block and joined at the
  returns, after the pad channels, tracks and submixes, so it overlaps the
  inserts lane's stages. It runs one priority lower (SCHED_FIFO 68): the
  inserts lane, which is joined first, takes core 2 from it whenever both
  have work, and the returns finish in the time left before their join. A
  late sampler lane can delay the mixer's, and all of them are counted.
- **Reverb returns a block late** (`two` and `three`; #260 for the engine,
  wired in the app by `Cores::start_mixer`). A return whose reverb can take
  the block back from its pre-delay goes late; every other return stays on
  the audio thread (see the mixer's section below). The app's own Return A
  is the Play page's delay, so the lane only has work once a reverb with a
  pre-delay, all wet, is put on a return. A late returns lane leaves the
  returns silent for that block and the next, and a block of the return's
  input is lost when it goes back on time: both are fallbacks, counted on
  the Dev row "Reverb returns on core 2 · late · lost", in the status line
  (`returns N blocks on core 2, N late, N starts, N stops, N lost`) and in
  the `sound cores: late:` line.
- **One deadline per period** (`cores::lane_deadline`, 12 Nov, after an
  outside review). Every join in a period waits until the same moment at
  most: the sampler's parts (`render_routed_until`), the synth, each mixer
  stage and the returns (`Mixer::set_lane_deadline`). The moment is when the
  sound card's queue would run dry, less a period for the work after the
  joins (the end of the mix and the write). Right after each write the audio
  loop reads the card's delay (`snd_pcm_delay`, one ioctl) and sets the next
  period's deadline from it: two periods after the write when the queue is
  full (three periods in the buffer), sooner after a long period, so the
  deadlines tighten by themselves under load. Before, each join waited a
  period from when it started, so a sampler part, two mixer stages and the
  returns could together wait four periods with three in the buffer. A
  delay that cannot be read counts as a full queue, and says so once in the
  log.
- **Late is a fallback, and never silent** (Jay, 12 Nov): a late sampler
  part leaves half the voices out of that part of the period; a late mixer
  stage leaves those strips silent for the block; a late synth is left out
  of the period; a late returns lane leaves the reverb returns silent for
  two blocks. The counts are on the Dev tab's CPU by engine page (red
  once one is late), in the 30 s `status:` line (`sound cores two: drum
  voices N parts on core 2, N late; mixer N stages on core 2, N late;
  returns N blocks on core 2, N late, N starts, N stops, N lost`, and
  `three` adds `; synth N periods on core 1, N late (left out), N waited
  for`), and in a log line within a second of a late one (`sound cores:
  late: ...`). A late part is a bug or an over-budget design to fix, not a
  setting to tune away.
- **What took, and lanes only where they were asked for.** The start lines
  (`sound cores: ...`) and the Dev page say whether the pin and each lane's
  core and priority took. The lanes are strict (above): a lane that did not
  get its core or SCHED_FIFO runs on the audio thread, in red (`on the audio
  thread (strict lane, not pinned: ...)`), with the same sound on one core.
  If the audio thread itself could not be pinned to core 3, no lane starts
  (`sound cores: lanes not started: ...`, a red "Lanes" row), since a lane's
  worker starts with the audio thread's cores. A refused priority for the
  audio thread is reported and the sound plays on.
- **Safety.** Only the program's own threads change: their allowed cores
  (`sched_setaffinity`) and priority (`sched_setscheduler`), in RAM. The
  only thing written is the Prefs row's line in the app's settings on the
  stick's data partition (never the MPC's own storage). Setting the row
  back to Shared undoes it from the next start; on the workstation,
  removing that line or `write-test-stick.py --fresh-data` does too.
- **Tests.** `cores.rs` checks the modes, the rows and the late lines, that
  the synth on its lane sounds the same bit for bit, that a late synth is
  left out and comes back after the write, the deadline's arithmetic, that
  no lane starts without the audio thread's pin, and that a lane without its
  core plays on the audio thread; mpfree-lanes checks strict lanes, and the
  mixer's tests that a block's deadline holds for every join in it and only
  that block; `rt_safety.rs`
  runs the sound path for 35 s on two cores, and with the synth on three
  (unpinned), with a reverb on a return running a block late, with no
  allocation on the sound path.
- **Other devices.** The cores above are the MPC One's. The MPC Gen 2 (8
  cores, 4 GB, bookmarked by Jay 12 Nov for later) would get its own plan:
  more lanes (each synth, the mixer and the returns on cores of their own),
  still with one core kept for interrupts and one for the screen.

The device round that compares the modes is the app thread's live test
runbook. Its app check picks up the `sound cores: ` and `status: sound
cores ` lines.

`Mixer::set_pipelined_returns` is the returns' turn (Jay 2026-11-12:
reverb returns only): a reverb return's inserts run on a lane one block
behind, during the next block's pad channels, tracks and submixes, so the
reverb leaves the audio core. The reverb takes the block back from its
pre-delay (`Node::lead_in`, `Chain::lead_slot`), so the output is the
reference on time, bit for bit; a return that cannot do that (no pre-delay,
a dry mix, a delay, parallel compression or drive) stays on time
(`mpfree-fx/tests/late_returns.rs`). It has its own lane. The app puts it
on core 2 beside the inserts lane, one priority below it (above), so the
two queue in the order their joins need.

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
