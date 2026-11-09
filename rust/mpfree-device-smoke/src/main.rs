//! Minimal RAM-only userspace test for the original MPC One.
//! This executable intentionally opens NO ALSA/DRM/evdev/MIDI/USB/network
//! devices, invokes no systemd commands, and never writes a file.
//! Its only output is a small human-readable self-test report on stdout.
#![forbid(unsafe_code)]

use mpfree_core::ClickEngine;

// Independent Rust watchdog thread requires no on-device timeout/nice utility.
// A permanently unschedulable process is still beyond any userspace watchdog.
const WALL_WATCHDOG_SECONDS: u64 = 5;

fn arm_self_watchdog() -> Result<(), std::io::Error> {
    std::thread::Builder::new()
        .name("mpfree-smoke-watchdog".to_string())
        .stack_size(64 * 1024)
        .spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(WALL_WATCHDOG_SECONDS));
            eprintln!("status=FAIL reason=rust_watchdog_deadline_exceeded");
            std::process::abort();
        })?;
    Ok(())
}

const BLOCK: usize = 128;
const FRAMES: usize = 4096;
const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

#[derive(Debug, PartialEq, Eq)]
struct ProbeSummary {
    frames: usize,
    beats: u64,
    nonzero_samples: usize,
    checksum: u64,
}

fn self_test() -> Result<ProbeSummary, &'static str> {
    let mut engine = ClickEngine::new();
    let mut left = [0.0_f32; BLOCK];
    let mut right = [0.0_f32; BLOCK];
    let mut nonzero_samples = 0;
    let mut checksum = FNV_OFFSET;

    for _ in 0..(FRAMES / BLOCK) {
        engine.render(&mut left, &mut right).map_err(|_| "channel mismatch")?;
        for (&l, &r) in left.iter().zip(right.iter()) {
            if !l.is_finite() || !r.is_finite() || l != r {
                return Err("nonfinite or nonidentical stereo samples");
            }
            nonzero_samples += usize::from(l != 0.0);
            for byte in l.to_bits().to_le_bytes() {
                checksum ^= u64::from(byte);
                checksum = checksum.wrapping_mul(FNV_PRIME);
            }
        }
    }

    let summary = ProbeSummary {
        frames: FRAMES,
        beats: engine.beats_emitted(),
        nonzero_samples,
        checksum,
    };
    if engine.rendered_frames() != FRAMES as u64
        || summary.beats != 1
        || summary.nonzero_samples < 10
        || summary.checksum == FNV_OFFSET
    {
        return Err("self-test audio calculation failed");
    }
    Ok(summary)
}

fn main() {
    let mut args = std::env::args();
    let _program = args.next();
    if args.next().as_deref() != Some("--self-test") || args.next().is_some() {
        eprintln!("Usage: mpfree-device-smoke --self-test");
        std::process::exit(64);
    }
    if let Err(error) = arm_self_watchdog() {
        eprintln!("status=FAIL reason=watchdog_unavailable error={error}");
        std::process::exit(70);
    }
    eprintln!("WATCHDOG=RUST_5S_ARMED");
    match self_test() {
        Ok(s) => {
            println!("MPFREE_ARMV7_SMOKE_V1");
            println!("os={}", std::env::consts::OS);
            println!("arch={}", std::env::consts::ARCH);
            println!("sample_rate_hz=44100");
            println!("frames={}", s.frames);
            println!("beats={}", s.beats);
            println!("nonzero_samples={}", s.nonzero_samples);
            println!("checksum_fnv1a64={:016x}", s.checksum);
            println!("status=PASS");
        }
        Err(reason) => {
            eprintln!("status=FAIL reason={reason}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finite_and_bounded_rust_dsp() {
        let got = self_test().unwrap();
        assert_eq!(got.frames, FRAMES);
        assert_eq!(got.beats, 1);
        assert!(got.nonzero_samples >= 10);
        assert_ne!(got.checksum, FNV_OFFSET);
    }
}
