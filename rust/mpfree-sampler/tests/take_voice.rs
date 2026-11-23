//! The take the Sample page shows, played by the engine off the pads (the
//! one Sample page's decision 1; mpfree-claude
//! `sampler/onepage/take-slot-check.md`): [`Event::TakeOn`] plays it on
//! voices that carry [`TAKE_PAD`], which no table indexed by pad reaches.
//! So a take never moves another kit's round robin or Mono, an event or
//! a question about pad 255 never panics, and a take follows an edit as
//! a pad does.

use mpfree_sampler::dsp::Ahdsr;
use mpfree_sampler::engine::{
    Engine, EngineView, Event, GrainView, PadField, PadOut, ProgramField, TAKE_PAD,
};
use mpfree_sampler::model::*;
use mpfree_sampler::warp::Warps;

const RATE: u32 = 44_100;
const BLOCK: usize = 128;

fn noise(frames: usize, seed: u32, name: &str) -> Source {
    let mut x = seed;
    let data = (0..frames * 2).map(|_| {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (x >> 8) as f32 / (1u32 << 24) as f32 - 0.5
    });
    Source::new(name, RATE, 2, data.collect()).unwrap()
}

/// A take of `s` played while held: it loops until let go, then ends at
/// once.
fn gate_take(s: &Source) -> Pad {
    let mut r = Region::whole(s);
    r.mode = PlayMode::Gate;
    r.sustain_loop = Some(Loop {
        start: 1_000,
        end: 9_000,
        crossfade: 200,
        mode: LoopMode::Forward,
    });
    let mut pad = Pad::new("Take 1", Layer::new(r));
    pad.amp = Ahdsr {
        attack_ms: 0.0,
        hold_ms: 0.0,
        decay_ms: 0.0,
        sustain: 1.0,
        release_ms: 1.0,
    };
    pad
}

/// A project with a gate take, and a one-shot on pad A1 from another
/// sound.
fn with_take() -> Project {
    let (a, b) = (noise(30_000, 1, "Take 1"), noise(30_000, 2, "Kick"));
    let mut p = Project::new();
    p.add_source(a.clone());
    p.add_source(b.clone());
    p.set_pad(
        0,
        Some(Pad::new("Kick", Layer::new(Region::whole(&b)))),
        None,
    )
    .unwrap();
    p.add_take(gate_take(&a)).unwrap();
    p
}

fn view(p: &Project, take: Option<usize>) -> EngineView {
    EngineView::build_with_take(p, RATE, &mut Warps::new(120.0), take)
}

/// The sum of squares of `blocks` blocks of `e`.
fn energy(e: &mut Engine, blocks: usize) -> f64 {
    let (mut l, mut r) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
    let mut sum = 0.0;
    for _ in 0..blocks {
        l.fill(0.0);
        r.fill(0.0);
        e.render(&mut l, &mut r);
        sum += l
            .iter()
            .chain(&r)
            .map(|&x| f64::from(x) * f64::from(x))
            .sum::<f64>();
    }
    sum
}

#[test]
fn a_take_plays_on_its_own_voice_and_lets_go() {
    let p = with_take();
    let mut e = Engine::new(RATE, 16);
    // A view without the take: a take hit plays nothing.
    e.set_view(view(&p, None));
    assert!(!view(&p, None).has_take());
    e.apply(Event::TakeOn { velocity: 100 });
    assert_eq!(e.active_voices(), 0);

    e.set_view(view(&p, Some(0)));
    assert!(view(&p, Some(0)).has_take());
    // Velocity 0 is no hit.
    e.apply(Event::TakeOn { velocity: 0 });
    assert_eq!(e.active_voices(), 0);
    e.apply(Event::TakeOn { velocity: 100 });
    assert_eq!(e.pad_voices(TAKE_PAD), 1);
    assert_eq!(e.pad_voices(0), 0, "pad A1 isn't hit");
    assert!(e
        .playheads()
        .all(|(pad, layer, _)| (pad, layer) == (TAKE_PAD, 0)));
    // Held, it loops on well past its loop's end.
    assert!(energy(&mut e, 200) > 1.0);
    assert_eq!(e.pad_voices(TAKE_PAD), 1);
    // Hit again, its own voice ends first: one plays.
    e.apply(Event::TakeOn { velocity: 100 });
    assert_eq!(e.pad_voices(TAKE_PAD), 1);
    // Let go, it ends.
    e.apply(Event::TakeOff);
    energy(&mut e, 20);
    assert_eq!(e.active_voices(), 0);
    assert_eq!(energy(&mut e, 4), 0.0);
    // A pad hit still plays the pad, not the take.
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 100,
    });
    assert_eq!((e.pad_voices(0), e.pad_voices(TAKE_PAD)), (1, 0));
}

