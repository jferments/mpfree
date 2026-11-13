//! Behaviour tests for every mpfree-fx node and the insert chain: the
//! node rules (valid descriptions, the same output for any block size,
//! finite output at every extreme), each effect doing what it says, the
//! Master safety ceiling, drive aliasing, and click-free bypass with tails
//! that keep ringing.

use std::f32::consts::PI;

use mpfree_fx::mpfree_node::{
    check_params, Ctx, Node, NodeEvent, NodeInfo, ParamInfo, Timed, MAX_BLOCK,
};
use mpfree_fx::*;

const RATE: u32 = 48_000;

/// A small deterministic random source.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
    fn unit(&mut self) -> f32 {
        (self.next() % 1_000_000) as f32 / 1_000_000.0
    }
    fn range(&mut self, lo: usize, hi: usize) -> usize {
        lo + (self.next() as usize) % (hi - lo + 1)
    }
}

const NAMES: [&str; 18] = [
    "filter",
    "dj filter",
    "drive",
    "crush",
    "delay",
    "compressor",
    "master safety",
    "cloud reverb",
    "eq",
    "chorus",
    "flanger",
    "phaser",
    "lo-fi",
    "comb",
    "character",
    "ducker",
    "transient shaper",
    "pitch shifter",
];

fn make(i: usize) -> Box<dyn Node> {
    match i {
        0 => Box::new(MultiFilter::new()),
        1 => Box::new(DjFilter::new()),
        2 => Box::new(Drive::new()),
        3 => Box::new(Crush::new()),
        4 => Box::new(Delay::new()),
        5 => Box::new(Compressor::new()),
        6 => Box::new(MasterSafety::new()),
        7 => Box::new(Cloud::new()),
        8 => Box::new(Eq::new()),
        9 => Box::new(Chorus::new()),
        10 => Box::new(Flanger::new()),
        11 => Box::new(Phaser::new()),
        12 => Box::new(LoFi::new()),
        13 => Box::new(Comb::new()),
        14 => Box::new(Character::new()),
        15 => Box::new(Ducker::new()),
        16 => Box::new(TransientShaper::new()),
        _ => Box::new(PitchShifter::new()),
    }
}

/// A busy stereo test signal in four 250 ms sections (quiet, silent, full,
/// and hot at about +8 dBFS): detuned saws, noise and a thump.
fn program(frames: usize) -> (Vec<f32>, Vec<f32>) {
    let mut rng = Lcg(7);
    let mut l = vec![0.0; frames];
    let mut r = vec![0.0; frames];
    for i in 0..frames {
        let t = i as f32 / RATE as f32;
        let saw = |hz: f32| 2.0 * (t * hz).fract() - 1.0;
        let gain = [0.3, 0.0, 1.0, 2.5][(i / 12_000) % 4];
        let noise = rng.unit() * 2.0 - 1.0;
        let b = (i % 24_000) as f32 / RATE as f32;
        let thump = (2.0 * PI * 55.0 * b).sin() * (-b * 8.0).exp();
        l[i] = gain * (0.5 * saw(110.0) + 0.2 * noise) + 0.6 * thump;
        r[i] = gain * (0.5 * saw(110.7) - 0.2 * noise) + 0.6 * thump;
    }
    (l, r)
}

/// Parameter moves at absolute frames: four random values across the range
/// of every writable parameter.
fn automation(
    params: &[ParamInfo],
    frames: usize,
    seed: u64,
    skip: &[u16],
) -> Vec<(usize, u16, f32)> {
    let mut rng = Lcg(seed);
    let mut moves = Vec::new();
    for p in params
        .iter()
        .filter(|p| !p.readonly && !skip.contains(&p.id))
    {
        for _ in 0..4 {
            moves.push((
                rng.range(0, frames - 1),
                p.id,
                p.min + (p.max - p.min) * rng.unit(),
            ));
        }
    }
    moves.sort_by_key(|m| m.0);
    moves
}

/// Runs `node` over `input` in blocks of the sizes `next_block` gives, with
/// `moves` as sample-timed events.
fn render(
    node: &mut dyn Node,
    input: &(Vec<f32>, Vec<f32>),
    next_block: &mut dyn FnMut() -> usize,
    moves: &[(usize, u16, f32)],
) -> (Vec<f32>, Vec<f32>) {
    node.prepare(RATE, MAX_BLOCK);
    node.reset();
    let (mut l, mut r) = input.clone();
    let n = l.len();
    let (mut at, mut k) = (0, 0);
    let mut events = Vec::new();
    while at < n {
        let len = next_block().clamp(1, MAX_BLOCK).min(n - at);
        events.clear();
        while k < moves.len() && moves[k].0 < at + len {
            events.push(Timed::new(
                (moves[k].0 - at) as u32,
                NodeEvent::Param {
                    id: moves[k].1,
                    value: moves[k].2,
                },
            ));
            k += 1;
        }
        node.process(
            &Ctx::new(RATE, at as u64),
            &events,
            &mut l[at..at + len],
            &mut r[at..at + len],
        );
        at += len;
    }
    (l, r)
}

