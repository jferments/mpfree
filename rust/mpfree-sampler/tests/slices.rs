//! Sliced pads (R4): one pad holds a whole break as slices. A hit plays the
//! pad's Slice (or the one a step lock or knob picks), a note on keys plays
//! its own slice at the sound's pitch, and every slice sounds as Chop to
//! pads' pad for it does.

use mpfree_sampler::engine::{Engine, EngineView, Event, PadField};
use mpfree_sampler::model::*;
use mpfree_sampler::save;

const RATE: u32 = 44_100;
const SLICES: usize = 8;
const LEN: usize = 8 * 4410;

/// A sound whose every frame is different, so a slice is known by what it
/// plays.
fn ramp() -> Source {
    let x: Vec<f32> = (0..LEN)
        .map(|i| (i as f32 * 0.37).sin() * 0.5 + (i as f32 / LEN as f32) * 0.4)
        .collect();
    Source::new("break", RATE, 1, x).unwrap()
}

fn project(s: &Source, pads: &[(usize, Pad)]) -> Project {
    let mut p = Project::new();
    p.add_source(s.clone());
    for (i, pad) in pads {
        p.set_pad(*i, Some(pad.clone()), None).unwrap();
    }
    p
}

fn engine(p: &Project) -> Engine {
    let mut e = Engine::new(RATE, 16);
    e.set_view(EngineView::build(p, RATE));
    e
}

fn run(e: &mut Engine, frames: usize) -> Vec<f32> {
    let mut l = vec![0.0; frames];
    let mut r = vec![0.0; frames];
    for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        e.render(a, b);
    }
    l
}

/// What a hit on `pad` plays, from a fresh engine.
fn hit(p: &Project, pad: u8, before: &[Event]) -> Vec<f32> {
    let mut e = engine(p);
    for ev in before {
        e.apply(*ev);
    }
    e.apply(Event::NoteOn { pad, velocity: 127 });
    run(&mut e, 6000)
}

fn sliced(s: &Source, slice: u8) -> Pad {
    let chop = ChopSet::grid("break", s, SLICES).unwrap();
    let mut p = Pad::new("sliced", Layer::new(chop.sliced(s).unwrap()));
    p.slice = slice;
    p
}

fn chopped(s: &Source, i: usize) -> Pad {
    let chop = ChopSet::grid("break", s, SLICES).unwrap();
    Pad::new("chopped", Layer::new(chop.region(s, i).unwrap()))
}

#[test]
fn every_slice_plays_as_chop_to_pads_pad_for_it_does() {
    let s = ramp();
    for i in 0..SLICES {
        let p = project(&s, &[(0, sliced(&s, i as u8)), (1, chopped(&s, i))]);
        let (a, b) = (hit(&p, 0, &[]), hit(&p, 1, &[]));
        assert!(b.iter().any(|x| x.abs() > 0.01), "slice {i} sounds");
        assert_eq!(a, b, "slice {i}");
    }
}

#[test]
fn a_lock_or_a_knob_picks_the_slice_rounded_and_nan_gives_the_pads_own_back() {
    let s = ramp();
    let p = project(
        &s,
        &[(0, sliced(&s, 1)), (1, chopped(&s, 5)), (2, chopped(&s, 1))],
    );
    let five = hit(&p, 1, &[]);
    let lock = Event::PadLock {
        pad: 0,
        field: PadField::Slice,
        value: 4.6,
    };
    assert_eq!(hit(&p, 0, &[lock]), five, "a lock of 4.6 plays slice 6");
    let live = Event::PadLive {
        pad: 0,
        field: PadField::Slice,
        value: 5.0,
    };
    assert_eq!(hit(&p, 0, &[live]), five);
    let back = Event::PadLive {
        pad: 0,
        field: PadField::Slice,
        value: f32::NAN,
    };
    assert_eq!(hit(&p, 0, &[live, back]), hit(&p, 2, &[]), "its own again");
}

#[test]
fn a_note_on_keys_plays_its_own_slice_at_the_sounds_pitch() {
    let s = ramp();
    let mut pad = sliced(&s, 0);
    pad.keys = Some(Keys {
        lo: 36,
        hi: 127,
        root: 60,
    });
    let p = project(&s, &[(0, pad), (1, chopped(&s, 3))]);
    let mut e = engine(&p);
    e.apply(Event::Key {
        note: 39,
        velocity: 127,
    });
    assert_eq!(run(&mut e, 6000), hit(&p, 1, &[]), "note 39 is slice 4");
    // Past the last slice: nothing.
    let mut e = engine(&p);
    e.apply(Event::Key {
        note: 36 + SLICES as u8,
        velocity: 127,
    });
    assert!(run(&mut e, 2000).iter().all(|x| *x == 0.0));
}

