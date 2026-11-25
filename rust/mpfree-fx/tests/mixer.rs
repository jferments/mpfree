//! Mixer tests: sums and the pan law against a standalone Master safety,
//! nested submixes, sends before and after the fader, output crossfades,
//! solo and mute, latency compensation (with live inputs left alone),
//! per-insert sidechain keys from a muted kick, capture points, input
//! monitoring, edits while playing, the NaN guard and the same output for
//! any block size with everything moving.

use std::f32::consts::{FRAC_PI_2, PI};

use mpfree_fx::ids::mixer::{self as mx, monitor, output, source};
use mpfree_fx::mpfree_node::{Ctx, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Timed, MAX_BLOCK};
use mpfree_fx::*;

const RATE: u32 = 48_000;

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

/// A pure delay that reports its latency, like a look-ahead effect.
struct Lag {
    n: usize,
    buf: [Vec<f32>; 2],
    w: usize,
}

impl Lag {
    fn boxed(n: usize) -> Box<dyn Node> {
        Box::new(Lag {
            n,
            buf: [vec![0.0; n + 1], vec![0.0; n + 1]],
            w: 0,
        })
    }
}

static LAG: NodeInfo = NodeInfo {
    id: "test.lag",
    name: "Lag",
    kind: Kind::Effect,
};
static GAIN: NodeInfo = NodeInfo {
    id: "test.gain",
    name: "Gain",
    kind: Kind::Effect,
};

impl Node for Lag {
    fn info(&self) -> &'static NodeInfo {
        &LAG
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
        for b in &mut self.buf {
            b.fill(0.0);
        }
    }
    fn process(&mut self, _ctx: &Ctx, _events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        let size = self.n + 1;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            self.buf[0][self.w] = *l;
            self.buf[1][self.w] = *r;
            let rd = (self.w + size - self.n) % size;
            *l = self.buf[0][rd];
            *r = self.buf[1][rd];
            self.w = (self.w + 1) % size;
        }
    }
    fn latency(&self) -> u32 {
        self.n as u32
    }
}

/// Multiplies by a fixed gain.
struct Gain(f32);

impl Node for Gain {
    fn info(&self) -> &'static NodeInfo {
        &GAIN
    }
    fn params(&self) -> &'static [ParamInfo] {
        &[]
    }
    fn set_param(&mut self, _id: u16, _value: f32) {}
    fn param(&self, _id: u16) -> f32 {
        f32::NAN
    }
    fn prepare(&mut self, _rate: u32, _max_block: usize) {}
    fn reset(&mut self) {}
    fn process(&mut self, _ctx: &Ctx, _events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        for v in left.iter_mut().chain(right.iter_mut()) {
            *v *= self.0;
        }
    }
}

type Stereo = (Vec<f32>, Vec<f32>);

fn sine(hz: f32, amp: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| amp * (2.0 * PI * hz * i as f32 / RATE as f32).sin())
        .collect()
}

fn mono(x: Vec<f32>) -> Stereo {
    (x.clone(), x)
}

fn impulse(at: usize, frames: usize) -> Stereo {
    let mut x = vec![0.0; frames];
    x[at] = 1.0;
    mono(x)
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| (v * v) as f64).sum::<f64>() / x.len().max(1) as f64).sqrt() as f32
}

/// What a mixer is fed and told while it plays.
#[derive(Default)]
struct Script {
    feeds: Vec<(StripId, Stereo)>,
    line: Option<Stereo>,
    events: Vec<(usize, StripId, u16, f32)>,
    /// Captures to collect, by slot.
    caps: Vec<usize>,
}

struct Run {
    out: Stereo,
    caps: Vec<Stereo>,
}

fn play(m: &mut Mixer, s: &Script, frames: usize, next_block: &mut dyn FnMut() -> usize) -> Run {
    play_each(m, s, frames, next_block, &mut |_| {})
}

/// [`play`], calling `before` on the mixer before each block.
fn play_each(
    m: &mut Mixer,
    s: &Script,
    frames: usize,
    next_block: &mut dyn FnMut() -> usize,
    before: &mut dyn FnMut(&mut Mixer),
) -> Run {
    let mut out = (vec![0.0; frames], vec![0.0; frames]);
    let mut caps = vec![(Vec::new(), Vec::new()); s.caps.len()];
    let mut at = 0;
    while at < frames {
        let len = next_block()
            .clamp(1, MAX_BLOCK.min(m.max_block()))
            .min(frames - at);
        for &(f, strip, id, value) in &s.events {
            if f >= at && f < at + len {
                assert!(m.queue(
                    strip,
                    Timed::new((f - at) as u32, NodeEvent::Param { id, value })
                ));
            }
        }
        for (strip, (l, r)) in &s.feeds {
            let (il, ir) = m.strip_input(*strip).unwrap();
            for i in 0..len {
                il[i] += l[at + i];
                ir[i] += r[at + i];
            }
        }
        if let Some((l, r)) = &s.line {
            let (hl, hr) = m.hw_input();
            hl[..len].copy_from_slice(&l[at..at + len]);
            hr[..len].copy_from_slice(&r[at..at + len]);
        }
        let ctx = Ctx::new(RATE, at as u64);
        before(m);
        m.process(&ctx, &mut out.0[at..at + len], &mut out.1[at..at + len]);
        for (c, &slot) in caps.iter_mut().zip(&s.caps) {
            let (l, r) = m.capture(slot).unwrap();
            c.0.extend_from_slice(l);
            c.1.extend_from_slice(r);
        }
        at += len;
    }
    Run { out, caps }
}

fn blocks(n: usize) -> impl FnMut() -> usize {
    move || n
}

/// What Master safety alone makes of `x`.
fn safety(x: &Stereo) -> Stereo {
    let mut s = MasterSafety::new();
    s.prepare(RATE, MAX_BLOCK);
    let (mut l, mut r) = x.clone();
    let mut at = 0;
    while at < l.len() {
        let n = (l.len() - at).min(512);
        s.process(
            &Ctx::new(RATE, at as u64),
            &[],
            &mut l[at..at + n],
            &mut r[at..at + n],
        );
        at += n;
    }
    (l, r)
}

fn close(name: &str, a: &Stereo, b: &Stereo, tol: f32) {
    assert_eq!(a.0.len(), b.0.len(), "{name}: lengths");
    for ch in 0..2 {
        let (x, y) = if ch == 0 { (&a.0, &b.0) } else { (&a.1, &b.1) };
        for i in 0..x.len() {
            assert!(
                (x[i] - y[i]).abs() <= tol,
                "{name}: side {ch}, frame {i}: {} vs {}",
                x[i],
                y[i]
            );
        }
    }
}

fn mixer(cfg: MixerConfig) -> Mixer {
    Mixer::new(cfg, RATE, 512)
}

fn small() -> MixerConfig {
    MixerConfig {
        inputs: 2,
        pads: 4,
        tracks: 4,
        slots: 4,
        max_latency_ms: 50.0,
    }
}

fn sum(parts: &[(&Stereo, f32, f32)]) -> Stereo {
    let n = parts[0].0 .0.len();
    let mut out = (vec![0.0; n], vec![0.0; n]);
    for (x, gl, gr) in parts {
        for i in 0..n {
            out.0[i] += x.0[i] * gl;
            out.1[i] += x.1[i] * gr;
        }
    }
    out
}

#[test]
fn tracks_sum_at_unity_with_the_balance_law() {
    let n = 9600;
    let a = mono(sine(220.0, 0.2, n));
    let b = mono(sine(330.0, 0.2, n));
    let c = (sine(440.0, 0.2, n), sine(550.0, 0.1, n));
    let mut m = mixer(small());
    m.set_now(StripId::Track(1), mx::VOLUME, -6.0);
    m.set_now(StripId::Track(1), mx::PAN, 50.0);
    m.set_now(StripId::Track(2), mx::PAN, -100.0);
    let s = Script {
        feeds: vec![
            (StripId::Track(0), a.clone()),
            (StripId::Track(1), b.clone()),
            (StripId::Track(2), c.clone()),
        ],
        ..Default::default()
    };
    let got = play(&mut m, &s, n, &mut blocks(128)).out;
    let g = db_to_gain(-6.0);
    let far = (FRAC_PI_2 * 0.5).cos();
    let want = safety(&sum(&[(&a, 1.0, 1.0), (&b, g * far, g), (&c, 1.0, 0.0)]));
    close("unity and balance", &got, &want, 1e-6);
    // Master safety's: its 1.5 ms look-ahead, its true-peak detector's 15
    // frames and its soft clip's 4.
    assert_eq!(m.latency(), 72 + 15 + 4);
}

#[test]
fn submixes_nest_and_returns_get_sends_before_and_after_the_fader() {
    let n = 9600;
    let a = mono(sine(220.0, 0.2, n));
    let b = mono(sine(330.0, 0.2, n));
    let mut m = mixer(small());
    // Track 0 to submix 1, submix 1 to submix 3, submix 3 to the master.
    m.set_now(StripId::Track(0), mx::OUTPUT, output::bus(0));
    m.set_now(StripId::Bus(0), mx::OUTPUT, output::bus(2));
    m.set_now(StripId::Bus(2), mx::VOLUME, -6.0);
    // A submix cannot go down: submix 3 to submix 2 falls back to the master.
    assert_eq!(m.destination(StripId::Bus(0)), Some(StripId::Bus(2)));
    m.set_now(StripId::Bus(1), mx::OUTPUT, output::bus(0));
    assert_eq!(m.destination(StripId::Bus(1)), Some(StripId::Master));
    // Track 1: fader all the way down, a pre-fader send to A at -6 dB and a
    // post-fader send to B (silent, as the fader is down).
    m.set_now(StripId::Track(1), mx::VOLUME, SILENT_DB);
    m.set_now(StripId::Track(1), mx::send(0), -6.0);
    m.set_now(StripId::Track(1), mx::send_pre(0), 1.0);
    m.set_now(StripId::Track(1), mx::send(1), 0.0);
    m.set_now(StripId::Return(0), mx::VOLUME, -3.0);
    let s = Script {
        feeds: vec![
            (StripId::Track(0), a.clone()),
            (StripId::Track(1), b.clone()),
        ],
        ..Default::default()
    };
    let got = play(&mut m, &s, n, &mut blocks(100)).out;
    let want = safety(&sum(&[
        (&a, db_to_gain(-6.0), db_to_gain(-6.0)),
        (&b, db_to_gain(-9.0), db_to_gain(-9.0)),
    ]));
    close("nested submixes and sends", &got, &want, 1e-5);
}