fn assert_identical(name: &str, how: &str, a: &(Vec<f32>, Vec<f32>), b: &(Vec<f32>, Vec<f32>)) {
    for (ch, (x, y)) in [(&a.0, &b.0), (&a.1, &b.1)].into_iter().enumerate() {
        if let Some(i) = (0..x.len()).find(|&i| x[i].to_bits() != y[i].to_bits()) {
            panic!(
                "{name}, {how}: channel {ch} differs first at frame {i}: {} vs {}",
                x[i], y[i]
            );
        }
    }
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| (v * v) as f64).sum::<f64>() / x.len().max(1) as f64).sqrt() as f32
}

fn sine(hz: f32, amp: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| amp * (2.0 * PI * hz * i as f32 / RATE as f32).sin())
        .collect()
}

/// One node, its parameters set, prepared and reset (so smoothing is done).
fn set_up(mut node: Box<dyn Node>, values: &[(u16, f32)]) -> Box<dyn Node> {
    node.prepare(RATE, MAX_BLOCK);
    for &(id, v) in values {
        node.set_param(id, v);
    }
    node.reset();
    node
}

/// Ten times an 8.0 impulse at the EQ's loudest point for these settings.
fn eq_bound(values: &[(u16, f32)]) -> f32 {
    let mut eq = Eq::new();
    for &(id, v) in values {
        eq.set_param(id, v);
    }
    let hz: Vec<f32> = (0..2000)
        .map(|i| 10.0 * 2300f32.powf(i as f32 / 1999.0))
        .collect();
    let mut db = vec![0.0; hz.len()];
    eq.response_db(&hz, &mut db);
    80.0 * db_to_gain(db.iter().fold(0.0f32, |m, &d| m.max(d))).max(12.5)
}

fn run_once(node: &mut dyn Node, l: &mut [f32], r: &mut [f32]) {
    for (i, (a, b)) in l.chunks_mut(256).zip(r.chunks_mut(256)).enumerate() {
        node.process(&Ctx::new(RATE, (i * 256) as u64), &[], a, b);
    }
}

#[test]
fn descriptions_are_valid() {
    for (i, name) in NAMES.iter().enumerate() {
        let node = make(i);
        check_params(node.params()).unwrap_or_else(|e| panic!("{name}: {e}"));
        for (k, p) in node.params().iter().enumerate() {
            assert_eq!(p.id as usize, k, "{name}: id is the index");
            assert_eq!(
                node.param(p.id),
                p.default,
                "{name}: {} starts at its default",
                p.name
            );
        }
        assert!(node.param(999).is_nan());
        assert_eq!(node.info().kind, mpfree_fx::mpfree_node::Kind::Effect);
        assert!(node.info().id.starts_with("mpfree.fx."));
    }
}

#[test]
fn same_output_for_any_block_size() {
    let frames = 48_000;
    let input = program(frames);
    for (i, name) in NAMES.iter().enumerate() {
        let moves = automation(make(i).params(), frames, 11 + i as u64, &[]);
        let reference = render(make(i).as_mut(), &input, &mut || 4096, &moves);
        for size in [1usize, 7, 128, 1000, MAX_BLOCK] {
            let out = render(make(i).as_mut(), &input, &mut || size, &moves);
            assert_identical(name, &format!("{size}-frame blocks"), &reference, &out);
        }
        let mut rng = Lcg(99);
        let out = render(make(i).as_mut(), &input, &mut || rng.range(1, 700), &moves);
        assert_identical(name, "random blocks", &reference, &out);
    }
}

#[test]
fn every_extreme_stays_finite_and_bounded() {
    let frames = 24_000;
    // Hot noise, DC, full-scale impulses and silence.
    let mut rng = Lcg(3);
    let hot: Vec<f32> = (0..frames)
        .map(|i| match (i / 3000) % 4 {
            0 => 4.0 * (rng.unit() * 2.0 - 1.0),
            1 => 2.0,
            2 if i % 997 == 0 => 8.0,
            _ => 0.0,
        })
        .collect();
    for (i, name) in NAMES.iter().enumerate() {
        let params = make(i).params();
        let writable: Vec<&ParamInfo> = params.iter().filter(|p| !p.readonly).collect();
        // Every parameter alone at each end, then all at once at each end.
        let mut cases: Vec<Vec<(u16, f32)>> = Vec::new();
        for p in &writable {
            cases.push(vec![(p.id, p.min)]);
            cases.push(vec![(p.id, p.max)]);
        }
        cases.push(writable.iter().map(|p| (p.id, p.min)).collect());
        cases.push(writable.iter().map(|p| (p.id, p.max)).collect());
        for case in cases {
            let mut node = set_up(make(i), &case);
            let (mut l, mut r) = (hot.clone(), hot.iter().map(|v| -v).collect::<Vec<_>>());
            run_once(node.as_mut(), &mut l, &mut r);
            let p = peak(&l).max(peak(&r));
            assert!(
                l.iter().chain(&r).all(|v| v.is_finite()),
                "{name} {case:?}: non-finite output"
            );
            // Generous: the loudest legitimate case is +24 dB of drive output
            // gain or filter gain on an 8.0 impulse. The EQ can stack four
            // bands and its output gain on one frequency, so its bound
            // follows its own curve.
            let bound = if *name == "eq" {
                eq_bound(&case)
            } else {
                1000.0
            };
            assert!(p < bound, "{name} {case:?}: peak {p}, bound {bound}");
        }
    }
}