#[test]
fn a_slice_of_the_take_plays_on_the_takes_voice() {
    // The Slice tool's tap on a slice of a take kept on the page.
    let p = with_take();
    let mut e = Engine::new(RATE, 16);
    let slice = Event::Audition {
        pad: TAKE_PAD,
        layer: 0,
        start: 2_000,
        end: 4_000,
        velocity: 100,
    };
    // A view without the take: nothing plays.
    e.set_view(view(&p, None));
    e.apply(slice);
    assert_eq!(e.active_voices(), 0);
    e.set_view(view(&p, Some(0)));
    // A layer the take doesn't have plays nothing either.
    e.apply(Event::Audition {
        pad: TAKE_PAD,
        layer: 1,
        start: 2_000,
        end: 4_000,
        velocity: 100,
    });
    assert_eq!(e.active_voices(), 0);
    e.apply(slice);
    assert_eq!(e.pad_voices(TAKE_PAD), 1);
    assert_eq!(e.pad_voices(0), 0, "pad A1 isn't hit");
    assert!(e
        .playheads()
        .all(|(pad, layer, at)| (pad, layer) == (TAKE_PAD, 0) && (2_000..4_000).contains(&at)));
    assert!(energy(&mut e, 4) > 0.0);
    // Tapped again, its own voice ends first: one plays.
    e.apply(slice);
    assert_eq!(e.pad_voices(TAKE_PAD), 1);
    // Let go (the take's lift), it ends.
    e.apply(Event::TakeOff);
    energy(&mut e, 20);
    assert_eq!(e.active_voices(), 0);
}

/// A PadOut keeping kit 1's pads A1 and the last pad (127), where kit 0's
/// take would land if a take had a pad number (128, or 255 with the
/// tables' `kit × 128 + pad`), and nothing else.
struct Kit1 {
    pairs: Pairs,
}

/// Kit 1's A1 and last pad, each a left and right side.
type Pairs = [[Vec<f32>; 2]; 2];

impl PadOut for Kit1 {
    fn pad(&mut self, kit: usize, pad: usize) -> Option<(&mut [f32], &mut [f32])> {
        let i = match (kit, pad) {
            (1, 0) => 0,
            (1, 127) => 1,
            _ => return None,
        };
        let [l, r] = &mut self.pairs[i];
        Some((&mut l[..], &mut r[..]))
    }
}

/// Kit 1's pads: A1 takes its two layers in turn; the last pad plays
/// Mono with Glide on every key.
fn kit1() -> Project {
    let (a, b) = (noise(20_000, 3, "a"), noise(20_000, 4, "b"));
    let mut p = Project::new();
    p.add_source(a.clone());
    p.add_source(b.clone());
    let mut rr = Pad::new("Round", Layer::new(Region::whole(&a)));
    rr.layers.push(Layer::new(Region::whole(&b)));
    rr.select = LayerSelect::Cycle;
    p.set_pad(0, Some(rr), None).unwrap();
    let mut lead = gate_take(&b);
    lead.name = "Lead".into();
    lead.keys = Some(Keys::all(60));
    lead.mono = Mono {
        on: true,
        legato: true,
        glide_ms: 80.0,
        glide_held: false,
        env_continue: false,
    };
    p.set_pad(127, Some(lead), None).unwrap();
    p
}

