//! The scene morph through the real mod engine: two rows' stored values on
//! scenes A and B, the morph position sweeping between them, a parameter
//! only one row holds moving from where it was, snap back to the sound
//! from before (knob turns made while morphing included), keep, saving a
//! row, and turns while morphing written back into the rows.

use mpfree_mod::{ModEngine, ModEvent, ModTimed, DEFAULT_TICK};
use mpfree_node::{Ctx, Curve, NodeEvent, ParamInfo, Timed, Unit};
use mpfree_perform::{save_scene, SceneMorph};
use mpfree_seq::mpfree_param::{Owner, ParamAddr, Strip};
use mpfree_seq::Project;

const RATE: u32 = 48_000;
const MIX: ParamInfo = ParamInfo {
    id: 5,
    name: "Mix",
    group: "Out",
    min: 0.0,
    max: 100.0,
    default: 50.0,
    unit: Unit::Percent,
    curve: Curve::Linear,
    readonly: false,
};
const CUT: ParamInfo = ParamInfo {
    id: 3,
    name: "Cutoff",
    group: "Filter",
    min: 20.0,
    max: 20000.0,
    default: 1000.0,
    unit: Unit::Hz,
    curve: Curve::Log,
    readonly: false,
};

fn mix() -> ParamAddr {
    ParamAddr::new(Owner::Insert(Strip::Track(0), 1), 5)
}
fn cut() -> ParamAddr {
    ParamAddr::new(Owner::Insert(Strip::Track(0), 1), 3)
}
fn delay() -> ParamAddr {
    ParamAddr::new(Owner::Insert(Strip::Track(0), 2), 5)
}

fn engine() -> ModEngine {
    let mut e = ModEngine::new(RATE, DEFAULT_TICK);
    e.add_target(mix(), MIX, 20.0).unwrap();
    e.add_target(cut(), CUT, 1000.0).unwrap();
    e.add_target(delay(), MIX, 10.0).unwrap();
    e
}

/// Run one block with `events` at its start; the last value sent for each
/// parameter it sent.
fn block(e: &mut ModEngine, frame: u64, events: &[ModEvent]) -> Vec<(ParamAddr, f32)> {
    let evs: Vec<ModTimed> = events.iter().map(|&ev| ModTimed::new(0, ev)).collect();
    let ctx = Ctx {
        rate: RATE,
        frame,
        bpm: 120.0,
        beats: 0.0,
        playing: true,
        sidechain: None,
    };
    let mut out: Vec<(ParamAddr, f32)> = vec![];
    e.process(
        &ctx,
        128,
        &evs,
        &[None, None, None, None],
        &mut |addr, t: Timed| {
            let NodeEvent::Param { value, .. } = t.ev else {
                panic!("only params")
            };
            out.retain(|o| o.0 != addr);
            out.push((addr, value));
        },
    );
    out
}

fn sets(v: &[(ParamAddr, f32)]) -> Vec<ModEvent> {
    v.iter()
        .map(|&(addr, value)| ModEvent::Set { addr, value })
        .collect()
}

fn morph_to(x: f32) -> ModEvent {
    ModEvent::Set {
        addr: ParamAddr::morph_pos(0),
        value: x,
    }
}

fn near(a: Option<f32>, b: f32) -> bool {
    a.is_some_and(|a| (a - b).abs() < 0.01 * b.abs().max(1.0))
}

/// Row 0: mix 0, cutoff 200 Hz. Row 2: mix 100, cutoff 20 kHz, delay 80.
fn project() -> Project {
    let mut p = Project::new();
    let s = p.scene_mut(0).unwrap();
    s.set_value(mix(), 0.0).unwrap();
    s.set_value(cut(), 200.0).unwrap();
    let s = p.scene_mut(2).unwrap();
    s.set_value(mix(), 100.0).unwrap();
    s.set_value(cut(), 20000.0).unwrap();
    s.set_value(delay(), 80.0).unwrap();
    p
}

