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

/// Pads 0, 1, 2 and 5 holding sines at A3, A4, C5 and A4 again, as a
/// folder of samples would land on a kit, and pad 3 left empty.
fn four_notes() -> Project {
    let mut p = Project::new();
    for (pad, hz) in [(0usize, 220.0), (1, 440.0), (2, 523.25), (5, 440.0)] {
        let mut s = sine(0.5, hz);
        s.name = format!("tone {pad}");
        p.add_source(s.clone());
        let mut r = Region::whole(&s);
        r.mode = PlayMode::Gate;
        p.set_pad(pad, Some(Pad::new(&s.name, Layer::new(r))), None)
            .unwrap();
    }
    p
}

#[test]
fn map_to_keys_spreads_the_pads_halfway_to_each_other_and_one_undo_takes_it_back() {
    let mut p = four_notes();
    let before: Vec<Option<Pad>> = (0..6).map(|k| p.pad(k).cloned()).collect();
    let keys = p
        .map_to_keys(&[(0, 57), (1, 69), (2, 72), (5, 69)])
        .unwrap();
    // A3 reaches down to the bottom and halfway up to A4; A4 halfway to
    // C5; C5 to the top. Pad 5 shares A4's zone.
    let a3 = Keys {
        lo: 0,
        hi: 62,
        root: 57,
    };
    let a4 = Keys {
        lo: 63,
        hi: 70,
        root: 69,
    };
    let c5 = Keys {
        lo: 71,
        hi: 127,
        root: 72,
    };
    assert_eq!(keys, [a3, a4, c5, a4]);
    for (pad, k) in [(0, a3), (1, a4), (2, c5), (5, a4)] {
        let pad = p.pad(pad).unwrap();
        assert_eq!(pad.keys, Some(k));
        assert_eq!(pad.polyphony, KEYGROUP_POLYPHONY);
        assert_eq!(pad.layers.len(), 1);
    }
    assert_eq!(p.pads_play(), PadsPlay::Keys { from: 48 });
    assert_eq!(p.keygroups_for(69), [1, 5]);
    // Each note plays at its own pitch from the nearest root below or above.
    for (note, hz) in [(57u8, 220.0f32), (62, 293.66), (64, 329.63), (74, 587.33)] {
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
    // One undo puts every pad and the pads' notes back.
    assert!(p.undo());
    let after: Vec<Option<Pad>> = (0..6).map(|k| p.pad(k).cloned()).collect();
    assert_eq!(after, before);
    assert_eq!(p.pads_play(), PadsPlay::Pads);
}

#[test]
fn map_to_keys_refuses_an_empty_pad_and_changes_nothing() {
    let mut p = four_notes();
    let rev = p.revision();
    assert_eq!(p.map_to_keys(&[(0, 57), (3, 60)]), Err(ModelError::BadPad));
    assert_eq!(p.map_to_keys(&[]), Err(ModelError::BadPad));
    assert_eq!(p.map_to_keys(&[(PADS, 60)]), Err(ModelError::BadPad));
    assert_eq!(p.revision(), rev);
    assert_eq!(p.pad(0).unwrap().keys, None);
}

#[test]
fn edges_fade_on_an_equal_power_curve_but_not_at_the_ends() {
    // No fade: full level inside, nothing outside.
    assert_eq!(edge_gain(60, 48, 72, 0, 0, 127), 1.0);
    assert_eq!(edge_gain(47, 48, 72, 4, 0, 127), 0.0);
    // A fade of 4 rises over the first four notes and falls over the last four.
    let g: Vec<f32> = (48..=72).map(|n| edge_gain(n, 48, 72, 4, 0, 127)).collect();
    let ramp = [0.309017, 0.587785, 0.809017, 0.951057];
    for k in 0..4 {
        assert!((g[k] - ramp[k]).abs() < 1e-5, "{k}: {}", g[k]);
        assert!((g[24 - k] - ramp[k]).abs() < 1e-5, "{k}: {}", g[24 - k]);
    }
    assert!(g[4..=20].iter().all(|&x| x == 1.0));
    // Two zones overlapping by the fade sum to full power on every note.
    for n in 56..=63 {
        let a = edge_gain(n, 40, 59, 4, 0, 127);
        let b = edge_gain(n, 56, 80, 4, 0, 127);
        assert!((a * a + b * b - 1.0).abs() < 1e-5, "note {n}: {a} {b}");
    }
    // The ends of the range never fade.
    assert_eq!(edge_gain(0, 0, 20, 8, 0, 127), 1.0);
    assert_eq!(edge_gain(127, 100, 127, 8, 0, 127), 1.0);
    assert_eq!(edge_gain(1, 1, 64, 8, 1, 127), 1.0);
    assert_eq!(edge_gain(1, 0, 64, 8, 1, 127), 1.0);
}

fn rms(x: &[f32]) -> f32 {
    let x = &x[2000..10_000];
    (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
}

/// One gate pad of a 220 Hz sine, its level fixed whatever the velocity.
fn fading(keys: Option<Keys>, key_fade: u8, vel_fade: u8) -> Project {
    let s = sine(1.0, 220.0);
    let mut p = Project::new();
    p.add_source(s.clone());
    let mut r = Region::whole(&s);
    r.mode = PlayMode::Gate;
    let mut pad = Pad::new("keys", Layer::new(r));
    pad.vel_to_level = 0.0;
    pad.keys = keys;
    pad.key_fade = key_fade;
    pad.vel_fade = vel_fade;
    pad.polyphony = KEYGROUP_POLYPHONY;
    p.set_pad(0, Some(pad), None).unwrap();
    p
}

#[test]
fn a_key_fade_lowers_the_notes_near_the_zones_edges_on_its_curve() {
    let keys = Keys {
        lo: 52,
        hi: 72,
        root: 57,
    };
    let p = fading(Some(keys), 4, 0);
    let level = |note: u8| {
        let mut e = engine(&p);
        e.apply(Event::Key {
            note,
            velocity: 100,
        });
        rms(&render(&mut e, 12_000))
    };
    let full = level(60);
    for note in [52u8, 53, 54, 55, 56, 69, 72] {
        let want = edge_gain(note, 52, 72, 4, 0, 127);
        let got = level(note) / full;
        assert!((got - want).abs() < 0.02, "note {note}: {got} for {want}");
    }
    // A hit on the pad itself plays it as recorded.
    let mut e = engine(&p);
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 100,
    });
    assert!((rms(&render(&mut e, 12_000)) / full - 1.0).abs() < 0.02);
    // Without the fade every note is at full level.
    let p = fading(Some(keys), 0, 0);
    let mut e = engine(&p);
    e.apply(Event::Key {
        note: 52,
        velocity: 100,
    });
    assert!((rms(&render(&mut e, 12_000)) / full - 1.0).abs() < 0.02);
}

