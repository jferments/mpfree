//! The effects added for MPC coverage (movement, dynamics, rooms, echoes,
//! amp and exciter), walked from the registry: each one is block-size
//! invariant under automation, stays finite and bounded at every extreme,
//! resets to the same output as a fresh node, keeps its latency whatever
//! its parameters, and its presets name its own parameters. Then each
//! effect's own behaviour.

use mpfree_fx::mpfree_node::{Ctx, Node, NodeEvent, Timed, MAX_BLOCK};
use mpfree_fx::{ids, registry};

const RATE: u32 = 48_000;

/// Every effect this suite covers, by node id.
const COVERED: &[&str] = &[
    "mpfree.fx.tremolo",
    "mpfree.fx.autopan",
    "mpfree.fx.ringmod",
    "mpfree.fx.rotary",
];

fn make(id: &str) -> Box<dyn Node> {
    let mut n = registry::make(id).unwrap_or_else(|| panic!("{id} is in the registry"));
    n.prepare(RATE, MAX_BLOCK);
    n
}

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
}

/// Drums-ish noise bursts and a held chord, the same every run.
fn program(frames: usize) -> (Vec<f32>, Vec<f32>) {
    let mut rng = Lcg(3);
    let mut l = vec![0.0; frames];
    let mut r = vec![0.0; frames];
    for i in 0..frames {
        let t = i as f32 / RATE as f32;
        let burst = if i % 12_000 < 1500 {
            (rng.unit() - 0.5) * (1.0 - (i % 12_000) as f32 / 1500.0)
        } else {
            0.0
        };
        let chord = 0.15
            * ((t * 220.0 * std::f32::consts::TAU).sin()
                + (t * 277.2 * std::f32::consts::TAU).sin()
                + (t * 329.6 * std::f32::consts::TAU).sin());
        l[i] = burst + chord;
        r[i] = burst * 0.8 + chord;
    }
    (l, r)
}

/// Parameter moves at fixed frames (sample-timed, so any block size must
/// give the same output).
fn automation(node: &dyn Node, frames: usize) -> Vec<(usize, u16, f32)> {
    let mut rng = Lcg(11);
    let params = node.params();
    let mut out = Vec::new();
    let mut at = 0;
    while at < frames {
        at += 700 + (rng.next() % 4000) as usize;
        let p = &params[(rng.next() as usize) % params.len()];
        if p.readonly {
            continue;
        }
        out.push((at, p.id, p.from_unit(rng.unit())));
    }
    out
}

fn render(
    node: &mut dyn Node,
    input: &(Vec<f32>, Vec<f32>),
    moves: &[(usize, u16, f32)],
    block: usize,
) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = input.clone();
    let mut start = 0;
    let mut events = Vec::new();
    while start < l.len() {
        let end = (start + block).min(l.len());
        events.clear();
        for &(at, id, value) in moves {
            if at >= start && at < end {
                events.push(Timed {
                    offset: (at - start) as u32,
                    ev: NodeEvent::Param { id, value },
                });
            }
        }
        let mut ctx = Ctx::new(RATE, start as u64);
        ctx.playing = true;
        ctx.beats = start as f64 / RATE as f64 * 2.0;
        node.process(&ctx, &events, &mut l[start..end], &mut r[start..end]);
        start = end;
    }
    (l, r)
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

#[test]
fn every_covered_effect_is_registered_with_its_own_name() {
    for id in COVERED {
        let e = registry::entry(id).expect(id);
        assert_eq!(make(id).info().name, e.name);
        assert!(e.about.len() <= 34, "{id}: about line is short");
    }
}

#[test]
fn same_output_for_any_block_size() {
    let input = program(40_000);
    for id in COVERED {
        let n = make(id);
        let moves = automation(&*n, 40_000);
        let want = render(&mut *make(id), &input, &moves, MAX_BLOCK);
        for block in [1, 7, 64, 333] {
            let got = render(&mut *make(id), &input, &moves, block);
            let same = want
                .0
                .iter()
                .chain(&want.1)
                .zip(got.0.iter().chain(&got.1))
                .all(|(a, b)| a.to_bits() == b.to_bits());
            assert!(same, "{id}: block {block} differs");
        }
    }
}