#[test]
fn delay_at_full_feedback_is_bounded_by_its_saturator() {
    let mut d = set_up(
        Box::new(Delay::new()),
        &[
            (ids::delay::SYNC, 0.0),
            (ids::delay::TIME, 20.0),
            (ids::delay::FEEDBACK, 110.0),
            (ids::delay::MIX, 100.0),
            (ids::delay::SATURATION, 0.0),
        ],
    );
    let frames = RATE as usize * 10;
    let mut l = vec![0.0f32; frames];
    l[..4800].copy_from_slice(&sine(200.0, 1.0, 4800));
    let mut r = l.clone();
    run_once(d.as_mut(), &mut l, &mut r);
    let last = &l[frames - 48_000..];
    // It keeps going (feedback over 100 %) but never runs away.
    assert!(rms(last) > 0.05, "self-oscillation holds: {}", rms(last));
    assert!(peak(&l) < 2.5, "bounded: {}", peak(&l));
}

#[test]
fn filters_have_their_slopes_and_the_dj_filter_is_transparent_in_the_middle() {
    let gain_at = |node: &mut Box<dyn Node>, hz: f32| {
        node.reset();
        let x = sine(hz, 0.25, RATE as usize);
        let (mut l, mut r) = (x.clone(), x.clone());
        run_once(node.as_mut(), &mut l, &mut r);
        gain_to_db(rms(&l[24_000..]) / rms(&x[24_000..]))
    };
    use ids::filter::*;
    let mut lp24 = set_up(
        Box::new(MultiFilter::new()),
        &[(TYPE, 0.0), (SLOPE, 1.0), (CUTOFF, 1000.0)],
    );
    assert!(gain_at(&mut lp24, 100.0).abs() < 0.2);
    assert!(
        (gain_at(&mut lp24, 1000.0) + 3.0).abs() < 0.3,
        "-3 dB at the cutoff"
    );
    let g = gain_at(&mut lp24, 8000.0);
    assert!(g < -66.0 && g > -78.0, "24 dB/oct, 3 octaves: {g}");
    let mut hp12 = set_up(
        Box::new(MultiFilter::new()),
        &[(TYPE, 1.0), (SLOPE, 0.0), (CUTOFF, 1000.0)],
    );
    let g = gain_at(&mut hp12, 125.0);
    assert!(g < -33.0 && g > -39.0, "12 dB/oct, 3 octaves: {g}");
    assert!(gain_at(&mut hp12, 10_000.0).abs() < 0.2);
    let mut bell = set_up(
        Box::new(MultiFilter::new()),
        &[(TYPE, 4.0), (CUTOFF, 2000.0), (GAIN, -12.0)],
    );
    assert!((gain_at(&mut bell, 2000.0) + 12.0).abs() < 0.3);

    let mut dj = set_up(Box::new(DjFilter::new()), &[]);
    let x = program(24_000);
    let (mut l, mut r) = x.clone();
    run_once(dj.as_mut(), &mut l, &mut r);
    assert!(l == x.0 && r == x.1, "the middle is bit-for-bit off");
    let mut dj = set_up(
        Box::new(DjFilter::new()),
        &[(ids::dj_filter::POSITION, -100.0)],
    );
    assert!(
        gain_at(&mut dj, 2000.0) < -40.0,
        "far left is a low-pass near 80 Hz"
    );
    let mut dj = set_up(
        Box::new(DjFilter::new()),
        &[(ids::dj_filter::POSITION, 100.0)],
    );
    assert!(
        gain_at(&mut dj, 500.0) < -30.0,
        "far right is a high-pass near 8 kHz"
    );
}

#[test]
fn drive_latency_is_whole_and_the_same_in_both_qualities() {
    use ids::drive::*;
    for quality in [0.0, 1.0] {
        let mut d = set_up(
            Box::new(Drive::new()),
            &[(DRIVE, 0.0), (BIAS, 0.0), (QUALITY, quality)],
        );
        assert_eq!(d.latency(), 55);
        let mut l = vec![0.0f32; 400];
        l[100] = 0.01;
        let mut r = l.clone();
        run_once(d.as_mut(), &mut l, &mut r);
        let at = (0..l.len())
            .max_by(|&a, &b| l[a].abs().total_cmp(&l[b].abs()))
            .unwrap();
        assert_eq!(
            at,
            155,
            "quality {quality}: the impulse peaks {} frames late",
            at - 100
        );
    }
    // Eco and HQ line up to well under a frame: a half-frame offset would
    // leave about 6 % of a 1 kHz sine in the difference.
    let x = sine(1000.0, 0.1, 9600);
    let mut outs = Vec::new();
    for quality in [0.0, 1.0] {
        let mut d = set_up(
            Box::new(Drive::new()),
            &[(DRIVE, 0.0), (BIAS, 0.0), (QUALITY, quality)],
        );
        let (mut l, mut r) = (x.clone(), x.clone());
        run_once(d.as_mut(), &mut l, &mut r);
        outs.push(l);
    }
    let diff: Vec<f32> = (4800..9600).map(|i| outs[0][i] - outs[1][i]).collect();
    assert!(
        rms(&diff) < 0.005 * rms(&outs[0][4800..]),
        "Eco and HQ differ by {}",
        rms(&diff) / rms(&outs[0][4800..])
    );
    // The dry side of the mix is delayed to match: Mix 0 is the input, late.
    let mut d = set_up(Box::new(Drive::new()), &[(MIX, 0.0)]);
    let x = program(4800);
    let (mut l, mut r) = x.clone();
    run_once(d.as_mut(), &mut l, &mut r);
    assert!((55..4800).all(|i| l[i] == x.0[i - 55] && r[i] == x.1[i - 55]));
}

