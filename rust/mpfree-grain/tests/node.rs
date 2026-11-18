//! The granular effect as an FX-chain node: its parameters, what it does
//! through the node interface, and the rules every node keeps.

use mpfree_grain::node::{param, voice_params};
use mpfree_grain::{FilterKind, GrainFilter, GrainNode};
use mpfree_node::{check_params, Ctx, Held, HeldStep, Kind, Node, NodeEvent, Timed};

const RATE: u32 = 44_100;

fn sine(frames: usize, hz: f32, level: f32) -> Vec<f32> {
    (0..frames)
        .map(|i| level * (i as f32 * hz * std::f32::consts::TAU / RATE as f32).sin())
        .collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn crossings(x: &[f32]) -> usize {
    x.windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count()
}

fn node() -> GrainNode {
    let mut n = GrainNode::new(3);
    n.prepare(RATE, 4096);
    // A dense, steady cloud close behind the input.
    n.set_param(param::POSITION, 90.0);
    n.set_param(param::SPRAY, 5.0);
    n.set_param(param::SIZE, 80.0);
    n.set_param(param::DENSITY, 30.0);
    n
}

/// Runs `n` over `input` (the same on both sides) in blocks of `block`,
/// with each event on its exact absolute frame. Returns left, right.
fn run(
    n: &mut GrainNode,
    input: &[f32],
    block: usize,
    events: &[(usize, NodeEvent)],
) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = (input.to_vec(), input.to_vec());
    let mut at = 0;
    while at < input.len() {
        let len = block.min(input.len() - at);
        let timed: Vec<Timed> = events
            .iter()
            .filter(|(f, _)| (at..at + len).contains(f))
            .map(|&(f, ev)| Timed::new((f - at) as u32, ev))
            .collect();
        n.process(
            &Ctx::new(RATE, at as u64),
            &timed,
            &mut l[at..at + len],
            &mut r[at..at + len],
        );
        at += len;
    }
    (l, r)
}

fn set(id: u16, value: f32) -> NodeEvent {
    NodeEvent::Param { id, value }
}

#[test]
fn it_describes_itself_and_clamps_its_parameters() {
    let mut n = GrainNode::new(1);
    assert_eq!(
        (n.info().id, n.info().kind),
        ("mpfree.grains", Kind::Effect)
    );
    check_params(n.params()).unwrap();
    for p in n.params() {
        assert_eq!(n.param(p.id), p.default, "{}", p.name);
    }
    n.set_param(param::MIX, 250.0);
    assert_eq!(n.param(param::MIX), 100.0);
    n.set_param(param::SIZE, f32::NAN);
    assert_eq!(n.param(param::SIZE), 120.0, "not finite: the default");
    n.set_param(param::FREEZE, 0.7);
    assert_eq!(n.param(param::FREEZE), 1.0);
    n.set_param(param::GRAINS, 9.0);
    assert_eq!(n.param(param::GRAINS), 0.0, "a meter can't be set");
    assert!(n.param(99).is_nan());
    n.set_param(99, 1.0); // ignored
}

#[test]
fn unprepared_or_fully_dry_it_passes_the_input_through() {
    let input = sine(20_000, 440.0, 0.5);
    let mut n = GrainNode::new(1);
    let (l, _) = run(&mut n, &input, 128, &[]);
    assert_eq!(l, input, "no memory yet: untouched");
    assert_eq!(n.tail_frames(), 0);
    let mut n = node();
    n.set_param(param::MIX, 0.0);
    let (l, r) = run(&mut n, &input, 128, &[]);
    assert!(l == input && r == input, "fully dry: exact");
    assert_eq!(n.fx().unwrap().stats().spawned, 0);
    assert_eq!(n.tail_frames(), 0);
}

#[test]
fn freeze_on_its_frame_holds_the_sound_after_the_input_stops() {
    // A second of 440 Hz, then two of silence; fully wet; frozen near the
    // end of the tone by a parameter event.
    let mut input = sine(RATE as usize, 440.0, 0.5);
    input.resize(RATE as usize * 3, 0.0);
    let tail = |freeze: bool| {
        let mut n = node();
        let mut events = vec![(0, set(param::MIX, 100.0))];
        if freeze {
            events.push((40_000, set(param::FREEZE, 1.0)));
        }
        let (l, _) = run(&mut n, &input, 256, &events);
        assert_eq!(n.param(param::FREEZE), if freeze { 1.0 } else { 0.0 });
        assert!(
            n.param(param::GRAINS) > 0.0 || !freeze,
            "the meter shows the grains"
        );
        (l[RATE as usize * 2..].to_vec(), n.tail_frames())
    };
    let ((held, frozen_tail), (gone, tail)) = (tail(true), tail(false));
    assert!(rms(&held) > 0.1, "frozen: {}", rms(&held));
    assert!(rms(&gone) < 1e-4, "not frozen: {}", rms(&gone));
    assert!((crossings(&held) as f32 / 880.0 - 1.0).abs() < 0.05);
    assert_eq!(
        frozen_tail,
        u32::MAX,
        "frozen, it rings for as long as it is called"
    );
    assert_eq!(
        tail,
        ((GrainNode::MEMORY_S + 0.08) * RATE as f32).ceil() as u32,
        "the memory and one grain"
    );
}

