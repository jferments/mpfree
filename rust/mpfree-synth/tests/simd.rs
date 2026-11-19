//! Poly's supersaw, the divide-down bank of Strings and Drawbar's combo
//! organ, and the Choir's formants, through `mpfree_simd`: the same
//! samples with the NEON kernels on and off. On the workstation both are the plain twin; as
//! ARMv7 under qemu-arm (CI's `rust-arm` job runs these) the first is the
//! NEON kernels, so this checks whole sounds, end to end, bit for bit
//! (docs/dev/design/simd.md).
//!
//! Turning the kernels off is program-wide, so this file's renders take a
//! lock: each "on" render really runs the kernel. Other tests in the same
//! program can't hear the switch, since both paths give the same samples.

use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_synth::params::*;
use mpfree_synth::{Choir, Drawbar, Poly, Strings};

const RATE: u32 = 44_100;

fn on(note: u8, velocity: u8) -> NodeEvent {
    NodeEvent::NoteOn { note, velocity }
}

/// Two seconds of chords in uneven blocks (so the kernel's tails of 1 to 3
/// frames are played too), left then right, as bits.
fn render(synth: &mut dyn Node, setup: &[(u16, f32)]) -> Vec<u32> {
    let mut events: Vec<(u64, NodeEvent)> = setup
        .iter()
        .map(|&(id, value)| (0, NodeEvent::Param { id, value }))
        .collect();
    for (i, chord) in [[36u8, 48, 55, 64], [41, 53, 60, 69], [43, 55, 62, 71]]
        .iter()
        .enumerate()
    {
        let at = 2_000 + i as u64 * 25_000;
        for (j, &n) in chord.iter().enumerate() {
            events.push((at + j as u64 * 37, on(n, 60 + 20 * j as u8)));
            events.push((at + 20_000, NodeEvent::NoteOff { note: n }));
        }
    }
    let frames = 2 * RATE as usize;
    let (mut l, mut r) = (vec![0.0f32; frames], vec![0.0f32; frames]);
    let (mut at, mut n) = (0usize, 0usize);
    let mut buf = Vec::new();
    while at < frames {
        let len = [128, 61, 7, 128, 3, 100][n % 6].min(frames - at);
        n += 1;
        buf.clear();
        for &(f, ev) in &events {
            if f >= at as u64 && f < (at + len) as u64 {
                buf.push(Timed::new((f - at as u64) as u32, ev));
            }
        }
        let ctx = Ctx {
            rate: RATE,
            frame: at as u64,
            bpm: 120.0,
            beats: at as f64 * 2.0 / RATE as f64,
            playing: true,
            sidechain: None,
        };
        synth.process(&ctx, &buf, &mut l[at..at + len], &mut r[at..at + len]);
        at += len;
    }
    l.iter().chain(&r).map(|x| x.to_bits()).collect()
}

static SWITCH: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Renders a preset (or Init with `setup`) with the kernels on, then off.
fn both(preset: Option<&str>, setup: &[(u16, f32)]) -> (Vec<u32>, Vec<u32>) {
    let _one_at_a_time = SWITCH.lock().unwrap_or_else(|e| e.into_inner());
    let make = || {
        let mut p = Poly::new(RATE);
        if let Some(name) = preset {
            let pr = Poly::presets()
                .iter()
                .find(|p| p.name == name)
                .unwrap_or_else(|| panic!("preset {name}"));
            p.load_preset(pr);
        }
        p
    };
    let kernel = render(&mut make(), setup);
    mpfree_simd::set_enabled(false);
    let plain = render(&mut make(), setup);
    mpfree_simd::set_enabled(true);
    // Poly's tables always fit the kernel: no call fell back.
    assert_eq!(mpfree_simd::fallbacks(), 0);
    (kernel, plain)
}

