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
