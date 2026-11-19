//! The NEON kernels, in ARMv7 assembly, and the safe calls into them.
//!
//! Each kernel is one function in `global_asm!` (stable Rust), assembled
//! with `.fpu neon` because the armv7-unknown-linux-gnueabihf target leaves
//! NEON off for the compiler. It follows the procedure call standard: its
//! arguments come as one pointer to a `#[repr(C)]` block, it saves r4 to
//! r11, and it uses only the NEON registers a call may change (q0 to q3
//! and q8 to q15), never q4 to q7.
//!
//! The safe wrapper beside each kernel checks everything the assembly
//! relies on (that every table index is in bounds, that each slice is as
//! long as the counts it passes) before calling it.
//!
//! Arithmetic matches the twins' to the bit: `VMLA.F32` on ARMv7 rounds the
//! product before it adds (it isn't fused, unlike `VFMA`), as Rust's
//! `a + b * c` does, and `VCVT.F32.U32` rounds to nearest, as `as f32`
//! does (and the values converted here are exact anyway). NEON flushes
//! denormals to zero whatever FPSCR says; see the crate's notes.

#![allow(unsafe_code)]

use core::arch::global_asm;

/// HWCAP_NEON in Linux's 32-bit ARM `hwcap.h`.
const HWCAP_NEON: libc::c_ulong = 1 << 12;

/// Whether this processor runs NEON, as Linux tells the program.
pub(crate) fn detect() -> bool {
    // SAFETY: getauxval only reads the auxiliary vector Linux gave the
    // program at start; it takes no pointers and can't fail unsafely.
    let hw = unsafe { libc::getauxval(libc::AT_HWCAP) };
    hw & HWCAP_NEON != 0
}

/// The block `mpfree_simd_table_sum_neon` reads its arguments from (the
/// offsets are in the assembly's comments).
#[repr(C)]
struct TableSum {
    /// 0: the frames, `frames` of them.
    buf: *mut f32,
    /// 4: a multiple of 4, at least 4.
    frames: u32,
    /// 8: the level, 2^(32 - shift) entries.
    table: *const [f32; 2],
    /// 12: 3 to 31.
    shift: u32,
    /// 16, 20, 24: `count` phases (read and written), increments, gains.
    ph: *mut u32,
    inc: *const u32,
    gain: *const f32,
    /// 28: at least 1.
    count: u32,
    /// 32: 0 if the first oscillator writes, else every one adds.
    add: u32,
}

extern "C" {
    fn mpfree_simd_table_sum_neon(args: *const TableSum);
}

