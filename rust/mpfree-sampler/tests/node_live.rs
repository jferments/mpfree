//! The sampler node's pad and kit settings: offsets on each pad's own
//! settings that reach the engine as live settings on their frames, for any
//! block size; at their defaults the pads play as they are; a new view gets
//! the offsets on its own settings; reset stops at once and keeps them.

use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_sampler::engine::{Engine, EngineView, Event, PadField, ProgramField};
use mpfree_sampler::model::*;
use mpfree_sampler::node::{
    field, ids, pad_id, SamplerControl, SamplerNode, DEFAULT_BASE_NOTE, LIVE_PADS,
};

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

/// Pad 1: a one-shot with no filter. Pad 2: a gate with a low-pass, its
/// own level, pan and tune. Pad 3: a one-shot with no attack. Pad 4: a
/// gate. `cutoff` and `level` are pad 2's.
fn kit_with(cutoff: f32, level: f32) -> Project {
    let mut p = Project::new();
    for (i, hz) in [110.0, 220.0, 330.0, 440.0].into_iter().enumerate() {
        let s = tone(&format!("t{i}"), 0.5, hz);
        p.add_source(s.clone());
        let mut r = Region::whole(&s);
        if i % 2 == 1 {
            r.mode = PlayMode::Gate;
        }
        let mut pad = Pad::new(&format!("pad {i}"), Layer::new(r));
        if i == 1 {
            pad.filter = Filter {
                kind: FilterKind::LowPass,
                poles: 2,
                cutoff_hz: cutoff,
                resonance: 0.3,
                env_octaves: 0.0,
            };
            pad.level_db = level;
            pad.pan = 0.2;
            pad.tune_cents = 50;
            pad.amp.release_ms = 40.0;
        }
        p.set_pad(i, Some(pad), None).unwrap();
    }
    p
}

fn kit() -> Project {
    kit_with(2_000.0, -3.0)
}

fn node(p: &Project) -> (SamplerNode, SamplerControl) {
    SamplerNode::new(RATE, VOICES, EngineView::build(p, RATE))
}

fn on(pad: u8, velocity: u8) -> NodeEvent {
    NodeEvent::NoteOn {
        note: DEFAULT_BASE_NOTE + pad,
        velocity,
    }
}

fn off(pad: u8) -> NodeEvent {
    NodeEvent::NoteOff {
        note: DEFAULT_BASE_NOTE + pad,
    }
}