/// Kit 1's two pairs, block by block, with kit 0's take hit (and let go)
/// on every block if `takes`. Also the layer each hit on kit 1's A1 took.
fn kit1_heard(takes: bool) -> (Vec<Pairs>, Vec<u8>) {
    let mut e = Engine::with_kits(RATE, 16, 2);
    e.set_kit_view(0, view(&with_take(), Some(0)));
    e.set_kit_view(1, view(&kit1(), None));
    let mut out = Kit1 {
        pairs: std::array::from_fn(|_| [vec![0.0; BLOCK], vec![0.0; BLOCK]]),
    };
    let (mut heard, mut layers) = (Vec::new(), Vec::new());
    for b in 0..48u32 {
        if takes {
            let ev = if b % 2 == 0 {
                Event::TakeOn { velocity: 90 }
            } else {
                Event::TakeOff
            };
            assert!(e.schedule_for(0, 7, ev));
        }
        match b {
            // Round robin on A1: a hit every 8 blocks.
            b if b % 8 == 0 => assert!(e.schedule_for(
                1,
                11,
                Event::NoteOn {
                    pad: 0,
                    velocity: 100
                }
            )),
            // Mono: a held key, another over it (a glide), let go of.
            3 => assert!(e.schedule_for(
                1,
                0,
                Event::Key {
                    note: 60,
                    velocity: 100
                }
            )),
            9 => assert!(e.schedule_for(
                1,
                5,
                Event::Key {
                    note: 67,
                    velocity: 100
                }
            )),
            20 => assert!(e.schedule_for(1, 9, Event::KeyOff { note: 67 })),
            30 => assert!(e.schedule_for(1, 1, Event::KeyOff { note: 60 })),
            _ => {}
        }
        for pair in out.pairs.iter_mut().flatten() {
            pair.fill(0.0);
        }
        e.render_routed(BLOCK, &mut out);
        heard.push(out.pairs.clone());
        if b % 8 == 0 {
            // The newest voice on kit 1's A1.
            let newest = e
                .kit_playheads(1)
                .filter(|&(pad, _, _)| pad == 0)
                .min_by_key(|&(_, _, frame)| frame)
                .map(|(_, layer, _)| layer);
            layers.push(newest.unwrap());
        }
    }
    (heard, layers)
}

#[test]
fn a_take_leaves_another_kits_round_robin_and_mono_as_they_were() {
    let (alone, layers) = kit1_heard(false);
    assert_eq!(layers, [0, 1, 0, 1, 0, 1], "A1 takes its layers in turn");
    let (with_takes, layers_with) = kit1_heard(true);
    assert_eq!(layers_with, layers);
    for (b, (x, y)) in alone.iter().zip(&with_takes).enumerate() {
        assert_eq!(x, y, "kit 1's A1 and last pad, block {b}");
    }
    let sum = |h: &[Pairs], i: usize| -> f32 {
        h.iter()
            .flat_map(|b| b[i].iter().flatten())
            .map(|x| x.abs())
            .sum()
    };
    assert!(sum(&alone, 0) > 1.0 && sum(&alone, 1) > 1.0, "both sounded");
}

