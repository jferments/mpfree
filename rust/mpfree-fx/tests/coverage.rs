//! The effects added for MPC coverage (movement, dynamics, rooms, echoes,
//! amp and exciter), walked from the registry: each one is block-size
//! invariant under automation, stays finite and bounded at every extreme,
//! resets to the same output as a fresh node, keeps its latency whatever
//! its parameters, and its presets name its own parameters. Then each
//! effect's own behaviour.

use mpfree_fx::mpfree_node::{Ctx, Node, NodeEvent, Timed, Unit, MAX_BLOCK};
use mpfree_fx::{ids, registry};

const RATE: u32 = 48_000;

/// Every effect this suite covers, by node id.
const COVERED: &[&str] = &[
    "mpfree.fx.tremolo",
    "mpfree.fx.autopan",
    "mpfree.fx.ringmod",
    "mpfree.fx.rotary",
    "mpfree.fx.limiter",
    "mpfree.fx.gate",
    "mpfree.fx.deesser",
    "mpfree.fx.multiband",
    "mpfree.fx.tape_delay",
    "mpfree.fx.multitap",
    "mpfree.fx.room",
    "mpfree.fx.plate",
    "mpfree.fx.spring",
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
                // Make-up and output gains stay at unity or under: a
                // boost the user asks for is not a runaway.
                let mut v = p.from_unit(t);
                if p.unit == Unit::Db && p.min < 0.0 && p.max > 0.0 {
                    v = v.min(0.0);
                }
                n.set_param(p.id, v);
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
            a.0.iter()
                .zip(&b.0)
                .all(|(x, y)| x.to_bits() == y.to_bits()),
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

// ------------------------------------------------------------ dynamics

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-9).log10()
}

#[test]
fn limiter_never_passes_its_ceiling_and_leaves_quiet_audio_alone() {
    let mut lim = make("mpfree.fx.limiter");
    lim.set_param(ids::limiter::GAIN, 18.0);
    lim.set_param(ids::limiter::CEILING, -1.0);
    lim.reset();
    let input = program(48_000);
    let (l, r) = render(&mut *lim, &input, &[], 128);
    let ceiling = 10f32.powf(-1.0 / 20.0);
    let p = peak(&l).max(peak(&r));
    assert!(p <= ceiling * 1.000_01, "peak {p} over {ceiling}");
    assert!(lim.param(ids::limiter::REDUCTION) > 3.0);

    // Under the ceiling, the output is the input, late by the latency.
    let mut lim = make("mpfree.fx.limiter");
    let quiet = sine(300.0, 9600);
    let (l, _) = render(&mut *lim, &(quiet.clone(), quiet.clone()), &[], 64);
    let d = lim.latency() as usize;
    assert_eq!(d, 77, "1.5 ms at 48 kHz plus the true-peak filter's half");
    for i in d..l.len() {
        assert!((l[i] - quiet[i - d]).abs() < 1e-6, "frame {i}");
    }
}

#[test]
fn limiter_true_peak_catches_peaks_between_samples() {
    // A sine at a quarter of the rate, 45 degrees off: its samples peak at
    // 0.707 but the wave itself reaches 1.0.
    let x: Vec<f32> = (0..24_000)
        .map(|i| (std::f32::consts::FRAC_PI_2 * i as f32 + std::f32::consts::FRAC_PI_4).sin())
        .collect();
    let level = |tp: f32| {
        let mut lim = make("mpfree.fx.limiter");
        lim.set_param(ids::limiter::CEILING, -1.0);
        lim.set_param(ids::limiter::TRUE_PEAK, tp);
        lim.reset();
        let (l, _) = render(&mut *lim, &(x.clone(), x.clone()), &[], 128);
        // A sine's true peak is its RMS times root 2.
        db(rms(&l[12_000..]) * std::f32::consts::SQRT_2)
    };
    let (sample, truep) = (level(0.0), level(1.0));
    assert!(sample > -0.2, "sample peaks pass: {sample}");
    assert!(truep < -0.8, "true peak held to the ceiling: {truep}");
}