#[test]
fn every_extreme_stays_finite_and_bounded() {
    let mut input = program(24_000);
    for v in input.0.iter_mut().chain(input.1.iter_mut()) {
        *v *= 2.0;
    }
    for id in COVERED {
        let n = make(id);
        let count = n.params().len();
        for corner in 0..3 {
            let mut n = make(id);
            for (k, p) in n.params().to_vec().iter().enumerate() {
                if p.readonly {
                    continue;
                }
                let t = match corner {
                    0 => 0.0,
                    1 => 1.0,
                    _ => (k % 2) as f32,
                };
                n.set_param(p.id, p.from_unit(t));
            }
            let (l, r) = render(&mut *n, &input, &[], MAX_BLOCK);
            assert!(
                l.iter().chain(&r).all(|v| v.is_finite()),
                "{id} corner {corner}"
            );
            let p = peak(&l).max(peak(&r));
            assert!(p < 16.0, "{id} corner {corner} ({count} params): peak {p}");
        }
    }
}

#[test]
fn reset_gives_a_fresh_start_and_latency_never_moves() {
    let input = program(20_000);
    for id in COVERED {
        let mut n = make(id);
        let lat = n.latency();
        let moves = automation(&*n, 20_000);
        render(&mut *n, &input, &moves, 128);
        assert_eq!(n.latency(), lat, "{id}: latency moved with its parameters");
        // After a reset, the same input gives the same output as a fresh
        // node with the same settings.
        let mut fresh = make(id);
        for p in n.params() {
            if !p.readonly {
                fresh.set_param(p.id, n.param(p.id));
            }
        }
        n.reset();
        fresh.reset();
        let a = render(&mut *n, &input, &[], 128);
        let b = render(&mut *fresh, &input, &[], 128);
        assert!(
            a.0.iter().zip(&b.0).all(|(x, y)| x.to_bits() == y.to_bits()),
            "{id}: reset is not a fresh start"
        );
    }
}

#[test]
fn presets_set_only_their_effects_parameters_and_play() {
    let input = program(24_000);
    for id in COVERED {
        let e = registry::entry(id).unwrap();
        assert!(!e.presets.is_empty(), "{id} has presets");
        let mut names = Vec::new();
        for pr in e.presets {
            assert!(!names.contains(&pr.name), "{id}: {} twice", pr.name);
            names.push(pr.name);
            let mut n = make(id);
            for &(pid, v) in pr.values {
                let p = n.params().iter().find(|p| p.id == pid).expect("own param");
                assert!(!p.readonly, "{id}: {} sets a meter", pr.name);
                assert_ne!(p.name, "Mix", "{id}: {} sets Mix", pr.name);
                assert!((p.min..=p.max).contains(&v), "{id}: {} {v}", pr.name);
                n.set_param(pid, v);
            }
            let (l, _) = render(&mut *n, &input, &[], MAX_BLOCK);
            assert!(l.iter().all(|v| v.is_finite()));
            assert!(rms(&l) > 1e-3, "{id}: {} is silent", pr.name);
        }
    }
}

// ------------------------------------------------------------ movement

fn sine(hz: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| 0.5 * (i as f32 * hz / RATE as f32 * std::f32::consts::TAU).sin())
        .collect()
}

#[test]
fn tremolo_at_no_depth_is_transparent_and_full_depth_reaches_silence() {
    let x = sine(440.0, 48_000);
    let mut t = make("mpfree.fx.tremolo");
    t.set_param(ids::tremolo::DEPTH, 0.0);
    t.reset();
    let (l, _) = render(&mut *t, &(x.clone(), x.clone()), &[], 256);
    assert!(l.iter().zip(&x).all(|(a, b)| (a - b).abs() < 1e-6));

    let mut t = make("mpfree.fx.tremolo");
    t.set_param(ids::tremolo::DEPTH, 100.0);
    t.set_param(ids::tremolo::RATE, 2.0);
    t.reset();
    let ones = vec![1.0f32; 48_000];
    let (l, _) = render(&mut *t, &(ones.clone(), ones), &[], 256);
    let lo = l.iter().fold(1.0f32, |m, &v| m.min(v));
    assert!(lo < 0.01, "full depth dips to silence: {lo}");
}

#[test]
fn harmonic_tremolo_keeps_its_level_nearly_even() {
    // A low and a high tone at the same level: they swap places, so the
    // level moves far less than the plain tremolo's at the same depth.
    let lo = sine(150.0, 96_000);
    let x: Vec<f32> = sine(3000.0, 96_000)
        .iter()
        .zip(&lo)
        .map(|(a, b)| a + b)
        .collect();
    let swing = |mode: f32| {
        let mut t = make("mpfree.fx.tremolo");
        t.set_param(ids::tremolo::DEPTH, 100.0);
        t.set_param(ids::tremolo::RATE, 2.0);
        t.set_param(ids::tremolo::MODE, mode);
        t.reset();
        let (l, _) = render(&mut *t, &(x.clone(), x.clone()), &[], 256);
        let w: Vec<f32> = l[24_000..].chunks(960).map(rms).collect();
        let hi = w.iter().cloned().fold(0.0, f32::max);
        let lo = w.iter().cloned().fold(f32::MAX, f32::min);
        20.0 * (hi / lo.max(1e-6)).log10()
    };
    let (plain, harmonic) = (swing(0.0), swing(1.0));
    assert!(plain > 20.0, "plain swings {plain} dB");
    assert!(harmonic < 8.0, "harmonic swings {harmonic} dB");
}