// For each oscillator: the phases of four frames in q8 (p, p+i, p+2i,
// p+3i), stepped 4i each round. A round reads the four table entries
// (their addresses worked out in NEON, moved to core registers for the
// loads, since NEON has no gather), splits them into samples and steps,
// and does v = a + step * frac, then buf = buf + v * gain (or 0 + v * gain
// for the first oscillator when writing): each frame in the twin's order.
global_asm!(
    ".pushsection .text.mpfree_simd_table_sum_neon,\"ax\",%progbits",
    ".fpu neon",
    ".arm",
    ".p2align 4",
    ".globl mpfree_simd_table_sum_neon",
    ".hidden mpfree_simd_table_sum_neon",
    ".type mpfree_simd_table_sum_neon,%function",
    "mpfree_simd_table_sum_neon:",
    "push {{r4-r11, lr}}",
    "ldr r1, [r0, #4]",  // frames
    "ldr r2, [r0, #8]",  // table
    "ldr r3, [r0, #12]", // shift
    "ldr r4, [r0, #16]", // phases
    "ldr r5, [r0, #20]", // increments
    "ldr r6, [r0, #24]", // gains
    "ldr r7, [r0, #28]", // oscillators
    "ldr r8, [r0, #32]", // add
    "ldr r0, [r0]",      // buf
    "rsb r9, r3, #3",    // 3 - shift: VSHL by a negative count shifts right
    "vdup.32 q10, r9",
    "mov r9, #1",
    "lsl r9, r9, r3",
    "sub r9, r9, #1", // the mask of the bits below the index
    "vdup.32 q11, r9",
    "vdup.32 q12, r2", // the table's address
    // Each oscillator.
    "1:",
    "ldr r9, [r4]",      // p
    "ldr r10, [r5], #4", // i
    "add r11, r9, r10",
    "vmov d16, r9, r11", // q8 = p, p + i,
    "add r9, r11, r10",
    "add r11, r9, r10",
    "vmov d17, r9, r11", //      p + 2i, p + 3i
    "lsl r10, r10, #2",
    "vdup.32 q9, r10",                 // 4i
    "vld1.32 {{d26[], d27[]}}, [r6]!", // q13 = the gain in every lane
    "mov r12, r0",
    "mov lr, r1",
    "cmp r8, #0",
    "beq 3f",
    // Adding: buf = buf + v * gain, four frames a round.
    "2:",
    "vshl.u32 q2, q8, q10",
    "vbic.i32 q2, #7", // (p >> shift) * 8: each entry's offset
    "vadd.i32 q2, q2, q12",
    "vmov r9, r10, d4",
    "vmov r11, r2, d5",
    "vld1.32 {{d0}}, [r9]", // [a, step] of each frame
    "vld1.32 {{d1}}, [r10]",
    "vld1.32 {{d2}}, [r11]",
    "vld1.32 {{d3}}, [r2]",
    "vuzp.32 q0, q1", // q0 = the four a, q1 = the four steps
    "vand q3, q8, q11",
    "vcvt.f32.u32 q3, q3", // the bits below the index, as floats
    "vmla.f32 q0, q1, q3", // v = a + step * frac
    "vld1.32 {{d28-d29}}, [r12]",
    "vmla.f32 q14, q0, q13", // buf + v * gain
    "vst1.32 {{d28-d29}}, [r12]!",
    "vadd.i32 q8, q8, q9",
    "subs lr, lr, #4",
    "bne 2b",
    "b 4f",
    // Writing (the first oscillator): buf = 0 + v * gain.
    "3:",
    "vshl.u32 q2, q8, q10",
    "vbic.i32 q2, #7",
    "vadd.i32 q2, q2, q12",
    "vmov r9, r10, d4",
    "vmov r11, r2, d5",
    "vld1.32 {{d0}}, [r9]",
    "vld1.32 {{d1}}, [r10]",
    "vld1.32 {{d2}}, [r11]",
    "vld1.32 {{d3}}, [r2]",
    "vuzp.32 q0, q1",
    "vand q3, q8, q11",
    "vcvt.f32.u32 q3, q3",
    "vmla.f32 q0, q1, q3",
    "vmov.i32 q14, #0",
    "vmla.f32 q14, q0, q13", // 0 + v * gain
    "vst1.32 {{d28-d29}}, [r12]!",
    "vadd.i32 q8, q8, q9",
    "subs lr, lr, #4",
    "bne 3b",
    "mov r8, #1", // the rest add
    "4:",
    "vst1.32 {{d16[0]}}, [r4]!", // the phase after the last frame
    "subs r7, r7, #1",
    "bne 1b",
    "pop {{r4-r11, pc}}",
    ".size mpfree_simd_table_sum_neon, .-mpfree_simd_table_sum_neon",
    ".popsection",
);