#[test]
fn limiter_auto_release_recovers_fast_after_a_click() {
    // A 1 ms spike then quiet: with Auto, the gain is back within half a
    // dB in under 100 ms; held down far longer with a 1 s fixed release.
    let mut x = vec![0.1f32; 48_000];
    for v in &mut x[4800..4848] {
        *v = 4.0;
    }
    let back = |auto: f32| {
        let mut lim = make("mpfree.fx.limiter");
        lim.set_param(
            ids::limiter::RELEASE,
            if auto > 0.5 { 100.0 } else { 1000.0 },
        );
        lim.set_param(ids::limiter::AUTO, auto);
        lim.reset();
        let (l, _) = render(&mut *lim, &(x.clone(), x.clone()), &[], 128);
        (5000..48_000).find(|&i| l[i] > 0.095).unwrap_or(48_000)
    };
    let (fast, slow) = (back(1.0), back(0.0));
    assert!(fast < 5000 + 4800, "auto is back in {fast}");
    assert!(slow > fast + 9600, "fixed 1 s is slower: {slow}");
}

#[test]
fn gate_shuts_on_quiet_noise_and_opens_without_chatter() {
    let mut rng = Lcg(9);
    let mut x: Vec<f32> = (0..48_000).map(|_| 0.002 * (rng.unit() - 0.5)).collect();
    for v in &mut x[24_000..30_000] {
        *v *= 200.0;
    }
    let mut g = make("mpfree.fx.gate");
    g.set_param(ids::gate::THRESHOLD, -40.0);
    g.reset();
    let (l, _) = render(&mut *g, &(x.clone(), x.clone()), &[], 128);
    assert!(peak(&l[8_000..23_000]) < 1e-6, "quiet noise is shut out");
    let d = g.latency() as usize;
    assert!(rms(&l[24_000 + d..30_000]) > 0.9 * rms(&x[24_000..30_000 - d]));

    // A level that wavers 1 dB over and 2 dB under the threshold keeps the
    // gate open (it shuts 4 dB under).
    let wav: Vec<f32> = (0..48_000)
        .map(|i| {
            let a = if (i / 2400) % 2 == 0 { -39.0 } else { -42.0 };
            10f32.powf(a / 20.0) * (i as f32 * 0.07).sin().signum()
        })
        .collect();
    let mut g = make("mpfree.fx.gate");
    g.set_param(ids::gate::THRESHOLD, -40.0);
    g.set_param(ids::gate::HOLD, 0.0);
    g.reset();
    let (l, _) = render(&mut *g, &(wav.clone(), wav.clone()), &[], 128);
    let shut = l[4800..]
        .iter()
        .zip(&wav[4800 - d..])
        .filter(|(a, b)| a.abs() < 0.5 * b.abs())
        .count();
    assert_eq!(shut, 0, "no chatter");
}

#[test]
fn gate_opens_from_the_sidechain_key() {
    let n = 24_000;
    let pad = vec![0.3f32; n];
    let key: Vec<f32> = (0..n)
        .map(|i| if (i / 6000) % 2 == 1 { 0.5 } else { 0.0 })
        .collect();
    let mut g = make("mpfree.fx.gate");
    g.set_param(ids::gate::KEY, 1.0);
    g.set_param(ids::gate::HOLD, 0.0);
    g.set_param(ids::gate::RELEASE, 10.0);
    g.reset();
    let (mut l, mut r) = (pad.clone(), pad.clone());
    for start in (0..n).step_by(128) {
        let end = (start + 128).min(n);
        let mut ctx = Ctx::new(RATE, start as u64);
        ctx.sidechain = Some((&key[start..end], &key[start..end]));
        g.process(&ctx, &[], &mut l[start..end], &mut r[start..end]);
    }
    assert!(peak(&l[2000..5900]) < 1e-3, "shut while the key is quiet");
    assert!(rms(&l[7000..11_900]) > 0.29, "open while the key plays");
}

#[test]
fn de_esser_turns_down_sibilance_and_leaves_the_rest() {
    let mut rng = Lcg(21);
    // A vocal-ish low tone with bursts of bright noise.
    let low = sine(200.0, 48_000);
    let mut hiss = vec![0.0f32; 48_000];
    let mut hp = 0.0f32;
    for (i, v) in hiss.iter_mut().enumerate() {
        let n = rng.unit() - 0.5;
        let y = n - hp; // first difference: a bright noise
        hp = n;
        if (i / 6000) % 2 == 1 {
            *v = 0.4 * y;
        }
    }
    let x: Vec<f32> = low.iter().zip(&hiss).map(|(a, b)| a + b).collect();
    let mut d = make("mpfree.fx.deesser");
    d.reset();
    let (l, _) = render(&mut *d, &(x.clone(), x.clone()), &[], 128);
    // What the de-esser did to the hiss: the output less the low tone
    // (which split mode passes untouched).
    let hiss_out: Vec<f32> = l.iter().zip(&low).map(|(a, b)| a - b).collect();
    let burst = |y: &[f32]| rms(&y[6500..11_900]);
    let quiet = |y: &[f32]| rms(&y[1000..5900]);
    assert!(
        db(burst(&hiss_out)) < db(burst(&hiss)) - 3.0,
        "sibilance down: {} to {}, reduction {}",
        db(burst(&hiss)),
        db(burst(&hiss_out)),
        d.param(ids::deesser::REDUCTION)
    );
    assert!(
        (db(quiet(&l)) - db(quiet(&x))).abs() < 0.05,
        "the low tone untouched"
    );
    assert!(d.param(ids::deesser::REDUCTION) >= 0.0);
}

