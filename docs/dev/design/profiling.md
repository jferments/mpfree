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

`tools/mpc-one/host_bench_mpc.py` runs the benchmark on the kexec test build
(the 3.10 build on the USB stick, which cannot see the eMMC). **RAM only:**
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
  software path's times for the same lists and its copy to the panel, and how
  far the GPU's picture is from the software one. A summary line gives the
  draw calls per page and how often the glyph atlas was cleared or a mask
  was too big for it (both fallbacks, so both are counted).
- `panel`: the GPU's pictures on the panel. Each page is drawn on the GPU,
  read back and written to `/dev/fb0` (as the app's screen writes, through
  `FbTarget`; no display setting changes) and held 1.5 s; then the busiest
  page is drawn and copied to the panel 60 times each way, GPU and software,
  timed. It answers what a whole frame costs to reach the panel from the
  GPU today, before page flips (G5) remove the read back.
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
`panel pages --hold-ms 1500 --frames 60`. The program draws through Mesa's
surfaceless EGL on the render node, as the 9 Nov offscreen tests did: no
display, no DRM master, no display setting changed. Only the last step
writes anything: the screen's picture, as the app does (the log display,
stopped for the run, redraws when it starts again; if it was not running,
the last page stays until something draws over it). It stops after `describe` if Mesa answers with its software renderer
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
