//! The CLAP host hands a test synth its tuning (tunings step T6): each
//! note plays where the tuning puts it, a slide moves along the tuning's
//! keys, held notes move when the tuning does, and 12-TET at A4 = 440 Hz
//! sounds exactly as before.

use mpfree_clap::testplug::{self, synth_params, SYNTH_ID};
use mpfree_clap::{ClapNode, Library};
use mpfree_node::{Ctx, Expr, Node, NodeEvent, Timed, Tuning};

const RATE: u32 = 44_100;

/// The test synth as a long sine: its decay at the most, so a held note
/// stays loud enough to measure for half a second and more.
fn synth() -> ClapNode {
    let lib = Library::from_entry(&testplug::ENTRY, "linked").expect("test entry");
    let mut n = lib.create(SYNTH_ID).expect("test synth");
    n.prepare(RATE, 512);
    let decay = n.param_index(synth_params::DECAY).expect("decay");
    let wave = n.param_index(synth_params::WAVE).expect("wave");
    n.set_param(decay, 2000.0);
    n.set_param(wave, 0.0);
    n
}

/// 5-limit just intonation on C, C4 where 12-TET has it.
fn just_on_c() -> Tuning {
    const RATIOS: [f64; 12] = [
        1.0,
        16.0 / 15.0,
        9.0 / 8.0,
        6.0 / 5.0,
        5.0 / 4.0,
        4.0 / 3.0,
        45.0 / 32.0,
        1.5,
        8.0 / 5.0,
        5.0 / 3.0,
        9.0 / 5.0,
        15.0 / 8.0,
    ];
    Tuning::from_keys(|k| {
        let from_c = k - 60;
        let octaves = from_c.div_euclid(12);
        let degree = from_c.rem_euclid(12) as usize;
        Some(c4() * RATIOS[degree] * 2f64.powi(octaves))
    })
    .expect("just on C")
}

fn c4() -> f64 {
    440.0 * 2f64.powf(-9.0 / 12.0)
}

fn tet(note: f64) -> f64 {
    440.0 * 2f64.powf((note - 69.0) / 12.0)
}

/// Renders `frames` frames in blocks of 256, `events` at absolute frames
/// and `between` run before the block that starts at its frame.
fn render(
    n: &mut ClapNode,
    events: &[(u64, NodeEvent)],
    between: &mut dyn FnMut(u64, &mut ClapNode),
    frames: usize,
) -> Vec<f32> {
    let (mut l, mut r) = (vec![0f32; frames], vec![0f32; frames]);
    let mut at = 0usize;
    let mut buf = Vec::new();
    while at < frames {
        let len = 256.min(frames - at);
        between(at as u64, n);
        buf.clear();
        for &(f, e) in events {
            if f >= at as u64 && f < (at + len) as u64 {
                buf.push(Timed::new((f - at as u64) as u32, e));
            }
        }
        n.process(
            &Ctx::new(RATE, at as u64),
            &buf,
            &mut l[at..at + len],
            &mut r[at..at + len],
        );
        at += len;
    }
    l
}

/// The pitch of one sine between two frames, from its rising zero
/// crossings.
fn hz(x: &[f32], from: usize, to: usize) -> f64 {
    let (mut first, mut last, mut cycles) = (None, 0.0, 0);
    for i in from..to - 1 {
        let (a, b) = (f64::from(x[i]), f64::from(x[i + 1]));
        if a < 0.0 && b >= 0.0 {
            let t = i as f64 + a / (a - b);
            if first.is_none() {
                first = Some(t);
            } else {
                cycles += 1;
            }
            last = t;
        }
    }
    let first = first.expect("a sound to measure");
    f64::from(cycles) * f64::from(RATE) / (last - first)
}

fn cents(got: f64, want: f64) -> f64 {
    1200.0 * (got / want).log2()
}

fn on(note: u8) -> NodeEvent {
    NodeEvent::NoteOn {
        note,
        velocity: 100,
    }
}

#[test]
fn each_note_plays_where_the_tuning_puts_it() {
    let just = just_on_c();
    for (note, want) in [
        (60, c4()),
        (64, c4() * 1.25),
        (67, c4() * 1.5),
        (71, c4() * 1.875),
    ] {
        let mut n = synth();
        assert!(n.follows_tuning(), "it takes CLAP's own notes");
        n.set_tuning(&just);
        let out = render(&mut n, &[(0, on(note))], &mut |_, _| {}, 30_000);
        let c = cents(hz(&out, 4_000, 30_000), want);
        assert!(c.abs() < 0.2, "note {note}: {c} cents off");
    }
    // Just's E4 is 13.7 cents under 12-TET's.
    assert!((cents(c4() * 1.25, tet(64.0)) + 13.686).abs() < 0.01);
}

#[test]
fn a_slide_moves_along_the_tunings_keys() {
    let mut n = synth();
    n.set_tuning(&just_on_c());
    let slide = NodeEvent::Expression {
        note: 62,
        kind: Expr::Slide,
        value: 2.0,
    };
    // D4 slid up two keys sounds at just's E4, not 12-TET's.
    let out = render(&mut n, &[(0, on(62)), (0, slide)], &mut |_, _| {}, 30_000);
    let c = cents(hz(&out, 4_000, 30_000), c4() * 1.25);
    assert!(c.abs() < 0.2, "{c} cents off just's E4");
}

#[test]
fn held_notes_move_when_the_tuning_does() {
    let mut n = synth();
    let just = just_on_c();
    let mut retune = |at: u64, n: &mut ClapNode| {
        if at == 20_480 {
            n.set_tuning(&just);
        }
    };
    let out = render(&mut n, &[(0, on(64))], &mut retune, 40_960);
    let before = cents(hz(&out, 2_000, 20_480), tet(64.0));
    let after = cents(hz(&out, 21_000, 40_960), c4() * 1.25);
    assert!(before.abs() < 0.2, "12-TET's E4 first: {before} cents off");
    assert!(after.abs() < 0.2, "then just's: {after} cents off");
}

#[test]
fn twelve_tet_at_440_sounds_as_before() {
    let notes = [
        (0, on(60)),
        (3_000, on(64)),
        (9_000, NodeEvent::NoteOff { note: 60 }),
    ];
    let plain = render(&mut synth(), &notes, &mut |_, _| {}, 20_000);
    let mut n = synth();
    n.set_tuning(&Tuning::standard());
    assert_eq!(render(&mut n, &notes, &mut |_, _| {}, 20_000), plain);
    // Back from another tuning to 12-TET, before any note: the same too.
    let mut n = synth();
    n.set_tuning(&just_on_c());
    n.set_tuning(&Tuning::standard());
    assert_eq!(render(&mut n, &notes, &mut |_, _| {}, 20_000), plain);
}
