//! The plain Rust twins: the reference each NEON kernel is checked
//! against, and the code that runs wherever the kernel doesn't (the
//! workstation, a processor without NEON, the kernels turned off).
//!
//! Each twin is written the way the engine's own loop was before the
//! kernel existed, so the workstation's numbers and sound don't change
//! when an engine moves onto it.

/// The mask for the phase bits below a table's index: `2^shift - 1`.
/// `shift` must be below 32.
#[inline]
pub fn mask(shift: u32) -> u32 {
    debug_assert!(shift < 32, "shift {shift}");
    (1u32 << shift) - 1
}

/// One read of a table level at phase `p`: linear between the entry at
/// `p >> shift` and the next, from the entry's sample and its scaled step.
#[inline]
pub fn read(table: &[[f32; 2]], shift: u32, mask: u32, p: u32) -> f32 {
    let [a, step] = table[(p >> shift) as usize];
    a + step * (p & mask) as f32
}

/// The twin of [`crate::table_sum`]: oscillator by oscillator, frame by
/// frame (see there for what it computes).
pub fn table_sum<const ADD: bool>(
    table: &[[f32; 2]],
    shift: u32,
    buf: &mut [f32],
    ph: &mut [u32],
    inc: &[u32],
    gain: &[f32],
) {
    let mask = mask(shift);
    for (k, ((p, &i), &g)) in ph.iter_mut().zip(inc).zip(gain).enumerate() {
        let mut q = *p;
        if k == 0 && !ADD {
            for x in buf.iter_mut() {
                *x = 0.0 + read(table, shift, mask, q) * g;
                q = q.wrapping_add(i);
            }
        } else {
            for x in buf.iter_mut() {
                *x += read(table, shift, mask, q) * g;
                q = q.wrapping_add(i);
            }
        }
        *p = q;
    }
}
