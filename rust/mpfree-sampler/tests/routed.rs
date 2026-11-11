//! `Engine::render_routed`: each pad's voices into its own pair (a mixer's
//! pad channels), adding up to what `Engine::render` mixes.

use mpfree_sampler::dsp::{Ahdsr, Filter, FilterKind};
use mpfree_sampler::engine::{Engine, EngineView, Event, PadOut};
use mpfree_sampler::model::*;

const RATE: u32 = 44_100;
const PAIRS: usize = 16;

/// Sixteen pad channels, like the mixer's: pads past them, or turned off,
/// get no output. Nothing here clears the pairs; `clear` does, as the mixer
/// does after each block.
struct Channels {
    left: Vec<Vec<f32>>,
    right: Vec<Vec<f32>>,
    off: Vec<usize>,
    asked: Vec<usize>,
}

impl Channels {
    fn new(block: usize) -> Channels {
        Channels {
            left: vec![vec![0.0; block]; PAIRS],
            right: vec![vec![0.0; block]; PAIRS],
            off: vec![],
            asked: vec![],
        }
    }

    fn clear(&mut self) {
        for c in self.left.iter_mut().chain(self.right.iter_mut()) {
            c.fill(0.0);
        }
    }
}

impl PadOut for Channels {
    fn pad(&mut self, pad: usize) -> Option<(&mut [f32], &mut [f32])> {
        self.asked.push(pad);
        if pad >= PAIRS || self.off.contains(&pad) {
            return None;
        }
        Some((&mut self.left[pad][..], &mut self.right[pad][..]))
    }
}

fn noise(frames: usize, seed: u32) -> Source {
    let mut x = seed;
    let data = (0..frames * 2).map(|_| {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (x >> 8) as f32 / (1u32 << 24) as f32 - 0.5
    });
    Source::new("noise", RATE, 2, data.collect()).unwrap()
}

/// Pads 0 to 3: a one-shot, a filtered loop, a reversed gate pad with an
/// envelope, and a granular pad.
fn kit() -> Project {
    let s = noise(20_000, 7);
    let mut p = Project::new();
    p.add_source(s.clone());
    let one = Region::whole(&s);
    let mut lp = Region::whole(&s);
    lp.mode = PlayMode::Loop;
    lp.sustain_loop = Some(Loop {
        start: 2000,
        end: 6000,
        crossfade: 300,
        mode: LoopMode::Forward,
    });
    let mut gate = Region::whole(&s);
    gate.mode = PlayMode::Gate;
    gate.reverse = true;
    gate.tune_cents = 700;
    let mut grains = Region::whole(&s);
    grains.mode = PlayMode::Granular;
    grains.grain = GrainParams {
        density: 200.0,
        size_ms: 40.0,
        spray: 0.4,
        ..GrainParams::default()
    };

    p.set_pad(0, Some(Pad::new("one", Layer::new(one))), None)
        .unwrap();
    let mut pad = Pad::new("loop", Layer::new(lp));
    pad.filter = Filter {
        kind: FilterKind::LowPass,
        poles: 4,
        cutoff_hz: 1500.0,
        resonance: 0.4,
        env_octaves: 2.0,
    };
    p.set_pad(1, Some(pad), None).unwrap();
    let mut pad = Pad::new("gate", Layer::new(gate));
    pad.amp = Ahdsr {
        attack_ms: 3.0,
        hold_ms: 0.0,
        decay_ms: 40.0,
        sustain: 0.5,
        release_ms: 20.0,
    };
    p.set_pad(2, Some(pad), None).unwrap();
    p.set_pad(3, Some(Pad::new("grains", Layer::new(grains))), None)
        .unwrap();
    p
}

fn engine(p: &Project) -> Engine {
    let mut e = Engine::new(RATE, 8);
    e.set_view(EngineView::build(p, RATE));
    e
}

