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
| `alloc::CountingAlloc` | `mpfree-prof` | A pass-through global allocator that counts per thread. Install it in a program and every `Span` and `Profiled` also reports allocator calls made inside the block (there should be none). |
| `budget` | `mpfree-prof` | The period and the per-module budget table below. |
| `model` | `mpfree-prof` | Reference kernels that turn a desktop time into an MPC estimate. |
| `procs` | `mpfree-prof` | Every process on the machine from `/proc`: CPU, memory, threads, real-time policy. The data for the Dev tools process page. Read-only. |
| `report` | `mpfree-prof` | Probe summaries as "CPU by engine" rows and a log line, each with its budget verdict. |
| `mpfree-bench` | `rust/mpfree-bench` | Runs every engine at the MPC's period against its budget. Builds for the desktop and for the MPC. |

## Budgets (proposal for Jay, 2026-11-11)

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

Each case plays an engine the way a busy song would (chords changing every
half second, a pad hit every 4 blocks over 20 pads with 24 voices, effects on
a tone under noise bursts) for 20 s of audio (2 s with `--quick`) at 128
frames and 44.1 kHz, and prints mean, p99 and worst per block as microseconds
and as a share of the period. `--check-allocs` fails if any engine
allocated inside a block. `--json FILE` writes the results; `--filter TEXT`
picks cases.

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

Planned, not yet run. The armv7 `mpfree-bench` is a static binary that reads
and computes only: it opens no device, writes nothing unless `--json` names
a file, and runs on one thread at normal priority for about a minute per
build. The step belongs in a Hello MPC test round (paced through
`tools/mpc-one/results.py`, one zip per round) so its output lands with the
rest of the round's results:

1. `mpfree-bench --calibrate` (default build, then the `cortex-a17` build):
   the calibration files, kept in `docs/perf/`.
2. `mpfree-bench --json …` for both builds: every engine's device numbers.

The runbook step comes with the round, written and rehearsed word for word
against Jay's paths, labelled with what it touches, and run only on Jay's
go.

## Dev tools pages (proposal to the Hello MPC thread)

Jay asked (2026-11-11) for tools in the Dev tools menu to see which
processes use resources and how much CPU each FX, synth, and the audio
playback and processing use. The Hello MPC thread owns the Dev tab; the
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

## CI (proposal to the CI thread)

- `cargo run --release -p mpfree-bench -- --quick --check-allocs` as a gate:
  allocation on the audio path fails the build. It is deterministic, so
  it is safe on shared runners.
- Timing gates on shared CI runners are noisy. A stable regression gate is
  an instruction count: run the armv7 bench under `qemu-arm` with an
  instruction-counting plugin and compare per case against a stored
  baseline. Wall-clock numbers stay a report, not a gate.
