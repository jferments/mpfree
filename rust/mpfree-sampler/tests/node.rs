//! The sampler as a node: it plays what the engine plays, to the sample,
//! for any block size; notes find their pads or keygroups and note-offs
//! follow them; views swap in one per block and come back to be dropped;
//! Level lands on its frame.

use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::*;
use mpfree_sampler::node::{ids, SamplerNode, DEFAULT_BASE_NOTE, VIEW_QUEUE};

const RATE: u32 = 44_100;
const VOICES: usize = 16;

/// A tone with a little noise, so pads and pitches differ.
fn tone(name: &str, seconds: f64, hz: f64) -> Source {
    let n = (seconds * f64::from(RATE)) as usize;
    let mut seed = 0x1234_5678u32 ^ hz as u32;
    let v = (0..n)
        .map(|i| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let noise = (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
            let t = i as f64 / f64::from(RATE);
            (0.4 * (std::f64::consts::TAU * hz * t).sin()) as f32 + 0.05 * noise
        })
        .collect();
    Source::new(name, RATE, 1, v).unwrap()
}

/// A kit: pads 1 to 4 one-shots and gates at four pitches.
fn kit() -> Project {
    let mut p = Project::new();
    for (i, hz) in [110.0, 220.0, 330.0, 440.0].into_iter().enumerate() {
        let s = tone(&format!("t{i}"), 0.4, hz);
        p.add_source(s.clone());
        let mut r = Region::whole(&s);
        if i % 2 == 1 {
            r.mode = PlayMode::Gate;
        }
        p.set_pad(i, Some(Pad::new(&format!("pad {i}"), Layer::new(r))), None)
            .unwrap();
    }
    p
}

/// A keygroup program: one gate pad over the whole keyboard, root A3.
fn keys() -> Project {
    let s = tone("keys", 1.0, 220.0);
    let mut p = Project::new();
    p.add_source(s.clone());
    let mut r = Region::whole(&s);
    r.mode = PlayMode::Gate;
    p.set_pad(0, Some(Pad::new("keys", Layer::new(r))), None)
        .unwrap();
    p.spread_to_keys(0, 57).unwrap();
    p
}

fn node(p: &Project) -> (SamplerNode, mpfree_sampler::node::SamplerControl) {
    SamplerNode::new(RATE, VOICES, EngineView::build(p, RATE))
}

fn on(note: u8, velocity: u8) -> NodeEvent {
    NodeEvent::NoteOn { note, velocity }
}

fn off(note: u8) -> NodeEvent {
    NodeEvent::NoteOff { note }
}

/// Renders `frames` frames through the node in blocks of `block`, with
/// `events` at absolute frames.
fn run(n: &mut dyn Node, frames: usize, events: &[(usize, NodeEvent)], block: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(frames * 2);
    let (mut l, mut r) = (vec![0.0f32; block], vec![0.0f32; block]);
    let mut buf = Vec::new();
    let mut at = 0;
    while at < frames {
        let len = block.min(frames - at);
        buf.clear();
        for &(f, e) in events {
            if f >= at && f < at + len {
                buf.push(Timed::new((f - at) as u32, e));
            }
        }
        n.process(
            &Ctx::new(RATE, at as u64),
            &buf,
            &mut l[..len],
            &mut r[..len],
        );
        for i in 0..len {
            out.push(l[i]);
            out.push(r[i]);
        }
        at += len;
    }
    out
}

/// A phrase on the kit: hits, a held gate let go, pressure, a repeat.
fn phrase() -> Vec<(usize, NodeEvent)> {
    let b = DEFAULT_BASE_NOTE;
    vec![
        (0, on(b, 100)),
        (37, on(b + 1, 90)),
        (1_000, on(b + 2, 127)),
        (
            1_000,
            NodeEvent::Pressure {
                note: b + 1,
                value: 80,
            },
        ),
        (3_001, on(b + 3, 64)),
        (5_555, off(b + 1)),
        (6_000, on(b, 50)),
        (9_000, off(b + 3)),
        (9_100, off(b)),
        (12_345, on(b + 1, 110)),
        (15_000, NodeEvent::AllOff),
    ]
}

