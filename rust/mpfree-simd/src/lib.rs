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
//! - [`formants4`]: five band-pass formant filters (zero-delay state
//!   variable filters) for four voices at once, summed: the Choir's
//!   formants, one voice a lane.
//! - [`waves4`] and [`mix4`]: the divide-down bank (`mpfree_synth::bank`)
//!   four notes at a time, one a lane: their waves without the step
//!   correction (the bank adds it where the kernel flags), then mixed into
//!   the buses along each bus's level ramps.
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
    // Elsewhere this is false at compile time, so the dispatch folds away
    // and an engine's loop is the twin's alone.
    if !cfg!(all(
        target_arch = "arm",
        target_endian = "little",
        target_os = "linux"
    )) {
        return false;
    }
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

/// Formant filters per voice in [`formants4`].
pub const FORMANTS: usize = 5;

/// Four voices' formant filters, one voice a lane: each field is
/// `[formant][lane]`. A filter is Simper's zero-delay state-variable
/// band-pass: `s1` and `s2` its state, `a1`, `a2` and `a3` its
/// coefficients (`a1 = 1 / (1 + g (g + k))`, `a2 = g a1`, `a3 = g a2`), and
/// `g` the signed gain its band-pass output is summed with. The layout is
/// the kernel's: `#[repr(C)]`, the fields in this order.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Formants4 {
    pub s1: [[f32; 4]; FORMANTS],
    pub s2: [[f32; 4]; FORMANTS],
    pub a1: [[f32; 4]; FORMANTS],
    pub a2: [[f32; 4]; FORMANTS],
    pub a3: [[f32; 4]; FORMANTS],
    pub g: [[f32; 4]; FORMANTS],
}

/// Runs four voices' source through their five formant filters: `out[n]`
/// gets, lane by lane, the sum of each filter's band-pass output times its
/// gain, from `src[n]`. Frame by frame, formant 0 first, each lane:
///
/// ```text
/// v3 = e - s2
/// v1 = a1 * s1 + a2 * v3          (the band-pass output)
/// v2 = s2 + a2 * s1 + a3 * v3
/// s1 = 2 * v1 - s1,  s2 = 2 * v2 - s2
/// y  = 0 + g0 * v1, then y + g * v1 for formants 1 to 4
/// ```
///
/// As many frames as the shorter of `src` and `out`; the rest of `out` is
/// left alone. The NEON kernel does the same operations in the same order,
/// four lanes at a time and a formant at a time, so the samples are the
/// same on either path.
pub fn formants4(src: &[[f32; 4]], out: &mut [[f32; 4]], f: &mut Formants4) {
    formants4_on(path(), src, out, f);
}

/// [`formants4`] on a chosen path (NEON only where [`neon_available`]);
/// returns the path that ran.
pub fn formants4_on(want: Path, src: &[[f32; 4]], out: &mut [[f32; 4]], f: &mut Formants4) -> Path {
    #[cfg(all(target_arch = "arm", target_endian = "little", target_os = "linux"))]
    if want == Path::Neon && neon_available() && neon::formants4(src, out, f) {
        return Path::Neon;
    }
    let _ = want;
    plain::formants4(src, out, f);
    Path::Plain
}

/// A wave of [`waves4`]: the divide-down bank's shapes
/// (`mpfree_synth::bank::Shape`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape4 {
    /// `t + t - 1`, a step at the cycle's end.
    Saw,
    /// -1 for the first half cycle, 1 for the second, a step at each.
    Square,
    /// `t (t - 1/2) (1 - t)` times [`CUBIC_SCALE`]: no step.
    Cubic,
}

/// The cubic's scale in [`waves4`] (the bank's: a peak of 1).
pub const CUBIC_SCALE: f32 = 20.784_609;

/// Four oscillators, one a lane: each one's 32-bit phase, its step a
/// frame, and that step as a share of a cycle. The layout is the
/// kernel's: `#[repr(C)]`, the fields in this order.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Osc4 {
    pub ph: [u32; 4],
    pub inc: [u32; 4],
    pub dt: [f32; 4],
}

/// Four oscillators' waves without their step correction, one a lane:
/// `out[j][lane]` at frame `j`, the phase moving on by `inc` a frame from
/// `ph` (which is left as it is). With `t = (p >> 8) as f32 / 2^24`:
///
/// - [`Shape4::Saw`]: `t + t - 1`;
/// - [`Shape4::Square`]: -1 while `p < 2^31`, else 1;
/// - [`Shape4::Cubic`]: `t * (t - 0.5) * (1 - t) * CUBIC_SCALE`.
///
/// `flags[j]` gets bit `lane` set where the four-point step correction
/// (`mpfree_synth::dsp::poly_blep4`) may not be zero, so the caller adds it
/// there and only there: for the saw where `dt >= 0.25`, `t < 2 dt` or
/// `t > 1 - 2 dt`; for the square the same for `t` or for `h`, the phase
/// half a cycle on; never for the cubic. As many frames as the shorter of
/// `out` and `flags`.
pub fn waves4(shape: Shape4, osc: &Osc4, out: &mut [[f32; 4]], flags: &mut [u8]) {
    waves4_on(path(), shape, osc, out, flags);
}

