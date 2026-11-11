//! Perform FX: entry on the grid, each effect doing what it says, the
//! release snapping back, loops that keep their slice however long a pad
//! is held, the pad held last winning, the echo ringing out, and the same
//! output for any block size.

use std::f32::consts::PI;

use mpfree_fx::mpfree_node::{check_params, Ctx, Node, NodeEvent, Timed, MAX_BLOCK};
use mpfree_fx::{
    ids, perform_state, Perform, XyPad, PERFORM_EFFECTS, PERFORM_LENGTHS, PERFORM_SLOTS,
};

const RATE: u32 = 48_000;
/// 120 BPM: a beat is 24 000 frames and a 1/16 is 6 000.
const BEAT: usize = 24_000;
const SIXTEENTH: usize = 6_000;

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

fn effect(name: &str) -> f32 {
    PERFORM_EFFECTS.iter().position(|e| *e == name).unwrap() as f32
}

fn length(name: &str) -> f32 {
    PERFORM_LENGTHS.iter().position(|e| *e == name).unwrap() as f32
}

/// Sets slot `k` to an effect, a length and an amount.
fn slot(p: &mut Perform, k: usize, fx: &str, len: &str, amount: f32) {
    let id = ids::perform::slot(k);
    p.set_param(id + ids::perform::EFFECT, effect(fx));
    p.set_param(id + ids::perform::LENGTH, length(len));
    p.set_param(id + ids::perform::AMOUNT, amount);
}

fn on(k: u8) -> NodeEvent {
    NodeEvent::NoteOn {
        note: k,
        velocity: 100,
    }
}

fn off(k: u8) -> NodeEvent {
    NodeEvent::NoteOff { note: k }
}

/// Runs `input` through the node in blocks from `next_block`, with events
/// at absolute frames, at 120 BPM with the transport playing (or not).
fn render(
    p: &mut Perform,
    input: &(Vec<f32>, Vec<f32>),
    events: &[(usize, NodeEvent)],
    playing: bool,
    next_block: &mut dyn FnMut() -> usize,
) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = input.clone();
    let n = l.len();
    let (mut at, mut k) = (0, 0);
    let mut evs = Vec::new();
    while at < n {
        let len = next_block().clamp(1, MAX_BLOCK).min(n - at);
        evs.clear();
        while k < events.len() && events[k].0 < at + len {
            evs.push(Timed::new((events[k].0 - at) as u32, events[k].1));
            k += 1;
        }
        let ctx = Ctx {
            bpm: 120.0,
            beats: at as f64 * 120.0 / 60.0 / RATE as f64,
            playing,
            ..Ctx::new(RATE, at as u64)
        };
        p.process(&ctx, &evs, &mut l[at..at + len], &mut r[at..at + len]);
        at += len;
    }
    (l, r)
}

fn run(
    p: &mut Perform,
    input: &(Vec<f32>, Vec<f32>),
    events: &[(usize, NodeEvent)],
    playing: bool,
) -> (Vec<f32>, Vec<f32>) {
    render(p, input, events, playing, &mut || 256)
}

/// Noise that never repeats, so every frame can be told from the others.
fn noise(frames: usize, seed: u64) -> (Vec<f32>, Vec<f32>) {
    let mut g = Lcg(seed);
    let l = (0..frames).map(|_| g.unit() * 2.0 - 1.0).collect();
    let r = (0..frames).map(|_| g.unit() * 2.0 - 1.0).collect();
    (l, r)
}

fn sine(hz: f32, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let s: Vec<f32> = (0..frames)
        .map(|i| 0.5 * (2.0 * PI * hz * i as f32 / RATE as f32).sin())
        .collect();
    (s.clone(), s)
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| (v * v) as f64).sum::<f64>() / x.len().max(1) as f64).sqrt() as f32
}

/// Upward zero crossings per second.
fn pitch(x: &[f32]) -> f32 {
    let n = x.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
    n as f32 * RATE as f32 / x.len() as f32
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-5
}

