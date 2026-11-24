//! A CLAP synth whose note port takes only MIDI (its own test program,
//! since it sets the test synth's dialects for every synth made after):
//! its notes go as MIDI, and it plays 12-TET whatever tuning it is handed,
//! saying so (tunings step T6).

use mpfree_clap::sys::CLAP_NOTE_DIALECT_MIDI;
use mpfree_clap::testplug::{self, synth_params, SYNTH_ID};
use mpfree_clap::Library;
use mpfree_node::{Ctx, Node, NodeEvent, Timed, Tuning};
use std::sync::atomic::Ordering;

const RATE: u32 = 44_100;

#[test]
fn a_synth_that_takes_only_midi_plays_its_notes_in_12_tet() {
    testplug::NOTE_DIALECTS.store(CLAP_NOTE_DIALECT_MIDI, Ordering::Relaxed);
    let lib = Library::from_entry(&testplug::ENTRY, "linked").expect("test entry");
    let mut n = lib.create(SYNTH_ID).expect("test synth");
    n.prepare(RATE, 512);
    let decay = n.param_index(synth_params::DECAY).expect("decay");
    n.set_param(decay, 2000.0);
    assert!(!n.follows_tuning(), "it can't follow a tuning");
    // Quarter tones: a key a quarter tone, so E4 would sound a lot lower.
    n.set_tuning(&Tuning::equal(24, 440.0));
    let (mut l, mut r) = (vec![0f32; 30_000], vec![0f32; 30_000]);
    let mut at = 0;
    while at < l.len() {
        let len = 256.min(l.len() - at);
        let ev = [Timed::new(
            0,
            NodeEvent::NoteOn {
                note: 64,
                velocity: 100,
            },
        )];
        let events: &[Timed] = if at == 0 { &ev } else { &[] };
        n.process(
            &Ctx::new(RATE, at as u64),
            events,
            &mut l[at..at + len],
            &mut r[at..at + len],
        );
        at += len;
    }
    // The note reached it as MIDI, and sounds at 12-TET's E4.
    let (mut first, mut last, mut cycles) = (None, 0.0, 0);
    for i in 4_000..l.len() - 1 {
        let (a, b) = (f64::from(l[i]), f64::from(l[i + 1]));
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
    let first = first.expect("the note sounds");
    let got = f64::from(cycles) * f64::from(RATE) / (last - first);
    let want = 440.0 * 2f64.powf(-5.0 / 12.0);
    let c = 1200.0 * (got / want).log2();
    assert!(c.abs() < 0.2, "{c} cents off 12-TET's E4");
    // A note-off goes as MIDI too: the note rings out and stops.
    let off = [Timed::new(0, NodeEvent::NoteOff { note: 64 })];
    let (mut l, mut r) = (vec![0f32; 256], vec![0f32; 256]);
    n.process(&Ctx::new(RATE, 30_000), &off, &mut l, &mut r);
    let mut tail = 0f32;
    for b in 0..400u64 {
        l.fill(0.0);
        r.fill(0.0);
        n.process(&Ctx::new(RATE, 30_256 + b * 256), &[], &mut l, &mut r);
        tail = l.iter().fold(0f32, |m, s| m.max(s.abs()));
    }
    assert!(tail < 1e-4, "released: {tail}");
}
