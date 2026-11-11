//! The master effect: grains from the last few seconds of everything the
//! pads play, mixed back over it, with freeze.

use mpfree_sampler::engine::{Engine, EngineView, Event, MASTER_MEMORY_S};
use mpfree_sampler::model::*;

const RATE: u32 = 44_100;

fn source(name: &str, frames: usize, f: impl Fn(usize) -> f32) -> Source {
    Source::new(name, RATE, 1, (0..frames).map(f).collect()).unwrap()
}
/// A second of 440 Hz.
fn tone() -> Source {
    source("tone", RATE as usize, |i| {
        0.5 * (i as f32 * 440.0 * std::f32::consts::TAU / RATE as f32).sin()
    })
}

fn cloud() -> GrainParams {
    GrainParams {
        position: 0.9,
        spray: 0.05,
        size_ms: 80.0,
        density: 30.0,
        spread: 0.5,
        ..GrainParams::default()
    }
}

fn project(s: &Source, mix: f32) -> Project {
    let mut p = Project::new();
    p.add_source(s.clone());
    p.set_pad(
        0,
        Some(Pad::new("tone", Layer::new(Region::whole(s)))),
        None,
    )
    .unwrap();
    p.set_master(
        MasterFx {
            mix,
            grain: cloud(),
        },
        None,
    )
    .unwrap();
    p
}

fn engine(p: &Project, with_fx: bool) -> Engine {
    let mut e = Engine::new(RATE, 8);
    e.set_view(EngineView::build(p, RATE));
    if with_fx {
        assert!(e.set_master_fx(Engine::make_master_fx(RATE)).is_none());
    }
    e
}

/// Renders `frames` in blocks of `block`, scheduling each event on its
/// exact frame.
fn play(
    e: &mut Engine,
    frames: usize,
    block: usize,
    events: &[(usize, Event)],
) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
    let mut at = 0;
    while at < frames {
        let n = block.min(frames - at);
        for &(f, ev) in events {
            if (at..at + n).contains(&f) {
                assert!(e.schedule((f - at) as u32, ev));
            }
        }
        e.render(&mut l[at..at + n], &mut r[at..at + n]);
        at += n;
    }
    (l, r)
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn crossings(x: &[f32]) -> usize {
    x.windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count()
}

const HIT: Event = Event::NoteOn {
    pad: 0,
    velocity: 127,
};

#[test]
fn master_settings_are_checked_and_undoable() {
    let mut p = Project::new();
    assert_eq!(*p.master(), MasterFx::default());
    assert_eq!(p.master().mix, 0.0, "a new project is dry");
    let d = MasterFx::default();
    for bad in [
        MasterFx { mix: 1.5, ..d },
        MasterFx { mix: -0.1, ..d },
        MasterFx { mix: f32::NAN, ..d },
        MasterFx {
            grain: GrainParams {
                density: f32::INFINITY,
                ..d.grain
            },
            ..d
        },
    ] {
        assert_eq!(p.set_master(bad, None), Err(ModelError::BadMasterFx));
    }
    assert!(!p.can_undo(), "a refused change leaves nothing to undo");
    assert_eq!(*p.master(), d);

    // A knob turn is one step.
    for k in 1..=10 {
        p.set_master(
            MasterFx {
                mix: k as f32 / 10.0,
                ..*p.master()
            },
            Some(7),
        )
        .unwrap();
    }
    assert_eq!(p.master().mix, 1.0);
    p.set_master(
        MasterFx {
            grain: GrainParams {
                size_ms: 300.0,
                ..p.master().grain
            },
            ..*p.master()
        },
        None,
    )
    .unwrap();
    let revision = p.revision();
    assert!(p.undo());
    assert!(p.revision() > revision);
    assert_eq!(
        (p.master().mix, p.master().grain.size_ms),
        (1.0, d.grain.size_ms)
    );
    assert!(p.undo());
    assert_eq!(*p.master(), d, "the whole knob turn undone at once");
    assert!(!p.undo());
    assert!(p.redo());
    assert_eq!(p.master().mix, 1.0);
    assert!(p.redo());
    assert_eq!(p.master().grain.size_ms, 300.0);
}