#[test]
fn the_output_is_the_same_at_any_block_size() {
    let input: Vec<f32> = sine(RATE as usize * 2, 330.0, 0.4)
        .iter()
        .enumerate()
        .map(|(i, x)| x + ((i * 31) % 97) as f32 * 1e-3)
        .collect();
    let events = [
        (0, set(param::MIX, 60.0)),
        (9_001, set(param::PITCH_SPRAY, 3.0)),
        (21_011, set(param::FREEZE, 1.0)),
        (33_333, set(param::REVERSE, 40.0)),
        (47_777, set(param::FREEZE, 0.0)),
        (60_000, set(param::MIX, 100.0)),
        (70_001, NodeEvent::AllOff), // means nothing to an effect
        (72_222, set(param::REPEAT, 4.0)),
        (75_000, set(param::REPEAT, 5.0)),
        (80_000, set(param::REPEAT, 0.0)),
    ];
    let first = run(&mut node(), &input, 128, &events);
    for block in [1, 37, 64, 500, 4096] {
        let other = run(&mut node(), &input, block, &events);
        let worst = first
            .0
            .iter()
            .chain(&first.1)
            .zip(other.0.iter().chain(&other.1))
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-4, "block {block}: {worst}");
    }
}

#[test]
fn reset_silences_it_and_forgets_what_it_heard() {
    let input = sine(RATE as usize, 440.0, 0.5);
    let mut n = node();
    n.set_param(param::MIX, 100.0);
    run(&mut n, &input, 128, &[]);
    assert!(n.param(param::GRAINS) > 0.0);
    n.reset();
    assert_eq!(n.param(param::GRAINS), 0.0);
    assert_eq!(n.param(param::MIX), 100.0, "settings stay");
    let (l, r) = run(&mut n, &vec![0.0; 20_000], 128, &[]);
    assert!(
        l.iter().chain(&r).all(|&x| x == 0.0),
        "nothing left to play"
    );
    // Preparing again at a new rate makes new memory; the settings carry over.
    n.prepare(48_000, 512);
    assert_eq!(
        n.fx().unwrap().buffer().capacity(),
        (GrainNode::MEMORY_S * 48_000.0) as usize
    );
    assert_eq!(n.fx().unwrap().mix(), 1.0);
}

/// Runs `n` over `input` as the transport plays at 120 BPM from beat 0.
fn run_playing(
    n: &mut GrainNode,
    input: &[f32],
    block: usize,
    events: &[(usize, NodeEvent)],
) -> Vec<f32> {
    let (mut l, mut r) = (input.to_vec(), input.to_vec());
    let mut at = 0;
    while at < input.len() {
        let len = block.min(input.len() - at);
        let timed: Vec<Timed> = events
            .iter()
            .filter(|(f, _)| (at..at + len).contains(f))
            .map(|&(f, ev)| Timed::new((f - at) as u32, ev))
            .collect();
        let ctx = Ctx {
            playing: true,
            beats: at as f64 / 22_050.0,
            ..Ctx::new(RATE, at as u64)
        };
        n.process(&ctx, &timed, &mut l[at..at + len], &mut r[at..at + len]);
        at += len;
    }
    l
}

#[test]
fn repeat_starts_on_its_line_of_the_beat_grid_and_plays_at_any_mix() {
    let input: Vec<f32> = sine(RATE as usize * 3, 220.0, 0.3)
        .iter()
        .enumerate()
        .map(|(i, x)| x * (1.0 + (i / 3000 % 5) as f32) / 5.0)
        .collect();
    // Fully dry; a quarter-note repeat asked for between beats 0 and 1.
    let mut n = node();
    n.set_param(param::MIX, 0.0);
    let events = [(10_000, set(param::REPEAT, 2.0))];
    let out = run_playing(&mut n, &input, 128, &events);
    assert_eq!(n.param(param::REPEAT), 2.0);
    // It waits for beat 1 (frame 22,050), then the beat before it plays
    // again every beat.
    assert_eq!(out[..22_050], input[..22_050]);
    let beat = 22_050;
    let off = 22_050 + 500;
    let diff = |a: usize, b: usize| {
        (0..20_000)
            .map(|j| (out[a + j] - out[b + j]).abs())
            .fold(0.0f32, f32::max)
    };
    assert!(
        diff(off + beat, off + 2 * beat) < 1e-3,
        "every beat the same"
    );
    assert!(
        (0..20_000).any(|j| (out[off + beat + j] - input[off + beat + j]).abs() > 0.05),
        "not the input"
    );
    assert_eq!(n.tail_frames(), u32::MAX, "it plays on while it repeats");
    assert!(n.param(param::GRAINS) >= 1.0);
}