#[test]
fn the_node_plays_what_the_engine_plays_for_any_block() {
    let p = kit();
    // The engine itself, the events scheduled on the same frames.
    let mut e = Engine::new(RATE, VOICES);
    e.set_view(EngineView::build(&p, RATE));
    let frames = 18_000;
    let mut want = Vec::new();
    let (mut l, mut r) = (vec![0.0f32; 128], vec![0.0f32; 128]);
    let b = DEFAULT_BASE_NOTE;
    for at in (0..frames).step_by(128) {
        for (f, ev) in phrase() {
            if f < at || f >= at + 128 {
                continue;
            }
            let o = (f - at) as u32;
            let ev = match ev {
                NodeEvent::NoteOn { note, velocity } => Event::NoteOn {
                    pad: note - b,
                    velocity,
                },
                NodeEvent::NoteOff { note } => Event::NoteOff { pad: note - b },
                NodeEvent::Pressure { note, value } => Event::Pressure {
                    pad: note - b,
                    value,
                },
                _ => Event::AllOff,
            };
            e.schedule(o, ev);
        }
        e.render(&mut l, &mut r);
        for i in 0..128 {
            want.push(l[i]);
            want.push(r[i]);
        }
    }
    want.truncate(2 * frames);
    assert!(want.iter().any(|x| x.abs() > 0.05), "the kit is silent");
    for block in [1, 7, 16, 64, 128, 1000] {
        let (mut n, _c) = node(&p);
        let got = run(&mut n, frames, &phrase(), block);
        let first = got.iter().zip(&want).position(|(a, b)| a != b);
        assert_eq!(first, None, "block {block}: first difference");
        assert_eq!(got.len(), want.len());
    }
}

#[test]
fn notes_find_their_pads_and_note_offs_follow_them() {
    let (mut n, _c) = node(&kit());
    let b = DEFAULT_BASE_NOTE;
    run(
        &mut n,
        256,
        &[(0, on(b + 1, 100)), (0, on(b + 3, 100))],
        128,
    );
    assert_eq!(n.engine().pad_voices(1), 1);
    assert_eq!(n.engine().pad_voices(3), 1);
    // Pad 1 note moves up a fifth; the held notes still let go of their pads.
    n.set_param(ids::BASE_NOTE, f32::from(b + 7));
    assert_eq!(n.param(ids::BASE_NOTE), f32::from(b + 7));
    run(&mut n, 9_000, &[(0, off(b + 1)), (0, off(b + 3))], 128);
    assert_eq!(n.engine().pad_voices(1), 0, "pad 2's gate let go");
    assert_eq!(n.engine().pad_voices(3), 0, "pad 4's gate let go");
    // Now pad 1 is note b + 7.
    run(&mut n, 128, &[(0, on(b + 8, 100))], 128);
    assert_eq!(n.engine().pad_voices(1), 1);
    // A "Pad 1 note" change in a block moves the notes after it.
    let (mut n, _c) = node(&kit());
    let set = NodeEvent::Param {
        id: ids::BASE_NOTE,
        value: 60.0,
    };
    run(&mut n, 128, &[(10, set), (20, on(63, 100))], 128);
    assert_eq!(n.engine().pad_voices(3), 1);
    // Outside the kit's pads nothing plays.
    let (mut n, _c) = node(&kit());
    run(
        &mut n,
        128,
        &[(0, on(b - 1, 100)), (0, on(b + 20, 100))],
        128,
    );
    assert_eq!(n.engine().active_voices(), 0);
}

#[test]
fn a_keygroup_program_plays_every_note_as_itself() {
    let (mut n, _c) = node(&keys());
    // Notes in the pads' range too: they are notes, not pads.
    run(
        &mut n,
        256,
        &[
            (0, on(DEFAULT_BASE_NOTE, 100)),
            (0, on(57, 100)),
            (0, on(69, 100)),
        ],
        128,
    );
    assert_eq!(n.engine().active_voices(), 3);
    run(&mut n, 9_000, &[(0, off(57))], 128);
    assert_eq!(n.engine().active_voices(), 2, "only A3 let go");
    run(&mut n, 9_000, &[(0, NodeEvent::AllOff)], 128);
    assert_eq!(n.engine().active_voices(), 0);
}

