//! Keygroups: pads that play notes, transposed from their root, and the
//! pads of a kit played as a keyboard.

use mpfree_analysis::yin;
use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::*;

const RATE: u32 = 44_100;

fn sine(seconds: f64, hz: f64) -> Source {
    let n = (seconds * f64::from(RATE)) as usize;
    let v = (0..n)
        .map(|i| (0.5 * (std::f64::consts::TAU * hz * i as f64 / f64::from(RATE)).sin()) as f32)
        .collect();
    Source::new("sine", RATE, 1, v).unwrap()
}

fn engine(p: &Project) -> Engine {
    let mut e = Engine::new(RATE, 16);
    e.set_view(EngineView::build(p, RATE));
    e
}

fn render(e: &mut Engine, frames: usize) -> Vec<f32> {
    let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
    for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        e.render(a, b);
    }
    l
}

fn pitch(x: &[f32]) -> f32 {
    yin(&x[1000..9192], RATE, 4096, 40.0, 3000.0, 0.15)
        .expect("pitched")
        .0
}

/// A gate keygroup over the whole keyboard, A3 (220 Hz) at its root.
fn spread() -> (Project, Source) {
    let s = sine(1.0, 220.0);
    let mut p = Project::new();
    p.add_source(s.clone());
    let mut r = Region::whole(&s);
    r.mode = PlayMode::Gate;
    p.set_pad(0, Some(Pad::new("keys", Layer::new(r))), None)
        .unwrap();
    p.spread_to_keys(0, 57).unwrap();
    (p, s)
}

#[test]
fn a_keygroup_plays_each_note_a_semitone_apart_from_its_root() {
    let (p, _) = spread();
    let pad = p.pad(0).unwrap();
    assert_eq!(pad.keys, Some(Keys::all(57)));
    assert_eq!(pad.polyphony, KEYGROUP_POLYPHONY);
    // The pads play notes from the C below the root.
    assert_eq!(p.pads_play(), PadsPlay::Keys { from: 48 });
    for (note, hz) in [(57u8, 220.0f32), (69, 440.0), (45, 110.0), (64, 329.63)] {
        let mut e = engine(&p);
        e.apply(Event::Key {
            note,
            velocity: 127,
        });
        let out = render(&mut e, 12_000);
        assert!(
            (pitch(&out) - hz).abs() < hz * 0.003,
            "note {note}: {}",
            pitch(&out)
        );
    }
}

#[test]
fn a_chord_holds_a_voice_per_note_and_each_key_lets_go_of_its_own() {
    let (p, _) = spread();
    let mut e = engine(&p);
    for note in [57, 61, 64] {
        e.apply(Event::Key {
            note,
            velocity: 100,
        });
    }
    render(&mut e, 256);
    assert_eq!(e.active_voices(), 3);
    e.apply(Event::KeyOff { note: 61 });
    render(&mut e, 4096);
    assert_eq!(e.active_voices(), 2, "only C#4 lets go");
    // A pad hit on the keyboard is its note: pad 10 is A3 from C3.
    e.apply(Event::NoteOff { pad: 9 });
    render(&mut e, 4096);
    assert_eq!(e.active_voices(), 1);
}

#[test]
fn the_pads_play_notes_from_the_kits_first_key() {
    let (p, _) = spread();
    let view = EngineView::build(&p, RATE);
    // Every pad sounds: each is a note the keygroup holds.
    assert!((0..16).all(|i| view.has_pad(i)));
    let mut e = engine(&p);
    // Pad 10 is A3 (from C3): the sound as recorded.
    e.apply(Event::NoteOn {
        pad: 9,
        velocity: 127,
    });
    assert!((pitch(&render(&mut e, 12_000)) - 220.0).abs() < 0.7);
    // Back to a drum kit, pad 1 is the keygroup itself, at its root.
    let mut q = p.clone();
    q.set_pads_play(PadsPlay::Pads).unwrap();
    let view = EngineView::build(&q, RATE);
    assert!(view.has_pad(0) && !view.has_pad(1));
    let mut e = engine(&q);
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
    assert!((pitch(&render(&mut e, 12_000)) - 220.0).abs() < 0.7);
}

#[test]
fn chop_to_keys_puts_a_slice_on_each_note_and_one_undo_takes_it_all_back() {
    // Four slices of a rising tone, one per quarter.
    let n = 4 * 11_025;
    let tones = [220.0, 277.18, 329.63, 440.0];
    let data: Vec<f32> = (0..n)
        .map(|i| {
            let hz = tones[i / 11_025];
            (0.5 * (std::f64::consts::TAU * hz * i as f64 / f64::from(RATE)).sin()) as f32
        })
        .collect();
    let s = Source::new("steps", RATE, 1, data).unwrap();
    let mut p = Project::new();
    p.add_source(s.clone());
    let revision = p.revision();
    let chop = ChopSet::grid("steps", &s, 4).unwrap();
    p.chop_to_keys(chop, 0, 4, 0, SLICE_KEYS_FROM, None)
        .unwrap();
    assert_eq!(p.pads_play(), PadsPlay::Keys { from: 36 });
    for k in 0..4 {
        assert_eq!(p.pad(k).unwrap().keys, Some(Keys::one(36 + k as u8)));
        assert_eq!(p.keygroups_for(36 + k as u8), vec![k]);
    }
    assert!(p.keygroups_for(40).is_empty());
    // Note 38 is the third slice, as recorded.
    let mut e = engine(&p);
    e.apply(Event::Key {
        note: 38,
        velocity: 127,
    });
    assert!((pitch(&render(&mut e, 10_000)) - 329.63).abs() < 1.0);
    // Pad 2 plays note 37: the second slice.
    let mut e = engine(&p);
    e.apply(Event::NoteOn {
        pad: 1,
        velocity: 127,
    });
    assert!((pitch(&render(&mut e, 10_000)) - 277.18).abs() < 1.0);
    assert!(p.undo());
    assert!(p.pad(0).is_none() && p.chop_sets.is_empty());
    assert_eq!(p.pads_play(), PadsPlay::Pads);
    assert!(p.revision() > revision);
    assert!(p.redo());
    assert_eq!(p.pads_play(), PadsPlay::Keys { from: 36 });
}

#[test]
fn bad_keys_are_refused() {
    let s = sine(0.1, 220.0);
    let mut p = Project::new();
    p.add_source(s.clone());
    let mut pad = Pad::new("k", Layer::new(Region::whole(&s)));
    pad.keys = Some(Keys {
        lo: 60,
        hi: 50,
        root: 55,
    });
    assert!(p.set_pad(0, Some(pad.clone()), None).is_err());
    pad.keys = Some(Keys {
        lo: 0,
        hi: 128,
        root: 60,
    });
    assert!(p.set_pad(0, Some(pad), None).is_err());
    assert!(p.set_pads_play(PadsPlay::Keys { from: 120 }).is_err());
    assert!(p.set_pads_play(PadsPlay::Keys { from: 112 }).is_ok());
    assert!(p.undo());
    assert_eq!(p.pads_play(), PadsPlay::Pads);
}
