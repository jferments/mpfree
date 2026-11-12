//! Tunings and per-note expression on every pitched engine (Poly, FM,
//! Wavetable, Pluck), through the node interface: a tuning moves notes to
//! its pitches, a note's slide moves only that note, values sent before a
//! note start it in place, a note-off clears them, nothing sent changes
//! nothing to the bit, and the output doesn't depend on the block size.

use mpfree_node::{Ctx, Expr, Node, NodeEvent, Timed, Tuning, MAX_BLOCK};
use mpfree_synth::{Fm, Pluck, Poly, Wavetable};
use mpfree_tuning::Builtin;

const RATE: u32 = 48_000;

const ENGINES: [&str; 4] = ["Poly", "FM", "Wavetable", "Pluck"];

/// A fresh engine in a tuning. Fresh each time: Pluck's noise bursts carry
/// on from note to note, so a reset engine doesn't repeat itself.
fn fresh(name: &str, tuning: &Tuning) -> Box<dyn Node> {
    let mut s: Box<dyn Node> = match name {
        "Poly" => Box::new(Poly::new(RATE)),
        "FM" => Box::new(Fm::new(RATE)),
        "Wavetable" => Box::new(Wavetable::new(RATE)),
        _ => Box::new(Pluck::new(RATE)),
    };
    s.set_tuning(tuning);
    s
}

fn on(note: u8) -> NodeEvent {
    NodeEvent::NoteOn {
        note,
        velocity: 110,
    }
}

fn off(note: u8) -> NodeEvent {
    NodeEvent::NoteOff { note }
}

fn slide(note: u8, value: f32) -> NodeEvent {
    NodeEvent::Expression {
        note,
        kind: Expr::Slide,
        value,
    }
}

/// Renders `frames` frames (left channel) with `events` at absolute frames,
/// in blocks of `block`.
fn render(s: &mut dyn Node, frames: usize, events: &[(u64, NodeEvent)], block: usize) -> Vec<f32> {
    let mut l = vec![0.0f32; frames];
    let mut r = vec![0.0f32; frames];
    let mut at = 0usize;
    let mut buf = Vec::new();
    while at < frames {
        let len = block.clamp(1, MAX_BLOCK).min(frames - at);
        buf.clear();
        for &(f, ev) in events {
            if f >= at as u64 && f < (at + len) as u64 {
                buf.push(Timed::new((f - at as u64) as u32, ev));
            }
        }
        let ctx = Ctx::new(RATE, at as u64);
        s.process(&ctx, &buf, &mut l[at..at + len], &mut r[at..at + len]);
        at += len;
    }
    l
}

/// The pitch in Hz of a steady stretch, by autocorrelation within 25 % of
/// `near` Hz, with the peak placed between lags.
fn pitch(x: &[f32], near: f32) -> f32 {
    let t = RATE as f32 / near;
    let (lo, hi) = ((t * 0.75) as usize, (t * 1.25) as usize + 1);
    let n = x.len() - hi - 1;
    let ac = |lag: usize| -> f64 {
        (0..n)
            .map(|i| f64::from(x[i]) * f64::from(x[i + lag]))
            .sum()
    };
    let best = (lo..=hi).max_by(|&a, &b| ac(a).total_cmp(&ac(b))).unwrap();
    let (a, b, c) = (ac(best - 1), ac(best), ac(best + 1));
    let shift = 0.5 * (a - c) / (a - 2.0 * b + c);
    RATE as f32 / (best as f32 + shift as f32)
}

fn cents(a: f32, b: f32) -> f32 {
    1200.0 * (a / b).log2()
}

/// One note's pitch over 0.1 to 0.4 s after it starts, with `events` too.
fn note_pitch(name: &str, t: &Tuning, note: u8, events: &[(u64, NodeEvent)], near: f32) -> f32 {
    let mut s = fresh(name, t);
    let mut ev = vec![(0, on(note))];
    ev.extend_from_slice(events);
    ev.sort_by_key(|e| e.0);
    let l = render(s.as_mut(), RATE as usize / 2, &ev, 256);
    pitch(&l[RATE as usize / 10..RATE as usize * 2 / 5], near)
}

#[test]
fn a_just_tuning_plays_pure_thirds_and_fifths() {
    let just = Builtin::Just.tuning(0, 440.0);
    let std = Tuning::standard();
    for name in ENGINES {
        let c = note_pitch(name, &std, 60, &[], 261.6);
        let e = note_pitch(name, &std, 64, &[], 329.6);
        assert!(
            (cents(e, c) - 400.0).abs() < 3.0,
            "{name}: 12-TET third {:.1} cents",
            cents(e, c)
        );
        let cj = note_pitch(name, &just, 60, &[], 261.6);
        let ej = note_pitch(name, &just, 64, &[], 327.0);
        let gj = note_pitch(name, &just, 67, &[], 392.0);
        assert!(
            (cents(cj, c)).abs() < 1.0,
            "{name}: C stays put: {:.2} cents",
            cents(cj, c)
        );
        assert!(
            (cents(ej, cj) - 386.31).abs() < 3.0,
            "{name}: just third {:.1} cents",
            cents(ej, cj)
        );
        assert!(
            (cents(gj, cj) - 701.96).abs() < 3.0,
            "{name}: just fifth {:.1} cents",
            cents(gj, cj)
        );
        // And back: the standard tuning plays as before.
        let mut a = fresh(name, &std);
        let mut b = fresh(name, &just);
        b.set_tuning(&std);
        let ev = [(0, on(64))];
        assert!(
            render(a.as_mut(), 9_600, &ev, 256) == render(b.as_mut(), 9_600, &ev, 256),
            "{name}: standard again"
        );
    }
}