#[test]
fn repeat_off_crossfades_back_on_its_frame() {
    let input = sine(RATE as usize * 2, 330.0, 0.4);
    let mut n = node();
    n.set_param(param::MIX, 0.0);
    let events = [
        (5_000, set(param::REPEAT, 3.0)),
        (40_000, set(param::REPEAT, 0.0)),
    ];
    let (l, _) = run(&mut n, &input, 256, &events);
    // Stopped transport: it starts on the frame it is set.
    assert_eq!(l[..5_000], input[..5_000]);
    assert!((5_441..40_000).any(|i| (l[i] - input[i]).abs() > 0.05));
    let back = 40_000 + 441 + 64;
    assert_eq!(l[back..], input[back..], "the input exactly, 10 ms on");
    assert_eq!(n.tail_frames(), 0);
    assert_eq!(n.param(param::REPEAT), 0.0);
}

#[test]
fn its_memory_and_settings_go_out_for_a_capture() {
    let mut n = node();
    n.set_param(param::PITCH, 7.0);
    n.set_param(param::SHAPE, 25.0);
    n.set_param(param::REVERSE, 30.0);
    n.set_param(param::FILTER, 2.0);
    n.set_param(param::CUTOFF, 640.0);
    n.set_param(param::FREEZE, 1.0);
    let mut held = Held::new(2, RATE as usize * 9);
    assert_eq!(
        n.copy_held(&mut held, 4096),
        HeldStep::Nothing,
        "nothing heard yet"
    );
    n.set_param(param::FREEZE, 0.0);
    run(
        &mut n,
        &sine(RATE as usize, 440.0, 0.5),
        256,
        &[(30_000, set(param::FREEZE, 1.0))],
    );
    let mut calls = 1;
    while n.copy_held(&mut held, 4096) == HeldStep::More {
        calls += 1;
    }
    assert_eq!(
        held.span(),
        Some((0, 30_000)),
        "everything up to the freeze"
    );
    assert_eq!(calls, 8);
    held.keep_values(&n);
    assert!(
        held.values().iter().all(|&(id, _)| id != param::GRAINS),
        "no meter"
    );
    let fx = n.fx().unwrap();
    assert_eq!(
        &voice_params(held.values()),
        fx.params(),
        "the pad plays as the effect did"
    );
    // Unprepared, it holds nothing.
    let mut cold = GrainNode::new(1);
    assert_eq!(
        cold.copy_held(&mut Held::new(2, 10), 4096),
        HeldStep::Nothing
    );
}

#[test]
fn its_filter_reaches_every_grain() {
    let mut n = node();
    let filter = |n: &GrainNode| n.fx().unwrap().params().filter;
    assert_eq!(filter(&n), GrainFilter::default(), "off by default");
    n.set_param(param::FILTER, 3.0);
    n.set_param(param::CUTOFF, 5_000.0);
    n.set_param(param::RESONANCE, 40.0);
    n.set_param(param::FILTER_SPRAY, 50.0);
    assert_eq!(
        filter(&n),
        GrainFilter {
            kind: FilterKind::High,
            cutoff_hz: 5_000.0,
            resonance: 0.4,
            spray: 0.5,
        }
    );
    n.set_param(param::CUTOFF, 1.0e6);
    assert_eq!(n.param(param::CUTOFF), 20_000.0);
    // A dark filter on a bright input takes its highs away.
    let mut x = 9u32;
    let noise: Vec<f32> = (0..RATE as usize)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (x >> 8) as f32 / (1u32 << 23) as f32 - 1.0
        })
        .collect();
    let bright = |n: &mut GrainNode| {
        n.set_param(param::MIX, 100.0);
        let (out, _) = run(n, &noise, 256, &[]);
        let y = &out[RATE as usize / 2..];
        let p: f32 = y.iter().map(|v| v * v).sum();
        let d: f32 = y.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum();
        d / p
    };
    let open = bright(&mut node());
    let mut dark = node();
    dark.set_param(param::FILTER, 1.0);
    dark.set_param(param::CUTOFF, 300.0);
    let shut = bright(&mut dark);
    assert!(shut < open / 10.0, "{shut} against {open}");
}
