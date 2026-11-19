<!--
covers: rust/mpfree-simd tools/simd_arm_cost.py
checked: 2026-11-19 79fc6f63
-->
# NEON kernels (`mpfree-simd`)

**Status:** built 2026-11-19 (the synth architecture's CPU work C3, first part): the crate, its first kernel (the supersaw's saws) with its plain Rust twin, the bit-for-bit check under qemu-arm, the ARMv7 instruction count and the bench cases that time both. **No engine uses it yet**: Poly's supersaw moves onto it in the next pull request, once `osc.rs` (PR #652) is on main. **Nothing here has run on the MPC**: whether the kernel stays is the device round's call.

The decision this builds is in [Synth architecture, NEON (lever 6)](synth-architecture.md#neon-lever-6-the-decision), and why the compiler can't do it for us in [Profiling, NEON](profiling.md#neon).

## Why a crate of hand-written kernels

The MPC One's Cortex-A17 cores have NEON, four `f32` lanes an instruction, and the synths' inner loops are made of exactly the arithmetic NEON does. Stable Rust can't reach it on 32-bit ARM: LLVM won't vectorise float loops for ARMv7 NEON (NEON flushes denormals, so it isn't IEEE exact), and the intrinsics and `#[target_feature(enable = "neon")]` are nightly-only there (checked with Rust 1.97 on 19 Nov). Module-level assembly (`global_asm!`) is stable, and a `.fpu neon` line inside it lets the assembler take NEON instructions on the default target. So:

- **Each kernel is one assembly function** in `rust/mpfree-simd/src/neon.rs`, built only for ARMv7 Linux. It follows the ARM procedure call standard: its arguments come as one pointer to a `#[repr(C)]` block, it saves r4 to r11, and it touches only the NEON registers a call may change (q0 to q3, q8 to q15).
- **Each kernel has a plain Rust twin** (`plain.rs`), written the way the engine's loop was before, so the workstation's sound and cost don't change. The twin is the reference, and the code that runs wherever the kernel doesn't.
- **This crate is the only unsafe code the synths reach.** Every public function is safe: the wrapper checks what the assembly relies on (every table index in bounds, every slice as long as the counts it passes) before the call, and runs the twin when a call doesn't fit. `mpfree-synth` keeps `#![forbid(unsafe_code)]`.

## Which code runs

`table_sum` (and each later kernel) takes the NEON kernel when all three hold, and the twin otherwise:

1. the program is an ARMv7 Linux build;
2. the processor has NEON, asked of Linux once with `getauxval(AT_HWCAP)` (bit 12, `HWCAP_NEON`); the MPC's cores and qemu-arm both say yes;
3. the kernels are on: `mpfree_simd::set_enabled` (on by default), for the device A/B and as a way out if a kernel ever misbehaves.

`path()` says which runs now and `status()` says why in a few words ("NEON", "plain Rust: NEON turned off", "plain Rust: this processor has no NEON", "plain Rust: not an ARMv7 build"), for the Dev page when an engine uses it. `table_sum_on` runs a chosen path and says which ran, for the bench and the tests. A call that asked for NEON on a processor with it but ran the twin because its table didn't fit is counted (`fallbacks()`), so a fallback is never silent; it stays 0 unless a caller is wrong.

## The first kernel: several oscillators reading one table

`table_sum::<ADD>(table, shift, buf, ph, inc, gain)` adds oscillators that read one level of a band-limited wave into `buf`: the supersaw's seven saws, which Poly reads through `osc.rs`'s `Classic::supersaw` (agreed with the synth library thread on 19 Nov).

- **The table** is one level of `osc.rs`'s `Waveset`: each entry a sample and its step to the next one, the step scaled to the phase bits below the index. Its length is 2^(32 − shift), so the top bits of a 32-bit phase index it.
- **Oscillator k** starts at `ph[k]`, moves `inc[k]` a frame and wraps; its sample is `a + step × (p & mask)` for the entry `[a, step]` at `p >> shift`, times `gain[k]`. There are as many oscillators as the shortest slice, so seven saws, or a voice's 4 × 7 with unison, are one call.
- **Writing or adding:** with `ADD` false the first oscillator writes `0.0 + x` (so never −0.0) and the rest add; with `ADD` true each adds to `buf`. Each frame sums the oscillators in their order, so a block of any size gives the same samples.

**How the kernel does it.** Oscillator by oscillator, four frames a round, as the twin goes oscillator by oscillator and frame by frame:

1. The four frames' phases sit in one register (p, p + i, p + 2i, p + 3i) and step by 4i a round.
2. Each frame's entry address is worked out in NEON (`(p >> shift) × 8` plus the table's address) and moved to four core registers, since NEON has no gather; four 8-byte loads fetch the `[a, step]` pairs and one `VUZP` splits them into four samples and four steps.
3. The bits below the index become floats (`VCVT.F32.U32`), and `VMLA.F32` gives `a + step × frac`, then `buf + v × gain` (or `0 + v × gain` for the first oscillator when writing).

The frames after the last whole group of four, and calls with fewer than four, go through the twin, which continues from the phases the kernel left.

**Bit for bit.** The kernel does the twin's arithmetic in the twin's order. ARMv7's `VMLA.F32` rounds the product before it adds (it isn't fused, unlike `VFMA`), as Rust's `a + b * c` does; `VCVT.F32.U32` rounds to nearest, as `as f32` does, and the values it converts are below 2^24 and so exact anyway. NEON always flushes denormals to zero; the app's sound threads turn flush-to-zero on for the scalar unit too ([Synth architecture, Denormals](synth-architecture.md#flatter-worst-periods-lever-5)), so there both paths flush the same values, and away from denormals they agree whatever the mode.

## How it's checked

`cargo test --target armv7-unknown-linux-gnueabihf -p mpfree-simd` under qemu-arm (CI's `rust-arm` job, "NEON kernels against their plain twins"):

- `neon_is_there`: qemu-arm reports NEON, so the other tests run the kernel and can't pass by running the twin twice.
- 3,000 random calls on both paths, compared bit for bit, samples and phases: tables of 8 to 4,096 entries, 1 to 9 oscillators, 0 to 69 frames, phases about to wrap, increments of 0 and backwards, gains of 0, writing and adding.
- Every level of a real band-limited saw at Poly's sizes (seven saws, 16 frames), and the bench's whole workload for 200 blocks.
- Values so small that their products are denormal, at the smallest and largest tables and with gains near zero, with flush-to-zero on: the same samples. With it off, that test fails, as it should.
- A table whose length isn't a power of two runs the twin and is counted.

To be sure the check bites, the kernel was changed by one instruction (a fused `VFMA` in place of the `VMLA` that reads the table, a difference of one rounding): four of the tests failed.

The twin's own tests run on the workstation too: it reads each table linearly and sums in order (against an f64 sum), writes from +0 and adds to what's there, gives the same samples for any block size, and leaves an empty call alone.

## What it costs

| Measure | Plain twin | NEON kernel |
|---|---|---|
| ARMv7 instructions a 128-frame block, 8 voices' supersaw saws with their mixing, under qemu (`tools/simd_arm_cost.py`) | 134,455 | 58,361 (43 %) |
| x86-64 instructions a block, the same case (`instr_gate.py`) | 133,524 | (not on x86) |
| Time on the MPC (`mpfree-bench`, "Supersaw saws: 8 voices, plain" against "..., NEON") | device round | device round |

The twin's ARMv7 loop takes about 16 instructions a saw a frame (two loads, the index and its bounds check, the conversion, two multiply-adds, a load and a store of the frame, and values reloaded from the stack); the kernel takes about 20 for four frames. **Fewer instructions don't prove it is faster**: NEON's lane moves to core registers, its loads and `VUZP` take more cycles each than a plain add, and the A17 runs NEON and VFP in one pipeline. The device round decides; see below.

The bench's workload (`mpfree_simd::workload::Supersaw`) plays what Poly gives the kernel in a busy song: eight voices of a chord over three octaves from C2, each voice's seven saws on one saw level, one call a voice for each 16-frame control step, added into left and right. The bench has it as two cases, "Supersaw saws: 8 voices, plain" everywhere and "..., NEON" on a processor with NEON, and they give the same samples. The instruction count builds `examples/simd_cost.rs` for ARMv7 and counts each path under `qemu-arm -d exec`, with N blocks and with none (the method of `tools/inst_arm_cost.py`); it reads plain ramps the size of the band-limited levels, since the count is the same and building those would take most of a traced run. It fails if the two paths' samples differ, if the NEON run ran the twin, and (with `--gate`, as CI runs it) if NEON doesn't take fewer instructions than plain.

## The rule: a kernel stays only if it wins

The device round runs `mpfree-bench` on the MPC as it does today; the two "Supersaw saws" cases are its A/B. The kernel stays if its p99 block is under the twin's; if not, it is removed and the twin stays (`synth-quality.md`, row N3). Once Poly uses it, the same round compares the whole "Poly Supersaw" case with the kernels on and off. The ask is in the synth library's device asks (project files, `synth-library/device-round-asks.md`).

If the kernel wins by little, the next things to try are two groups of four frames a round (so the loads of one overlap the arithmetic of the other) and working the addresses out in core registers beside the NEON work.

## Adding a kernel

1. Write the twin in `plain.rs` the way the engine's loop is now, and move the engine onto it (the sound and the workstation's cost don't change).
2. Write the kernel in `neon.rs` doing the twin's operations in the twin's order, lanes across frames; never a fused multiply-add, never a reordered sum.
3. Give it a safe wrapper that checks every bound the assembly relies on and returns what it didn't do, for the twin.
4. Test it against the twin under qemu: random shapes, the engine's real sizes, the bench workload, denormals with flush-to-zero on.
5. Add its workload, a case in `examples/simd_cost.rs` and `tools/simd_arm_cost.py`, and the bench's two cases; add its timing to the device asks.

**Next kernels:** the shared divide-down bank's oscillators (`Bank::render`, as the combo organ's pull request moves Strings' bank into `mpfree-synth/src/bank.rs`), and Choir's five formants a voice, with the Choir thread (its voices' values are laid out four at a time for this).