#[test]
fn a_dry_master_changes_nothing() {
    let s = tone();
    let p = project(&s, 0.0);
    let events = [
        (100, HIT),
        (20_000, Event::Freeze { on: true }),
        (30_000, HIT),
        (40_000, Event::Freeze { on: false }),
    ];
    let plain = play(&mut engine(&p, false), 60_000, 128, &events);
    let mut e = engine(&p, true);
    let with = play(&mut e, 60_000, 128, &events);
    assert!(
        plain.0 == with.0 && plain.1 == with.1,
        "fully dry, the output is the pads exactly"
    );
    let fx = e.master_fx().unwrap();
    assert_eq!(fx.stats().spawned, 0, "and no grains are made");
    // It keeps listening (except while frozen), so the mix has something to
    // play: 40 000 frames, less the 32 waiting for the next 64-frame chunk.
    assert_eq!(fx.buffer().written(), 40_000 - 32);
    assert_eq!(
        fx.buffer().capacity(),
        (MASTER_MEMORY_S * RATE as f32) as usize
    );
}

#[test]
fn freeze_holds_the_pads_after_they_stop() {
    // The tone plays for a second; the output is all grains (mix 1).
    let s = tone();
    let p = project(&s, 1.0);
    let tail = |freeze: bool| {
        let mut e = engine(&p, true);
        let mut events = vec![(0, HIT)];
        if freeze {
            events.push((40_000, Event::Freeze { on: true }));
        }
        let (l, _) = play(&mut e, RATE as usize * 3, 128, &events);
        assert_eq!(e.master_fx().unwrap().frozen(), freeze);
        l[RATE as usize * 2..].to_vec() // a second after the tone ended
    };
    let (held, gone) = (tail(true), tail(false));
    assert!(
        rms(&held) > 0.05,
        "frozen, the cloud keeps playing the tone: rms {}",
        rms(&held)
    );
    assert!(
        rms(&gone) < 1e-4,
        "not frozen, it plays the silence that came after: rms {}",
        rms(&gone)
    );
    let c = crossings(&held) as f32;
    assert!(
        (c / 880.0 - 1.0).abs() < 0.05,
        "still 440 Hz: {c} crossings in a second"
    );
}

#[test]
fn the_output_is_the_same_at_any_period_size() {
    // Sample pads and a granular pad under a half-wet master, frozen and
    // let go on odd frames.
    let s = tone();
    let mut p = project(&s, 0.7);
    let mut r = Region::whole(&s);
    r.mode = PlayMode::Granular;
    r.grain = GrainParams {
        spray: 0.4,
        size_ms: 60.0,
        density: 40.0,
        pitch_spray: 2.0,
        ..GrainParams::default()
    };
    p.set_pad(1, Some(Pad::new("g", Layer::new(r))), None)
        .unwrap();
    p.set_master(
        MasterFx {
            mix: 0.7,
            grain: GrainParams {
                spray: 0.3,
                reverse: 0.3,
                pitch_spray: 3.0,
                ..cloud()
            },
        },
        None,
    )
    .unwrap();
    let events = [
        (0, HIT),
        (
            5_003,
            Event::NoteOn {
                pad: 1,
                velocity: 100,
            },
        ),
        (21_011, Event::Freeze { on: true }),
        (30_500, HIT),
        (40_000, Event::NoteOff { pad: 1 }),
        (47_777, Event::Freeze { on: false }),
        (52_000, HIT),
    ];
    let render = |block: usize| play(&mut engine(&p, true), 66_150, block, &events);
    let first = render(128);
    // As for the effect alone: the same to within float rounding of the
    // grains' read positions.
    for block in [1, 37, 64, 500, 4096] {
        let other = render(block);
        let worst = first
            .0
            .iter()
            .chain(&first.1)
            .zip(other.0.iter().chain(&other.1))
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-4, "period {block}: {worst}");
    }
}

