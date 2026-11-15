# Profiling and performance budgets

MPFree is written on a fast desktop and runs on the MPC One: four 32-bit
Cortex-A17 cores at 1.608 GHz (RK3288, NEON and VFPv4, PREEMPT_RT kernel). The
audio thread is due every **128 frames at 44.1 kHz: 2.902 ms**. Anything the
audio thread does in one period must finish inside that period, every time,
or the output underruns. So every engine has to say what it costs on the
device, in the same units, against one budget. This page is how.

## The pieces

| Piece | Where | What it is for |
| --- | --- | --- |
| `Probe`, `Span` | `rust/mpfree-prof` | Times audio blocks: mean, p50, p99, p99.9, worst, blocks over a limit. Fixed memory, no allocation, safe on the audio thread. A lifetime total plus a window the screen thread takes. |
| `Profiled<N>` | `mpfree-prof` | Wraps any `mpfree_node::Node` (synth, effect, Grains) and times its `process`. It is a node itself, so it drops into a chain slot. |
| `Profiler` | `mpfree-prof` | A fixed set of named probes for a host's audio loop (`period`, `sampler`, `synth`, `fx`, `looper`, `time`, `mix`, or `module/detail`). |
| `SharedSummary` | `mpfree-prof` | Hands a summary from the audio thread to the screen with no lock (a sequence lock over atomics). |
| `alloc::CountingAlloc` | `mpfree-prof`, feature `counting-alloc` | A pass-through global allocator that counts calls and bytes per thread. Install it in a program and every `Span` and `Profiled` also reports allocator calls made inside the block (there should be none), and `alloc::bytes_in` tells what an engine holds once built. It is the crate's only unsafe code, so it compiles only with the feature: mpfree-bench turns it on, the pad player does not. |
| `budget` | `mpfree-prof` | The period and the per-module budget table below. |
| `model` | `mpfree-prof` | Reference kernels that turn a desktop time into an MPC estimate. |
| `procs` | `mpfree-prof` | Every process on the machine from `/proc`: CPU, memory, threads, real-time policy. The data for the Dev tools process page. Read-only. |
| `report` | `mpfree-prof` | Probe summaries as "CPU by engine" rows and a log line, each with its budget verdict. |
| `mpfree-bench` | `rust/mpfree-bench` | Runs every engine at the MPC's period against its budget. Builds for the desktop and for the MPC. |
| `Lane` | `rust/mpfree-lanes` | Runs part of each period on another core and joins it before the mix, with a deadline and a fallback. `mpfree-bench --lanes` measures the handoff and the split. See [Concurrency](concurrency.md). |

## Budgets (agreed with Jay, 2026-11-11)

The budget is the **worst block**, not the mean: one slow block in ten
thousand is an audible click. The whole audio thread stays under **60 %** of
a period in its worst block (1.74 ms), which leaves 40 % for the kernel,
interrupts, ALSA and a late wake-up.

**Which block is judged** (Jay, 2026-11-11, after round 7): an engine's
verdict uses its **p99.9** block, and the whole period's verdict uses its
**worst** block. In round 7 a single stray period marked looper, time and
mix OVER although their p99s fit; the period's worst block is still judged
strictly because it is what underruns. `budget::judge(module, &summary,
MPC)` applies the rule, and the report, the bench (on the device) and the
Dev page should all use it.

| Module | Worst block | µs | Covers |
| --- | --- | --- | --- |
| sampler | 25 % | 726 | Every pad voice, granular pads, the master grain effect. Round 6: worst 24.1 %. |
| synth | 20 % | 580 | Every synth track's instrument. Round 6 (one Poly, up to 18 voices): worst 19.9 %. |
| fx | 10 % | 290 | Every insert, send and master effect, including the safety limiter. |
| looper | 3 % | 87 | The audio looper and the line input. |
| time | 1 % | 29 | Transport, sequencer, clips, arp. |
| mix | 1 % | 29 | Summing, levels, meters, the output copy. |
| **audio thread** | **60 %** | **1741** | Everything above. |

Round 6 shows the synth and the sampler already at their lines. Two things
follow: their worst blocks need a closer look (the mean is about 2 %, the
worst 20 to 24 %, so the worst is a spike such as voice starts, a page fault
or preemption, not steady cost), and new engines must come with device
numbers before they are turned on by default.

## Load shedding: giving up voices nobody hears

When the periods fall behind, the pad player gives up the quietest voices
rather than let the sound card's queue run dry and click, as the
[Deluge](https://seangoodvibes.github.io/DelugeDocs/documentation/getting-started/device-overview/performance)
does (`rust/mpfree-app/src/shed.rs`; item 3 of the 14 Nov performance
review). It is a safety net for a busy bar, not a budget: a song that needs
it every bar is too big, and the log says so.

- **When.** Right after each write the audio loop hands `Shed::decide` the
  period just made: its work, from the last write to this one (the write's
  own wait left out), and how far the card's queue is below full
  (`PCM delay`). A period that took longer than it plays is falling behind.
  While the queue is still within a quarter of its spare periods of full
  (64 frames with 3 periods queued), the queue takes it and nothing is
  given up, so a lone slow period costs nothing. A fast export (the card
  stopped, the loop waiting for the files) is never behind and gives up
  nothing, so the file has every voice; a Listen export plays in real time
  and sheds as playing does, so its file is what was heard.
- **How many.** Enough voices to bring the period back to 90 % of its
  length, at the period's own cost a voice (the sampler's, the synth's and
  the tracks' instruments' time over their voices), at most 8 at once. When
  giving up every voice would still not bring it back (the time went to the
  mixer or the effects), none is: the music is not thinned for nothing, and
  the period is counted as one voices would not have helped.