/// [`crate::table_sum`] in NEON for the whole groups of four frames.
/// Returns the frames it did (the rest are the caller's, for the twin),
/// or None when the table's length isn't 2^(32 - shift) for a shift of 3
/// to 31, so the kernel can't be sure of its bounds.
pub(crate) fn table_sum(
    table: &[[f32; 2]],
    shift: u32,
    buf: &mut [f32],
    ph: &mut [u32],
    inc: &[u32],
    gain: &[f32],
    add: bool,
) -> Option<usize> {
    if !(3..=31).contains(&shift) || table.len() != 1usize << (32 - shift) {
        return None;
    }
    let count = ph.len().min(inc.len()).min(gain.len());
    let frames = buf.len() & !3;
    let (Ok(count32), Ok(frames32)) = (u32::try_from(count), u32::try_from(frames)) else {
        return Some(0);
    };
    if count == 0 || frames == 0 {
        return Some(0);
    }
    let args = TableSum {
        buf: buf.as_mut_ptr(),
        frames: frames32,
        table: table.as_ptr(),
        shift,
        ph: ph.as_mut_ptr(),
        inc: inc.as_ptr(),
        gain: gain.as_ptr(),
        count: count32,
        add: u32::from(add),
    };
    // SAFETY: the kernel reads and writes `frames` floats from `buf`
    // (frames <= buf.len()), reads and writes `count` phases and reads
    // `count` increments and gains (count <= each slice's length), and
    // reads 8-byte table entries at index p >> shift, which is below
    // 2^(32 - shift) = table.len() for any 32-bit phase. It keeps the
    // registers the calling convention asks it to, uses no stack beyond
    // its own pushes, and the borrows above outlive the call.
    unsafe { mpfree_simd_table_sum_neon(&args) };
    Some(frames)
}

extern "C" {
    fn mpfree_simd_formants4_neon(
        src: *const [f32; 4],
        out: *mut [f32; 4],
        frames: u32,
        f: *mut crate::Formants4,
    );
}

// Formant by formant: its state and coefficients stay in q8 to q13 while
// every frame goes through it, four voices a vector; then its state is
// stored. The first formant writes each frame's sum (0 + g v1), the rest
// add to it. Per lane the operations are the twin's, in its order; and
// formants don't share state, so going formant by formant instead of
// frame by frame changes nothing.
//
// Formants4 offsets: s1 0, s2 80, a1 160, a2 240, a3 320, g 400 (each
// field five rows of 16 bytes).
global_asm!(
    ".pushsection .text.mpfree_simd_formants4_neon,\"ax\",%progbits",
    ".fpu neon",
    ".arm",
    ".p2align 4",
    ".globl mpfree_simd_formants4_neon",
    ".hidden mpfree_simd_formants4_neon",
    ".type mpfree_simd_formants4_neon,%function",
    "mpfree_simd_formants4_neon:",
    // r0 src, r1 out, r2 frames (at least 1), r3 the filters.
    "push {{r4-r6, lr}}",
    "mov r4, #0", // the formant
    "1:",
    "add r5, r3, r4, lsl #4",    // its row of s1
    "vld1.32 {{d16-d17}}, [r5]", // q8 = s1
    "add r6, r5, #80",
    "vld1.32 {{d18-d19}}, [r6]", // q9 = s2
    "add r6, r5, #160",
    "vld1.32 {{d20-d21}}, [r6]", // q10 = a1
    "add r6, r5, #240",
    "vld1.32 {{d22-d23}}, [r6]", // q11 = a2
    "add r6, r5, #320",
    "vld1.32 {{d24-d25}}, [r6]", // q12 = a3
    "add r6, r5, #400",
    "vld1.32 {{d26-d27}}, [r6]", // q13 = g
    "mov r6, r0",
    "mov r12, r1",
    "mov lr, r2",
    "cmp r4, #0",
    "bne 3f",
    // The first formant: y = 0 + g * v1.
    "2:",
    "vld1.32 {{d0-d1}}, [r6]!", // e
    "vsub.f32 q1, q0, q9",      // v3 = e - s2
    "vmul.f32 q2, q10, q8",     // a1 * s1
    "vmla.f32 q2, q11, q1",     // v1 = a1 * s1 + a2 * v3
    "vmov q3, q9",
    "vmla.f32 q3, q11, q8", // s2 + a2 * s1
    "vmla.f32 q3, q12, q1", // v2 = s2 + a2 * s1 + a3 * v3
    "vadd.f32 q14, q2, q2",
    "vsub.f32 q8, q14, q8", // s1 = 2 * v1 - s1
    "vadd.f32 q15, q3, q3",
    "vsub.f32 q9, q15, q9", // s2 = 2 * v2 - s2
    "vmov.i32 q14, #0",
    "vmla.f32 q14, q13, q2", // y = 0 + g * v1
    "vst1.32 {{d28-d29}}, [r12]!",
    "subs lr, lr, #1",
    "bne 2b",
    "b 4f",
    // Formants 1 to 4: y = y + g * v1.
    "3:",
    "vld1.32 {{d0-d1}}, [r6]!",
    "vsub.f32 q1, q0, q9",
    "vmul.f32 q2, q10, q8",
    "vmla.f32 q2, q11, q1",
    "vmov q3, q9",
    "vmla.f32 q3, q11, q8",
    "vmla.f32 q3, q12, q1",
    "vadd.f32 q14, q2, q2",
    "vsub.f32 q8, q14, q8",
    "vadd.f32 q15, q3, q3",
    "vsub.f32 q9, q15, q9",
    "vld1.32 {{d28-d29}}, [r12]",
    "vmla.f32 q14, q13, q2", // y + g * v1
    "vst1.32 {{d28-d29}}, [r12]!",
    "subs lr, lr, #1",
    "bne 3b",
    "4:",
    "vst1.32 {{d16-d17}}, [r5]", // the state, for the next block
    "add r6, r5, #80",
    "vst1.32 {{d18-d19}}, [r6]",
    "add r4, r4, #1",
    "cmp r4, #5",
    "bne 1b",
    "pop {{r4-r6, pc}}",
    ".size mpfree_simd_formants4_neon, .-mpfree_simd_formants4_neon",
    ".popsection",
);