#[test]
fn views_swap_in_one_per_block_and_come_back() {
    let a = kit();
    let b = keys();
    let (mut n, c) = node(&a);
    let rev = |n: &SamplerNode| n.engine().view_revision();
    let first = rev(&n);
    let vb = EngineView::build(&b, RATE);
    let b_rev = vb.revision;
    assert_ne!(first, b_rev, "the two programs must be told apart");
    assert!(c.send_view(vb).is_ok());
    assert!(c.take_old().is_none(), "nothing back before a block");
    run(&mut n, 64, &[], 64);
    assert_eq!(rev(&n), b_rev);
    assert_eq!(c.take_old().map(|v| v.revision), Some(first));
    // The new view decides what notes do: keygroups now.
    run(&mut n, 64, &[(0, on(69, 100))], 64);
    assert_eq!(n.engine().active_voices(), 1);
    // A full queue gives the view back.
    for _ in 0..VIEW_QUEUE {
        assert!(c.send_view(EngineView::build(&a, RATE)).is_ok());
    }
    assert!(c.send_view(EngineView::build(&a, RATE)).is_err());
    // One per block: two blocks take both in, and nobody takes the old
    // ones back, so the return queue is full.
    run(&mut n, 64, &[], 64);
    assert_eq!(rev(&n), first);
    run(&mut n, 64, &[], 64);
    // One more view still goes in; the view it replaces waits in the node.
    let v = EngineView::build(&b, RATE);
    assert!(c.send_view(v).is_ok());
    run(&mut n, 64, &[], 64);
    assert_eq!(rev(&n), b_rev);
    // No more go in until that one has gone back.
    assert!(c.send_view(EngineView::build(&a, RATE)).is_ok());
    run(&mut n, 64, &[], 64);
    run(&mut n, 64, &[], 64);
    assert_eq!(rev(&n), b_rev, "no room to give the old view back");
    let drain = || std::iter::from_fn(|| c.take_old()).count();
    assert_eq!(drain(), VIEW_QUEUE);
    run(&mut n, 64, &[], 64);
    assert_eq!(rev(&n), first, "room again: the waiting view went in");
    assert_eq!(drain(), 2, "the parked view and the one it made way for");
    // Nothing is lost: every view sent came back or is playing.
    run(&mut n, 64, &[], 64);
    assert_eq!(drain(), 0);
}

#[test]
fn level_lands_on_its_frame_for_any_block() {
    let p = kit();
    let b = DEFAULT_BASE_NOTE;
    let level = |db: f32| NodeEvent::Param {
        id: ids::LEVEL,
        value: db,
    };
    // Pad 3 is a 0.4 s one-shot: 17,640 frames of sound.
    let frames = 16_000;
    let events = [
        (0, on(b + 2, 127)),
        (1_000, level(-12.0)),
        (9_003, level(3.0)),
    ];
    let (mut plain, _c) = node(&p);
    let dry = run(&mut plain, frames, &events[..1], 128);
    let (mut n, _c) = node(&p);
    let want = run(&mut n, frames, &events, 128);
    assert_eq!(
        &want[..2 * 1_000],
        &dry[..2 * 1_000],
        "nothing changes before its frame"
    );
    assert_ne!(
        want[2 * 1_001],
        dry[2 * 1_001],
        "the change starts on its frame"
    );
    // The gain over 400 frames, 6,000 frames (13 time constants) after
    // each change, when the smoothing has settled.
    let at = |i: usize| {
        let rms = |x: &[f32]| x.iter().map(|v| v * v).sum::<f32>().sqrt();
        rms(&want[2 * i..2 * (i + 400)]) / rms(&dry[2 * i..2 * (i + 400)])
    };
    assert!((at(7_000) - 0.2512).abs() < 0.0005, "-12 dB: {}", at(7_000));
    assert!(
        (at(15_003) - 1.4125).abs() < 0.0005,
        "+3 dB: {}",
        at(15_003)
    );
    assert_eq!(n.param(ids::LEVEL), 3.0);
    for block in [1, 9, 333] {
        let (mut n, _c) = node(&p);
        assert!(run(&mut n, frames, &events, block) == want, "block {block}");
    }
}
