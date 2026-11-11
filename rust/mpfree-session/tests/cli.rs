//! The mpfree-session program, run as a whole: its report, that offline and
//! paced runs make the same audio, and that it refuses bad arguments.

use std::collections::HashMap;
use std::process::{Command, Output};

fn session(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mpfree-session")).args(args).output().expect("run mpfree-session")
}

fn report(out: &Output) -> HashMap<String, String> {
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut lines = stdout.lines();
    assert_eq!(lines.next(), Some("MPFREE_SESSION_V1"));
    lines.filter_map(|l| l.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

#[test]
fn offline_check_reports_the_whole_session() {
    let out = session(&["--offline-check"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("MPFREE_SESSION_RUST_WATCHDOG_7S_ARMED"));
    let r = report(&out);
    assert_eq!(r["status"], "PASS");
    assert_eq!(r["backend"], "null");
    assert_eq!(r["pacing"], "offline");
    assert_eq!(r["sample_rate_hz"], "44100");
    assert_eq!(r["period_frames"], "128");
    assert_eq!(r["frames"], "88200");
    assert_eq!(r["callback_count"], mpfree_session_callbacks());
    assert_eq!(r["deadline_misses"], "0");
    assert_eq!(r["late_starts"], "0");
    assert_eq!(r["groove_events"], "12");
    assert_eq!(r["sampler_notes"], "4");
    assert_eq!(r["checksum_fnv1a64"].len(), 16);
}

fn mpfree_session_callbacks() -> String {
    88_200usize.div_ceil(128).to_string()
}

#[test]
fn paced_self_test_makes_the_same_audio_as_offline() {
    let offline = report(&session(&["--offline-check"]));
    let paced = report(&session(&["--self-test"]));
    assert_eq!(paced["pacing"], "monotonic");
    assert_eq!(paced["status"], "PASS");
    assert_eq!(paced["checksum_fnv1a64"], offline["checksum_fnv1a64"]);
    assert_eq!(paced["frames"], offline["frames"]);
    // Two seconds of audio paced in real time take at least most of two seconds.
    let wall: u64 = paced["wall_elapsed_us"].parse().unwrap();
    assert!(wall >= 1_900_000, "paced run took only {wall} us");
}

#[test]
fn anything_but_one_mode_is_a_usage_error() {
    for args in [vec![], vec!["--run"], vec!["--self-test", "--offline-check"]] {
        let out = session(&args);
        assert_eq!(out.status.code(), Some(64), "{args:?}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("Usage"));
        assert!(out.stdout.is_empty());
    }
}
