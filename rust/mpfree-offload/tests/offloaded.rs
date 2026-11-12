//! The offloaded node over a real loopback link: the server's answer while
//! the link holds, a lost block faded out, the stand-in when the link stays
//! down, the crossfade back, a Cloud send landing where it would have here,
//! and a reset that clears the server's tail.
//!
//! Each test waits for the answers it expects (up to 10 s) instead of
//! racing the period, so a busy machine can't fail it; the losses are made
//! by a relay or by stopping the server, so they are the same every run.

mod common;

use std::f32::consts::FRAC_PI_2;
use std::net::{SocketAddr, UdpSocket};

use common::{relay, server, signal, wait_for, wait_sent};
use mpfree_fx::{ids, Cloud};
use mpfree_node::{Ctx, Kind, Node, NodeInfo, ParamInfo, Timed};
use mpfree_offload::client::{connect, Config, Link};
use mpfree_offload::offloaded::{cloud_send, Mode, Offloaded, RECOVER_AFTER, RETURN_FRAMES};

const N: usize = 128;
const RATE: u32 = 44_100;
/// The default latency, in periods.
const L: u64 = 2;

/// The stand-in for an `echo` partition: half its input, so a test can
/// tell which side it hears.
struct Half;

static HALF: NodeInfo = NodeInfo {
    id: "test.half",
    name: "Half",
    kind: Kind::Effect,
};

impl Node for Half {
    fn info(&self) -> &'static NodeInfo {
        &HALF
    }
    fn params(&self) -> &'static [ParamInfo] {
        &[]
    }
    fn set_param(&mut self, _id: u16, _value: f32) {}
    fn param(&self, _id: u16) -> f32 {
        f32::NAN
    }
    fn prepare(&mut self, _rate: u32, _max_block: usize) {}
    fn reset(&mut self) {}
    fn process(&mut self, _ctx: &Ctx, _events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        for s in left.iter_mut().chain(right.iter_mut()) {
            *s *= 0.5;
        }
    }
}

/// A 220 Hz sine at 0.5 on the absolute frame: smooth, so a click shows
/// as a jump between two samples.
fn sine(k: u64, l: &mut [f32], r: &mut [f32]) {
    for i in 0..l.len() {
        let t = (k * N as u64 + i as u64) as f64 / RATE as f64;
        let v = (0.5 * (2.0 * std::f64::consts::PI * 220.0 * t).sin()) as f32;
        l[i] = v;
        r[i] = -v;
    }
}

fn silence(_k: u64, l: &mut [f32], r: &mut [f32]) {
    l.fill(0.0);
    r.fill(0.0);
}