fn assert_follows(
    name: &str,
    out: &[f32],
    want: impl Fn(usize) -> f32,
    frames: std::ops::Range<usize>,
) {
    for i in frames {
        assert!(
            close(out[i], want(i)),
            "{name}: frame {i} is {} but should be {}",
            out[i],
            want(i)
        );
    }
}

#[test]
fn the_description_is_valid_and_the_bank_is_full() {
    let p = Perform::new();
    check_params(p.params()).unwrap();
    assert_eq!(
        p.params().len(),
        2 + 4 * PERFORM_SLOTS + 3 + PERFORM_SLOTS + 1
    );
    assert_eq!(ids::perform::MIX, 85);
    assert_eq!(p.param(ids::perform::MIX), 100.0);
    assert_eq!(
        p.param(ids::perform::slot(15) + ids::perform::EFFECT),
        effect("Wash throw"),
        "pad 16 is the Play page's wash"
    );
    for k in 0..PERFORM_SLOTS {
        let fx = p.param(ids::perform::slot(k) + ids::perform::EFFECT);
        assert!(fx >= 1.0, "pad {} has an effect by default", k + 1);
        assert_eq!(
            p.param(ids::perform::slot(k) + ids::perform::STATE),
            perform_state::OFF
        );
    }
    // Under 10 MiB at 48 kHz: the ring, two loop buffers, the echo and the
    // wash.
    assert!(p.memory_bytes() < 10 << 20, "{} bytes", p.memory_bytes());
}

#[test]
fn a_stutter_waits_for_the_grid_then_loops_what_came_next() {
    let mut p = Perform::new();
    slot(&mut p, 0, "Stutter", "1/16", 100.0);
    let x = noise(5 * SIXTEENTH, 1);
    // Pressed between grid lines: it starts on the next 1/16.
    let y = run(&mut p, &x, &[(1_000, on(0))], true);
    assert_follows("before the grid", &y.0, |i| x.0[i], 0..SIXTEENTH);
    // The first pass is the live sound, then the same 1/16 again and again
    // (each pass fades into the frames before its start at the very end).
    let seam = 192; // 4 ms
    assert_follows(
        "first pass",
        &y.0,
        |i| x.0[i],
        SIXTEENTH..2 * SIXTEENTH - seam,
    );
    for pass in 2..5 {
        let at = pass * SIXTEENTH;
        assert_follows(
            "repeats",
            &y.1,
            |i| x.1[SIXTEENTH + i - at],
            at..at + SIXTEENTH - seam,
        );
    }
}

#[test]
fn with_the_transport_stopped_a_pad_starts_at_once() {
    let mut p = Perform::new();
    slot(&mut p, 0, "Beat repeat", "1/16", 100.0);
    let x = noise(3 * SIXTEENTH, 2);
    let at = SIXTEENTH + 1_000;
    let y = run(&mut p, &x, &[(at, on(0))], false);
    // After the 4 ms entry fade it plays the 1/16 before the press.
    assert_follows(
        "beat repeat",
        &y.0,
        |i| x.0[i - SIXTEENTH],
        at + 200..at + SIXTEENTH - 200,
    );
}

#[test]
fn letting_go_snaps_back_to_the_live_sound() {
    let mut p = Perform::new();
    slot(&mut p, 0, "Beat repeat", "1/16", 100.0);
    let x = noise(4 * SIXTEENTH, 3);
    let y = run(&mut p, &x, &[(0, on(0)), (SIXTEENTH + 777, off(0))], true);
    // Live again once the 4 ms fade is over.
    assert_follows(
        "released",
        &y.0,
        |i| x.0[i],
        SIXTEENTH + 777 + 193..x.0.len(),
    );
    let state = p.param(ids::perform::slot(0) + ids::perform::STATE);
    assert_eq!(state, perform_state::OFF);
}