- **Which.** The least audible voice across the sampler, the pad synth and
  the tracks' own instruments, one at a time. Each instrument offers its
  candidate through `Node::cull_candidate` (a `Cull`: its level now and
  whether its note has ended) and gives it up with `Node::cull`;
  `Cull::audibility` counts a releasing voice 12 dB quieter, since it is
  ending anyway. Levels are relative to a full-velocity note: an envelope
  times the velocity and level gains in the synths, and in the sampler what
  the voice has played lately (`Voice::heard`, its last output sample at the
  end of each render, held and falling 16 % a render), so a quiet sample
  reads quiet. A voice already fading is never offered again, and a plugin
  offers none (its process doesn't say what it plays). A voice given up gets
  the fade a stolen one gets (3 ms in the synths, 64 frames in the sampler).
- **Said.** Each voice given up is counted. The log has one line a second at
  most (`load: gave up 3 voices to keep the sound going (2 of them
  releasing); the slowest period took 112 % of its time`), the 30 s status
  line the totals (`load shedding: ...`), and the Dev tab's Debug page the
  count next to the voices stolen ("Voices stolen · given up").
- **Cost.** Nothing in a period on time but one clock read. Behind, a scan
  of the voices per voice given up. No allocation but the log line.
- **Tests.** `shed.rs` runs a simulated song through a 3-period queue: 24
  voices at 119 % lose 8 (down to what fits) and never run dry; a sudden
  jump to 155 % is caught before the queue empties; lone slow periods give
  up nothing; an overloaded mixer keeps its voices. Each engine's own
  candidate, fade and count: `mpfree-synth/tests/cull.rs`,
  `mpfree-sampler/tests/cull.rs`, the track host's `tests/host.rs`; the
  no-allocation tests give up voices too.

## Using it in an engine or a host

In a host's audio loop:

```rust
use mpfree_prof::{budget::MPC, Profiler, Span};

// At setup (allocates).
let mut prof = Profiler::new(&["period", "sampler", "synth", "fx"], MPC.ns());
let (period, sampler, synth) = (0, 1, 2);

// Every period (never allocates).
let whole = Span::start();
let s = Span::start();
engine.render(&mut l, &mut r);
prof.stop(sampler, s, 128);
// ... the synth, the FX ...
prof.stop(period, whole, 128);

// On the screen thread every 250 ms: take_windows, then
// mpfree_prof::report::engine_rows(...) for the page and the log line.
```

For one node, wrap it: `Profiled::new(node, limit_ns)`. For several slots in
a chain, the chain owner (FX thread) adds a probe per slot.

A `Span` costs two clock reads (about 90 ns on the build container;
`mpfree-bench --calibrate` prints the figure for the machine it runs on). On
the audio thread that is fine per engine or per slot; do not time per voice
or per sample.

## mpfree-bench

```text
cargo run --release -p mpfree-bench -- --quick --check-allocs
```

Each case plays an engine the way a busy song would for 20 s of audio (2 s
with `--quick`) at 128 frames and 44.1 kHz:

- **synth:** every Poly preset with 8-note chords changing every half
  second, and Init at 16 voices with unison 4.
- **sampler:** a pad hit every 4 blocks over 20 pads with 24 voices, with
  and without the master grain effect.
- **fx:** every effect, Grains FX and a 4-slot insert chain, on a tone under
  noise bursts, and the mixer with a song's inserts.
- **time:** the transport looping 4 bars at 128 BPM with 8 clips playing
  16th-note chords (one at 3:2 speed, one swung and quantised), the first
  through the arp at 32nds over 2 octaves.
- **looper:** 60 s of looper memory, recording a 2-bar first pass from the
  input, then overdubbing layer after layer with an undo every 30 s.
- **mix:** the mixer (from the FX thread) idle, and routing 12 tracks and 8
  pads, and 16 tracks and 32 pads, with no inserts. The pad player's own
  summing, meters and output copy are measured on the device by its "CPU by
  engine" page.
- **period (the whole audio thread, 60 %):** "Song" runs a busy song's
  engines one after another in every block: the sampler with the master
  grains, an 8-voice Supersaw and the Sub Bass (a mono preset), the mixer with a
  song's inserts, the time engine and the looper. It answers "does a whole
  song fit", including what the engines cost each other in the caches.

It prints mean, p99 and worst per block as microseconds and as a share of
the period, and **mem MB**: what the engine holds once built (its buffers,
delay lines and sample memory, counted by the allocator). `--check-allocs`
fails if any engine allocated inside a block. `--json FILE` writes the
results (each case has `mem_bytes`); `--filter TEXT` picks cases by name or
module.

On the desktop, the biggest holders are the looper (20 MB for 60 s of
stereo; the app's Prefs "Loop memory" default of 5 minutes is about 110 MB,
and 20 minutes about 437 MB, all locked in RAM), the Cloud reverb (9.4 MB per instance), the master grain effect and
Grains FX (about 2.7 MB each) and the delay (1.4 MB). The CPU budget is per
period; memory is per instance, so an engine that is cheap to run can still
be costly to have many of. The sampler's Rec tab (line input into the
sampler, once it lands after round 8) adds one 75 s stereo buffer, 26.46
MB, only while the line input is on, and nothing otherwise.

