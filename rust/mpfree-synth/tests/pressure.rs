//! Pad pressure through the shared shaper into Poly's and FM's slots.

use mpfree_node::{Ctx, Node, NodeEvent, Timed, MAX_BLOCK};
use mpfree_synth::{fm_params as fp, params as pp, Fm, Poly};

const RATE: u32 = 48_000;
const MS: u64 = 48;

fn render(synth: &mut dyn Node, frames: usize, events: &[(u64, NodeEvent)], mut cut: impl FnMut(usize) -> usize) -> (Vec<f32>, Vec<f32>) {
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
        let ctx = Ctx { rate: RATE, frame: at as u64, bpm: 120.0, beats: at as f64 * 2.0 / RATE as f64, playing: true, sidechain: None };
        synth.process(&ctx, &buf, &mut l[at..at + len], &mut r[at..at + len]);
        at += len;
    }
    (l, r)
}

fn on(note: u8, velocity: u8) -> NodeEvent { NodeEvent::NoteOn { note, velocity } }
fn off(note: u8) -> NodeEvent { NodeEvent::NoteOff { note } }
fn press(note: u8, value: u8) -> NodeEvent { NodeEvent::Pressure { note, value } }
fn peak(x: &[f32]) -> f32 { x.iter().fold(0.0f32, |m, v| m.max(v.abs())) }
fn db(x: f32) -> f32 { 20.0 * x.max(1e-9).log10() }

/// A held chord, pressed (from 50 ms) at `value` on every note, or not.
fn chord_events(notes: &[u8], value: Option<u8>) -> Vec<(u64, NodeEvent)> {
    let mut ev: Vec<(u64, NodeEvent)> = notes.iter().map(|&n| (0, on(n, 127))).collect();
    if let Some(v) = value {
        for t in (50..1_500).step_by(5) {
            for &n in notes { ev.push((t * MS, press(n, v))); }
        }
    }
    for &n in notes { ev.push((1_500 * MS, off(n))); }
    ev.sort_by_key(|e| e.0);
    ev
}

/// Renders a preset's chord, unpressed and fully pressed, with `edit`
/// applied after loading. Returns the two peaks in dBFS.
fn poly_peaks(p: &pp::Preset, edit: impl Fn(&mut Poly)) -> (f32, f32) {
    let run = |value| {
        let mut s = Poly::new(RATE);
        s.load_preset(p);
        edit(&mut s);
        db(peak(&render(&mut s, 96_000, &chord_events(&[48, 55, 60, 64], value), |_| 256).0))
    };
    (run(None), run(Some(127)))
}

fn fm_peaks(p: &fp::FmPreset, edit: impl Fn(&mut Fm)) -> (f32, f32) {
    let run = |value| {
        let mut s = Fm::new(RATE);
        s.load_preset(p);
        edit(&mut s);
        db(peak(&render(&mut s, 96_000, &chord_events(&[48, 55, 60, 64], value), |_| 256).0))
    };
    (run(None), run(Some(127)))
}

#[test]
fn every_preset_stays_in_range_at_full_pressure() {
    for p in Poly::presets() {
        let (rest, full) = poly_peaks(p, |_| {});
        assert!((-30.0..=-1.0).contains(&rest) && (-30.0..=-1.0).contains(&full), "Poly {}: {rest:.1} / {full:.1} dBFS", p.name);
    }
    for p in Fm::presets() {
        let (rest, full) = fm_peaks(&p, |_| {});
        assert!((-30.0..=-1.0).contains(&rest) && (-30.0..=-1.0).contains(&full), "FM {}: {rest:.1} / {full:.1} dBFS", p.name);
    }
}

#[test]
fn a_level_slot_is_a_swell_never_a_boost() {
    // With the level slot, full pressure peaks no higher than the same
    // preset without it: the swell only takes level away.
    let mut checked = 0;
    for p in Poly::presets() {
        for n in 0..4 {
            let Some(&(_, t)) = p.values.iter().find(|(id, _)| *id == pp::press_target(n)) else { continue };
            if t as u8 != pp::target::LEVEL { continue; }
            let (rest, with) = poly_peaks(p, |_| {});
            let (_, without) = poly_peaks(p, |s| s.set_param(pp::press_target(n), 0.0));
            assert!(with <= without + 0.05, "Poly {}: {with:.2} with the swell, {without:.2} without", p.name);
            // Unpressed, it sits lower by about the swell's depth.
            assert!(rest < with - 3.0, "Poly {}: rest {rest:.1}, pressed {with:.1}", p.name);
            checked += 1;
        }
    }
    for p in Fm::presets() {
        for n in 0..4 {
            let Some(&(_, t)) = p.values.iter().find(|(id, _)| *id == fp::press_target(n)) else { continue };
            if t as u8 != fp::target::LEVEL { continue; }
            let (_, with) = fm_peaks(&p, |_| {});
            let (_, without) = fm_peaks(&p, |s| s.set_param(fp::press_target(n), 0.0));
            assert!(with <= without + 0.05, "FM {}: {with:.2} with the swell, {without:.2} without", p.name);
            checked += 1;
        }
    }
    assert_eq!(checked, 4, "Brass, Dust Pad, Brass Section and Silk Pad swell");
}

