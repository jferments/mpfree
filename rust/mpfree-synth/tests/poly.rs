//! Behaviour tests for the Poly synth through the mpfree-node interface.

use mpfree_node::{check_params, Ctx, Node, NodeEvent, Timed, MAX_BLOCK};
use mpfree_synth::params::*;
use mpfree_synth::Poly;

const RATE: u32 = 48_000;

/// Renders `frames` frames with `events` (absolute frames), cut into blocks
/// by `cut(i)`, and returns the left and right channels.
fn render(
    synth: &mut Poly,
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

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

/// A busy phrase: chords, a mono switch, glide, bend, pressure, parameter
/// automation, unison, a voice overflow and AllOff.
fn busy_events() -> Vec<(u64, NodeEvent)> {
    let mut ev = vec![
        (
            0,
            NodeEvent::Param {
                id: UNISON,
                value: 3.0,
            },
        ),
        (
            0,
            NodeEvent::Param {
                id: LFO_SYNC,
                value: 6.0,
            },
        ),
        (
            0,
            NodeEvent::Param {
                id: LFO_CUTOFF,
                value: 1.0,
            },
        ),
        (
            0,
            NodeEvent::Param {
                id: DRIVE,
                value: 10.0,
            },
        ),
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
        (9_000, NodeEvent::Bend { value: 0.5 }),
        (11_111, off(55)),
        (
            12_000,
            NodeEvent::Param {
                id: CUTOFF,
                value: 300.0,
            },
        ),
        (
            15_013,
            NodeEvent::Param {
                id: FILTER_MODE,
                value: 2.0,
            },
        ),
    ];
    // More notes than voices, so stealing happens.
    for k in 0..20u64 {
        ev.push((16_000 + k * 37, on(40 + k as u8, 64 + k as u8)));
    }
    ev.push((22_000, NodeEvent::AllOff));
    ev.push((
        26_000,
        NodeEvent::Param {
            id: VOICE_MODE,
            value: 2.0,
        },
    ));
    ev.push((
        26_000,
        NodeEvent::Param {
            id: GLIDE,
            value: 80.0,
        },
    ));
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
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
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
    let ups = tail
        .windows(2)
        .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
        .count();
    assert!(
        (ups as i32 - 220).abs() <= 1,
        "{ups} crossings in 0.5 s, wanted 220 (440 Hz)"
    );
}

#[test]
fn one_full_note_peaks_near_minus_12_dbfs() {
    let mut s = Poly::new(RATE);
    let (l, r) = render(
        &mut s,
        RATE as usize,
        &[(0, on(60, 127)), (24_000, off(60))],
        |_| 128,
    );
    let p = peak(&l).max(peak(&r));
    let db = 20.0 * p.log10();
    assert!((-16.0..=-8.0).contains(&db), "peak {db:.1} dBFS");
}

#[test]
fn voices_end_after_release_and_all_off() {
    let mut s = Poly::new(RATE);
    s.set_param(AMP_R, 50.0);
    render(
        &mut s,
        4800,
        &[(0, on(60, 100)), (0, on(64, 100)), (100, off(60))],
        |_| 128,
    );
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
    let ev: Vec<_> = (0..10u64)
        .map(|k| (k * 480, on(50 + k as u8, 100)))
        .collect();
    let (l, _) = render(&mut s, 9600, &ev, |_| 64);
    assert!(s.param(METER_VOICES) <= 4.0);
    assert_eq!(s.dropped, 0);
    // No sample-to-sample jump bigger than the signal itself makes.
    let jump = l
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max);
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
    assert!(
        p < 0.25,
        "a retrigger would overshoot to the full level; peak {p}"
    );
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
            let (l, r) = render(
                &mut s2,
                4000,
                &[(0, on(30, 127)), (0, on(100, 127)), (2000, off(30))],
                |_| 333,
            );
            assert!(
                l.iter().chain(&r).all(|x| x.is_finite()),
                "{} = {v}",
                p.name
            );
        }
    }
    // Everything at the top at once: resonance, drive, distortion, unison,
    // LFO depths.
    for id in [
        RESONANCE,
        DRIVE,
        DIST,
        UNISON,
        LFO_PITCH,
        LFO_CUTOFF,
        LFO_AMP,
        LFO_PW,
        NOISE_LEVEL,
        SUB_LEVEL,
        FENV_AMOUNT,
        LEVEL,
    ] {
        s.set_param(id, f32::MAX);
    }
    let (l, _) = render(
        &mut s,
        48_000,
        &[
            (0, on(20, 127)),
            (0, on(127, 127)),
            (100, NodeEvent::Bend { value: 1.0 }),
        ],
        |_| 128,
    );
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
            assert_eq!(
                p.clamp(v),
                v,
                "{}: {} = {v} out of range",
                preset.name,
                p.name
            );
        }
    }
    assert!(s.param(9999).is_nan());
}