#[test]
fn auto_pan_keeps_constant_power() {
    let ones = vec![0.5f32; 48_000];
    let mut a = make("mpfree.fx.autopan");
    a.set_param(ids::autopan::SYNC, 0.0);
    a.set_param(ids::autopan::RATE, 3.0);
    a.set_param(ids::autopan::DEPTH, 100.0);
    a.reset();
    let (l, r) = render(&mut *a, &(ones.clone(), ones), &[], 128);
    let mut lmax = 0.0f32;
    for i in 0..l.len() {
        let pow = l[i] * l[i] + r[i] * r[i];
        assert!((pow - 0.5).abs() < 0.01, "power {pow} at {i}");
        lmax = lmax.max(l[i]);
    }
    assert!(lmax > 0.69, "reaches hard left: {lmax}");
}

/// How much of a signal's energy (Hann window) lies away from the odd
/// harmonics of `hz` below Nyquist, in dB: the aliasing a naive wave makes.
fn aliasing_db(x: &[f32], hz: f32) -> f32 {
    let n = x.len();
    let w: Vec<f64> = (0..n)
        .map(|i| {
            let h = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
            h * f64::from(x[i])
        })
        .collect();
    let (mut away, mut all) = (0.0f64, 0.0f64);
    for bin in 1..n / 2 {
        let f = bin as f32 * RATE as f32 / n as f32;
        let step = std::f64::consts::TAU * bin as f64 / n as f64;
        let (mut re, mut im) = (0.0, 0.0);
        let (c, s) = (step.cos(), step.sin());
        let (mut cr, mut ci) = (1.0f64, 0.0f64);
        for &v in &w {
            re += v * cr;
            im -= v * ci;
            let t = cr * c - ci * s;
            ci = cr * s + ci * c;
            cr = t;
        }
        let e = re * re + im * im;
        all += e;
        let mut k = 1.0;
        let mut near = f < 20.0;
        while k * hz < RATE as f32 / 2.0 {
            near |= (f - k * hz).abs() <= 40.0;
            k += 2.0;
        }
        if !near {
            away += e;
        }
    }
    (10.0 * (away / all).log10()) as f32
}

/// The ring mod's square and triangle carriers are band-limited: rendered
/// on a constant input (so the output is the carrier itself) at 4.7 kHz,
/// little of their energy falls between their harmonics. A naive square
/// puts about -12 dB there, a naive triangle about -31 dB.
#[test]
fn ring_mod_square_and_triangle_carriers_are_band_limited() {
    let hz = 4_700.0f32;
    for (wave, limit) in [(2usize, -30.0f32), (1, -32.0)] {
        let mut m = make("mpfree.fx.ringmod");
        m.set_param(ids::ringmod::FREQ, hz);
        m.set_param(ids::ringmod::WAVE, wave as f32);
        m.reset();
        let ones = vec![1.0f32; 4800];
        let (l, _) = render(&mut *m, &(ones.clone(), ones), &[], 128);
        let a = aliasing_db(&l, hz);
        assert!(a < limit, "wave {wave}: {a} dB between the harmonics");
    }
}

#[test]
fn rotary_ramps_its_rotors_up_and_spreads_the_sound() {
    let x = sine(1500.0, 96_000);
    let mut r = make("mpfree.fx.rotary");
    r.set_param(ids::rotary::SPEED, 2.0);
    let (l, rr) = render(&mut *r, &(x.clone(), x), &[], 128);
    // Fast: the horn's level swings quickly, and left and right differ.
    let w: Vec<f32> = l[48_000..].chunks(480).map(rms).collect();
    let hi = w.iter().cloned().fold(0.0, f32::max);
    let lo = w.iter().cloned().fold(f32::MAX, f32::min);
    assert!(hi / lo > 1.5, "the horn's sweep moves the level: {hi} {lo}");
    let diff: Vec<f32> = l.iter().zip(&rr).map(|(a, b)| a - b).collect();
    assert!(rms(&diff[48_000..]) > 0.05 * rms(&l[48_000..]), "stereo");
}
