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

/// The twin of [`crate::formants4`]: frame by frame, lane by lane, formant
/// by formant (see there for what it computes).
pub fn formants4(src: &[[f32; 4]], out: &mut [[f32; 4]], f: &mut crate::Formants4) {
    for (e, y) in src.iter().zip(out.iter_mut()) {
        for lane in 0..4 {
            let e = e[lane];
            let mut acc = 0.0f32;
            for k in 0..crate::FORMANTS {
                let (s1, s2) = (f.s1[k][lane], f.s2[k][lane]);
                let v3 = e - s2;
                let v1 = f.a1[k][lane] * s1 + f.a2[k][lane] * v3;
                let v2 = s2 + f.a2[k][lane] * s1 + f.a3[k][lane] * v3;
                f.s1[k][lane] = 2.0 * v1 - s1;
                f.s2[k][lane] = 2.0 * v2 - s2;
                acc += f.g[k][lane] * v1;
            }
            y[lane] = acc;
        }
    }
}

/// `t` of a 32-bit phase: its top 24 bits as a share of a cycle.
#[inline(always)]
fn share(p: u32) -> f32 {
    (p >> 8) as f32 * (1.0 / 16_777_216.0)
}

/// Whether the four-point step correction may not be zero at `t` for a
/// step of `dt` (`poly_blep4`'s regions: `dt >= 0.25` runs `poly_blep`,
/// nonzero within `dt` of the step; else within `2 dt`).
#[inline(always)]
fn near_step(t: f32, dt: f32) -> bool {
    let two = dt + dt;
    dt >= 0.25 || t < two || t > 1.0 - two
}

/// The twin of [`crate::waves4`]: frame by frame, lane by lane.
pub fn waves4(shape: crate::Shape4, osc: &crate::Osc4, out: &mut [[f32; 4]], flags: &mut [u8]) {
    let mut p = osc.ph;
    for (w, f) in out.iter_mut().zip(flags.iter_mut()) {
        let mut bits = 0u8;
        for lane in 0..4 {
            let q = p[lane];
            let t = share(q);
            let dt = osc.dt[lane];
            let (x, near) = match shape {
                crate::Shape4::Saw => (t + t - 1.0, near_step(t, dt)),
                crate::Shape4::Square => {
                    let h = share(q.wrapping_add(1 << 31));
                    let down = if q < 1 << 31 { -1.0 } else { 1.0 };
                    (down, near_step(t, dt) || near_step(h, dt))
                }
                crate::Shape4::Cubic => (t * (t - 0.5) * (1.0 - t) * crate::CUBIC_SCALE, false),
            };
            w[lane] = x;
            bits |= u8::from(near) << lane;
            p[lane] = q.wrapping_add(osc.inc[lane]);
        }
        *f = bits;
    }
}

/// The twin of [`crate::mix4`].
pub fn mix4(
    waves: &[[f32; 4]],
    amp: &mut [[f32; 4]],
    damp: &[[f32; 4]],
    buses: &mut [[f32; crate::MIX_STEP]],
) {
    mix4_from(0, waves, amp, damp, buses);
}

/// [`mix4`] from frame `from` on (where the kernel stopped).
pub(crate) fn mix4_from(
    from: usize,
    waves: &[[f32; 4]],
    amp: &mut [[f32; 4]],
    damp: &[[f32; 4]],
    buses: &mut [[f32; crate::MIX_STEP]],
) {
    let frames = waves.len().min(crate::MIX_STEP);
    if from >= frames {
        return;
    }
    for ((a, d), bus) in amp.iter_mut().zip(damp).zip(buses.iter_mut()) {
        let heard: [bool; 4] = std::array::from_fn(|l| a[l] != 0.0 || d[l] != 0.0);
        for j in from..frames {
            for lane in 0..4 {
                if heard[lane] {
                    bus[j] += waves[j][lane] * a[lane];
                    a[lane] += d[lane];
                }
            }
        }
    }
}
