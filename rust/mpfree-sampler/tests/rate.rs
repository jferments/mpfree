//! A sound at another rate than the output's, converted when it is
//! loaded: it keeps its pitch and level, everything placed on it moves to
//! the same moments, and its hits play as plain drum hits again.

use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::*;

const RATE: u32 = 44_100;

fn sine(rate: u32, seconds: f64, hz: f64) -> Source {
    let n = (seconds * f64::from(rate)) as usize;
    let v = (0..n)
        .map(|i| (0.5 * (std::f64::consts::TAU * hz * i as f64 / f64::from(rate)).sin()) as f32)
        .collect();
    Source::new("tone", rate, 1, v).unwrap()
}

/// A pad on `s` with every kind of frame set: a trimmed region, a loop
/// with a crossfade, fades, warp pins.
fn busy_pad(s: &Source) -> Pad {
    let r = f64::from(s.rate());
    let at = |sec: f64| (sec * r).round() as u64;
    let mut region = Region::whole(s);
    region.start = at(0.1);
    region.end = at(0.9);
    region.mode = PlayMode::Loop;
    region.sustain_loop = Some(Loop {
        start: at(0.3),
        end: at(0.7),
        crossfade: at(0.02) as u32,
        mode: LoopMode::Forward,
    });
    region.fade_in.frames = at(0.01) as u32;
    region.fade_out.frames = at(0.05) as u32;
    let mut w = Warp::new(120.0);
    w.pins = vec![
        Pin {
            frame: at(0.35),
            beat: 0.5,
        },
        Pin {
            frame: at(0.6),
            beat: 1.0,
        },
    ];
    region.warp = Some(w);
    region.check(s).expect("the pad as made is valid");
    Pad::new("busy", Layer::new(region))
}

#[test]
fn a_converted_sound_keeps_every_point_at_the_same_moment() {
    let s = sine(48_000, 1.0, 1_000.0);
    let pad = busy_pad(&s);
    let chop = ChopSet::from_markers("c", &s, ChopMode::Manual, [12_000, 24_000, 36_000]).unwrap();
    let mut sources = vec![s.clone()];
    let mut pads = vec![pad.clone()];
    let mut chops = vec![chop];
    let moved = convert_to_rate(&mut sources, &mut pads, &mut chops, RATE);
    assert_eq!(moved.len(), 1);
    let t = &sources[0];
    assert_eq!(
        (t.rate(), t.frames(), t.name.as_str()),
        (RATE, 44_100, "tone")
    );
    assert_eq!(moved[0].from, s.id());
    assert_eq!(moved[0].to, t.id());
    let (a, b) = (&pad.layers[0].region, &pads[0].layers[0].region);
    b.check(t).expect("still valid on the new sound");
    // Seconds on the old sound, and on the new.
    let sec = |f: u64, r: u32| f as f64 / f64::from(r);
    let close = |x: u64, y: u64| (sec(x, 48_000) - sec(y, RATE)).abs() < 1.5 / f64::from(RATE);
    assert!(close(a.start, b.start) && close(a.end, b.end));
    let (la, lb) = (a.sustain_loop.unwrap(), b.sustain_loop.unwrap());
    assert!(close(la.start, lb.start) && close(la.end, lb.end));
    assert!(close(u64::from(la.crossfade), u64::from(lb.crossfade)));
    assert!(close(
        u64::from(a.fade_in.frames),
        u64::from(b.fade_in.frames)
    ));
    assert!(close(
        u64::from(a.fade_out.frames),
        u64::from(b.fade_out.frames)
    ));
    let (pa, pb) = (
        &a.warp.as_ref().unwrap().pins,
        &b.warp.as_ref().unwrap().pins,
    );
    assert_eq!(pa.len(), pb.len());
    for (x, y) in pa.iter().zip(pb) {
        assert!(close(x.frame, y.frame) && x.beat == y.beat);
    }
    assert_eq!(chops[0].markers(), &[11_025, 22_050, 33_075]);
    // The other settings are the pad's own still.
    let mut same = pads[0].clone();
    same.layers[0].region = pad.layers[0].region.clone();
    assert_eq!(same, pad);
}

#[test]
fn a_sound_at_the_output_rate_is_left_as_it_is() {
    let s = sine(RATE, 1.0, 440.0);
    let mut sources = vec![s.clone()];
    let mut pads = vec![busy_pad(&s)];
    let before = pads.clone();
    assert!(convert_to_rate(&mut sources, &mut pads, &mut [], RATE).is_empty());
    assert_eq!(sources[0].id(), s.id());
    assert_eq!(pads, before);
}

#[test]
fn a_sound_too_short_to_convert_stays_as_it_was() {
    // One frame at 96 kHz is under half a frame at 44.1 kHz.
    let s = Source::new("click", 96_000, 1, vec![0.5]).unwrap();
    let mut sources = vec![s.clone()];
    assert!(convert_to_rate(&mut sources, &mut [], &mut [], RATE).is_empty());
    assert_eq!(sources[0].id(), s.id());
}