#[test]
fn multiband_sums_flat_and_squashes_only_the_loud_band() {
    let mut m = make("mpfree.fx.multiband");
    for b in 0..3 {
        m.set_param(ids::multiband::ratio(b), 1.0);
    }
    for hz in [60.0f32, 200.0, 900.0, 2500.0, 9000.0] {
        m.reset();
        let x = sine(hz, 24_000);
        let (l, _) = render(&mut *m, &(x.clone(), x.clone()), &[], 128);
        let change = db(rms(&l[12_000..])) - db(rms(&x[12_000..]));
        assert!(change.abs() < 0.1, "{hz} Hz: {change} dB");
    }
    // A loud low tone and a quiet high one: only the low band comes down.
    let mut m = make("mpfree.fx.multiband");
    m.set_param(ids::multiband::threshold(0), -30.0);
    m.set_param(ids::multiband::ratio(0), 10.0);
    m.reset();
    let lo = sine(80.0, 48_000);
    let hi: Vec<f32> = sine(8000.0, 48_000).iter().map(|v| v * 0.05).collect();
    let both: Vec<f32> = lo.iter().zip(&hi).map(|(a, b)| a + b).collect();
    let (l, _) = render(&mut *m, &(both.clone(), both), &[], 128);
    assert!(m.param(ids::multiband::reduction(0)) > 10.0);
    assert!(m.param(ids::multiband::reduction(2)) < 0.1);
    assert!(db(rms(&l[24_000..])) < db(rms(&lo[24_000..])) - 10.0);
}

// ------------------------------------------------------------ rooms

/// The reverb's impulse response, fully wet.
fn impulse_response(id: &str, set: &[(u16, f32)], frames: usize) -> (Vec<f32>, Vec<f32>) {
    let mut n = make(id);
    for &(p, v) in set {
        n.set_param(p, v);
    }
    let mix = n.params().iter().find(|p| p.name == "Mix").unwrap().id;
    n.set_param(mix, 100.0);
    n.reset();
    let mut x = vec![0.0f32; frames];
    x[0] = 1.0;
    render(&mut *n, &(x.clone(), x), &[], 256)
}

/// The time to fall 60 dB, from the backward-integrated energy between
/// -5 and -35 dB (Schroeder's method), in seconds.
fn rt60(x: &[f32]) -> f32 {
    let mut edc = vec![0.0f64; x.len()];
    let mut acc = 0.0f64;
    for i in (0..x.len()).rev() {
        acc += f64::from(x[i]) * f64::from(x[i]);
        edc[i] = acc;
    }
    let total = edc[0];
    let at = |db: f64| {
        edc.iter()
            .position(|&e| 10.0 * (e / total).log10() < db)
            .unwrap_or(x.len())
    };
    let (a, b) = (at(-5.0), at(-35.0));
    (b - a) as f32 / RATE as f32 * 2.0
}

#[test]
fn reverbs_decay_in_about_their_decay_time() {
    for (id, decay, damping) in [
        ("mpfree.fx.plate", ids::plate::DECAY, ids::plate::DAMPING),
        ("mpfree.fx.room", ids::room::DECAY, ids::room::DAMPING),
    ] {
        for secs in [0.8f32, 2.0] {
            let (l, r) = impulse_response(
                id,
                &[(decay, secs * 1000.0), (damping, 0.0)],
                4 * RATE as usize,
            );
            let t = rt60(&l).max(rt60(&r));
            assert!(
                (t / secs - 1.0).abs() < 0.3,
                "{id}: {t} s for a decay of {secs} s"
            );
        }
    }
    let (l, _) = impulse_response(
        "mpfree.fx.spring",
        &[(ids::spring::DECAY, 2000.0)],
        4 * RATE as usize,
    );
    let t = rt60(&l);
    assert!((t / 2.0 - 1.0).abs() < 0.4, "spring: {t} s for 2 s");
}