`--compare A.json B.json` puts two reports side by side, case by case:
B's p99 and worst as a share of A's, and any change in memory. Use it for
the default against the cortex-a17 build from the same MPC run (is NEON
worth it?), or for a change against main on the same machine.

### What can we afford? (`--afford`)

```text
cargo run --release -p mpfree-bench -- --afford [--calibration FILE]
```

It plays each Poly preset at 1, 2, 4, 8 and 16 voices, the sampler with
1, 4, 8, 16 and 32 looping pads held, and four granular pads held with 16,
32, 64, 128 and 256 grains alive in all. It fits `p99 = fixed + voices x per
voice`, and prints how many voices fit in the module's budget (synth 20 %,
sampler 25 %). That is for one engine alone; several synth tracks share the
synth budget, so two Supersaw tracks get half each. "16+ (75)" means every
count measured fits and the line runs out at 75. Mono and legato presets
play one voice, so they show one cost. In the grains row a "voice" is one
live grain, so its per-voice cost is the cost of a grain; 256 is the
sampler's whole grain budget. With `--calibration` the times and
counts are MPC estimates (the kernel range in brackets); on the MPC they are
its own. `host_bench_mpc.py` runs it on the device after the other two
runs; round 7's step was frozen before that, so it first runs in a later
round.

On the desktop the numbers are the desktop's. With `--calibration FILE` (the
file `--calibrate` prints on the MPC) it adds an MPC estimate and a verdict
against the module budget. On the MPC (the armv7 build) the numbers are the
device's own.

### Lanes (`--lanes`)

```text
cargo run --release -p mpfree-bench -- --lanes [--lane-cores A,B] [--quick]
```

Runs only the lane cases: the same work on one core and split over two
(see [Concurrency](concurrency.md)). It is the one mode that starts more
threads: a worker per lane case, at normal priority, closed before the next
case. `--lane-cores A,B` pins the bench's thread to core A and the workers to
core B (3,2 on the MPC); without it nothing is pinned. The default run is
unchanged. The sampler appears three ways: one engine of 24 voices, two
engines of 12 voices splitting the pads, and one engine of 24 voices split
natively (`Engine::split`, see the sampler architecture), each on one core
and on two.

### The cost model

Desktop and A17 differ by a different factor for each kind of work, so
`model` has five reference kernels, each about one block of one kind: a
biquad cascade (scalar float), a mix (vectorisable), `sinf`/`expf`/`floorf`
(library maths; `floorf` is a library call on ARMv7), interpolated random
reads in 8 MiB (cache misses: grains, delays, reverbs) and voice-allocation
style control code. Their device/desktop ratios give the estimate (the
geometric mean) and its range (the lowest and highest ratio). Device numbers
always win over estimates.

### NEON

