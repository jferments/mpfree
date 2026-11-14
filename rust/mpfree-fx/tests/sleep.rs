//! Sleep on silence (`chain.rs`): every effect sounds the same whether its
//! slot may sleep or not, sleeps where it says it may, and gives the same
//! output for any block size while it does.

use mpfree_fx::{ids, Chain, Compressor, Drive, Eq, EFFECTS};
use mpfree_node::{Ctx, NodeEvent, Timed, MAX_BLOCK};

const RATE: u32 = 48_000;

/// Bursts of a tone with noise, each followed by a longer silence, with a
/// parameter change in the middle of one silence.
fn program() -> (Vec<f32>, Vec<f32>) {
    let mut seed = 7u32;
    let mut noise = move || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 9) as f32 / (1u32 << 23) as f32 - 0.5
    };
    let (mut l, mut r) = (Vec::new(), Vec::new());
    for (burst_s, gap_s) in [(0.2, 0.3), (0.15, 1.0), (0.3, 2.5), (0.1, 0.6)] {
        let n = (burst_s * RATE as f32) as usize;
        for i in 0..n {
            let t = i as f32 / RATE as f32;
            let tone = 0.4 * (std::f32::consts::TAU * 220.0 * t).sin();
            l.push(tone + 0.1 * noise());
            r.push(0.8 * tone + 0.1 * noise());
        }
        let gap = (gap_s * RATE as f32) as usize;
        l.resize(l.len() + gap, 0.0);
        r.resize(r.len() + gap, 0.0);
    }
    (l, r)
}

/// The frame of the parameter change: in the long silence.
fn change_at() -> usize {
    ((0.2 + 0.3 + 0.15 + 1.0 + 0.3 + 2.0) * RATE as f32) as usize
}

struct Heard {
    out: (Vec<f32>, Vec<f32>),
    /// Frames played while the slot slept.
    slept: usize,
}

fn hear(id: &str, sleep: bool, next_block: &mut dyn FnMut() -> usize) -> Heard {
    hear_with(id, &[], sleep, next_block)
}

/// [`hear`] with some of the effect's parameters set first.
fn hear_with(
    id: &str,
    set: &[(u16, f32)],
    sleep: bool,
    next_block: &mut dyn FnMut() -> usize,
) -> Heard {
    hear_changing(id, set, None, sleep, next_block)
}

/// [`hear_with`], changing parameter `change` in the long silence (the
/// first parameter when `None`).
fn hear_changing(
    id: &str,
    set: &[(u16, f32)],
    change: Option<u16>,
    sleep: bool,
    next_block: &mut dyn FnMut() -> usize,
) -> Heard {
    let (l, r) = program();
    let mut c = Chain::with_capacity(2);
    c.prepare(RATE, MAX_BLOCK);
    let mut node = mpfree_fx::registry::make(id).unwrap();
    for &(p, v) in set {
        node.set_param(p, v);
    }
    node.reset();
    let info = match change {
        Some(c) => node.params().iter().find(|p| p.id == c).copied(),
        None => node.params().first().copied(),
    };
    c.push(node);
    c.set_sleep(sleep);
    let (mut ol, mut or) = (l.clone(), r.clone());
    let mut slept = 0;
    let mut at = 0;
    let change = change_at();
    while at < l.len() {
        let len = next_block().clamp(1, MAX_BLOCK).min(l.len() - at);
        if let Some(p) = info {
            if change >= at && change < at + len {
                let value = p.from_unit(0.6);
                assert!(c.queue(
                    0,
                    Timed::new((change - at) as u32, NodeEvent::Param { id: p.id, value })
                ));
            }
        }
        let ctx = Ctx::new(RATE, at as u64);
        c.process(&ctx, &mut ol[at..at + len], &mut or[at..at + len]);
        slept += c.sleeping() * len;
        at += len;
    }
    Heard {
        out: (ol, or),
        slept,
    }
}

fn worst(a: &(Vec<f32>, Vec<f32>), b: &(Vec<f32>, Vec<f32>)) -> f32 {
    a.0.iter()
        .zip(&b.0)
        .chain(a.1.iter().zip(&b.1))
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max)
}

/// The effects whose slot sleeps at their defaults within these silences.
/// (The transient shaper and Lo-fi sleep too, after longer silences: the
/// shaper's slowest follower carries over some 10 s at its default speed,
/// and Lo-fi's crackle and hiss fade for 4 s after the music; Delay sleeps
/// once its tail is 100 dB down, Cloud, Plate and Room too without
/// modulation, and Spring (below). Crush sleeps only with its rate at
/// full, where no held sample falls anywhere, and the Gate only on its own
/// signal, not a key.)
const SLEEPERS: &[&str] = &[
    "mpfree.fx.compressor",
    "mpfree.fx.limiter",
    "mpfree.fx.gate",
    "mpfree.fx.deesser",
    "mpfree.fx.multiband",
    "mpfree.fx.eq",
    "mpfree.fx.filter",
    "mpfree.fx.dj_filter",
    "mpfree.fx.comb",
    "mpfree.fx.drive",
    "mpfree.fx.character",
    "mpfree.fx.amp",
    "mpfree.fx.exciter",
];