#[test]
fn a_velocity_fade_crossfades_overlapping_layers() {
    // Two stacked layers, soft 1-80 and hard 73-127: they overlap by 8.
    let mut p = fading(None, 0, 8);
    let mut pad = p.pad(0).unwrap().clone();
    let mut hard = pad.layers[0].clone();
    pad.layers[0].vel_hi = 80;
    hard.vel_lo = 73;
    pad.layers.push(hard);
    p.set_pad(0, Some(pad.clone()), None).unwrap();
    let level = |p: &Project, velocity: u8| {
        let mut e = engine(p);
        e.apply(Event::NoteOn { pad: 0, velocity });
        rms(&render(&mut e, 12_000))
    };
    let one = level(&p, 40);
    for v in [73u8, 76, 80] {
        let soft = edge_gain(v, 1, 80, 8, 1, 127);
        let hard = edge_gain(v, 73, 127, 8, 1, 127);
        // The layers are the same sound in phase: their levels add.
        let got = level(&p, v) / one;
        assert!((got - (soft + hard)).abs() < 0.02, "velocity {v}: {got}");
    }
    assert!((level(&p, 127) / one - 1.0).abs() < 0.02, "127 is the top");
    // In Cycle, each set of round robins takes turns and the overlapping
    // sets both play: two round robins at each velocity, four layers.
    let mut rr = pad.clone();
    rr.select = LayerSelect::Cycle;
    let (soft, hard) = (rr.layers[0].clone(), rr.layers[1].clone());
    rr.layers = vec![soft.clone(), hard.clone(), soft, hard];
    p.set_pad(0, Some(rr.clone()), None).unwrap();
    let mut e = engine(&p);
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 76,
    });
    render(&mut e, 256);
    assert_eq!(e.active_voices(), 2, "one of each set in the overlap");
    let mut e = engine(&p);
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 40,
    });
    render(&mut e, 256);
    assert_eq!(e.active_voices(), 1, "one round robin below the overlap");
    // Without a fade, Cycle takes turns over every matching layer, as before.
    rr.vel_fade = 0;
    p.set_pad(0, Some(rr), None).unwrap();
    let mut e = engine(&p);
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 76,
    });
    render(&mut e, 256);
    assert_eq!(e.active_voices(), 1);
}

