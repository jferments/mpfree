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

## Budgets (agreed with Jay, 2026-11-11)

The budget is the **worst block**, not the mean: one slow block in ten
thousand is an audible click. The whole audio thread stays under **60 %** of
a period in its worst block (1.74 ms), which leaves 40 % for the kernel,
interrupts, ALSA and a late wake-up.

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
stereo), the Cloud reverb (9.4 MB per instance), the master grain effect and
Grains FX (about 2.7 MB each) and the delay (1.4 MB). The CPU budget is per
period; memory is per instance, so an engine that is cheap to run can still
be costly to have many of.

`--compare A.json B.json` puts two reports side by side, case by case:
B's p99 and worst as a share of A's, and any change in memory. Use it for
the default against the cortex-a17 build from the same MPC run (is NEON
worth it?), or for a change against main on the same machine.

### What can we afford? (`--afford`)

```text
cargo run --release -p mpfree-bench -- --afford [--calibration FILE]
```

It plays each Poly preset at 1, 2, 4, 8 and 16 voices and the sampler with
1, 4, 8, 16 and 32 looping pads held, fits `p99 = fixed + voices x per
voice`, and prints how many voices fit in the module's budget (synth 20 %,
sampler 25 %). That is for one engine alone; several synth tracks share the
synth budget, so two Supersaw tracks get half each. "16+ (75)" means every
count measured fits and the line runs out at 75. Mono and legato presets
play one voice, so they show one cost. With `--calibration` the times and
counts are MPC estimates (the kernel range in brackets); on the MPC they are
its own. `host_bench_mpc.py` runs it on the device after the other two
runs; round 7's step was frozen before that, so it first runs in a later
round.

On the desktop the numbers are the desktop's. With `--calibration FILE` (the
file `--calibrate` prints on the MPC) it adds an MPC estimate and a verdict
against the module budget. On the MPC (the armv7 build) the numbers are the
device's own.

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

`tools/mpc-one/host_bench_mpc.py` runs the benchmark on the kexec test build
(the 3.10 build on the USB stick, which cannot see the eMMC). **RAM only:**
it copies the two builds to `/run/mpfree-bench`, runs each with fixed
arguments (`--calibrate`, `--json <build>.json`, then `--afford`), and prints the output
and the JSON into the results log. It starts, stops and changes no service,
and the benchmark opens no device. It runs as an ordinary process, so the pad
player's real-time audio thread always comes first. Like host_hello_mpc.py it
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
`docs/perf/mpc-one-calibration.txt` and the first per-engine device table.

## Dev tools pages

Jay asked (2026-11-11) for tools in the Dev tools menu to see which
processes use resources and how much CPU each FX, synth, and the audio
playback and processing use. The Hello MPC thread owns the Dev tab and
agreed both pages; their wiring into mpfree-hello rides with round 7. The
data side is here:

- **Processes**: `procs::ProcTable::sample()` once a second on the screen
  thread. Every process on the MPC (Akai's `MPC` if it runs, `mpfree-hello`,
  system services) with CPU, memory, threads and scheduling (normal, FIFO
  80, …), busiest first, plus total and per-core CPU; a tap shows one
  process's threads (`ProcTable::threads`).
- **CPU by engine**: the audio loop's `Profiler` windows through
  `report::engine_rows`: one row per engine and per slot (`synth/Poly
  (track 1)`, `fx/Cloud reverb (master)`), mean, p99 and worst as shares of
  the period, real CPU used, the budget and a fits / tight / OVER verdict,
  and any allocation on the audio thread in red. The same rows go to the log
  every 30 s with the existing `perf:` line, so test-round results carry
  them.

## CI

- `cargo run --release -p mpfree-bench -- --quick --check-allocs` is a gate
  in the rust job (the CI thread's gates PR): allocation on the audio path
  fails the build. It is deterministic, so it is safe on shared runners.
- Timing gates on shared CI runners are noisy. A stable regression gate is
  an instruction count: run the armv7 bench under `qemu-arm` with an
  instruction-counting plugin and compare per case against a stored
  baseline. Wall-clock numbers stay a report, not a gate.