#[test]
fn reverse_plays_the_last_slice_backward() {
    let mut p = Perform::new();
    slot(&mut p, 0, "Reverse", "1/16", 100.0);
    let x = noise(4 * SIXTEENTH, 4);
    let y = run(&mut p, &x, &[(SIXTEENTH, on(0))], true);
    let e = SIXTEENTH;
    assert_follows(
        "first pass",
        &y.0,
        |i| x.0[2 * e - 1 - i],
        e + 200..2 * e - 200,
    );
    // Later passes play the same slice backward again.
    assert_follows(
        "second pass",
        &y.0,
        |i| x.0[3 * e - 1 - i],
        2 * e..3 * e - 200,
    );
}

#[test]
fn a_loop_keeps_its_slice_however_long_the_pad_is_held() {
    // Held for 30 seconds: far longer than the input's ring.
    let mut p = Perform::new();
    slot(&mut p, 0, "Beat repeat", "1/4", 100.0);
    let n = 30 * RATE as usize;
    let x = noise(n, 5);
    let y = run(&mut p, &x, &[(BEAT, on(0))], true);
    let seam = 192;
    let late = n - 2 * BEAT;
    let start = late - late % BEAT; // a pass starts on each beat
    assert_follows(
        "after 30 s",
        &y.0,
        |i| x.0[i - start],
        start..start + BEAT - seam,
    );
}

#[test]
fn loops_wrap_without_a_click() {
    // A sine whose period does not divide the loop: the wrap would jump.
    let mut p = Perform::new();
    slot(&mut p, 0, "Stutter", "1/16", 100.0);
    let x = sine(437.0, 6 * SIXTEENTH);
    let y = run(&mut p, &x, &[(0, on(0))], true);
    let slope = 0.5 * 2.0 * PI * 437.0 / RATE as f32;
    let worst =
        y.0.windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
    assert!(
        worst < 2.0 * slope,
        "worst step {worst}, the sine's own {slope}"
    );
}

#[test]
fn the_pad_held_last_wins_and_letting_it_go_returns_to_the_one_before() {
    let mut p = Perform::new();
    slot(&mut p, 0, "Beat repeat", "1/4", 100.0);
    slot(&mut p, 1, "Stutter", "1/16", 100.0);
    let x = noise(6 * BEAT, 6);
    let events = [
        (BEAT, on(0)),
        (2 * BEAT, on(1)),
        (2 * BEAT + 2 * SIXTEENTH, off(1)),
    ];
    let y = run(&mut p, &x, &events, true);
    // Pad 2's stutter while both are held...
    let e = 2 * BEAT;
    assert_follows(
        "stutter",
        &y.0,
        |i| x.0[e + (i - e) % SIXTEENTH],
        e + SIXTEENTH..e + 2 * SIXTEENTH - 192,
    );
    // ...then pad 1 again, starting afresh: it repeats the beat before.
    let back = e + 2 * SIXTEENTH;
    assert_follows(
        "beat repeat again",
        &y.0,
        |i| x.0[i - BEAT],
        back + 200..back + BEAT - 200,
    );
}

#[test]
fn tape_stop_slows_down_then_goes_silent() {
    let mut p = Perform::new();
    slot(&mut p, 0, "Tape stop", "1/4", 100.0);
    let x = sine(880.0, 3 * BEAT);
    let y = run(&mut p, &x, &[(BEAT, on(0))], true);
    let early = pitch(&y.0[BEAT..BEAT + BEAT / 8]);
    let late = pitch(&y.0[BEAT + BEAT / 2..BEAT + 3 * BEAT / 4]);
    assert!(early > 750.0, "starts near the pitch: {early} Hz");
    assert!(late < 0.6 * early, "slows down: {late} Hz after {early} Hz");
    assert_eq!(rms(&y.0[2 * BEAT..3 * BEAT]), 0.0, "stopped while held");
}

