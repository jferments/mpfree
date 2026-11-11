//! M7: finite, independently measurable null-audio session harness.
//!
//! There is *no* device HAL binding. The synthetic pacer approximates the
//! 128-frame period using std::thread::sleep and does not grant real-time
//! scheduling privileges, open ALSA or interact with the factory application.
//! No on-device launcher is authorized for M7 in this milestone.
#![forbid(unsafe_code)]

use mpfree_session::run;
use std::time::Duration;

const WATCHDOG_SECONDS: u64 = 7;

fn main() {
    let mut args = std::env::args();
    let _exe = args.next();
    let mode = args.next();
    if args.next().is_some() || !matches!(mode.as_deref(), Some("--self-test" | "--offline-check"))
    {
        eprintln!("Usage: mpfree-session --self-test|--offline-check");
        std::process::exit(64);
    }
    let paced = mode.as_deref() == Some("--self-test");
    if let Err(error) = std::thread::Builder::new()
        .name("mpfree-session-watchdog".to_string())
        .stack_size(64 * 1024)
        .spawn(|| {
            std::thread::sleep(Duration::from_secs(WATCHDOG_SECONDS));
            eprintln!("status=FAIL reason=M7_internal_watchdog_elapsed");
            std::process::abort();
        })
    {
        eprintln!("status=FAIL reason=watchdog_unavailable error={error}");
        std::process::exit(70);
    }
    eprintln!("MPFREE_SESSION_RUST_WATCHDOG_7S_ARMED");

    match run(paced) {
        Ok(s) => {
            println!("MPFREE_SESSION_V1");
            println!("os={}", std::env::consts::OS);
            println!("arch={}", std::env::consts::ARCH);
            println!("backend=null");
            println!("pacing={}", if paced { "monotonic" } else { "offline" });
            println!("sample_rate_hz=44100");
            println!("period_frames=128");
            println!("frames={}", s.audio_frames);
            println!("callback_count={}", s.callback_count);
            println!("deadline_misses={}", s.deadline_misses);
            println!("late_starts={}", s.late_starts);
            println!("max_callback_us={}", s.max_callback_ns.div_ceil(1000));
            println!("p99_callback_us={}", s.p99_callback_ns.div_ceil(1000));
            println!(
                "max_start_lateness_us={}",
                s.max_start_lateness_ns.div_ceil(1000)
            );
            println!("total_render_us={}", s.total_render_ns.div_ceil(1000));
            println!("wall_elapsed_us={}", s.wall_elapsed_ns.div_ceil(1000));
            println!("groove_events={}", s.audio.groove_events);
            println!("sampler_notes={}", s.audio.sampler_notes);
            println!("checksum_fnv1a64={:016x}", s.audio.sink.checksum_fnv1a64);
            println!("status=PASS");
        }
        Err(error) => {
            eprintln!("status=FAIL reason={error:?}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn watchdog_is_a_bounded_self_test_only() {
        assert_eq!(super::WATCHDOG_SECONDS, 7);
    }
}
