//! Damaged datagrams: whatever arrives on the port, the reader and the
//! server answer with an error or a reply that keeps the format's promises,
//! never a panic. A datagram is the one input anyone on the network can
//! send.

#[path = "../../test-support/mutate.rs"]
mod mutate;

use mpfree_node::{Expr, NodeEvent, Timed};
use mpfree_offload::server::Server;
use mpfree_offload::wire::{self, Header, Kind, MAX_DATAGRAM, MAX_EVENTS, MAX_FRAMES};
use mpfree_offload::{node_by_name, Echo};
use mutate::Mutator;

fn good_requests() -> Vec<Vec<u8>> {
    let l: Vec<f32> = (0..128).map(|i| (i as f32 * 0.05).sin() * 0.4).collect();
    let events = [
        Timed::new(
            0,
            NodeEvent::NoteOn {
                note: 60,
                velocity: 100,
            },
        ),
        Timed::new(
            5,
            NodeEvent::Pressure {
                note: 60,
                value: 40,
            },
        ),
        Timed::new(9, NodeEvent::Bend { value: -0.5 }),
        Timed::new(
            20,
            NodeEvent::Expression {
                note: 60,
                kind: Expr::Slide,
                value: 2.0,
            },
        ),
        Timed::new(64, NodeEvent::Param { id: 3, value: 0.7 }),
        Timed::new(100, NodeEvent::NoteOff { note: 60 }),
        Timed::new(127, NodeEvent::AllOff),
    ];
    let mut out = Vec::new();
    let mut buf = vec![0u8; MAX_DATAGRAM];
    for (frames, ev, audio) in [
        (128usize, &events[..], true),
        (16, &events[..2], true),
        (128, &events[..], false),
        (1, &[][..], true),
    ] {
        let ev: Vec<Timed> = ev
            .iter()
            .copied()
            .filter(|t| (t.offset as usize) < frames)
            .collect();
        let a = audio.then_some((&l[..frames], &l[..frames]));
        let n = wire::encode(&mut buf, &Header::request(0, 1, 128, frames), &ev, a).unwrap();
        out.push(buf[..n].to_vec());
    }
    out
}

#[test]
fn damaged_datagrams_are_refused_or_read_within_the_limits() {
    let mut m = Mutator::new(0x0FF1_0AD5);
    let mut accepted = 0;
    let goods = good_requests();
    for case in 0..30_000 {
        let d = if case % 50 == 0 {
            let len = m.below(MAX_DATAGRAM + 64);
            m.bytes(len)
        } else {
            m.mutate(&goods[case % goods.len()])
        };
        if let Ok(b) = wire::parse(&d) {
            accepted += 1;
            let h = b.header;
            assert!(
                (1..=MAX_FRAMES).contains(&(h.frames as usize)),
                "case {case}"
            );
            assert!(h.events as usize <= MAX_EVENTS, "case {case}");
            assert!(h.channels == 0 || h.channels == 2, "case {case}");
            assert_eq!(h.len(), d.len(), "case {case}");
            let mut last = 0;
            for t in b.events() {
                assert!(
                    (t.offset as usize) < h.frames as usize && t.offset >= last,
                    "case {case}"
                );
                last = t.offset;
            }
            let mut ch = vec![0.0; h.frames as usize];
            if b.read_channel(0, &mut ch) {
                assert!(ch.iter().all(|s| s.is_finite()), "case {case}");
            }
        }
    }
    // Damage that leaves a datagram valid (a changed sample, a changed
    // block number) must still be read; most damage must not be.
    assert!(accepted > 1_000 && accepted < 29_000, "{accepted} accepted");
}

#[test]
fn the_server_answers_damaged_requests_with_a_valid_reply_or_nothing() {
    let mut s = Server::new(44_100);
    s.add(Box::new(Echo));
    s.add(node_by_name("cloud-eco").unwrap());
    let mut m = Mutator::new(77);
    let goods = good_requests();
    let mut answered = 0;
    for case in 0..20_000 {
        let mut d = m.mutate(&goods[case % goods.len()]);
        if d.len() >= 16 && case % 3 == 0 {
            // Keep the block numbers rising now and then, so more requests
            // get past the staleness check and reach the nodes.
            d[8..16].copy_from_slice(&(case as u64 + 10).to_le_bytes());
        }
        if let Ok(reply) = s.answer(&d) {
            answered += 1;
            let b = wire::parse(reply).expect("a reply always parses");
            assert_eq!(b.header.kind, Kind::Reply, "case {case}");
            let mut l = vec![0.0; b.header.frames as usize];
            assert!(b.read_channel(0, &mut l), "case {case}");
        }
    }
    assert!(answered > 100, "{answered} answered");
}