#[test]
fn a_sliced_pad_is_saved_and_opens_with_its_slices() {
    let s = ramp();
    let p = project(&s, &[(0, sliced(&s, 3))]);
    let text = save::to_text(&[("kit", &p)], 0, &|_| "break.wav".into());
    assert!(text.contains("region.slices = "), "{text}");
    let opened = save::from_text(&text, &mut |_| Ok(s.clone())).unwrap();
    let (_, q) = &opened.kits[0];
    let pad = q.pad(0).unwrap();
    assert_eq!(pad.slice, 3);
    assert_eq!(pad.layers[0].region.slice_count(), SLICES);
    assert_eq!(
        pad.layers[0].region.slices,
        p.pad(0).unwrap().layers[0].region.slices
    );
}

#[test]
fn a_trim_passes_over_the_slices_left_outside_and_bad_ones_are_refused() {
    let s = ramp();
    let mut r = ChopSet::grid("break", &s, SLICES)
        .unwrap()
        .sliced(&s)
        .unwrap();
    let step = (LEN / SLICES) as u64;
    assert_eq!(r.slice_bounds(2), Some((2 * step, 3 * step)));
    r.start = 2 * step + 10;
    assert_eq!(r.slice_count(), SLICES - 2);
    assert_eq!(r.slice_bounds(0), Some((2 * step + 10, 3 * step)));
    assert_eq!(r.slice_bounds(SLICES - 2), None);
    assert!(r.check(&s).is_ok(), "a trim keeps the region good");
    let mut bad = r.clone();
    bad.slices.swap(0, 1);
    assert!(bad.check(&s).is_err(), "out of order");
    let mut many = Region::whole(&s);
    many.slices = (1..=MAX_SLICES as u64).collect();
    assert!(many.check(&s).is_err(), "past {MAX_SLICES} slices");
}

/// A second of 220 Hz, then a second of 440 Hz, stereo.
fn two_tones() -> Source {
    let tone = |hz: f64, v: &mut Vec<f32>| {
        for i in 0..RATE as usize {
            let x = (0.5 * (std::f64::consts::TAU * hz * i as f64 / f64::from(RATE)).sin()) as f32;
            v.extend([x, x]);
        }
    };
    let mut v = Vec::new();
    tone(220.0, &mut v);
    tone(440.0, &mut v);
    Source::new("two", RATE, 2, v).unwrap()
}

#[test]
fn a_stretched_sliced_pad_plays_each_slice_of_its_copy() {
    // Trimmed to start half a second in and cut where the tone changes,
    // warped from 100 to 120 BPM: the copy is 1/1.2 as long.
    let s = two_tones();
    let mut r = Region::whole(&s);
    r.start = 22_050;
    r.slices = vec![44_100];
    r.warp = Some(Warp {
        bpm: 100.0,
        method: WarpMethod::Stretch,
        pins: Vec::new(),
    });
    let p = project(&s, &[(0, Pad::new("two", Layer::new(r)))]);
    let mut warps = mpfree_sampler::warp::Warps::new(120.0);
    for k in warps.missing(&p) {
        let mut job = mpfree_sampler::warp::Warps::job(&p, &k).unwrap();
        while !job.run(4096).finished {}
        warps.insert(k, job.into_output().unwrap(), &p);
    }
    for (slice, frames, hz) in [(0, 18_375.0, 220.0), (1, 36_750.0, 440.0)] {
        let mut e = Engine::new(RATE, 8);
        e.set_tempo(120.0);
        e.set_view(EngineView::build_warped(&p, RATE, &mut warps));
        e.apply(Event::PadLive {
            pad: 0,
            field: PadField::Slice,
            value: slice as f32,
        });
        e.apply(Event::NoteOn {
            pad: 0,
            velocity: 127,
        });
        let mut out = Vec::new();
        let (mut l, mut r) = (vec![0.0; 128], vec![0.0; 128]);
        while e.active_voices() > 0 && out.len() < 4 * RATE as usize {
            e.render(&mut l, &mut r);
            out.extend_from_slice(&l);
        }
        let end = out
            .iter()
            .rposition(|v| v.abs() > 1e-6)
            .map_or(0, |i| i + 1);
        out.truncate(end);
        assert!(
            (out.len() as f64 - frames).abs() < 300.0,
            "slice {slice}: {} frames",
            out.len()
        );
        let m = out.len() / 2;
        let heard = mpfree_analysis::yin(&out[m - 4096..m + 4096], RATE, 4096, 60.0, 2000.0, 0.15)
            .expect("pitched")
            .0;
        assert!((heard - hz).abs() < 2.0, "slice {slice}: {heard} Hz");
    }
}