/// The largest alias of a sine in the audible band, in dB against the
/// fundamental, by DFT at every folded harmonic that is not a real one.
fn worst_alias(y: &[f32], f0: usize) -> f32 {
    let n = y.len();
    assert_eq!(n, RATE as usize, "1 Hz bins");
    let amp = |hz: usize| {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, &v) in y.iter().enumerate() {
            let ph = 2.0 * std::f64::consts::PI * (hz * i % n) as f64 / n as f64;
            re += v as f64 * ph.cos();
            im -= v as f64 * ph.sin();
        }
        ((re * re + im * im).sqrt() * 2.0 / n as f64) as f32
    };
    let fund = amp(f0);
    let nyq = RATE as usize / 2;
    let mut worst = 0.0f32;
    for k in 2..200 {
        let h = k * f0;
        if h < nyq {
            continue;
        }
        let m = h % RATE as usize;
        let folded = if m > nyq { RATE as usize - m } else { m };
        if (20..=20_000).contains(&folded) && folded % f0 != 0 {
            worst = worst.max(amp(folded));
        }
    }
    gain_to_db(worst / fund)
}

#[test]
fn oversampling_keeps_aliasing_down() {
    use ids::drive::*;
    let f0 = 5000;
    let x = sine(f0 as f32, 0.5, 2 * RATE as usize);
    // A plain tanh at 1x, for comparison.
    let g = db_to_gain(18.0);
    let naive: Vec<f32> = x.iter().map(|v| (v * g).tanh()).collect();
    let naive_db = worst_alias(&naive[RATE as usize..], f0);
    let mut results = Vec::new();
    for quality in [0.0, 1.0] {
        let mut d = set_up(
            Box::new(Drive::new()),
            &[(MODE, 0.0), (DRIVE, 18.0), (BIAS, 0.0), (QUALITY, quality)],
        );
        let (mut l, mut r) = (x.clone(), x.clone());
        run_once(d.as_mut(), &mut l, &mut r);
        results.push(worst_alias(&l[RATE as usize..], f0));
    }
    println!("worst alias of a 5 kHz sine through tanh at +18 dB: 1x {naive_db:.1} dB, Eco 2x {:.1} dB, HQ 4x {:.1} dB", results[0], results[1]);
    assert!(
        results[0] < naive_db - 12.0,
        "Eco beats 1x by 12 dB or more"
    );
    assert!(
        results[1] < results[0] - 12.0,
        "HQ beats Eco by 12 dB or more"
    );
    assert!(results[1] < -70.0, "HQ: {}", results[1]);
}

#[test]
fn crush_quantises_and_holds() {
    use ids::crush::*;
    let mut c = set_up(Box::new(Crush::new()), &[(BITS, 3.0)]);
    let x = sine(100.0, 0.9, 4800);
    let (mut l, mut r) = (x.clone(), x.clone());
    run_once(c.as_mut(), &mut l, &mut r);
    // 3 bits: levels at multiples of 1/4.
    assert!(
        l.iter().all(|v| (v * 4.0 - (v * 4.0).round()).abs() < 1e-6),
        "on the 3-bit grid"
    );
    // 4.8 kHz: each value held about 10 frames (the phase is a float, so
    // a hold can be one frame longer or shorter).
    let mut c = set_up(Box::new(Crush::new()), &[(BITS, 16.0), (RATE, 4800.0)]);
    let mut rng = Lcg(1);
    let x: Vec<f32> = (0..4800).map(|_| rng.unit() - 0.5).collect();
    let (mut l, mut r) = (x.clone(), x.clone());
    run_once(c.as_mut(), &mut l, &mut r);
    let mut runs = vec![1usize];
    for i in 1..l.len() {
        if l[i] == l[i - 1] {
            *runs.last_mut().unwrap() += 1;
        } else {
            runs.push(1);
        }
    }
    assert!((475..=481).contains(&runs.len()), "{} holds", runs.len());
    assert!(
        runs[1..runs.len() - 1]
            .iter()
            .all(|&n| (9..=11).contains(&n)),
        "{runs:?}"
    );
}