/// The same hits and lifts, block by block, on frames inside the block.
fn events(block: u32) -> Vec<(u32, Event)> {
    match block {
        0 => vec![
            (
                0,
                Event::NoteOn {
                    pad: 0,
                    velocity: 120,
                },
            ),
            (
                17,
                Event::NoteOn {
                    pad: 1,
                    velocity: 90,
                },
            ),
        ],
        2 => vec![
            (
                5,
                Event::NoteOn {
                    pad: 2,
                    velocity: 100,
                },
            ),
            (
                99,
                Event::NoteOn {
                    pad: 3,
                    velocity: 110,
                },
            ),
        ],
        9 => vec![(64, Event::NoteOff { pad: 2 })],
        14 => vec![
            (1, Event::NoteOff { pad: 1 }),
            (33, Event::NoteOff { pad: 3 }),
        ],
        20 => vec![(
            70,
            Event::NoteOn {
                pad: 0,
                velocity: 60,
            },
        )],
        _ => vec![],
    }
}

fn play(e: &mut Engine, block: u32) {
    for (o, ev) in events(block) {
        e.schedule(o, ev);
    }
}

/// [`play`], but only the events for `pads`.
fn play_only(e: &mut Engine, block: u32, pads: &[u8]) {
    for (o, ev) in events(block) {
        let pad = match ev {
            Event::NoteOn { pad, .. } | Event::NoteOff { pad } => pad,
            _ => continue,
        };
        if pads.contains(&pad) {
            e.schedule(o, ev);
        }
    }
}

#[test]
fn the_pads_pairs_add_up_to_the_mixed_render() {
    let p = kit();
    let (mut mixed, mut routed) = (engine(&p), engine(&p));
    let mut ch = Channels::new(128);
    let (mut l, mut r) = (vec![0.0f32; 128], vec![0.0f32; 128]);
    let mut heard = [0f32; PAIRS];
    for block in 0..60 {
        play(&mut mixed, block);
        play(&mut routed, block);
        mixed.render(&mut l, &mut r);
        routed.render_routed(128, &mut ch);
        for k in 0..128 {
            let (sl, sr): (f32, f32) = (
                ch.left.iter().map(|c| c[k]).sum(),
                ch.right.iter().map(|c| c[k]).sum(),
            );
            assert!(
                (sl - l[k]).abs() < 1e-5 && (sr - r[k]).abs() < 1e-5,
                "block {block} frame {k}: {sl} {sr} vs {} {}",
                l[k],
                r[k]
            );
        }
        for (pad, h) in heard.iter_mut().enumerate() {
            *h = h.max(
                ch.left[pad]
                    .iter()
                    .chain(&ch.right[pad])
                    .fold(0f32, |m, x| m.max(x.abs())),
            );
        }
        assert_eq!(
            mixed.active_voices(),
            routed.active_voices(),
            "block {block}"
        );
        ch.clear();
    }
    // Every pad hit is heard on its own pair, and no other pair hears anything.
    for (pad, h) in heard.iter().enumerate() {
        assert_eq!(*h > 0.01, pad < 4, "pad {pad}: peak {h}");
    }
    // Only the pads that play are asked for their pair.
    assert!(ch.asked.iter().all(|p| *p < 4));
    assert_eq!(mixed.frames(), routed.frames());
}