#[test]
fn the_mix_follows_the_view_and_hands_its_grains_back() {
    let s = source("dc", RATE as usize * 4, |_| 0.5);
    let dry_p = project(&s, 0.0);
    let mut wet_p = dry_p.clone();
    wet_p
        .set_master(
            MasterFx {
                mix: 1.0,
                ..*dry_p.master()
            },
            None,
        )
        .unwrap();
    let (dry_v, wet_v) = (
        EngineView::build(&dry_p, RATE),
        EngineView::build(&wet_p, RATE),
    );

    let mut plain = engine(&dry_p, false);
    let mut e = engine(&dry_p, true);
    plain.apply(HIT);
    e.apply(HIT);
    let mut l = vec![0.0; 40_000];
    let mut r = vec![0.0; 40_000];
    let (mut pl, mut pr) = (vec![0.0; 40_000], vec![0.0; 40_000]);
    let mut back = None;
    for (k, ((a, b), (c, d))) in l
        .chunks_mut(128)
        .zip(r.chunks_mut(128))
        .zip(pl.chunks_mut(128).zip(pr.chunks_mut(128)))
        .enumerate()
    {
        if k == 40 {
            back = Some(e.set_view(wet_v.clone()));
        }
        if k == 200 {
            e.set_view(back.take().unwrap());
        }
        e.render(a, b);
        plain.render(c, d);
    }
    let (on, off) = (40 * 128, 200 * 128);
    assert!(l[..on] == pl[..on], "dry until the view brings the mix up");
    let step = l
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max);
    assert!(step < 0.01, "the mix glides both ways: largest step {step}");
    assert!(rms(&l[on + 2000..off]) > 0.05, "fully wet, the grains play");
    assert!(
        l[off + 441..] == pl[off + 441..] && r[off + 441..] == pr[off + 441..],
        "back to exactly dry after a 10 ms glide"
    );
    assert_eq!(
        e.live_grains(),
        0,
        "dry again, the master's grains are handed back"
    );
    assert!(e.master_fx().unwrap().stats().spawned > 0);

    // Taking it out hands back the effect, its grains silenced.
    e.set_view(wet_v);
    let mut x = [0.0; 4410];
    let mut y = [0.0; 4410];
    e.render(&mut x, &mut y);
    assert!(e.live_grains() > 0);
    let old = e.set_master_fx(None).expect("the old effect");
    assert_eq!((old.stats().active, e.live_grains()), (0, 0));
    assert!(e.master_fx().is_none());
    // Freeze without an effect does nothing; the pads carry on as before.
    e.apply(Event::Freeze { on: true });
    e.render(&mut x, &mut y);
    assert!(
        x.iter().all(|&v| v == x[0]) && x[0] > 0.3,
        "the dc pad, dry: {}",
        x[0]
    );
    drop(dry_v);
}

#[test]
fn a_new_master_effect_glides_in() {
    // Whatever the mix, an effect put in while the pads play comes in from
    // dry over 10 ms instead of jumping.
    let s = source("dc", RATE as usize * 2, |_| 0.5);
    let p = project(&s, 1.0);
    let mut e = engine(&p, false);
    e.apply(HIT);
    let mut l = vec![0.0; 20_000];
    let mut r = vec![0.0; 20_000];
    for (k, (a, b)) in l.chunks_mut(128).zip(r.chunks_mut(128)).enumerate() {
        if k == 50 {
            assert!(e.set_master_fx(Engine::make_master_fx(RATE)).is_none());
        }
        e.render(a, b);
    }
    let level = l[0];
    assert!(
        level > 0.3 && l[..50 * 128].iter().all(|&x| x == level),
        "dry until it goes in"
    );
    let step = l
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max);
    assert!(step < 0.01, "a glide, not a jump of {level}: step {step}");
    assert!(Engine::make_master_fx(0).is_none());
}
