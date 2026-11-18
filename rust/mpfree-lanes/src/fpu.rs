//! The processor's flush-to-zero mode, for the threads that render sound
//! (the synth architecture's lever 5, C2).
//!
//! A float result under the smallest normal number (about 1.2e-38, some
//! 750 dB under full scale) is a *denormal*. Many processors take far
//! longer over denormals than over normal numbers: a filter or reverb
//! state decaying through them can cost tens of times its usual time,
//! which shows as a worst period many times the average. With
//! flush-to-zero on, such a result is zero instead, so the sound changes
//! by less than −700 dB and never by an audible amount.
//!
//! The mode belongs to a thread: [`flush_to_zero`] turns it on for the
//! thread that calls it. On Linux a thread starts with the mode of the
//! thread that started it (the kernel copies the floating-point control
//! register), so the threads the audio thread starts later flush too,
//! helpers included; that changes nothing audible. The app turns the mode
//! on for the audio thread, and each lane asked to
//! ([`crate::Config::flush_to_zero`]) turns it on for its worker as it
//! starts, whichever thread started it, so a part renders the same on a
//! lane as on the audio thread. Engines still flush their own tails, so a
//! silent engine puts out exact zeros whatever the mode.
//!
//! - **ARMv7** (the MPC): FPSCR's FZ bit (24). NEON flushes denormals
//!   whatever the bit says; this makes the VFP unit do the same.
//! - **AArch64**: FPCR's FZ bit (24).
//! - **x86-64** (the workstation): MXCSR's FTZ bit (15), and DAZ (6),
//!   which reads denormal inputs as zero.
//! - Anything else: nothing is changed and [`flush_to_zero`] says so.
//!
//! Each processor's part is one register read and one write, here only.

#![allow(unsafe_code)]

/// Turns flush-to-zero on for the calling thread. Returns whether it is
/// on afterwards: false only on a processor this module doesn't know.
pub fn flush_to_zero() -> bool {
    imp::set();
    imp::on()
}

/// Whether flush-to-zero is on for the calling thread.
pub fn flushing() -> bool {
    imp::on()
}

#[cfg(target_arch = "x86_64")]
mod imp {
    use std::arch::asm;

    /// MXCSR's flush-to-zero (15) and denormals-are-zero (6) bits.
    const BITS: u32 = (1 << 15) | (1 << 6);

    fn get() -> u32 {
        let mut csr: u32 = 0;
        // SAFETY: STMXCSR writes the 32-bit MXCSR to the address given,
        // which is `csr`'s, valid and aligned for the write.
        unsafe {
            asm!(
                "stmxcsr [{}]",
                in(reg) &mut csr as *mut u32,
                options(nostack, preserves_flags)
            );
        }
        csr
    }

    pub fn set() {
        let csr = get() | BITS;
        // SAFETY: LDMXCSR reads MXCSR from `csr`'s address, valid for the
        // read. Only FTZ and DAZ change: the rounding mode and the
        // exception masks are kept, so no exception is unmasked.
        unsafe {
            asm!(
                "ldmxcsr [{}]",
                in(reg) &csr as *const u32,
                options(nostack, preserves_flags, readonly)
            );
        }
    }

    pub fn on() -> bool {
        get() & BITS == BITS
    }
}

#[cfg(target_arch = "aarch64")]
mod imp {
    use std::arch::asm;

    /// FPCR's flush-to-zero bit.
    const FZ: u64 = 1 << 24;

    fn get() -> u64 {
        let v: u64;
        // SAFETY: reading FPCR has no side effects.
        unsafe { asm!("mrs {}, fpcr", out(reg) v, options(nomem, nostack, preserves_flags)) };
        v
    }

    pub fn set() {
        let v = get() | FZ;
        // SAFETY: only FZ changes; rounding and the trap enables are kept.
        unsafe { asm!("msr fpcr, {}", in(reg) v, options(nomem, nostack, preserves_flags)) };
    }

    pub fn on() -> bool {
        get() & FZ != 0
    }
}

#[cfg(target_arch = "arm")]
mod imp {
    use std::arch::asm;

    /// FPSCR's flush-to-zero bit.
    const FZ: u32 = 1 << 24;

    fn get() -> u32 {
        let v: u32;
        // SAFETY: reading FPSCR into a core register has no side effects
        // (it is not the form that copies its flags into the APSR).
        unsafe { asm!("vmrs {}, fpscr", out(reg) v, options(nomem, nostack, preserves_flags)) };
        v
    }

    pub fn set() {
        let v = get() | FZ;
        // SAFETY: only FZ changes; rounding, the vector length and stride
        // and the trap enables are kept.
        unsafe { asm!("vmsr fpscr, {}", in(reg) v, options(nomem, nostack, preserves_flags)) };
    }

    pub fn on() -> bool {
        get() & FZ != 0
    }
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64", target_arch = "arm")))]
mod imp {
    pub fn set() {}

    pub fn on() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hint::black_box;

    /// 1e-30 times 1e-10 is 1e-40, a denormal.
    fn tiny_product() -> f32 {
        black_box(1e-30f32) * black_box(1e-10f32)
    }

    #[test]
    fn a_thread_flushes_denormals_once_asked_and_the_threads_it_starts_do_too() {
        std::thread::spawn(|| {
            let known = cfg!(any(
                target_arch = "x86_64",
                target_arch = "aarch64",
                target_arch = "arm"
            ));
            assert!(!flushing() || !known || cfg!(target_arch = "arm"));
            assert_eq!(flush_to_zero(), known);
            assert_eq!(flushing(), known);
            if known {
                assert_eq!(tiny_product(), 0.0);
                // Normal numbers are untouched.
                assert_eq!(black_box(1e-20f32) * black_box(1e-10f32), 1e-30);
                assert_eq!(black_box(0.5f32) * black_box(0.25f32), 0.125);
                // On Linux a thread started from here starts with the
                // mode on. Checked on x86-64 only: the ARM tests run under
                // an emulator, which would be what this checked.
                let fresh = std::thread::spawn(|| (flushing(), tiny_product()))
                    .join()
                    .unwrap();
                if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
                    assert_eq!(fresh, (true, 0.0));
                }
            }
        })
        .join()
        .unwrap();
    }
}
