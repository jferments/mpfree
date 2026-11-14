//! Where a song's inserts spend the mixer's time: builds the bench's
//! "Mixer, 12 tracks and 8 pads, with a song's inserts" case (EQ and
//! compressor on 6 tracks, one keyed by track 1's kick, drive and glue on
//! submixes, Cloud reverb and delay on returns, master EQ), runs it with
//! profiling on and prints every insert's mean and p99 per 128-frame block,
//! dearest first, with its share of a period. Run with `--release`; the
//! reverb runs at Full (as the bench case), or `-- light` / `-- eco` at
//! its Light or Eco (default) quality. `-- gaps` plays each track and pad
//! a quarter of the time, in turns, as a song does, so silent inserts
//! sleep (chain.rs); `-- awake` keeps every insert awake, to compare.
//! Desktop numbers; `mpfree-bench` has the MPC model.

use mpfree_fx::ids::mixer::{self as mx, output};
use mpfree_fx::mpfree_node::Ctx;
use mpfree_fx::{ids, Cloud, Compressor, Delay, Drive, Eq, Mixer, MixerConfig, StripId};

const RATE: u32 = 44_100;
const BLOCK: usize = 128;
const BLOCKS: u64 = 20_000;

fn main() {
    let quality = if std::env::args().any(|a| a == "eco") {
        2.0
    } else if std::env::args().any(|a| a == "light") {
        1.0
    } else {
        0.0
    };
    let (tracks, pads) = (12, 8);
    let mut m = Mixer::new(MixerConfig::default(), RATE, 1024);
    for p in 0..pads {
        m.set_now(StripId::Pad(p), mx::OUTPUT, output::TRACK);
        m.set_now(StripId::Pad(p), mx::send(1), -12.0);
    }
    for t in 0..tracks {
        let s = StripId::Track(t);
        m.set_now(s, mx::OUTPUT, output::bus(t % 3));
        m.set_now(s, mx::PAN, (t as f32 * 37.0) % 200.0 - 100.0);
        m.set_now(s, mx::send(0), -18.0);
        m.set_now(s, mx::send_pre(1), 1.0);
        m.set_now(s, mx::send(1), -24.0);
    }
    m.set_now(StripId::Bus(0), mx::OUTPUT, output::bus(3));
    for t in 0..6 {
        let c = m.chain_mut(StripId::Track(t)).unwrap();
        c.push(Box::new(Eq::new()));
        c.push(Box::new(Compressor::new()));
    }
    m.chain_mut(StripId::Bus(1))
        .unwrap()
        .push(Box::new(Drive::new()));
    m.chain_mut(StripId::Bus(3))
        .unwrap()
        .push(Box::new(Compressor::new()));
    m.chain_mut(StripId::Return(0))
        .unwrap()
        .push(Box::new(Cloud::new()));
    m.chain_mut(StripId::Return(0))
        .unwrap()
        .set_param(0, ids::cloud::QUALITY, quality);
    m.chain_mut(StripId::Return(1))
        .unwrap()
        .push(Box::new(Delay::new()));
    m.chain_mut(StripId::Master)
        .unwrap()
        .push(Box::new(Eq::new()));
    m.chain_mut(StripId::Track(1))
        .unwrap()
        .set_param(1, ids::compressor::SIDECHAIN, 1.0);
    m.set_key(StripId::Track(1), 1, Some(StripId::Track(0)));
    let gaps = std::env::args().any(|a| a == "gaps");
    if std::env::args().any(|a| a == "awake") {
        let strips: Vec<StripId> = m.strips().collect();
        for s in strips {
            if let Some(c) = m.chain_mut(s) {
                c.set_sleep(false);
            }
        }
    }
    // With gaps, strip `k` sounds in one turn (about 3 s) of every four.
    let sounds = |k: usize, b: u64| !gaps || (b / 1000 + k as u64).is_multiple_of(4);
    m.set_profiling(true);
    let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    let mut seed = 1u32;
    let mut noise = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 9) as f32 / (1u32 << 23) as f32 * 0.4 - 0.2
    };
    for b in 0..BLOCKS {
        for t in 0..tracks {
            if !sounds(t, b) {
                continue;
            }
            let (il, ir) = m.track_input(t).unwrap();
            for i in 0..BLOCK {
                il[i] = noise();
                ir[i] = il[i];
            }
        }
        for p in 0..pads {
            if !sounds(p, b) {
                continue;
            }
            let (il, ir) = m.pad_input(p).unwrap();
            for i in 0..BLOCK {
                il[i] = noise();
                ir[i] = il[i];
            }
        }
        m.process(&Ctx::new(RATE, b * BLOCK as u64), &mut l, &mut r);
    }
    let period_us = BLOCK as f64 / RATE as f64 * 1e6;
    let whole = m.probe().lifetime();
    println!(
        "whole mixer: mean {:.1} us ({:.1} % of a period), p99 {:.1} us ({:.1} %)",
        whole.mean_ns / 1e3,
        whole.mean_ns / 1e3 / period_us * 100.0,
        whole.p99_ns as f64 / 1e3,
        whole.p99_ns as f64 / 1e3 / period_us * 100.0
    );
    let mut rows = Vec::new();
    let strips: Vec<StripId> = m.strips().collect();
    for s in strips {
        let Some(c) = m.chain(s) else { continue };
        for slot in 0..c.len() {
            let name = c.node(slot).map_or("?", |n| n.info().name);
            let p = c.probe(slot).unwrap().lifetime();
            rows.push((
                format!("{s:?} slot {slot} {name}"),
                p.mean_ns / 1e3,
                p.p99_ns as f64 / 1e3,
            ));
        }
    }
    let inserts: f64 = rows.iter().map(|r| r.1).sum();
    println!("inserts: mean {inserts:.1} us ({:.1} %); the rest (routing, faders, meters, Master safety) {:.1} us",
             inserts / period_us * 100.0, whole.mean_ns / 1e3 - inserts);
    rows.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (what, mean, p99) in rows {
        println!(
            "  {what:<36} mean {mean:6.2} us ({:4.1} %)  p99 {p99:6.2} us",
            mean / period_us * 100.0
        );
    }
}
