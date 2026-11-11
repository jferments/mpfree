//! Behaviour tests for the Poly synth through the mpfree-node interface.

use mpfree_node::{check_params, Ctx, Node, NodeEvent, Timed, MAX_BLOCK};
use mpfree_synth::params::*;
use mpfree_synth::Poly;

const RATE: u32 = 48_000;

/// Renders `frames` frames with `events` (absolute frames), cut into blocks
/// by `cut(i)`, and returns the left and right channels.
fn render(synth: &mut Poly, frames: usize, events: &[(u64, NodeEvent)], mut cut: impl FnMut(usize) -> usize) -> (Vec<f32>, Vec<f32>) {
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
            if f >= at as u64 && f < (at + len) as u64 { buf.push(Timed::new((f - at as u64) as u32, ev)); }
        }
        let ctx = Ctx { rate: RATE, frame: at as u64, bpm: 120.0, beats: at as f64 * 2.0 / RATE as f64, playing: true, sidechain: None };
        synth.process(&ctx, &buf, &mut l[at..at + len], &mut r[at..at + len]);
        at += len;
    }
    (l, r)
}

fn on(note: u8, velocity: u8) -> NodeEvent { NodeEvent::NoteOn { note, velocity } }
fn off(note: u8) -> NodeEvent { NodeEvent::NoteOff { note } }

fn peak(x: &[f32]) -> f32 { x.iter().fold(0.0f32, |m, v| m.max(v.abs())) }

/// A busy phrase: chords, a mono switch, glide, bend, pressure, parameter
/// automation, unison, a voice overflow and AllOff.
fn busy_events() -> Vec<(u64, NodeEvent)> {
    let mut ev = vec![
        (0, NodeEvent::Param { id: UNISON, value: 3.0 }),
        (0, NodeEvent::Param { id: LFO_SYNC, value: 6.0 }),
        (0, NodeEvent::Param { id: LFO_CUTOFF, value: 1.0 }),
        (0, NodeEvent::Param { id: DRIVE, value: 10.0 }),
        (5, on(48, 100)),
        (5, on(55, 90)),
        (7, on(60, 127)),
        (3_001, NodeEvent::Pressure { note: 60, value: 90 }),
        (9_000, NodeEvent::Bend { value: 0.5 }),
        (11_111, off(55)),
        (12_000, NodeEvent::Param { id: CUTOFF, value: 300.0 }),
        (15_013, NodeEvent::Param { id: FILTER_MODE, value: 2.0 }),
    ];
    // More notes than voices, so stealing happens.
    for k in 0..20u64 { ev.push((16_000 + k * 37, on(40 + k as u8, 64 + k as u8))); }
    ev.push((22_000, NodeEvent::AllOff));
    ev.push((26_000, NodeEvent::Param { id: VOICE_MODE, value: 2.0 }));
    ev.push((26_000, NodeEvent::Param { id: GLIDE, value: 80.0 }));
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
    let frames = 48_000;
    let (rl, rr) = render(&mut Poly::new(RATE), frames, &ev, |_| 128);
    assert!(peak(&rl) > 0.01);
    for size in [1usize, 7, 16, 17, 64, 1000, 4096, 8192] {
        let (l, r) = render(&mut Poly::new(RATE), frames, &ev, |_| size);
        assert!(l == rl && r == rr, "block size {size} differs");
    }
    let mut seed = 12345u64;
    let (l, r) = render(&mut Poly::new(RATE), frames, &ev, |_| {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        1 + (seed >> 33) as usize % 700
    });
    assert!(l == rl && r == rr, "random block sizes differ");
}

