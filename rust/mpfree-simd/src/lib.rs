//! NEON kernels for the MPC One's ARMv7 cores (the synth architecture's
//! lever 6, C3: `docs/dev/design/simd.md`).
//!
//! Stable Rust gives no float NEON code for 32-bit ARM: LLVM won't
//! vectorise `f32` loops for ARMv7 NEON (it flushes denormals, so it isn't
//! IEEE exact), and the NEON intrinsics and `#[target_feature(enable =
//! "neon")]` are nightly-only there. So each kernel here is hand-written
//! NEON assembly (`global_asm!`, stable), and each has a **plain Rust
//! twin**:
//!
//! - The twin is the reference. It runs on the workstation and on any
//!   processor without NEON, and the tests check the kernel against it bit
//!   for bit under qemu-arm (`cargo test --target
//!   armv7-unknown-linux-gnueabihf -p mpfree-simd`, the ARMv7 CI job).
//! - The kernel does the twin's arithmetic in the twin's order, four frames
//!   at a time, so both give the same samples. NEON always flushes
//!   denormals to zero; the app's sound threads flush them too
//!   (`mpfree_lanes::fpu`), so there the two agree even on values that
//!   small, and away from them they agree whatever the mode.
//! - A kernel is kept only if it wins on the device: `mpfree-bench` times
//!   both, and the device round decides.
//!
//! This crate holds the only unsafe code the synths reach; `mpfree-synth`
//! keeps `#![forbid(unsafe_code)]`. Every public function is safe: it
//! checks what the assembly relies on (table length, slice lengths) before
//! calling it, and runs the twin when a call doesn't fit.
//!
//! # The kernels
//!
//! - [`table_sum`]: several oscillators reading one band-limited table, each
//!   at its own rate and gain, added into a buffer: the supersaw's seven
//!   saws (`mpfree_synth::osc::Classic::supersaw`).
//!
//! # Choosing the code that runs
//!
//! [`table_sum`] takes the NEON kernel when this is an ARMv7 build, the
//! processor has NEON (asked of Linux once, through `getauxval`), and the
//! kernels are on ([`set_enabled`]; on by default). [`path`] says which runs
//! now and [`status`] says why, for the Dev page. [`table_sum_on`] runs a
//! chosen one, for the bench's A/B and the tests.
//!
//! Nothing here allocates, locks, blocks or panics while sounding.

#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};

#[cfg(all(target_arch = "arm", target_endian = "little", target_os = "linux"))]
mod neon;
pub mod plain;
pub mod workload;

/// The code a kernel runs as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Path {
    /// The plain Rust twin.
    Plain,
    /// The NEON kernel.
    Neon,
}

impl Path {
    /// A short name for reports: "plain" or "NEON".
    pub fn name(self) -> &'static str {
        match self {
            Path::Plain => "plain",
            Path::Neon => "NEON",
        }
    }
}

static ENABLED: AtomicBool = AtomicBool::new(true);
/// Calls that asked for NEON on a processor with it but ran the twin
/// because their table didn't fit the kernel ([`fallbacks`]).
static FALLBACKS: AtomicU32 = AtomicU32::new(0);

/// Turns the NEON kernels on or off for the whole program (on by default).
/// Off, every kernel runs its plain twin: the device A/B, and a way out if
/// a kernel ever misbehaves.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// Whether the NEON kernels are on ([`set_enabled`]); they run only where
/// [`neon_available`] too.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Whether this build has the NEON kernels and this processor runs them.
/// Asked of Linux once (`getauxval(AT_HWCAP)`); false on anything but an
/// ARMv7 Linux build.
pub fn neon_available() -> bool {
    // 0: not asked yet, 1: no, 2: yes.
    static NEON: AtomicU8 = AtomicU8::new(0);
    match NEON.load(Ordering::Relaxed) {
        0 => {
            let yes = detect();
            NEON.store(if yes { 2 } else { 1 }, Ordering::Relaxed);
            yes
        }
        n => n == 2,
    }
}