#[test]
fn half_speed_and_pitch_drop_go_an_octave_down() {
    let x = sine(1_000.0, 3 * BEAT);
    let mut p = Perform::new();
    slot(&mut p, 0, "Half speed", "1/4", 100.0);
    let y = run(&mut p, &x, &[(BEAT, on(0))], true);
    let hz = pitch(&y.0[BEAT + 1_000..2 * BEAT]);
    assert!((hz - 500.0).abs() < 25.0, "half speed: {hz} Hz");

    let mut p = Perform::new();
    slot(&mut p, 0, "Pitch drop", "1/16", 50.0);
    let y = run(&mut p, &x, &[(BEAT, on(0))], true);
    let hz = pitch(&y.0[BEAT + BEAT / 2..2 * BEAT]);
    assert!(
        (hz - 500.0).abs() < 40.0,
        "pitch drop of 12 semitones: {hz} Hz"
    );
}

#[test]
fn the_trance_gate_opens_on_the_grid() {
    let mut p = Perform::new();
    slot(&mut p, 0, "Trance gate", "1/16", 100.0);
    let x = sine(220.0, 3 * BEAT);
    let y = run(&mut p, &x, &[(0, on(0))], true);
    for k in 1..8 {
        let at = k * SIXTEENTH;
        let open = rms(&y.0[at + 200..at + SIXTEENTH / 2 - 200]);
        let shut = rms(&y.0[at + SIXTEENTH / 2 + 200..at + SIXTEENTH - 200]);
        assert!(
            open > 0.3 && shut < 1e-6,
            "1/16 {k}: open {open}, shut {shut}"
        );
    }
}

#[test]
fn sweeps_move_the_cutoff_over_their_length() {
    // Low-pass: a 4 kHz tone is gone by the end of a beat-long sweep.
    let x = sine(4_000.0, 2 * BEAT);
    let mut p = Perform::new();
    slot(&mut p, 0, "LP sweep", "1/4", 70.0);
    let y = run(&mut p, &x, &[(0, on(0))], true);
    let start = rms(&y.0[1_000..3_000]);
    let end = rms(&y.0[BEAT + 1_000..2 * BEAT]);
    assert!(start > 0.3 && end < 0.02, "LP sweep: {start} then {end}");
    // High-pass: a 100 Hz tone goes the same way.
    let x = sine(100.0, 2 * BEAT);
    let mut p = Perform::new();
    slot(&mut p, 0, "HP sweep", "1/4", 70.0);
    let y = run(&mut p, &x, &[(0, on(0))], true);
    let start = rms(&y.0[2_000..4_000]);
    let end = rms(&y.0[BEAT + 1_000..2 * BEAT]);
    assert!(start > 0.3 && end < 0.02, "HP sweep: {start} then {end}");
}

#[test]
fn crush_holds_and_quantises() {
    let mut p = Perform::new();
    slot(&mut p, 0, "Crush", "1/16", 100.0);
    let x = sine(100.0, BEAT);
    let y = run(&mut p, &x, &[(0, on(0))], true);
    // 4 bits and every 8th frame held, once the 4 ms fade is in.
    let held = &y.0[1_024..BEAT];
    let steps = held.windows(2).filter(|w| w[0] != w[1]).count();
    assert!(steps <= held.len() / 8 + 1, "{steps} changes");
    for v in held {
        let q = v * 8.0;
        assert!((q - q.round()).abs() < 1e-4, "{v} is not on a 4-bit level");
    }
}

#[test]
fn an_echo_throw_rings_out_after_the_pad_is_let_go() {
    let mut p = Perform::new();
    slot(&mut p, 0, "Echo throw", "1/8", 60.0);
    // A short burst, thrown, then silence.
    let mut x = sine(440.0, 4 * BEAT);
    x.0[BEAT / 4..].fill(0.0);
    x.1[BEAT / 4..].fill(0.0);
    let y = run(&mut p, &x, &[(0, on(0)), (BEAT / 4, off(0))], true);
    // The dry sound is untouched before the first repeat (1/8 = 12 000).
    let eighth = BEAT / 2;
    assert_follows("dry", &y.0, |i| x.0[i], 0..BEAT / 4);
    let echo1 = rms(&y.0[eighth..eighth + BEAT / 4]);
    let echo3 = rms(&y.0[3 * eighth..3 * eighth + BEAT / 4]);
    let gap = rms(&y.0[BEAT / 4 + 500..eighth - 500]);
    assert!(echo1 > 0.05, "first repeat {echo1}");
    assert!(echo3 > 0.01 && echo3 < echo1, "third repeat {echo3}");
    assert!(gap < 1e-3, "silence between {gap}");
}

