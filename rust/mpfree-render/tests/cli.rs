//! The mpfree-render program, run as a whole: its WAV header, its length,
//! that it is the same every time, and that it refuses bad arguments.

use std::path::PathBuf;
use std::process::{Command, Output};

fn render(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mpfree-render")).args(args).output().expect("run mpfree-render")
}

fn out_path(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("mpfree-render-cli");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

#[test]
fn one_second_is_a_44_1_khz_stereo_s32_wav_with_two_beats() {
    let path = out_path("one-second.wav");
    let out = render(&[path.to_str().unwrap(), "1", "120"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("44100 frames; 2 beat triggers"), "{stdout}");

    let wav = std::fs::read(&path).unwrap();
    let frames = 44_100usize;
    assert_eq!(wav.len(), 44 + frames * 8);
    assert_eq!(&wav[0..4], b"RIFF");
    assert_eq!(u32_at(&wav, 4) as usize, wav.len() - 8);
    assert_eq!(&wav[8..16], b"WAVEfmt ");
    assert_eq!(u32_at(&wav, 16), 16);
    assert_eq!(u16_at(&wav, 20), 1, "integer PCM");
    assert_eq!(u16_at(&wav, 22), 2, "stereo");
    assert_eq!(u32_at(&wav, 24), 44_100);
    assert_eq!(u32_at(&wav, 28), 44_100 * 8, "byte rate");
    assert_eq!(u16_at(&wav, 32), 8, "block align");
    assert_eq!(u16_at(&wav, 34), 32, "bits per sample");
    assert_eq!(&wav[36..40], b"data");
    assert_eq!(u32_at(&wav, 40) as usize, frames * 8);
    // The click is audible, and the same in both channels.
    let samples: Vec<i32> = wav[44..].chunks_exact(4).map(|c| i32::from_le_bytes(c.try_into().unwrap())).collect();
    assert!(samples.iter().any(|&s| s != 0));
    assert!(samples.chunks_exact(2).all(|lr| lr[0] == lr[1]));
}

#[test]
fn the_same_arguments_render_the_same_bytes() {
    let a = out_path("same-a.wav");
    let b = out_path("same-b.wav");
    assert!(render(&[a.to_str().unwrap(), "0.5", "90"]).status.success());
    assert!(render(&[b.to_str().unwrap(), "0.5", "90"]).status.success());
    assert_eq!(std::fs::read(a).unwrap(), std::fs::read(b).unwrap());
}

#[test]
fn defaults_are_four_seconds_at_120_bpm() {
    let path = out_path("defaults.wav");
    let out = render(&[path.to_str().unwrap()]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("176400 frames; 8 beat triggers"));
}

#[test]
fn bad_arguments_fail_without_writing_a_file() {
    let path = out_path("never.wav");
    let p = path.to_str().unwrap();
    for (args, message) in [
        (vec![], "Usage"),
        (vec![p, "1", "120", "extra"], "Usage"),
        (vec![p, "0"], "seconds must be"),
        (vec![p, "61"], "seconds must be"),
        (vec![p, "NaN"], "seconds must be"),
        (vec![p, "1", "10"], "tempo must be"),
        (vec![p, "1", "401"], "tempo must be"),
        (vec![p, "one"], "invalid float"),
    ] {
        let out = render(&args);
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(message), "{args:?}: {stderr}");
        assert!(!path.exists(), "{args:?} wrote a file");
    }
}