#[test]
fn every_preset_plays_at_a_sane_level() {
    for preset in Poly::presets() {
        let mut s = Poly::new(RATE);
        s.load_preset(preset);
        let ev = [
            (0, on(48, 110)),
            (0, on(55, 110)),
            (0, on(60, 110)),
            (36_000, off(48)),
            (36_000, off(55)),
            (36_000, off(60)),
        ];
        let (l, r) = render(&mut s, 192_000, &ev, |_| 128);
        let p = peak(&l).max(peak(&r));
        let db = 20.0 * p.log10();
        assert!(
            (-30.0..=-1.0).contains(&db),
            "{}: chord peak {db:.1} dBFS",
            preset.name
        );
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
    assert!(
        first > 0.05 && second < first * 0.05,
        "first {first}, second {second}"
    );
}

/// Live test 2: "Drive seems to make synths quieter". Turning the drive up
/// on any preset, one note or a chord, keeps it within 2 dB of its level
/// with no drive (the most is Acid's resonant filter, about -1.7 dB).
/// Before the fix, 24 dB of drive took up to 14 dB off.
/// The distortion after the filter keeps the level the same way (Jay
/// after live test 3: both kinds).
#[test]
fn turning_the_drive_up_keeps_every_preset_about_as_loud() {
    let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
    let level = |preset: &mpfree_synth::Preset, id: u16, drive: f32, notes: &[u8]| {
        let mut s = Poly::new(RATE);
        s.load_preset(preset);
        s.set_param(id, drive);
        let ev: Vec<(u64, NodeEvent)> = notes.iter().map(|&n| (0, on(n, 110))).collect();
        let (l, r) = render(&mut s, 36_000, &ev, |_| 128);
        // The sustain, after every attack.
        20.0 * (rms(&l[12_000..]) + rms(&r[12_000..])).log10()
    };
    for preset in Poly::presets() {
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

/// Live test 3: "Drive has almost no effect". Grit Bass's grit is its
/// distortion, after the filter, so its low cutoff (260 Hz) doesn't take
/// the grit out: at its own 18 dB, the harmonics from the 5th up carry
/// several times the share of the power they carry with none (about 2 %
/// to 21 % on E2), and the level stays put. The same 18 dB of drive,
/// before the filter, gives far less (about 5 %): why there are both
/// (Jay: "add both types of drive and distortion").
#[test]
fn grit_bass_distortion_is_heard_through_its_low_cutoff() {
    let grit = Poly::presets()
        .iter()
        .find(|p| p.name == "Grit Bass")
        .unwrap();
    let (note, f0) = (40u8, 82.407f32);
    let play = |drive: f32, dist: f32| {
        let mut s = Poly::new(RATE);
        s.load_preset(grit);
        s.set_param(DRIVE, drive);
        s.set_param(DIST, dist);
        let (l, _) = render(&mut s, 72_000, &[(0, on(note, 100))], |_| 128);
        l[24_000..].to_vec()
    };
    // Power at harmonic `k` (Goertzel), and the share from the 5th up.
    let upper = |x: &[f32]| {
        let power = |hz: f32| {
            let c = 2.0 * (2.0 * std::f32::consts::PI * hz / RATE as f32).cos();
            let (mut s1, mut s2) = (0.0f32, 0.0f32);
            for &v in x {
                let s0 = v + c * s1 - s2;
                s2 = s1;
                s1 = s0;
            }
            s1 * s1 + s2 * s2 - c * s1 * s2
        };
        let h: Vec<f32> = (1..=60).map(|k| power(f0 * k as f32)).collect();
        h[4..].iter().sum::<f32>() / h.iter().sum::<f32>()
    };
    let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
    let (dry, wet, driven) = (play(0.0, 0.0), play(0.0, 18.0), play(18.0, 0.0));
    let (a, b, c) = (upper(&dry), upper(&wet), upper(&driven));
    assert!(b > 4.0 * a, "harmonics 5-60: {a:.3} dry, {b:.3} at 18 dB");
    assert!(b > 2.0 * c, "distortion {b:.3}, drive {c:.3}");
    let change = 20.0 * (rms(&wet) / rms(&dry)).log10();
    assert!(change.abs() < 1.0, "level {change:+.1} dB");
}

/// The supersaw (either oscillator on wave 4): the same for any block
/// size, about as loud as one saw, and its Saw detune heard as beating.
#[test]
fn the_supersaw_plays_as_loud_as_a_saw_and_its_detune_beats() {
    let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
    let setup = |wave: f32, detune: f32| {
        let mut s = Poly::new(RATE);
        s.set_param(OSC1_WAVE, wave);
        s.set_param(OSC2_LEVEL, 0.0);
        s.set_param(CUTOFF, 20_000.0);
        s.set_param(SUPER_DETUNE, detune);
        s
    };
    let ev = [(0, on(57, 110)), (30_000, on(64, 90)), (40_000, off(57))];
    let (whole, _) = render(&mut setup(4.0, 60.0), 48_000, &ev, |_| 128);
    for size in [1usize, 17, 1000] {
        let (l, _) = render(&mut setup(4.0, 60.0), 48_000, &ev, |_| size);
        assert!(l == whole, "block size {size} differs");
    }
    let one = &[(0, on(57, 110))];
    let (saw, _) = render(&mut setup(0.0, 60.0), 24_000, one, |_| 256);
    let (sup, _) = render(&mut setup(4.0, 60.0), 24_000, one, |_| 256);
    let db = 20.0 * (rms(&sup[4800..]) / rms(&saw[4800..])).log10();
    assert!(db.abs() < 2.0, "supersaw {db} dB against a saw");
    // Detuned, the stack's level moves as its saws drift through each
    // other; at no detune it barely moves.
    let wobble = |x: &[f32]| {
        let w: Vec<f32> = x[4800..].chunks(960).map(rms).collect();
        let (lo, hi) = w
            .iter()
            .fold((f32::MAX, 0f32), |(a, b), &v| (a.min(v), b.max(v)));
        20.0 * (hi / lo).log10()
    };
    let (still, _) = render(&mut setup(4.0, 0.0), 24_000, one, |_| 256);
    assert!(
        wobble(&sup) > 2.0 * wobble(&still),
        "{} vs {}",
        wobble(&sup),
        wobble(&still)
    );
}

/// A sine voice with the filter open and nothing else on it, so the
/// output's level is the amp envelope's times the velocity's gain.
fn plain_sine() -> Poly {
    let mut s = Poly::new(RATE);
    s.prepare(RATE, MAX_BLOCK);
    for (id, v) in [
        (OSC1_WAVE, 3.0),
        (OSC2_LEVEL, 0.0),
        (FILTER_MODE, 0.0),
        (CUTOFF, 20_000.0),
        (RESONANCE, 0.0),
        (FENV_AMOUNT, 0.0),
        (KEY_TRACK, 0.0),
        (VEL_CUTOFF, 0.0),
        (AMP_D, 1.0),
        (AMP_S, 100.0),
        (VEL_AMP, 0.0),
    ] {
        s.set_param(id, v);
    }
    s
}

/// The peak of each 2 ms window, in dB below the loudest (for a note
/// high enough to have a few cycles in each).
fn level_db(x: &[f32]) -> Vec<f32> {
    let w = RATE as usize / 500;
    let peaks: Vec<f32> = x.chunks(w).map(peak).collect();
    let top = peaks.iter().fold(0.0f32, |m, &p| m.max(p));
    peaks
        .iter()
        .map(|&p| 20.0 * (p.max(1e-9) / top).log10())
        .collect()
}

#[test]
fn the_envelope_times_mean_what_they_say() {
    // An attack of 100 ms reaches full level at 100 ms (the old envelope
    // got there at 32 ms), and a release of 1 s falls 60 dB in 1 s and
    // ends just after (the analog curve Poly always had).
    let mut s = plain_sine();
    s.set_param(AMP_A, 100.0);
    s.set_param(AMP_R, 1_000.0);
    let ms = |t: f32| (t * RATE as f32 / 1000.0) as u64;
    let (l, _) = render(
        &mut s,
        ms(1_700.0) as usize,
        &[(0, on(93, 127)), (ms(500.0), off(93))],
        |_| 64,
    );
    let db = level_db(&l);
    let at = |t: f32| db[(t / 2.0) as usize];
    assert!(at(30.0) < -4.0, "not yet full at 30 ms: {:.1} dB", at(30.0));
    assert!(at(80.0) < -0.3, "still rising at 80 ms: {:.2} dB", at(80.0));
    assert!(at(102.0) > -0.2, "full at 100 ms: {:.2} dB", at(102.0));
    // From the key's release at 500 ms: half way down by 500 ms more is
    // about -20 dB, 1 s after it 60 dB down, and silent at 1.02 s.
    assert!((at(1_000.0) + 20.4).abs() < 2.0, "{:.1} dB", at(1_000.0));
    assert!((at(1_496.0) + 60.0).abs() < 4.0, "{:.1} dB", at(1_496.0));
    assert!(peak(&l[ms(1_500.0) as usize..ms(1_512.0) as usize]) > 0.0);
    assert!(peak(&l[ms(1_530.0) as usize..]) == 0.0);
}

#[test]
fn a_sound_saved_before_format_2_plays_as_it_did() {
    use mpfree_node::preset::Preset;
    // Saved when an attack of 100 ms reached full level at 32 ms and the
    // velocity was a share.
    let old = "mpfree-preset = 1\nengine = mpfree.poly\nname = Old\n\
               osc-1.wave = Sine\nosc-2.level = 0\nfilter.cutoff = 20000\nfilter.resonance = 0\n\
               filter.env-amount = 0\nfilter.key-track = 0\nfilter.velocity = 0\n\
               amp-env.attack = 100\namp-env.decay = 1\namp-env.sustain = 100\n\
               amp-env.release = 1000\namp-env.velocity = 60\n\
               filter-env.attack = 50\nfilter-env.release = 200\n";
    let mut s = Poly::new(RATE);
    s.prepare(RATE, MAX_BLOCK);
    let p = Preset::parse(old, s.info(), s.params()).unwrap();
    p.apply(&mut s).unwrap();
    let near = |a: f32, b: f32| (a - b).abs() < 1e-3 * b.abs().max(1.0);
    assert!(near(s.param(AMP_A), 31.876_9), "{}", s.param(AMP_A));
    assert!(near(s.param(FENV_A), 15.938_5), "{}", s.param(FENV_A));
    assert!(near(s.param(AMP_R), 982.567), "{}", s.param(AMP_R));
    assert!(near(s.param(FENV_R), 196.513), "{}", s.param(FENV_R));
    // 60 % put a velocity 1 note at 0.4 of full level: 8 dB down.
    assert!(near(s.param(VEL_AMP), 7.957), "{}", s.param(VEL_AMP));
    assert_eq!(s.param(AMP_D), 1.0, "decay keeps its meaning");
    // It reaches full level at about 32 ms, as it did.
    let ms = |t: f32| (t * RATE as f32 / 1000.0) as usize;
    let (l, _) = render(&mut s, ms(200.0), &[(0, on(93, 127))], |_| 64);
    let db = level_db(&l);
    assert!(db[(20.0 / 2.0) as usize] < -1.0, "{:.2}", db[10]);
    assert!(db[(34.0 / 2.0) as usize] > -0.2, "{:.2}", db[17]);
    // The same text in format 2 is taken as it is.
    let new = old.replace("mpfree-preset = 1", "mpfree-preset = 2");
    Preset::parse(&new, s.info(), s.params())
        .unwrap()
        .apply(&mut s)
        .unwrap();
    assert_eq!(s.param(AMP_A), 100.0);
    // Even a full share loads, at the most the range allows.
    let full = old.replace("amp-env.velocity = 60", "amp-env.velocity = 100");
    Preset::parse(&full, s.info(), s.params())
        .unwrap()
        .apply(&mut s)
        .unwrap();
    assert_eq!(s.param(VEL_AMP), 40.0);
}

#[test]
fn the_softest_note_is_the_velocity_range_below_the_hardest() {
    let level = |velocity: u8, range: f32| {
        let mut s = plain_sine();
        s.set_param(AMP_A, 0.1);
        s.set_param(VEL_AMP, range);
        let (l, _) = render(&mut s, 9_600, &[(0, on(93, velocity))], |_| 64);
        20.0 * peak(&l[4_800..]).log10()
    };
    for range in [0.0, 8.0, 24.0, 40.0] {
        let drop = level(127, range) - level(1, range);
        assert!((drop - range).abs() < 0.2, "{range} dB: {drop:.2}");
    }
    // The square-law curve: half velocity is a third of the range down.
    let half = level(127, 24.0) - level(64, 24.0);
    assert!((half - 8.15).abs() < 0.2, "{half:.2}");
}

// ---- play routes and LFO 2 ----

fn ctl(ctl: mpfree_node::Ctl, value: f32) -> NodeEvent {
    NodeEvent::Control { ctl, value }
}

/// Sets route `n` to `source` ([`mpfree_synth::voice::Source`]'s index),
/// `target` ([`target`]) and `depth` (%).
fn route(s: &mut Poly, n: usize, source: f32, target: u8, depth: f32) {
    s.set_param(route_id(n, route::SOURCE), source);
    s.set_param(route_id(n, route::TARGET), f32::from(target));
    s.set_param(route_id(n, route::DEPTH), depth);
}

/// Each cycle's pitch in semitones from A4, from the rising zero
/// crossings (placed between samples).
fn cycle_pitches(x: &[f32]) -> Vec<f32> {
    let ups: Vec<f32> = x
        .windows(2)
        .enumerate()
        .filter(|(_, w)| w[0] < 0.0 && w[1] >= 0.0)
        .map(|(i, w)| i as f32 + w[0] / (w[0] - w[1]))
        .collect();
    ups.windows(2)
        .map(|w| 12.0 * (RATE as f32 / (w[1] - w[0]) / 440.0).log2())
        .collect()
}

fn spread(x: &[f32]) -> f32 {
    let lo = x.iter().fold(f32::MAX, |m, &v| m.min(v));
    let hi = x.iter().fold(f32::MIN, |m, &v| m.max(v));
    hi - lo
}

use mpfree_node::Ctl;

const SRC_VELOCITY: f32 = 1.0;
const SRC_TONE: f32 = 3.0;
const SRC_WHEEL: f32 = 4.0;
const SRC_BREATH: f32 = 5.0;
const SRC_RANDOM: f32 = 8.0;

#[test]
fn the_mod_wheel_brings_in_vibrato_by_default() {
    // Route 1 sends the wheel to vibrato, half a semitone at the top, at
    // Vib rate. With the wheel down nothing moves.
    let vibrato = |wheel: f32| {
        let mut s = plain_sine();
        let (l, _) = render(
            &mut s,
            RATE as usize,
            &[(0, ctl(Ctl::ModWheel, wheel)), (0, on(69, 127))],
            |_| 256,
        );
        spread(&cycle_pitches(&l[RATE as usize / 2..]))
    };
    assert!(vibrato(0.0) < 0.02, "{:.3}", vibrato(0.0));
    let full = vibrato(1.0);
    assert!((full - 1.0).abs() < 0.1, "±0.5 semitone: {full:.3}");
    let half = vibrato(0.5);
    assert!((half - 0.5).abs() < 0.07, "±0.25 semitone: {half:.3}");
}

#[test]
fn a_sound_plays_the_same_with_its_routes_until_a_controller_moves() {
    // The default route (the wheel to vibrato) changes nothing until the
    // wheel moves: every preset sounds as it did before routes, to the bit.
    let mut a = Poly::new(RATE);
    let mut b = Poly::new(RATE);
    b.set_param(route_id(0, route::SOURCE), 0.0);
    let ev = busy_events();
    assert_eq!(
        render(&mut a, 96_000, &ev, |_| 128),
        render(&mut b, 96_000, &ev, |_| 128)
    );
}

/// How bright a sound is: its first difference's level against its own.
fn brightness(x: &[f32]) -> f32 {
    let rms = |v: &mut dyn Iterator<Item = f32>| {
        let (s, n) = v.fold((0.0f64, 0usize), |(s, n), x| (s + f64::from(x * x), n + 1));
        (s / n.max(1) as f64).sqrt() as f32
    };
    let d = rms(&mut x.windows(2).map(|w| w[1] - w[0]));
    d / rms(&mut x.iter().copied()).max(1e-9)
}

#[test]
fn a_route_from_velocity_opens_the_filter() {
    let bright = |velocity: u8| {
        let mut s = Poly::new(RATE);
        s.set_param(CUTOFF, 400.0);
        s.set_param(FENV_AMOUNT, 0.0);
        s.set_param(VEL_AMP, 0.0);
        route(&mut s, 1, SRC_VELOCITY, target::CUTOFF, 75.0);
        let (l, _) = render(&mut s, 24_000, &[(0, on(48, velocity))], |_| 128);
        brightness(&l[4_800..])
    };
    let (soft, hard) = (bright(20), bright(127));
    assert!(hard > 2.0 * soft, "soft {soft:.4}, hard {hard:.4}");
}

#[test]
fn a_route_from_the_notes_own_tone_opens_the_filter() {
    // MPE's Y (a note's tone) does nothing on Poly until a route takes it.
    let bright = |tone: f32| {
        let mut s = Poly::new(RATE);
        s.set_param(CUTOFF, 400.0);
        s.set_param(FENV_AMOUNT, 0.0);
        route(&mut s, 1, SRC_TONE, target::CUTOFF, 75.0);
        let ev = [
            (
                0,
                NodeEvent::Expression {
                    note: 48,
                    kind: mpfree_node::Expr::Tone,
                    value: tone,
                },
            ),
            (0, on(48, 100)),
        ];
        let (l, _) = render(&mut s, 24_000, &ev, |_| 128);
        brightness(&l[4_800..])
    };
    assert!(bright(1.0) > 2.0 * bright(0.0));
}

#[test]
fn breath_on_level_is_a_swell_never_a_boost() {
    // Breath to Level at 100 %: 24 dB down with no breath, the sound's own
    // level at full breath, never louder.
    let level = |breath: Option<f32>| {
        let mut s = plain_sine();
        s.set_param(AMP_A, 0.1);
        if breath.is_some() {
            route(&mut s, 0, SRC_BREATH, target::LEVEL, 100.0);
        }
        let mut ev = vec![(0, on(81, 127))];
        if let Some(b) = breath {
            ev.insert(0, (0, ctl(Ctl::Breath, b)));
        }
        let (l, _) = render(&mut s, 9_600, &ev, |_| 64);
        20.0 * peak(&l[4_800..]).log10()
    };
    let plain = level(None);
    let none = level(Some(0.0));
    let full = level(Some(1.0));
    assert!((plain - none - 24.0).abs() < 0.3, "{plain:.2} {none:.2}");
    assert!((full - plain).abs() < 0.05, "{full:.2} {plain:.2}");
    assert!(full <= plain + 0.01);
}

#[test]
fn random_to_pan_puts_each_note_somewhere_of_its_own() {
    let mut s = plain_sine();
    route(&mut s, 0, SRC_RANDOM, target::PAN, 100.0);
    s.set_param(AMP_R, 1.0);
    let step = 4_800u64;
    let ev: Vec<(u64, NodeEvent)> = (0..12)
        .flat_map(|k| [(k * step, on(81, 127)), (k * step + step / 2, off(81))])
        .collect();
    let (l, r) = render(&mut s, (12 * step) as usize, &ev, |_| 128);
    let sides: Vec<f32> = (0..12)
        .map(|k| {
            let a = (k * step) as usize + 480;
            let (pl, pr) = (peak(&l[a..a + 960]), peak(&r[a..a + 960]));
            (pr - pl) / (pr + pl)
        })
        .collect();
    assert!(sides.iter().any(|&x| x < -0.2), "{sides:?}");
    assert!(sides.iter().any(|&x| x > 0.2), "{sides:?}");
    // With no route the note sits in the middle.
    let mut c = plain_sine();
    route(&mut c, 0, 0.0, 0, 0.0);
    let (l, r) = render(&mut c, 9_600, &[(0, on(81, 127))], |_| 128);
    assert_eq!(l, r);
}

#[test]
fn the_wheel_on_lfo_1_amount_brings_the_lfo_in() {
    // LFO 1 set to a semitone of vibrato, and the wheel on LFO 1 amount at
    // 100 %: none with the wheel down, all of it at the top.
    let vibrato = |wheel: f32| {
        let mut s = plain_sine();
        s.set_param(LFO_PITCH, 1.0);
        s.set_param(LFO_RATE, 6.0);
        route(&mut s, 0, SRC_WHEEL, target::LFO1_AMOUNT, 100.0);
        let (l, _) = render(
            &mut s,
            RATE as usize,
            &[(0, ctl(Ctl::ModWheel, wheel)), (0, on(69, 127))],
            |_| 256,
        );
        spread(&cycle_pitches(&l[RATE as usize / 2..]))
    };
    assert!(vibrato(0.0) < 0.02, "{:.3}", vibrato(0.0));
    assert!((vibrato(1.0) - 2.0).abs() < 0.15, "{:.3}", vibrato(1.0));
}

#[test]
fn lfo_2_starts_with_each_note_and_fades_in() {
    // A square LFO 2 on level at 4 Hz: full for the first 125 ms of each
    // note, then silent for 125 ms, wherever the note starts.
    let mut s = plain_sine();
    s.set_param(AMP_A, 0.1);
    s.set_param(LFO2_SHAPE, 3.0);
    s.set_param(LFO2_RATE, 4.0);
    s.set_param(LFO2_AMP, 100.0);
    let ms = |t: f32| (t * RATE as f32 / 1000.0) as usize;
    let start = ms(333.0);
    let (l, _) = render(
        &mut s,
        start + ms(400.0),
        &[(start as u64, on(81, 127))],
        |_| 100,
    );
    let at = |t: f32| peak(&l[start + ms(t)..start + ms(t + 10.0)]);
    assert!(at(50.0) > 0.1, "{}", at(50.0));
    assert!(at(180.0) < 1e-3, "{}", at(180.0));
    assert!(at(300.0) > 0.1, "{}", at(300.0));

    // With a fade in of 400 ms, a sine on pitch starts still and grows.
    let mut s = plain_sine();
    s.set_param(LFO2_RATE, 5.0);
    s.set_param(LFO2_PITCH, 1.0);
    s.set_param(LFO2_FADE, 400.0);
    let (l, _) = render(&mut s, ms(1_200.0), &[(0, on(69, 127))], |_| 128);
    // (Past the note's first few milliseconds, where the attack's edge
    // makes crossings of its own.)
    let early = spread(&cycle_pitches(&l[ms(20.0)..ms(100.0)]));
    let late = spread(&cycle_pitches(&l[ms(500.0)..ms(1_200.0)]));
    assert!(early < 0.5, "a quarter in at most by 100 ms: {early:.3}");
    assert!((late - 2.0).abs() < 0.15, "{late:.3}");
}

#[test]
fn routes_and_lfo_2_keep_the_same_output_for_any_block_size() {
    let set = |s: &mut Poly| {
        route(s, 1, SRC_RANDOM, target::PAN, 60.0);
        route(s, 2, SRC_BREATH, target::LEVEL, 50.0);
        s.set_param(LFO2_PW, 20.0);
        s.set_param(LFO2_SYNC, 6.0);
        s.set_param(LFO2_RETRIG, 0.0);
        s.set_param(OSC1_WAVE, 1.0);
    };
    let mut ev = busy_events();
    ev.push((1_000, ctl(Ctl::ModWheel, 0.7)));
    ev.push((20_000, ctl(Ctl::Breath, 0.4)));
    ev.push((40_000, ctl(Ctl::Breath, 1.0)));
    ev.sort_by_key(|e| e.0);
    let mut a = Poly::new(RATE);
    set(&mut a);
    let mut b = Poly::new(RATE);
    set(&mut b);
    let one = render(&mut a, 96_000, &ev, |_| 4096);
    let odd = render(&mut b, 96_000, &ev, |i| [1, 7, 33, 500, 16, 3][i % 6]);
    assert_eq!(one, odd);
}

#[test]
fn the_wheel_on_pitch_at_100_percent_is_an_octave_up() {
    let mut s = plain_sine();
    route(&mut s, 0, SRC_WHEEL, target::PITCH, 100.0);
    let (l, _) = render(
        &mut s,
        RATE as usize,
        &[(0, ctl(Ctl::ModWheel, 1.0)), (0, on(69, 127))],
        |_| 256,
    );
    let p = cycle_pitches(&l[RATE as usize / 2..]);
    let mean = p.iter().sum::<f32>() / p.len() as f32;
    assert!((mean - 12.0).abs() < 0.01, "{mean}");
}