#[test]
fn each_pair_is_that_pad_alone_and_is_added_to_from_the_events_frame() {
    // Pad 1's pair holds what pad 1 plays alone, and the engine adds into
    // the pair (the mixer clears it), from the hit's frame on.
    let p = kit();
    let mut alone = engine(&p);
    let mut both = engine(&p);
    let mut ch = Channels::new(256);
    for c in ch.left.iter_mut().chain(ch.right.iter_mut()) {
        c.fill(1.0);
    }
    let (mut l, mut r) = (vec![0.0f32; 256], vec![0.0f32; 256]);
    alone.schedule(
        40,
        Event::NoteOn {
            pad: 1,
            velocity: 90,
        },
    );
    both.schedule(
        40,
        Event::NoteOn {
            pad: 1,
            velocity: 90,
        },
    );
    both.schedule(
        3,
        Event::NoteOn {
            pad: 0,
            velocity: 127,
        },
    );
    alone.render(&mut l, &mut r);
    both.render_routed(256, &mut ch);
    for k in 0..256 {
        assert!((ch.left[1][k] - 1.0 - l[k]).abs() < 1e-6, "frame {k}");
        assert!((ch.right[1][k] - 1.0 - r[k]).abs() < 1e-6, "frame {k}");
    }
    assert!(l[40..].iter().any(|x| x.abs() > 0.01), "pad 1 plays");
    assert!(
        ch.left[1][..40].iter().all(|x| *x == 1.0),
        "nothing before the hit's frame"
    );
    assert!(
        ch.left[0][..3].iter().all(|x| *x == 1.0) && ch.left[0][3..40].iter().any(|x| *x != 1.0),
        "pad 0 starts on frame 3"
    );
    assert!(
        ch.left[5].iter().all(|x| *x == 1.0),
        "pads that don't play are left alone"
    );
}

#[test]
fn a_pad_without_an_output_goes_unheard_but_plays_on_and_ends_on_time() {
    // Pad 1 (the loop) and pad 3 (grains) have no channel. Their voices
    // still play, through blocks longer than the engine's scratch buffer,
    // and end on the same frames as in the mixed render.
    let p = kit();
    let (mut mixed, mut routed, mut only) = (engine(&p), engine(&p), engine(&p));
    let mut ch = Channels::new(600);
    ch.off = vec![1, 3];
    let (mut l, mut r) = (vec![0.0f32; 600], vec![0.0f32; 600]);
    let (mut ol, mut or) = (vec![0.0f32; 600], vec![0.0f32; 600]);
    for block in 0..60 {
        play(&mut mixed, block);
        play(&mut routed, block);
        // `only` plays pads 0 and 2, so its render is what routed lets through.
        play_only(&mut only, block, &[0, 2]);
        mixed.render(&mut l, &mut r);
        routed.render_routed(600, &mut ch);
        only.render(&mut ol, &mut or);
        let heard: Vec<f32> = (0..600).map(|k| ch.left[0][k] + ch.left[2][k]).collect();
        for k in 0..600 {
            assert!((heard[k] - ol[k]).abs() < 1e-6, "block {block} frame {k}");
        }
        assert!(ch.left[1].iter().chain(&ch.left[3]).all(|x| *x == 0.0));
        assert_eq!(
            mixed.active_voices(),
            routed.active_voices(),
            "block {block}"
        );
        assert_eq!(mixed.live_grains(), routed.live_grains(), "block {block}");
        ch.clear();
    }
    assert_eq!(
        (
            routed.pad_voices(1),
            routed.pad_voices(3),
            mixed.pad_voices(1)
        ),
        (0, 0, 0),
        "the unheard pads were lifted and ended"
    );
}

#[test]
fn the_master_effect_only_runs_on_the_mixed_render() {
    let mut p = kit();
    let mut plain = engine(&p);
    p.set_master(
        MasterFx {
            mix: 0.8,
            ..MasterFx::default()
        },
        None,
    )
    .unwrap();
    let mut wet = engine(&p);
    assert!(wet.set_master_fx(Engine::make_master_fx(RATE)).is_none());
    let (mut a, mut b) = (Channels::new(128), Channels::new(128));
    for block in 0..20 {
        play(&mut plain, block);
        play(&mut wet, block);
        plain.render_routed(128, &mut a);
        wet.render_routed(128, &mut b);
        assert_eq!(a.left, b.left, "block {block}");
        assert_eq!(a.right, b.right, "block {block}");
        a.clear();
        b.clear();
    }
    assert_eq!(wet.master_fx().unwrap().stats().spawned, 0);
}