#[test]
fn synced_delay_lands_on_the_grid_and_follows_the_tempo() {
    use ids::delay::*;
    let mut d = set_up(
        Box::new(Delay::new()),
        &[
            (SYNC, 6.0),
            (FEEDBACK, 0.0),
            (MIX, 100.0),
            (TONE, 20_000.0),
            (SATURATION, 0.0),
        ],
    );
    // 1/8 at 120 BPM is 0.25 s.
    let mut l = vec![0.0f32; 30_000];
    l[0] = 0.5;
    let mut r = l.clone();
    run_once(d.as_mut(), &mut l, &mut r);
    let at = (0..l.len())
        .max_by(|&a, &b| l[a].abs().total_cmp(&l[b].abs()))
        .unwrap();
    assert_eq!(at, 12_000);
    // At 90 BPM it moves to 1/3 s (after a short tape-style glide).
    let mut l = vec![0.0f32; 96_000];
    l[48_000] = 0.5;
    let mut r = l.clone();
    for (i, (a, b)) in l.chunks_mut(256).zip(r.chunks_mut(256)).enumerate() {
        let ctx = Ctx {
            bpm: 90.0,
            ..Ctx::new(RATE, (i * 256) as u64)
        };
        d.process(&ctx, &[], a, b);
    }
    let at = (0..l.len())
        .max_by(|&a, &b| l[a].abs().total_cmp(&l[b].abs()))
        .unwrap();
    assert_eq!(at, 48_000 + 16_000);
}

#[test]
fn ping_pong_alternates() {
    use ids::delay::*;
    let mut d = set_up(
        Box::new(Delay::new()),
        &[
            (SYNC, 0.0),
            (TIME, 100.0),
            (FEEDBACK, 50.0),
            (MIX, 100.0),
            (PING_PONG, 1.0),
            (TONE, 20_000.0),
            (SATURATION, 0.0),
        ],
    );
    let mut l = vec![0.0f32; 24_000];
    let mut r = vec![0.0f32; 24_000];
    l[0] = 0.5;
    r[0] = 0.5;
    run_once(d.as_mut(), &mut l, &mut r);
    let e = |x: &[f32], k: usize| peak(&x[k * 4800 - 100..k * 4800 + 100]);
    assert!(e(&l, 1) > 0.3 && e(&r, 1) < 0.01, "first repeat left");
    assert!(e(&r, 2) > 0.1 && e(&l, 2) < 0.01, "second repeat right");
    assert!(e(&l, 3) > 0.05 && e(&r, 3) < 0.01, "third repeat left");
}

#[test]
fn compressor_reaches_its_static_curve_and_ducks_from_a_sidechain() {
    use ids::compressor::*;
    // A Nyquist square has a constant level: -6.02 dBFS.
    let mut c = set_up(
        Box::new(Compressor::new()),
        &[(THRESHOLD, -18.0), (RATIO, 4.0), (KNEE, 0.0), (ATTACK, 1.0)],
    );
    let x: Vec<f32> = (0..24_000)
        .map(|i| if i % 2 == 0 { 0.5 } else { -0.5 })
        .collect();
    let (mut l, mut r) = (x.clone(), x.clone());
    run_once(c.as_mut(), &mut l, &mut r);
    let out_db = gain_to_db(l[23_999].abs());
    let want = -18.0 + (gain_to_db(0.5) + 18.0) / 4.0;
    assert!((out_db - want).abs() < 0.05, "{out_db} vs {want}");
    assert!((c.param(REDUCTION) - (want - gain_to_db(0.5))).abs() < 0.05);

    // Sidechain: a quiet signal ducks under a loud key.
    let mut c = set_up(
        Box::new(Compressor::new()),
        &[(THRESHOLD, -30.0), (RATIO, 10.0), (SIDECHAIN, 1.0)],
    );
    let key = vec![0.9f32; 256];
    let mut l = vec![0.05f32; 256];
    let mut r = vec![0.05f32; 256];
    for i in 0..40 {
        let ctx = Ctx {
            sidechain: Some((&key, &key)),
            ..Ctx::new(RATE, i * 256)
        };
        l.fill(0.05);
        r.fill(0.05);
        c.process(&ctx, &[], &mut l, &mut r);
    }
    assert!(
        gain_to_db(l[255] / 0.05) < -20.0,
        "ducked: {} dB",
        gain_to_db(l[255] / 0.05)
    );
}

#[test]
fn master_safety_never_passes_the_ceiling_in_safe_mode() {
    use ids::master::*;
    for rate in [44_100u32, 48_000, 96_000] {
        for (ceiling, trim) in [(-0.3f32, 0.0f32), (-0.3, 12.0), (-6.0, 6.0), (0.0, 0.0)] {
            let mut m = MasterSafety::new();
            m.prepare(rate, MAX_BLOCK);
            m.set_param(CEILING, ceiling);
            m.set_param(TRIM, trim);
            m.reset();
            let (mut l, mut r) = program(48_000);
            // Add the worst case: a full-scale step out of silence.
            for v in &mut l[30_000..30_200] {
                *v = 6.0;
            }
            for (i, (a, b)) in l.chunks_mut(97).zip(r.chunks_mut(97)).enumerate() {
                m.process(&Ctx::new(rate, (i * 97) as u64), &[], a, b);
            }
            let ceil = db_to_gain(ceiling);
            let p = peak(&l).max(peak(&r));
            assert!(
                p <= ceil,
                "rate {rate}, ceiling {ceiling}, trim {trim}: peak {p} over {ceil}"
            );
            let s = m.stats();
            assert!(s.overs > 0 && s.clipped == 0, "{s:?}");
            assert!(s.peak_out_db <= ceiling + 1e-4);
        }
    }
}