#[cfg(all(target_arch = "arm", target_endian = "little", target_os = "linux"))]
fn detect() -> bool {
    neon::detect()
}

#[cfg(not(all(target_arch = "arm", target_endian = "little", target_os = "linux")))]
fn detect() -> bool {
    false
}

/// The code [`table_sum`] and the other kernels run as now.
pub fn path() -> Path {
    if enabled() && neon_available() {
        Path::Neon
    } else {
        Path::Plain
    }
}

/// Which code the kernels run as and why, in a few words for the Dev page.
pub fn status() -> &'static str {
    if neon_available() {
        if enabled() {
            "NEON"
        } else {
            "plain Rust: NEON turned off"
        }
    } else if cfg!(all(target_arch = "arm", target_os = "linux")) {
        "plain Rust: this processor has no NEON"
    } else {
        "plain Rust: not an ARMv7 build"
    }
}

/// Calls since the program started that asked for NEON on a processor
/// with it, but ran the plain twin because their table didn't fit the
/// kernel (a length that isn't 2^(32 - shift)). Always 0 unless a caller
/// is wrong; shown so a fallback is never silent.
pub fn fallbacks() -> u32 {
    FALLBACKS.load(Ordering::Relaxed)
}

/// Adds oscillators that read one table into `buf`: the supersaw's saws.
///
/// - `table` is one level of a band-limited wave: each entry is a sample
///   and its step to the next one, the step already scaled to the phase
///   bits below the index (`mpfree_synth::osc::Waveset`). Its length must be
///   2^(32 - `shift`), so the top bits of a phase index it.
/// - Oscillator `k` starts at phase `ph[k]` (a 32-bit fraction of a cycle)
///   and moves `inc[k]` a frame, wrapping; its sample is `a + step *
///   (p & mask) as f32` for the entry `[a, step]` at `p >> shift`, times
///   `gain[k]`. There are as many oscillators as the shortest of the three
///   slices. Each phase is left where the block ends.
/// - With `ADD` false the first oscillator writes (`0.0 + x`, so never
///   -0.0) and the rest add; with `ADD` true every one adds to what `buf`
///   holds. Each frame sums the oscillators in their order, so the result
///   is the same for any block size, and the same on either path.
///
/// Runs the NEON kernel where [`path`] says so, else the plain twin
/// ([`plain::table_sum`]).
pub fn table_sum<const ADD: bool>(
    table: &[[f32; 2]],
    shift: u32,
    buf: &mut [f32],
    ph: &mut [u32],
    inc: &[u32],
    gain: &[f32],
) {
    table_sum_on::<ADD>(path(), table, shift, buf, ph, inc, gain);
}

/// [`table_sum`] on a chosen path, for A/B timing and the tests. NEON runs
/// only where [`neon_available`] (whatever [`enabled`] says); elsewhere the
/// twin runs. Returns the path that ran.
///
/// The kernel does whole groups of four frames; the frames after the last
/// group (and any call with fewer than four, or a table that doesn't fit
/// the kernel) go through the twin, which gives the same samples.
#[allow(clippy::too_many_arguments)]
pub fn table_sum_on<const ADD: bool>(
    want: Path,
    table: &[[f32; 2]],
    shift: u32,
    buf: &mut [f32],
    ph: &mut [u32],
    inc: &[u32],
    gain: &[f32],
) -> Path {
    #[cfg(all(target_arch = "arm", target_endian = "little", target_os = "linux"))]
    if want == Path::Neon && neon_available() {
        match neon::table_sum(table, shift, buf, ph, inc, gain, ADD) {
            Some(0) => {}
            Some(done) => {
                plain::table_sum::<ADD>(table, shift, &mut buf[done..], ph, inc, gain);
                return Path::Neon;
            }
            None => {
                FALLBACKS.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    let _ = want;
    plain::table_sum::<ADD>(table, shift, buf, ph, inc, gain);
    Path::Plain
}

#[cfg(test)]
mod tests;
