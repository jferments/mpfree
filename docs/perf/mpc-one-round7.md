# MPC One: first device numbers (round 7)

Jay ran round 7 (head cc7f06d) on the MPC One on 2026-11-11. This page is
the profiling thread's reading of those numbers: what they mean for the
budgets in [profiling.md](../project/profiling.md), and what to change.

**Sources:** the round 7 benchmark step (`host_bench_mpc.py`, both builds,
the 35-case benchmark from #77), and the pad player's own status lines from
the same boot. The calibration lines are in
[mpc-one-calibration.txt](mpc-one-calibration.txt).

**One caveat on the whole run:** the benchmark ran while the pad player was
up. Its audio thread runs at SCHED_FIFO 70, and the boot has isolcpus=2-3.
The means should be sound. The p99 and worst blocks may include preemption,
so treat them as upper bounds.

All percentages are shares of one period: 128 frames at 44.1 kHz, 2902 µs.

## 1. How much slower the MPC is

The MPC is about **6.6× slower than the build workstation** on average.
That is the geometric mean of the ratio of means over the 35 cases. It
counts the test-signal overhead described in section 2, so it is a little
high for the small effects. The ratio is not one number. It depends on the
kind of work:

| Kind of work | MPC ÷ workstation | Cases |
| --- | --- | --- |
| Synth voices, drive, chorus, phaser, EQ | 4 to 6× | Poly presets, Drive, Chorus, Phaser, EQ |
| Sampler voices | 8× | Sampler, 20 pads |
| Big delay memories | 9.5 to 13× | Cloud reverb, Grains FX |
| The reference kernels | 2.6× (biquad) to 12.6× (mix) | `--calibrate` |

Engines that read a lot of memory (reverb delay lines, sample data, grain
buffers) lose the most. The A17's caches and memory are far smaller and
slower than the workstation's. So a desktop number times one factor is not
a safe estimate: a reverb tuned on the workstation costs twice what the
average factor says.

**The `cortex-a17` build is no faster.** Its means are within ±6 % of the
default build's in 33 of 35 cases. The other two go opposite ways (Delay
15 % faster, Lo-fi 34 % slower), which looks like noise from the running
pad player. This confirms the NEON finding in
profiling.md: the flag alone does not vectorise our `f32` code. A gain from
NEON needs code written for it. That is a later decision, and the
algorithm work in section 4 comes first.

## 2. What fits today

These are means, with p99 in brackets, from the default build.

| Engine | MPC | Budget |
| --- | --- | --- |
| Poly, 8 voices (Init, Pluck, Brass) | 7 to 8 % (17 to 18 %) | synth 20 % |
| Poly, 8 voices (Supersaw, Dust Pad) | 9 to 11 % (23 %) | synth 20 % |
| Poly, mono patches (Grit Bass, Acid, Sub Bass, Lead) | 1.2 to 1.6 % (6 %) | synth 20 % |
| Poly Init, 16 voices, unison 4 | 19 % (36 %) | synth 20 % |
| Sampler, 20 pads, 24 voices | **50 % (126 %)** | sampler 25 % |
| Sampler + master grains | **72 % (163 %)** | sampler 25 % |
| Cloud reverb: Plate / Room / Hall / Cloud / Echoes | **40 / 27 / 38 / 56 / 12 %** | fx 10 % |
| Grains FX (dense) | **19 % (34 %)** | fx 10 % |
| A 4-slot insert chain (drive, filter, delay, comp) | 7.7 % (12 %) | fx 10 % |
| Single effects | 1 to 6 %, mostly the test signal (see below) | fx 10 % |

What this means in practice:

- **Synths:** one 8-voice Poly track fits on average but takes the whole
  synth budget at p99. Mono patches are cheap. Unison is not.
- **Sampler:** each voice costs about **60 µs on average** (two layers and a
  filter per pad in this case). The 25 % budget holds about 12 voices on
  average and about 5 at p99. Round 7 played fine because its pads were
  short one-shots: the sampler averaged 0.3 % and peaked at 12 %.
- **Reverb and Grains:** any Cloud character except Echoes is over the
  whole fx budget on its own. PR #94's Light quality roughly halves Cloud.
  That still leaves 15 to 25 %.