/// [`waves4`] on a chosen path (NEON only where [`neon_available`]);
/// returns the path that ran.
pub fn waves4_on(
    want: Path,
    shape: Shape4,
    osc: &Osc4,
    out: &mut [[f32; 4]],
    flags: &mut [u8],
) -> Path {
    #[cfg(all(target_arch = "arm", target_endian = "little", target_os = "linux"))]
    if want == Path::Neon && neon_available() && neon::waves4(shape, osc, out, flags) {
        return Path::Neon;
    }
    let _ = want;
    plain::waves4(shape, osc, out, flags);
    Path::Plain
}

/// Frames a [`mix4`] call takes at most, and each bus row's length: one
/// control step of the bank.
pub const MIX_STEP: usize = 16;

/// Adds four oscillators' waves (`waves[j][lane]`, one oscillator a lane)
/// into buses, each along its own level ramps: for bus `b` and frame `j`,
/// oscillator by oscillator in lane order,
///
/// ```text
/// buses[b][j] += waves[j][lane] * a;  a += d
/// ```
///
/// with `a` starting at `amp[b][lane]`, `d` = `damp[b][lane]`, and `a`'s
/// end left in `amp`. An oscillator whose `a` and `d` are both zero on a
/// bus is skipped there (the bank's "not heard on this bus"). As many
/// buses as the shortest of `amp`, `damp` and `buses`, and as many frames
/// as `waves` holds, up to [`MIX_STEP`]. The NEON kernel gives the same
/// samples and levels to the bit for finite waves and levels.
pub fn mix4(
    waves: &[[f32; 4]],
    amp: &mut [[f32; 4]],
    damp: &[[f32; 4]],
    buses: &mut [[f32; MIX_STEP]],
) {
    mix4_on(path(), waves, amp, damp, buses);
}

/// [`mix4`] on a chosen path (NEON only where [`neon_available`]);
/// returns the path that ran (the frames after the last whole group of
/// four always run as the twin).
pub fn mix4_on(
    want: Path,
    waves: &[[f32; 4]],
    amp: &mut [[f32; 4]],
    damp: &[[f32; 4]],
    buses: &mut [[f32; MIX_STEP]],
) -> Path {
    #[cfg(all(target_arch = "arm", target_endian = "little", target_os = "linux"))]
    if want == Path::Neon && neon_available() {
        let done = neon::mix4(waves, amp, damp, buses);
        if done > 0 {
            plain::mix4_from(done, waves, amp, damp, buses);
            return Path::Neon;
        }
    }
    let _ = want;
    plain::mix4(waves, amp, damp, buses);
    Path::Plain
}

/// Rows and columns of the block [`gemm4x8`] works on.
pub const GEMM_MR: usize = 4;
pub const GEMM_NR: usize = 8;

/// The stem separation's GEMM micro-kernel: over `kc` steps,
///
/// ```text
/// acc[r * 8 + j] = acc[r * 8 + j] + a[k * 4 + r] * b[k * 8 + j]
/// ```
///
/// for k from 0 up, each lane in its own order: `a` is a packed panel of
/// four rows (`kc × 4` values), `b` one of eight columns (`kc × 8`), and
/// `acc` the 4 × 8 block, row by row. The NEON kernel's `VMLA.F32` rounds
/// the product before it adds, as the twin does, so both give the same
/// sums. Calls where `a` or `b` is shorter than `kc` steps take the twin's
/// path and panic there, as a slice index does.
#[inline]
pub fn gemm4x8(kc: usize, a: &[f32], b: &[f32], acc: &mut [f32; GEMM_MR * GEMM_NR]) {
    gemm4x8_on(path(), kc, a, b, acc);
}

/// [`gemm4x8`] on a chosen path (NEON only where [`neon_available`]);
/// returns the path that ran.
#[inline]
pub fn gemm4x8_on(
    want: Path,
    kc: usize,
    a: &[f32],
    b: &[f32],
    acc: &mut [f32; GEMM_MR * GEMM_NR],
) -> Path {
    #[cfg(all(target_arch = "arm", target_endian = "little", target_os = "linux"))]
    if want == Path::Neon && neon_available() && neon::gemm4x8(kc, a, b, acc) {
        return Path::Neon;
    }
    let _ = want;
    plain::gemm4x8(kc, a, b, acc);
    Path::Plain
}

/// IEEE half-precision values (as bits) to f32, as many as the shorter
/// slice holds: exact for zeros, subnormals, normals and infinities; a NaN
/// becomes the default NaN (`0x7fc0_0000`), as NEON makes it. The NEON
/// kernel (`VCVT.F32.F16`, VFPv4's half-precision conversion) does eight at
/// a time, the rest go through the twin. NEON runs only where
/// [`neon_available`] and the processor reports VFPv4.
#[inline]
pub fn f16_to_f32(src: &[u16], dst: &mut [f32]) {
    f16_to_f32_on(path(), src, dst);
}

/// [`f16_to_f32`] on a chosen path; returns the path that ran.
#[inline]
pub fn f16_to_f32_on(want: Path, src: &[u16], dst: &mut [f32]) -> Path {
    let n = src.len().min(dst.len());
    #[cfg(all(target_arch = "arm", target_endian = "little", target_os = "linux"))]
    if want == Path::Neon && neon_available() && neon::half_available() {
        let done = neon::f16_to_f32(&src[..n], &mut dst[..n]);
        plain::f16_to_f32(&src[done..n], &mut dst[done..n]);
        return Path::Neon;
    }
    let _ = want;
    plain::f16_to_f32(&src[..n], &mut dst[..n]);
    Path::Plain
}

#[cfg(test)]
mod tests;