Rust's `armv7-unknown-linux-gnueabihf` target leaves NEON off. Turning it on
(`-C target-cpu=cortex-a17` or `-C target-feature=+neon`) does **not**
vectorise float DSP by itself: LLVM will not auto-vectorise `f32` loops with
ARMv7 NEON, because NEON flushes denormals and so is not IEEE 754 exact. A
check of the mpfree-bench binary built both ways found no NEON float
instructions in either (only libc's `memchr`). What the flag may still buy
is scheduling for the A17 and the full VFPv4 register file. Float SIMD on
the device needs explicit NEON code (today a nightly-only
`core::arch::arm` API) or a fixed-point path. The device run of both
builds says what the flag is worth; until then, treat "+neon makes it
faster" as unproven.

## Measuring on the device

**First results (round 7):** [docs/dev/perf/mpc-one-round7.md](../perf/mpc-one-round7.md).
The MPC is about 6.6× slower than the build workstation on average, but 4×
for synth-like work and 9 to 13× for memory-heavy engines (reverb, sampler,
grains), so one desktop factor is not a safe estimate. The device's
calibration file is [docs/dev/perf/mpc-one-calibration.txt](../perf/mpc-one-calibration.txt).

`tools/mpc-one/host_bench_mpc.py` runs the benchmark on the test build
(the 3.10 system on the USB stick, which cannot see the eMMC; on the installed
3.10.1 the boot hook starts it at power-on with the stick in). **RAM only:**
it copies the two builds to `/run/mpfree-bench`, runs each with fixed
arguments (`--calibrate`, `--json <build>.json`, `--afford`, then `--lanes --lane-cores 3,2`), and prints the output
and the JSON into the results log. The benchmark opens no device and runs as an ordinary process. For a clean
run (the round 8 ask), after the copy the tool stops our own pad player and log display (`mpfree-test-app`,
`mpfree-test-screen`; RAM state only, Akai's services are never touched) and starts whichever was running
again at the end, also when a run fails or is stopped. Like host_app_mpc.py it
uses only the test build's own host key, so on stock 3.6 it runs nothing.
It needs `MPFREE_ACCEPT_BENCH=YES` and a terminal; each step waits for
Enter. Its tests (`tests/test_host_bench_mpc.py`) run it against a fake test
build under BusyBox.

The two builds it expects:

- default: `rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-bench`
  (static, as every device build: `-C target-feature=+crt-static -C
  relocation-model=static`);
- a17: the same plus `-C target-cpu=cortex-a17`, built with `--target-dir
  rust/target/a17`.

The step goes in a Hello MPC test round's runbook, rehearsed against Jay's
paths, and runs only on Jay's go. Its results become
`docs/dev/perf/mpc-one-calibration.txt` and the first per-engine device table.

### GPU benchmark (step G2)

Jay asked (12 Nov) for the screen to be drawn on the GPU, and for GPU compute
where it pays. Before any of it runs in the app, `mpfree-gpu-bench`
([rust/mpfree-gpu](../../../rust/mpfree-gpu/README.md)) measures it on the MPC:

- `screens`: every sample page, recorded by `mpfree --display-lists`, played
  on the GPU and in software at the panel's size (800 × 1280, turned 270
  degrees): the first frame with its uploads, the whole page again (what a
  page change costs), the redraw a tick later, reading the frame back, the
  software path's times for the same lists and its turn into the panel's
  layout, and how far the GPU's picture is from the software one. A summary line gives the
  draw calls per page, how often the glyph atlas was cleared or a mask
  was too big for it (both fallbacks, so both are counted), the vector
  shapes the GPU worked out and how often their atlas was cleared.
- `panel`: the GPU's pictures on the panel. Each page is drawn on the GPU,
  read back and written to `/dev/fb0` (as the app's screen writes, through
  `FbTarget`; no display setting changes) and held 1.5 s; then the busiest
  page is drawn and copied to the panel 60 times each way, GPU and software,
  timed. It answers what a whole frame costs to reach the panel from the
  GPU with a read back (live test 3: 63 ms a frame, against software's 32).
  Software's time is split into the draw, the turn into the panel's layout
  and the write to `/dev/fb0` (since 15 Nov; `flip` splits it the same
  way). In live test 3 software took 32 ms a frame to reach the panel and
  a page's draw about 11 ms, so the turn and the write may cost more than
  the draw. Drawing software in the panel's own layout
  would save the turn but not the write, so these say whether that is
  worth doing.
- `flip`: the GPU drawing straight into buffers the panel shows (DRM/KMS
  page flips; no read back, no copy). With the screen untouched it checks
  the first page in a screen buffer's own memory and against software;
  then it takes the display, points the screen at its buffers in the mode
  it already runs, shows each page 1.5 s and times 60 frames each way: the
  GPU's (draw into the next buffer, flip) and software's (draw, turn and
  copy into a buffer that is never shown), with the CPU each takes. It puts
  the screen back as it found it before it exits.
- `compute`: an FIR filter over a stereo block as an OpenGL ES 3.1 compute
  shader, uploaded, run and read back, against the same job on one CPU core:
  one period with 64 and with 1024 taps, and 32 periods with 1024 taps. It
  answers whether GPU work can come back inside a period (2.902 ms) at all,
  and from what size the GPU beats a core.

`tools/mpc-one/host_gpu_mpc.py` runs it on the kexec test build. **RAM only,
offscreen:** like host_bench_mpc.py it talks only to the test build's own host
key. It checks the ARM build on the workstation (loader
`/lib/ld-linux-armhf.so.3`, only `libc.so.6` and `libgcc_s.so.1` linked,
nothing newer than glibc 2.34), makes the pages with the workstation's build
of the app, checks on the MPC (read-only) that the GPU's render node and
`libEGL.so.1` and `libGLESv2.so.2` are there and who holds `/dev/dri`, copies
the program and the pages to `/run/mpfree-gpu`, stops our pad player and log
display for a clean run (started again at the end, whatever happens), and
runs `describe`, `compute --iterations 500`, `screens pages --frames 30` and
`panel pages --hold-ms 1500 --frames 60`, and, only with `--flip`, step 7:
`flip pages --hold-ms 1500 --frames 60`. The program draws through Mesa's
surfaceless EGL on the render node, as the 9 Nov offscreen tests did: no
display, no DRM master, no display setting changed. Only the last step
writes anything: the screen's picture, as the app does (the log display,
stopped for the run, redraws when it starts again; if it was not running,
the last page stays until something draws over it). **Step 7 is state-changing, in RAM:** it first checks (read-only) that `/dev/dri/card0` is there and no program has a display node open (so the kernel can take the screen back if the program were killed), then waits for Enter; the program takes the display (DRM master) and points the lit CRTC at its own buffers, changing no mode, timing or panel setting, and puts the CRTC back before it exits, also on Ctrl-C, a closed session or a failure. If it ends without saying "the screen is back as it was", the tool says to switch the MPC off and on; nothing is stored. It stops after `describe` if Mesa answers with its software renderer
instead of the Mali, since the numbers would not be the GPU's. It needs
`MPFREE_ACCEPT_GPU_BENCH=YES` and a terminal; each step waits for Enter. Its
tests (`tests/test_host_gpu_mpc.py`) run it against a fake test build under
BusyBox; on 2026-11-12 it also ran end to end against that fake build with the
real ARM program under qemu-arm and Ubuntu's ARM Mesa. Like every device step
it goes in a test round's runbook and runs only on Jay's go.

## Dev tools pages

Jay asked (2026-11-11) for tools in the Dev tools menu to see which
processes use resources and how much CPU each FX, synth, and the audio
playback and processing use. The Hello MPC thread owns the Dev tab and
agreed both pages; their wiring into mpfree-app rides with round 7. The
data side is here:

- **Processes**: `procs::ProcTable::sample()` once a second on the screen
  thread. Every process on the MPC (Akai's `MPC` if it runs, `mpfree-app`,
  system services) with CPU, memory, threads and scheduling (normal, FIFO
  80, …), busiest first, plus total and per-core CPU; a tap shows one
  process's threads (`ProcTable::threads`).
- **CPU by engine**: the audio loop's `Profiler` windows through
  `report::engine_rows`: one row per engine and per slot (`synth/Poly
  (track 1)`, `fx/Cloud reverb (master)`), mean, p99 and worst as shares of
  the period, real CPU used, the budget and a fits / tight / OVER verdict
  (`budget::judge`: p99.9 for an engine, worst for the period),
  and any allocation on the audio thread in red. The same rows go to the log
  every 30 s with the existing `perf:` line, so test-round results carry
  them. In the app (`engines.rs`) a `control` row follows the engines: the
  whole period less the five engines, worked out each period when the
  period's span stops. It is everything on the audio thread that is not
  sound (the screen's actions, MIDI in, lights and MIDI out, the app's own
  work, log lines, the meters and panels sent to the screen, the copy to the
  card's format), which the outside review of 12 Nov named as the first
  thing to move off the audio thread. It has no budget yet, so it is shown
  on its worst block, dim, until the control thread moves it.
- **Slow periods outside the engines** (`parts.rs`): the `control` row is
  one number, so the loop also times its own parts outside the engines
  (screen actions, MIDI in and buttons, app and effects, pads and lights,
  screen updates, and the time after the sound card's write, which the
  period's span leaves out), and each screen action, MIDI event and effect.
  A period whose parts take half a period or more is slow: counted, on this
  page under the budgets ("Slow periods outside the engines", red once there
  is one, with its worst, and below it the part that took most), and said in a
  `control: slow period at +T s: ...` line (at most one a second, the rest
  counted) that names the slowest part and the slowest single action,
  event or effect by its debug text, cut to 48 bytes without allocating.
  Every 30 s `status: control parts (worst in 30 s): ...` gives each part's
  worst. Why: in the 12 Nov live test the period's worst reached 176 % and
  295 % with every engine under 51 %, and nothing said where the time went
  (the looper's waveform, drawn in one go outside the looper's probe on
  that build; #223 has since spread it over periods inside the probe). The
  same rows show the controller output's refused, dropped and stalled
  messages and the report lines dropped (control.rs).
- **Memory by engine** (`memory.rs`, 13 Nov; Jay asked for RAM beside
  CPU): under the audio loop's rows on CPU by engine, what each part holds,
  from what it knows of its own buffers: every kit's sounds (those undo
  keeps included, a sound two kits share counted once), the stretched
  copies, the Rec tab's input buffer while it is held, the looper's sound
  used and set aside, and the synth's wavetables. Then "Everything else",
  the app's resident memory (from the developer tools' /proc sample) less
  what the rows count, where a leak would show, and the app in all against
  the system's memory. Live test 3's resident size peaked at 195.8 MB of
  2016.7 MB, most of it the looper's 110 MB set aside. There is no memory
  budget per engine yet: these rows are what one would be judged by. The
  last row, "Free for loads (keeps 256 MB spare)", is the system's
  `MemAvailable`, red under the floor below.
  **First device numbers (live test 2, 13 Nov, main 7b2cae9, sound cores
  shared, about 11 minutes of free play, at most 10 sounds):** no slow
  period. Each part's worst in any 30 s window stayed between 0.03 and
  0.37 ms, against the 1.45 ms mark (13 % of a period at most). The 0.37 ms
  were screen actions: a step-editor tap and a synth preset load. MIDI in
  and buttons peaked at 0.26 ms, and "after the write" ran 0.16 to 0.25 ms.
  There were no controller output, report or chain editor problems, and 1
  underrun in 216 968 periods, at the start. So nothing more moves to a
  control thread for now (see [concurrency.md](concurrency.md)). A busier
  run (a full song, the lanes on) should be read the same way. The lines
  are in the project files, `hello-mpc/live-test-2-0623/control-lines.txt`.

## Memory floor: refuse a load rather than be killed

The MPC has 2 GB and no swap, so when memory runs out the kernel kills the
app mid-song. The caps per file (64 MB a prepared sample, 128 samples a
sampler) don't bound a whole project: samples, stretched copies and
auto-samples could outgrow it (review item 6, 14 Nov). `memory.rs` keeps a
floor, `FLOOR` = 256 MB of `MemAvailable` (what the kernel can give
without swapping; live test 3 had 1706 to 1718 MB of it with MPFree
running):

- `room_in(free, need)` refuses a load that would leave less than the
  floor, with the reason: `not enough memory: needs about 120 MB, 300 MB
  free, 256 MB kept spare`. Where /proc/meminfo can't be read (not Linux)
  everything is let through.
- What a load needs is estimated before it allocates: a sound file's
  length times 3 (the file and its 16-bit samples as `f32`), times 5 for
  FLAC (`decode_need`); a stretched copy's frames and the source part the
  stretch thread is handed; an auto-sample's `Plan::estimate().bytes_most`.
- The pad player checks at each load: the browser's Load and preview
  (`read_one`, on their threads), each sound as a project's kits open (one
  that doesn't fit is left out, named in the log, and the hint says how
  many), each stretched copy (refused once per copy and tempo; the pad
  repitches, as it does until any copy is ready) and Auto-sample's Start.
  A test gives the page its own free memory (`SamplePage::set_mem_free`).
- The looper and the audio tracks set their memory aside at start (Prefs'
  Loop memory and Audio track memory), and the Rec tab's input buffer is a
  fixed 26 MB, so they aren't checked.
- The screen thread already reads /proc/meminfo every 2 s for the RAM
  chip; `Watch` logs one line, and says it in place of the hint, when free
  memory falls under the floor (`memory: 200 MB free, under the 256 MB
  MPFree keeps spare; ...`) and one when it is 32 MB above it again (with
  how long and how low).

## The screen's cost

The software screen thread (`mpfree-screen`) took 18 to 40 % of a core in
live tests 2 and 3, the largest user after the sound. `screen_cost.rs`
times each frame on that thread: painting the changed widgets into the
canvas, and copying them to the panel (`gui::present`, the one place a
frame leaves the thread, where a GPU drawing process would take it), with
the native rows the copy wrote and the loop's wakeups.

- **Dev > Performance, "Screen" row**, every second: `N fps · paint X ms ·
  copy Y ms · slowest Z ms`. While the GPU draws (`--gpu-screen`,
  drawing.rs): `GPU · N fps · paint X · send Y · GPU Z ms`, where painting
  records the frame's list, sending hands it to the GPU process, and Z is
  that process's slowest frame in the second (building and drawing it).
  After a fallback to software, in the warning colour: `software, GPU off
  1× · N fps · slowest Z ms`. In the 30 s line the copy is the send while
  the GPU draws, and it writes no native rows.
- **`status: screen (30 s): ...`** every 30 s in the log: frames, the share
  of a core spent drawing, paint and copy mean and worst, native rows a
  frame, wakeups a second, and the page that cost the most. The app check
  (`host_app_mpc.py`) keeps these lines in its summary.

**Why the copy matters more than the paint.** The panel is mounted turned:
its native rows are the screen's columns. A changed rectangle's width is
the number of native rows copied, so a thin strip across the screen (the
top bar, 1256 by 60) rewrites 1256 of the panel's 1280 rows, about 4 MB.
On the build workstation (13 Nov) that strip took 0.07 ms to paint and 1.2
to 1.8 ms to copy.

**Turning only what changed** (14 Nov). The copy used to turn whole native
rows from the canvas, so that strip turned the whole screen to change
1256 × 60 pixels. `FbTarget` now keeps a copy of what the panel shows, in
the panel's own layout. It turns only the changed rectangles into that copy
and writes the same whole rows from it, so the kernel gets the same bytes
as before (`the_screen_gets_the_same_bytes_as_when_whole_rows_were_turned`,
fb.rs). Measured on the workstation with `turn_speed` (fb.rs, ignored by
default):

| Changed | Whole rows turned | Only the rectangle |
| --- | --- | --- |
| A chip in the top bar, 120 × 40 | 0.12 ms | 0.005 ms |
| The top bar, 1280 × 48 | 1.6 ms | 0.06 ms |
| A meter, 24 × 300 | 0.022 ms | 0.007 ms |
| A panel, 600 × 400 | 0.70 ms | 0.33 ms |
| The whole screen | 1.7 ms | 0.8 ms |

The whole screen gains too, because the old way cleared its buffer before
each turn. The write itself is unchanged: whole native rows, as the kernel
needs, so the strip still sends about 4 MB to the kernel. The MPC's numbers
will come from the next app round's `status: screen` lines.

**Idle redraws** (fixed 13 Nov). Two things redrew with nothing changing:
the Pads tab's browser, at every frame (30 a second: 0.5 ms paint and 0.9
ms copy each on the workstation, so roughly 30 to 40 % of a core on the
MPC by round 7's 6.6 to 10 times, an estimate), because reaching for its widget to take
its inputs marked it changed (`Ui::get_mut`; `Ui::get_mut_quiet` is for
state a widget does not draw); and the top bar, every second, whether or
not a chip changed. The test `no_page_redraws_while_idle` (gui.rs) ticks
every sample screen for three idle seconds and fails if any page draws,
except the ones that show something live (the touch test's fading trails,
and the Rec tab's meters falling back after the input stops).

**The pages' clock and the loop's sleep** (fixed 13 Nov, `pace.rs`,
`touch_bell.rs`). The screen loop woke every 4 ms to look at the
touchscreen, and while nothing redrew it ticked the pages on each of those
passes by the time since the last frame drawn (up to 0.1 s a tick). The
pages' clocks ran about 23 times fast: one idle second on the workstation
was 241 wakeups and 233 ticks that moved the clocks 22.8 s, so a told line
(4 s) went in about 0.2 s, holding a macro card opened the macros panel in
well under the half second the manual gives, and the top bar looked at its
chips many times a second. Now the pages tick every 33 ms, drawn or not,
each by the time since the last tick, and the next tick waits a frame after
a frame finishes drawing, as before. Between ticks the thread sleeps until a
message, the next tick or the touchscreen's doorbell: a thread of its own
(`mpfree-touch-bell`) sleeps in read(2) on a second read-only handle on the
panel (each evdev reader gets its own copy of every event, and nothing is
grabbed) and wakes the screen thread when a report comes, once until the
screen thread reads. Idle: 32 wakeups and 32 ticks a second, 0.998 s of
clock. If the doorbell cannot open or stops, the thread reads the panel
every 4 ms as before; that is logged (`touch: no doorbell` or `touch:
doorbell stopped`), counted, and shown after the reader's line in the Touch
lab. Tests: `the_idle_screen_thread_keeps_time_and_sleeps` and
`without_the_doorbell_the_screen_thread_reads_the_panel_every_4_ms` (gui.rs),
`pace.rs` and `touch_bell.rs`.

**Live parts** (fixed 13 Nov). The Rec tab's input view (meters and the
input strip, 904 by 494) redrew whole at every frame, input or not: on the
workstation 0.42 ms to paint and 1.11 ms to copy its 904 native rows, so
roughly 30 to 45 % of an MPC core while the tab shows (round 7's 6.6 to 10
times, an estimate). Now it works out its picture in pixels
(`InputView::look`: the meters' lengths and texts, each pixel column of the
strip, the marks' places) and the tab redraws it only when that changes.
The strip moves a whole column at a time (the newest column's edge used to
creep and jump back between columns), marks a fixed time back such as Skip
back stay put, and "held" counts whole seconds (in hundredths it changed at
every frame until the buffer was full). A still input now redraws 3 of 90
frames (the caption, once a second). With no input at all the tab no longer
redraws at every frame either. While input comes in, the picture moves at
every frame, so the tab redraws it at most every other frame
(`rec_tab::LIVE_GAP_S`, about 15 a second; meters included, since they
share its rows): a loud input redraws 45 of 90 frames (was 90), 13,560
native rows a second instead of 27,120 (14 Nov). The tab compares with the
picture last painted (`InputView::painted`), whatever painted it, so the
screen is never more than a frame behind; a change of settings still shows
at the next frame. The test `the_strip_redraws_only_when_its_picture_changes`
(mpfree-sampler-ui) checks both, and that the screen matches a full redraw
whenever the picture is up to date. The I/O view asked for a
frame at every tick and painted nothing (`io_tick` reached for its widget
with `Ui::get_mut`); it now asks only when a section changed. The touch
test (a developer page) still redraws its area at every frame.

Still to do: a loud input still copies 904 native rows 15 times a second.
The panel takes whole native rows, so the meters, which run across the
view's width, cost about as much as the whole view; going lower needs a
different picture (the meters upright at one side, the strip wiping
across instead of scrolling), which is the owner's call.

**Painting each pixel once** (14 Nov). A whole-page paint filled the
screen with the background, then every widget filled its own rectangle
with it again, and then most widgets covered it with an opaque panel: on
the 73 sample pages, 3.1 times the screen's pixels written opaque per
page. `Ui::paint` now fills only the gaps between the widgets
(`Rect::uncovered`), and clears a widget's part only where its first call
leaves it: nothing under an opaque fill that covers it, only the corners
under an opaque rounded panel (`Cleared`, in ui.rs). That is 1.5 times the
screen's pixels a page, and the same for each panel's part redrawn.
Measured on the workstation by replaying the 73 pages' lists in software:
plain opaque fills from 161 to 43 million pixels in all, and about 82 ms to 62 ms
for the pages together (instructions down 10 %; the rest is memory traffic,
which costs the MPC more). The picture is the same: every preview screen in
all four themes is byte for byte as before, and two tests check it against
painting the old way over a screen of noise, with random widgets and two
themes whose background or backdrop lets through what is under it (there a
background fill no longer hides it, so the whole screen is filled as
before): `clearing_less_draws_the_same_picture` and
`pages_paint_the_same_picture_as_before` (ui.rs).

## CI

- `cargo run --release -p mpfree-bench -- --quick --check-allocs` is a gate
  in the rust job (the CI thread's gates PR): allocation on the audio path
  fails the build. It is deterministic, so it is safe on shared runners.
- Timing gates on shared CI runners are noisy, so the regression gate is an
  instruction count (below). Wall-clock numbers stay a report, not a gate.

### Instruction counts (the `instructions` CI job)

```text
cargo build --manifest-path rust/Cargo.toml --release --locked -p mpfree-bench
python3 tools/ci/instr_gate.py [--filter TEXT] [--bless]
```

`mpfree-bench --count` runs each case for 200 blocks (after the same 50-block
warm-up) between two callgrind client requests (`rust/mpfree-bench/src/vg.rs`;
outside Valgrind they do nothing). `instr_gate.py` runs it under
`valgrind --tool=callgrind`, reads one dump per case, and compares the
instructions per block with `rust/mpfree-bench/instructions.txt`:

- **RISE** (more than 5 % above the baseline) fails the job.
- **DROP** (more than 5 % cheaper) is reported: bless to keep the gain.
- **NEW** and **GONE** cases are named and don't fail; bless to add or drop them.

The counts are exact: two runs give the same numbers, whatever else the
runner is doing. They are x86-64 counts, a stand-in for the MPC's cost, so
they catch a change in what an engine does, not the device's real time; the
device rounds still measure that. A count depends on the compiler, so the job
and the baseline use the toolchain clippy pins (1.97.0), and the script warns
when the baseline came from another one. The whole run takes about a minute
under callgrind. After an intended change in cost, bless with
`python3 tools/ci/instr_gate.py --bless` and say why in the PR. The runner needs
`valgrind`; without it the job warns and skips.

### ARMv7 counts for the instrument engines

```text
python3 tools/inst_arm_cost.py [--blocks N] [--jobs J] [--no-build] [CASE ...]
```

The x86 counts above miss what is slow on the MPC's ARMv7 build: no hardware
divide, `floor` and 64-bit conversions as library calls. So Keys, Drawbar,
Squelch and Analog are also counted as ARMv7 code, the way the effects are
(fx.md). `rust/mpfree-synth/examples/inst_cost.rs` plays the bench's cases
for the four engines, plus Poly Init with 8 voices for scale, with the same
notes from the same seeds. The script builds it for
`armv7-unknown-linux-gnueabihf` and runs each case under
`qemu-arm -one-insn-per-tb -d exec,nochain` with N blocks (200 by default)
and with none. Every instruction run prints one trace line, so the
difference over N is the instructions per block. It also prints each case's
output hash, which matches the native build's: the engines sound the same to
the bit on ARMv7. The trace runs about 300k instructions a second, so all 16
cases take about 20 minutes, four at a time. These are instructions, not
cycles: cache misses and VFP stalls come on top, so the device still decides
voice counts. The counts of 14 Nov are in synths.md, beside each engine's
x86 count.

### The soak (the `soak` CI job)

```text
cargo run --release -p mpfree-bench -- --soak [--blocks N] [--filter TEXT]
```

The instructions gate runs each case for 2 s. A buffer that grows only when a
count passes a power of two shows much later: on live test 3, #273's write
gaps reallocated after 51 minutes. `--soak` runs each whole-period case
(the song: sampler, two synths, the mixer with its effects, the time host
and the looper) for 2^21 blocks, about 101 minutes of sound, with the
counting allocator read around every block, after the usual 50-block
warm-up. It prints a progress line every 2^18 blocks and one line a case.
The first run (14 Nov, the build host, busy with a test run at the same
time, 822 s):

```text
Song: sampler, 2 synths, mixer+FX, time, looper: 2097152 blocks (101.4 min of sound), worst 38542 us (1328 % of the period) at block 193926, 321 slower than the period; no block allocated
```

The slow blocks there are the busy host taking the CPU away, not the song.

Any block that allocates or frees fails it (exit 2), naming the first eight
blocks and their calls, so a doubling shows at the block where it happened.
The times are for reading. The `soak` job runs it only by hand: on GitHub,
Actions, "MPC read-only inventory checks", **Run workflow** on `main` with
the Soak box ticked (before a long device test, say). A quarter of an hour
of a runner is too much for every PR or every merge, and the runners' job
guard (jferments/ci-infrastructure `docs/runner-job-guard.md`) admits push,
pull request and by-hand runs but no schedule, so it can't be nightly
without a change there. It covers the engines' audio path, not the
pad player's own loop (the sound card's writes, the report): that loop
runs only on a sound card, so a long device run still checks it.