#[test]
fn plate_and_room_are_wide_and_room_gate_cuts_the_tail() {
    for id in ["mpfree.fx.plate", "mpfree.fx.room"] {
        let (l, r) = impulse_response(id, &[], RATE as usize);
        let (mut lr, mut ll, mut rr) = (0.0f64, 0.0f64, 0.0f64);
        for i in 0..l.len() {
            lr += f64::from(l[i] * r[i]);
            ll += f64::from(l[i] * l[i]);
            rr += f64::from(r[i] * r[i]);
        }
        let corr = lr / (ll * rr).sqrt();
        assert!(corr.abs() < 0.5, "{id}: left and right correlate {corr}");
    }
    let (l, _) = impulse_response(
        "mpfree.fx.room",
        &[(ids::room::DECAY, 3000.0), (ids::room::GATE, 200.0)],
        RATE as usize,
    );
    assert!(rms(&l[2000..9000]) > 1e-3, "rings while the gate is open");
    assert!(peak(&l[12_000..]) < 1e-6, "silent once the gate shuts");
}

#[test]
fn spring_chirps_lows_after_highs() {
    // The chain delays low frequencies more than high ones below its
    // corner: a 300 Hz burst's first return comes later than a 3 kHz one's.
    let arrival = |hz: f32| {
        let mut n = make("mpfree.fx.spring");
        n.set_param(ids::spring::MIX, 100.0);
        n.set_param(ids::spring::LOW_CUT, 20.0);
        n.reset();
        let mut x = vec![0.0f32; 12_000];
        for (i, v) in x.iter_mut().enumerate().take(480) {
            let w = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / 480.0).cos();
            *v = w * (std::f32::consts::TAU * hz * i as f32 / RATE as f32).sin();
        }
        let (l, _) = render(&mut *n, &(x.clone(), x), &[], 128);
        // The energy centre of the first return (before 1.5 Length).
        let (mut num, mut den) = (0.0f64, 0.0f64);
        for (i, v) in l.iter().enumerate().take(2800).skip(1000) {
            num += i as f64 * f64::from(v * v);
            den += f64::from(v * v);
        }
        num / den
    };
    let (lo, hi) = (arrival(300.0), arrival(3000.0));
    assert!(lo > hi + 30.0, "300 Hz at {lo}, 3 kHz at {hi}");
}

#[test]
fn tape_and_multitap_echo_at_their_times() {
    let mut x = vec![0.0f32; 48_000];
    x[0] = 1.0;
    let mut t = make("mpfree.fx.tape_delay");
    for (p, v) in [
        (ids::tape::TIME, 250.0),
        (ids::tape::WOW, 0.0),
        (ids::tape::FLUTTER, 0.0),
        (ids::tape::FEEDBACK, 0.0),
        (ids::tape::MIX, 100.0),
        (ids::tape::LOW_CUT, 20.0),
        (ids::tape::TONE, 16_000.0),
    ] {
        t.set_param(p, v);
    }
    t.reset();
    let (l, _) = render(&mut *t, &(x.clone(), x.clone()), &[], 128);
    let at = (0..l.len())
        .max_by(|&a, &b| l[a].abs().total_cmp(&l[b].abs()))
        .unwrap();
    assert!((at as i64 - 12_000).abs() < 30, "tape echo at {at}");

    // At 120 BPM a sixteenth is 6000 frames: taps at 3, 6, 10 and 16.
    let mut m = make("mpfree.fx.multitap");
    m.set_param(ids::multitap::MIX, 100.0);
    m.set_param(ids::multitap::FEEDBACK, 0.0);
    m.set_param(ids::multitap::LOW_CUT, 20.0);
    m.reset();
    let mut x = vec![0.0f32; 110_000];
    x[0] = 1.0;
    let (l, r) = render(&mut *m, &(x.clone(), x), &[], 128);
    for (tap, six) in [3usize, 6, 10, 16].into_iter().enumerate() {
        let want = six * 6000;
        let near = |y: &[f32]| peak(&y[want - 8..want + 8]);
        assert!(near(&l).max(near(&r)) > 0.2, "tap {tap} at {want}");
    }
}
