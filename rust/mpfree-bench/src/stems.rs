//! `--stems`: the stem separation (`mpfree-stems`) on Spleeter's real
//! shapes, with seeded weights (they cost what Deezer's do, so no model
//! file is needed), on 30 s of seeded noise: the speed and memory the
//! plan's estimate guessed at, for the device round
//! (`docs/dev/design/stems.md`).
//!
//! In order: its two NEON kernels against their plain twins, bit for bit
//! (a mismatch stops it, exit code 1); the free memory (below 400 MB it
//! skips itself, exit code 0, so it can never push the MPC into swap or
//! the OOM killer); the GEMM kernel alone on core 0, NEON and plain, its
//! data in the L1 cache (the peak the U-Nets' rate compares with); then
//! four runs, each timed by stage: NEON on two
//! threads on cores 0 and 1 at 1024 bins (the job's own setting), on one
//! thread on core 0, the plain twins on two, and NEON on two at full band
//! (1536 bins); then an estimate for 10 s and 60 s samples.
//!
//! It only computes and reads: `/proc/meminfo`, `/proc/self/status`, and
//! the CPU's temperature and clock from `/sys`. Pinning changes only this
//! program's own threads, and ends with it.

use mpfree_simd::{Path, GEMM_MR, GEMM_NR};
use mpfree_stems::{Band, Model, Separator, Settings, Sink, Stem, RATE};
use std::process::ExitCode;
use std::time::Instant;

/// Seconds of noise each run separates: three segments.
const SECONDS: usize = 30;
/// The seed of the network's weights and of the noise.
const SEED: u32 = 11;
/// Below this much free memory the runs are skipped.
const FLOOR_MB: u64 = 400;
/// Loading the model from the stick, assumed until the app reads the real
/// file (79 MB at an assumed 30 MB/s).
const LOAD_S: f64 = 2.6;

/// What the kernels' check found.
enum Check {
    Matches,
    NoNeon,
    Differs(String),
}

fn check_kernels() -> Check {
    if !mpfree_simd::neon_available() {
        return Check::NoNeon;
    }
    let mut seed = 0x6e44u32;
    let mut unit = move || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    };
    for case in 0..300 {
        let kc = [0, 1, 2, 7, 64, 256][case % 6] + case % 3;
        let a: Vec<f32> = (0..kc * GEMM_MR).map(|_| unit()).collect();
        let b: Vec<f32> = (0..kc * GEMM_NR).map(|_| unit() * 4.0).collect();
        let start: [f32; 32] = std::array::from_fn(|_| unit());
        let (mut plain, mut neon) = (start, start);
        mpfree_simd::gemm4x8_on(Path::Plain, kc, &a, &b, &mut plain);
        let took = mpfree_simd::gemm4x8_on(Path::Neon, kc, &a, &b, &mut neon);
        if took != Path::Neon || plain.map(f32::to_bits) != neon.map(f32::to_bits) {
            return Check::Differs(format!("the GEMM kernel, call {case} of {kc} steps"));
        }
    }
    let halves: Vec<u16> = (0..=u16::MAX).collect();
    let (mut plain, mut neon) = (vec![0f32; halves.len()], vec![0f32; halves.len()]);
    mpfree_simd::f16_to_f32_on(Path::Plain, &halves, &mut plain);
    let took = mpfree_simd::f16_to_f32_on(Path::Neon, &halves, &mut neon);
    if took != Path::Neon {
        return Check::Differs("the half-to-float kernel didn't run (no VFPv4?)".into());
    }
    if let Some(h) = (0..halves.len()).find(|&i| plain[i].to_bits() != neon[i].to_bits()) {
        return Check::Differs(format!(
            "the half-to-float kernel, half {h:#06x}: {:#010x} against {:#010x}",
            neon[h].to_bits(),
            plain[h].to_bits()
        ));
    }
    Check::Matches
}

/// The GEMM kernel alone on `path`, its data in the L1 cache (a 256-step
/// block, as the U-Nets' layers run it): multiply-adds a second over about
/// a quarter of a second. Against the U-Nets' own rate it says how much of
/// their time goes outside the kernel.
fn kernel_rate(path: Path) -> f64 {
    const KC: usize = 256;
    let a: Vec<f32> = (0..KC * GEMM_MR)
        .map(|i| (i % 7) as f32 * 0.1 - 0.3)
        .collect();
    let b: Vec<f32> = (0..KC * GEMM_NR)
        .map(|i| (i % 5) as f32 * 0.1 - 0.2)
        .collect();
    let mut acc = [0f32; 32];
    let (mut calls, t) = (0u64, Instant::now());
    while t.elapsed().as_secs_f64() < 0.25 {
        for _ in 0..256 {
            mpfree_simd::gemm4x8_on(path, KC, &a, &b, &mut acc);
        }
        calls += 256;
    }
    std::hint::black_box(acc);
    (calls * (KC * GEMM_MR * GEMM_NR) as u64) as f64 / t.elapsed().as_secs_f64()
}