/// [`crate::formants4`] in NEON. Returns false, having done nothing, when
/// there is no frame to do (the caller's twin then does the same nothing).
pub(crate) fn formants4(src: &[[f32; 4]], out: &mut [[f32; 4]], f: &mut crate::Formants4) -> bool {
    const _: () = assert!(core::mem::size_of::<crate::Formants4>() == 480);
    let frames = src.len().min(out.len());
    let Ok(frames32) = u32::try_from(frames) else {
        return false;
    };
    if frames == 0 {
        return false;
    }
    // SAFETY: the kernel reads `frames` rows of 16 bytes from `src` and
    // writes as many to `out` (frames <= both lengths), and reads and
    // writes the 480 bytes of `f` (a #[repr(C)] block whose offsets the
    // assembly names, checked by the size assertion above and the
    // field-offset test). It keeps the registers the calling convention
    // asks it to, and the borrows outlive the call.
    unsafe { mpfree_simd_formants4_neon(src.as_ptr(), out.as_mut_ptr(), frames32, f) };
    true
}

#[repr(C)]
struct Waves4 {
    osc: *const crate::Osc4,
    out: *mut [f32; 4],
    flags: *mut u8,
    frames: u32,
    shape: u32,
}

extern "C" {
    fn mpfree_simd_waves4_neon(args: *const Waves4);
    fn mpfree_simd_mix4_neon(args: *const Mix4);
}