// R4 part 5: settings per slice.

/// `sliced(s, slice)` with slice `i`'s own settings `set`.
fn with_set(s: &Source, slice: u8, i: usize, set: SliceSet) -> Pad {
    let mut p = sliced(s, slice);
    p.layers[0].region.set_slice_set(i, set);
    p
}

/// What a hit on pad 0 of `p` plays, to the end of its voice.
fn whole_hit(p: &Project) -> Vec<f32> {
    let mut e = engine(p);
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
    let mut out = Vec::new();
    let (mut l, mut r) = (vec![0.0; 128], vec![0.0; 128]);
    while e.active_voices() > 0 && out.len() < 4 * RATE as usize {
        e.render(&mut l, &mut r);
        out.extend_from_slice(&l);
    }
    let end = out
        .iter()
        .rposition(|v| v.abs() > 1e-6)
        .map_or(0, |i| i + 1);
    out.truncate(end);
    out
}

fn hz(x: &[f32]) -> f32 {
    let m = x.len() / 2;
    mpfree_analysis::yin(&x[m - 4096..m + 4096], RATE, 4096, 60.0, 2000.0, 0.15)
        .expect("pitched")
        .0
}

#[test]
fn a_slices_own_level_and_reverse_change_that_slice_alone() {
    let s = ramp();
    let quiet = SliceSet {
        level_db: -6.0,
        ..SliceSet::DEFAULT
    };
    let plain = project(&s, &[(0, sliced(&s, 2)), (1, sliced(&s, 3))]);
    let p = project(
        &s,
        &[
            (0, with_set(&s, 2, 2, quiet)),
            (1, with_set(&s, 3, 2, quiet)),
        ],
    );
    let (a, b) = (hit(&p, 0, &[]), hit(&plain, 0, &[]));
    let g = 10f32.powf(-6.0 / 20.0);
    assert!(b.iter().any(|x| x.abs() > 0.01));
    for (x, y) in a.iter().zip(&b) {
        assert!((x - y * g).abs() < 1e-5, "{x} against {y} at -6 dB");
    }
    assert_eq!(hit(&p, 1, &[]), hit(&plain, 1, &[]), "slice 4 as it was");
    // Reversed, slice 3 plays as Chop to pads' reversed pad for it.
    let back = SliceSet {
        reverse: true,
        ..SliceSet::DEFAULT
    };
    let mut pad = chopped(&s, 2);
    pad.layers[0].region.reverse = true;
    let p = project(&s, &[(0, with_set(&s, 2, 2, back)), (1, pad)]);
    assert_eq!(hit(&p, 0, &[]), hit(&p, 1, &[]));
}

#[test]
fn a_slices_own_pitch_repitches_it_and_plays_to_takes_it_on_to_the_end() {
    // Two slices of a second each: 220 Hz, then 440 Hz.
    let s = two_tones();
    let mut r = Region::whole(&s);
    r.slices = vec![44_100];
    let pad = |set: SliceSet| {
        let mut r = r.clone();
        r.set_slice_set(0, set);
        Pad::new("two", Layer::new(r))
    };
    let plain = whole_hit(&project(&s, &[(0, pad(SliceSet::DEFAULT))]));
    assert!(
        (plain.len() as f64 - 44_100.0).abs() < 300.0,
        "{}",
        plain.len()
    );
    assert!((hz(&plain) - 220.0).abs() < 2.0);
    let up = whole_hit(&project(
        &s,
        &[(
            0,
            pad(SliceSet {
                tune_st: 12,
                ..SliceSet::DEFAULT
            }),
        )],
    ));
    assert!((up.len() as f64 - 22_050.0).abs() < 300.0, "{}", up.len());
    assert!((hz(&up) - 440.0).abs() < 2.0, "{} Hz", hz(&up));
    let on = whole_hit(&project(
        &s,
        &[(
            0,
            pad(SliceSet {
                to_end: true,
                ..SliceSet::DEFAULT
            }),
        )],
    ));
    assert!((on.len() as f64 - 88_200.0).abs() < 300.0, "{}", on.len());
    assert_eq!(on[..40_000], plain[..40_000], "the same slice, on further");
}