#[test]
fn fades_past_their_limits_are_refused() {
    let mut p = fading(None, 0, 0);
    let mut pad = p.pad(0).unwrap().clone();
    pad.key_fade = MAX_KEY_FADE + 1;
    assert!(p.set_pad(0, Some(pad.clone()), None).is_err());
    pad.key_fade = MAX_KEY_FADE;
    pad.vel_fade = MAX_VEL_FADE + 1;
    assert!(p.set_pad(0, Some(pad.clone()), None).is_err());
    pad.vel_fade = MAX_VEL_FADE;
    assert!(p.set_pad(0, Some(pad), None).is_ok());
}

#[test]
fn ranges_that_meet_overlap_by_the_fade_centred_where_they_met() {
    // Three velocity layers edge to edge, as auto-sample makes them.
    let mut r = vec![(1, 40), (41, 90), (91, 127)];
    overlap_ranges(&mut r, 8);
    assert_eq!(r, vec![(1, 44), (37, 94), (87, 127)]);
    // Again with another fade: the same edges move, from the same middle.
    overlap_ranges(&mut r, 3);
    assert_eq!(r, vec![(1, 42), (40, 92), (90, 127)]);
    overlap_ranges(&mut r, 0);
    assert_eq!(r, vec![(1, 40), (41, 90), (91, 127)]);
    // A gap, a stack and a fade wider than a range are left sensible.
    let mut r = vec![(0, 30), (40, 60), (40, 127)];
    overlap_ranges(&mut r, 6);
    assert_eq!(r, vec![(0, 30), (40, 60), (40, 127)], "a gap and a stack");
    let mut r = vec![(0, 50), (51, 53), (54, 127)];
    overlap_ranges(&mut r, 24);
    assert!(
        r.windows(2).all(|w| w[0].0 < w[1].0 && w[0].1 < w[1].1),
        "{r:?}"
    );
}

#[test]
fn a_velocity_fade_moves_the_layers_to_overlap_round_robins_together() {
    let mut pad = fading(None, 0, 0).pad(0).unwrap().clone();
    let l = pad.layers[0].clone();
    pad.layers.clear();
    for (lo, hi) in [(1, 40), (41, 90), (91, 127)] {
        for _ in 0..2 {
            let mut x = l.clone();
            (x.vel_lo, x.vel_hi) = (lo, hi);
            pad.layers.push(x);
        }
    }
    pad.set_vel_fade(8);
    let got: Vec<(u8, u8)> = pad.layers.iter().map(|l| (l.vel_lo, l.vel_hi)).collect();
    assert_eq!(
        got,
        vec![(1, 44), (1, 44), (37, 94), (37, 94), (87, 127), (87, 127)]
    );
    assert_eq!(pad.vel_fade, 8);
    pad.set_vel_fade(200);
    assert_eq!(pad.vel_fade, MAX_VEL_FADE);
}

#[test]
fn a_key_fade_for_the_kit_overlaps_the_zones_and_one_undo_takes_it_back() {
    // Three keygroups edge to edge, as Map to keys leaves them, and a
    // pad that isn't one.
    let s = sine(1.0, 220.0);
    let mut p = Project::new();
    p.add_source(s.clone());
    let pad = Pad::new("keys", Layer::new(Region::whole(&s)));
    for (i, (lo, hi, root)) in [(0, 52, 48), (53, 62, 57), (63, 127, 67)]
        .into_iter()
        .enumerate()
    {
        let mut q = pad.clone();
        q.keys = Some(Keys { lo, hi, root });
        p.set_pad(i, Some(q), None).unwrap();
    }
    p.set_pad(5, Some(pad), None).unwrap();
    assert_eq!(p.key_fade(), Some(0));
    p.set_key_fade(4).unwrap();
    let zones: Vec<(u8, u8)> = (0..3)
        .map(|i| p.pad(i).unwrap().keys.map(|k| (k.lo, k.hi)).unwrap())
        .collect();
    assert_eq!(zones, vec![(0, 54), (51, 64), (61, 127)]);
    assert_eq!(p.key_fade(), Some(4));
    assert_eq!(p.pad(5).unwrap().key_fade, 0, "not a keygroup");
    // Each note in an overlap sums to full power.
    for n in 51..=54 {
        let a = edge_gain(n, 0, 54, 4, 0, 127);
        let b = edge_gain(n, 51, 64, 4, 0, 127);
        assert!((a * a + b * b - 1.0).abs() < 1e-5, "note {n}");
    }
    assert!(p.undo());
    assert_eq!(p.key_fade(), Some(0));
    assert_eq!(p.pad(1).unwrap().keys.map(|k| (k.lo, k.hi)), Some((53, 62)));
    // A kit without keygroups has no key fade to set.
    let mut q = Project::new();
    assert_eq!(q.key_fade(), None);
    assert!(q.set_key_fade(3).is_err());
}