/// A `/proc` file's value for `key` (the first number after it).
fn proc_value(path: &str, key: &str) -> Option<u64> {
    let text = std::fs::read_to_string(path).ok()?;
    let line = text.lines().find(|l| l.starts_with(key))?;
    line[key.len()..].split_whitespace().next()?.parse().ok()
}

fn temperature_and_clock() -> String {
    let read = |p: &str| -> Option<f64> { std::fs::read_to_string(p).ok()?.trim().parse().ok() };
    let temp = read("/sys/class/thermal/thermal_zone0/temp")
        .map(|t| format!("{:.1} C", t / 1000.0))
        .unwrap_or_else(|| "temperature not readable".into());
    let clock = read("/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq")
        .map(|k| format!("{:.0} MHz", k / 1000.0))
        .unwrap_or_else(|| "clock not readable".into());
    format!("{temp}, core 0 at {clock}")
}

/// Each stem's sum of |sample| and the frames it got: shows the run made
/// numbers, not NaNs.
#[derive(Default)]
struct Levels {
    sum: [f64; 4],
    frames: [usize; 4],
}

impl Sink for Levels {
    fn put(&mut self, stem: Stem, stereo: &[f32]) -> Result<(), String> {
        let k = Stem::ALL.iter().position(|&s| s == stem).unwrap_or(0);
        self.sum[k] += stereo.iter().map(|v| f64::from(v.abs())).sum::<f64>();
        self.frames[k] += stereo.len() / 2;
        Ok(())
    }
}

/// One run's measurements.
struct Run {
    secs: f64,
    /// The U-Nets' seconds a segment, and everything else's a second of
    /// audio: what the estimate scales.
    per_segment: f64,
    per_second: f64,
}

/// STFT segments a sample of `secs` seconds makes.
fn segments(secs: f64) -> usize {
    let frames = ((secs * f64::from(RATE)) as usize + 4096).div_ceil(1024);
    frames.div_ceil(512)
}

/// One of the four runs.
struct Case {
    what: &'static str,
    band: Band,
    threads: usize,
    cores: &'static [usize],
    /// The NEON kernels on (unless the build runs `--plain`).
    neon: bool,
}

fn run_one(case: &Case, model: &Model, input: &[f32], plain_build: bool) -> Result<Run, String> {
    let Case {
        what,
        band,
        threads,
        cores,
        neon,
    } = *case;
    mpfree_simd::set_enabled(neon && !plain_build);
    let path = match mpfree_simd::path() {
        Path::Neon => "NEON kernels",
        Path::Plain => "plain twins",
    };
    let pin = match mpfree_lanes::os::pin_set(cores) {
        Ok(()) => format!("on cores {cores:?}"),
        Err(e) => format!("not pinned ({e})"),
    };
    let settings = Settings { band, threads };
    let mut levels = Levels::default();
    let t = Instant::now();
    let mut sep = Separator::new(model, input, 2, RATE, &settings).map_err(|e| e.to_string())?;
    while sep.step(&mut levels).map_err(|e| e.to_string())? {}
    let secs = t.elapsed().as_secs_f64();
    let st = sep.stats();
    let audio = (input.len() / 2) as f64 / f64::from(RATE);
    let nets = st.nets.as_secs_f64();
    let rest = (st.analyse + st.masks + st.sink).as_secs_f64();
    let finite = levels.sum.iter().all(|v| v.is_finite());
    let frames_ok = levels.frames.iter().all(|&f| f == input.len() / 2);
    let hwm = proc_value("/proc/self/status", "VmHWM:")
        .map_or("not readable".into(), |k| format!("{} MB", k / 1024));
    let threads_word = if threads == 1 { "thread" } else { "threads" };
    println!("stems: {what} ({path}, {threads} {threads_word} {pin}):");
    println!(
        "  {secs:.2} s for {audio:.0} s of audio ({:.3} s a second): STFT {:.2} s, U-Nets {nets:.2} s \
         (unpacking halves {:.2} s), masks and inverse STFT {:.2} s",
        secs / audio,
        st.analyse.as_secs_f64(),
        st.unpack.as_secs_f64(),
        st.masks.as_secs_f64()
    );
    println!(
        "  {:.2} G kernel multiply-adds, {:.2} G/s over the U-Nets; {} segments; promised {:.0} MB; \
         peak memory so far (VmHWM) {hwm}",
        st.macs as f64 / 1e9,
        st.macs as f64 / 1e9 / nets.max(1e-9),
        st.segments,
        model.need(&settings) as f64 / 1e6
    );
    let mean: Vec<String> = levels
        .sum
        .iter()
        .map(|s| format!("{:.4}", s / (input.len() as f64)))
        .collect();
    println!(
        "  stems' mean |sample| (vocals drums bass other): {}{}",
        mean.join(" "),
        if finite && frames_ok {
            ", all finite and whole"
        } else {
            ""
        }
    );
    if !finite || !frames_ok {
        return Err(format!("{what}: stems not finite or not whole"));
    }
    Ok(Run {
        secs,
        per_segment: nets / st.segments.max(1) as f64,
        per_second: rest / audio,
    })
}