#[test]
fn changing_an_output_crossfades_with_no_dip_or_click() {
    let n = 24_000;
    let a = mono(sine(200.0, 0.5, n));
    let reference = {
        let mut m = mixer(small());
        play(
            &mut m,
            &Script {
                feeds: vec![(StripId::Track(0), a.clone())],
                ..Default::default()
            },
            n,
            &mut blocks(128),
        )
        .out
    };
    let mut m = mixer(small());
    let events = vec![
        (4000, StripId::Track(0), mx::OUTPUT, output::bus(3)),
        (4100, StripId::Track(0), mx::OUTPUT, output::bus(1)),
        (4150, StripId::Track(0), mx::OUTPUT, output::NONE),
        (9000, StripId::Track(0), mx::OUTPUT, output::MASTER),
    ];
    assert!(m.set_capture(0, Some((StripId::Master, CapturePoint::BeforeFader))));
    let run = play(
        &mut m,
        &Script {
            feeds: vec![(StripId::Track(0), a.clone())],
            events,
            caps: vec![0],
            ..Default::default()
        },
        n,
        &mut blocks(128),
    );
    let got = run.out;
    // Between equal paths the crossfade sums to one: no change at all.
    close(
        "crossfade between submixes",
        &(got.0[..4150].to_vec(), got.1[..4150].to_vec()),
        &(reference.0[..4150].to_vec(), reference.1[..4150].to_vec()),
        1e-5,
    );
    // To nowhere: a glide down, then silence; back: a glide up.
    // (Measured before Master safety, whose DC blocker rings on a little.)
    assert!(peak(&run.caps[0].0[7000..9000]) == 0.0);
    // (Master safety's DC blocker still carries a trace of the gap.)
    assert!(close_to(&got.0[14_000..], &reference.0[14_000..], 1e-3));
    // No step bigger than the sine itself makes.
    let step = got
        .0
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(
        step < 0.5 * 2.0 * PI * 200.0 / RATE as f32 * 1.05,
        "largest step {step}"
    );
}

fn close_to(a: &[f32], b: &[f32], tol: f32) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
}

#[test]
fn mute_and_solo_land_on_their_frame_and_glide() {
    let n = 24_000;
    let a = mono(sine(300.0, 0.3, n));
    let b = mono(sine(450.0, 0.3, n));
    let mut m = mixer(small());
    m.set_now(StripId::Track(1), mx::OUTPUT, output::bus(0));
    let events = vec![
        (6000, StripId::Track(0), mx::MUTE, 1.0),
        (12_000, StripId::Track(0), mx::MUTE, 0.0),
        // Soloing the submix keeps track 1 (it feeds it) and cuts track 0.
        (12_000, StripId::Bus(0), mx::SOLO, 1.0),
    ];
    let s = Script {
        feeds: vec![
            (StripId::Track(0), a.clone()),
            (StripId::Track(1), b.clone()),
        ],
        events,
        ..Default::default()
    };
    let got = play(&mut m, &s, n, &mut blocks(256)).out;
    let both = safety(&sum(&[(&a, 1.0, 1.0), (&b, 1.0, 1.0)]));
    let lat = m.latency() as usize;
    // Untouched up to the mute's frame (plus the output's latency).
    assert!(close_to(&got.0[..6000 + lat], &both.0[..6000 + lat], 1e-6));
    // Then track 0 is gone once the glide settles, and stays gone while the
    // submix is soloed.
    // (Master safety's DC blocker takes a moment to forget track 0.)
    let only_b = safety(&sum(&[(&b, 1.0, 1.0), (&a, 0.0, 0.0)]));
    assert!(close_to(
        &got.0[8000..11_000],
        &only_b.0[8000..11_000],
        5e-3
    ));
    assert!(close_to(&got.0[16_000..], &only_b.0[16_000..], 5e-3));
    assert!(
        m.is_audible(StripId::Track(1))
            && !m.is_audible(StripId::Track(0))
            && m.is_audible(StripId::Return(0))
    );
}

#[test]
fn latency_is_compensated_except_for_live_inputs() {
    let n = 4800;
    let mut m = mixer(small());
    // A 100-frame look-ahead on track 0 and a 30-frame one on pad 0, which
    // goes to track 2 (so every track waits 30 frames for it); track 1 is
    // dry and sends to return A, which has a 10-frame look-ahead. Input 0
    // has 40 frames of its own and goes straight to the master.
    m.chain_mut(StripId::Track(0))
        .unwrap()
        .push(Lag::boxed(100));
    m.chain_mut(StripId::Pad(0)).unwrap().push(Lag::boxed(30));
    m.chain_mut(StripId::Return(0))
        .unwrap()
        .push(Lag::boxed(10));
    m.chain_mut(StripId::Input(0)).unwrap().push(Lag::boxed(40));
    m.set_now(StripId::Pad(0), mx::OUTPUT, output::TRACK);
    m.set_now(StripId::Pad(0), mx::OUTPUT_TRACK, 2.0);
    m.set_now(StripId::Input(0), mx::MONITOR, monitor::ON);
    m.set_now(StripId::Track(1), mx::send(0), 0.0);
    // Run once so the delays are worked out.
    let _ = play(&mut m, &Script::default(), 256, &mut blocks(256));
    let safety_lat = MasterSafety::new().latency() as usize;
    let at = 1000;
    let probe = |strip: StripId| {
        let mut m2 = mixer(small());
        m2.chain_mut(StripId::Track(0))
            .unwrap()
            .push(Lag::boxed(100));
        m2.chain_mut(StripId::Pad(0)).unwrap().push(Lag::boxed(30));
        m2.chain_mut(StripId::Return(0))
            .unwrap()
            .push(Lag::boxed(10));
        m2.chain_mut(StripId::Input(0))
            .unwrap()
            .push(Lag::boxed(40));
        m2.set_now(StripId::Pad(0), mx::OUTPUT, output::TRACK);
        m2.set_now(StripId::Pad(0), mx::OUTPUT_TRACK, 2.0);
        m2.set_now(StripId::Input(0), mx::MONITOR, monitor::ON);
        m2.set_now(StripId::Track(1), mx::send(0), 0.0);
        let s = if strip == StripId::Input(0) {
            Script {
                line: Some(impulse(at, n)),
                ..Default::default()
            }
        } else {
            Script {
                feeds: vec![(strip, impulse(at, n))],
                ..Default::default()
            }
        };
        let out = play(&mut m2, &s, n, &mut blocks(64)).out.0;
        let lat = m2.latency() as usize;
        let hits: Vec<usize> = (0..n).filter(|&i| out[i].abs() > 0.05).collect();
        (hits, lat)
    };
    // The slowest path: return A's 10 frames after track 0's 100, after
    // the 30 every track waits for pad 0.
    assert_eq!(m.latency() as usize, 140 + safety_lat);
    for strip in [
        StripId::Track(0),
        StripId::Track(1),
        StripId::Track(2),
        StripId::Pad(0),
    ] {
        let (hits, lat) = probe(strip);
        assert_eq!(lat, 140 + safety_lat);
        // Every path, dry or sent, lands on the same frame.
        assert!(
            !hits.is_empty() && hits.iter().all(|&h| h.abs_diff(at + lat) <= 1),
            "{strip:?}: {hits:?}"
        );
    }
    // The live input only waits for its own inserts and Master safety.
    let (hits, _) = probe(StripId::Input(0));
    assert!(
        !hits.is_empty() && hits.iter().all(|&h| h.abs_diff(at + 40 + safety_lat) <= 1),
        "input: {hits:?}"
    );
    assert!(!m.stats().misaligned);
}