#[test]
fn a_tuning_change_reaches_a_sounding_note() {
    for name in ENGINES {
        let mut s = fresh(name, &Tuning::standard());
        let l = render(s.as_mut(), RATE as usize / 2, &[(0, on(64))], 256);
        s.set_tuning(&Builtin::Just.tuning(0, 440.0));
        let frames = RATE as usize / 2;
        let mut r = vec![0.0f32; frames];
        let ctx = Ctx::new(RATE, frames as u64);
        let mut l2 = vec![0.0f32; frames];
        s.process(&ctx, &[], &mut l2, &mut r);
        let before = pitch(&l[RATE as usize / 10..], 329.6);
        let after = pitch(&l2[RATE as usize / 20..], 327.0);
        assert!(
            (cents(after, before) + 13.69).abs() < 3.0,
            "{name}: E moves down 13.7 cents: {:.1}",
            cents(after, before)
        );
    }
}

#[test]
fn a_slide_moves_its_own_note_only() {
    let q = RATE as u64 / 2;
    let std = Tuning::standard();
    for name in ENGINES {
        let plain = note_pitch(name, &std, 60, &[], 261.6);
        // Up a fifth, sent at once: the note plays G.
        let up = note_pitch(name, &std, 60, &[(0, slide(60, 7.0))], 392.0);
        assert!(
            (cents(up, plain) - 700.0).abs() < 3.0,
            "{name}: slide +7 gives {:.1} cents",
            cents(up, plain)
        );
        // A slide for another note changes nothing, to the bit.
        let a = render(fresh(name, &std).as_mut(), q as usize, &[(0, on(60))], 256);
        let b = render(
            fresh(name, &std).as_mut(),
            q as usize,
            &[(0, on(60)), (100, slide(61, 5.0)), (900, slide(72, -3.0))],
            256,
        );
        assert!(a == b, "{name}: another note's slide moved this one");
        // A slide part way through a held note.
        let l = render(
            fresh(name, &std).as_mut(),
            RATE as usize,
            &[(0, on(60)), (q, slide(60, 12.0))],
            256,
        );
        let late = pitch(&l[q as usize + RATE as usize / 10..], 523.3);
        assert!(
            (cents(late, plain) - 1200.0).abs() < 3.0,
            "{name}: slide +12 part way: {:.1} cents",
            cents(late, plain)
        );
    }
}

#[test]
fn a_note_off_clears_its_values_for_the_next_note() {
    let std = Tuning::standard();
    for name in ENGINES {
        let plain = note_pitch(name, &std, 60, &[], 261.6);
        let mut s = fresh(name, &std);
        let events = [
            (0, slide(60, 12.0)),
            (0, on(60)),
            (4_800, off(60)),
            (24_000, on(60)),
        ];
        let l = render(s.as_mut(), RATE as usize, &events, 256);
        let second = pitch(&l[24_000 + 4_800..24_000 + 19_200], 261.6);
        assert!(
            cents(second, plain).abs() < 3.0,
            "{name}: the next note is back at its pitch ({:.1} cents)",
            cents(second, plain)
        );
    }
}

#[test]
fn neutral_values_change_nothing_to_the_bit() {
    for name in ENGINES {
        let notes = [
            (0, on(60)),
            (500, on(64)),
            (9_000, off(60)),
            (15_000, off(64)),
        ];
        let a = render(
            fresh(name, &Tuning::standard()).as_mut(),
            24_000,
            &notes,
            128,
        );
        let mut busy = notes.to_vec();
        busy.extend([
            (0, slide(60, 0.0)),
            (
                700,
                NodeEvent::Expression {
                    note: 64,
                    kind: Expr::Release,
                    value: 0.9,
                },
            ),
            (
                800,
                NodeEvent::Expression {
                    note: 64,
                    kind: Expr::Slide,
                    value: f32::NAN,
                },
            ),
        ]);
        busy.sort_by_key(|e| e.0);
        let mut s = fresh(name, &Builtin::Just.tuning(0, 440.0));
        s.set_tuning(&Tuning::standard());
        let b = render(s.as_mut(), 24_000, &busy, 128);
        assert!(a == b, "{name}: neutral expression changed the sound");
    }
}

#[test]
fn slides_land_the_same_for_any_block_size() {
    let events = [
        (0, slide(60, -2.0)),
        (0, on(60)),
        (700, on(67)),
        (3_001, slide(67, 1.5)),
        (3_001, slide(60, 0.0)),
        (5_555, slide(67, -12.0)),
        (9_000, off(60)),
        (12_000, off(67)),
    ];
    let mean = Builtin::Meantone.tuning(2, 440.0);
    for name in ENGINES {
        let want = render(fresh(name, &mean).as_mut(), 16_000, &events, 4096);
        for block in [1, 7, 64, 1000] {
            let got = render(fresh(name, &mean).as_mut(), 16_000, &events, block);
            assert!(got == want, "{name}: block {block} differs");
        }
    }
}