#[test]
fn master_safety_is_transparent_below_the_ceiling_and_raw_is_raw() {
    use ids::master::*;
    let mut m = set_up(Box::new(MasterSafety::new()), &[]);
    let x = sine(1000.0, 0.5, 9600);
    let (mut l, mut r) = (x.clone(), x.clone());
    run_once(m.as_mut(), &mut l, &mut r);
    let lat = m.latency() as usize;
    assert_eq!(lat, 72);
    // Only the 5 Hz DC blocker touches it (a 0.3 degree phase lead at 1 kHz).
    assert!((gain_to_db(rms(&l[4800..]) / rms(&x[4800..]))).abs() < 0.01);
    assert!((4800..9600).all(|i| (l[i] - x[i - lat]).abs() < 4e-3));

    let mut m = set_up(Box::new(MasterSafety::new()), &[(MODE, 1.0)]);
    let x = sine(100.0, 3.0, 9600);
    let (mut l, mut r) = (x.clone(), x.clone());
    run_once(m.as_mut(), &mut l, &mut r);
    assert!(
        peak(&l) <= db_to_gain(-0.3),
        "soft clip stays under the ceiling"
    );

    let mut raw = MasterSafety::new();
    raw.prepare(RATE, MAX_BLOCK);
    raw.set_param(MODE, 2.0);
    raw.reset();
    let (mut l, mut r) = (x.clone(), x.clone());
    run_once(&mut raw, &mut l, &mut r);
    assert!(peak(&l) > 2.5, "raw lets it through");
    assert!(raw.stats().clipped > 0 && raw.stats().overs > 0);
    raw.clear_stats();
    assert_eq!(raw.stats().clipped, 0);
}

/// Runs a chain block by block, cutting blocks at the bypass switches.
fn run_chain(
    chain: &mut Chain,
    input: &(Vec<f32>, Vec<f32>),
    next_block: &mut dyn FnMut() -> usize,
    switches: &[(usize, usize, bool)],
    moves: &[(usize, usize, u16, f32)],
) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = input.clone();
    let n = l.len();
    let (mut at, mut s, mut k) = (0, 0, 0);
    while at < n {
        while s < switches.len() && switches[s].0 <= at {
            chain.set_on(switches[s].1, switches[s].2);
            s += 1;
        }
        let mut len = next_block().clamp(1, MAX_BLOCK).min(n - at);
        if s < switches.len() {
            len = len.min(switches[s].0 - at);
        }
        while k < moves.len() && moves[k].0 < at + len {
            let (f, slot, id, value) = moves[k];
            assert!(chain.queue(
                slot,
                Timed::new((f - at) as u32, NodeEvent::Param { id, value })
            ));
            k += 1;
        }
        chain.process(
            &Ctx::new(RATE, at as u64),
            &mut l[at..at + len],
            &mut r[at..at + len],
        );
        at += len;
    }
    (l, r)
}

fn full_chain() -> Chain {
    let mut c = Chain::new();
    c.prepare(RATE, MAX_BLOCK);
    c.push(Box::new(Drive::new()));
    c.push(Box::new(MultiFilter::new()));
    c.push(Box::new(Crush::new()));
    c.push(Box::new(Delay::new()));
    c.push(Box::new(Compressor::new()));
    c.push(Box::new(DjFilter::new()));
    c.push(Box::new(Cloud::new()));
    c.push(Box::new(Eq::new()));
    c.push(Box::new(Chorus::new()));
    c.push(Box::new(Flanger::new()));
    c.push(Box::new(Phaser::new()));
    c.push(Box::new(LoFi::new()));
    c.push(Box::new(MasterSafety::new()));
    c
}

#[test]
fn chain_gives_the_same_output_for_any_block_size() {
    let frames = 96_000;
    let input = program(frames);
    let switches = [
        (5_000, 3, false),
        (9_000, 0, false),
        (20_000, 3, true),
        (20_500, 2, false),
        (30_000, 0, true),
        (31_000, 3, false),
        (40_000, 6, false),
        (60_000, 3, true),
        (60_400, 3, false),
        (70_000, 2, true),
        (80_000, 6, true),
    ];
    let mut moves = Vec::new();
    let c = full_chain();
    for slot in 0..c.len() {
        // No freeze here: an endless tail turning finite is checked per block.
        let skip: &[u16] = match slot {
            3 => &[ids::delay::FREEZE],
            6 => &[ids::cloud::FREEZE],
            _ => &[],
        };
        for (f, id, v) in automation(
            c.node(slot).unwrap().params(),
            frames,
            40 + slot as u64,
            skip,
        ) {
            moves.push((f, slot, id, v));
        }
    }
    moves.sort_by_key(|m| m.0);
    let reference = run_chain(&mut full_chain(), &input, &mut || 4096, &switches, &moves);
    for size in [1usize, 64, 333, MAX_BLOCK] {
        let out = run_chain(&mut full_chain(), &input, &mut || size, &switches, &moves);
        assert_identical("chain", &format!("{size}-frame blocks"), &reference, &out);
    }
    let mut rng = Lcg(5);
    let out = run_chain(
        &mut full_chain(),
        &input,
        &mut || rng.range(1, 900),
        &switches,
        &moves,
    );
    assert_identical("chain", "random blocks", &reference, &out);
}