fn set(id: u16, value: f32) -> NodeEvent {
    NodeEvent::Param { id, value }
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

/// What a pad setting at `v` asks of the engine, worked out here from the
/// rules in the node's docs: `None` for the pad's own.
fn live(view: &EngineView, pad: usize, f: u16, v: f32) -> Option<f32> {
    let field = PadField::ALL[usize::from(f)];
    let own = view.pad_own(pad, field);
    let default = if f >= field::ATTACK { 100.0 } else { 0.0 };
    if v == default {
        return None;
    }
    Some(match f {
        field::LEVEL => own.unwrap() + v,
        field::PAN => own.unwrap() + v / 100.0,
        field::TUNE => own.unwrap() + v * 100.0,
        field::CUTOFF if own.is_none() && v > 0.0 => return None,
        field::CUTOFF => own.unwrap_or(22_000.0) * v.exp2(),
        field::RESONANCE => own.unwrap_or(0.0) + v / 100.0,
        _ => own.unwrap().max(1.0) * v / 100.0,
    })
}

/// A phrase with every pad setting and both kit offsets moving, some while
/// pads play and some just before a hit.
fn phrase() -> Vec<(usize, NodeEvent)> {
    vec![
        (0, on(1, 100)),
        (0, on(0, 90)),
        (500, set(pad_id(1, field::LEVEL), -9.0)),
        (800, set(pad_id(1, field::PAN), -50.0)),
        (1_200, set(pad_id(1, field::CUTOFF), -2.0)),
        (1_500, set(pad_id(1, field::RESONANCE), 40.0)),
        (2_000, set(pad_id(0, field::CUTOFF), -3.0)),
        (2_100, on(0, 120)),
        (2_500, set(pad_id(2, field::TUNE), 7.0)),
        (2_600, on(2, 100)),
        (3_000, set(pad_id(2, field::ATTACK), 2_000.0)),
        (3_100, on(2, 100)),
        (3_333, set(pad_id(1, field::RELEASE), 300.0)),
        (4_000, set(ids::TUNE, -5.0)),
        (4_500, set(ids::CUTOFF, -1.0)),
        (5_000, off(1)),
        (6_000, set(pad_id(1, field::LEVEL), 0.0)),
        (6_001, on(1, 80)),
        (7_777, set(pad_id(3, field::DECAY), 50.0)),
        (7_800, on(3, 100)),
        (9_000, set(ids::CUTOFF, 0.0)),
        (10_000, off(3)),
        (10_000, off(1)),
    ]
}

/// The engine itself, given the live values on the phrase's frames, in
/// blocks of `block`.
fn engine_plays(p: &Project, frames: usize, block: usize) -> Vec<f32> {
    let view = EngineView::build(p, RATE);
    let mut e = Engine::new(RATE, VOICES);
    e.set_view(EngineView::build(p, RATE));
    let mut out = Vec::new();
    let (mut l, mut r) = (vec![0.0f32; block], vec![0.0f32; block]);
    for at in (0..frames).step_by(block) {
        for (f, ev) in phrase() {
            if f < at || f >= at + block {
                continue;
            }
            let o = (f - at) as u32;
            let b = DEFAULT_BASE_NOTE;
            let ev = match ev {
                NodeEvent::NoteOn { note, velocity } => Event::NoteOn {
                    pad: note - b,
                    velocity,
                },
                NodeEvent::NoteOff { note } => Event::NoteOff { pad: note - b },
                NodeEvent::Param {
                    id: ids::TUNE,
                    value,
                } => Event::ProgramLive {
                    field: ProgramField::Tune,
                    value: value * 100.0,
                },
                NodeEvent::Param {
                    id: ids::CUTOFF,
                    value,
                } => Event::ProgramLive {
                    field: ProgramField::Cutoff,
                    value,
                },
                NodeEvent::Param { id, value } => {
                    let (pad, f) = (usize::from(id / 16), id % 16);
                    Event::PadLive {
                        pad: pad as u8,
                        field: PadField::ALL[usize::from(f)],
                        value: live(&view, pad, f, value).unwrap_or(f32::NAN),
                    }
                }
                _ => unreachable!(),
            };
            e.schedule(o, ev);
        }
        e.render(&mut l, &mut r);
        for i in 0..block {
            out.push(l[i]);
            out.push(r[i]);
        }
    }
    out.truncate(2 * frames);
    out
}

#[test]
fn settings_reach_the_engine_on_their_frames_for_any_block() {
    let p = kit();
    let frames = 14_000;
    let want = engine_plays(&p, frames, 128);
    // The settings change the sound: the same phrase without them differs.
    let notes: Vec<_> = phrase()
        .into_iter()
        .filter(|(_, e)| !matches!(e, NodeEvent::Param { .. }))
        .collect();
    let (mut plain, _c) = node(&p);
    let plain = run(&mut plain, frames, &notes, 128);
    assert!(want.iter().any(|x| x.abs() > 0.05), "the kit is silent");
    assert_ne!(plain, want, "the settings changed nothing");
    for block in [1, 7, 64, 128, 1000] {
        let (mut n, _c) = node(&p);
        let got = run(&mut n, frames, &phrase(), block);
        let first = got.iter().zip(&want).position(|(a, b)| a != b);
        assert_eq!(first, None, "block {block}: first difference");
        assert_eq!(got.len(), want.len());
    }
}

#[test]
fn at_their_defaults_the_pads_play_as_they_are() {
    let p = kit();
    let notes = [
        (0, on(0, 100)),
        (0, on(1, 100)),
        (300, on(2, 90)),
        (300, on(3, 90)),
    ];
    let (mut plain, _c) = node(&p);
    let want = run(&mut plain, 4_000, &notes, 64);
    // Every setting moved away and back, and set to its default again.
    let (mut n, _c) = node(&p);
    for prm in n.params().iter().filter(|p| !p.readonly && p.id < 300) {
        let away = if prm.default == prm.max {
            prm.min
        } else {
            prm.max
        };
        n.set_param(prm.id, away);
        n.set_param(prm.id, prm.default);
        let (pad, f) = (prm.id / 16, prm.id % 16);
        let field = PadField::ALL[usize::from(f)];
        assert_eq!(
            n.engine().pad_live_value(pad as u8, field),
            None,
            "{}",
            prm.id
        );
        assert_eq!(n.param(prm.id), prm.default);
    }
    for id in [ids::TUNE, ids::CUTOFF] {
        n.set_param(id, 3.0);
        n.set_param(id, 0.0);
    }
    let mut events: Vec<_> = notes.to_vec();
    for prm in n.params().iter().filter(|p| !p.readonly) {
        events.push((100, set(prm.id, prm.default)));
    }
    let got = run(&mut n, 4_000, &events, 64);
    assert_eq!(got, want);
}

#[test]
fn pad_settings_work_from_the_pads_own() {
    let (mut n, _c) = node(&kit());
    let get = |n: &SamplerNode, pad: u8, f: PadField| n.engine().pad_live_value(pad, f);
    // Pad 2: level -3 dB, pan 0.2, tune 50 cents, low-pass at 2 kHz, 0.3.
    n.set_param(pad_id(1, field::LEVEL), -6.0);
    n.set_param(pad_id(1, field::PAN), 50.0);
    n.set_param(pad_id(1, field::TUNE), -12.0);
    n.set_param(pad_id(1, field::CUTOFF), -1.0);
    n.set_param(pad_id(1, field::RESONANCE), 50.0);
    n.set_param(pad_id(1, field::RELEASE), 50.0);
    assert_eq!(get(&n, 1, PadField::Level), Some(-9.0));
    assert_eq!(get(&n, 1, PadField::Pan), Some(0.7));
    assert_eq!(get(&n, 1, PadField::Tune), Some(-1150.0));
    assert_eq!(get(&n, 1, PadField::Cutoff), Some(1_000.0));
    assert_eq!(get(&n, 1, PadField::Resonance), Some(0.8));
    assert_eq!(get(&n, 1, PadField::Release), Some(20.0));
    // Out of the engine's range: clamped there.
    n.set_param(pad_id(1, field::PAN), 100.0);
    assert_eq!(get(&n, 1, PadField::Pan), Some(1.0));
    // Out of the setting's range: clamped to it.
    n.set_param(pad_id(1, field::LEVEL), 40.0);
    assert_eq!(n.param(pad_id(1, field::LEVEL)), 12.0);
    assert_eq!(get(&n, 1, PadField::Level), Some(9.0));
    // Pad 1 has no filter: a lower cutoff gives it a low-pass from open, a
    // higher one leaves it open.
    n.set_param(pad_id(0, field::CUTOFF), -2.0);
    assert_eq!(get(&n, 0, PadField::Cutoff), Some(5_500.0));
    n.set_param(pad_id(0, field::CUTOFF), 2.0);
    assert_eq!(get(&n, 0, PadField::Cutoff), None);
    // Pad 3 has no attack: a longer one scales from 1 ms.
    n.set_param(pad_id(2, field::ATTACK), 1_000.0);
    assert_eq!(get(&n, 2, PadField::Attack), Some(10.0));
    // The kit's offsets.
    n.set_param(ids::TUNE, -7.0);
    n.set_param(ids::CUTOFF, 1.5);
    let program = |n: &SamplerNode, f| n.engine().program_live_value(f);
    assert_eq!(program(&n, ProgramField::Tune), -700.0);
    assert_eq!(program(&n, ProgramField::Cutoff), 1.5);
    assert_eq!(n.param(ids::TUNE), -7.0);
    // A pad past the 16, and an unknown id, are not settings.
    assert!(n.param(pad_id(LIVE_PADS, field::LEVEL)).is_nan());
    assert!(n.param(pad_id(0, field::COUNT as u16)).is_nan());
    assert!(n.param(999).is_nan());
}

#[test]
fn a_new_view_gets_the_offsets_on_its_own_settings() {
    let (mut n, c) = node(&kit());
    n.set_param(pad_id(1, field::LEVEL), -6.0);
    n.set_param(pad_id(1, field::CUTOFF), 1.0);
    n.set_param(ids::TUNE, 3.0);
    // Pad 2 is now at -10 dB with its low-pass at 500 Hz.
    c.send_view(EngineView::build(&kit_with(500.0, -10.0), RATE))
        .unwrap();
    run(&mut n, 64, &[], 64);
    let get = |pad: u8, f: PadField| n.engine().pad_live_value(pad, f);
    assert_eq!(get(1, PadField::Level), Some(-16.0));
    assert_eq!(get(1, PadField::Cutoff), Some(1_000.0));
    assert_eq!(get(1, PadField::Pan), None);
    assert_eq!(get(0, PadField::Level), None);
    assert_eq!(n.engine().program_live_value(ProgramField::Tune), 300.0);
    assert!(c.take_old().is_some());
}

#[test]
fn reset_stops_at_once_and_keeps_the_settings() {
    let (mut n, _c) = node(&kit());
    n.set_param(pad_id(1, field::LEVEL), -6.0);
    n.set_param(ids::CUTOFF, -1.0);
    let held = run(&mut n, 2_000, &[(0, on(1, 100)), (0, on(3, 100))], 128);
    assert!(held[held.len() - 256..].iter().any(|x| x.abs() > 0.01));
    n.reset();
    assert_eq!(n.engine().active_voices(), 0);
    let after = run(&mut n, 256, &[], 128);
    assert!(after.iter().all(|&x| x == 0.0), "a voice played on");
    assert_eq!(n.param(pad_id(1, field::LEVEL)), -6.0);
    assert_eq!(n.engine().pad_live_value(1, PadField::Level), Some(-9.0));
    assert_eq!(n.engine().program_live_value(ProgramField::Cutoff), -1.0);
}

#[test]
fn every_pad_has_its_settings_where_drums_puts_them() {
    let (n, _c) = node(&kit());
    let params = n.params();
    assert_eq!(params.len(), LIVE_PADS * field::COUNT + 5);
    let mut ids_seen: Vec<u16> = params.iter().map(|p| p.id).collect();
    ids_seen.sort_unstable();
    ids_seen.dedup();
    assert_eq!(ids_seen.len(), params.len(), "ids are unique");
    for pad in 0..LIVE_PADS {
        for f in 0..field::COUNT as u16 {
            let p = params.iter().find(|p| p.id == pad_id(pad, f)).unwrap();
            assert_eq!(p.group, format!("Pad {}", pad + 1));
            assert_eq!(n.param(p.id), p.default);
            assert!(p.min <= p.default && p.default <= p.max);
        }
    }
    for id in [
        ids::LEVEL,
        ids::BASE_NOTE,
        ids::TUNE,
        ids::CUTOFF,
        ids::METER_VOICES,
    ] {
        assert_eq!(params.iter().filter(|p| p.id == id).count(), 1);
    }
}
