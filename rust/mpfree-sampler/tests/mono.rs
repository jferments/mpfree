//! Mono, Legato and Glide for a pad with keys (sampler R5.5): one note at a
//! time, the last key played; letting go of it goes back to a key still
//! held; Legato moves the sounding note without starting it again; Glide
//! slides the pitch over its time, to every note or to held keys only.

use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::*;

const RATE: u32 = 44_100;
/// The sine's own pitch is A4 (440 Hz), MIDI note 69.
const A4: u8 = 69;
const A5: u8 = 81;

fn sine() -> Source {
    let frames = RATE as usize * 4;
    let x = (0..frames)
        .map(|i| 0.5 * (i as f32 * 440.0 * std::f32::consts::TAU / RATE as f32).sin())
        .collect();
    Source::new("sine", RATE, 1, x).unwrap()
}

/// A pad on every key, A4 at the sound's own pitch, played while held.
fn keyed(mono: Mono) -> Pad {
    let s = sine();
    let mut region = Region::whole(&s);
    region.mode = PlayMode::Gate;
    let mut pad = Pad::new("Lead", Layer::new(region));
    pad.keys = Some(Keys::all(A4));
    pad.mono = mono;
    pad
}

fn engine(pad: &Pad) -> Engine {
    let mut p = Project::new();
    p.add_source(sine());
    p.set_pad(0, Some(pad.clone()), None).unwrap();
    let mut e = Engine::new(RATE, 16);
    e.set_view(EngineView::build(&p, RATE));
    e
}

fn run(e: &mut Engine, seconds: f32) -> Vec<f32> {
    let frames = (seconds * RATE as f32) as usize;
    let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
    for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        e.render(a, b);
    }
    l
}

fn on(e: &mut Engine, note: u8) {
    e.apply(Event::Key {
        note,
        velocity: 100,
    });
}

fn off(e: &mut Engine, note: u8) {
    e.apply(Event::KeyOff { note });
}

