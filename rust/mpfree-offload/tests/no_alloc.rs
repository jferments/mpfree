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
