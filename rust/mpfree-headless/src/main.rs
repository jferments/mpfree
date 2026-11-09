//! Finite standalone MPFree sequencer + sampler, exclusively NullAudioBackend.
//! Deliberately does not open real ALSA/MIDI/DRM/input/network devices, and
//! does not stop or signal factory services. No persistent storage required.
#![forbid(unsafe_code)]

use mpfree_headless::self_test;
use std::time::{Duration, Instant};

/// Passive non-persistent /proc metrics: read only *after* the audio render,
/// never inside a render callback or a timing-critical code path.
fn proc_status_kib(field: &str, status: &str) -> Option<u64> {
    status.lines().find_map(|line| {
        let (label, rest) = line.split_once(':')?;
        if label != field {
            return None;
        }
        let mut tokens = rest.split_whitespace();
        let number = tokens.next()?.parse::<u64>().ok()?;
        (tokens.next() == Some("kB") && tokens.next().is_none()).then_some(number)
    })
}

fn print_memory_metrics() {
    match std::fs::read_to_string("/proc/self/status") {
        Ok(status) => {
            for (field, name) in [("VmHWM", "peak_rss_kib"), ("VmRSS", "rss_kib")] {
                match proc_status_kib(field, &status) {
                    Some(kib) => println!("{name}={kib}"),
                    None => println!("{name}=unavailable"),
                }
            }
        }
        Err(_) => {
            println!("peak_rss_kib=unavailable");
            println!("rss_kib=unavailable");
        }
    }
}

const WATCHDOG_SECONDS: u64 = 5;
const WATCHDOG_MARKER: &str = "MPFREE_HEADLESS_RUST_WATCHDOG_5S_ARMED";

fn main() {
    let mut args = std::env::args();
    let _exe = args.next();
    if args.next().as_deref() != Some("--self-test") || args.next().is_some() {
        eprintln!("Usage: mpfree-headless --self-test");
        std::process::exit(64);
    }

    // A bounded smoke-test watchdog, NOT an independent DAW supervisor.
    if let Err(err) = std::thread::Builder::new()
        .name("mpfree-headless-watchdog".to_string())
        .stack_size(64 * 1024)
        .spawn(|| {
            std::thread::sleep(Duration::from_secs(WATCHDOG_SECONDS));
            eprintln!("status=FAIL reason=headless_watchdog_exceeded");
            std::process::abort();
        })
    {
        eprintln!("status=FAIL reason=watchdog_spawn error={err}");
        std::process::exit(70);
    }
    eprintln!("{WATCHDOG_MARKER}");

    let begin = Instant::now();
    match self_test() {
        Ok(s) => {
            println!("MPFREE_HEADLESS_V1");
            println!("os={}", std::env::consts::OS);
            println!("arch={}", std::env::consts::ARCH);
            println!("backend=null");
            println!("sample_rate_hz=44100");
            println!("frames={}", s.sink.frames);
            println!("groove_events={}", s.groove_events);
            println!("sampler_notes={}", s.sampler_notes);
            println!("groove_voice_steals={}", s.groove_stolen_voices);
            println!("sampler_voice_steals={}", s.sampler_stolen_voices);
            println!("prepared_bytes={}", s.prepared_bytes);
            println!("nonzero_samples={}", s.sink.nonzero_samples);
            println!("peak_abs={:.6}", s.sink.peak_abs);
            println!("checksum_fnv1a64={:016x}", s.sink.checksum_fnv1a64);
            println!("elapsed_us={}", begin.elapsed().as_micros());
            print_memory_metrics();
            println!("status=PASS");
        }
        Err(err) => {
            eprintln!("status=FAIL reason={err}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn parse_linux_proc_memory_in_kib_without_unsafe_code() {
        let status = "Name:\tmpfree-headless\nVmRSS:\t  3020 kB\nVmHWM:\t4096 kB\n";
        assert_eq!(super::proc_status_kib("VmRSS", status), Some(3020));
        assert_eq!(super::proc_status_kib("VmHWM", status), Some(4096));
        assert_eq!(super::proc_status_kib("VmRSS", "VmRSS:\tnot-a-number kB\n"), None);
        assert_eq!(super::proc_status_kib("VmRSS", "VmRSS:\t20 MB\n"), None);
        assert_eq!(super::proc_status_kib("VmRSS", "VmRSS:\t20 kB extra\n"), None);
        assert_eq!(super::proc_status_kib("VmRSS", "VmRSSBad:\t20 kB\n"), None);
    }

    #[test]
    fn watchdog_constant_is_positive_and_bounded() {
        assert!((1..=5).contains(&super::WATCHDOG_SECONDS));
        assert!(super::WATCHDOG_MARKER.contains("WATCHDOG_5S_ARMED"));
    }
}