pub fn run(plain_build: bool, machine: &str, build: &str) -> ExitCode {
    println!("mpfree-bench --stems on {machine}, build {build}");
    match check_kernels() {
        Check::Matches => println!(
            "stems: NEON matches plain (the GEMM kernel over 300 calls, the half-to-float kernel on all 65,536 halves)"
        ),
        Check::NoNeon => println!("stems: no NEON on this machine; the kernels' check runs on the MPC"),
        Check::Differs(what) => {
            println!("stems: NEON differs from plain: {what}");
            return ExitCode::from(1);
        }
    }
    match proc_value("/proc/meminfo", "MemAvailable:").map(|k| k / 1024) {
        Some(mb) if mb < FLOOR_MB => {
            println!("stems: skipped, only {mb} MB free (it needs {FLOOR_MB})");
            return ExitCode::SUCCESS;
        }
        Some(mb) => println!("stems: {mb} MB free"),
        None => println!("stems: free memory not readable; going on"),
    }
    let all = mpfree_lanes::os::allowed_cores();
    let pin = match mpfree_lanes::os::pin_set(&[0]) {
        Ok(()) => "on core 0".to_string(),
        Err(e) => format!("not pinned ({e})"),
    };
    let plain = kernel_rate(Path::Plain);
    if mpfree_simd::neon_available() {
        let neon = kernel_rate(Path::Neon);
        println!(
            "stems: the GEMM kernel alone, {pin}, its data in the L1 cache: NEON {:.2} G multiply-adds a second, plain {:.2} G ({:.2} x)",
            neon / 1e9,
            plain / 1e9,
            neon / plain
        );
    } else {
        println!(
            "stems: the GEMM kernel alone, {pin}, its data in the L1 cache: plain {:.2} G multiply-adds a second (no NEON here)",
            plain / 1e9
        );
    }
    if !all.is_empty() {
        let _ = mpfree_lanes::os::pin_set(&all);
    }
    let t = Instant::now();
    let model = Model::synthetic(SEED);
    println!(
        "stems: Spleeter's 4-stem networks with seeded weights, built in {:.2} s: {:.1} MB of weights (halves)",
        t.elapsed().as_secs_f64(),
        model.bytes() as f64 / 1e6
    );
    let mut seed = SEED;
    let input: Vec<f32> = (0..SECONDS * RATE as usize * 2)
        .map(|_| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 0.5
        })
        .collect();
    println!("stems: before: {}", temperature_and_clock());
    let case = |what, band, threads, cores, neon| Case {
        what,
        band,
        threads,
        cores,
        neon,
    };
    let runs = [
        case(
            "the job's two cores, 1024 bins",
            Band::Spleeter,
            2,
            &[0, 1],
            true,
        ),
        case("one core, 1024 bins", Band::Spleeter, 1, &[0], true),
        case(
            "the kernels off, 1024 bins",
            Band::Spleeter,
            2,
            &[0, 1],
            false,
        ),
        case("full band, 1536 bins", Band::Full, 2, &[0, 1], true),
    ];
    let mut done = Vec::new();
    for c in &runs {
        match run_one(c, &model, &input, plain_build) {
            Ok(r) => done.push(r),
            Err(e) => {
                println!("stems: {e}");
                mpfree_simd::set_enabled(!plain_build);
                return ExitCode::from(1);
            }
        }
    }
    mpfree_simd::set_enabled(!plain_build);
    if !all.is_empty() {
        let _ = mpfree_lanes::os::pin_set(&all);
    }
    println!("stems: after: {}", temperature_and_clock());
    let (narrow, full) = (&done[0], &done[3]);
    println!(
        "stems: the kernels off against on at 1024 bins: {:.2} s against {:.2} s ({:.2} x); one core against two: {:.2} x",
        done[2].secs,
        narrow.secs,
        done[2].secs / narrow.secs,
        done[1].secs / narrow.secs
    );
    let est =
        |r: &Run, secs: f64| LOAD_S + segments(secs) as f64 * r.per_segment + secs * r.per_second;
    println!(
        "stems: estimate (two cores, the first split of a session, with {LOAD_S} s assumed to load the model): \
         a 10 s sample {:.1} s at 1024 bins, {:.1} s full band; a 60 s sample {:.1} s and {:.1} s",
        est(narrow, 10.0),
        est(full, 10.0),
        est(narrow, 60.0),
        est(full, 60.0)
    );
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_sample_makes_whole_segments() {
        assert_eq!(super::segments(10.0), 1);
        assert_eq!(super::segments(30.0), 3);
        assert_eq!(super::segments(60.0), 6);
    }
}
