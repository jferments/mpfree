//! A pad's movement (sampler R5 part 4): its LFO on pitch, cutoff, level
//! or pan, its pitch envelope, velocity on its pitch and cutoff, and the
//! live depth and envelope a step lock or a Q-Link gives.

use mpfree_sampler::dsp::{Filter, FilterKind};
use mpfree_sampler::engine::{Engine, EngineView, Event, PadField};
use mpfree_sampler::model::*;

const RATE: u32 = 44_100;

fn source(name: &str, frames: usize, f: impl Fn(usize) -> f32) -> Source {
    Source::new(name, RATE, 1, (0..frames).map(f).collect()).unwrap()
}
fn dc() -> Source {
    source("dc", RATE as usize * 3, |_| 0.25)
}
fn sine(hz: f32) -> Source {
    source("sine", RATE as usize * 3, |i| {
        0.5 * (i as f32 * hz * std::f32::consts::TAU / RATE as f32).sin()
    })
}

fn pad_of(s: &Source) -> Pad {
    Pad::new("p", Layer::new(Region::whole(s)))
}

fn engine_for(pad: &Pad, s: &Source) -> Engine {
    let mut p = Project::new();
    p.add_source(s.clone());
    p.set_pad(0, Some(pad.clone()), None).unwrap();
    let mut e = Engine::new(RATE, 8);
    e.set_view(EngineView::build(&p, RATE));
    e
}

fn run(e: &mut Engine, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
    for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        e.render(a, b);
    }
    (l, r)
}

fn hit(e: &mut Engine, velocity: u8) {
    e.apply(Event::NoteOn { pad: 0, velocity });
}

