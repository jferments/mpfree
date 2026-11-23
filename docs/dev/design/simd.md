<!--
covers: rust/mpfree-simd tools/simd_arm_cost.py
checked: 2026-11-19 65b24e8d
-->
# NEON kernels (`mpfree-simd`)

**Status:** built 2026-11-19 (the synth architecture's CPU work C3): the crate, its first kernel (the supersaw's saws) with its plain Rust twin, the bit-for-bit check under qemu-arm, the ARMv7 instruction count and the bench cases that time both (PR #658); Poly's supersaw runs on it, the bench's `--plain` turns the kernels off, the Dev page says which code runs, a second kernel does the Choir's formant filters, four voices at once (#663), and the Choir engine runs on it (#696); and two more render the divide-down bank of Strings and the combo organ four notes at a time (#686); two more, a GEMM micro-kernel and a half-to-float conversion, run the stem separation's networks ([Stem separation](stems.md), 2026-11-23).  **Nothing here has run on the MPC**: whether the kernel stays is the device round's call.

The decision this builds is in [Synth architecture, NEON (lever 6)](synth-architecture.md#neon-lever-6-the-decision), and why the compiler can't do it for us in [Profiling, NEON](profiling.md#neon).

## Why a crate of hand-written kernels

The MPC One's Cortex-A17 cores have NEON, four `f32` lanes an instruction, and the synths' inner loops are made of exactly the arithmetic NEON does. Stable Rust can't reach it on 32-bit ARM: LLVM won't vectorise float loops for ARMv7 NEON (NEON flushes denormals, so it isn't IEEE exact), and the intrinsics and `#[target_feature(enable = "neon")]` are nightly-only there (checked with Rust 1.97 on 19 Nov). Module-level assembly (`global_asm!`) is stable, and a `.fpu neon` line inside it lets the assembler take NEON instructions on the default target. So:

- **Each kernel is one assembly function** in `rust/mpfree-simd/src/neon.rs`, built only for ARMv7 Linux. It follows the ARM procedure call standard: its arguments come as one pointer to a `#[repr(C)]` block, it saves r4 to r11, and it touches only the NEON registers a call may change (q0 to q3, q8 to q15).
- **Each kernel has a plain Rust twin** (`plain.rs`), written the way the engine's loop was before, so the workstation's sound and cost don't change. The twin is the reference, and the code that runs wherever the kernel doesn't.
- **This crate is the only unsafe code the synths reach.** Every public function is safe: the wrapper checks what the assembly relies on (every table index in bounds, every slice as long as the counts it passes) before the call, and runs the twin when a call doesn't fit. `mpfree-synth` keeps `#![forbid(unsafe_code)]`.

## Which code runs

Each kernel (`table_sum`, `formants4`, `waves4`, `mix4`, `gemm4x8`, `f16_to_f32`) takes NEON when all three hold, and the twin otherwise:

1. the program is an ARMv7 Linux build;
2. the processor has NEON, asked of Linux once with `getauxval(AT_HWCAP)` (bit 12, `HWCAP_NEON`); the MPC's cores and qemu-arm both say yes;
3. the kernels are on: `mpfree_simd::set_enabled` (on by default), for the device A/B and as a way out if a kernel ever misbehaves.

`f16_to_f32` also needs VFPv4, for its half-precision conversion: asked of Linux the same way (bit 16, `HWCAP_VFPv4`). The MPC's cores report it (`vfpv4` in their cpuinfo, [ABI survey](../hardware/abi-handoff-survey.md)), and so does qemu-arm.

`path()` says which runs now and `status()` says why in a few words ("NEON", "plain Rust: NEON turned off", "plain Rust: this processor has no NEON", "plain Rust: not an ARMv7 build"). The Dev page's **NEON kernels** row shows it, as the audio thread starts (with the Denormals row), green for NEON and amber for an ARM build without it; `mpfree-bench` prints it at the top of its run, and `mpfree-bench --plain` turns the kernels off for the whole run (any mode), for the device A/B. `table_sum_on`, `formants4_on`, `waves4_on`, `mix4_on`, `gemm4x8_on` and `f16_to_f32_on` run a chosen path and say which ran, for the bench and the tests. A call that asked for NEON on a processor with it but ran the twin because its table didn't fit is counted (`fallbacks()`), so a fallback is never silent; it stays 0 unless a caller is wrong.

## The first kernel: several oscillators reading one table

`table_sum::<ADD>(table, shift, buf, ph, inc, gain)` adds oscillators that read one level of a band-limited wave into `buf`: the supersaw's seven saws. Poly reads them through `osc.rs`'s `Classic::supersaw`, which works out each saw's increment and gain and makes this one call (the shape agreed with the synth library thread on 19 Nov); its old saw-by-saw loop is the twin, so the workstation's samples and cost are unchanged.

- **The table** is one level of `osc.rs`'s `Waveset`: each entry a sample and its step to the next one, the step scaled to the phase bits below the index. Its length is 2^(32 − shift), so the top bits of a 32-bit phase index it.
- **Oscillator k** starts at `ph[k]`, moves `inc[k]` a frame and wraps; its sample is `a + step × (p & mask)` for the entry `[a, step]` at `p >> shift`, times `gain[k]`. There are as many oscillators as the shortest slice, so seven saws, or a voice's 4 × 7 with unison, are one call.
- **Writing or adding:** with `ADD` false the first oscillator writes `0.0 + x` (so never −0.0) and the rest add; with `ADD` true each adds to `buf`. Each frame sums the oscillators in their order, so a block of any size gives the same samples.

**How the kernel does it.** Oscillator by oscillator, four frames a round, as the twin goes oscillator by oscillator and frame by frame:

1. The four frames' phases sit in one register (p, p + i, p + 2i, p + 3i) and step by 4i a round.
2. Each frame's entry address is worked out in NEON (`(p >> shift) × 8` plus the table's address) and moved to four core registers, since NEON has no gather; four 8-byte loads fetch the `[a, step]` pairs and one `VUZP` splits them into four samples and four steps.
3. The bits below the index become floats (`VCVT.F32.U32`), and `VMLA.F32` gives `a + step × frac`, then `buf + v × gain` (or `0 + v × gain` for the first oscillator when writing).

The frames after the last whole group of four, and calls with fewer than four, go through the twin, which continues from the phases the kernel left.

## The second kernel: four voices' formant filters

`formants4(src, out, &mut f)` runs four voices' source through their five formant filters each and sums them: the Choir's formants (shape and operations agreed with the Choir thread on 19 Nov). A formant filter is a recurrence over frames, so frames can't share a vector; four voices' copies of the same formant can, so the lanes go across voices.

- **The data** is rows of four, one voice a lane: `src[frame][lane]` and `out[frame][lane]`, and `Formants4`, whose fields `s1`, `s2` (each filter's state), `a1`, `a2`, `a3` (its coefficients) and `g` (the signed gain its output is summed with) are each `[formant][lane]`. `Formants4` is `#[repr(C)]` and its field offsets are the kernel's (a test checks them). The Choir sets the coefficients every 16-frame control step, so a call is at most 16 frames; any length works.
- **The filter** is Simper's zero-delay state-variable band-pass (`a1 = 1 / (1 + g (g + k))`, `a2 = g a1`, `a3 = g a2`, from `g = tan(pi fc / fs)` and `k = 1 / Q`). Per frame and lane, formant 0 first: `v3 = e - s2`, `v1 = a1 s1 + a2 v3` (its output), `v2 = (s2 + a2 s1) + a3 v3`, `s1 = 2 v1 - s1`, `s2 = 2 v2 - s2`; the sum starts as `0 + g0 v1` and adds `g v1` for formants 1 to 4. A voice that isn't playing is a lane of zeros.

**How the kernel does it.** Formant by formant: one formant's state and coefficients stay in six NEON registers while every frame goes through it, four voices a vector, then its state is stored. The first formant writes each frame's `0 + g v1`; the other four load the frame's sum, add their `g v1` and store it. Each lane does the twin's operations in the twin's order, and the formants share no state, so going formant by formant instead of frame by frame changes no bit. There is no tail: any number of frames goes through the kernel.

**Bit for bit,** as for the first kernel: only non-fused `VMLA.F32`, `VMUL`, `VADD` and `VSUB`, the sum started from +0 as the twin's is (so a silent formant gives +0, never -0), and flush-to-zero on the sound threads for denormals. A filter dying away under flush-to-zero can end up cycling at the smallest normal numbers (about 1e-37, some 700 dB down) instead of reaching zero; both paths do the same.

**Bit for bit.** The kernel does the twin's arithmetic in the twin's order. ARMv7's `VMLA.F32` rounds the product before it adds (it isn't fused, unlike `VFMA`), as Rust's `a + b * c` does; `VCVT.F32.U32` rounds to nearest, as `as f32` does, and the values it converts are below 2^24 and so exact anyway. NEON always flushes denormals to zero; the app's sound threads turn flush-to-zero on for the scalar unit too ([Synth architecture, Denormals](synth-architecture.md#flatter-worst-periods-lever-5)), so there both paths flush the same values, and away from denormals they agree whatever the mode.

## The third and fourth kernels: the divide-down bank, four notes at a time

The bank (`mpfree_synth::bank::Bank`, shared by Strings and Drawbar's combo organ) renders every note some key uses: one band-limited oscillator a note, its wave added into the buses (Strings' six footage halves, the combo's flute and reed) along each bus's level ramp across the 16-frame control step. As ARMv7 code its waves and that mixing were about 28 % of Strings with 6 keys (profiled under qemu by source line on 19 Nov: the Ensemble is another 37 %, the registers' filters 10 %). A note's wave is a recurrence only in its phase, and its level on a bus a recurrence across frames, so the lanes go across notes, four at a time:

- **`waves4(shape, osc, out, flags)`**: four oscillators' waves (`Osc4`: each lane's phase, step and step as a share of a cycle), `out[frame][lane]`, without the four-point step correction: the saw `t + t - 1`, the square ±1, the cubic `t (t - 1/2) (1 - t)` times its scale. The step correction has five branches and a division, which NEON can't do to the bit, so the kernel flags instead: `flags[frame]` has bit `lane` set where the correction may not be zero (`dt >= 0.25`, or `t` within `2 dt` of the step, or for the square `t` or `h`, half a cycle on). The bank then works those samples out again in full with `dsp::poly_blep4`, exactly as before; most frames have no flag.
- **`mix4(waves, amp, damp, buses)`**: for each bus and frame, the four oscillators add `wave × level` in lane order, the level stepping on after each frame; an oscillator whose level and step are both zero on a bus is skipped there. The kernel works bus by bus, four frames a round: the four frames' products, then the 4 × 4 block turned over (two `VTRN`, two `VSWP`) so the four notes add into the four frames as vectors, still note by note. A skipped note's products become -0 (adding -0 leaves any sum as it is, as skipping does), and its level is put back as it was.

`Bank::render` runs this where NEON runs (`render4`): four notes at a time in the order they are heard, so each bus gets the notes in the same order as before and the samples are the same to the bit; the one to three notes left over go one at a time. Everywhere else, and with the kernels off, it keeps its note-at-a-time loop (`render1`), the reference: the workstation's cost is unchanged (CI's instruction gate) and `mpfree-bench --plain` on the MPC times the bank exactly as before. With fewer than four notes heard it stays on that loop, which costs less there. `render4` is built only for ARM (and for the tests, which run it everywhere): on the workstation its mere presence, though never called, changed how the compiler inlined `render1` and cost the combo organ 1.2 %, and without it the workstation builds exactly the old loop.

## The fifth and sixth kernels: the stem separation's GEMM

`mpfree-stems` runs Spleeter's U-Nets as GEMMs, about 24 to 37 G multiply-adds for 12 s of sound, so its inner loop is the one place a kernel pays off most.

- **`gemm4x8(kc, a, b, acc)`**: over `kc` steps, `acc[r][j] += a[k][r] * b[k][j]` for a packed panel of four rows (`a`, `kc × 4`) and one of eight columns (`b`, `kc × 8`), into a 4 × 8 block (`acc`, row by row). The kernel keeps the block's 32 sums in q8 to q15; each step loads the four `a` values and the eight `b` values and does eight `VMLA.F32` by scalar. Each sum gets its products in the twin's order, and `VMLA.F32` rounds the product before it adds, as Rust's `c + a * b` does, so the block is the twin's to the bit. A call with `a` or `b` shorter than `kc` steps runs the twin, which panics as a slice index does.
- **`f16_to_f32(src, dst)`**: IEEE halves (as bits) to f32, as many as the shorter slice holds: exact for zeros, subnormals, normals and infinities, and a NaN becomes the default NaN (`0x7fc00000`), as NEON makes it. The kernel does eight a step with two `VCVT.F32.F16` (built with `.fpu neon-vfpv4`); the rest go through the twin. The separation keeps its weights as halves and unpacks each layer with it as the layer runs.

The weights and activations are far from the denormal range, so flush-to-zero doesn't come into it: the job's threads don't turn it on, and the tests use values whose products stay normal.

## How it's checked

`cargo test --target armv7-unknown-linux-gnueabihf -p mpfree-simd` under qemu-arm (CI's `rust-arm` job, "NEON kernels against their plain twins"):

- `neon_is_there`: qemu-arm reports NEON, so the other tests run the kernel and can't pass by running the twin twice.
- 3,000 random calls on both paths, compared bit for bit, samples and phases: tables of 8 to 4,096 entries, 1 to 9 oscillators, 0 to 69 frames, phases about to wrap, increments of 0 and backwards, gains of 0, writing and adding.
- Every level of a real band-limited saw at Poly's sizes (seven saws, 16 frames), and the bench's whole workload for 200 blocks.
- Values so small that their products are denormal, at the smallest and largest tables and with gains near zero, with flush-to-zero on: the same samples. With it off, that test fails, as it should.
- A table whose length isn't a power of two runs the twin and is counted.
- The formant kernel: 3,000 random calls on both paths (0 to 16 frames, the Choir's steps, and now and then 17 to 116), filters from 50 Hz to 0.45 of the rate with Q from 0.5 to 50, ringing state, signed gains, silent lanes and gains of -0: samples, state and the untouched rest of `out` the same to the bit; a voice dying away for 4,000 steps of silence with flush-to-zero on; and the Choir workload for 200 blocks.
- The bank's kernels: `waves4` against its twin for 2,000 random sets of four oscillators (steps from 0 to past a quarter of a cycle, phases about to wrap or cross half) on all three shapes, waves and flags to the bit; `mix4` for 3,000 random calls (1 to 8 buses, 0 to 16 frames, notes heard or not, buses and levels at -0); and tiny levels with flush-to-zero on.
- The GEMM kernel: 400 calls on both paths (0 to 600 steps, the panels longer than the steps need, as slices of a bigger panel are) and one of a real layer's size (12,800 steps, weights the size of Spleeter's): the block the same to the bit. The half kernel on all 65,536 halves, at every start within a group of eight and with tails of 0, 1 and 7 for the twin: the same bits; `vfpv4_is_there_for_the_half_kernel` checks qemu-arm reports VFPv4, so those tests run the kernel. The twins' own: the GEMM's against an f64 sum, the half's against each half's IEEE value worked out by hand.
- Poly end to end (`mpfree-synth/tests/simd.rs`, run as ARMv7 in the same CI step): every Super preset, and both oscillators on Supersaw with three unison copies (so the second oscillator adds to the first), played for two seconds in uneven blocks with the kernels on and then off: the same samples, and no call fell back. `osc.rs`'s own test checks `Classic::supersaw` against its old loop. The same test plays all 26 Strings presets and the eight Combo presets with the kernels on and off, and all 25 Choir presets with the formant kernel on and off (flush-to-zero on, as on the sound threads): the same samples. The Choir's golden renders in the bench didn't move when it went onto `mpfree_simd::formants4`. `bank.rs`'s own test runs four notes at a time against one at a time (on the kernels or their twins) for 300 steps of chords of 1 to 40 notes, steps of 1 to 16 frames, on Strings' six saw buses and on mixed shapes: the same samples, phases and levels.

To be sure the check bites, the kernel was changed by one instruction (a fused `VFMA` in place of the `VMLA` that reads the table, a difference of one rounding): four of the tests failed. The formant kernel was changed three ways (a fused `VFMA` for the sum, its two multiply-adds swapped, and `g v1` in place of `0 + g v1`): each made two to four tests fail. The bank's kernels were changed six ways (one skip mask dropped, two notes added out of order, a level stepped before its first use, skipped levels not put back, the saw's flag edge moved, the cubic multiplied in another order): each made one or two tests fail. The GEMM kernel was changed two ways (one row's multiply-add reading another row's `a`, and the second half of `b` read without stepping on) and the half kernel one (the second four converted from the first four's register): each made two or three tests fail.

The twins' own tests run on the workstation too: the saws' twin reads each table linearly and sums in order (against an f64 sum), writes from +0 and adds to what's there, gives the same samples for any block size, and leaves an empty call alone; the waves' twin flags every frame the step correction touches (for 400 random sets on all three shapes, its wave plus the correction where flagged is the bank's old formula to the bit); the mix's twin is the bank's note-by-note loop to the bit; the formants' twin is five band-passes summed (against the filter written out in f64), passes a sine at a formant's centre at its level and one three octaves up at under 3 %, gives the same samples in any block size, and plays as many frames as the shorter slice.

## What it costs

| Measure | Plain twin | NEON kernel |
|---|---|---|
| ARMv7 instructions a 128-frame block, 8 voices' supersaw saws with their mixing, under qemu (`tools/simd_arm_cost.py`) | 134,455 | 58,361 (43 %) |
| x86-64 instructions a block, the same case (`instr_gate.py`) | 133,524 | (not on x86) |
| Time on the MPC (`mpfree-bench`, "Supersaw saws: 8 voices, plain" against "..., NEON") | device round | device round |
| ARMv7 instructions a 128-frame block, 8 Choir voices' formants with their source and mixing, under qemu | 126,280 | 40,486 (32 %) |
| x86-64 instructions a block, the same case | 39,041 | (not on x86) |
| Time on the MPC ("Choir formants: 8 voices, plain" against "..., NEON") | device round | device round |
| ARMv7 instructions a block, the Choir engine: Init with 1 and 6 keys (three singers each), Big Choir with 8 (eight singers), under qemu, with the kernel off and on (`tools/inst_arm_cost.py --plain`, then without) | 249.1k, 445.2k, 748.8k | 206.3k (83 %), 359.5k (81 %), 663.2k (89 %) |
| x86-64 instructions a block, those cases (`instr_gate.py`) | unchanged (the twin is the engine's old loop) | (not on x86) |
| ARMv7 instructions a block, Strings Init with 1, 6 and 49 keys, under qemu, with the kernels off and on (`tools/inst_arm_cost.py --plain`, then without) | 130.6k, 186.1k, 327.4k | 130.7k (flat), 161.8k (87 %), 247.9k (76 %) |
| The same, Strings Orchestra with 6 keys, Drawbar Combo and Combo Reed with 8 | 237.2k, 193.8k, 207.4k | 198.8k (84 %), 138.3k (71 %), 151.4k (73 %) |
| x86-64 instructions a block, those cases (`instr_gate.py`) | unchanged (the note-at-a-time loop runs) | (not on x86) |
| Time on the MPC (those cases with and without `mpfree-bench --plain`) | device round | device round |

The twin's ARMv7 loop takes about 16 instructions a saw a frame (two loads, the index and its bounds check, the conversion, two multiply-adds, a load and a store of the frame, and values reloaded from the stack); the kernel takes about 20 for four frames. **Fewer instructions don't prove it is faster**: NEON's lane moves to core registers, its loads and `VUZP` take more cycles each than a plain add, and the A17 runs NEON and VFP in one pipeline. The device round decides; see below.

The bench's workloads play what an engine gives a kernel in a busy song. `mpfree_simd::workload::Supersaw` is Poly's: eight voices of a chord over three octaves from C2, each voice's seven saws on one saw level, one call a voice for each 16-frame control step, added into left and right. `workload::Formants` is the Choir's: eight voices of a chord from C3 in two groups of four, each voice a plain saw through five formants (a soprano's "ah" for one group, an alto's "oh" for the other), one call a group for each step; its coefficients are set once, since the engine's own control-rate work is the same on both paths. The bench has each as two cases, "..., plain" everywhere and "..., NEON" on a processor with NEON, and they give the same samples. The instruction count builds `examples/simd_cost.rs` for ARMv7 and counts each path under `qemu-arm -d exec`, with N blocks and with none (the method of `tools/inst_arm_cost.py`); it reads plain ramps the size of the band-limited levels, since the count is the same and building those would take most of a traced run. It fails if the two paths' samples differ, if the NEON run ran the twin, and (with `--gate`, as CI runs it) if NEON doesn't take fewer instructions than plain.

## The rule: a kernel stays only if it wins

The device round runs `mpfree-bench` on the MPC as it does today; the two "Supersaw saws" cases and the two "Choir formants" cases are its A/Bs. Each kernel stays if its p99 block is under its twin's; if not, it is removed and the twin stays (`synth-quality.md`, rows N3 and N7). The same round runs the bench again with `--plain` and compares the Supersaw presets' cases (`--compare`), the whole of Poly with the kernel and without, the Strings and Combo cases, the bank's A/B (row N10): the bank's kernels stay only if those are faster with them, and the three Choir cases, the formant kernel inside the whole engine (row N7). The ask is Ask 5 in the synth library's device asks (project files, `synth-library/device-round-asks.md`).

The stem separation's two kernels have their own A/B in `mpfree-bench --stems`: its 1024-bin run with the kernels on and with them off (both on cores 0 and 1). They stay if the separation is faster with them.

If the kernel wins by little, the next things to try are two groups of four frames a round (so the loads of one overlap the arithmetic of the other) and working the addresses out in core registers beside the NEON work.

## Adding a kernel

1. Write the twin in `plain.rs` the way the engine's loop is now, and move the engine onto it (the sound and the workstation's cost don't change). Where the kernel needs the loop laid out differently and that would cost the workstation more, keep the engine's loop as the reference and run the new layout only where NEON runs, as the bank does (`render1` and `render4`), build the new layout for ARM and the tests only (`#[cfg(any(target_arch = "arm", test))]`) so the workstation's loop compiles as before, and test the two against each other.
2. Write the kernel in `neon.rs` doing the twin's operations in the twin's order, lanes across frames; never a fused multiply-add, never a reordered sum.
3. Give it a safe wrapper that checks every bound the assembly relies on and returns what it didn't do, for the twin.
4. Test it against the twin under qemu: random shapes, the engine's real sizes, the bench workload, denormals with flush-to-zero on.
5. Add its workload, a case in `examples/simd_cost.rs` and `tools/simd_arm_cost.py`, and the bench's two cases; add its timing to the device asks.

**Next:** the Ensemble (`mpfree_fx::Ensemble`) is now the largest part of Strings as ARMv7 code; it isn't a synth engine's loop and is outside this crate's brief so far.
