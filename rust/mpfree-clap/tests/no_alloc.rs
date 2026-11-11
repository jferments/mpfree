//! The audio thread's part of the CLAP host never allocates or frees:
//! notes, pressure, bend, parameters (as events and between blocks), the
//! values the plugin sends back, blocks cut to the plugin's size, and an
//! effect after the synth.

use mpfree_clap::testplug::{self, gain_params, synth_params, GAIN_ID, SYNTH_ID};
use mpfree_clap::Library;
use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_prof::alloc::{installed, thread_count, CountingAlloc};

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

const RATE: u32 = 44_100;

#[test]
fn the_audio_threads_part_never_allocates() {
    assert!(installed(), "the counting allocator is on for this test");
    let lib = Library::from_entry(&testplug::ENTRY, "linked").expect("entry");
    let mut synth = lib.create(SYNTH_ID).expect("synth");
    let mut gain = lib.create(GAIN_ID).expect("gain");
    // Activated for 64 frames, run in blocks of 128: every block is cut.
    synth.prepare(RATE, 64);
    gain.prepare(RATE, 256);
    let decay = synth.param_index(synth_params::DECAY).expect("decay");
    let wave = synth.param_index(synth_params::WAVE).expect("wave");
    let level = gain.param_index(gain_params::GAIN).expect("gain");
    let mut events: Vec<Timed> = Vec::with_capacity(16);
    let (mut l, mut r) = (vec![0f32; 128], vec![0f32; 128]);
    let before = thread_count();
    for b in 0..2000u64 {
        events.clear();
        if b % 3 == 0 {
            events.push(Timed::new(
                5,
                NodeEvent::NoteOn {
                    note: 40 + (b % 40) as u8,
                    velocity: 100,
                },
            ));
        }
        if b % 4 == 0 {
            events.push(Timed::new(
                20,
                NodeEvent::Pressure {
                    note: 40 + (b % 40) as u8,
                    value: (b % 128) as u8,
                },
            ));
        }
        if b % 5 == 0 {
            events.push(Timed::new(
                70,
                NodeEvent::NoteOff {
                    note: 40 + ((b + 7) % 40) as u8,
                },
            ));
            events.push(Timed::new(
                90,
                NodeEvent::Param {
                    id: wave,
                    value: (b % 3) as f32,
                },
            ));
        }
        if b % 7 == 0 {
            events.push(Timed::new(
                100,
                NodeEvent::Bend {
                    value: ((b % 20) as f32 - 10.0) / 10.0,
                },
            ));
        }
        if b % 500 == 499 {
            events.push(Timed::new(127, NodeEvent::AllOff));
        }
        if b % 11 == 0 {
            synth.set_param(decay, 50.0 + (b % 1000) as f32);
            gain.set_param(level, -((b % 12) as f32));
        }
        l.fill(0.0);
        r.fill(0.0);
        let c = Ctx {
            bpm: 90.0 + (b % 30) as f64,
            playing: true,
            ..Ctx::new(RATE, b * 128)
        };
        synth.process(&c, &events, &mut l, &mut r);
        gain.process(&c, &[], &mut l, &mut r);
        let _ = (synth.param(0), synth.stats(), gain.stats());
    }
    let calls = thread_count() - before;
    assert_eq!(calls, 0, "allocations or frees on the audio thread's path");
    assert_eq!(
        (
            synth.stats().errors,
            synth.stats().dropped,
            gain.stats().errors
        ),
        (0, 0, 0)
    );
    assert!(
        synth.stats().blocks >= 4000,
        "every 128-frame block ran as two 64-frame calls"
    );
}