#[test]
fn bypass_is_click_free_and_latency_never_moves() {
    use ids::drive::*;
    let mut c = Chain::new();
    c.prepare(RATE, MAX_BLOCK);
    let d = c.push(Box::new(Drive::new()));
    c.set_param(d, MODE, 1.0);
    c.set_param(d, DRIVE, 30.0);
    c.set_param(d, TONE, 3000.0);
    assert_eq!(c.latency(), 55);
    let x = sine(80.0, 0.5, 48_000);
    let input = (x.clone(), x.clone());
    let out = run_chain(
        &mut c,
        &input,
        &mut || 128,
        &[(12_000, d, false), (30_000, d, true)],
        &[],
    );
    assert_eq!(c.latency(), 55, "the same when bypassed");
    let steps = |a: usize, b: usize| {
        (a..b)
            .map(|i| (out.0[i] - out.0[i - 1]).abs())
            .fold(0.0f32, f32::max)
    };
    let steady = steps(6_000, 11_000).max(steps(20_000, 29_000));
    let around = steps(11_900, 14_000).max(steps(29_900, 32_000));
    assert!(
        around <= steady * 1.05,
        "largest step at a switch {around} vs steady {steady}"
    );
    // Fully bypassed, the output is the input, exactly, one latency late.
    assert!((16_000..29_000).all(|i| out.0[i] == x[i - 55]));
}

#[test]
fn delay_keeps_ringing_after_bypass_then_rests() {
    use ids::delay::*;
    let mut c = Chain::new();
    c.prepare(RATE, MAX_BLOCK);
    let s = c.push(Box::new(Delay::new()));
    for (id, v) in [(SYNC, 0.0), (TIME, 100.0), (FEEDBACK, 50.0), (MIX, 50.0)] {
        c.set_param(s, id, v);
    }
    let frames = 96_000;
    let mut x = vec![0.0f32; frames];
    x[..2400].copy_from_slice(&sine(440.0, 0.5, 2400));
    let out = run_chain(
        &mut c,
        &(x.clone(), x.clone()),
        &mut || 128,
        &[(7_200, s, false)],
        &[],
    );
    // Echoes every 100 ms keep coming after the slot is bypassed at 150 ms.
    for k in 2..6 {
        let at = k * 4800;
        assert!(
            peak(&out.0[at..at + 2400]) > 0.5 * 0.5f32.powi(k as i32) * 0.5,
            "echo {k} after bypass: {}",
            peak(&out.0[at..at + 2400])
        );
    }
    // And they die away instead of stopping dead.
    assert!(peak(&out.0[90_000..]) < 1e-3);
}

/// A node that puts out NaN once, at a set frame.
struct Glitch {
    at: u64,
    done: bool,
}

static GLITCH: NodeInfo = NodeInfo {
    id: "test.glitch",
    name: "Glitch",
    kind: mpfree_fx::mpfree_node::Kind::Effect,
};

impl Node for Glitch {
    fn info(&self) -> &'static NodeInfo {
        &GLITCH
    }
    fn params(&self) -> &'static [ParamInfo] {
        &[]
    }
    fn set_param(&mut self, _id: u16, _value: f32) {}
    fn param(&self, _id: u16) -> f32 {
        f32::NAN
    }
    fn prepare(&mut self, _rate: u32, _max_block: usize) {}
    fn reset(&mut self) {
        self.done = true;
    }
    fn process(&mut self, ctx: &Ctx, _events: &[Timed], left: &mut [f32], _right: &mut [f32]) {
        for (i, v) in left.iter_mut().enumerate() {
            if !self.done && ctx.frame + i as u64 == self.at {
                *v = f32::NAN;
            }
        }
    }
}

#[test]
fn a_node_that_outputs_nan_is_reset_and_counted() {
    let mut c = Chain::new();
    c.prepare(RATE, MAX_BLOCK);
    c.push(Box::new(Glitch {
        at: 1000,
        done: false,
    }));
    c.push(Box::new(MultiFilter::new()));
    let x = program(4800);
    let out = run_chain(&mut c, &x, &mut || 256, &[], &[]);
    assert_eq!(c.stats().nan_resets, 1);
    assert!(out.0.iter().chain(&out.1).all(|v| v.is_finite()));
    // The glitching slot plays dry for that block, which is what it would
    // have played anyway; the filter after it is untouched.
    let mut clean = Chain::new();
    clean.prepare(RATE, MAX_BLOCK);
    clean.push(Box::new(Glitch {
        at: 1000,
        done: true,
    }));
    clean.push(Box::new(MultiFilter::new()));
    let want = run_chain(&mut clean, &x, &mut || 256, &[], &[]);
    assert_identical("nan guard", "against a clean run", &want, &out);
    assert_eq!(clean.stats().nan_resets, 0);
}