/// What `pads` drum hits on sounds at `rate` play through an engine that
/// allows one full voice: how many sound at once.
fn voices_after_hits(rate: u32, convert: bool) -> usize {
    let mut p = Project::new();
    for i in 0..3 {
        let mut s = sine(rate, 0.5, 200.0 + 100.0 * i as f64);
        if convert {
            s = s.at_rate(RATE).unwrap().0;
        }
        p.add_source(s.clone());
        p.set_pad(
            i,
            Some(Pad::new("hit", Layer::new(Region::whole(&s)))),
            None,
        )
        .unwrap();
    }
    let mut e = Engine::new(RATE, 8);
    e.set_full_voices(1);
    e.set_view(EngineView::build(&p, RATE));
    for pad in 0..3 {
        e.apply(Event::NoteOn { pad, velocity: 100 });
    }
    let (mut l, mut r) = (vec![0.0; 256], vec![0.0; 256]);
    e.render(&mut l, &mut r);
    e.active_voices()
}

#[test]
fn hits_on_a_converted_sound_play_as_plain_drum_hits() {
    // At 48 kHz every hit is read faster than the output's rate: a full
    // voice, so with one allowed, each hit takes the last one's place.
    assert_eq!(voices_after_hits(48_000, false), 1);
    // Converted, they are plain hits again and all three ring together.
    assert_eq!(voices_after_hits(48_000, true), 3);
}

#[test]
fn a_converted_sound_plays_its_tone_at_its_pitch_and_level() {
    let s = sine(48_000, 0.5, 3_000.0);
    let (t, _) = s.at_rate(RATE).unwrap();
    let render = |s: &Source| {
        let mut p = Project::new();
        p.add_source(s.clone());
        let mut r = Region::whole(s);
        r.mode = PlayMode::Gate;
        p.set_pad(0, Some(Pad::new("t", Layer::new(r))), None)
            .unwrap();
        let mut e = Engine::new(RATE, 4);
        e.set_view(EngineView::build(&p, RATE));
        e.apply(Event::NoteOn {
            pad: 0,
            velocity: 127,
        });
        let (mut l, mut r) = (vec![0.0; 12_000], vec![0.0; 12_000]);
        for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
            e.render(a, b);
        }
        l
    };
    let level = |x: &[f32]| {
        let x = &x[2_000..10_000];
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, &v) in x.iter().enumerate() {
            let ph = std::f64::consts::TAU * 3_000.0 * i as f64 / f64::from(RATE);
            re += f64::from(v) * ph.cos();
            im += f64::from(v) * ph.sin();
        }
        2.0 * (re * re + im * im).sqrt() / x.len() as f64
    };
    let (a, b) = (level(&render(&s)), level(&render(&t)));
    assert!(
        (20.0 * (b / a).log10()).abs() < 0.05,
        "as it was {a:.4}, converted {b:.4}"
    );
}

#[test]
fn a_kit_read_from_a_file_is_put_at_the_output_rate_with_its_scenes_and_slices() {
    let s = sine(48_000, 1.0, 1_000.0);
    let kept = sine(RATE, 0.5, 440.0);
    let mut p = Project::new();
    p.add_source(s.clone());
    p.add_source(kept.clone());
    p.set_pad(0, Some(busy_pad(&s)), None).unwrap();
    p.set_pad(
        1,
        Some(Pad::new("kept", Layer::new(Region::whole(&kept)))),
        None,
    )
    .unwrap();
    let chop = ChopSet::from_markers("c", &s, ChopMode::Manual, [12_000, 24_000]).unwrap();
    p.add_chop_set(chop).unwrap();
    p.scenes[0] = Some(mpfree_sampler::scene::Scene::of(&p));
    let kept_pad = p.pad(1).cloned();
    let revision = p.revision();
    let moved = p.convert_to_rate(RATE);
    assert_eq!(moved.len(), 1);
    let to = moved[0].to;
    assert_eq!(p.sources().len(), 2);
    assert!(p.sources().iter().all(|s| s.rate() == RATE));
    let t = p.source(to).expect("the converted sound is in the kit");
    let on = |pad: &Pad| pad.layers[0].region.source;
    assert_eq!(on(p.pad(0).unwrap()), to);
    p.pad(0).unwrap().layers[0].region.check(t).unwrap();
    let scene = p.scenes[0].as_ref().unwrap();
    assert_eq!(on(scene.pad(0).unwrap()), to);
    assert_eq!(scene.pad(0), p.pad(0), "the scene moved as the pad did");
    assert_eq!(p.pad(1).cloned(), kept_pad, "a sound at the rate stays");
    assert_eq!(p.chop_sets[0].source, to);
    assert_eq!(p.chop_sets[0].markers(), &[11_025, 22_050]);
    // Undo can't take apart what was converted, and the view rebuilds.
    assert!(!p.can_undo());
    assert!(p.revision() > revision);
    // A second time there is nothing to do.
    assert!(p.convert_to_rate(RATE).is_empty());
}