#[test]
fn a_slices_own_fade_and_cutoff_shape_it() {
    let s = ramp();
    let plain = hit(&project(&s, &[(0, sliced(&s, 2))]), 0, &[]);
    let fade = SliceSet {
        fade_out_ms: Some(20.0),
        ..SliceSet::DEFAULT
    };
    let faded = hit(&project(&s, &[(0, with_set(&s, 2, 2, fade))]), 0, &[]);
    let n = LEN / SLICES;
    assert_eq!(
        faded[..n - 900],
        plain[..n - 900],
        "the same until the fade"
    );
    let tail = |x: &[f32]| x[n - 300..n].iter().map(|v| v.abs()).sum::<f32>();
    assert!(tail(&faded) < 0.2 * tail(&plain), "faded out");
    // A low cutoff on the 440 Hz slice takes most of it away.
    let s = two_tones();
    let mut r = Region::whole(&s);
    r.slices = vec![44_100];
    let pad = |cutoff_hz: Option<f32>| {
        let mut r = r.clone();
        r.set_slice_set(
            1,
            SliceSet {
                cutoff_hz,
                ..SliceSet::DEFAULT
            },
        );
        let mut p = Pad::new("two", Layer::new(r));
        p.slice = 1;
        p
    };
    let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
    let open = hit(&project(&s, &[(0, pad(None))]), 0, &[]);
    let low = hit(&project(&s, &[(0, pad(Some(100.0)))]), 0, &[]);
    assert!(rms(&low[2000..]) < 0.25 * rms(&open[2000..]), "filtered");
}

#[test]
fn slice_settings_are_saved_kept_by_a_chop_onto_the_pad_and_checked() {
    let s = ramp();
    let set = SliceSet {
        tune_st: -5,
        level_db: -3.5,
        reverse: true,
        cutoff_hz: Some(1200.0),
        fade_out_ms: Some(15.0),
        to_end: true,
    };
    let pad = with_set(&s, 0, 3, set);
    let mut r = pad.layers[0].region.clone();
    assert_eq!(r.slice_sets.len(), 4, "defaults up to slice 4");
    assert_eq!(r.slice_set(3), set);
    assert_eq!(r.slice_set(7), SliceSet::DEFAULT);
    r.set_slice_set(3, SliceSet::DEFAULT);
    assert!(r.slice_sets.is_empty(), "no default entries at the end");
    let p = project(&s, &[(0, pad), (1, chopped(&s, 0))]);
    let text = save::to_text(&[("kit", &p)], 0, &|_| "break.wav".into());
    assert!(text.contains("region.slice_sets = yes"), "{text}");
    assert!(text.contains("region.slice_sets = no"), "the plain pad");
    let opened = save::from_text(&text, &mut |_| Ok(s.clone())).unwrap();
    let (_, q) = &opened.kits[0];
    let back = &q.pad(0).unwrap().layers[0].region;
    assert_eq!(
        back.slice_sets,
        p.pad(0).unwrap().layers[0].region.slice_sets
    );
    assert!(q.pad(1).unwrap().layers[0].region.slice_sets.is_empty());
    // Chopped again onto itself (the pad as the template), it keeps them.
    let mut p = p;
    let c = ChopSet::grid("break", &s, SLICES).unwrap();
    let own = p.pad(0).cloned();
    p.chop_to_one_pad(c, 0, SLICES, 0, own.as_ref()).unwrap();
    assert_eq!(p.pad(0).unwrap().layers[0].region.slice_set(3), set);
    // Out of range: refused.
    let mut bad = Region::whole(&s);
    bad.slices = vec![1000];
    bad.set_slice_set(
        0,
        SliceSet {
            tune_st: 30,
            ..SliceSet::DEFAULT
        },
    );
    assert!(bad.check(&s).is_err());
}
