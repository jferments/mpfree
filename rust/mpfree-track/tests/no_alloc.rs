//! The audio thread's part of the host never allocates or frees: notes,
//! parameters, tunings, swaps (instruments made beforehand), ring-outs, STOP and
//! the sequencer's sink, with Poly and FM, into the real mixer.

use mpfree_fx::{Mixer, MixerConfig};
use mpfree_node::Ctx;
use mpfree_play::{Event, Out, Sink, ALL};
use mpfree_prof::alloc::{installed, thread_count, CountingAlloc};
use mpfree_synth::{Fm, Poly};
use mpfree_track::{Instrument, TrackHost};

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

const RATE: u32 = 44_100;

#[test]
fn the_audio_threads_part_never_allocates() {
    assert!(installed(), "the counting allocator is on for this test");
    let cfg = MixerConfig {
        inputs: 0,
        pads: 0,
        tracks: 4,
        slots: 2,
        max_latency_ms: 10.0,
    };
    let mut mixer = Mixer::new(cfg, RATE, 256);
    let mut host = TrackHost::new(4, RATE, 256);
    // Made on the "control thread", before the watch starts.
    let mut spare: Vec<Option<Instrument>> = (0..8)
        .map(|k| {
            Some(if k % 2 == 0 {
                Instrument::new(Box::new(Poly::new(RATE)), RATE, 256)
            } else {
                Instrument::new(Box::new(Fm::new(RATE)), RATE, 256)
            })
        })
        .collect();
    let tunings = [
        mpfree_node::Tuning::equal(24, 440.0),
        mpfree_node::Tuning::equal(12, 432.0),
        mpfree_node::Tuning::standard(),
    ];
    host.swap(0, spare[0].take()).unwrap();
    host.swap(1, spare[1].take()).unwrap();
    host.set_profiling(true);
    let (mut l, mut r) = (vec![0f32; 128], vec![0f32; 128]);
    let mut retired = Vec::with_capacity(16);
    // Track 1 frozen for a while: a take made beforehand, and the thawed
    // one kept (the control thread would drop it).
    let mut take = Some(mpfree_fx::Frozen {
        left: vec![0.01; 4096],
        right: vec![0.01; 4096],
        origin: 0.0,
        beats: 4.0,
        bpm: 120.0,
        rate: RATE,
    });
    let mut thawed = Vec::with_capacity(2);
    let before = thread_count();
    let mut frame = 0u64;
    for p in 0..3000usize {
        if p % 7 == 0 {
            host.note_on(
                p % 2,
                (p % 128) as u32,
                40 + (p % 40) as u8,
                60 + (p % 60) as u8,
            );
        }
        if p % 11 == 0 {
            host.note_off((p + 1) % 2, 5, 40 + ((p + 9) % 40) as u8);
        }
        if p % 13 == 0 {
            host.send(Out {
                offset: 7,
                frame,
                track: 0,
                dest: (p % 3) as u16,
                event: Event::NoteOn {
                    note: 60,
                    velocity: 90,
                },
            });
        }
        if p % 17 == 0 {
            host.send(Out {
                offset: 9,
                frame,
                track: 0,
                dest: (p % 3) as u16,
                event: Event::NoteOff { note: 60 },
            });
        }
        if p % 5 == 0 {
            host.param(
                0,
                (p % 128) as u32,
                mpfree_synth::params::CUTOFF,
                200.0 + (p % 4000) as f32,
            );
            host.param(
                1,
                (p % 64) as u32,
                mpfree_synth::fm_params::FEEDBACK,
                (p % 8) as f32,
            );
        }
        if p % 450 == 0 {
            host.set_tuning((p / 450) % 3, &tunings[(p / 450) % 3]);
        }
        if p % 600 == 300 {
            let k = 2 + (p / 600) % 6;
            let _ = host.swap(p / 600 % 2, spare[k].take());
        }
        if p % 23 == 0 {
            // Load shedding: the quietest voice across the slots.
            if let Some((slot, _)) = host.cull_candidate() {
                host.cull(slot);
            }
            let _ = host.voices();
        }
        if p == 2000 {
            host.send(Out {
                offset: 0,
                frame,
                track: ALL,
                dest: ALL,
                event: Event::AllOff,
            });
        }
        if p == 2500 {
            host.stop();
        }
        if p == 1000 {
            if let Some(t) = take.take() {
                assert!(mixer.freeze(mpfree_fx::StripId::Track(1), t).is_ok());
            }
        }
        if p == 1800 {
            assert!(mixer.thaw(mpfree_fx::StripId::Track(1)));
        }
        let c = Ctx::new(RATE, frame);
        host.render(&c, 128, &mut mixer);
        l.fill(0.0);
        r.fill(0.0);
        mixer.process(&c, &mut l, &mut r);
        if let Some(t) = mixer.pop_thawed() {
            if thawed.len() < thawed.capacity() {
                thawed.push(t);
            }
        }
        while let Some(old) = host.pop_retired() {
            // The control thread would drop these; here they wait.
            if retired.len() < retired.capacity() {
                retired.push(old);
            }
        }
        let _ = (
            host.stats(0),
            host.param_value(1, 0),
            host.probe_mut(0).map(|p| p.take_window()),
        );
        frame += 128;
    }
    let calls = thread_count() - before;
    assert_eq!(calls, 0, "allocations or frees on the audio thread's path");
    assert!(host.stats(0).notes + host.stats(1).notes > 300);
    assert!(!retired.is_empty(), "swapped-out instruments came back");
    assert!(host.stats(1).rested > 500, "track 1's instrument rested");
    assert_eq!(thawed.len(), 1, "the take came back");
}