/// `seconds` of pad 0 hit at `velocity`.
fn played(pad: &Pad, s: &Source, velocity: u8, seconds: f32) -> (Vec<f32>, Vec<f32>) {
    let mut e = engine_for(pad, s);
    hit(&mut e, velocity);
    run(&mut e, (seconds * RATE as f32) as usize)
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// Cycles a second in `x` from its zero crossings.
fn hz(x: &[f32]) -> f32 {
    let n = x
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count();
    n as f32 / 2.0 / (x.len() as f32 / RATE as f32)
}

/// Frames `a` to `b` seconds.
fn span(x: &[f32], a: f32, b: f32) -> &[f32] {
    &x[(a * RATE as f32) as usize..(b * RATE as f32) as usize]
}

fn lfo(target: LfoTarget, shape: LfoShape, rate_hz: f32, depth: f32) -> PadLfo {
    PadLfo {
        shape,
        rate_hz,
        target,
        depth,
        ..PadLfo::default()
    }
}

#[test]
fn a_pad_with_no_movement_plays_exactly_as_before() {
    let s = sine(440.0);
    let plain = pad_of(&s);
    // An LFO with a wave and a rate but no target, a pitch envelope of 0
    // and velocity on neither: the same samples.
    let mut idle = plain.clone();
    idle.lfo = lfo(LfoTarget::Off, LfoShape::Square, 9.0, 1.0);
    idle.pitch_env.decay_ms = 2_000.0;
    assert_eq!(played(&plain, &s, 90, 0.5), played(&idle, &s, 90, 0.5));
}

#[test]
fn an_lfo_on_pitch_is_a_vibrato_around_the_pads_pitch() {
    let s = sine(440.0);
    let mut pad = pad_of(&s);
    // 2 Hz, a tenth of the way: 1.2 semitones either way.
    pad.lfo = lfo(LfoTarget::Pitch, LfoShape::Sine, 2.0, 0.1);
    let (l, _) = played(&pad, &s, 127, 1.0);
    // Highest a quarter of the way through the cycle, lowest at three
    // quarters (the sine starts at 0 on each hit).
    let (top, bottom) = (hz(span(&l, 0.10, 0.15)), hz(span(&l, 0.35, 0.40)));
    let up = 440.0 * (1.2f32 / 12.0).exp2();
    let down = 440.0 * (-1.2f32 / 12.0).exp2();
    assert!((top - up).abs() < 20.0, "top {top} Hz, about {up}");
    assert!(
        (bottom - down).abs() < 20.0,
        "bottom {bottom} Hz, about {down}"
    );
    // A full cycle on, back where it started.
    assert!((hz(span(&l, 0.60, 0.65)) - up).abs() < 20.0);
}

#[test]
fn the_pitch_envelope_starts_away_and_comes_home_over_its_decay() {
    let s = sine(440.0);
    let mut pad = pad_of(&s);
    pad.pitch_env = PitchEnv {
        semitones: 12.0,
        decay_ms: 1_000.0,
    };
    let (l, _) = played(&pad, &s, 127, 1.4);
    // The first 20 ms nearly an octave up, falling fast at first (it has
    // come most of the way home a quarter of the way through its decay);
    // after the decay, home.
    assert!(
        hz(span(&l, 0.0, 0.02)) > 790.0,
        "{}",
        hz(span(&l, 0.0, 0.02))
    );
    let quarter = hz(span(&l, 0.24, 0.26));
    assert!(quarter < 520.0 && quarter > 445.0, "{quarter}");
    let home = hz(span(&l, 1.05, 1.35));
    assert!((home - 440.0).abs() < 4.0, "{home}");
    // Down from the start.
    let mut d = pad.clone();
    d.pitch_env.semitones = -12.0;
    let (l, _) = played(&d, &s, 127, 0.1);
    assert!(hz(span(&l, 0.0, 0.02)) < 260.0);
}

#[test]
fn velocity_moves_the_pitch_and_darkens_the_cutoff_of_soft_hits() {
    let s = sine(440.0);
    let mut pad = pad_of(&s);
    pad.vel_to_pitch = 12.0;
    // The hardest hit plays as tuned; the softest an octave down.
    let (hard, _) = played(&pad, &s, 127, 0.3);
    let (soft, _) = played(&pad, &s, 1, 0.3);
    assert!((hz(&hard) - 440.0).abs() < 5.0);
    assert!((hz(&soft) - 222.0).abs() < 6.0, "{}", hz(&soft));

    // A 1 kHz tone under a low-pass at 2 kHz: full at the hardest hit, 4
    // octaves darker (the cutoff near 125 Hz) at the softest.
    let s = sine(1_000.0);
    let mut pad = pad_of(&s);
    pad.vel_to_level = 0.0;
    pad.filter = Filter {
        kind: FilterKind::LowPass,
        poles: 2,
        cutoff_hz: 2_000.0,
        resonance: 0.0,
        env_octaves: 0.0,
        drive_db: 0.0,
    };
    pad.vel_to_cutoff = 1.0;
    let (hard, _) = played(&pad, &s, 127, 0.3);
    let (soft, _) = played(&pad, &s, 1, 0.3);
    let (h, s2) = (rms(span(&hard, 0.1, 0.3)), rms(span(&soft, 0.1, 0.3)));
    assert!(s2 < 0.05 * h, "soft {s2} hard {h}");
}

#[test]
fn an_lfo_on_level_is_a_tremolo_and_on_pan_moves_side_to_side() {
    let s = dc();
    let mut pad = pad_of(&s);
    pad.vel_to_level = 0.0;
    // A square at 4 Hz, all the way: full for an eighth of a second,
    // silent the next.
    pad.lfo = lfo(LfoTarget::Level, LfoShape::Square, 4.0, 1.0);
    let (l, _) = played(&pad, &s, 127, 0.5);
    let (on, off) = (rms(span(&l, 0.02, 0.10)), rms(span(&l, 0.15, 0.23)));
    assert!(off < 1e-4 && on > 0.1, "on {on} off {off}");
    // Half way: down to half at its low point.
    pad.lfo.depth = 0.5;
    let (l, _) = played(&pad, &s, 127, 0.5);
    let (on, low) = (rms(span(&l, 0.02, 0.10)), rms(span(&l, 0.15, 0.23)));
    assert!((low / on - 0.5).abs() < 0.01, "{}", low / on);
    // Each step glides: never a jump bigger than a sixteenth of the way.
    let jump = l
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(jump < on / 16.0 + 1e-4, "{jump}");

    // On pan, all the way: right then left.
    pad.lfo = lfo(LfoTarget::Pan, LfoShape::Square, 4.0, 1.0);
    let (l, r) = played(&pad, &s, 127, 0.5);
    assert!(rms(span(&l, 0.02, 0.10)) < 1e-4 && rms(span(&r, 0.02, 0.10)) > 0.1);
    assert!(rms(span(&r, 0.15, 0.23)) < 1e-4 && rms(span(&l, 0.15, 0.23)) > 0.1);
}

#[test]
fn an_lfo_on_cutoff_opens_and_closes_the_filter() {
    let s = sine(1_000.0);
    let mut pad = pad_of(&s);
    pad.filter = Filter {
        kind: FilterKind::LowPass,
        poles: 4,
        cutoff_hz: 1_000.0,
        resonance: 0.0,
        env_octaves: 0.0,
        drive_db: 0.0,
    };
    // A square at 4 Hz, half way: 2 octaves up, then 2 down.
    pad.lfo = lfo(LfoTarget::Cutoff, LfoShape::Square, 4.0, 0.5);
    let (l, _) = played(&pad, &s, 127, 0.5);
    let (open, shut) = (rms(span(&l, 0.03, 0.10)), rms(span(&l, 0.16, 0.23)));
    assert!(shut < 0.1 * open, "open {open} shut {shut}");
}

#[test]
fn retrig_off_joins_the_cycle_the_pad_is_in_and_synced_keeps_to_the_beats() {
    let s = dc();
    let mut pad = pad_of(&s);
    pad.vel_to_level = 0.0;
    // A square at 1 Hz on the level: full the first half of each second,
    // silent the second.
    pad.lfo = lfo(LfoTarget::Level, LfoShape::Square, 1.0, 1.0);
    let start_late = |pad: &Pad| {
        let mut e = engine_for(pad, &s);
        run(&mut e, RATE as usize * 3 / 2);
        hit(&mut e, 127);
        run(&mut e, RATE as usize / 10).0
    };
    // From each hit: the cycle starts full.
    assert!(rms(span(&start_late(&pad), 0.02, 0.08)) > 0.1);
    // Running on: 1.5 s in, the pad's cycle is in its silent half.
    pad.lfo.retrig = false;
    assert!(rms(span(&start_late(&pad), 0.02, 0.08)) < 1e-4);

    // Synced to a beat at 120 BPM with the transport at beat 3 (half way
    // through a 1-beat... a 2-beat cycle): silent; at beat 4, full.
    pad.lfo.beats = Some(2.0);
    let on_beat = |beat: f64| {
        let mut e = engine_for(&pad, &s);
        e.set_tempo(120.0);
        e.set_beat(Some(beat));
        hit(&mut e, 127);
        run(&mut e, RATE as usize / 10).0
    };
    assert!(rms(span(&on_beat(3.0), 0.02, 0.08)) < 1e-4);
    assert!(rms(span(&on_beat(4.0), 0.02, 0.08)) > 0.1);
}

#[test]
fn live_depth_reaches_playing_voices_and_live_envelope_new_ones() {
    let s = dc();
    let mut pad = pad_of(&s);
    pad.vel_to_level = 0.0;
    pad.lfo = lfo(LfoTarget::Level, LfoShape::Square, 4.0, 0.0);
    let mut e = engine_for(&pad, &s);
    hit(&mut e, 127);
    let (l, _) = run(&mut e, RATE as usize / 4);
    // No depth: steady.
    assert!((rms(span(&l, 0.15, 0.23)) - rms(span(&l, 0.02, 0.10))).abs() < 1e-4);
    // A Q-Link (or a lock) gives it all the way, on the voice playing.
    e.apply(Event::PadLive {
        pad: 0,
        field: PadField::LfoDepth,
        value: 1.0,
    });
    let (l, _) = run(&mut e, RATE as usize / 2);
    // 0.25 s on, the square is high again until 0.375 s, then silent.
    assert!(rms(span(&l, 0.02, 0.10)) > 0.1);
    assert!(rms(span(&l, 0.15, 0.23)) < 1e-4);
    assert_eq!(e.pad_live_value(0, PadField::LfoDepth), Some(1.0));

    // A live pitch envelope reaches the next hit.
    let s = sine(440.0);
    let mut e = engine_for(&pad_of(&s), &s);
    for (field, value) in [(PadField::PitchEnv, 12.0), (PadField::PitchDecay, 1_000.0)] {
        e.apply(Event::PadLive {
            pad: 0,
            field,
            value,
        });
    }
    hit(&mut e, 127);
    let (l, _) = run(&mut e, RATE as usize / 10);
    assert!(hz(span(&l, 0.0, 0.02)) > 760.0);
}

#[test]
fn an_edit_of_the_lfo_reaches_a_voice_while_it_plays() {
    let s = dc();
    let mut pad = pad_of(&s);
    pad.vel_to_level = 0.0;
    let mut p = Project::new();
    p.add_source(s.clone());
    p.set_pad(0, Some(pad.clone()), None).unwrap();
    let mut e = Engine::new(RATE, 8);
    e.set_view(EngineView::build(&p, RATE));
    hit(&mut e, 127);
    run(&mut e, RATE as usize / 10);
    // The Play page turns the LFO on: the voice playing takes it.
    pad.lfo = lfo(LfoTarget::Level, LfoShape::Square, 4.0, 1.0);
    p.set_pad(0, Some(pad), None).unwrap();
    e.edit_view(EngineView::build(&p, RATE));
    let (l, _) = run(&mut e, RATE as usize / 2);
    assert!(rms(span(&l, 0.02, 0.10)) > 0.1);
    assert!(rms(span(&l, 0.15, 0.23)) < 1e-4);
}