// A frame a round, four oscillators a vector: t from the phase's top 24
// bits (VCVT with 24 fraction bits, exact, as the twin's `as f32` times
// 2^-24), the wave, its store, then the lanes near a step as a 4-bit mask
// stored as one byte a frame.
//
// Waves4 offsets: osc 0, out 4, flags 8, frames 12, shape 16. Osc4: ph 0,
// inc 16, dt 32.
global_asm!(
    ".pushsection .text.mpfree_simd_waves4_neon,\"ax\",%progbits",
    ".fpu neon",
    ".arm",
    ".p2align 4",
    ".globl mpfree_simd_waves4_neon",
    ".hidden mpfree_simd_waves4_neon",
    ".type mpfree_simd_waves4_neon,%function",
    "mpfree_simd_waves4_neon:",
    "push {{r4-r7, lr}}",
    "ldr r1, [r0, #0]",           // osc
    "ldr r2, [r0, #4]",           // out
    "ldr r3, [r0, #8]",           // flags
    "ldr r4, [r0, #12]",          // frames, at least 1
    "ldr r5, [r0, #16]",          // shape: 0 saw, 1 square, 2 cubic
    "vld1.32 {{d16-d17}}, [r1]!", // q8 = p
    "vld1.32 {{d18-d19}}, [r1]!", // q9 = inc
    "vld1.32 {{d20-d21}}, [r1]",  // q10 = dt
    "vmov.f32 q15, #1.0",
    "vadd.f32 q11, q10, q10", // two = dt + dt
    "vsub.f32 q12, q15, q11", // 1 - two
    "vmov.f32 q13, #0.25",
    "vcge.f32 q13, q10, q13", // dt >= 0.25: every frame flagged
    "adr r6, 9f",
    "vld1.32 {{d28-d29}}, [r6]", // q14 = lane bits 1, 2, 4, 8
    "vmov.i32 q10, #0x80000000", // half a cycle (dt is done with)
    "cmp r5, #1",
    "beq 3f",
    "bgt 5f",
    // The saw: t + t - 1.
    "2:",
    "vshr.u32 q0, q8, #8",
    "vcvt.f32.u32 q0, q0, #24", // t
    "vadd.f32 q1, q0, q0",
    "vsub.f32 q1, q1, q15", // t + t - 1
    "vst1.32 {{d2-d3}}, [r2]!",
    "vcgt.f32 q2, q11, q0", // t < two
    "vcgt.f32 q3, q0, q12", // t > 1 - two
    "vorr q2, q2, q3",
    "vorr q2, q2, q13",
    "vand q2, q2, q14",
    "vpadd.i32 d4, d4, d5",
    "vpadd.i32 d4, d4, d4",
    "vmov.32 r7, d4[0]",
    "strb r7, [r3], #1",
    "vadd.i32 q8, q8, q9",
    "subs r4, r4, #1",
    "bne 2b",
    "pop {{r4-r7, pc}}",
    // The square: -1 for p < 2^31, else 1; flagged near t's step or h's.
    "3:",
    "vshr.u32 q0, q8, #8",
    "vcvt.f32.u32 q0, q0, #24", // t
    "veor q1, q8, q10",
    "vshr.u32 q1, q1, #8",
    "vcvt.f32.u32 q1, q1, #24", // h, half a cycle on
    "vmvn q3, q8",
    "vand q3, q3, q10",
    "vorr q3, q3, q15", // 1.0 with the sign of p < 2^31
    "vst1.32 {{d6-d7}}, [r2]!",
    "vcgt.f32 q2, q11, q0",
    "vcgt.f32 q3, q0, q12",
    "vorr q2, q2, q3",
    "vcgt.f32 q3, q11, q1",
    "vorr q2, q2, q3",
    "vcgt.f32 q3, q1, q12",
    "vorr q2, q2, q3",
    "vorr q2, q2, q13",
    "vand q2, q2, q14",
    "vpadd.i32 d4, d4, d5",
    "vpadd.i32 d4, d4, d4",
    "vmov.32 r7, d4[0]",
    "strb r7, [r3], #1",
    "vadd.i32 q8, q8, q9",
    "subs r4, r4, #1",
    "bne 3b",
    "pop {{r4-r7, pc}}",
    // The cubic: t (t - 0.5) (1 - t) CUBIC_SCALE, never flagged.
    "5:",
    "vmov.f32 q13, #0.5",
    "movw r7, #0x46e1",
    "movt r7, #0x41a6",
    "vdup.32 q12, r7", // CUBIC_SCALE
    "mov r7, #0",
    "6:",
    "vshr.u32 q0, q8, #8",
    "vcvt.f32.u32 q0, q0, #24", // t
    "vsub.f32 q1, q0, q13",     // t - 0.5
    "vmul.f32 q1, q0, q1",      // t (t - 0.5)
    "vsub.f32 q2, q15, q0",     // 1 - t
    "vmul.f32 q1, q1, q2",
    "vmul.f32 q1, q1, q12",
    "vst1.32 {{d2-d3}}, [r2]!",
    "strb r7, [r3], #1",
    "vadd.i32 q8, q8, q9",
    "subs r4, r4, #1",
    "bne 6b",
    "pop {{r4-r7, pc}}",
    ".p2align 3",
    "9:",
    ".word 1, 2, 4, 8",
    ".size mpfree_simd_waves4_neon, .-mpfree_simd_waves4_neon",
    ".popsection",
);

