//! The audio thread's side of a link allocates nothing: sending a block,
//! taking an answer, a miss. This file installs the counting allocator,
//! whose counts are per thread.

mod common;

use common::{server, signal, wait_for};
use mpfree_node::{NodeEvent, Timed};
use mpfree_offload::client::{connect, Config, Outcome};
use mpfree_prof::alloc::{self, CountingAlloc};

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

#[test]
fn process_allocates_nothing_on_the_audio_thread() {
    assert!(alloc::installed());
    let srv = server(&["echo"]);
    let (mut remote, link) = connect(srv.addr, Config::default()).unwrap();
    let (mut l, mut r) = (vec![0.0f32; 128], vec![0.0f32; 128]);
    let events = [
        Timed::new(0, NodeEvent::Param { id: 1, value: 0.5 }),
        Timed::new(
            64,
            NodeEvent::NoteOn {
                note: 60,
                velocity: 90,
            },
        ),
    ];
    let before = alloc::thread_count();
    let mut hits = 0;
    for k in 0..300u64 {
        signal(k, &mut l, &mut r);
        if remote.process(k * 128, &events, &mut l, &mut r) == Outcome::Hit {
            hits += 1;
        }
        // Now and then, don't wait: that answer races its turn.
        if k % 50 != 7 {
            wait_for(&remote, k);
        }
    }
    let used = alloc::thread_count() - before;
    assert_eq!(used, 0, "the audio thread's side allocated {used} times");
    assert!(hits > 250, "{hits} hits");
    drop(link);
}

#[test]
fn an_offloaded_send_allocates_nothing_through_a_fallback_and_back() {
    use common::{relay, wait_sent};
    use mpfree_fx::ids;
    use mpfree_node::{Ctx, Node};
    use mpfree_offload::offloaded::{cloud_send, Mode};

    assert!(alloc::installed());
    let srv = server(&["cloud-full"]);
    // Requests 50 to 89 are lost: the Eco stand-in takes over, then the
    // server comes back with a crossfade.
    let lossy = relay(srv.addr, 50..90);
    let (remote, link) = connect(lossy.addr, Config::default()).unwrap();
    let mut node = cloud_send(remote);
    node.prepare(44_100, 128);
    let (mut l, mut r) = (vec![0.0f32; 128], vec![0.0f32; 128]);
    let mut modes = Vec::with_capacity(220);
    let before = alloc::thread_count();
    for k in 0..220u64 {
        if k >= 2 && !(50..90).contains(&(k - 2)) {
            wait_for(node.remote(), k - 2);
        }
        let events = [Timed::new(
            32,
            NodeEvent::Param {
                id: ids::cloud::SIZE,
                value: 40.0 + (k % 10) as f32,
            },
        )];
        if k % 7 == 0 {
            node.set_param(ids::cloud::DECAY, 2.0 + (k % 3) as f32);
        }
        signal(k, &mut l, &mut r);
        node.process(&Ctx::new(44_100, k * 128), &events, &mut l, &mut r);
        modes.push(node.mode());
        wait_sent(node.remote());
    }
    let used = alloc::thread_count() - before;
    assert_eq!(used, 0, "the offloaded send allocated {used} times");
    for m in [Mode::Bridge, Mode::Local, Mode::Return] {
        assert!(modes.contains(&m), "never reached {m:?}");
    }
    assert_eq!(node.mode(), Mode::Remote);
    drop(link);
}