#[test]
fn the_supersaw_presets_sound_the_same_with_the_kernels_off() {
    let names: Vec<&str> = Poly::presets()
        .iter()
        .map(|p| p.name)
        .filter(|n| n.starts_with("Super"))
        .collect();
    assert!(names.len() >= 4, "{names:?}");
    for name in names {
        let (kernel, plain) = both(Some(name), &[]);
        assert!(kernel.iter().any(|&b| b != 0), "{name} is silent");
        assert!(kernel == plain, "{name}: the kernel's samples differ");
    }
}

/// Both oscillators on Supersaw (the second adds to the first), with
/// unison copies spread apart: every way Poly calls the saws.
#[test]
fn both_oscillators_on_supersaw_with_unison_sound_the_same() {
    let setup = [
        (OSC1_WAVE, 4.0),
        (OSC2_WAVE, 4.0),
        (OSC2_LEVEL, 70.0),
        (OSC2_SEMI, 7.0),
        (UNISON, 3.0),
        (UNISON_SPREAD, 80.0),
        (SUPER_DETUNE, 85.0),
        (SUPER_MIX, 30.0),
    ];
    let (kernel, plain) = both(None, &setup);
    assert!(kernel.iter().any(|&b| b != 0));
    assert!(kernel == plain, "the kernel's samples differ");
}

/// Renders `make()` with `setup` with the kernels on, then off.
fn both_of(make: impl Fn() -> Box<dyn Node>, setup: &[(u16, f32)]) -> (Vec<u32>, Vec<u32>) {
    let _one_at_a_time = SWITCH.lock().unwrap_or_else(|e| e.into_inner());
    let made = || {
        let mut n = make();
        n.prepare(RATE, mpfree_node::MAX_BLOCK);
        n
    };
    let kernel = render(made().as_mut(), setup);
    mpfree_simd::set_enabled(false);
    let plain = render(made().as_mut(), setup);
    mpfree_simd::set_enabled(true);
    (kernel, plain)
}

/// Every Strings preset (its six saw buses) and the combo organ's presets
/// (its two buses of different shapes): four-key chords give the bank
/// eight to twelve notes, so it renders four at a time and the rest one
/// at a time.
#[test]
fn strings_and_the_combo_organ_sound_the_same_with_the_kernels_off() {
    let mut played = 0;
    for p in Strings::presets() {
        let (kernel, plain) = both_of(|| Box::new(Strings::new(RATE)), p.values);
        assert!(
            kernel.iter().any(|&b| b != 0),
            "Strings {} is silent",
            p.name
        );
        assert!(
            kernel == plain,
            "Strings {}: the kernels' samples differ",
            p.name
        );
        played += 1;
    }
    for p in Drawbar::presets()
        .iter()
        .filter(|p| p.name.starts_with("Combo"))
    {
        let (kernel, plain) = both_of(|| Box::new(Drawbar::new(RATE)), p.values);
        assert!(
            kernel.iter().any(|&b| b != 0),
            "Drawbar {} is silent",
            p.name
        );
        assert!(
            kernel == plain,
            "Drawbar {}: the kernels' samples differ",
            p.name
        );
        played += 1;
    }
    assert!(played >= 4, "{played}");
}

/// Every Choir preset: four-note chords sung through the five formants,
/// four voices a vector. Flush-to-zero is on, as on the app's sound
/// threads: NEON always flushes denormals, so without it a filter dying
/// through them could differ from the twin under 1e-38 (it doesn't today:
/// the Choir lets its voices go first).
#[test]
fn the_choir_presets_sound_the_same_with_the_kernel_off() {
    assert!(mpfree_lanes::fpu::flush_to_zero());
    let mut played = 0;
    for p in Choir::presets() {
        let (kernel, plain) = both_of(|| Box::new(Choir::new(RATE)), p.values);
        assert!(kernel.iter().any(|&b| b != 0), "Choir {} is silent", p.name);
        assert!(
            kernel == plain,
            "Choir {}: the kernel's samples differ",
            p.name
        );
        played += 1;
    }
    assert!(played >= 20, "{played}");
}

#[test]
fn the_kernel_runs_where_neon_is() {
    if cfg!(all(target_arch = "arm", target_os = "linux")) {
        assert!(mpfree_simd::neon_available(), "{}", mpfree_simd::status());
    }
}