#[test]
fn a_muted_kick_keys_a_compressor_on_another_track() {
    let n = 48_000;
    // Track 3: a kick every 0.25 s, muted. Track 0: a steady pad with a
    // compressor keyed from track 3 (a later track: the mixer reorders).
    let kick: Vec<f32> = (0..n)
        .map(|i| {
            if i % 12_000 < 2400 {
                0.9 * (2.0 * PI * 60.0 * i as f32 / RATE as f32).sin()
            } else {
                0.0
            }
        })
        .collect();
    let pad = mono(sine(440.0, 0.1, n));
    let mut m = mixer(small());
    {
        use mpfree_fx::ids::compressor::*;
        let c = m.chain_mut(StripId::Track(0)).unwrap();
        c.push(Box::new(Compressor::new()));
        c.set_param(0, THRESHOLD, -30.0);
        c.set_param(0, RATIO, 10.0);
        c.set_param(0, ATTACK, 1.0);
        c.set_param(0, RELEASE, 50.0);
        c.set_param(0, SIDECHAIN, 1.0);
    }
    assert!(m.set_key(StripId::Track(0), 0, Some(StripId::Track(3))));
    assert_eq!(m.key(StripId::Track(0), 0), Some(StripId::Track(3)));
    // A key from a later stage, or a loop, is refused.
    m.chain_mut(StripId::Track(3))
        .unwrap()
        .push(Box::new(Compressor::new()));
    assert!(!m.set_key(StripId::Track(3), 0, Some(StripId::Track(0))));
    assert!(!m.set_key(StripId::Track(3), 0, Some(StripId::Bus(0))));
    assert!(!m.set_key(StripId::Track(0), 5, Some(StripId::Track(1))));
    assert_eq!(m.key(StripId::Track(3), 0), None);
    m.set_now(StripId::Track(3), mx::MUTE, 1.0);
    let s = Script {
        feeds: vec![
            (StripId::Track(0), pad.clone()),
            (StripId::Track(3), mono(kick)),
        ],
        ..Default::default()
    };
    let out = play(&mut m, &s, n, &mut blocks(128)).out.0;
    let lat = m.latency() as usize;
    // Ducked under each kick, back up between them; the kick itself is not heard.
    let under = rms(&out[12_000 + lat + 500..12_000 + lat + 2000]);
    let between = rms(&out[12_000 + lat + 8000..12_000 + lat + 11_000]);
    assert!(between > 0.06, "between kicks {between}");
    assert!(under < between * 0.3, "under the kick {under} vs {between}");
    assert!(peak(&out) < 0.2, "the muted kick is not heard");
}

/// The sidechain list greys a strip for the same reasons `set_key`
/// refuses it, and says which.
#[test]
fn the_sidechain_refusals_match_what_set_key_refuses() {
    use mpfree_fx::sidechain::Refusal;
    let mut m = mixer(small());
    for t in [0, 1, 2] {
        m.chain_mut(StripId::Track(t))
            .unwrap()
            .push(Box::new(Compressor::new()));
    }
    for b in [0, 1] {
        m.chain_mut(StripId::Bus(b))
            .unwrap()
            .push(Box::new(Compressor::new()));
    }
    let (t0, t1, t2) = (StripId::Track(0), StripId::Track(1), StripId::Track(2));
    assert_eq!(m.sidechain_refusal(t0, t0), Some(Refusal::Itself));
    assert_eq!(
        m.sidechain_refusal(t0, StripId::Bus(0)),
        Some(Refusal::Later)
    );
    assert_eq!(
        m.sidechain_refusal(t0, StripId::Master),
        Some(Refusal::Later)
    );
    assert_eq!(
        m.sidechain_refusal(StripId::Bus(0), StripId::Bus(1)),
        Some(Refusal::HigherSubmix)
    );
    assert_eq!(m.sidechain_refusal(StripId::Bus(1), StripId::Bus(0)), None);
    assert_eq!(m.sidechain_refusal(t0, StripId::Pad(0)), None);
    assert_eq!(m.sidechain_refusal(t0, StripId::Input(0)), None);
    // Track 1 hears track 0, and track 2 hears track 1: track 0 can hear
    // neither of them (a loop through one or two strips), but track 2
    // still can hear track 0.
    assert_eq!(m.sidechain_refusal(t1, t0), None);
    assert!(m.set_key(t1, 0, Some(t0)));
    assert!(m.set_key(t2, 0, Some(t1)));
    assert_eq!(m.sidechain_refusal(t0, t1), Some(Refusal::Loop));
    assert_eq!(m.sidechain_refusal(t0, t2), Some(Refusal::Loop));
    assert!(!m.set_key(t0, 0, Some(t1)));
    assert!(!m.set_key(t0, 0, Some(t2)));
    assert_eq!(m.sidechain_refusal(t2, t0), None);
    for (s, k) in [(t0, StripId::Bus(0)), (StripId::Bus(0), StripId::Bus(1))] {
        assert!(m.sidechain_refusal(s, k).is_some());
        assert!(!m.set_key(s, 0, Some(k)));
    }
    assert_eq!(Refusal::Loop.text(), "it listens to this one");
}

/// A strip's inserts move onto an empty strip of the same stage with their
/// keys (a kit's strip settings following it to its track), and play
/// there from the next block; a move that would break something is
/// refused and changes nothing.
#[test]
fn move_chain_takes_the_inserts_and_their_keys_to_an_empty_strip() {
    let mut m = mixer(small());
    let (t0, t1, t2, t3) = (
        StripId::Track(0),
        StripId::Track(1),
        StripId::Track(2),
        StripId::Track(3),
    );
    let c = m.chain_mut(t0).unwrap();
    c.push(Box::new(Gain(0.5)));
    c.push(Box::new(Compressor::new()));
    assert!(m.set_key(t0, 1, Some(StripId::Pad(0))));
    m.chain_mut(t2).unwrap().push(Box::new(Gain(1.0)));
    // Onto a strip with inserts, another stage, or itself: no.
    assert!(!m.move_chain(t0, t2));
    assert!(!m.move_chain(t0, StripId::Bus(0)));
    assert!(!m.move_chain(t0, t0));
    assert!(m.move_chain(t0, t1));
    assert_eq!(m.chain(t0).unwrap().len(), 0);
    assert_eq!(m.chain(t1).unwrap().len(), 2);
    assert_eq!(m.key(t1, 1), Some(StripId::Pad(0)), "the key moves along");
    // It plays there from the next block, as if built there.
    let n = 2048;
    let s = Script {
        feeds: vec![(t1, mono(sine(200.0, 0.4, n)))],
        ..Default::default()
    };
    let mut built = mixer(small());
    let c = built.chain_mut(t1).unwrap();
    c.push(Box::new(Gain(0.5)));
    c.push(Box::new(Compressor::new()));
    assert!(built.set_key(t1, 1, Some(StripId::Pad(0))));
    built.chain_mut(t2).unwrap().push(Box::new(Gain(1.0)));
    let want = play(&mut built, &s, n, &mut blocks(256)).out;
    let got = play(&mut m, &s, n, &mut blocks(256)).out;
    assert!(peak(&want.0) > 0.05);
    assert_eq!(got, want);
    // A strip another is keyed from stays put; so does one with an edit
    // waiting.
    assert!(m.set_key(t2, 0, Some(t1)));
    assert!(!m.move_chain(t1, t3));
    assert!(m.set_key(t2, 0, None));
    assert!(m.edit(Edit::Remove { strip: t1, at: 0 }).is_ok());
    assert!(!m.move_chain(t1, t3));
    assert_eq!(m.chain(t1).unwrap().len(), 2);
}

#[test]
fn capture_points_take_what_they_say() {
    let n = 4096;
    let line = (sine(500.0, 0.4, n), sine(700.0, 0.2, n));
    let mut m = mixer(small());
    m.chain_mut(StripId::Input(0))
        .unwrap()
        .push(Box::new(Gain(0.5)));
    m.set_now(StripId::Input(0), mx::VOLUME, -6.0);
    m.set_now(StripId::Input(0), mx::MONITOR, monitor::ON);
    assert!(m.set_capture(0, Some((StripId::Input(0), CapturePoint::BeforeInserts))));
    assert!(m.set_capture(1, Some((StripId::Input(0), CapturePoint::BeforeFader))));
    assert!(m.set_capture(2, Some((StripId::Input(0), CapturePoint::AfterFader))));
    assert!(m.set_capture(3, Some((StripId::Master, CapturePoint::AfterSafety))));
    assert!(m.set_capture(4, Some((StripId::Master, CapturePoint::BeforeFader))));
    assert!(!m.set_capture(5, Some((StripId::Track(0), CapturePoint::AfterSafety))));
    assert!(!m.set_capture(CAPTURES, None));
    assert!(m.capture(6).is_none());
    let s = Script {
        line: Some(line.clone()),
        caps: vec![0, 1, 2, 3, 4],
        ..Default::default()
    };
    let r = play(&mut m, &s, n, &mut blocks(200));
    let g = db_to_gain(-6.0);
    close("before inserts", &r.caps[0], &line, 0.0);
    close("before fader", &r.caps[1], &sum(&[(&line, 0.5, 0.5)]), 1e-7);
    close(
        "after fader",
        &r.caps[2],
        &sum(&[(&line, 0.5 * g, 0.5 * g)]),
        1e-6,
    );
    close("after safety", &r.caps[3], &r.out, 0.0);
    close("master before safety", &safety(&r.caps[4]), &r.out, 1e-6);
    assert_eq!(m.capture_latency(0), Some(0));
    assert_eq!(m.capture_latency(3), Some(m.latency()));
}

/// Plays tracks 0 to 2 (track 1 behind a look-ahead insert, so the others
/// wait for it) into the master, track 0 moving to a submix and back on
/// the way, with capture 0 at `from` (None: the master's way in too) and
/// capture 1 at the master's way in. `quiet` leaves track 1 silent.
fn three_tracks(from: Option<CaptureFrom>, quiet: bool, n: usize) -> Run {
    let mut m = mixer(small());
    m.chain_mut(StripId::Track(1)).unwrap().push(Lag::boxed(37));
    m.set_now(StripId::Track(1), mx::VOLUME, -4.0);
    m.set_now(StripId::Track(2), mx::PAN, 60.0);
    m.set_now(StripId::Bus(0), mx::VOLUME, -2.0);
    let master_in = (StripId::Master, CapturePoint::BeforeInserts);
    match from {
        Some(_) => assert!(m.set_capture_from(0, from)),
        None => assert!(m.set_capture(0, Some(master_in))),
    }
    assert!(m.set_capture(1, Some(master_in)));
    let a = mono(sine(220.0, 0.3, n));
    let b = (sine(330.0, 0.3, n), sine(440.0, 0.2, n));
    let c = mono(sine(550.0, 0.2, n));
    let b = if quiet {
        (vec![0.0; n], vec![0.0; n])
    } else {
        b
    };
    let s = Script {
        feeds: vec![
            (StripId::Track(0), a),
            (StripId::Track(1), b),
            (StripId::Track(2), c),
        ],
        events: vec![
            (2000, StripId::Track(0), mx::OUTPUT, output::bus(0)),
            (5000, StripId::Track(0), mx::OUTPUT, output::MASTER),
        ],
        caps: vec![0, 1],
        ..Default::default()
    };
    play(&mut m, &s, n, &mut blocks(100))
}