- **Single effects** were overstated in round 7. Each effect case built its
  own test signal inside the timed block (a `sin` and an `exp` per frame).
  The workstation shows that costs 3 to 4 µs a block, which is roughly
  25 µs on the MPC. So simple filters, Crush and Master safety probably
  cost a few µs there, not 30 to 50. This PR fixes the benchmark: the
  signal is made once and copied, and round 8 will show the real numbers.

A busy song (24 sampler voices, an 8-voice synth, a reverb) does not fit on
the MPC today. The sampler alone is half a period on average.

## 3. The pad player, live

These come from about 10 minutes of pads, Poly Pluck and the looper, then
line input into the looper. There were **0 underruns** in 206,807 periods.

- **The whole period:** mean 3.8 to 5.3 %, p99 22 to 32 %, worst 44 %.
- **Per engine, p99:** sampler 4 to 6 %, synth 2.5 to 4 %, looper 3 to 4 %,
  time 0.6 %, mix 0.6 %. These add up to about 12 %.

**A gap to explain.** The engines' p99s add up to about 12 %, but the
period's p99 is about 29 %. So 15 % or more of the period's tail lies
outside every engine. Candidates:

- ALSA and line-input I/O inside the timed span;
- preemption by interrupts;
- the core's clock dropping when it's idle. At 4 % average load, an
  "ondemand" or "interactive" cpufreq governor would lower the clock, and
  each period would then start slow. This is an assumption; nothing has
  checked the governor yet.

Round 8 should answer this with a read-only look (see section 5).

**The clock costs 1.2 µs per span on the MPC** (two `Instant::now()`
calls), against 68 ns on the workstation. The pad player's six engine
probes cost about 7 µs a period (0.25 %), which is fine. A probe per
insert slot or per voice is not: 16 chain slots would cost about 0.7 % of
every period just to measure. So per-slot and per-voice probes should run
only while their Dev page is open, or time one block in eight.

## 4. Where to cut, in order

1. **Sampler voice cost** (the sampler thread). On the workstation,
   callgrind puts most of the time in `Voice::read` (a 4-tap Hermite read
   per channel, with a clamp and a channel `match` per tap), `finish`
   (envelope, gain, filter) and the render loop. Things to try:
   - specialise the read loop for each channel layout and hoist the bounds
     checks out of the per-frame loop (clamp only near the ends);
   - offer linear interpolation for drums;
   - skip the filter work when the filter is off;
   - keep 64-bit position arithmetic out of the inner loop, because on
     ARMv7 each 64-bit operation takes several instructions.

   The p99 is 2.5× the mean, so voice starts are worth a look too.
2. **Cloud reverb and Grains** (the FX thread). Make Light, or something
   cheaper, the MPC default. Then cut memory traffic: fewer or shorter
   delay lines, or running the tank at half rate.
3. **Synth unison and pads** (the synth thread). Unison 4 costs 2.7× the
   voice. The `--afford` table in round 8 will show the cost per voice.
4. **NEON**, after the above. Stable Rust has no 32-bit ARM NEON
   intrinsics, so this means small C kernels or a different toolchain. That
   is a decision for later.

## 5. For round 8

- **A clean benchmark:** run the benchmark step while the pad player is
  stopped, so p99 and worst are the engines' own. This is the Hello
  thread's runbook.
- **Two new runs:** the benchmark from #79, #90, #91 and #103 adds memory,
  time, looper, the mixer, the whole-song case and `--afford`.
- **Find the missing 15 %:** a read-only check, during play, of
  `/sys/devices/system/cpu/cpu*/cpufreq/scaling_governor` and
  `scaling_cur_freq`, plus `/proc/interrupts` before and after. If the
  governor is the cause, pinning the audio core's frequency is a later,
  state-changing step for Jay to decide.
- **The verdict rule:** the pad player marked looper, time and mix OVER
  because of a single worst period, while their p99s fit. Jay chose
  (2026-11-11): per-engine verdicts on p99.9, and the whole period's
  verdict on its worst block, because the worst block is what underruns.
  `mpfree_prof::budget::judge` applies it.
