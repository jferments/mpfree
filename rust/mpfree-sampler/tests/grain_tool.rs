//! The `mpfree-grain` listening tool: it runs, writes every preset, and the
//! same seed gives the same files.

use std::path::{Path, PathBuf};
use std::process::Command;

fn demo(name: &str, seed: &str) -> PathBuf {
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&out);
    let run = Command::new(env!("CARGO_BIN_EXE_mpfree-grain"))
        .arg("--demo").arg(&out).args(["--seconds", "2", "--seed", seed])
        .output().unwrap();
    let text = String::from_utf8_lossy(&run.stdout);
    assert!(run.status.success(), "{text}\n{}", String::from_utf8_lossy(&run.stderr));
    assert!(text.contains("result: PASS"), "{text}");
    out
}

#[test]
fn the_grain_tool_renders_every_preset_repeatably() {
    let a = demo("mpfree-grain-a", "7");
    let presets = ["cloud", "stretch", "shimmer", "reverse-spray", "stutter-120", "freeze-live", "pressure-pad"];
    for p in presets {
        let s = mpfree_sampler::PreparedSample::read_wav(&a.join(format!("{p}.wav"))).unwrap();
        assert_eq!(s.sample_rate(), 44_100);
        // Stretch is four times the 9.25 s demo, capped at a minute; the rest are 2 s.
        let want = if p == "stretch" { 4 * (44_100 * 4 + 220_500 + 11_025) } else { 88_200 };
        assert_eq!(s.frames(), want, "{p}");
    }
    assert!(a.join("grain-report.txt").is_file() && a.join("demo-input.wav").is_file());
    let b = demo("mpfree-grain-b", "7");
    let c = demo("mpfree-grain-c", "8");
    let read = |dir: &Path, f: &str| std::fs::read(dir.join(f)).unwrap();
    for p in presets {
        let f = format!("{p}.wav");
        assert_eq!(read(&a, &f), read(&b, &f), "{p} differs with the same seed");
    }
    assert_ne!(read(&a, "cloud.wav"), read(&c, "cloud.wav"));
    let bad = Command::new(env!("CARGO_BIN_EXE_mpfree-grain")).arg("--demo").output().unwrap();
    assert!(!bad.status.success(), "usage error without an output folder");
}
