//! Poly's Table wave (Wavetable folded into Poly): an oscillator reading
//! one of Wavetable's tables at a position, bent by a warp.

use std::sync::Arc;

use mpfree_node::{Ctx, Node, NodeEvent, Timed, MAX_BLOCK};
use mpfree_synth::params::*;
use mpfree_synth::wavetable::factory;
use mpfree_synth::wavetable::table::{Spectrum, Table};
use mpfree_synth::Poly;

const RATE: u32 = 48_000;

fn render(
    synth: &mut Poly,
    frames: usize,
    events: &[(u64, NodeEvent)],
    mut cut: impl FnMut(usize) -> usize,
) -> (Vec<f32>, Vec<f32>) {
    let mut l = vec![0.0f32; frames];
    let mut r = vec![0.0f32; frames];
    let (mut at, mut n) = (0usize, 0usize);
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

/// Power at `f` Hz (Hann-windowed DFT bin).
fn bin(x: &[f32], f: f64) -> f64 {
    let n = x.len();
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (i, &v) in x.iter().enumerate() {
        let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
        let ph = std::f64::consts::TAU * f * i as f64 / f64::from(RATE);
        re += f64::from(v) * w * ph.cos();
        im += f64::from(v) * w * ph.sin();
    }
    re * re + im * im
}

/// Harmonic `k`'s level against the fundamental, in dB, of note `note`.
fn harmonic_db(x: &[f32], note: u8, k: u32) -> f64 {
    let f0 = 440.0 * 2f64.powf((f64::from(note) - 69.0) / 12.0);
    10.0 * (bin(x, f0 * f64::from(k)) / bin(x, f0)).log10()
}

/// Poly with oscillator 1 on the Table wave and nothing else in the way:
/// oscillator 2, the filter's colour and the drive all out.
fn table_poly(table: f32, pos: f32) -> Poly {
    let mut s = Poly::new(RATE);
    s.set_param(OSC1_WAVE, f32::from(WAVE_TABLE));
    s.set_param(OSC1_TABLE, table);
    s.set_param(OSC1_POS, pos);
    s.set_param(OSC2_LEVEL, 0.0);
    s.set_param(CUTOFF, 20_000.0);
    s.set_param(RESONANCE, 0.0);
    s.set_param(FENV_AMOUNT, 0.0);
    s.set_param(KEY_TRACK, 0.0);
    s.set_param(AMP_A, 1.0);
    s.set_param(AMP_S, 100.0);
    s
}

fn steady(s: &mut Poly, note: u8) -> Vec<f32> {
    let (l, _) = render(s, 4_800 + 8_192, &[(0, on(note, 127))], |_| 128);
    l[4_800..].to_vec()
}

/// Basic's first frame is a sine and its last a square: the Table wave
/// plays what the table holds at its position.
#[test]
fn a_table_wave_plays_the_table_at_its_position() {
    let x = steady(&mut table_poly(0.0, 0.0), 57);
    for k in 2..=5 {
        let db = harmonic_db(&x, 57, k);
        assert!(db < -60.0, "sine frame: harmonic {k} at {db:.1} dB");
    }
    let x = steady(&mut table_poly(0.0, 100.0), 57);
    let h3 = harmonic_db(&x, 57, 3);
    let h5 = harmonic_db(&x, 57, 5);
    assert!(
        (h3 + 9.54).abs() < 1.0,
        "square frame: harmonic 3 at {h3:.1} dB"
    );
    assert!(
        (h5 + 13.98).abs() < 1.0,
        "square frame: harmonic 5 at {h5:.1} dB"
    );
    assert!(
        harmonic_db(&x, 57, 2) < -50.0,
        "a square has no even harmonics"
    );
}

/// The filter envelope sweeps the position down from the square to the
/// sine: bright at the attack, plain by the end of the decay.
#[test]
fn the_filter_envelope_moves_the_position() {
    let mut s = table_poly(0.0, 0.0);
    s.set_param(FENV_POS1, 100.0);
    s.set_param(FENV_A, 1.0);
    s.set_param(FENV_D, 300.0);
    s.set_param(FENV_S, 0.0);
    let (l, _) = render(&mut s, 48_000, &[(0, on(57, 127))], |_| 128);
    let early = harmonic_db(&l[480..480 + 2_048], 57, 3);
    let late = harmonic_db(&l[40_000..40_000 + 4_096], 57, 3);
    assert!(
        early > -14.0,
        "the attack reads near the square: h3 {early:.1} dB"
    );
    assert!(late < -50.0, "the decay ends on the sine: h3 {late:.1} dB");
}

/// Pressure on Position and a route on Warp do what they say.
#[test]
fn pressure_moves_the_position() {
    let mut s = table_poly(0.0, 0.0);
    s.set_param(press_target(0), f32::from(target::POSITION));
    s.set_param(press_depth(0), 100.0);
    let ev = [
        (0, on(57, 127)),
        (
            1_000,
            NodeEvent::Pressure {
                note: 57,
                value: 127,
            },
        ),
    ];
    let (l, _) = render(&mut s, 24_000, &ev, |_| 128);
    let pressed = harmonic_db(&l[12_000..12_000 + 8_192], 57, 3);
    assert!(
        pressed > -12.0,
        "full pressure reads the square: h3 {pressed:.1} dB"
    );
}

/// FM: oscillator 2 moves oscillator 1's phase, heard or not, and the
/// amount sets how far.
#[test]
fn the_fm_warp_reads_the_other_oscillator() {
    let fm = |amt: f32| {
        let mut s = table_poly(0.0, 0.0);
        s.set_param(OSC1_WARP_MODE, 4.0);
        s.set_param(OSC1_WARP, amt);
        s.set_param(OSC2_WAVE, 3.0);
        s.set_param(OSC2_OCT, 1.0);
        s.set_param(OSC2_FINE, 0.0);
        steady(&mut s, 57)
    };
    let plain = harmonic_db(&fm(0.0), 57, 3);
    let bent = harmonic_db(&fm(60.0), 57, 3);
    assert!(plain < -60.0, "no FM: a sine ({plain:.1} dB)");
    assert!(
        bent > plain + 30.0,
        "FM brings in sidebands: {plain:.1} -> {bent:.1} dB"
    );
}

/// Every table at every warp, at the last position, across the keyboard,
/// with both oscillators on FM moving each other: always in range.
#[test]
fn every_table_and_warp_stays_in_range_at_the_end_of_the_table() {
    for table in 0..factory().len() {
        for mode in [1.0, 2.0, 3.0, 4.0] {
            let mut s = Poly::new(RATE);
            for (wave, ids) in [OSC1_WAVE, OSC2_WAVE].into_iter().zip(TABLE_IDS) {
                s.set_param(wave, f32::from(WAVE_TABLE));
                s.set_param(ids[0], table as f32);
                s.set_param(ids[1], 100.0);
                s.set_param(ids[2], mode);
                s.set_param(ids[3], 100.0);
            }
            s.set_param(OSC2_LEVEL, 100.0);
            let ev: Vec<(u64, NodeEvent)> = (0..17u64)
                .map(|k| (k * 2_000, on(24 + 5 * k as u8, 100)))
                .collect();
            let (l, r) = render(&mut s, 36_000, &ev, |_| 256);
            assert!(
                l.iter().chain(&r).all(|x| x.is_finite() && x.abs() < 8.0),
                "table {table}, warp {mode}"
            );
        }
    }
}

/// Everything the Table wave does at once (every warp in turn, FM one way
/// and both ways, the envelope, LFO, velocity, pressure and a route on
/// the positions and warps, eight unison stacks, a table change mid-note,
/// Age): the same output however the host cuts the blocks.
#[test]
fn table_sounds_keep_the_same_output_for_any_block_size() {
    let mut ev = vec![
        (0, set(OSC1_WAVE, f32::from(WAVE_TABLE))),
        (0, set(OSC2_WAVE, f32::from(WAVE_TABLE))),
        (0, set(OSC1_TABLE, 4.0)),
        (0, set(OSC2_TABLE, 1.0)),
        (0, set(OSC1_POS, 20.0)),
        (0, set(OSC2_LEVEL, 70.0)),
        (0, set(UNISON, 8.0)),
        (0, set(FENV_POS1, 60.0)),
        (0, set(FENV_WARP, 40.0)),
        (0, set(LFO_POS2, 50.0)),
        (0, set(LFO_SYNC, 6.0)),
        (0, set(VEL_POS, -30.0)),
        (0, set(OSC1_WARP_MODE, 1.0)),
        (0, set(OSC1_WARP, 30.0)),
        (0, set(press_target(1), f32::from(target::WARP))),
        (0, set(press_depth(1), 50.0)),
        (0, set(route_id(1, route::SOURCE), 4.0)),
        (
            0,
            set(route_id(1, route::TARGET), f32::from(target::POSITION)),
        ),
        (0, set(route_id(1, route::DEPTH), 40.0)),
        (0, set(AGE, 60.0)),
        (5, on(48, 100)),
        (5, on(55, 70)),
        (7, on(60, 127)),
        (
            3_001,
            NodeEvent::Pressure {
                note: 60,
                value: 90,
            },
        ),
        (4_000, set(OSC1_WARP_MODE, 2.0)),
        (6_000, set(OSC1_WARP_MODE, 3.0)),
        (8_000, set(OSC1_WARP_MODE, 4.0)),
        (10_000, set(OSC2_TABLE, 7.0)),
        (11_111, off(55)),
        (12_000, set(OSC2_WARP_MODE, 4.0)),
        (12_000, set(OSC2_WARP, 60.0)),
        (16_000, set(OSC1_WARP_MODE, 0.0)),
        (18_000, set(OSC2_WAVE, 0.0)),
        (20_000, set(OSC1_LEVEL, 0.0)),
        (22_000, NodeEvent::AllOff),
    ];
    for k in 0..12u64 {
        ev.push((23_000 + k * 41, on(40 + 3 * k as u8, 60 + 5 * k as u8)));
    }
    ev.sort_by_key(|e| e.0);
    let (rl, rr) = render(&mut Poly::new(RATE), 30_000, &ev, |_| 128);
    assert!(rl.iter().any(|x| x.abs() > 1e-3));
    for size in [1usize, 7, 64, 333, MAX_BLOCK] {
        let (l, r) = render(&mut Poly::new(RATE), 30_000, &ev, |_| size);
        assert_eq!(l, rl, "left differs at block size {size}");
        assert_eq!(r, rr, "right differs at block size {size}");
    }
    let mut n = 0usize;
    let (l, r) = render(&mut Poly::new(RATE), 30_000, &ev, |_| {
        n += 1;
        [3, 128, 17, 250, 1][n % 5]
    });
    assert_eq!((l, r), (rl, rr), "irregular blocks differ");
}

/// A table loaded into an oscillator plays when its Table is User; the
/// one it replaces comes back to be dropped off the audio thread; User
/// with nothing loaded plays Basic.
#[test]
fn a_user_table_plays_when_its_table_is_user() {
    let mut s = Poly::new(RATE);
    s.set_param(OSC1_TABLE, 10.0);
    assert!(Arc::ptr_eq(s.table(0), &factory()[0]));
    // One frame: a sine's third harmonic alone, so it plays an octave and
    // a fifth up.
    let third =
        Arc::new(Table::from_spectra("third", &[Spectrum::from_sines(&[0.0, 0.0, 1.0])]).unwrap());
    assert!(s.set_user_table(0, third.clone()).is_none());
    assert!(Arc::ptr_eq(s.table(0), &third));
    let mut s2 = table_poly(10.0, 0.0);
    s2.set_user_table(0, third.clone());
    let x = steady(&mut s2, 45);
    assert!(
        harmonic_db(&x, 45, 3) > 40.0,
        "the user table plays its third harmonic"
    );
    let back = s.set_user_table(0, factory()[2].clone()).unwrap();
    assert!(Arc::ptr_eq(&back, &third));
}

/// Unison goes to eight stacks (as Wavetable's did) and they play at
/// about one stack's level.
#[test]
fn eight_unison_stacks_play_at_about_one_stacks_level() {
    let level = |stacks: f32| {
        let mut s = table_poly(0.0, 60.0);
        s.set_param(UNISON, stacks);
        let x = steady(&mut s, 57);
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    };
    let (one, eight) = (level(1.0), level(8.0));
    let db = 20.0 * (eight / one).log10();
    assert!(db.abs() < 3.0, "eight stacks against one: {db:+.1} dB");
}

/// The Table wave reads only harmonics below Nyquist at any pitch:
/// Basic's saw (frame 10 of 16) at C6, C7 and G7.
#[test]
fn high_table_notes_do_not_alias() {
    for note in [84u8, 96, 103] {
        let mut s = table_poly(0.0, 1000.0 / 15.0);
        let x = steady(&mut s, note);
        let f0 = 440.0 * 2f64.powf((f64::from(note) - 69.0) / 12.0);
        let fund = bin(&x, f0);
        let nyq = f64::from(RATE) / 2.0;
        let mut worst = 0.0f64;
        for n in 2..600 {
            let f = (n as f64 * f0) % f64::from(RATE);
            let f = if f > nyq { f64::from(RATE) - f } else { f };
            let h = (f / f0).round();
            if n as f64 * f0 <= nyq
                || (f - h * f0).abs() < 4.0 * f64::from(RATE) / x.len() as f64
                || f < 20.0
            {
                continue;
            }
            worst = worst.max(bin(&x, f));
        }
        let db = 10.0 * (worst / fund).log10();
        assert!(db < -50.0, "note {note}: alias at {db:.1} dB");
    }
}