#[test]
fn plays_the_right_pitch() {
    let mut s = Poly::new(RATE);
    s.set_param(OSC1_WAVE, 3.0); // sine
    s.set_param(OSC2_LEVEL, 0.0);
    s.set_param(FILTER_MODE, 1.0);
    s.set_param(CUTOFF, 20_000.0);
    s.set_param(FENV_AMOUNT, 0.0);
    let (l, _) = render(&mut s, RATE as usize, &[(0, on(69, 127))], |_| 256);
    // Count rising zero crossings over the last half second.
    let tail = &l[RATE as usize / 2..];
    let ups = tail.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
    assert!((ups as i32 - 220).abs() <= 1, "{ups} crossings in 0.5 s, wanted 220 (440 Hz)");
}

#[test]
fn one_full_note_peaks_near_minus_12_dbfs() {
    let mut s = Poly::new(RATE);
    let (l, r) = render(&mut s, RATE as usize, &[(0, on(60, 127)), (24_000, off(60))], |_| 128);
    let p = peak(&l).max(peak(&r));
    let db = 20.0 * p.log10();
    assert!((-16.0..=-8.0).contains(&db), "peak {db:.1} dBFS");
}

#[test]
fn voices_end_after_release_and_all_off() {
    let mut s = Poly::new(RATE);
    s.set_param(AMP_R, 50.0);
    render(&mut s, 4800, &[(0, on(60, 100)), (0, on(64, 100)), (100, off(60))], |_| 128);
    assert_eq!(s.param(METER_VOICES), 1.0);
    render(&mut s, 4800, &[(0, NodeEvent::AllOff)], |_| 128);
    render(&mut s, 9600, &[], |_| 128);
    assert_eq!(s.active_voices(), 0);
    // Silence once the voices are gone.
    let (l, _) = render(&mut s, 1000, &[], |_| 128);
    assert!(l.iter().all(|&x| x == 0.0));
}

#[test]
fn polyphony_limit_steals_without_clicks() {
    let mut s = Poly::new(RATE);
    s.set_param(POLYPHONY, 4.0);
    s.set_param(AMP_R, 2000.0);
    // Sines, so any jump is a click and not the waveform.
    s.set_param(OSC1_WAVE, 3.0);
    s.set_param(OSC2_WAVE, 3.0);
    let ev: Vec<_> = (0..10u64).map(|k| (k * 480, on(50 + k as u8, 100))).collect();
    let (l, _) = render(&mut s, 9600, &ev, |_| 64);
    assert!(s.param(METER_VOICES) <= 4.0);
    assert_eq!(s.dropped, 0);
    // No sample-to-sample jump bigger than the signal itself makes.
    let jump = l.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
    assert!(jump < 0.02, "largest step {jump}");
}

#[test]
fn instruments_add_to_the_buffer() {
    let mut s = Poly::new(RATE);
    let mut l = vec![0.5f32; 256];
    let mut r = vec![0.5f32; 256];
    s.process(&Ctx::new(RATE, 0), &[], &mut l, &mut r);
    assert!(l.iter().chain(&r).all(|&x| x == 0.5));
}

#[test]
fn legato_glides_without_retrigger() {
    let mut s = Poly::new(RATE);
    s.set_param(VOICE_MODE, 2.0);
    s.set_param(GLIDE, 100.0);
    s.set_param(AMP_A, 0.5);
    s.set_param(AMP_D, 5.0);
    s.set_param(AMP_S, 50.0);
    render(&mut s, 4800, &[(0, on(48, 100))], |_| 128);
    assert_eq!(s.active_voices(), 1);
    // A second key while the first is held: still one voice, no new attack.
    let (l, _) = render(&mut s, 4800, &[(0, on(60, 100))], |_| 128);
    assert_eq!(s.active_voices(), 1);
    let p = peak(&l);
    assert!(p < 0.25, "a retrigger would overshoot to the full level; peak {p}");
    // Letting go of the new key returns to the held one; letting go of that releases.
    render(&mut s, 480, &[(0, off(60))], |_| 128);
    assert_eq!(s.param(METER_VOICES), 1.0);
    render(&mut s, 48_000, &[(0, off(48))], |_| 128);
    assert_eq!(s.active_voices(), 0);
}

