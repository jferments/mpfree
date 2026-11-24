//! Kits: one engine playing several kits at once from one voice pool
//! (`Engine::with_kits`), as a kit per track does. Each kit has its own
//! view, live settings and pads; a voice knows its kit, so mute groups,
//! lifts and edits stay within it, while stealing looks at every voice.

use mpfree_sampler::dsp::Ahdsr;
use mpfree_sampler::engine::{Engine, EngineView, Event, PadField, PadOut, ProgramField};
use mpfree_sampler::model::*;

const RATE: u32 = 44_100;
const BLOCK: usize = 128;
/// Pads each kit has a pair for here.
const PAIRS: usize = 4;

/// A pair for each of the first [`PAIRS`] pads of each kit.
struct Pairs {
    left: Vec<Vec<f32>>,
    right: Vec<Vec<f32>>,
}

impl Pairs {
    fn new(kits: usize) -> Pairs {
        Pairs {
            left: vec![vec![0.0; BLOCK]; kits * PAIRS],
            right: vec![vec![0.0; BLOCK]; kits * PAIRS],
        }
    }

    fn clear(&mut self) {
        for c in self.left.iter_mut().chain(self.right.iter_mut()) {
            c.fill(0.0);
        }
    }

    fn energy(&self, kit: usize, pad: usize) -> f64 {
        let i = kit * PAIRS + pad;
        self.left[i]
            .iter()
            .chain(&self.right[i])
            .map(|&x| f64::from(x) * f64::from(x))
            .sum()
    }
}