/// Every event and question on pad 255 (the take's voices' pad number),
/// with the take sounding and without, on one kit and on two: none
/// panics, and none but the take's own plays the take.
#[test]
fn pad_255_is_safe_everywhere() {
    let p = with_take();
    let mut e = Engine::with_kits(RATE, 16, 2);
    for k in 0..2 {
        e.set_kit_view(k, view(&p, Some(0)));
    }
    let mut grains = vec![GrainView::default(); 64];
    let pad = TAKE_PAD;
    for k in 0..2 {
        let mut ask = |e: &mut Engine| {
            let _ = e.kit_pad_voices(k, pad);
            let _ = e.pad_voices(pad);
            let _ = e.kit_grains_of(k, pad, 0, &mut grains);
            let _ = e.grains_of(pad, 0, &mut grains);
            let _ = e.cloud_of(pad, 0);
            let _ = e.kit_cloud_of(k, pad, 0);
            let _ = e.kit_playheads(k).count();
            let _ = e.latched_pads();
            for f in PadField::ALL {
                let _ = e.kit_pad_live_value(k, pad, f);
            }
            energy(e, 2);
        };
        // A pad hit on 255 plays nothing: the take isn't a pad.
        e.apply_for(k, Event::NoteOn { pad, velocity: 100 });
        assert_eq!(e.kit_pad_voices(k, pad), 0);
        ask(&mut e);
        for take_on in [false, true] {
            if take_on {
                e.apply_for(k, Event::TakeOn { velocity: 100 });
                assert_eq!(e.kit_pad_voices(k, pad), 1);
            }
            for ev in [
                Event::Pressure { pad, value: 90 },
                Event::PadLive {
                    pad,
                    field: PadField::Level,
                    value: -6.0,
                },
                Event::PadLock {
                    pad,
                    field: PadField::Decay,
                    value: 50.0,
                },
                Event::PadLive {
                    pad,
                    field: PadField::Level,
                    value: f32::NAN,
                },
                Event::PadLock {
                    pad,
                    field: PadField::Decay,
                    value: f32::NAN,
                },
                Event::ProgramLive {
                    field: ProgramField::Tune,
                    value: 100.0,
                },
                Event::Audition {
                    pad,
                    layer: 0,
                    start: 0,
                    end: 1_000,
                    velocity: 100,
                },
                Event::LiveChop { pad: Some(pad) },
                Event::Mark,
                Event::LiveChop { pad: None },
                Event::NoteOn { pad, velocity: 100 },
                Event::NoteOff { pad },
                Event::ProgramLive {
                    field: ProgramField::Tune,
                    value: 0.0,
                },
            ] {
                e.apply_for(k, ev);
                ask(&mut e);
            }
            // An edit while it sounds: the take's voice follows it.
            e.edit_kit_view(k, view(&p, Some(0)));
            ask(&mut e);
            e.apply_for(k, Event::TakeOff);
            e.apply_for(k, Event::AllOff);
            ask(&mut e);
        }
    }
}

#[test]
fn a_sounding_take_follows_an_edit_and_a_new_view() {
    let mut p = with_take();
    let mut e = Engine::new(RATE, 16);
    e.set_view(view(&p, Some(0)));
    e.apply(Event::TakeOn { velocity: 127 });
    energy(&mut e, 40);
    let loud = energy(&mut e, 40);
    // Turned down 20 dB while it sounds: it follows, gliding there.
    p.edit_take(0, None, |t| t.level_db = -20.0).unwrap();
    e.edit_view(view(&p, Some(0)));
    energy(&mut e, 40);
    let soft = energy(&mut e, 40);
    assert_eq!(e.pad_voices(TAKE_PAD), 1);
    let db = 10.0 * (soft / loud).log10();
    assert!((db + 20.0).abs() < 1.0, "{db:.2} dB");
    // A view without the take (it went onto a pad): the voice plays on
    // as it started, and lets go as it would have.
    p.put_take_on(0, 5).unwrap();
    e.set_view(view(&p, None));
    assert!(energy(&mut e, 10) > 0.0);
    assert_eq!(e.pad_voices(TAKE_PAD), 1);
    e.apply(Event::TakeOff);
    energy(&mut e, 20);
    assert_eq!(e.active_voices(), 0);
    // Now it is pad A6, and plays as one.
    e.apply(Event::NoteOn {
        pad: 5,
        velocity: 127,
    });
    assert_eq!((e.pad_voices(5), e.pad_voices(TAKE_PAD)), (1, 0));
}
