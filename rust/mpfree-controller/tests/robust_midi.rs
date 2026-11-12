//! Robustness: any byte stream from the MPC's MIDI ports (a cable pulled
//! mid-message, a burst of junk, running status, SysEx that never ends)
//! is parsed without a panic into well-formed messages, and every message
//! decodes into a control event or none.

#[path = "../../test-support/mutate.rs"]
mod mutate;

use mpfree_controller::midi::{Message, Parser, SYSEX_CAPACITY};
use mpfree_controller::mpc_one::decode;
use mutate::Mutator;

/// Plausible MPC One traffic: pad notes and pressure, Q-Link and wheel CCs,
/// clock, and the session SysEx, back to back.
fn traffic() -> Vec<u8> {
    let mut v = Vec::new();
    for k in 0..16u8 {
        v.extend([0x99, 0x24 + k, 0x40 + k]); // pad down
        v.extend([0xA9, 0x24 + k, 0x20]); // pressure
        v.extend([0x89, 0x24 + k, 0x00]); // pad up
        v.extend([0xB0, 0x10 + (k % 4), 0x01 + k]); // Q-Link
        v.push(0xF8);
    }
    v.extend([0xF0, 0x47, 0x7F, 0x46, 0x62, 0x00, 0x01, 0x40, 0xF7]);
    v.extend([0x90, 0x30, 0x7F, 0x31, 0x7F, 0x32, 0x00]); // running status
    v
}

fn check(case: usize, msg: &Message<'_>) {
    match *msg {
        Message::Channel {
            status,
            data1,
            data2,
        } => {
            assert!((0x80..0xF0).contains(&status), "case {case}: {msg:?}");
            assert!(data1 < 0x80 && data2 < 0x80, "case {case}: {msg:?}");
        }
        Message::SysEx { bytes, .. } => {
            assert!(
                bytes.len() <= SYSEX_CAPACITY,
                "case {case}: {} bytes",
                bytes.len()
            );
            assert_eq!(bytes.first(), Some(&0xF0), "case {case}: {msg:?}");
            assert_eq!(bytes.last(), Some(&0xF7), "case {case}: {msg:?}");
        }
        Message::RealTime(b) => assert!(b >= 0xF8, "case {case}: {msg:?}"),
        Message::Common {
            status,
            data1,
            data2,
        } => {
            assert!((0xF1..=0xF7).contains(&status), "case {case}: {msg:?}");
            assert!(data1 < 0x80 && data2 < 0x80, "case {case}: {msg:?}");
        }
    }
    let _ = decode(msg);
}

#[test]
fn damaged_midi_parses_into_well_formed_messages() {
    let seed = traffic();
    let mut m = Mutator::new(10);
    let mut messages = 0usize;
    for case in 0..5_000 {
        let input = m.mutate(&seed);
        let mut p = Parser::new();
        for &b in &input {
            if let Some(msg) = p.feed(b) {
                check(case, &msg);
                messages += 1;
            }
        }
    }
    assert!(messages > 100_000, "{messages}");
}

#[test]
fn random_bytes_and_endless_sysex_are_survived() {
    let mut m = Mutator::new(11);
    let mut p = Parser::new();
    for case in 0..2_000 {
        let n = m.below(400);
        let mut input = m.bytes(n);
        if case % 5 == 0 {
            input.insert(0, 0xF0); // a SysEx that may never end
        }
        for &b in &input {
            if let Some(msg) = p.feed(b) {
                check(case, &msg);
            }
        }
    }
    // Still in step afterwards: a clean note comes through.
    let mut last = None;
    for b in [0xF7, 0x90, 0x3C, 0x64] {
        last = p.feed(b).map(|m| format!("{m:?}"));
    }
    assert_eq!(
        last.as_deref(),
        Some("Channel { status: 144, data1: 60, data2: 100 }")
    );
}