#[test]
fn the_noise_riser_rises() {
    let mut p = Perform::new();
    slot(&mut p, 0, "Noise riser", "1/4", 100.0);
    let x = (vec![0.0; 2 * BEAT], vec![0.0; 2 * BEAT]);
    let y = run(&mut p, &x, &[(0, on(0))], true);
    let a = rms(&y.0[BEAT / 8..BEAT / 4]);
    let b = rms(&y.0[3 * BEAT / 4..BEAT]);
    assert!(b > 4.0 * a && b > 0.01, "riser: {a} then {b}");
}

#[test]
fn the_state_shows_waiting_on_and_fading() {
    let mut p = Perform::new();
    slot(&mut p, 0, "Trance gate", "1/16", 100.0);
    let state = |p: &Perform| p.param(ids::perform::slot(0) + ids::perform::STATE);
    let block = |p: &mut Perform, at: usize, events: &[Timed]| {
        let (mut l, mut r) = (vec![0.1; 1_000], vec![0.1; 1_000]);
        let ctx = Ctx {
            bpm: 120.0,
            beats: at as f64 * 2.0 / RATE as f64,
            playing: true,
            ..Ctx::new(RATE, at as u64)
        };
        p.process(&ctx, events, &mut l, &mut r);
    };
    block(&mut p, 0, &[Timed::new(100, on(0))]);
    assert_eq!(state(&p), perform_state::WAITING);
    for k in 1..7 {
        block(&mut p, k * 1_000, &[]);
    }
    assert_eq!(state(&p), perform_state::ON);
    block(&mut p, 7_000, &[Timed::new(0, off(0))]);
    assert_eq!(state(&p), perform_state::OFF, "the 4 ms fade is over");
    block(
        &mut p,
        8_000,
        &[Timed::new(0, on(0)), Timed::new(990, off(0))],
    );
    assert_eq!(
        state(&p),
        perform_state::OFF,
        "never started: let go before the grid"
    );
    block(&mut p, 9_000, &[]);
    block(&mut p, 10_000, &[]);
    block(&mut p, 11_000, &[Timed::new(500, on(0))]);
    block(&mut p, 12_000, &[Timed::new(900, off(0))]);
    assert_eq!(state(&p), perform_state::FADING);
}

#[test]
fn the_same_output_for_any_block_size() {
    // Every pad pressed and let go at random, settings moving, 12 seconds.
    let n = 12 * RATE as usize;
    let x = noise(n, 9);
    let mut g = Lcg(10);
    let mut events = Vec::new();
    let mut at = 0;
    while at < n {
        at += 1 + (g.next() as usize % 9_000);
        let k = (g.next() % PERFORM_SLOTS as u64) as u8;
        let ev = match g.next() % 8 {
            0 | 1 => on(k),
            2 | 3 => off(k),
            4 => NodeEvent::Param {
                id: ids::perform::slot(k as usize) + (g.next() % 3) as u16,
                value: g.unit() * 100.0,
            },
            // The XY pad and the DJ knob: the filter, the echo, its time.
            5 => NodeEvent::Param {
                id: ids::perform::DJ_FILTER,
                value: g.unit() * 200.0 - 100.0,
            },
            // The overall mix, often all the way up.
            7 => NodeEvent::Param {
                id: ids::perform::MIX,
                value: if g.next().is_multiple_of(2) {
                    100.0
                } else {
                    g.unit() * 100.0
                },
            },
            _ => NodeEvent::Param {
                id: ids::perform::ECHO + (g.next() % 2) as u16,
                value: if g.next().is_multiple_of(3) {
                    0.0
                } else {
                    g.unit() * 100.0
                },
            },
        };
        events.push((at.min(n - 1), ev));
    }
    let make = || {
        let mut p = Perform::new();
        p.set_param(ids::perform::QUANTISE, 2.0);
        p
    };
    let reference = render(&mut make(), &x, &events, true, &mut || 64);
    let mut sizes = Lcg(11);
    for (how, mut next) in [
        ("1 frame", Box::new(|| 1usize) as Box<dyn FnMut() -> usize>),
        ("odd 37", Box::new(|| 37)),
        ("512", Box::new(|| 512)),
        (
            "random",
            Box::new(move || 1 + (sizes.next() as usize % 700)),
        ),
    ] {
        let y = render(&mut make(), &x, &events, true, &mut *next);
        for (ch, (a, b)) in [(&reference.0, &y.0), (&reference.1, &y.1)]
            .into_iter()
            .enumerate()
        {
            if let Some(i) = (0..n).find(|&i| a[i].to_bits() != b[i].to_bits()) {
                panic!(
                    "{how}: channel {ch} differs first at frame {i}: {} vs {}",
                    a[i], b[i]
                );
            }
        }
    }
    assert!(reference.0.iter().all(|v| v.is_finite()));
}

