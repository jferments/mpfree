//! A server on loopback and the client: what comes back is the same node's
//! output, L blocks later, bit for bit; a lost request is one silent block;
//! no server is silence, never a hang.

mod common;

use common::{relay, server, signal, wait_for};
use mpfree_node::Ctx;
use mpfree_offload::client::{connect, Config, Outcome};
use mpfree_offload::node_by_name;
use mpfree_offload::wire::MAX_FRAMES;
use std::net::UdpSocket;

const N: usize = 128;

#[test]
fn the_answer_is_the_local_render_l_blocks_later_bit_for_bit() {
    for (name, latency) in [("cloud-full", 2u32), ("cloud-eco", 1), ("cloud-full", 5)] {
        let srv = server(&[name]);
        let cfg = Config {
            latency,
            ..Config::default()
        };
        let (mut remote, link) = connect(srv.addr, cfg).unwrap();
        assert_eq!(remote.latency_frames(), latency * N as u32);
        let mut local = node_by_name(name).unwrap();
        local.prepare(44_100, MAX_FRAMES);
        let mut history: Vec<(Vec<f32>, Vec<f32>)> = Vec::new();
        let (mut l, mut r) = (vec![0.0f32; N], vec![0.0f32; N]);
        let mut loud = false;
        for k in 0..400u64 {
            signal(k, &mut l, &mut r);
            let (mut el, mut er) = (l.clone(), r.clone());
            local.process(&Ctx::new(44_100, k * N as u64), &[], &mut el, &mut er);
            history.push((el, er));
            let out = remote.process(k * N as u64, &[], &mut l, &mut r);
            if k < latency as u64 {
                assert_eq!(out, Outcome::Warmup);
                assert!(l.iter().chain(&r).all(|&s| s == 0.0));
            } else {
                assert_eq!(out, Outcome::Hit, "{name}, block {k}");
                let (el, er) = &history[(k - latency as u64) as usize];
                assert!(
                    *el == l && *er == r,
                    "{name}: block {k} differs from the local render"
                );
                loud |= l.iter().any(|s| s.abs() > 1e-3);
            }
            wait_for(&remote, k);
        }
        assert!(loud, "{name} made no sound");
        let st = remote.stats();
        assert_eq!(
            (st.misses, st.gaps, st.ring_full, st.not_sent),
            (0, 0, 0, 0)
        );
        let m = link.close();
        assert_eq!((m.sent, m.received, m.late), (400, 400, 0));
        assert_eq!(m.round_trip_us.len(), 400);
    }
}

#[test]
fn a_lost_request_is_one_silent_block_and_the_stream_carries_on() {
    let srv = server(&["echo"]);
    let lossy = relay(srv.addr, 9);
    let (mut remote, link) = connect(lossy.addr, Config::default()).unwrap();
    let (mut l, mut r) = (vec![0.0f32; N], vec![0.0f32; N]);
    let mut sent = Vec::new();
    for k in 0..40u64 {
        signal(k, &mut l, &mut r);
        sent.push((l.clone(), r.clone()));
        let out = remote.process(k * N as u64, &[], &mut l, &mut r);
        match k {
            0 | 1 => assert_eq!(out, Outcome::Warmup),
            11 => {
                assert_eq!(out, Outcome::Miss, "block 9's request was lost");
                assert!(l.iter().chain(&r).all(|&s| s == 0.0));
            }
            _ => {
                assert_eq!(out, Outcome::Hit, "block {k}");
                let (el, er) = &sent[k as usize - 2];
                assert!(*el == l && *er == r, "block {k}");
            }
        }
        if k != 9 {
            wait_for(&remote, k);
        }
    }
    let st = remote.stats();
    assert_eq!((st.hits, st.misses, st.gaps), (37, 1, 1));
    let m = link.close();
    assert_eq!((m.sent, m.received), (40, 39));
}

#[test]
fn with_no_server_every_block_is_silent_and_nothing_hangs() {
    // One socket that never answers, and one address nobody listens on.
    let deaf = UdpSocket::bind("127.0.0.1:0").unwrap();
    let gone = {
        let s = UdpSocket::bind("127.0.0.1:0").unwrap();
        s.local_addr().unwrap()
    };
    for addr in [deaf.local_addr().unwrap(), gone] {
        let (mut remote, link) = connect(addr, Config::default()).unwrap();
        let (mut l, mut r) = (vec![0.5f32; N], vec![0.5f32; N]);
        for k in 0..30u64 {
            l.fill(0.5);
            r.fill(0.5);
            let out = remote.process(k * N as u64, &[], &mut l, &mut r);
            assert_eq!(
                out,
                if k < 2 {
                    Outcome::Warmup
                } else {
                    Outcome::Miss
                }
            );
            assert!(l.iter().chain(&r).all(|&s| s == 0.0));
        }
        assert_eq!(remote.stats().misses, 28);
        let m = link.close();
        assert_eq!(m.received, 0);
    }
}

#[test]
fn a_block_of_the_wrong_length_or_not_finite_is_not_sent() {
    let srv = server(&["echo"]);
    let (mut remote, link) = connect(srv.addr, Config::default()).unwrap();
    let (mut l, mut r) = (vec![0.1f32; 64], vec![0.1f32; 64]);
    assert_eq!(remote.process(0, &[], &mut l, &mut r), Outcome::Warmup);
    let (mut l, mut r) = (vec![f32::NAN; N], vec![0.1f32; N]);
    assert_eq!(
        remote.process(N as u64, &[], &mut l, &mut r),
        Outcome::Warmup
    );
    assert_eq!(remote.stats().not_sent, 2);
    drop(link);
}