#[test]
fn plucks_and_mallets_ignore_pressure() {
    let p = Poly::presets().iter().find(|p| p.name == "Pluck").unwrap();
    let mut a = Poly::new(RATE);
    a.load_preset(p);
    let mut b = Poly::new(RATE);
    b.load_preset(p);
    assert_eq!(render(&mut a, 48_000, &chord_events(&[60], None), |_| 64), render(&mut b, 48_000, &chord_events(&[60], Some(127)), |_| 64), "Pluck");
    for name in ["Steel Pluck", "Log Drum"] {
        let p = Fm::presets().into_iter().find(|p| p.name == name).unwrap();
        let mut a = Fm::new(RATE);
        a.load_preset(&p);
        let mut b = Fm::new(RATE);
        b.load_preset(&p);
        let ra = render(&mut a, 48_000, &chord_events(&[60], None), |_| 64);
        let rb = render(&mut b, 48_000, &chord_events(&[60], Some(127)), |_| 64);
        assert_eq!(ra, rb, "{name}");
    }
}

/// Per-voice pressure after `frames`, from `pressures`.
fn poly_pressures(s: &mut Poly, frames: usize, ev: &[(u64, NodeEvent)]) -> Vec<f32> {
    render(s, frames, ev, |_| 100);
    let mut out = [0.0; 16];
    let n = s.pressures(&mut out);
    out[..n].to_vec()
}

#[test]
fn off_plays_the_preset_as_designed() {
    // Brass with pressure Off sounds as if it had no pressure slots: the
    // swell sits at full level, not at its resting dip.
    let p = Poly::presets().iter().find(|p| p.name == "Brass").unwrap();
    let ev = chord_events(&[48, 55, 60], Some(127));
    let mut off = Poly::new(RATE);
    off.load_preset(p);
    off.set_param(pp::PRESS_MODE, 2.0);
    let mut none = Poly::new(RATE);
    none.load_preset(p);
    for n in 0..4 {
        none.set_param(pp::press_target(n), 0.0);
    }
    assert_eq!(render(&mut off, 48_000, &ev, |_| 128), render(&mut none, 48_000, &ev, |_| 128));
    let p = Fm::presets().into_iter().find(|p| p.name == "Silk Pad").unwrap();
    let mut off = Fm::new(RATE);
    off.load_preset(&p);
    off.set_param(fp::PRESS_MODE, 2.0);
    let mut none = Fm::new(RATE);
    none.load_preset(&p);
    for n in 0..4 {
        none.set_param(fp::press_target(n), 0.0);
    }
    assert_eq!(render(&mut off, 48_000, &ev, |_| 128), render(&mut none, 48_000, &ev, |_| 128));
}

#[test]
fn per_note_pressure_moves_only_the_pressed_note() {
    let mut s = Poly::new(RATE);
    let ev = [(0, on(60, 100)), (0, on(64, 100)), (0, on(67, 100)), (50 * MS, press(64, 120))];
    let p = poly_pressures(&mut s, 200 * MS as usize, &ev);
    assert_eq!(p.len(), 3);
    assert_eq!(p[0], 0.0);
    assert!(p[1] > 0.9, "{p:?}");
    assert_eq!(p[2], 0.0);
    // The pressed voice is brighter: its filter is an octave up. Render
    // the same with and without the press and compare the high end.
    let hi = |x: &[f32]| x.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum::<f32>();
    let mut a = Poly::new(RATE);
    let mut b = Poly::new(RATE);
    let (la, _) = render(&mut a, 200 * MS as usize, &ev[..3], |_| 100);
    let (lb, _) = render(&mut b, 200 * MS as usize, &ev, |_| 100);
    let tail = 100 * MS as usize..;
    assert!(hi(&lb[tail.clone()]) > hi(&la[tail]) * 1.2);
}