#[test]
fn the_master_without_a_track_is_a_second_sum_not_a_subtraction() {
    let n = 8000;
    let mut m = mixer(small());
    let from = m
        .capture_for(
            (StripId::Master, CapturePoint::BeforeInserts),
            StripId::Track(1),
        )
        .unwrap();
    assert_eq!(from.without, Some(StripId::Track(1)));
    let got = three_tracks(Some(from), false, n);
    let want = three_tracks(None, true, n);
    // To the sample, through the submix and the glides: the same sum the
    // master makes when track 1 is silent.
    close("master without track 1", &got.caps[0], &want.caps[1], 0.0);
    // The master itself still hears track 1 (0.3 at -4 dB, after the
    // first glide), and nothing else changed.
    let track1: Vec<f32> = (got.caps[1].0.iter().zip(&got.caps[0].0))
        .map(|(a, b)| a - b)
        .collect();
    assert!((rms(&track1[1000..]) - 0.3 * db_to_gain(-4.0) / 2f32.sqrt()).abs() < 0.01);
    let plain = three_tracks(None, false, n);
    close("the master is as it was", &got.out, &plain.out, 0.0);
    close("its way in too", &got.caps[1], &plain.caps[1], 0.0);
    assert_eq!(m.capture_latency(0), None);
    m.chain_mut(StripId::Track(1)).unwrap().push(Lag::boxed(37));
    assert!(m.set_capture_from(0, Some(from)));
    assert!(m.set_capture(1, Some((StripId::Master, CapturePoint::BeforeInserts))));
    let (mut l, mut r) = (vec![0.0; 64], vec![0.0; 64]);
    m.process(&Ctx::new(RATE, 0), &mut l, &mut r);
    assert_eq!(m.capture_latency(0), Some(37));
    assert_eq!(m.capture_latency(0), m.capture_latency(1));
    // Pointed plainly again, the capture is the whole way in.
    assert!(m.set_capture(0, Some((StripId::Master, CapturePoint::BeforeInserts))));
    let r = play(
        &mut m,
        &Script {
            feeds: vec![(StripId::Track(1), mono(sine(330.0, 0.3, 2000)))],
            caps: vec![0, 1],
            ..Default::default()
        },
        2000,
        &mut blocks(128),
    );
    close("plain again", &r.caps[0], &r.caps[1], 0.0);
    assert!(peak(&r.caps[0].0) > 0.2);
}

#[test]
fn capture_for_answers_on_the_graph() {
    use CaptureRefusal as No;
    let mut m = mixer(small());
    let t = StripId::Track(1);
    let master_in = (StripId::Master, CapturePoint::BeforeInserts);
    let plain = |strip, point| CaptureFrom {
        strip,
        point,
        without: None,
    };
    // Straight into the master: the master's way in without it.
    assert_eq!(
        m.capture_for(master_in, t),
        Ok(CaptureFrom {
            without: Some(t),
            ..plain(StripId::Master, CapturePoint::BeforeInserts)
        })
    );
    for p in [
        CapturePoint::BeforeFader,
        CapturePoint::AfterFader,
        CapturePoint::AfterSafety,
    ] {
        assert_eq!(
            m.capture_for((StripId::Master, p), t),
            Err(No::PastItsWayIn)
        );
    }
    // Another track, a submix it doesn't reach: as they are.
    let other = (StripId::Track(0), CapturePoint::AfterFader);
    assert_eq!(m.capture_for(other, t), Ok(plain(other.0, other.1)));
    let bus = (StripId::Bus(0), CapturePoint::BeforeInserts);
    assert_eq!(m.capture_for(bus, t), Ok(plain(bus.0, bus.1)));
    assert_eq!(
        m.capture_for((t, CapturePoint::BeforeInserts), t),
        Err(No::Itself)
    );
    assert_eq!(
        m.capture_for((StripId::Track(9), CapturePoint::AfterFader), t),
        Err(No::NotInMix)
    );
    assert_eq!(m.capture_for(master_in, StripId::Bus(9)), Err(No::NotInMix));
    assert_eq!(
        m.capture_for((StripId::Track(0), CapturePoint::AfterSafety), t),
        Err(No::NotInMix)
    );
    // Into submix 1, and on to submix 3 and the master.
    m.set_now(t, mx::OUTPUT, output::bus(0));
    m.set_now(StripId::Bus(0), mx::OUTPUT, output::bus(2));
    assert_eq!(
        m.capture_for(master_in, t),
        Err(No::Through(StripId::Bus(0)))
    );
    let bus3 = (StripId::Bus(2), CapturePoint::BeforeInserts);
    assert_eq!(m.capture_for(bus3, t), Err(No::Through(StripId::Bus(0))));
    assert_eq!(
        m.capture_for(bus, t),
        Ok(CaptureFrom {
            without: Some(t),
            ..plain(bus.0, bus.1)
        })
    );
    assert_eq!(
        m.capture_for((StripId::Bus(0), CapturePoint::AfterFader), t),
        Err(No::PastItsWayIn)
    );
    // Back on the master with a send to Return B: its echoes would be in.
    m.set_now(t, mx::OUTPUT, output::MASTER);
    m.set_now(t, mx::send(1), -12.0);
    assert_eq!(m.capture_for(master_in, t), Err(No::Sends(1)));
    assert_eq!(
        m.capture_for((StripId::Return(1), CapturePoint::BeforeInserts), t),
        Err(No::Sends(1))
    );
    // A submix the send doesn't reach is fine; the send down, all is.
    assert_eq!(m.capture_for(bus, t), Ok(plain(bus.0, bus.1)));
    m.set_now(t, mx::send(1), SILENT_DB);
    assert!(m.capture_for(master_in, t).is_ok());
    // A submix that sends: through it, to the master.
    m.set_now(t, mx::OUTPUT, output::bus(0));
    m.set_now(StripId::Bus(0), mx::OUTPUT, output::MASTER);
    m.set_now(StripId::Bus(0), mx::send(0), 0.0);
    assert_eq!(
        m.capture_for(master_in, t),
        Err(No::Through(StripId::Bus(0)))
    );
    // Going nowhere: nothing to leave out.
    m.set_now(t, mx::OUTPUT, output::NONE);
    assert_eq!(
        m.capture_for(master_in, t),
        Ok(plain(master_in.0, master_in.1))
    );
    // Every answer is one the mixer takes; what it can't leave out, it
    // refuses.
    assert!(m.set_capture_from(0, Some(m.capture_for(bus, t).unwrap())));
    let bad = |strip, point, without| CaptureFrom {
        strip,
        point,
        without: Some(without),
    };
    assert!(!m.set_capture_from(
        0,
        Some(bad(StripId::Track(0), CapturePoint::BeforeInserts, t))
    ));
    assert!(!m.set_capture_from(0, Some(bad(StripId::Master, CapturePoint::BeforeFader, t))));
    assert!(!m.set_capture_from(
        0,
        Some(bad(
            StripId::Master,
            CapturePoint::BeforeInserts,
            StripId::Master
        ))
    ));
    assert!(!m.set_capture_from(
        0,
        Some(bad(
            StripId::Master,
            CapturePoint::BeforeInserts,
            StripId::Track(9)
        ))
    ));
    assert!(m.set_capture_from(0, None));
    assert!(m.capture(0).is_none());
    for no in [
        No::NotInMix,
        No::Itself,
        No::PastItsWayIn,
        No::Through(StripId::Bus(0)),
        No::Sends(0),
    ] {
        assert!(!no.text().is_empty());
    }
}

#[test]
fn inputs_follow_their_source_and_monitoring() {
    let n = 9600;
    let line = (sine(500.0, 0.4, n), sine(700.0, 0.2, n));
    let mut m = mixer(small());
    assert!(m.set_capture(0, Some((StripId::Input(0), CapturePoint::BeforeFader))));
    // Auto monitoring, not armed: recorded but not heard.
    let s = Script {
        line: Some(line.clone()),
        caps: vec![0],
        ..Default::default()
    };
    let r = play(&mut m, &s, n, &mut blocks(128));
    assert!(peak(&r.out.0) == 0.0 && peak(&r.out.1) == 0.0);
    close("recorded while not heard", &r.caps[0], &line, 0.0);
    // Armed at frame 3000: heard from there. Source In 1 at 6000: the left
    // side on both.
    let events = vec![
        (3000, StripId::Input(0), mx::ARM, 1.0),
        (6000, StripId::Input(0), mx::SOURCE, source::LEFT),
    ];
    let mut m = mixer(small());
    assert!(m.set_capture(0, Some((StripId::Input(0), CapturePoint::BeforeInserts))));
    let r = play(
        &mut m,
        &Script {
            line: Some(line.clone()),
            events,
            caps: vec![0],
            ..Default::default()
        },
        n,
        &mut blocks(128),
    );
    let lat = m.latency() as usize;
    assert!(peak(&r.out.0[..3000 + lat]) == 0.0);
    assert!(peak(&r.out.0[4000..6000]) > 0.3);
    assert!(close_to(&r.caps[0].1[6000..], &line.0[6000..], 0.0));
    assert!(close_to(&r.caps[0].1[..6000], &line.1[..6000], 0.0));
    // The input meter reads even when the input is not heard.
    let mut m = mixer(small());
    m.set_now(StripId::Input(1), mx::SOURCE, source::RIGHT);
    let _ = play(
        &mut m,
        &Script {
            line: Some(line.clone()),
            ..Default::default()
        },
        2048,
        &mut blocks(128),
    );
    let p = m.take_input_peaks(StripId::Input(1));
    assert!(
        (p[0] - 0.2).abs() < 0.01 && (p[1] - 0.2).abs() < 0.01,
        "{p:?}"
    );
    assert_eq!(m.take_peaks(StripId::Input(1)), [0.0; 2]);
}