/// [`crate::waves4`] in NEON. Returns false, having done nothing, when
/// there is no frame to do.
pub(crate) fn waves4(
    shape: crate::Shape4,
    osc: &crate::Osc4,
    out: &mut [[f32; 4]],
    flags: &mut [u8],
) -> bool {
    const _: () = assert!(core::mem::size_of::<crate::Osc4>() == 48);
    let frames = out.len().min(flags.len());
    let Ok(frames) = u32::try_from(frames) else {
        return false;
    };
    if frames == 0 {
        return false;
    }
    let args = Waves4 {
        osc,
        out: out.as_mut_ptr(),
        flags: flags.as_mut_ptr(),
        frames,
        shape: match shape {
            crate::Shape4::Saw => 0,
            crate::Shape4::Square => 1,
            crate::Shape4::Cubic => 2,
        },
    };
    // SAFETY: the kernel reads the 48 bytes of `osc` (#[repr(C)], offsets
    // checked by the size assertion and the layout test), writes `frames`
    // rows of 16 bytes to `out` and `frames` bytes to `flags` (frames <=
    // both lengths), and keeps the registers the calling convention asks
    // it to; the borrows outlive the call.
    unsafe { mpfree_simd_waves4_neon(&args) };
    true
}

#[repr(C)]
struct Mix4 {
    waves: *const [f32; 4],
    frames: u32,
    buses: *mut [f32; crate::MIX_STEP],
    count: u32,
    amp: *mut [f32; 4],
    damp: *const [f32; 4],
}

