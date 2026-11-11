//! The mpfree-device-smoke program, run as a whole on the host: its report,
//! that it is the same every run, and that it refuses bad arguments.

use std::process::{Command, Output};

fn smoke(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mpfree-device-smoke")).args(args).output().expect("run mpfree-device-smoke")
}

#[test]
fn self_test_passes_with_a_stable_report() {
    let first = smoke(&["--self-test"]);
    assert!(first.status.success(), "{}", String::from_utf8_lossy(&first.stderr));
    assert!(String::from_utf8_lossy(&first.stderr).contains("WATCHDOG=RUST_5S_ARMED"));
    let stdout = String::from_utf8_lossy(&first.stdout).into_owned();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.first(), Some(&"MPFREE_ARMV7_SMOKE_V1"));
    assert_eq!(lines.last(), Some(&"status=PASS"));
    for want in ["sample_rate_hz=44100", "frames=4096", "beats=1"] {
        assert!(lines.contains(&want), "missing {want}: {stdout}");
    }
    assert!(lines.iter().any(|l| l.starts_with("checksum_fnv1a64=") && l.len() == "checksum_fnv1a64=".len() + 16));
    // The audio is computed, not timed, so a second run reports the same.
    assert_eq!(smoke(&["--self-test"]).stdout, first.stdout);
}

#[test]
fn anything_but_self_test_is_a_usage_error() {
    for args in [vec![], vec!["--selftest"], vec!["--self-test", "again"]] {
        let out = smoke(&args);
        assert_eq!(out.status.code(), Some(64), "{args:?}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("Usage: mpfree-device-smoke --self-test"));
        assert!(out.stdout.is_empty());
    }
}