#[test]
fn a_strike_spike_is_ignored_and_pressure_glides_in() {
    let mut s = Poly::new(RATE);
    // The pad spikes to full as it's hit, then settles at 70.
    let ev = [(0, on(60, 127)), (0, press(60, 127)), (10 * MS, press(60, 70))];
    for ms in [5u64, 15, 29] {
        let mut s = Poly::new(RATE);
        assert_eq!(poly_pressures(&mut s, (ms * MS) as usize, &ev), vec![0.0], "{ms} ms");
    }
    // After the hold-off it rises to the settled value, never the spike.
    let mut prev = 0.0;
    let mut frames = 30 * MS as usize;
    render(&mut s, frames, &ev, |_| 100);
    for _ in 0..40 {
        let (mut l, mut r) = ([0.0; 48], [0.0; 48]);
        s.process(&Ctx { rate: RATE, frame: frames as u64, bpm: 120.0, beats: 0.0, playing: false, sidechain: None }, &[], &mut l, &mut r);
        frames += 48;
        let mut out = [0.0; 1];
        s.pressures(&mut out);
        assert!(out[0] >= prev && out[0] < 0.8, "{} after {frames}", out[0]);
        prev = out[0];
    }
    assert!(prev > 0.3, "{prev}");
}

#[test]
fn release_hold_keeps_the_value_as_the_finger_lifts() {
    // Held at 110, lifted to 0 over 20 ms, then the note-off.
    let mut ev = vec![(0, on(60, 100))];
    for t in (40..300).step_by(5) { ev.push((t * MS, press(60, 110))); }
    for k in 0..10u64 { ev.push(((300 + k * 2) * MS, press(60, (110 - k * 11) as u8))); }
    ev.push((320 * MS, press(60, 0)));
    ev.push((321 * MS, off(60)));
    let at = |release: f32, ms: u64| {
        let mut s = Poly::new(RATE);
        s.set_param(pp::PRESS_RELEASE, release);
        s.set_param(pp::AMP_R, 2_000.0);
        poly_pressures(&mut s, (ms * MS) as usize, &ev)[0]
    };
    let before = at(0.0, 299);
    let hold = at(0.0, 600);
    assert!(hold > 0.6 * before, "Hold: {hold} of {before}");
    assert_eq!(at(0.0, 400), hold, "Hold keeps still");
    assert!(at(1.0, 600) < 0.01, "Follow follows the lift down");
    assert!(at(2.0, 600) < 0.01, "Fade falls to 0");
}

/// The pitch range (semitones, highest minus lowest) of a plain sine from
/// its zero crossings, over `x`.
fn pitch_range(x: &[f32]) -> f32 {
    let ups: Vec<f32> = x.windows(2).enumerate().filter(|(_, w)| w[0] <= 0.0 && w[1] > 0.0).map(|(i, w)| i as f32 + w[0] / (w[0] - w[1])).collect();
    let periods: Vec<f32> = ups.windows(2).map(|w| w[1] - w[0]).collect();
    let hi = periods.iter().cloned().fold(f32::MIN, f32::max);
    let lo = periods.iter().cloned().fold(f32::MAX, f32::min);
    12.0 * (hi / lo).log2()
}

#[test]
fn pressure_vibrato_and_pitch_move_the_pitch() {
    let sine = |s: &mut Poly| {
        for (id, v) in [(pp::OSC1_WAVE, 3.0), (pp::OSC2_LEVEL, 0.0), (pp::CUTOFF, 20_000.0), (pp::FENV_AMOUNT, 0.0), (pp::PRESS_HOLDOFF, 0.0)] {
            s.set_param(id, v);
        }
    };
    let ev = |v| [(0, on(69, 100)), (0, press(69, v))];
    // Vibrato at 100 %: a semitone either way at full pressure.
    let mut s = Poly::new(RATE);
    sine(&mut s);
    s.set_param(pp::press_target(0), f32::from(pp::target::VIBRATO));
    s.set_param(pp::press_depth(0), 100.0);
    let (l, _) = render(&mut s, 96_000, &ev(127), |_| 128);
    let r = pitch_range(&l[24_000..]);
    assert!((1.7..=2.3).contains(&r), "vibrato range {r} semitones");
    // Unpressed, the pitch holds.
    let mut s = Poly::new(RATE);
    sine(&mut s);
    s.set_param(pp::press_target(0), f32::from(pp::target::VIBRATO));
    s.set_param(pp::press_depth(0), 100.0);
    let (l, _) = render(&mut s, 96_000, &ev(0), |_| 128);
    assert!(pitch_range(&l[24_000..]) < 0.05);
    // Pitch at +100 % is an octave up at full pressure.
    let mut s = Poly::new(RATE);
    sine(&mut s);
    s.set_param(pp::press_target(0), f32::from(pp::target::PITCH));
    s.set_param(pp::press_depth(0), 100.0);
    s.set_param(pp::PRESS_CURVE, 1.0);
    let (l, _) = render(&mut s, 48_000, &ev(127), |_| 128);
    let ups = l[24_000..].windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
    assert!((ups as i32 - 440).abs() <= 2, "{ups} cycles in 0.5 s, wanted 440 (880 Hz)");
}

