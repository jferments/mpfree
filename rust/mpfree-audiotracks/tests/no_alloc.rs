//! The real-time calls (`command`, `process`, and the getters a screen
//! reads each frame) must not allocate or free. A counting global
//! allocator checks that across a busy session: recording on several
//! tracks at once, punch in and out, loop recording, locates, a tempo
//! change, comping, every edit, undo and redo, forgetting takes, a full
//! memory, latency changes and stops, a take loaded from a save and takes
//! read for one, at period sizes from 1 to 4096.
//!
//! This test file is its own program, so its global allocator (the only
//! unsafe code here, a pass-through to the system allocator) does not
//! affect the library, which forbids unsafe code. Only the test's own
//! thread is watched. (The pattern of mpfree-looper's no_alloc test.)

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use mpfree_audiotracks::save::TakeFile;
use mpfree_audiotracks::{AudioTracks, Command as A, Input, Monitor, MAX_TRACKS};
use mpfree_transport::{Block, Command as T, Transport};

struct Counting;
thread_local! {
    static WATCH: Cell<bool> = const { Cell::new(false) };
}
static CALLS: AtomicUsize = AtomicUsize::new(0);

fn count() {
    if WATCH.with(Cell::get) {
        CALLS.fetch_add(1, Ordering::Relaxed);
    }
}
fn watch(on: bool) {
    WATCH.with(|w| w.set(on));
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        count();
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        count();
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn commands_and_periods_never_allocate() {
    const RATE: u32 = 22_050;
    const BAR: u64 = 22_050; // 240 BPM
    const BT: i64 = 3_840;
    for period in [1usize, 64, 333, 4096] {
        let mut t = Transport::new(RATE, 240.0).unwrap();
        // 6 s: fills up during the session.
        let mut a = AudioTracks::new(RATE, 6);
        let mut b = Block::new();
        let mut input = vec![0.0f32; 2 * period];
        let mut out = vec![0.0f32; 2 * period];
        for (j, s) in input.iter_mut().enumerate() {
            *s = ((j * 31) % 97) as f32 / 97.0 - 0.5;
        }
        // Commands at bar k, transport commands at bar k too.
        let script: Vec<(u64, Vec<A>, Vec<T>)> = vec![
            (
                0,
                vec![
                    A::Arm(0, true),
                    A::Arm(1, true),
                    A::SetInput(1, Input::Stereo),
                    A::SetLatency(200),
                ],
                vec![T::Record],
            ),
            (
                2,
                vec![A::Arm(2, true), A::SetMonitor(2, Monitor::In)],
                vec![],
            ),
            (3, vec![], vec![T::Record]),
            (4, vec![], vec![T::Record]),
            (5, vec![], vec![T::SetLoop(Some((0, BT))), T::Locate(0)]),
            (8, vec![], vec![T::Stop, T::SetLoop(None)]),
            (
                9,
                vec![
                    A::Comp {
                        take: 0,
                        from: 10,
                        to: 2000,
                    },
                    A::Split { track: 1, at: 1000 },
                    A::Move {
                        track: 1,
                        at: 1500,
                        to: 9 * BT,
                    },
                ],
                vec![T::PlayStart],
            ),
            (
                10,
                vec![
                    A::Erase {
                        track: 0,
                        from: 100,
                        to: 200,
                    },
                    A::SetGain {
                        track: 0,
                        at: 50,
                        gain: 0.5,
                    },
                    A::SetFades {
                        track: 0,
                        at: 50,
                        fade_in: 100,
                        fade_out: 100,
                    },
                    A::Undo,
                    A::Undo,
                    A::Redo,
                ],
                vec![T::SetTempo(200.0)],
            ),
            (
                11,
                vec![
                    A::SetPan(0, 0.5),
                    A::SetLevel(1, 1.5),
                    A::Mute(2, true),
                    A::Solo(1, true),
                ],
                vec![T::Record],
            ),
            (
                13,
                vec![
                    A::Arm(1, false),
                    A::Forget(3),
                    A::ClearHistory,
                    A::Forget(4),
                    A::SetActive(false),
                ],
                vec![T::Stop],
            ),
            (
                14,
                vec![A::SetActive(true), A::Arm(0, false), A::Arm(2, false)],
                vec![T::PlayStart],
            ),
        ];
        // Warm up anything lazy (the first period) before watching.
        t.process(period as u32, &mut b);
        let map = *t.tempo_map();
        a.process(&b, &map, &input, &mut out);
        let mut load = Some(TakeFile {
            id: 200,
            track: 5,
            group: 9,
            pass: 0,
            bpm: 120.0,
            anchor: 100.5,
            frames: 30_000,
            channels: 1,
            file: String::new(),
            audio: vec![0.25; 30_000],
        });
        // Whatever load_take gives back is kept here, so it is not freed
        // while watching.
        let mut held = None;
        let mut piece = [0.0f32; 512];
        let mut frame = period as u64;
        let mut next = 0;
        let end = 16 * BAR;
        CALLS.store(0, Ordering::Relaxed);
        while frame < end {
            watch(true);
            while next < script.len() && script[next].0 * BAR <= frame {
                for c in &script[next].1 {
                    a.command(*c);
                }
                for c in &script[next].2 {
                    t.command(frame, *c);
                }
                next += 1;
            }
            if frame >= BAR && load.is_some() {
                held = Some(a.load_take(load.take().unwrap()));
            }
            let _ = a.read_take(0, 1000, &mut piece);
            let _ = a.loading();
            t.process(period as u32, &mut b);
            let map = *t.tempo_map();
            out.fill(0.0);
            a.process(&b, &map, &input, &mut out);
            let _ = (
                a.stats(),
                a.memory(),
                a.history(),
                a.regions(0).len(),
                a.recording(1),
            );
            for k in 0..MAX_TRACKS {
                let _ = a.track_out(k, period).len();
            }
            let _ = a.take(0);
            watch(false);
            frame += period as u64;
        }
        assert_eq!(CALLS.load(Ordering::Relaxed), 0, "period {period}");
        assert!(matches!(held, Some(Ok(()))), "period {period}");
        assert!(a.take_returned().is_some(), "period {period}");
        assert!(a.takes().count() > 3, "period {period}");
        // A project opening empties the tracks on the audio thread too.
        watch(true);
        let _ = a.settings(0);
        let cleared = a.clear();
        watch(false);
        assert_eq!(CALLS.load(Ordering::Relaxed), 0, "clear, period {period}");
        assert!(cleared, "period {period}");
        assert_eq!(a.takes().count(), 0, "period {period}");
    }
}

#[test]
fn streaming_never_allocates_on_the_audio_thread() {
    use mpfree_audiotracks::Io;
    use std::collections::HashMap;
    const RATE: u32 = 22_050;
    const BAR: u64 = 22_050; // 240 BPM
    const BT: i64 = 3_840;
    for period in [1usize, 128, 4096] {
        let mut t = Transport::new(RATE, 240.0).unwrap();
        // 4 s of memory for 30 s of recording on two tracks.
        let mut a = AudioTracks::new(RATE, 4);
        assert!(a.stream(true));
        let mut b = Block::new();
        let mut input = vec![0.0f32; 2 * period];
        let mut out = vec![0.0f32; 2 * period];
        for (j, s) in input.iter_mut().enumerate() {
            *s = ((j * 31) % 97) as f32 / 97.0 - 0.5;
        }
        let mut disk: HashMap<(u16, u32), Vec<f32>> = HashMap::new();
        let mut reads: Vec<(u16, u32)> = Vec::new();
        let mut buf = vec![0.0f32; mpfree_audiotracks::CHUNK];
        let script: Vec<(u64, Vec<A>, Vec<T>)> = vec![
            (
                0,
                vec![
                    A::Arm(0, true),
                    A::Arm(1, true),
                    A::SetInput(1, Input::Stereo),
                ],
                vec![T::Record],
            ),
            (15, vec![A::Arm(1, false)], vec![]),
            (30, vec![A::Arm(0, false)], vec![T::Stop]),
            (
                31,
                vec![],
                vec![T::Locate(0), T::SetLoop(Some((4 * BT, 6 * BT)))],
            ),
            (32, vec![], vec![T::Play]),
            (
                40,
                vec![A::Comp {
                    take: 1,
                    from: 4 * BT,
                    to: 5 * BT,
                }],
                vec![T::Locate(10 * BT)],
            ),
            (44, vec![A::Undo], vec![T::SetLoop(None)]),
            (50, vec![], vec![T::Stop]),
        ];
        let end = 52 * BAR;
        let mut frame = 0u64;
        let mut calls = 0;
        while frame < end {
            for (bar, cmds, tcmds) in &script {
                let at = bar * BAR;
                if at >= frame && at < frame + period as u64 {
                    for c in cmds {
                        watch(true);
                        a.command(*c);
                        watch(false);
                    }
                    for c in tcmds {
                        t.command(at, *c);
                    }
                }
            }
            t.process(period as u32, &mut b);
            let map = *t.tempo_map();
            out.fill(0.0);
            CALLS.store(0, Ordering::Relaxed);
            watch(true);
            a.process(&b, &map, &input, &mut out);
            let _ = (a.stats(), a.memory(), a.unwritten(), a.peaks(0, 0, 10_000));
            watch(false);
            calls += CALLS.load(Ordering::Relaxed);
            // The disk thread's side: allocation allowed there, but the
            // tracks' calls it makes are watched.
            loop {
                watch(true);
                let r = a.take_io();
                watch(false);
                let Some(r) = r else { break };
                match r {
                    Io::Write { take, chunk } => {
                        watch(true);
                        let n = a.chunk(take, chunk).map(|c| {
                            buf[..c.len()].copy_from_slice(c);
                            c.len()
                        });
                        watch(false);
                        if let Some(n) = n {
                            disk.insert((take, chunk), buf[..n].to_vec());
                        }
                        watch(true);
                        a.written(take, chunk);
                        watch(false);
                    }
                    Io::Read { take, chunk } => reads.push((take, chunk)),
                    Io::Finish { .. } => {}
                }
            }
            for (take, chunk) in reads.drain(..) {
                let data = disk.get(&(take, chunk)).cloned();
                watch(true);
                match &data {
                    Some(d) => {
                        a.fill(take, chunk, d);
                    }
                    None => a.fill_failed(take, chunk),
                }
                watch(false);
            }
            calls += CALLS.load(Ordering::Relaxed);
            frame += period as u64;
        }
        assert_eq!(calls, 0, "allocations at period {period}");
        assert!(a.stats().full == 0, "streaming kept every recording");
        assert!(disk.len() > 20);
    }
}