#[test]
fn the_dj_filter_cuts_each_way_and_is_off_in_the_middle() {
    // In the middle (the default) with nothing held: the input exactly.
    let x = noise(BEAT, 12);
    let y = run(&mut Perform::new(), &x, &[], true);
    assert_eq!(x, y);
    // All the way left, a low-pass takes 5 kHz away; right, a high-pass
    // takes 60 Hz away.
    for (pos, hz) in [(-100.0, 5_000.0), (100.0, 60.0)] {
        let x = sine(hz, BEAT);
        let set = NodeEvent::Param {
            id: ids::perform::DJ_FILTER,
            value: pos,
        };
        let y = run(&mut Perform::new(), &x, &[(0, set)], true);
        let late = rms(&y.0[BEAT / 2..]);
        assert!(late < 0.01, "DJ filter at {pos}: {hz} Hz left at {late}");
    }
}

#[test]
fn the_echo_knob_sends_into_the_echo_and_its_tail_rings_out() {
    let mut p = Perform::new();
    // A short burst with the echo up, then the knob back down.
    let mut x = sine(440.0, 4 * BEAT);
    x.0[BEAT / 4..].fill(0.0);
    x.1[BEAT / 4..].fill(0.0);
    let set = |v: f32| NodeEvent::Param {
        id: ids::perform::ECHO,
        value: v,
    };
    let y = run(&mut p, &x, &[(0, set(70.0)), (BEAT / 4, set(0.0))], true);
    // The echo time is 1/8 dotted by default: 18 000 frames at 120 BPM.
    let d = 3 * BEAT / 4;
    assert_follows("dry", &y.0, |i| x.0[i], 0..BEAT / 8);
    let first = rms(&y.0[d..d + BEAT / 4]);
    // Ping-pong: the second repeat is on the right.
    let second = rms(&y.1[2 * d..2 * d + BEAT / 4]);
    let between = rms(&y.0[BEAT / 4 + 1_000..d - 1_000]);
    assert!(first > 0.05, "first repeat {first}");
    assert!(second > 0.01 && second < first, "second repeat {second}");
    assert!(between < 1e-3, "nothing between {between}");
}

#[test]
fn the_xy_pad_plays_two_parameters_and_snaps_back_or_holds() {
    let mut xy = XyPad::perform();
    assert!(xy.at_rest());
    assert_eq!(
        xy.values(),
        [(ids::perform::DJ_FILTER, 0.0), (ids::perform::ECHO, 0.0)]
    );
    // A finger at the top right: high-pass all the way, echo full.
    xy.touch(1.0, 1.0);
    assert_eq!(
        xy.values(),
        [
            (ids::perform::DJ_FILTER, 100.0),
            (ids::perform::ECHO, 100.0)
        ]
    );
    assert!(!xy.tick(10.0), "no glide under a finger");
    // Lifted: back to rest within five glide times, a frame at a time.
    xy.release();
    let mut ms = 0.0;
    while xy.tick(16.0) {
        ms += 16.0;
        assert!(ms < 5.0 * xy.snap_ms + 32.0, "still gliding after {ms} ms");
    }
    assert!(xy.at_rest());
    // With Hold on it stays where the finger left it, until Hold goes off.
    xy.set_hold(true);
    xy.touch(0.25, 0.5);
    xy.release();
    assert!(!xy.tick(100.0));
    assert_eq!(xy.values()[0], (ids::perform::DJ_FILTER, -50.0));
    xy.set_hold(false);
    assert!(xy.tick(16.0));
    let (x, y) = xy.position();
    assert!(x > 0.25 && x < 0.5 && y < 0.5, "gliding back: {x}, {y}");
}