// Bus by bus, four frames a round: the four frames' products (wave times
// level, the level stepping on after each, lane by lane as the twin), a
// skipped oscillator's products made -0 (adding -0 leaves any sum as it
// is, as skipping does), then the 4 x 4 block turned over (frames across
// oscillators to oscillators across frames) so the four oscillators add
// into the bus's four frames in lane order. A bus no oscillator is heard
// on is left alone; a skipped oscillator's level is put back as it was.
//
// Mix4 offsets: waves 0, frames 4, buses 8, count 12, amp 16, damp 20.
global_asm!(
    ".pushsection .text.mpfree_simd_mix4_neon,\"ax\",%progbits",
    ".fpu neon",
    ".arm",
    ".p2align 4",
    ".globl mpfree_simd_mix4_neon",
    ".hidden mpfree_simd_mix4_neon",
    ".type mpfree_simd_mix4_neon,%function",
    "mpfree_simd_mix4_neon:",
    "push {{r4-r8, lr}}",
    "ldr r1, [r0, #0]",          // waves
    "ldr r2, [r0, #4]",          // frames: a multiple of 4, at least 4
    "ldr r3, [r0, #8]",          // the first bus's row
    "ldr r4, [r0, #12]",         // buses, at least 1
    "ldr r5, [r0, #16]",         // amp
    "ldr r6, [r0, #20]",         // damp
    "vmov.i32 q14, #0x80000000", // -0.0
    "1:",
    "vld1.32 {{d16-d17}}, [r5]",  // q8 = a
    "vld1.32 {{d18-d19}}, [r6]!", // q9 = d
    "vceq.f32 q12, q8, #0",
    "vceq.f32 q15, q9, #0",
    "vand q12, q12, q15", // q12 = skipped
    "vmvn q15, q12",
    "vorr d30, d30, d31",
    "vmov r7, r8, d30",
    "orrs r7, r7, r8",
    "beq 3f",       // no oscillator heard on this bus
    "vmov q11, q8", // the levels as they came
    "mov r7, r1",
    "mov r8, r3",
    "mov lr, r2",
    "2:",
    "vld1.32 {{d0-d3}}, [r7]!", // frames j, j + 1
    "vld1.32 {{d4-d7}}, [r7]!", // frames j + 2, j + 3
    "vmul.f32 q0, q0, q8",
    "vadd.f32 q8, q8, q9",
    "vmul.f32 q1, q1, q8",
    "vadd.f32 q8, q8, q9",
    "vmul.f32 q2, q2, q8",
    "vadd.f32 q8, q8, q9",
    "vmul.f32 q3, q3, q8",
    "vadd.f32 q8, q8, q9",
    "vbit q0, q14, q12",
    "vbit q1, q14, q12",
    "vbit q2, q14, q12",
    "vbit q3, q14, q12",
    "vtrn.32 q0, q1",
    "vtrn.32 q2, q3",
    "vswp d1, d4",
    "vswp d3, d6", // q0..q3: oscillator 0..3 across the four frames
    "vld1.32 {{d26-d27}}, [r8]",
    "vadd.f32 q13, q13, q0",
    "vadd.f32 q13, q13, q1",
    "vadd.f32 q13, q13, q2",
    "vadd.f32 q13, q13, q3",
    "vst1.32 {{d26-d27}}, [r8]!",
    "subs lr, lr, #4",
    "bne 2b",
    "vbsl q12, q11, q8", // a skipped oscillator's level as it came
    "vst1.32 {{d24-d25}}, [r5]",
    "3:",
    "add r5, r5, #16",
    "add r3, r3, #64",
    "subs r4, r4, #1",
    "bne 1b",
    "pop {{r4-r8, pc}}",
    ".size mpfree_simd_mix4_neon, .-mpfree_simd_mix4_neon",
    ".popsection",
);

/// [`crate::mix4`] in NEON, for the whole groups of four frames: returns
/// the frames done (0 when there is no whole group or no bus), for the
/// twin to go on from.
pub(crate) fn mix4(
    waves: &[[f32; 4]],
    amp: &mut [[f32; 4]],
    damp: &[[f32; 4]],
    buses: &mut [[f32; crate::MIX_STEP]],
) -> usize {
    let frames = waves.len().min(crate::MIX_STEP) & !3;
    let count = amp.len().min(damp.len()).min(buses.len());
    let Ok(count32) = u32::try_from(count) else {
        return 0;
    };
    if frames == 0 || count == 0 {
        return 0;
    }
    let args = Mix4 {
        waves: waves.as_ptr(),
        frames: frames as u32,
        buses: buses.as_mut_ptr(),
        count: count32,
        amp: amp.as_mut_ptr(),
        damp: damp.as_ptr(),
    };
    // SAFETY: the kernel reads `frames` rows of `waves` (frames <=
    // waves.len()), and for each of `count` buses (count <= every slice's
    // length) reads and writes its row's first `frames` values (frames <=
    // MIX_STEP, the row's length) and its 16 bytes of `amp`, and reads its
    // 16 bytes of `damp`; it keeps the registers the calling convention
    // asks it to; the borrows outlive the call.
    unsafe { mpfree_simd_mix4_neon(&args) };
    frames
}