#[test]
fn every_effect_sounds_the_same_asleep_and_sleeps_where_it_may() {
    let mut report = Vec::new();
    let mut slept = Vec::new();
    for e in EFFECTS {
        let awake = hear(e.id, false, &mut || 256);
        let asleep = hear(e.id, true, &mut || 256);
        let diff = worst(&awake.out, &asleep.out);
        let total = awake.out.0.len();
        report.push(format!(
            "{}: worst difference {:.1e} ({:.0} dB), asleep {:.0}% of the time",
            e.id,
            diff,
            20.0 * diff.max(1e-12).log10(),
            100.0 * asleep.slept as f64 / total as f64
        ));
        assert_eq!(awake.slept, 0, "{}: sleep off never sleeps", e.id);
        // Under -80 dB: a difference nobody can hear, even soloed.
        assert!(diff <= 1e-4, "{}", report.join("\n"));
        if asleep.slept > 0 {
            slept.push(e.id);
        }
    }
    println!("{}", report.join("\n"));
    assert_eq!(slept, SLEEPERS, "{}", report.join("\n"));
}

#[test]
fn sleeping_effects_give_the_same_output_for_any_block_size() {
    for id in SLEEPERS {
        let reference = hear(id, true, &mut || 512).out;
        for size in [7, 64, 333] {
            let got = hear(id, true, &mut || size).out;
            assert_eq!(worst(&got, &reference), 0.0, "{id}: {size}-frame blocks");
        }
        let mut seed = 3u32;
        let got = hear(id, true, &mut || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            1 + (seed >> 8) as usize % 512
        })
        .out;
        assert_eq!(worst(&got, &reference), 0.0, "{id}: random blocks");
    }
}

/// Delay, Cloud and the reverbs sleep once their tails are 100 dB down:
/// here a delay with no feedback and short rooms. Cloud is half wet (all
/// wet, it could lead a late return); Cloud, Plate and Room are without
/// modulation (which keeps time across silence), the only ones of them
/// that sleep. The reverbs' first parameters (pre-delay, spring length)
/// glide for over a second, longer than the silence after the change, and
/// a slot that sleeps through a change wakes with the glide done; so for
/// them the change is to Low cut, which takes effect at once.
#[test]
fn delay_cloud_and_reverbs_sleep_once_their_tails_are_quiet() {
    use mpfree_fx::ids::{cloud, delay, plate, room, spring};
    type Case<'a> = (&'a str, &'a [(u16, f32)], Option<u16>);
    let short: [Case; 5] = [
        ("mpfree.fx.delay", &[(delay::FEEDBACK, 0.0)], None),
        (
            "mpfree.fx.cloud",
            &[
                (cloud::DECAY, 300.0),
                (cloud::MIX, 50.0),
                (cloud::MODULATION, 0.0),
            ],
            None,
        ),
        (
            "mpfree.fx.plate",
            &[(plate::DECAY, 300.0), (plate::MOD, 0.0)],
            Some(plate::LOW_CUT),
        ),
        (
            "mpfree.fx.room",
            &[(room::DECAY, 300.0), (room::MOD, 0.0)],
            Some(room::LOW_CUT),
        ),
        (
            "mpfree.fx.spring",
            &[(spring::DECAY, 600.0)],
            Some(spring::LOW_CUT),
        ),
    ];
    for (id, set, change) in short {
        let awake = hear_changing(id, set, change, false, &mut || 256);
        let asleep = hear_changing(id, set, change, true, &mut || 256);
        assert!(asleep.slept > 0, "{id} sleeps");
        let diff = worst(&awake.out, &asleep.out);
        assert!(diff <= 1e-4, "{id}: {diff:e}");
        let reference = hear_changing(id, set, change, true, &mut || 512).out;
        let got = hear_changing(id, set, change, true, &mut || 333).out;
        assert_eq!(worst(&got, &reference), 0.0, "{id}: 333-frame blocks");
    }
}

/// Sound and silence of every length, some of it just over or under
/// silence (-140 dB), through effects that sleep quickly: the output is
/// the same for any block size, so every decision falls on a frame.
#[test]
fn sleep_decisions_fall_on_frames_not_blocks() {
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            self.0 >> 33
        }
    }
    let n = 200_000;
    let mut rng = Lcg(9);
    let (mut l, mut r) = (Vec::new(), Vec::new());
    while l.len() < n {
        let len = 1 + rng.next() as usize % 3000;
        let amp = [0.0, 0.5, 1e-6, 1e-8][rng.next() as usize % 4];
        for _ in 0..len {
            let x = amp * ((rng.next() % 1000) as f32 / 500.0 - 1.0);
            l.push(x);
            r.push(0.7 * x);
        }
    }
    let play = |make: &dyn Fn(&mut Chain), next: &mut dyn FnMut() -> usize| {
        let mut c = Chain::with_capacity(2);
        c.prepare(RATE, MAX_BLOCK);
        make(&mut c);
        let (mut ol, mut or) = (l[..n].to_vec(), r[..n].to_vec());
        let mut at = 0;
        while at < n {
            let len = next().clamp(1, MAX_BLOCK).min(n - at);
            c.process(
                &Ctx::new(RATE, at as u64),
                &mut ol[at..at + len],
                &mut or[at..at + len],
            );
            at += len;
        }
        (ol, or)
    };
    type Build<'a> = &'a dyn Fn(&mut Chain);
    let chains: [(&str, Build); 3] = [
        ("EQ", &|c: &mut Chain| {
            c.push(Box::new(Eq::new()));
            c.set_param(0, ids::eq::gain(1), 4.0);
        }),
        ("Compressor", &|c: &mut Chain| {
            c.push(Box::new(Compressor::new()));
        }),
        ("Drive", &|c: &mut Chain| {
            c.push(Box::new(Drive::new()));
        }),
    ];
    for (name, make) in chains {
        let reference = play(make, &mut || 128);
        let mut rng = Lcg(3);
        let got = play(make, &mut || 1 + rng.next() as usize % 512);
        assert_eq!(worst(&got, &reference), 0.0, "{name}");
    }
}