fn hold(k: usize, on: bool) -> NodeEvent {
    NodeEvent::Param {
        id: ids::perform::hold(k),
        value: if on { 1.0 } else { 0.0 },
    }
}

#[test]
fn a_hold_plays_a_pad_like_a_finger() {
    // A stutter on a lock (Hold on at a frame, off later) sounds exactly
    // like the same pad held by a note, at any block size.
    let x = noise(6 * SIXTEENTH, 7);
    let fresh = || {
        let mut p = Perform::new();
        slot(&mut p, 0, "Stutter", "1/16", 100.0);
        slot(&mut p, 1, "Crush", "1/16", 80.0);
        p
    };
    let by_note = run(
        &mut fresh(),
        &x,
        &[
            (1_000, on(0)),
            (9_000, on(1)),
            (20_000, off(0)),
            (26_000, off(1)),
        ],
        true,
    );
    let by_hold = run(
        &mut fresh(),
        &x,
        &[
            (1_000, hold(0, true)),
            (9_000, hold(1, true)),
            (20_000, hold(0, false)),
            (26_000, hold(1, false)),
        ],
        true,
    );
    assert_eq!(by_note, by_hold);
    let mut k = 0;
    let ragged = render(
        &mut fresh(),
        &x,
        &[
            (1_000, hold(0, true)),
            (9_000, hold(1, true)),
            (20_000, hold(0, false)),
            (26_000, hold(1, false)),
        ],
        true,
        &mut || {
            k = (k * 7 + 13) % 997;
            k + 1
        },
    );
    assert_eq!(by_note, ragged);
}

#[test]
fn a_pad_sounds_while_a_finger_or_its_hold_holds_it() {
    let mut p = Perform::new();
    slot(&mut p, 0, "Stutter", "1/16", 100.0);
    let x = noise(4 * SIXTEENTH, 9);
    let state = |p: &Perform| p.param(ids::perform::slot(0) + ids::perform::STATE);
    // Hold on, a finger on and off: still on. Hold off: off.
    run(
        &mut p,
        &x,
        &[(0, hold(0, true)), (100, on(0)), (200, off(0))],
        false,
    );
    assert_eq!(state(&p), perform_state::ON);
    assert_eq!(p.param(ids::perform::hold(0)), 1.0);
    run(&mut p, &x, &[(0, hold(0, false))], false);
    assert_eq!(state(&p), perform_state::OFF);
    // A finger, then Hold on and off: the finger still holds it.
    run(
        &mut p,
        &x,
        &[(0, on(0)), (100, hold(0, true)), (200, hold(0, false))],
        false,
    );
    assert_eq!(state(&p), perform_state::ON);
    run(&mut p, &x, &[(0, off(0))], false);
    assert_eq!(state(&p), perform_state::OFF);
    // Without a frame (set_param), Hold lands at the start of the next
    // block; AllOff lets the Holds go too.
    p.set_param(ids::perform::hold(0), 1.0);
    assert_eq!(p.param(ids::perform::hold(0)), 1.0);
    run(&mut p, &x, &[], false);
    assert_eq!(state(&p), perform_state::ON);
    run(&mut p, &x, &[(0, NodeEvent::AllOff)], false);
    assert_eq!(state(&p), perform_state::OFF);
    assert_eq!(p.param(ids::perform::hold(0)), 0.0);
}