#[test]
fn a_full_event_queue_drops_and_counts() {
    let mut c = Chain::new();
    c.push(Box::new(Crush::new()));
    for i in 0..MAX_SLOT_EVENTS {
        assert!(c.queue(
            0,
            Timed::new(i as u32 % 64, NodeEvent::Param { id: 0, value: 4.0 })
        ));
    }
    assert!(!c.queue(0, Timed::new(0, NodeEvent::Param { id: 0, value: 4.0 })));
    assert_eq!(c.stats().dropped_events, 1);
    assert!(
        !c.queue(7, Timed::new(0, NodeEvent::Param { id: 0, value: 4.0 })),
        "no such slot"
    );
}

/// The strongest frequency in `x` within `lo..hi` Hz, by a Goertzel scan in
/// 1-cent steps.
fn strongest(x: &[f32], lo: f32, hi: f32) -> f32 {
    let mut best = (0.0f32, lo);
    let mut f = lo;
    while f <= hi {
        let w = 2.0 * PI * f / RATE as f32;
        let c = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0f32, 0.0f32);
        for &v in x {
            let s = v + c * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        let p = s1 * s1 + s2 * s2 - c * s1 * s2;
        if p > best.0 {
            best = (p, f);
        }
        f *= 2f32.powf(1.0 / 1200.0);
    }
    best.1
}

#[test]
fn the_comb_rings_in_tune_and_notes_play_it() {
    // An impulse into the comb: its ring peaks at the pitch, within 5
    // cents, from 55 Hz to 4 kHz, and at its second harmonic up to 5 kHz
    // (above that the damping's delay, matched at the pitch, sharpens the
    // upper harmonics a little, as in a real string).
    for &hz in &[55.0f32, 220.0, 1_000.0, 4_000.0] {
        let mut c = Comb::new();
        c.prepare(RATE, MAX_BLOCK);
        c.set_param(ids::comb::PITCH, hz);
        c.set_param(ids::comb::FEEDBACK, 95.0);
        c.set_param(ids::comb::MIX, 100.0);
        c.set_param(ids::comb::SPREAD, 0.0);
        c.reset();
        let n = RATE as usize;
        let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
        l[0] = 1.0;
        r[0] = 1.0;
        run_once(&mut c, &mut l, &mut r);
        // Past the first few periods (the impulse itself is broadband).
        let ring = &l[(3.0 * RATE as f32 / hz) as usize..];
        for k in [1.0f32, 2.0] {
            if hz * k > 5_000.0 {
                continue;
            }
            let got = strongest(ring, hz * k * 0.95, hz * k * 1.05);
            let cents = 1200.0 * (got / (hz * k)).log2();
            assert!(
                cents.abs() < 5.0,
                "{hz} Hz, harmonic {k}: {got} Hz ({cents:.1} cents)"
            );
        }
        // The peaks stay near unity gain.
        assert!(peak(&l) < 1.5, "{hz} Hz: peak {}", peak(&l));
    }
    // A note sets the pitch (Key follow on by default): A3 = 220 Hz.
    let mut c = Comb::new();
    c.prepare(RATE, MAX_BLOCK);
    c.set_param(ids::comb::PITCH, 1_000.0);
    c.reset();
    let (mut l, mut r) = (vec![0.0f32; 64], vec![0.0f32; 64]);
    c.process(
        &Ctx::new(RATE, 0),
        &[Timed::new(
            0,
            NodeEvent::NoteOn {
                note: 57,
                velocity: 100,
            },
        )],
        &mut l,
        &mut r,
    );
    assert!((c.param(ids::comb::PITCH) - 220.0).abs() < 0.01);
    // Negative feedback: odd harmonics of half the pitch.
    let mut c = Comb::new();
    c.prepare(RATE, MAX_BLOCK);
    c.set_param(ids::comb::PITCH, 440.0);
    c.set_param(ids::comb::FEEDBACK, -95.0);
    c.set_param(ids::comb::MIX, 100.0);
    c.set_param(ids::comb::SPREAD, 0.0);
    c.reset();
    let n = RATE as usize;
    let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
    l[0] = 1.0;
    run_once(&mut c, &mut l, &mut r);
    let got = strongest(&l[n / 20..], 200.0, 240.0);
    assert!(
        (got - 220.0).abs() < 1.0,
        "negative feedback rings at {got} Hz"
    );
    // The drawn curve matches: a peak at the pitch, a dip between.
    let mut c = Comb::new();
    c.prepare(RATE, MAX_BLOCK);
    c.set_param(ids::comb::PITCH, 220.0);
    let (at, between) = (c.response_db(220.0), c.response_db(330.0));
    assert!(
        at > -1.5 && between < at - 15.0,
        "curve: {at} dB, {between} dB"
    );
}