/// A busy mixer: inputs, pads into tracks, nested submixes, sends, a keyed
/// compressor, look-ahead, and automation of every kind.
fn busy(seed: u64, n: usize) -> (Mixer, Script) {
    let mut m = mixer(MixerConfig {
        inputs: 2,
        pads: 6,
        tracks: 6,
        slots: 4,
        max_latency_ms: 20.0,
    });
    let mut rng = Lcg(seed);
    m.chain_mut(StripId::Track(0)).unwrap().push(Lag::boxed(37));
    m.chain_mut(StripId::Pad(1)).unwrap().push(Lag::boxed(13));
    m.chain_mut(StripId::Bus(1))
        .unwrap()
        .push(Box::new(Drive::new()));
    m.chain_mut(StripId::Return(0))
        .unwrap()
        .push(Box::new(Delay::new()));
    m.chain_mut(StripId::Return(1))
        .unwrap()
        .push(Box::new(Cloud::new()));
    m.chain_mut(StripId::Master)
        .unwrap()
        .push(Box::new(Eq::new()));
    {
        let c = m.chain_mut(StripId::Track(2)).unwrap();
        c.push(Box::new(Compressor::new()));
        c.set_param(0, ids::compressor::SIDECHAIN, 1.0);
        c.set_param(0, ids::compressor::THRESHOLD, -30.0);
    }
    assert!(m.set_key(StripId::Track(2), 0, Some(StripId::Track(4))));
    m.chain_mut(StripId::Input(0))
        .unwrap()
        .push(Box::new(Chorus::new()));
    m.set_now(StripId::Input(0), mx::MONITOR, monitor::ON);
    let mut feeds = Vec::new();
    for t in 0..6 {
        let x: Vec<f32> = (0..n)
            .map(|i| {
                if (i / 900 + t) % 4 == 0 {
                    0.0
                } else {
                    0.3 * (rng.unit() * 2.0 - 1.0)
                }
            })
            .collect();
        feeds.push((
            StripId::Track(t),
            (x.clone(), x.iter().map(|v| v * 0.7).collect()),
        ));
    }
    for p in 0..3 {
        let x: Vec<f32> = (0..n)
            .map(|i| {
                if i % 3000 < 400 {
                    0.5 * (rng.unit() * 2.0 - 1.0)
                } else {
                    0.0
                }
            })
            .collect();
        feeds.push((StripId::Pad(p), mono(x)));
    }
    let line = (sine(310.0, 0.3, n), sine(470.0, 0.2, n));
    let strips = [
        StripId::Input(0),
        StripId::Input(1),
        StripId::Pad(0),
        StripId::Pad(1),
        StripId::Pad(2),
        StripId::Track(0),
        StripId::Track(1),
        StripId::Track(2),
        StripId::Track(3),
        StripId::Track(4),
        StripId::Track(5),
        StripId::Bus(0),
        StripId::Bus(1),
        StripId::Bus(3),
        StripId::Return(0),
        StripId::Return(1),
        StripId::Master,
    ];
    let mut events = Vec::new();
    for _ in 0..400 {
        let f = rng.next() as usize % n;
        let s = strips[rng.next() as usize % strips.len()];
        let (id, v) = match rng.next() % 9 {
            0 => (mx::VOLUME, -30.0 + 36.0 * rng.unit()),
            1 => (mx::PAN, -100.0 + 200.0 * rng.unit()),
            2 => (mx::MUTE, rng.next().is_multiple_of(4) as u8 as f32),
            3 => (mx::SOLO, rng.next().is_multiple_of(6) as u8 as f32),
            4 => (mx::OUTPUT, (rng.next() % 11) as f32),
            5 => (mx::OUTPUT_TRACK, (rng.next() % 6) as f32),
            6 => (
                mx::send((rng.next() % 4) as usize),
                -40.0 + 40.0 * rng.unit(),
            ),
            7 => (
                mx::send_pre((rng.next() % 4) as usize),
                (rng.next() % 2) as f32,
            ),
            _ => (
                if rng.next().is_multiple_of(2) {
                    mx::SOURCE
                } else {
                    mx::MONITOR
                },
                (rng.next() % 3) as f32,
            ),
        };
        events.push((f, s, id, v));
    }
    events.sort_by_key(|e| e.0);
    (
        m,
        Script {
            feeds,
            line: Some(line),
            events,
            caps: Vec::new(),
        },
    )
}

#[test]
fn same_output_for_any_block_size_with_everything_moving() {
    let n = 30_000;
    let reference = {
        let (mut m, s) = busy(5, n);
        play(&mut m, &s, n, &mut blocks(512)).out
    };
    assert!(peak(&reference.0) > 0.05);
    for size in [1, 7, 64, 128, 333] {
        let (mut m, s) = busy(5, n);
        let got = play(&mut m, &s, n, &mut blocks(size)).out;
        close(&format!("{size}-frame blocks"), &got, &reference, 0.0);
    }
    let (mut m, s) = busy(5, n);
    let mut rng = Lcg(99);
    let got = play(&mut m, &s, n, &mut || 1 + rng.next() as usize % 512).out;
    close("random blocks", &got, &reference, 0.0);
    assert!(got.0.iter().chain(&got.1).all(|v| v.is_finite()));
}

#[test]
fn edits_while_playing_are_click_free_and_hand_back_what_they_take() {
    let n = 48_000;
    let a = mono(sine(150.0, 0.5, n));
    let mut m = mixer(small());
    let s = Script {
        feeds: vec![(StripId::Track(0), a.clone())],
        ..Default::default()
    };
    let mut out = (vec![0.0; n], vec![0.0; n]);
    let mut at = 0;
    let mut step = 0;
    let base = m.latency();
    while at < n {
        let len = 128.min(n - at);
        match at {
            4992 => assert!(m
                .edit(Edit::Insert {
                    strip: StripId::Track(0),
                    at: 0,
                    insert: Insert::new(Lag::boxed(480), RATE, 512)
                })
                .is_ok()),
            19_968 => assert!(m
                .edit(Edit::Replace {
                    strip: StripId::Track(0),
                    at: 0,
                    insert: Insert::new(Box::new(Gain(0.5)), RATE, 512)
                })
                .is_ok()),
            30_080 => {
                assert!(m
                    .edit(Edit::Remove {
                        strip: StripId::Track(0),
                        at: 3
                    })
                    .is_ok());
                assert!(m
                    .edit(Edit::Insert {
                        strip: StripId::Track(0),
                        at: 0,
                        insert: Insert::new(Box::new(Gain(1.0)), RATE, 256)
                    })
                    .is_ok());
            }
            _ => {}
        }
        let (il, ir) = m.track_input(0).unwrap();
        il[..len].copy_from_slice(&s.feeds[0].1 .0[at..at + len]);
        ir[..len].copy_from_slice(&s.feeds[0].1 .1[at..at + len]);
        m.process(
            &Ctx::new(RATE, at as u64),
            &mut out.0[at..at + len],
            &mut out.1[at..at + len],
        );
        if at == 15_000 / 128 * 128 {
            assert_eq!(m.latency(), base + 480);
        }
        at += len;
        step += 1;
    }
    assert!(step > 0);
    assert_eq!(m.edits_pending(), 0);
    // The Lag came back from the Replace; the bad Remove and the insert
    // built for another block size came back too, and were counted.
    let mut back = 0;
    while m.pop_retired().is_some() {
        back += 1;
    }
    assert_eq!(back, 2);
    assert_eq!(m.stats().bad_edits, 2);
    assert_eq!(m.latency(), base);
    // The edits duck the strip: no sample jumps further than the sine and a
    // 10 ms fade can make.
    let biggest = out
        .0
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(biggest < 0.05, "largest step {biggest}");
    // Back at full level at the end, through Gain(0.5).
    assert!((rms(&out.0[40_000..]) - 0.5 * rms(&a.0[40_000..])).abs() < 0.01);
}

#[test]
fn bad_input_is_silenced_and_a_hot_sum_is_caught() {
    let n = 4800;
    let mut m = mixer(small());
    let loud = mono(sine(100.0, 0.9, n));
    let feeds = (0..4).map(|t| (StripId::Track(t), loud.clone())).collect();
    let r = play(
        &mut m,
        &Script {
            feeds,
            ..Default::default()
        },
        n,
        &mut blocks(256),
    );
    assert!(
        peak(&r.out.0) <= 1.0 && peak(&r.out.1) <= 1.0,
        "Master safety holds a sum of 3.6"
    );
    assert!(m.safety().stats().overs > 0);
    let mut bad = loud.clone();
    bad.0[1000] = f32::NAN;
    bad.1[2000] = f32::INFINITY;
    let r = play(
        &mut m,
        &Script {
            feeds: vec![(StripId::Pad(2), bad)],
            ..Default::default()
        },
        n,
        &mut blocks(256),
    );
    assert!(r.out.0.iter().chain(&r.out.1).all(|v| v.is_finite()));
    assert_eq!(m.stats().bad_input, 2);
}