#[test]
fn a_wash_throw_rings_on_after_the_pad_is_let_go_then_sleeps() {
    let mut p = Perform::new();
    slot(&mut p, 0, "Wash throw", "1 bar", 70.0);
    let sec = RATE as usize;
    let n = 8 * sec;
    let mut x = noise(n, 3);
    for v in x.0.iter_mut().chain(x.1.iter_mut()) {
        *v *= 0.3;
    }
    x.0[sec..].fill(0.0);
    x.1[sec..].fill(0.0);
    let y = run(&mut p, &x, &[(0, on(0)), (sec / 2, off(0))], true);
    // The dry sound passes untouched until the first reflection (25 ms).
    assert_follows("dry", &y.0, |i| x.0[i], 0..1000);
    // While held the wash is under the dry sound but plainly there.
    let dry = rms(&x.0[sec / 4..sec / 2]);
    let wet: Vec<f32> = (sec / 4..sec / 2).map(|i| y.0[i] - x.0[i]).collect();
    let db = 20.0 * (rms(&wet) / dry).log10();
    assert!((-12.0..-3.0).contains(&db), "the wash at {db} dB");
    // Once the music stops the tail rings on, then falls away.
    let t1 = rms(&y.0[sec + sec / 10..sec + sec / 5]);
    let t2 = rms(&y.0[2 * sec..2 * sec + sec / 10]);
    assert!(t1 > 0.005 * dry, "the tail {t1}");
    assert!(t2 < 0.25 * t1, "the tail falls: {t1} then {t2}");
    // Then it sleeps: silence in is silence out, and no tail is left.
    assert!(y.0[6 * sec..]
        .iter()
        .chain(&y.1[6 * sec..])
        .all(|&v| v == 0.0));
    assert!(p.tail_frames() < 2_000, "tail {}", p.tail_frames());
}

#[test]
fn a_longer_wash_rings_longer() {
    let sec = RATE as usize;
    let tail = |amount: f32| {
        let mut p = Perform::new();
        slot(&mut p, 0, "Wash throw", "1 bar", amount);
        let mut x = noise(4 * sec, 5);
        x.0[sec / 2..].fill(0.0);
        x.1[sec / 2..].fill(0.0);
        let y = run(&mut p, &x, &[(0, on(0)), (sec / 2, off(0))], true);
        rms(&y.0[2 * sec..2 * sec + sec / 10])
    };
    let (short, long) = (tail(20.0), tail(100.0));
    assert!(long > 10.0 * short, "100 % rings {long}, 20 % {short}");
}

#[test]
fn the_mix_sets_how_much_of_the_effects_is_heard() {
    let n = 3 * BEAT;
    let x = noise(n, 21);
    // A stutter held throughout, with an echo throw and a wash ringing.
    let events = [
        (0, on(0)),
        (BEAT / 2, on(9)),
        (BEAT / 2 + 3_000, off(9)),
        (BEAT, on(15)),
        (BEAT + 6_000, off(15)),
        (2 * BEAT, off(0)),
    ];
    let make = |mix: f32| {
        let mut p = Perform::new();
        p.set_param(ids::perform::MIX, mix);
        p.reset();
        p
    };
    let full = run(&mut make(100.0), &x, &events, true);
    let none = run(&mut make(0.0), &x, &events, true);
    let half = run(&mut make(50.0), &x, &events, true);
    let moved = rms(&(0..n).map(|i| full.0[i] - x.0[i]).collect::<Vec<_>>());
    assert!(moved > 0.1, "the effects change the sound ({moved})");
    for i in 0..n {
        for (a, b) in [(none.0[i], x.0[i]), (none.1[i], x.1[i])] {
            assert_eq!(a.to_bits(), b.to_bits(), "mix 0 at frame {i}: {a} vs {b}");
        }
        let want = x.0[i] + 0.5 * (full.0[i] - x.0[i]);
        assert!(
            (half.0[i] - want).abs() < 1e-5,
            "mix 50 at frame {i}: {} vs {want}",
            half.0[i]
        );
    }
}