/// Cycles a second in `x` from its zero crossings.
fn hz(x: &[f32]) -> f32 {
    let n = x
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count();
    n as f32 / 2.0 / (x.len() as f32 / RATE as f32)
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn near(got: f32, want: f32, by: f32) -> bool {
    (got - want).abs() <= by
}

fn mono() -> Mono {
    Mono {
        on: true,
        ..Mono::default()
    }
}

#[test]
fn mono_plays_the_last_key_and_goes_back_to_one_still_held() {
    let mut e = engine(&keyed(mono()));
    on(&mut e, A4);
    let first = run(&mut e, 0.2);
    assert!(
        near(hz(&first[4410..]), 440.0, 6.0),
        "{}",
        hz(&first[4410..])
    );
    let level = rms(&first[4410..]);
    on(&mut e, A5);
    let both = run(&mut e, 0.3);
    // One voice: A5 alone (the A4 voice fades out in a few ms).
    assert!(near(hz(&both[2205..]), 880.0, 6.0), "{}", hz(&both[2205..]));
    assert_eq!(e.active_voices(), 1);
    // Letting go of A5 goes back to A4, still held.
    off(&mut e, A5);
    let back = run(&mut e, 0.3);
    assert!(near(hz(&back[2205..]), 440.0, 6.0), "{}", hz(&back[2205..]));
    assert!(near(rms(&back[2205..]), level, 0.05 * level));
    // An older key let go changes nothing; the last one lets go.
    on(&mut e, A5);
    off(&mut e, A4);
    let still = run(&mut e, 0.2);
    assert!(
        near(hz(&still[2205..]), 880.0, 6.0),
        "{}",
        hz(&still[2205..])
    );
    off(&mut e, A5);
    let gone = run(&mut e, 0.5);
    assert!(rms(&gone[gone.len() - 4410..]) < 1e-3);
}

#[test]
fn without_mono_the_keys_play_together() {
    let mut e = engine(&keyed(Mono::default()));
    on(&mut e, A4);
    run(&mut e, 0.1);
    on(&mut e, A5);
    run(&mut e, 0.1);
    assert_eq!(e.active_voices(), 2);
}

#[test]
fn legato_moves_the_note_without_starting_it_again() {
    // A slow attack shows a new start: it comes in from silence.
    let attack = |legato: bool| {
        let mut pad = keyed(Mono { legato, ..mono() });
        pad.amp.attack_ms = 200.0;
        let mut e = engine(&pad);
        on(&mut e, A4);
        let before = rms(&run(&mut e, 0.4)[13_230..]);
        on(&mut e, A5);
        let x = run(&mut e, 0.3);
        (rms(&x[..441]) / before, hz(&x[2205..]), e.active_voices())
    };
    let (level, pitch, voices) = attack(true);
    assert!(level > 0.95, "legato goes on at full level: {level}");
    assert!(near(pitch, 880.0, 6.0), "{pitch}");
    assert_eq!(voices, 1);
    let (level, pitch, _) = attack(false);
    assert!(level < 0.3, "without legato it starts again: {level}");
    assert!(near(pitch, 880.0, 6.0), "{pitch}");
}

#[test]
fn legato_lets_go_back_to_the_held_key_without_starting_again() {
    let mut pad = keyed(Mono {
        legato: true,
        ..mono()
    });
    pad.amp.attack_ms = 200.0;
    let mut e = engine(&pad);
    on(&mut e, A4);
    let before = rms(&run(&mut e, 0.4)[13_230..]);
    on(&mut e, A5);
    run(&mut e, 0.2);
    off(&mut e, A5);
    let x = run(&mut e, 0.2);
    assert!(rms(&x[..441]) > 0.95 * before);
    assert!(near(hz(&x[2205..]), 440.0, 6.0), "{}", hz(&x[2205..]));
}

#[test]
fn glide_slides_the_pitch_over_its_time() {
    let mut e = engine(&keyed(Mono {
        glide_ms: 200.0,
        ..mono()
    }));
    on(&mut e, A4);
    run(&mut e, 0.2);
    on(&mut e, A5);
    let x = run(&mut e, 0.5);
    let at = |a: f32, b: f32| hz(&x[(a * RATE as f32) as usize..(b * RATE as f32) as usize]);
    // Halfway in time is halfway in pitch: six semitones up, 622 Hz.
    let early = at(0.0, 0.04);
    let mid = at(0.08, 0.12);
    let late = at(0.3, 0.5);
    assert!(early > 430.0 && early < 560.0, "{early}");
    assert!(near(mid, 622.0, 40.0), "{mid}");
    assert!(near(late, 880.0, 6.0), "{late}");
}

#[test]
fn glide_to_every_note_slides_from_the_last_one_after_it_let_go() {
    let glide_from_a4 = |glide_held: bool| {
        let mut e = engine(&keyed(Mono {
            glide_ms: 300.0,
            glide_held,
            ..mono()
        }));
        on(&mut e, A4);
        run(&mut e, 0.2);
        off(&mut e, A4);
        run(&mut e, 0.3);
        on(&mut e, A5);
        let x = run(&mut e, 0.06);
        hz(&x[441..])
    };
    let every = glide_from_a4(false);
    assert!(every < 700.0, "every note glides, from A4: {every}");
    let held = glide_from_a4(true);
    assert!(
        near(held, 880.0, 15.0),
        "held keys only: a separate note jumps: {held}"
    );
}

#[test]
fn glide_on_held_keys_slides_between_overlapping_keys() {
    let mut e = engine(&keyed(Mono {
        glide_ms: 300.0,
        glide_held: true,
        ..mono()
    }));
    on(&mut e, A5);
    run(&mut e, 0.2);
    on(&mut e, A4);
    let x = run(&mut e, 0.06);
    let early = hz(&x[441..]);
    assert!(early > 700.0, "it starts near A5 and slides down: {early}");
}

#[test]
fn mono_needs_keys_a_hit_on_the_pad_plays_as_before() {
    let mut pad = keyed(mono());
    pad.keys = None;
    pad.layers[0].region.mode = PlayMode::OneShot;
    let mut e = engine(&pad);
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 100,
    });
    run(&mut e, 0.05);
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 100,
    });
    run(&mut e, 0.05);
    assert_eq!(e.active_voices(), 2, "the pad's own polyphony");
}

#[test]
fn all_off_forgets_the_keys_held() {
    let mut e = engine(&keyed(mono()));
    on(&mut e, A4);
    on(&mut e, A5);
    run(&mut e, 0.1);
    e.apply(Event::AllOff);
    run(&mut e, 0.5);
    // A5 lets go after the stop: nothing comes back.
    off(&mut e, A5);
    let x = run(&mut e, 0.3);
    assert!(rms(&x) < 1e-3);
}