#[test]
fn quiet_strips_wake_on_their_first_frame() {
    // Pads silent for a long time (so they sleep), then one impulse: it
    // arrives, on time, exactly as a pad that never slept.
    let n = 48_000;
    let mut m = mixer(small());
    m.set_now(StripId::Pad(3), mx::OUTPUT, output::TRACK);
    m.set_now(StripId::Pad(3), mx::OUTPUT_TRACK, 1.0);
    m.set_now(StripId::Pad(3), mx::send(2), -6.0);
    let at = 40_001;
    let r = play(
        &mut m,
        &Script {
            feeds: vec![(StripId::Pad(3), impulse(at, n))],
            ..Default::default()
        },
        n,
        &mut blocks(64),
    );
    let want = safety(&sum(&[(
        &impulse(at, n),
        1.0 + db_to_gain(-6.0),
        1.0 + db_to_gain(-6.0),
    )]));
    close("woken pad", &r.out, &want, 1e-6);
}

#[test]
fn params_describe_every_strip() {
    let m = mixer(MixerConfig::default());
    assert_eq!(m.strips().count(), 2 + 32 + 16 + BUSES + RETURNS + 1);
    assert_eq!(BUSES, 8);
    assert_eq!(MixerConfig::default().slots, 8);
    for s in m.strips() {
        for p in strip_params() {
            if mixer_param_applies(s, p.id) {
                assert!(m.param(s, p.id).is_finite(), "{s:?} {}", p.name);
            }
        }
    }
}

#[test]
fn profiling_times_the_mixer_its_strips_and_their_inserts_only_when_on() {
    let n = 4096;
    let mut m = mixer(small());
    m.chain_mut(StripId::Track(1)).unwrap().push(Lag::boxed(3));
    let script = Script {
        feeds: vec![(StripId::Track(1), mono(sine(220.0, 0.3, n)))],
        ..Default::default()
    };
    // Off by default: nothing recorded, and the output is the same either
    // way.
    let off = play(&mut m, &script, n, &mut blocks(128));
    assert!(!m.profiling());
    assert_eq!(m.probe().lifetime().blocks, 0);
    assert_eq!(
        m.strip_probe(StripId::Track(1)).unwrap().lifetime().blocks,
        0
    );
    m.reset();
    m.set_profiling(true);
    let on = play(&mut m, &script, n, &mut blocks(128));
    close("profiled", &on.out, &off.out, 0.0);
    let blocks_run = (n / 128) as u64;
    assert_eq!(m.probe().lifetime().blocks, blocks_run);
    assert_eq!(m.probe().lifetime().frames, n as u64);
    assert_eq!(
        m.strip_probe(StripId::Track(1)).unwrap().lifetime().blocks,
        blocks_run
    );
    assert_eq!(
        m.chain(StripId::Track(1))
            .unwrap()
            .probe(0)
            .unwrap()
            .lifetime()
            .blocks,
        blocks_run
    );
    assert_eq!(m.probe_mut().take_window().blocks, blocks_run);
    assert_eq!(m.probe_mut().take_window().blocks, 0);
    m.set_profiling(false);
    play(&mut m, &script, 1024, &mut blocks(128));
    assert_eq!(m.probe().lifetime().blocks, blocks_run);
}

/// Spending (System status's tracks): each strip's inserts' time counts
/// while on, by strip, strips without inserts never; a take starts again;
/// off, nothing counts; the output is the same either way.
#[test]
fn spending_counts_each_strips_inserts_only_while_on() {
    let n = 2048;
    let mut m = mixer(small());
    m.chain_mut(StripId::Track(1)).unwrap().push(Lag::boxed(3));
    let script = Script {
        feeds: vec![
            (StripId::Track(1), mono(sine(220.0, 0.3, n))),
            (StripId::Track(2), mono(sine(330.0, 0.3, n))),
        ],
        ..Default::default()
    };
    let off = play(&mut m, &script, n, &mut blocks(128));
    let mut seen = Vec::new();
    m.take_spent(|s, ns| seen.push((s, ns)));
    assert!(seen.is_empty(), "nothing while off: {seen:?}");
    m.reset();
    m.set_spending(true);
    let on = play(&mut m, &script, n, &mut blocks(128));
    close("spending", &on.out, &off.out, 0.0);
    m.take_spent(|s, ns| seen.push((s, ns)));
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].0, StripId::Track(1));
    assert!(seen[0].1 > 0);
    seen.clear();
    m.take_spent(|s, ns| seen.push((s, ns)));
    assert!(seen.is_empty(), "a take starts again");
    m.set_spending(false);
    play(&mut m, &script, 512, &mut blocks(128));
    m.take_spent(|s, ns| seen.push((s, ns)));
    assert!(seen.is_empty(), "off again");
}

/// The busy mixer with inserts on more tracks and pad channels, so a lane
/// has work: an EQ and a compressor on tracks 1, 3 and 5, a drive on pads
/// 0 and 2 (track 2 is keyed by track 4, so those two stay).
fn busy_for_lanes(seed: u64, n: usize) -> (Mixer, Script) {
    let (mut m, s) = busy(seed, n);
    for t in [1, 3, 5] {
        let c = m.chain_mut(StripId::Track(t)).unwrap();
        c.push(Box::new(Eq::new()));
        c.set_param(0, ids::eq::gain(1), 4.0);
        c.push(Box::new(Compressor::new()));
    }
    for p in [0, 2] {
        m.chain_mut(StripId::Pad(p))
            .unwrap()
            .push(Box::new(Drive::new()));
    }
    (m, s)
}

#[test]
fn inserts_on_a_second_core_give_the_same_output_bit_for_bit() {
    use std::time::Duration;
    let n = 30_000;
    let reference = {
        let (mut m, s) = busy_for_lanes(5, n);
        play(&mut m, &s, n, &mut blocks(128)).out
    };
    assert!(peak(&reference.0) > 0.05);
    for (name, threaded) in [("on this thread", false), ("on a worker", true)] {
        let (mut m, s) = busy_for_lanes(5, n);
        let cfg = mpfree_lanes::Config {
            threaded,
            ..mpfree_lanes::Config::default()
        };
        assert!(m.set_lane(Some(cfg), Duration::from_secs(1)).is_some());
        assert!(m.lane_on());
        let mut rng = Lcg(3);
        let got = play(&mut m, &s, n, &mut || 1 + rng.next() as usize % 512).out;
        close(name, &got, &reference, 0.0);
        let st = m.stats();
        assert!(st.lane_posts > 0, "{name}: the lane had inserts");
        assert_eq!(st.lane_late, 0, "{name}");
        assert!(m.set_lane(None, Duration::ZERO).is_none());
        assert!(!m.lane_on());
    }
}

/// Plays `s` again until a lane join comes back late, at most 50 times,
/// and gives the late joins it saw. A wait of zero is late only when the
/// lane has not finished by the join: from 1 % to 20 % of joins here, and
/// fewer still with other tests on the cores, so one pass can have none.
fn play_until_late(m: &mut Mixer, s: &Script, n: usize) -> u64 {
    let before = m.stats().lane_late;
    for _ in 0..50 {
        play(m, s, n, &mut blocks(64));
        if m.stats().lane_late > before {
            break;
        }
    }
    m.stats().lane_late - before
}

#[test]
fn a_late_lane_leaves_its_strips_dry_until_their_inserts_come_back() {
    use std::time::Duration;
    let n = 20_000;
    let (mut m, s) = busy_for_lanes(7, n);
    // A wait of zero: a join the lane hasn't finished by is late, as on a
    // lost period.
    let setup = m.set_lane(Some(mpfree_lanes::Config::default()), Duration::ZERO);
    let threaded = setup.is_some_and(|s| s.threaded.is_ok());
    let out = play(&mut m, &s, n, &mut blocks(64)).out;
    assert!(out.0.iter().chain(&out.1).all(|v| v.is_finite()));
    if threaded && m.stats().lane_late == 0 {
        play_until_late(&mut m, &s, n);
    }
    let late = m.stats().lane_late;
    println!(
        "zero wait: {late} late of {} posts (worker thread: {threaded})",
        m.stats().lane_posts
    );
    if threaded {
        assert!(late > 0, "a zero wait is late");
    }
    // A normal wait: the inserts come back and stay.
    m.set_lane(
        Some(mpfree_lanes::Config::default()),
        Duration::from_secs(1),
    );
    let out = play(&mut m, &s, n, &mut blocks(64)).out;
    assert!(out.0.iter().chain(&out.1).all(|v| v.is_finite()));
    assert!(peak(&out.0) > 0.05);
    assert_eq!(m.stats().lane_late, late, "no more late joins");
    for t in [1, 3, 5] {
        assert_eq!(m.chain(StripId::Track(t)).unwrap().len(), 2, "track {t}");
    }
    for p in [0, 2] {
        assert_eq!(m.chain(StripId::Pad(p)).unwrap().len(), 1, "pad {p}");
    }
}