#[test]
fn the_morph_sweeps_between_two_rows_and_snaps_back() {
    let mut e = engine();
    let p = project();
    block(&mut e, 0, &[]);
    let mut m = SceneMorph::default();
    let loaded = m.load(&mut e, &p, 0, 2).unwrap();
    assert_eq!((loaded.morphing, loaded.unknown), (3, 0));
    assert_eq!(m.rows(), Some((0, 2)));
    // At 0 the parameters are row 0's; the delay, only row 2 has, stays
    // where it was.
    block(&mut e, 128, &[morph_to(0.0)]);
    assert!(near(e.value(mix()), 0.0) && near(e.value(cut()), 200.0));
    assert!(near(e.value(delay()), 10.0));
    // Halfway: mix halfway, cutoff halfway on its log knob, delay halfway.
    block(&mut e, 256, &[morph_to(0.5)]);
    assert!(near(e.value(mix()), 50.0), "{:?}", e.value(mix()));
    assert!(near(e.value(cut()), 2000.0), "{:?}", e.value(cut()));
    assert!(near(e.value(delay()), 45.0));
    block(&mut e, 384, &[morph_to(1.0)]);
    assert!(near(e.value(mix()), 100.0) && near(e.value(cut()), 20000.0));
    // A turn while morphing edits the nearer scene, and moves the base.
    block(
        &mut e,
        512,
        &[ModEvent::Set {
            addr: mix(),
            value: 70.0,
        }],
    );
    assert!(near(e.value(mix()), 70.0));
    // Snap back: every parameter to where it was before, the morph to 0.
    let back = m.snap_back(&mut e);
    assert_eq!(m.rows(), None);
    block(&mut e, 640, &sets(&back));
    assert!(near(e.value(mix()), 20.0), "{:?}", e.value(mix()));
    assert!(near(e.value(cut()), 1000.0) && near(e.value(delay()), 10.0));
    assert!(near(e.value(ParamAddr::morph_pos(0)), 0.0));
}

#[test]
fn keep_leaves_the_parameters_where_the_morph_has_them() {
    let mut e = engine();
    let p = project();
    let mut m = SceneMorph::default();
    m.load(&mut e, &p, 0, 2).unwrap();
    block(&mut e, 0, &[morph_to(0.25)]);
    let kept = m.keep(&mut e);
    block(&mut e, 128, &sets(&kept));
    assert!(near(e.base(mix()), 25.0), "{:?}", e.base(mix()));
    assert!(near(e.value(mix()), 25.0));
    // No longer morphing: the position moves nothing.
    block(&mut e, 256, &[morph_to(1.0)]);
    assert!(near(e.value(mix()), 25.0));
}

#[test]
fn reloading_while_morphing_keeps_the_first_snapshot() {
    let mut e = engine();
    let mut p = project();
    p.scene_mut(1).unwrap().set_value(mix(), 60.0).unwrap();
    let mut m = SceneMorph::default();
    m.load(&mut e, &p, 0, 2).unwrap();
    block(&mut e, 0, &[morph_to(1.0)]);
    m.load(&mut e, &p, 2, 1).unwrap();
    block(&mut e, 128, &[morph_to(1.0)]);
    assert!(near(e.value(mix()), 60.0));
    // Row 1 has no cutoff: its side is the snapshot's 1000 Hz, not the
    // 20 kHz the first morph had it at.
    assert!(near(e.value(cut()), 1000.0), "{:?}", e.value(cut()));
    let back = m.snap_back(&mut e);
    block(&mut e, 256, &sets(&back));
    assert!(near(e.value(mix()), 20.0) && near(e.value(cut()), 1000.0));
}

#[test]
fn a_row_saves_the_sound_and_stores_turns_made_while_morphing() {
    let mut e = engine();
    let mut p = Project::new();
    block(&mut e, 0, &[]);
    // Three targets and eight macros.
    assert_eq!(save_scene(&e, &mut p, 3), Some(11));
    assert_eq!(p.scene(3).unwrap().value(mix()), Some(20.0));
    assert_eq!(save_scene(&e, &mut p, 16), None, "no such row");
    let mut p = project();
    let mut m = SceneMorph::default();
    m.load(&mut e, &p, 0, 2).unwrap();
    block(&mut e, 128, &[morph_to(0.9)]);
    // Turned near B: B's mix becomes 40.
    block(
        &mut e,
        256,
        &[ModEvent::Set {
            addr: mix(),
            value: 40.0,
        }],
    );
    assert!(m.store(&e, &mut p) >= 1);
    assert!(near(p.scene(2).unwrap().value(mix()), 40.0));
    assert!(near(p.scene(0).unwrap().value(mix()), 0.0));
}

#[test]
fn values_for_parameters_the_engine_lacks_are_counted_not_loaded() {
    let mut e = engine();
    let mut p = project();
    let stray = ParamAddr::new(Owner::Insert(Strip::Track(5), 1), 1);
    p.scene_mut(0).unwrap().set_value(stray, 1.0).unwrap();
    let mut m = SceneMorph::default();
    let l = m.load(&mut e, &p, 0, 2).unwrap();
    assert_eq!((l.morphing, l.unknown), (3, 1));
    assert!(m.load(&mut e, &p, 0, 16).is_err());
}