/// The largest jump between two samples in a row. The sine alone moves at
/// most 0.0157 per sample.
fn max_step(blocks: &[Vec<f32>]) -> f32 {
    let all: Vec<f32> = blocks.iter().flatten().copied().collect();
    all.windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

/// What a run gave, block by block (left channel).
#[derive(Default)]
struct Run {
    input: Vec<Vec<f32>>,
    out: Vec<Vec<f32>>,
    modes: Vec<Mode>,
}

/// Plays blocks `from..to` through `node`. Before each, waits for the
/// answer due, unless `answered` says its request never reached a server;
/// after each, waits until its request is sent.
fn play(
    node: &mut Offloaded,
    from: u64,
    to: u64,
    input: fn(u64, &mut [f32], &mut [f32]),
    answered: impl Fn(u64) -> bool,
    run: &mut Run,
) {
    let (mut l, mut r) = (vec![0.0f32; N], vec![0.0f32; N]);
    for k in from..to {
        if k >= L && answered(k - L) {
            wait_for(node.remote(), k - L);
        }
        input(k, &mut l, &mut r);
        run.input.push(l.clone());
        node.process(&Ctx::new(RATE, k * N as u64), &[], &mut l, &mut r);
        run.out.push(l.clone());
        run.modes.push(node.mode());
        wait_sent(node.remote());
    }
}

/// An echo partition with the Half stand-in, through `addr`.
fn halved(addr: SocketAddr) -> (Offloaded, Link) {
    let (remote, link) = connect(addr, Config::default()).unwrap();
    let mut node = Offloaded::new(remote, Box::new(Half));
    node.prepare(RATE, N);
    (node, link)
}

fn scaled(block: &[f32], g: f32) -> Vec<f32> {
    block.iter().map(|s| s * g).collect()
}

#[test]
fn while_the_link_holds_the_server_is_heard_exactly() {
    let srv = server(&["echo"]);
    let (mut node, _link) = halved(srv.addr);
    assert_eq!(node.latency(), 2 * N as u32);
    let mut run = Run::default();
    play(&mut node, 0, 200, sine, |_| true, &mut run);
    for k in L as usize..200 {
        assert_eq!(run.out[k], run.input[k - L as usize], "block {k}");
    }
    assert!(run.modes.iter().all(|&m| m == Mode::Remote));
    let st = node.stats();
    assert_eq!((st.bridges, st.local_blocks, st.concealed), (0, 0, 0));
}

#[test]
fn one_lost_block_fades_out_and_back_and_the_stand_in_is_never_heard() {
    let srv = server(&["echo"]);
    let lossy = relay(srv.addr, 50..51);
    let (mut node, _link) = halved(lossy.addr);
    let mut run = Run::default();
    play(&mut node, 0, 150, sine, |seq| seq != 50, &mut run);
    // Block 52 would have played request 50's answer. Instead it plays the
    // last block heard (block 51's: request 49's answer) backwards from its
    // last sample, fading out.
    let heard = &run.input[49];
    let want: Vec<f32> = (0..N)
        .map(|i| heard[N - 1 - i] * (N - 1 - i) as f32 / N as f32)
        .collect();
    assert_eq!(run.out[52], want);
    // Then request 51's answer fades in.
    let want: Vec<f32> = (0..N)
        .map(|i| run.input[51][i] * (i + 1) as f32 / N as f32)
        .collect();
    assert_eq!(run.out[53], want);
    for k in (L as usize..52).chain(54..150) {
        assert_eq!(run.out[k], run.input[k - L as usize], "block {k}");
    }
    assert_eq!(run.modes[52], Mode::Bridge);
    assert_eq!(run.modes[53], Mode::Remote);
    let st = node.stats();
    assert_eq!(
        (st.concealed, st.bridges, st.fallbacks, st.local_blocks),
        (1, 1, 0, 1)
    );
    assert!(max_step(&run.out) < 0.03, "a click: {}", max_step(&run.out));
}

#[test]
fn a_long_outage_falls_back_to_the_stand_in_and_crossfades_back() {
    let srv = server(&["echo"]);
    let lossy = relay(srv.addr, 50..90);
    let (mut node, _link) = halved(lossy.addr);
    let mut run = Run::default();
    let lost = 50..90u64;
    play(
        &mut node,
        0,
        220,
        sine,
        |seq| !lost.contains(&seq),
        &mut run,
    );
    let at = |k: usize| run.modes[k];
    // The first miss (block 52) starts the stand-in unheard; L periods
    // later it is heard, faded in.
    assert_eq!(
        (at(51), at(52), at(53), at(54)),
        (Mode::Remote, Mode::Bridge, Mode::Bridge, Mode::Local)
    );
    assert_eq!(run.out[53], vec![0.0; N]);
    let want: Vec<f32> = (0..N)
        .map(|i| run.input[52][i] * 0.5 * (i + 1) as f32 / N as f32)
        .collect();
    assert_eq!(run.out[54], want);
    // The stand-in, held back by the link's latency, lines up exactly.
    for k in 55..123 {
        assert_eq!(run.out[k], scaled(&run.input[k - 2], 0.5), "block {k}");
    }
    // Answers come again from block 92; after RECOVER_AFTER of them the
    // crossfade back starts, and lasts RETURN_FRAMES.
    let back = 92 + RECOVER_AFTER as usize - 1;
    let fade = RETURN_FRAMES / N;
    assert_eq!((at(back - 1), at(back)), (Mode::Local, Mode::Return));
    assert_eq!(
        (at(back + fade - 2), at(back + fade - 1)),
        (Mode::Return, Mode::Remote)
    );
    for (j, k) in (back..back + fade).enumerate() {
        let x = &run.input[k - 2];
        for i in 0..N {
            let t = ((j * N + i + 1) as f32 / RETURN_FRAMES as f32).min(1.0);
            let want = x[i] * (t * FRAC_PI_2).sin() + x[i] * 0.5 * (t * FRAC_PI_2).cos();
            assert!((run.out[k][i] - want).abs() < 1e-6, "block {k} frame {i}");
        }
    }
    for k in back + fade..220 {
        assert_eq!(run.out[k], run.input[k - 2], "block {k}");
    }
    let st = node.stats();
    assert_eq!(
        (st.concealed, st.bridges, st.fallbacks, st.returns),
        (1, 1, 1, 1)
    );
    assert!(max_step(&run.out) < 0.03, "a click: {}", max_step(&run.out));
}

#[test]
fn when_the_server_stops_the_song_keeps_playing_on_the_stand_in() {
    let srv = server(&["echo"]);
    let (mut node, _link) = halved(srv.addr);
    let mut run = Run::default();
    play(&mut node, 0, 100, sine, |_| true, &mut run);
    // Let the last two answers in, then unplug.
    wait_for(node.remote(), 98);
    wait_for(node.remote(), 99);
    drop(srv);
    play(&mut node, 100, 300, sine, |seq| seq < 100, &mut run);
    assert_eq!(run.out[101], run.input[99]);
    assert_eq!(
        (run.modes[102], run.modes[104]),
        (Mode::Bridge, Mode::Local)
    );
    for k in 105..300 {
        assert_eq!(run.out[k], scaled(&run.input[k - 2], 0.5), "block {k}");
    }
    assert!(max_step(&run.out) < 0.03, "a click: {}", max_step(&run.out));
}

#[test]
fn with_no_server_the_stand_in_plays_from_the_start() {
    let gone = UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let (mut node, _link) = halved(gone);
    let mut run = Run::default();
    play(&mut node, 0, 60, sine, |_| false, &mut run);
    assert_eq!(
        &run.modes[..5],
        &[
            Mode::Remote,
            Mode::Remote,
            Mode::Bridge,
            Mode::Bridge,
            Mode::Local
        ]
    );
    for k in 5..60 {
        assert_eq!(run.out[k], scaled(&run.input[k - 2], 0.5), "block {k}");
    }
    // Nothing was heard before the misses, so nothing was faded out.
    assert_eq!(node.stats().concealed, 0);
}

/// The first frame louder than `floor`, counting from block `from`.
fn onset(blocks: &[Vec<f32>], from: usize, floor: f32) -> Option<usize> {
    blocks[from..]
        .iter()
        .flatten()
        .position(|s| s.abs() > floor)
}

fn impulse_at_400(k: u64, l: &mut [f32], r: &mut [f32]) {
    silence(k, l, r);
    if k == 400 {
        l[0] = 1.0;
        r[0] = 1.0;
    }
}

/// Cloud here, all wet, at `quality` and `pre_delay` ms.
fn cloud(quality: f32, pre_delay: f32) -> Cloud {
    let mut c = Cloud::new();
    c.set_param(ids::cloud::QUALITY, quality);
    c.prepare(RATE, N);
    c.set_param(ids::cloud::MIX, 100.0);
    c.set_param(ids::cloud::PRE_DELAY, pre_delay);
    c
}

fn render(
    node: &mut dyn Node,
    blocks: u64,
    input: fn(u64, &mut [f32], &mut [f32]),
) -> Vec<Vec<f32>> {
    let (mut l, mut r) = (vec![0.0f32; N], vec![0.0f32; N]);
    (0..blocks)
        .map(|k| {
            input(k, &mut l, &mut r);
            node.process(&Ctx::new(RATE, k * N as u64), &[], &mut l, &mut r);
            l.clone()
        })
        .collect()
}

#[test]
fn a_cloud_send_lands_where_cloud_here_would() {
    let srv = server(&["cloud-full"]);
    let (remote, _link) = connect(srv.addr, Config::default()).unwrap();
    let mut node = cloud_send(remote);
    node.prepare(RATE, N);
    assert_eq!(node.latency(), 0);
    let mut run = Run::default();
    play(&mut node, 0, 600, impulse_at_400, |_| true, &mut run);
    // The server takes the link's 5.8 ms off the pre-delay (10 ms by
    // default): the same Cloud Full here with the full pre-delay starts its
    // tail on the same frame.
    let mut here = cloud(0.0, 10.0);
    let want = render(&mut here, 600, impulse_at_400);
    let (a, b) = (onset(&run.out, 400, 1e-6), onset(&want, 400, 1e-6));
    assert!(a.is_some() && b.is_some(), "no tail: {a:?} {b:?}");
    let (a, b) = (a.unwrap(), b.unwrap());
    assert!(
        a.abs_diff(b) <= 1,
        "the tail starts at {a} offloaded, {b} here"
    );
    assert!(a >= 400, "the tail started before the pre-delay: {a}");
    assert_eq!(node.mode(), Mode::Remote);
    assert_eq!(node.stats().local_blocks, 0);
}

#[test]
fn the_server_renders_cloud_with_the_same_parameters_bit_for_bit() {
    let srv = server(&["cloud-full"]);
    let (remote, _link) = connect(srv.addr, Config::default()).unwrap();
    let mut node = cloud_send(remote);
    node.prepare(RATE, N);
    let d_ms = (2 * N) as f32 * 1000.0 / RATE as f32;
    // Cloud Full here as the server has it: every parameter as set, the
    // pre-delay shortened by the link's latency.
    let mut here = Cloud::new();
    here.set_param(ids::cloud::QUALITY, 0.0);
    here.prepare(RATE, N);
    for p in node.params() {
        let v = node.param(p.id);
        let v = if p.id == ids::cloud::PRE_DELAY {
            (v - d_ms).max(0.0)
        } else {
            v
        };
        here.set_param(p.id, v);
    }
    let (mut l, mut r) = (vec![0.0f32; N], vec![0.0f32; N]);
    let mut want: Vec<(Vec<f32>, Vec<f32>)> = Vec::new();
    for k in 0..400u64 {
        if k >= L {
            wait_for(node.remote(), k - L);
        }
        // Changes on the way: one set between blocks, one timed.
        let mut events = Vec::new();
        if k == 150 {
            node.set_param(ids::cloud::DECAY, 4.0);
            here.set_param(ids::cloud::DECAY, 4.0);
        }
        if k == 250 {
            node.set_param(ids::cloud::PRE_DELAY, 30.0);
            here.set_param(ids::cloud::PRE_DELAY, 30.0 - d_ms);
        }
        if k == 200 {
            events.push(Timed::new(
                64,
                mpfree_node::NodeEvent::Param {
                    id: ids::cloud::SIZE,
                    value: 80.0,
                },
            ));
        }
        signal(k, &mut l, &mut r);
        let (mut el, mut er) = (l.clone(), r.clone());
        here.process(&Ctx::new(RATE, k * N as u64), &events, &mut el, &mut er);
        want.push((el, er));
        node.process(&Ctx::new(RATE, k * N as u64), &events, &mut l, &mut r);
        if k >= L {
            let (el, er) = &want[(k - L) as usize];
            assert!(*el == l && *er == r, "block {k} differs");
        }
    }
    assert_eq!(node.param(ids::cloud::PRE_DELAY), 30.0);
    assert_eq!(node.stats().local_blocks, 0);
}

#[test]
fn a_cloud_stand_in_is_silent_for_its_first_l_periods() {
    // The stand-in's pre-delay is at least the link's latency, so its
    // first L periods are silent and it can start unheard.
    let d_ms = (2 * N) as f32 * 1000.0 / RATE as f32;
    let mut eco = cloud(2.0, d_ms);
    // A reset settles the pre-delay at its target (a stopped stand-in is
    // reset), so this is the shortest it can be.
    eco.reset();
    let out = render(&mut eco, 40, signal);
    assert!(out[..L as usize].iter().flatten().all(|&s| s == 0.0));
    assert!(
        out.iter().flatten().any(|s| s.abs() > 1e-3),
        "no sound at all"
    );
}

#[test]
fn with_no_server_the_cloud_stand_in_lands_on_time_too() {
    let gone = UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let (remote, _link) = connect(gone, Config::default()).unwrap();
    let mut node = cloud_send(remote);
    node.prepare(RATE, N);
    let mut run = Run::default();
    play(&mut node, 0, 600, impulse_at_400, |_| false, &mut run);
    assert_eq!(node.mode(), Mode::Local);
    let mut here = cloud(2.0, 10.0);
    let want = render(&mut here, 600, impulse_at_400);
    let (a, b) = (
        onset(&run.out, 400, 1e-6).unwrap(),
        onset(&want, 400, 1e-6).unwrap(),
    );
    assert!(
        a.abs_diff(b) <= 2,
        "the tail starts at {a} on the stand-in, {b} here"
    );
}

#[test]
fn reset_clears_the_servers_tail() {
    let srv = server(&["cloud-full"]);
    let (remote, _link) = connect(srv.addr, Config::default()).unwrap();
    let mut node = cloud_send(remote);
    node.prepare(RATE, N);
    let mut run = Run::default();
    play(&mut node, 0, 100, signal_on, |_| true, &mut run);
    assert!(
        run.out[99].iter().any(|s| s.abs() > 1e-3),
        "no tail to clear"
    );
    // Let the answers already asked for come in, so they are the ones the
    // reset plays as silence.
    wait_for(node.remote(), 98);
    wait_for(node.remote(), 99);
    node.reset();
    let mut after = Run::default();
    play(&mut node, 100, 200, silence, |_| true, &mut after);
    assert!(
        after.out.iter().flatten().all(|&s| s == 0.0),
        "the server's tail rang on after the reset"
    );
    assert_eq!(node.mode(), Mode::Remote);
}

/// `signal`, with the noise on all the time.
fn signal_on(k: u64, l: &mut [f32], r: &mut [f32]) {
    signal(k / 40 * 80 + k % 40, l, r);
}