#[test]
fn a_blocks_deadline_holds_for_every_join_in_it_and_only_that_block() {
    use std::time::{Duration, Instant};
    let n = 20_000;
    let reference = {
        let (mut m, s) = busy_for_lanes(9, n);
        play(&mut m, &s, n, &mut blocks(64)).out
    };
    let (mut m, s) = busy_for_lanes(9, n);
    // A wait of zero, which alone makes some joins late (above),
    // and a far deadline given to each block: no join is late, and the
    // output is the same, bit for bit.
    let setup = m.set_lane(Some(mpfree_lanes::Config::default()), Duration::ZERO);
    let threaded = setup.is_some_and(|s| s.threaded.is_ok());
    let far = |m: &mut Mixer| m.set_lane_deadline(Some(Instant::now() + Duration::from_secs(5)));
    let got = play_each(&mut m, &s, n, &mut blocks(64), &mut { far }).out;
    close("a far deadline each block", &got, &reference, 0.0);
    let st = m.stats();
    assert!(st.lane_posts > 0);
    assert_eq!(
        st.lane_late, 0,
        "the block's deadline replaced the zero wait"
    );
    // The deadline holds for one block: without a new one, the zero wait
    // is back.
    if threaded {
        assert!(
            play_until_late(&mut m, &s, n) > 0,
            "a zero wait is late again"
        );
    }
}

/// A mixer whose only way to the master is through the returns: track 0
/// goes nowhere and sends to a delay on return A and the Cloud reverb (Eco)
/// on return B.
fn returns_only(n: usize) -> (Mixer, Script) {
    let mut m = mixer(small());
    m.set_now(StripId::Track(0), mx::OUTPUT, output::NONE);
    m.set_now(StripId::Track(0), mx::send(0), -6.0);
    m.set_now(StripId::Track(0), mx::send(1), -6.0);
    m.chain_mut(StripId::Return(0))
        .unwrap()
        .push(Box::new(Delay::new()));
    // All wet with a pre-delay, settled from the start: the reverb can run
    // a block late from the first block.
    let mut cloud = Cloud::new();
    cloud.set_param(ids::cloud::QUALITY, 2.0);
    cloud.set_param(ids::cloud::MIX, 100.0);
    cloud.set_param(ids::cloud::PRE_DELAY, 20.0);
    cloud.reset();
    m.chain_mut(StripId::Return(1))
        .unwrap()
        .push(Box::new(cloud));
    let mut rng = Lcg(41);
    let x: Vec<f32> = (0..n)
        .map(|i| {
            if (i / 2000) % 3 == 0 {
                0.4 * (rng.unit() * 2.0 - 1.0)
            } else {
                0.0
            }
        })
        .collect();
    let s = Script {
        feeds: vec![(StripId::Track(0), mono(x))],
        line: None,
        events: Vec::new(),
        caps: Vec::new(),
    };
    (m, s)
}

#[test]
fn pipelined_returns_sound_the_same_bit_for_bit() {
    use std::time::Duration;
    let (n, b) = (24_000, 128);
    let reference = {
        let (mut m, s) = returns_only(n);
        play(&mut m, &s, n, &mut blocks(b)).out
    };
    assert!(peak(&reference.0) > 0.01);
    for (name, threaded) in [("on this thread", false), ("on a worker", true)] {
        let (mut m, s) = returns_only(n);
        let cfg = mpfree_lanes::Config {
            threaded,
            ..mpfree_lanes::Config::default()
        };
        assert!(m
            .set_pipelined_returns(Some(cfg), Duration::from_secs(1))
            .is_some());
        assert!(m.pipelined_returns());
        let got = play(&mut m, &s, n, &mut blocks(b)).out;
        // The reverb runs a block late and takes the block back from its
        // pre-delay; the delay (no pre-delay to take it from) stays on
        // time. Together: the reference, bit for bit, from the first block.
        close(name, &got, &reference, 0.0);
        let st = m.stats();
        // Late from the first block (a start) to the last, a shorter one,
        // where it goes back on time (a stop).
        assert_eq!(n % b, 64);
        assert_eq!(st.pipe_posts as usize, n / b - 1, "{name}");
        assert_eq!(st.pipe_starts, 1, "{name}");
        assert_eq!(st.pipe_stops, 1, "{name}");
        assert_eq!(st.pipe_late, 0, "{name}");
        assert_eq!(m.chain(StripId::Return(1)).unwrap().lead(), 0);
        // The inserts are home between blocks.
        assert_eq!(m.chain(StripId::Return(1)).unwrap().len(), 1);
        assert!(m.set_pipelined_returns(None, Duration::ZERO).is_none());
        assert!(!m.pipelined_returns());
        assert_eq!(m.stats().pipe_lost_input, 0);
    }
}

#[test]
fn pipelined_returns_take_insert_controls_and_play_with_everything_else() {
    use std::time::Duration;
    let n = 30_000;
    let (mut m, s) = busy(9, n);
    let c = m.chain_mut(StripId::Return(1)).unwrap();
    c.set_param(0, ids::cloud::MIX, 100.0);
    c.set_param(0, ids::cloud::PRE_DELAY, 30.0);
    assert!(m
        .set_pipelined_returns(
            Some(mpfree_lanes::Config::default()),
            Duration::from_secs(1)
        )
        .is_some());
    let mut rng = Lcg(5);
    // Blocks of any length: the reverb starts and stops around each change.
    let out = play(&mut m, &s, n / 2, &mut || 1 + rng.next() as usize % 512).out;
    assert!(out.0.iter().chain(&out.1).all(|v| v.is_finite()));
    assert!(peak(&out.0) > 0.05);
    let out = play(&mut m, &s, n / 4, &mut blocks(256)).out;
    assert!(out.0.iter().chain(&out.1).all(|v| v.is_finite()));
    assert!(m.stats().pipe_posts > 0);
    assert!(m.stats().pipe_starts > 1);
    // Between blocks the returns' inserts are home: knob turns and bypass
    // reach them.
    assert!(m.queue_insert(
        StripId::Return(0),
        0,
        Timed::new(
            0,
            NodeEvent::Param {
                id: ids::delay::FEEDBACK,
                value: 10.0
            }
        )
    ));
    m.set_on(StripId::Return(1), 0, false);
    let out = play(&mut m, &s, n / 2, &mut blocks(256)).out;
    assert!(out.0.iter().chain(&out.1).all(|v| v.is_finite()));
    let st = m.stats();
    assert!(st.pipe_posts > 0);
    assert_eq!(st.pipe_late, 0);
    assert_eq!(st.dropped_events, 0);
}

#[test]
fn a_late_pipeline_silences_the_returns_until_their_inserts_come_back() {
    use std::time::Duration;
    let n = 20_000;
    let (mut m, s) = returns_only(n);
    let setup = m.set_pipelined_returns(Some(mpfree_lanes::Config::default()), Duration::ZERO);
    let threaded = setup.is_some_and(|s| s.threaded.is_ok());
    let out = play(&mut m, &s, n, &mut blocks(64)).out;
    assert!(out.0.iter().chain(&out.1).all(|v| v.is_finite()));
    let late = m.stats().pipe_late;
    if threaded {
        assert!(late > 0, "a zero wait is late");
    }
    m.set_pipelined_returns(
        Some(mpfree_lanes::Config::default()),
        Duration::from_secs(1),
    );
    let out = play(&mut m, &s, n, &mut blocks(64)).out;
    assert!(peak(&out.0) > 0.01);
    assert_eq!(m.stats().pipe_late, late, "no more late joins");
    assert_eq!(m.chain(StripId::Return(0)).unwrap().len(), 1);
    assert_eq!(m.chain(StripId::Return(1)).unwrap().len(), 1);
}

/// What the next block will have: the last value queued for a control,
/// else the control as it stands, so steps given between two blocks add
/// up (the audio tracks' panel level and pan step from it).
#[test]
fn wanted_is_the_last_value_queued_else_the_value_now() {
    let mut m = mixer(small());
    let t = StripId::Track(1);
    m.set_now(t, mx::VOLUME, -3.0);
    assert_eq!(m.wanted(t, mx::VOLUME), -3.0);
    assert!(m.set(t, mx::VOLUME, -4.0));
    assert!(m.set(t, mx::PAN, 20.0));
    assert!(m.set(t, mx::VOLUME, -5.0));
    assert_eq!(m.param(t, mx::VOLUME), -3.0, "not yet");
    assert_eq!(m.wanted(t, mx::VOLUME), -5.0);
    assert_eq!(m.wanted(t, mx::PAN), 20.0);
    let s = Script::default();
    play(&mut m, &s, 512, &mut blocks(512));
    assert_eq!(m.param(t, mx::VOLUME), -5.0);
    assert_eq!(m.wanted(t, mx::VOLUME), -5.0);
    assert!(m.wanted(StripId::Track(99), mx::VOLUME).is_nan());
}

