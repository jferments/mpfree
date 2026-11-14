//! Behaviour tests for the Wavetable synth through the mpfree-node interface.

use std::sync::Arc;

use mpfree_node::{check_params, Ctx, Node, NodeEvent, Timed, MAX_BLOCK};
use mpfree_synth::wavetable::params::*;
use mpfree_synth::wavetable::table::{Table, FRAME};
use mpfree_synth::wavetable::{factory, factory::factory_bytes};
use mpfree_synth::Wavetable;

const RATE: u32 = 48_000;

fn render(
    synth: &mut Wavetable,
    frames: usize,
    events: &[(u64, NodeEvent)],
    mut cut: impl FnMut(usize) -> usize,
) -> (Vec<f32>, Vec<f32>) {
    let mut l = vec![0.0f32; frames];
    let mut r = vec![0.0f32; frames];
    let mut at = 0usize;
    let mut n = 0usize;
    let mut buf = Vec::new();
    while at < frames {
        let len = cut(n).clamp(1, MAX_BLOCK).min(frames - at);
        n += 1;
        buf.clear();
        for &(f, ev) in events {
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
    (l, r)
}

fn on(note: u8, velocity: u8) -> NodeEvent {
    NodeEvent::NoteOn { note, velocity }
}
fn off(note: u8) -> NodeEvent {
    NodeEvent::NoteOff { note }
}
fn set(id: u16, value: f32) -> NodeEvent {
    NodeEvent::Param { id, value }
}
fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}
fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn busy_events() -> Vec<(u64, NodeEvent)> {
    let mut ev = vec![
        (0, set(UNISON, 3.0)),
        (0, set(LFO_SYNC, 6.0)),
        (0, set(LFO_POS_A, 50.0)),
        (0, set(MENV_POS_B, 80.0)),
        (0, set(osc_id(1, OSC_LEVEL), 60.0)),
        (0, set(osc_id(0, WARP_MODE), 4.0)),
        (0, set(osc_id(0, WARP), 30.0)),
        (5, on(48, 100)),
        (5, on(55, 90)),
        (7, on(60, 127)),
        (
            3_001,
            NodeEvent::Pressure {
                note: 60,
                value: 90,
            },
        ),
        (6_000, set(osc_id(1, WARP_MODE), 2.0)),
        (6_000, set(osc_id(1, WARP), 50.0)),
        (9_000, NodeEvent::Bend { value: 0.5 }),
        (10_000, set(TABLE, 4.0)),
        (11_111, off(55)),
        (12_000, set(CUTOFF, 900.0)),
        (13_000, set(osc_id(0, WARP_MODE), 1.0)),
        (14_000, set(osc_id(0, WARP_MODE), 3.0)),
        (15_013, set(FILTER_MODE, 2.0)),
    ];
    for k in 0..20u64 {
        ev.push((16_000 + k * 37, on(40 + k as u8, 64 + k as u8)));
    }
    ev.push((22_000, NodeEvent::AllOff));
    ev.push((26_000, set(VOICE_MODE, 2.0)));
    ev.push((26_000, set(GLIDE, 80.0)));
    ev.push((26_500, on(50, 120)));
    ev.push((28_000, on(57, 120)));
    ev.push((30_000, off(57)));
    ev.push((32_000, off(50)));
    ev.sort_by_key(|e| e.0);
    ev
}

#[test]
fn same_output_for_any_block_size() {
    let ev = busy_events();
    let frames = 40_000;
    let (rl, rr) = render(&mut Wavetable::new(RATE), frames, &ev, |_| 128);
    assert!(peak(&rl) > 0.01);
    for size in [1usize, 7, 16, 17, 100, 4096] {
        let (l, r) = render(&mut Wavetable::new(RATE), frames, &ev, |_| size);
        assert!(l == rl && r == rr, "block size {size} differs");
    }
}

#[test]
fn describes_itself() {
    let s = Wavetable::new(RATE);
    check_params(s.params()).unwrap();
    assert_eq!(s.info().id, "mpfree.wavetable");
    assert_eq!(TABLE_NAMES.len(), factory().len() + 1);
}

#[test]
fn every_preset_sounds_and_stays_in_range() {
    for preset in Wavetable::presets() {
        let mut s = Wavetable::new(RATE);
        s.load_preset(preset);
        let ev = [
            (0, on(48, 110)),
            (0, on(55, 100)),
            (0, on(60, 100)),
            (0, on(64, 90)),
            (30_000, off(48)),
            (30_000, off(55)),
            (30_000, off(60)),
            (30_000, off(64)),
        ];
        let (l, r) = render(&mut s, 48_000, &ev, |_| 128);
        let p = peak(&l).max(peak(&r));
        assert!(l.iter().chain(&r).all(|x| x.is_finite()), "{}", preset.name);
        assert!(p > 0.02 && p < 1.5, "{}: peak {p}", preset.name);
    }
}

#[test]
fn a_release_ends_in_silence_and_frees_voices() {
    let mut s = Wavetable::new(RATE);
    let ev = [(0, on(60, 100)), (4_800, off(60))];
    let (l, _) = render(&mut s, 48_000, &ev, |_| 256);
    assert!(rms(&l[1_000..4_000]) > 0.01);
    assert!(peak(&l[40_000..]) == 0.0);
    assert_eq!(s.active_voices(), 0);
}

#[test]
fn position_moves_through_the_table() {
    // Basic: sine at 0, square at the end. The square is much brighter.
    let bright = |pos: f32| {
        let mut s = Wavetable::new(RATE);
        s.set_param(osc_id(0, POSITION), pos);
        s.set_param(CUTOFF, 20_000.0);
        let (l, _) = render(&mut s, 9_600, &[(0, on(57, 127))], |_| 128);
        let d: Vec<f32> = l.windows(2).map(|w| w[1] - w[0]).collect();
        rms(&d[2_000..]) / rms(&l[2_000..])
    };
    let (sine, square) = (bright(0.0), bright(100.0));
    assert!(square > sine * 3.0, "sine {sine} square {square}");
}

/// Energy at `hz` in a Hann-windowed DFT (a rotating phasor, no trig per
/// sample).
fn bin(x: &[f32], hz: f64) -> f64 {
    let n = x.len();
    let w = std::f64::consts::TAU * hz / RATE as f64;
    let (dc, ds) = (w.cos(), w.sin());
    let (mut c, mut s) = (1.0f64, 0.0f64);
    let (mut re, mut im) = (0.0, 0.0);
    for (i, &v) in x.iter().enumerate() {
        let h = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
        re += f64::from(v) * h * c;
        im += f64::from(v) * h * s;
        let nc = c * dc - s * ds;
        s = c * ds + s * dc;
        c = nc;
    }
    re * re + im * im
}

/// A bright saw high up. Aliases of harmonic `n` (above Nyquist, or an
/// interpolation image) land at `n * f0` folded into 0 to Nyquist; probes
/// each fold for n up to 600 that is clear of a real harmonic and returns
/// the strongest relative to the fundamental, in dB.
fn worst_alias(note: u8) -> f64 {
    let mut s = Wavetable::new(RATE);
    // Basic's saw sits at frame 10 of 16.
    s.set_param(osc_id(0, POSITION), 1000.0 / 15.0);
    s.set_param(CUTOFF, 20_000.0);
    s.set_param(KEY_TRACK, 0.0);
    s.set_param(AMP_A, 0.5);
    let (l, _) = render(&mut s, 8_192 + 4_800, &[(0, on(note, 127))], |_| 128);
    let x = &l[4_800..];
    let f0 = 440.0 * 2f64.powf((f64::from(note) - 69.0) / 12.0);
    let fund = bin(x, f0);
    let nyq = RATE as f64 / 2.0;
    let mut worst = 0.0f64;
    for n in 2..600 {
        let f = (n as f64 * f0) % RATE as f64;
        let f = if f > nyq { RATE as f64 - f } else { f };
        let h = (f / f0).round();
        // Skip folds that land on (or within 4 bins of) a real harmonic, and
        // the real harmonics themselves.
        if n as f64 * f0 <= nyq
            || (f - h * f0).abs() < 4.0 * RATE as f64 / x.len() as f64
            || f < 20.0
        {
            continue;
        }
        worst = worst.max(bin(x, f));
    }
    10.0 * (worst / fund).log10()
}

#[test]
fn high_notes_do_not_alias() {
    for note in [84u8, 96, 103] {
        let db = worst_alias(note);
        eprintln!("note {note}: worst alias {db:.1} dB");
        assert!(db < -50.0, "note {note}: alias at {db:.1} dB");
    }
}

#[test]
fn a_user_table_plays_and_swaps_back() {
    let mut s = Wavetable::new(RATE);
    let sq: Vec<f32> = (0..FRAME)
        .map(|i| if i < FRAME / 2 { 1.0 } else { -1.0 })
        .collect();
    let t = Arc::new(Table::from_frames("square", &sq, FRAME).unwrap());
    assert!(s.set_user_table(0, t.clone()).is_none());
    s.set_param(TABLE, USER_TABLE);
    assert!(Arc::ptr_eq(s.table(0), &t));
    let (l, _) = render(&mut s, 4_800, &[(0, on(60, 100))], |_| 128);
    assert!(rms(&l[1_000..]) > 0.01);
    // Replacing hands the old one back; the node never drops the last copy.
    let t2 = Arc::new(Table::from_frames("square2", &sq, FRAME).unwrap());
    let old = s.set_user_table(0, t2).unwrap();
    assert!(Arc::ptr_eq(&old, &t));
    assert_eq!(Arc::strong_count(&t), 2);
    // Back on a factory table.
    s.set_param(TABLE, 2.0);
    assert!(Arc::ptr_eq(s.table(0), &factory()[2]));
    // "User" with nothing loaded plays Basic.
    let mut s = Wavetable::new(RATE);
    s.set_param(osc_id(1, TABLE), USER_TABLE);
    assert!(Arc::ptr_eq(s.table(1), &factory()[0]));
}

#[test]
fn factory_tables_fit_the_memory_budget() {
    let mb = factory_bytes() as f64 / 1e6;
    eprintln!("factory tables: {mb:.2} MB");
    assert!(mb < 10.0);
}

/// As Poly's: turning the drive up keeps every preset within 2 dB of its
/// level with no drive ("Drive seems to make synths quieter", live test
/// 2). A static scale alone left the Organ 12 dB louder at full drive.
#[test]
fn turning_the_drive_up_keeps_every_preset_about_as_loud() {
    let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
    let level = |preset: &mpfree_synth::Preset, id: u16, drive: f32, notes: &[u8]| {
        let mut s = Wavetable::new(RATE);
        s.load_preset(preset);
        s.set_param(id, drive);
        let ev: Vec<(u64, NodeEvent)> = notes.iter().map(|&n| (0, on(n, 110))).collect();
        let (l, r) = render(&mut s, 36_000, &ev, |_| 128);
        20.0 * (rms(&l[12_000..]) + rms(&r[12_000..])).log10()
    };
    for preset in Wavetable::presets() {
        for (id, what) in [(DRIVE, "drive"), (DIST, "distortion")] {
            for notes in [&[60u8][..], &[48, 55, 60, 64]] {
                let dry = level(preset, id, 0.0, notes);
                for drive in [3.0, 6.0, 12.0, 18.0, 24.0] {
                    let change = level(preset, id, drive, notes) - dry;
                    assert!(
                        (-2.0..=2.0).contains(&change),
                        "{} {notes:?}: {drive} dB of {what} changes the level by {change:+.1} dB",
                        preset.name
                    );
                }
            }
        }
    }
}