#[test]
fn low_cutoff_darkens_the_sound() {
    let energy_of_diff = |cut: f32| {
        let mut s = Poly::new(RATE);
        s.set_param(CUTOFF, cut);
        s.set_param(FENV_AMOUNT, 0.0);
        s.set_param(VEL_CUTOFF, 0.0);
        let (l, _) = render(&mut s, 24_000, &[(0, on(48, 127))], |_| 128);
        // First difference stands in for high-frequency energy.
        l.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum::<f32>()
    };
    assert!(energy_of_diff(200.0) * 20.0 < energy_of_diff(8000.0));
}

#[test]
fn extreme_settings_stay_finite() {
    let mut s = Poly::new(RATE);
    for p in PARAMS {
        let mut s2 = Poly::new(RATE);
        for v in [p.min, p.max] {
            s2.set_param(p.id, v);
            let (l, r) = render(&mut s2, 4000, &[(0, on(30, 127)), (0, on(100, 127)), (2000, off(30))], |_| 333);
            assert!(l.iter().chain(&r).all(|x| x.is_finite()), "{} = {v}", p.name);
        }
    }
    // Everything at the top at once: resonance, drive, unison, LFO depths.
    for id in [RESONANCE, DRIVE, UNISON, LFO_PITCH, LFO_CUTOFF, LFO_AMP, LFO_PW, NOISE_LEVEL, SUB_LEVEL, FENV_AMOUNT, LEVEL] {
        s.set_param(id, f32::MAX);
    }
    let (l, _) = render(&mut s, 48_000, &[(0, on(20, 127)), (0, on(127, 127)), (100, NodeEvent::Bend { value: 1.0 })], |_| 128);
    assert!(l.iter().all(|x| x.is_finite()));
}

#[test]
fn parameters_and_presets_are_valid() {
    let s = Poly::new(RATE);
    check_params(s.params()).unwrap();
    assert!(s.params().iter().all(|p| (p.id as usize) < ID_SPACE));
    for preset in Poly::presets() {
        for &(id, v) in preset.values {
            let p = info(id).unwrap_or_else(|| panic!("{}: unknown id {id}", preset.name));
            assert_eq!(p.clamp(v), v, "{}: {} = {v} out of range", preset.name, p.name);
        }
    }
    assert!(s.param(9999).is_nan());
}

#[test]
fn every_preset_plays_at_a_sane_level() {
    for preset in Poly::presets() {
        let mut s = Poly::new(RATE);
        s.load_preset(preset);
        let ev = [(0, on(48, 110)), (0, on(55, 110)), (0, on(60, 110)), (36_000, off(48)), (36_000, off(55)), (36_000, off(60))];
        let (l, r) = render(&mut s, 192_000, &ev, |_| 128);
        let p = peak(&l).max(peak(&r));
        let db = 20.0 * p.log10();
        assert!((-30.0..=-1.0).contains(&db), "{}: chord peak {db:.1} dBFS", preset.name);
        // Every voice has ended by the end, and the last samples are quiet.
        let tail = peak(&l[l.len() - 480..]);
        assert!(tail < 1e-3, "{}: tail {tail}", preset.name);
    }
}

#[test]
fn tempo_synced_lfo_follows_the_beat() {
    // A square LFO on the level, one cycle per beat: the level follows the
    // beat whatever the block size (checked above), and changes on the beat.
    let mut s = Poly::new(RATE);
    s.set_param(LFO_SHAPE, 3.0);
    s.set_param(LFO_SYNC, 5.0);
    s.set_param(LFO_AMP, 100.0);
    s.set_param(OSC1_WAVE, 3.0);
    s.set_param(OSC2_LEVEL, 0.0);
    let (l, _) = render(&mut s, RATE as usize, &[(0, on(60, 127))], |_| 128);
    // 120 BPM: a beat is 24000 frames. The first half beat is loud, the second silent.
    let first = peak(&l[2_000..11_000]);
    let second = peak(&l[13_000..23_000]);
    assert!(first > 0.05 && second < first * 0.05, "first {first}, second {second}");
}