/// A performance offset on a send (the Play page's Loop group adds its
/// Delay to an audio track's Send A this way): a gain on top of the send's
/// own level as it plays, with the send's glide, the same as the send set
/// to that level; the send's own value is never written, so taking the
/// offset away puts the sound back exactly, and the mixer's answers for a
/// recording's input count the offset.
#[test]
fn a_send_offset_plays_on_top_of_the_send_and_never_writes_it() {
    let n = 9600;
    let a = mono(sine(220.0, 0.2, n));
    let feed = Script {
        feeds: vec![(StripId::Track(0), a.clone())],
        ..Default::default()
    };
    // The same, with Send A moved to `db` on the first frame.
    let to = |db: f32| Script {
        feeds: vec![(StripId::Track(0), a.clone())],
        events: vec![(0, StripId::Track(0), mx::send(0), db)],
        ..Default::default()
    };
    let db = |g: f32| 20.0 * g.log10();
    // Half a unit on Send A, which is off: as the send moved to -6 dB.
    let mut m = mixer(small());
    assert!(m.set_send_add(StripId::Track(0), 0, 0.5));
    assert_eq!(m.send_add(StripId::Track(0), 0), 0.5);
    let wet = play(&mut m, &feed, n, &mut blocks(100)).out;
    let mut r = mixer(small());
    let want = play(&mut r, &to(db(0.5)), n, &mut blocks(100)).out;
    close("with the offset", &wet, &want, 1e-6);
    assert_eq!(m.param(StripId::Track(0), mx::send(0)), SILENT_DB);
    // Counted where the mixer asks what a strip's sound reaches.
    let master = (StripId::Master, CapturePoint::BeforeInserts);
    assert!(m.capture_for(master, StripId::Track(0)).is_err());
    // Taken away: as the send moved back to off, and still off.
    assert!(m.set_send_add(StripId::Track(0), 0, 0.0));
    let back = play(&mut m, &feed, n, &mut blocks(100)).out;
    let want = play(&mut r, &to(SILENT_DB), n, &mut blocks(100)).out;
    close("back at rest", &back, &want, 1e-6);
    assert_eq!(m.param(StripId::Track(0), mx::send(0)), SILENT_DB);
    assert!(m.capture_for(master, StripId::Track(0)).is_ok());
    // On a send of its own: the offset adds to its gain.
    let mut m = mixer(small());
    m.set_now(StripId::Track(0), mx::send(0), -6.0);
    assert!(m.set_send_add(StripId::Track(0), 0, 0.25));
    let both = play(&mut m, &feed, n, &mut blocks(100)).out;
    let mut r = mixer(small());
    r.set_now(StripId::Track(0), mx::send(0), -6.0);
    let want = play(
        &mut r,
        &to(db(db_to_gain(-6.0) + 0.25)),
        n,
        &mut blocks(100),
    )
    .out;
    close("on top of its own", &both, &want, 1e-5);
    assert_eq!(m.param(StripId::Track(0), mx::send(0)), -6.0);
    // Up to +6 dB at most; the master has no sends.
    assert!(m.set_send_add(StripId::Track(0), 0, 9.0));
    assert!(m.send_add(StripId::Track(0), 0) < 2.0);
    assert!(!m.set_send_add(StripId::Master, 0, 0.5));
}

/// The routing check (the audio tracks rethink, phase 6.0.4): whether a
/// render can stand in for its selection with the mix the same, on the
/// plan's table of cases.
#[test]
fn stand_in_answers_the_table_of_cases() {
    use StandCapture as At;
    use StandRefusal as No;
    let (t0, t1, t2) = (StripId::Track(0), StripId::Track(1), StripId::Track(2));
    let (b0, b1) = (StripId::Bus(0), StripId::Bus(1));
    let mut m = mixer(small());
    let run = |m: &mut Mixer| {
        let (mut l, mut r) = (vec![0.0; 64], vec![0.0; 64]);
        m.process(&Ctx::new(RATE, 0), &mut l, &mut r);
    };
    run(&mut m);
    let lat0 = m.stand_in(&[t1], At::BeforeFader).unwrap().latency;

    // One track before its fader: always, copying its channel.
    let one = m.stand_in(&[t1], At::BeforeFader).unwrap();
    assert_eq!(one.copies, Some(t1));
    assert_eq!(one.to, Some(StripId::Master));
    assert_eq!(one.keyed().count(), 0);
    // With sends before and after its fader, still: the new channel
    // copies them.
    m.set_now(t1, mx::send(0), -6.0);
    m.set_now(t1, mx::send(1), -12.0);
    m.set_now(t1, mx::send_pre(1), 1.0);
    assert!(m.stand_in(&[t1], At::BeforeFader).is_ok());
    // After its fader, a send up refuses (the first one up named).
    assert_eq!(m.stand_in(&[t1], At::AfterFader), Err(No::Sends(t1, 0)));
    m.set_now(t1, mx::send(0), SILENT_DB);
    assert_eq!(m.stand_in(&[t1], At::AfterFader), Err(No::Sends(t1, 1)));
    m.set_now(t1, mx::send(1), SILENT_DB);
    let flat = m.stand_in(&[t1], At::AfterFader).unwrap();
    assert_eq!((flat.copies, flat.to), (None, Some(StripId::Master)));

    // A sidechain fed from the track: pointed at the new channel before
    // the fader, refused after it.
    {
        use mpfree_fx::ids::compressor::*;
        let c = m.chain_mut(t0).unwrap();
        c.push(Box::new(Compressor::new()));
        c.set_param(0, SIDECHAIN, 1.0);
    }
    assert!(m.set_key(t0, 0, Some(t1)));
    let keyed = m.stand_in(&[t1], At::BeforeFader).unwrap();
    assert_eq!(keyed.keyed().collect::<Vec<_>>(), vec![(t0, 0)]);
    assert_eq!(
        m.stand_in(&[t1], At::AfterFader),
        Err(No::Keys {
            from: t1,
            keyed: t0
        })
    );
    assert_eq!(
        m.stand_in(&[t1, t2], At::AfterFader),
        Err(No::Keys {
            from: t1,
            keyed: t0
        })
    );
    assert!(m.set_key(t0, 0, None));

    // Several tracks: summed after their faders, only into one place.
    assert_eq!(m.stand_in(&[t1, t2], At::BeforeFader), Err(No::OneOnly));
    assert!(m.stand_in(&[t1, t2], At::AfterFader).is_ok());
    m.set_now(t1, mx::OUTPUT, output::bus(0));
    m.set_now(t2, mx::OUTPUT, output::bus(1));
    assert_eq!(
        m.stand_in(&[t1, t2], At::AfterFader),
        Err(No::Apart(t1, t2))
    );
    m.set_now(t2, mx::OUTPUT, output::bus(0));
    let two = m.stand_in(&[t1, t2], At::AfterFader).unwrap();
    assert_eq!(two.to, Some(b0));

    // A submix's inputs, exactly: it stands in for the submix, taking its
    // output, where the submix went.
    m.set_now(b0, mx::OUTPUT, output::bus(1));
    let bus = m.stand_in(&[t1, t2], At::Bus(b0)).unwrap();
    assert_eq!((bus.copies, bus.to), (None, Some(b1)));
    assert_eq!(m.stand_in(&[t1], At::Bus(b0)), Err(No::NotItsInputs(t2)));
    assert_eq!(
        m.stand_in(&[t0, t1, t2], At::Bus(b0)),
        Err(No::NotItsInputs(t0))
    );
    assert_eq!(m.stand_in(&[t1, t2], At::Bus(t0)), Err(No::NotATrack(t0)));
    // An input of it with a send up refuses, and the submix's own send.
    m.set_now(t2, mx::send(3), -20.0);
    assert_eq!(m.stand_in(&[t1, t2], At::Bus(b0)), Err(No::Sends(t2, 3)));
    m.set_now(t2, mx::send(3), SILENT_DB);
    m.set_now(b0, mx::send(2), -20.0);
    assert_eq!(m.stand_in(&[t1, t2], At::Bus(b0)), Err(No::Sends(b0, 2)));
    m.set_now(b0, mx::send(2), SILENT_DB);
    assert!(m.stand_in(&[t1, t2], At::Bus(b0)).is_ok());
    m.set_now(t1, mx::OUTPUT, output::MASTER);
    m.set_now(t2, mx::OUTPUT, output::MASTER);

    // A pad channel playing into the track: its sound is in the render;
    // with a send up, its instrument resting would silence the send.
    let pad = StripId::Pad(2);
    m.set_now(pad, mx::OUTPUT, output::TRACK);
    m.set_now(pad, mx::OUTPUT_TRACK, 1.0);
    assert!(m.stand_in(&[t1], At::BeforeFader).is_ok());
    m.set_now(pad, mx::send(0), -10.0);
    assert_eq!(m.stand_in(&[t1], At::BeforeFader), Err(No::PadSends(pad)));
    assert!(m.stand_in(&[t2], At::BeforeFader).is_ok(), "another track");
    m.set_now(pad, mx::OUTPUT, output::MASTER);

    // An input playing into the track is live.
    let input = StripId::Input(0);
    m.set_now(input, mx::OUTPUT, output::TRACK);
    m.set_now(input, mx::OUTPUT_TRACK, 1.0);
    assert_eq!(m.stand_in(&[t1], At::BeforeFader), Err(No::Live(input)));
    assert_eq!(m.stand_in(&[t1], At::AfterFader), Err(No::Live(input)));
    m.set_now(input, mx::OUTPUT, output::bus(3));
    assert_eq!(
        m.stand_in(&[input], At::Bus(StripId::Bus(3))),
        Err(No::Live(input))
    );
    m.set_now(input, mx::OUTPUT, output::MASTER);

    // Latency through an effect that reports it.
    m.chain_mut(t1).unwrap().push(Lag::boxed(37));
    run(&mut m);
    let lagged = m.stand_in(&[t1], At::BeforeFader).unwrap();
    assert_eq!(lagged.latency, lat0 + 37);
    assert_eq!(Some(lagged.latency), {
        assert!(m.set_capture(0, Some((t1, CapturePoint::BeforeFader))));
        run(&mut m);
        m.capture_latency(0)
    });

    // The mix itself, nothing, or strips not in the mix.
    assert_eq!(m.stand_in(&[t1], At::Master), Err(No::Mix));
    assert_eq!(m.stand_in(&[], At::BeforeFader), Err(No::NotInMix));
    assert_eq!(
        m.stand_in(&[StripId::Track(9)], At::BeforeFader),
        Err(No::NotInMix)
    );
    assert_eq!(m.stand_in(&[b0], At::AfterFader), Err(No::NotATrack(b0)));
    assert_eq!(
        m.stand_in(&[t1, t1], At::AfterFader),
        Err(No::NotATrack(t1))
    );
    assert!(No::Keys {
        from: t1,
        keyed: t0
    }
    .text()
    .contains("sidechain"));
}
