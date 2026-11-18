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