impl PadOut for Pairs {
    fn pad(&mut self, kit: usize, pad: usize) -> Option<(&mut [f32], &mut [f32])> {
        if pad >= PAIRS || kit * PAIRS >= self.left.len() {
            return None;
        }
        let i = kit * PAIRS + pad;
        Some((&mut self.left[i][..], &mut self.right[i][..]))
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

/// Pads 0 to 2: a one-shot, a looping gate pad with a short release, and a
/// one-shot pitched up a fifth, from a noise of its own (`seed`).
fn kit(seed: u32) -> Project {
    let s = noise(30_000, seed);
    let mut p = Project::new();
    p.add_source(s.clone());
    p.set_pad(
        0,
        Some(Pad::new("one", Layer::new(Region::whole(&s)))),
        None,
    )
    .unwrap();
    let mut gate = Region::whole(&s);
    gate.mode = PlayMode::Gate;
    gate.sustain_loop = Some(Loop {
        start: 1000,
        end: 9000,
        crossfade: 200,
        mode: LoopMode::Forward,
    });
    let mut pad = Pad::new("gate", Layer::new(gate));
    pad.amp = Ahdsr {
        attack_ms: 0.0,
        hold_ms: 0.0,
        decay_ms: 0.0,
        sustain: 1.0,
        release_ms: 1.0,
    };
    p.set_pad(1, Some(pad), None).unwrap();
    let mut up = Region::whole(&s);
    up.tune_cents = 700;
    p.set_pad(2, Some(Pad::new("up", Layer::new(up))), None)
        .unwrap();
    p
}

fn view(p: &Project) -> EngineView {
    EngineView::build(p, RATE)
}

/// `blocks` blocks of `e` routed into `out`, with `events` (block, offset,
/// kit, event) on their frames. Returns each block's pairs, kit by kit.
fn run(
    e: &mut Engine,
    kits: usize,
    blocks: u32,
    events: &[(u32, u32, usize, Event)],
) -> Vec<Vec<Vec<f32>>> {
    let mut out = Pairs::new(kits);
    let mut all = Vec::new();
    for b in 0..blocks {
        for &(_, o, k, ev) in events.iter().filter(|x| x.0 == b) {
            assert!(e.schedule_for(k, o, ev));
        }
        out.clear();
        e.render_routed(BLOCK, &mut out);
        let mut block = out.left.clone();
        block.extend(out.right.iter().cloned());
        all.push(block);
    }
    all
}

/// The hits, block by block: kit 0 and kit 1 on their own frames.
fn hits() -> Vec<(u32, u32, usize, Event)> {
    let on = |pad, velocity| Event::NoteOn { pad, velocity };
    vec![
        (0, 3, 0, on(0, 110)),
        (0, 40, 1, on(0, 90)),
        (1, 17, 1, on(1, 127)),
        (2, 0, 0, on(2, 64)),
        (3, 99, 0, on(1, 100)),
        (6, 5, 1, Event::NoteOff { pad: 1 }),
        (8, 70, 1, on(2, 120)),
        (12, 1, 0, Event::NoteOff { pad: 1 }),
    ]
}

/// Each kit sounds in its own pairs exactly as an engine of that kit alone
/// would play it, bit for bit, while the other kit plays from the same pool.
#[test]
fn each_kit_plays_into_its_own_pairs_as_it_would_alone() {
    let (a, b) = (kit(1), kit(2));
    let mut both = Engine::with_kits(RATE, 16, 2);
    assert_eq!(both.kits(), 2);
    both.set_kit_view(0, view(&a));
    both.set_kit_view(1, view(&b));
    let together = run(&mut both, 2, 40, &hits());

    for (k, p) in [(0, &a), (1, &b)] {
        let mut alone = Engine::new(RATE, 16);
        alone.set_view(view(p));
        let only: Vec<_> = hits()
            .into_iter()
            .filter(|h| h.2 == k)
            .map(|(b, o, _, ev)| (b, o, 0, ev))
            .collect();
        let solo = run(&mut alone, 1, 40, &only);
        let mut heard = 0.0;
        for (block, (t, s)) in together.iter().zip(&solo).enumerate() {
            for pad in 0..PAIRS {
                // `together` holds both kits' left pairs, then their right.
                let (tl, tr) = (&t[k * PAIRS + pad], &t[2 * PAIRS + k * PAIRS + pad]);
                let (sl, sr) = (&s[pad], &s[PAIRS + pad]);
                assert_eq!(tl, sl, "kit {k}, pad {pad}, block {block}, left");
                assert_eq!(tr, sr, "kit {k}, pad {pad}, block {block}, right");
                heard += tl.iter().map(|x| x.abs()).sum::<f32>();
            }
        }
        assert!(heard > 1.0, "kit {k} sounded");
    }
}

/// A mute group, a lift and a second hit on a pad act on that kit's own
/// voices only: kit 1's pads never cut kit 0's, though their pads have the
/// same numbers and groups.
#[test]
fn mute_groups_and_lifts_stay_within_a_kit() {
    let mut p = kit(3);
    for pad in [0, 1] {
        let mut x = p.pad(pad).unwrap().clone();
        x.mute_group = Some(1);
        p.set_pad(pad, Some(x), None).unwrap();
    }
    let mut e = Engine::with_kits(RATE, 16, 2);
    e.set_kit_view(0, view(&p));
    e.set_kit_view(1, view(&p));
    let on = |pad| Event::NoteOn { pad, velocity: 100 };

    // Kit 0 holds its gate pad; kit 1's pad 0, in the same group number,
    // leaves it alone.
    e.apply_for(0, on(1));
    e.apply_for(1, on(0));
    assert_eq!((e.kit_pad_voices(0, 1), e.kit_pad_voices(1, 0)), (1, 1));
    // Kit 1's own gate pad cuts kit 1's pad 0 (its group), not kit 0's.
    e.apply_for(1, on(1));
    assert_eq!(e.kit_pad_voices(1, 0), 0, "kit 1's group cut its pad 0");
    assert_eq!(e.kit_pad_voices(0, 1), 1, "kit 0's gate still plays");
    assert_eq!(e.kit_pad_voices(1, 1), 1);

    // Kit 1 lets go of its gate pad: only its voice releases.
    e.apply_for(1, Event::NoteOff { pad: 1 });
    let mut out = Pairs::new(2);
    for _ in 0..40 {
        out.clear();
        e.render_routed(BLOCK, &mut out);
    }
    assert_eq!(e.kit_pad_voices(1, 1), 0, "kit 1's gate ended");
    assert_eq!(e.kit_pad_voices(0, 1), 1, "kit 0's gate is held");
    assert!(out.energy(0, 1) > 1.0, "and still sounds");
    assert_eq!(out.energy(1, 1), 0.0);

    // Kit 0's lift ends its own.
    e.apply_for(0, Event::NoteOff { pad: 1 });
    for _ in 0..40 {
        out.clear();
        e.render_routed(BLOCK, &mut out);
    }
    assert_eq!(e.active_voices(), 0);
}

/// Live settings, locks and the program's offsets are a kit's own, and so
/// is its view: a new view or an edit for one kit leaves the other's
/// voices, values and revision as they were.
#[test]
fn live_settings_views_and_edits_belong_to_their_kit() {
    let (a, b) = (kit(4), kit(5));
    let mut e = Engine::with_kits(RATE, 16, 2);
    let mut va = view(&a);
    va.revision = 10;
    let mut vb = view(&b);
    vb.revision = 20;
    e.set_kit_view(0, va);
    e.set_kit_view(1, vb);
    assert_eq!((e.kit_revision(0), e.kit_revision(1)), (10, 20));
    assert_eq!(e.view_revision(), 10, "kit 0 is the engine's own");

    e.apply_for(
        1,
        Event::PadLive {
            pad: 1,
            field: PadField::Level,
            value: -30.0,
        },
    );
    e.apply_for(
        1,
        Event::PadLock {
            pad: 1,
            field: PadField::Pan,
            value: 0.5,
        },
    );
    e.apply_for(
        1,
        Event::ProgramLive {
            field: ProgramField::Tune,
            value: 200.0,
        },
    );
    assert_eq!(e.kit_pad_live_value(1, 1, PadField::Level), Some(-30.0));
    assert_eq!(e.kit_pad_lock_value(1, 1, PadField::Pan), Some(0.5));
    assert_eq!(e.kit_program_live_value(1, ProgramField::Tune), 200.0);
    assert_eq!(e.pad_live_value(1, PadField::Level), None, "kit 0's own");
    assert_eq!(e.pad_lock_value(1, PadField::Pan), None);
    assert_eq!(e.program_live_value(ProgramField::Tune), 0.0);

    // Both kits hold their gate pad; kit 1 gets another kit's view, then an
    // edit of it.
    e.apply_for(
        0,
        Event::NoteOn {
            pad: 1,
            velocity: 100,
        },
    );
    e.apply_for(
        1,
        Event::NoteOn {
            pad: 1,
            velocity: 100,
        },
    );
    let mut vc = view(&kit(6));
    vc.revision = 30;
    let old = e.set_kit_view(1, vc);
    assert_eq!(old.revision, 20);
    assert_eq!((e.kit_revision(0), e.kit_revision(1)), (10, 30));
    assert_eq!(
        e.kit_pad_live_value(1, 1, PadField::Level),
        None,
        "a new view clears its kit's live values"
    );
    assert_eq!(
        e.kit_pad_lock_value(1, 1, PadField::Pan),
        Some(0.5),
        "and keeps its locks"
    );
    let mut edited = kit(6);
    let mut pad = edited.pad(1).unwrap().clone();
    pad.level_db = -12.0;
    edited.set_pad(1, Some(pad), None).unwrap();
    let mut vd = view(&edited);
    vd.revision = 31;
    e.edit_kit_view(1, vd);
    assert_eq!((e.kit_revision(0), e.kit_revision(1)), (10, 31));
    assert_eq!((e.kit_pad_voices(0, 1), e.kit_pad_voices(1, 1)), (1, 1));

    // Out of range: nothing changes, and the view comes back.
    let mut stray = view(&a);
    stray.revision = 99;
    assert_eq!(e.set_kit_view(2, stray).revision, 99);
    assert!(!e.schedule_for(
        2,
        0,
        Event::NoteOn {
            pad: 0,
            velocity: 100
        }
    ));
    e.apply_for(
        2,
        Event::NoteOn {
            pad: 0,
            velocity: 100,
        },
    );
    assert_eq!(e.active_voices(), 2);
    assert_eq!(e.kit_revision(2), 0);
}

/// With the pool full, a new hit takes the oldest voice of the pad holding
/// the most voices, counting each kit's pads apart: kit 1's pad 0 holds
/// two, so it gives one up, though kit 0's pad 0 (the same number) is older.
#[test]
fn stealing_counts_each_kits_pads_apart() {
    let p = kit(7);
    let mut e = Engine::with_kits(RATE, 3, 2);
    e.set_kit_view(0, view(&p));
    e.set_kit_view(1, view(&p));
    let on = |pad| Event::NoteOn { pad, velocity: 100 };
    e.apply_for(0, on(0));
    e.apply_for(1, on(0));
    e.apply_for(1, on(0));
    assert_eq!(e.active_voices(), 3);
    e.apply_for(0, on(2));
    assert_eq!(e.kit_pad_voices(0, 0), 1, "kit 0's older voice kept");
    assert_eq!(e.kit_pad_voices(1, 0), 1, "kit 1's pad gave one up");
    assert_eq!(e.kit_pad_voices(0, 2), 1);
    assert_eq!(e.stats().steals, 1);
}

/// AllOff, from any kit, releases every kit's voices.
#[test]
fn all_off_reaches_every_kit() {
    let p = kit(8);
    let mut e = Engine::with_kits(RATE, 8, 3);
    for k in 0..3 {
        e.set_kit_view(k, view(&p));
        e.apply_for(
            k,
            Event::NoteOn {
                pad: 1,
                velocity: 100,
            },
        );
    }
    assert_eq!(e.active_voices(), 3);
    e.apply_for(2, Event::AllOff);
    let mut out = Pairs::new(3);
    for _ in 0..40 {
        out.clear();
        e.render_routed(BLOCK, &mut out);
    }
    assert_eq!(e.active_voices(), 0);
}

/// What each kit's voices cost is counted by kit while counting is on
/// (`Engine::count_kits`): two kits playing the same pads count the same,
/// a kit playing fewer counts less, a kit playing nothing counts nothing,
/// and nothing is counted while it is off. The sound is the same either
/// way.
#[test]
fn each_kits_voices_are_counted_by_their_cost() {
    let make = || {
        let mut e = Engine::with_kits(RATE, 16, 4);
        for k in 0..4 {
            e.set_kit_view(k, view(&kit(k as u32 + 1)));
        }
        e
    };
    let on = |pad| Event::NoteOn { pad, velocity: 100 };
    // Kits 0 and 2: pads 0 and 2; kit 1: pad 0; kit 3: none.
    let hits = [
        (0, 0, 0, on(0)),
        (0, 0, 0, on(2)),
        (0, 0, 1, on(0)),
        (0, 0, 2, on(0)),
        (0, 0, 2, on(2)),
    ];
    let mut e = make();
    let quiet = run(&mut e, 4, 2, &hits);
    let mut counts = [0u64; 4];
    e.take_kit_counts(&mut counts);
    assert_eq!(counts, [0; 4], "nothing counted while it is off");

    let mut f = make();
    f.count_kits(true);
    let counted = run(&mut f, 4, 2, &hits);
    assert_eq!(quiet, counted, "counting doesn't change the sound");
    let mut counts = [0u64; 4];
    f.take_kit_counts(&mut counts);
    assert_eq!(
        counts[0], counts[2],
        "the same pads cost the same: {counts:?}"
    );
    assert!(counts[0] > counts[1] && counts[1] > 0, "{counts:?}");
    assert_eq!(counts[3], 0);
    // A take starts again.
    let mut again = [0u64; 4];
    f.take_kit_counts(&mut again);
    assert_eq!(again, [0; 4]);
}