#[test]
fn highest_mode_swells_the_whole_chord() {
    let mut s = Poly::new(RATE);
    s.set_param(pp::PRESS_MODE, 1.0);
    let ev = [(0, on(60, 100)), (0, on(64, 100)), (50 * MS, press(64, 120)), (50 * MS, press(60, 30))];
    let p = poly_pressures(&mut s, 300 * MS as usize, &ev);
    assert!(p[0] > 0.9 && p[0] == p[1], "{p:?}");
    // Off: nothing moves.
    let mut s = Poly::new(RATE);
    s.set_param(pp::PRESS_MODE, 2.0);
    assert_eq!(poly_pressures(&mut s, 300 * MS as usize, &ev), vec![0.0, 0.0]);
}

#[test]
fn fm_brightness_brightens_without_getting_louder() {
    let p = Fm::presets().into_iter().find(|p| p.name == "Brass Section").unwrap();
    let only_bright = |s: &mut Fm| {
        // Slot 1 (the swell) off, so only brightness acts.
        s.set_param(fp::press_target(0), 0.0);
    };
    let run = |value| {
        let mut s = Fm::new(RATE);
        s.load_preset(&p);
        only_bright(&mut s);
        render(&mut s, 72_000, &chord_events(&[60], value), |_| 256).0
    };
    let (a, b) = (run(None), run(Some(127)));
    let w = 24_000..60_000;
    let hi = |x: &[f32]| x.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum::<f32>() / x.iter().map(|v| v * v).sum::<f32>();
    assert!(hi(&b[w.clone()]) > hi(&a[w.clone()]) * 1.3, "{} vs {}", hi(&b[w.clone()]), hi(&a[w.clone()]));
    // FM keeps each carrier's amplitude, so the loudness (RMS) holds.
    let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
    let (ra, rb) = (db(rms(&a[w.clone()])), db(rms(&b[w])));
    assert!((rb - ra).abs() < 1.0, "RMS {ra:.2} dB unpressed, {rb:.2} dB pressed");
}

/// Pressure-heavy phrases for the block-size check, in every mode.
fn pressure_events() -> Vec<(u64, NodeEvent)> {
    let mut ev = vec![(0, on(48, 100)), (0, on(55, 90)), (7, on(60, 127)), (3, press(60, 127))];
    let mut x = 0x9E37_79B9u32;
    for t in (100..40_000).step_by(37) {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        ev.push((t, press([48, 55, 60][(x % 3) as usize], (x >> 8) as u8 & 127)));
    }
    ev.push((11_111, off(55)));
    ev.push((14_000, on(55, 80)));
    ev.push((20_000, off(60)));
    ev.push((25_000, off(48)));
    ev.push((33_000, off(55)));
    ev.sort_by_key(|e| e.0);
    ev
}

#[test]
fn same_output_for_any_block_size_with_pressure() {
    for mode in 0..3 {
        for release in 0..3 {
            let setup = |s: &mut dyn Node, slots: &[(u16, u16, u8)]| {
                for &(t, d, target) in slots {
                    s.set_param(t, f32::from(target));
                    s.set_param(d, 60.0);
                }
            };
            let poly_slots: Vec<(u16, u16, u8)> = (0..4).map(|n| (pp::press_target(n), pp::press_depth(n), [1u8, 3, 4, 7][n])).collect();
            let fm_slots: Vec<(u16, u16, u8)> = (0..4).map(|n| (fp::press_target(n), fp::press_depth(n), [1u8, 2, 4, 7][n])).collect();
            let make: [Box<dyn Fn() -> Box<dyn Node>>; 2] = [
                Box::new(|| {
                    let mut s: Box<dyn Node> = Box::new(Poly::new(RATE));
                    s.set_param(pp::PRESS_MODE, mode as f32);
                    s.set_param(pp::PRESS_RELEASE, release as f32);
                    setup(&mut *s, &poly_slots);
                    s
                }),
                Box::new(|| {
                    let mut s: Box<dyn Node> = Box::new(Fm::new(RATE));
                    s.set_param(fp::PRESS_MODE, mode as f32);
                    s.set_param(fp::PRESS_RELEASE, release as f32);
                    setup(&mut *s, &fm_slots);
                    s
                }),
            ];
            let ev = pressure_events();
            for m in &make {
                let reference = render(&mut *m(), 40_000, &ev, |_| 40_000);
                assert!(peak(&reference.0) > 0.01);
                for size in [1usize, 7, 16, 100, 333] {
                    assert!(render(&mut *m(), 40_000, &ev, |_| size) == reference, "mode {mode} release {release} block {size}");
                }
            }
        }
    }
}
