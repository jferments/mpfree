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
//! thread is watched, and counted, so tests running at once never count
//! each other's calls. (The pattern of mpfree-looper's no_alloc test.)

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use mpfree_audiotracks::save::TakeFile;
use mpfree_audiotracks::{AudioTracks, Command as A, Input, Monitor, MAX_TRACKS};
use mpfree_transport::{Block, Command as T, Transport};

struct Counting;
thread_local! {
    static WATCH: Cell<bool> = const { Cell::new(false) };
    static CALLS: Cell<usize> = const { Cell::new(0) };
}

fn count() {
    if WATCH.with(Cell::get) {
        CALLS.with(|c| c.set(c.get() + 1));
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
        CALLS.with(|c| c.set(0));
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
        assert_eq!(CALLS.with(Cell::get), 0, "period {period}");
        assert!(matches!(held, Some(Ok(()))), "period {period}");
        assert!(a.take_returned().is_some(), "period {period}");
        assert!(a.takes().count() > 3, "period {period}");
        // A project opening empties the tracks on the audio thread too.
        watch(true);
        let _ = a.settings(0);
        let cleared = a.clear();
        watch(false);
        assert_eq!(CALLS.with(Cell::get), 0, "clear, period {period}");
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
            CALLS.with(|c| c.set(0));
            watch(true);
            a.process(&b, &map, &input, &mut out);
            let _ = (a.stats(), a.memory(), a.unwritten(), a.peaks(0, 0, 10_000));
            watch(false);
            calls += CALLS.with(Cell::get);
            // The disk thread's side: allocation allowed there, but the
            // tracks' calls it makes are watched.
            loop {
                watch(true);
                let r = a.take_io();
                watch(false);
                let Some(r) = r else { break };
                match r {
                    Io::Write { take, chunk } | Io::Rewrite { take, chunk } => {
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
                    Io::Finish { take } => {
                        watch(true);
                        a.finished(take, true);
                        watch(false);
                    }
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
            calls += CALLS.with(Cell::get);
            frame += period as u64;
        }
        assert_eq!(calls, 0, "allocations at period {period}");
        assert!(a.stats().full == 0, "streaming kept every recording");
        assert!(disk.len() > 20);
    }
}

/// Phase 0.2: a save fills a room made off the audio thread
/// (`Saved::room`) without allocating, and holds what `save_file` gives;
/// while a track records it waits ("not settled"), still without
/// allocating, and a second save into the same room allocates nothing
/// either.
#[test]
fn a_save_fills_its_room_without_allocating() {
    use mpfree_audiotracks::save::Saved;
    const RATE: u32 = 22_050;
    const BAR: u64 = 22_050; // 240 BPM
    let period = 256usize;
    let mut t = Transport::new(RATE, 240.0).unwrap();
    let mut a = AudioTracks::new(RATE, 6);
    let mut b = Block::new();
    let input: Vec<f32> = (0..2 * period)
        .map(|j| ((j * 31) % 97) as f32 / 97.0 - 0.5)
        .collect();
    let mut out = vec![0.0f32; 2 * period];
    let mut room = Saved::room(RATE);
    for c in [
        A::Arm(0, true),
        A::Arm(2, true),
        A::SetInput(2, Input::Stereo),
        A::SetLevel(5, 0.5),
        A::SetMonitor(6, Monitor::In),
    ] {
        a.command(c);
    }
    t.command(0, T::Record);
    t.command(2 * BAR, T::Stop);
    let mut frame = 0u64;
    let mut waited = 0;
    while frame < 3 * BAR {
        t.process(period as u32, &mut b);
        let map = *t.tempo_map();
        a.process(&b, &map, &input, &mut out);
        CALLS.with(|c| c.set(0));
        watch(true);
        let filled = a.save_into(&mut room);
        watch(false);
        assert_eq!(CALLS.with(Cell::get), 0, "at frame {frame}");
        if frame + (period as u64) <= 2 * BAR {
            assert!(filled.is_none(), "recording at frame {frame}");
            waited += 1;
        }
        frame += period as u64;
    }
    assert!(waited > 0);
    a.command(A::Arm(0, false));
    a.command(A::Arm(2, false));
    for _ in 0..2 {
        CALLS.with(|c| c.set(0));
        watch(true);
        let filled = a.save_into(&mut room);
        watch(false);
        assert_eq!(CALLS.with(Cell::get), 0, "settled");
        let (tracks, takes) = filled.expect("settled");
        let mut got = room.clone();
        got.tracks.truncate(tracks);
        got.takes.truncate(takes);
        assert_eq!(Some(got), a.save_file());
        assert_eq!(
            (tracks, takes),
            (4, 2),
            "tracks 1, 3, 6, 7; a take each on 1 and 3"
        );
    }
    // A room too small is left as it was.
    let mut small = Saved::room(RATE);
    small.takes.pop();
    let before = small.clone();
    assert!(a.save_into(&mut small).is_none());
    assert_eq!(small, before);
}

#[test]
#[ignore = "finding 1: the host's period, streaming, allocates (a new take's file name, the messages to \
            the disk thread); fixed by the fixed-size queues of mpfree-stream (P1)"]
fn the_host_period_never_allocates_while_streaming() {
    use mpfree_audiotracks::host::AudioHost;
    const RATE: u32 = 22_050;
    const BAR: u64 = 22_050; // 240 BPM
    const P: usize = 256;
    let dir = std::env::temp_dir().join(format!("mpfree-na-host-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut h = AudioHost::streaming(RATE, 4, dir.clone()).unwrap();
    let mut t = Transport::new(RATE, 240.0).unwrap();
    let mut b = Block::new();
    let input: Vec<f32> = (0..2 * P)
        .map(|j| ((j * 31) % 97) as f32 / 97.0 - 0.5)
        .collect();
    let mut out = vec![0.0f32; 2 * P];
    h.command(A::Arm(0, true));
    h.command(A::SetInput(0, Input::Left));
    t.command(100, T::Record);
    t.command(100 + 20 * BAR, T::Stop);
    let mut frame = 0u64;
    let mut calls = 0;
    while frame < 22 * BAR {
        t.process(P as u32, &mut b);
        let map = *t.tempo_map();
        CALLS.with(|c| c.set(0));
        watch(true);
        h.period(&b, &map, &input, &mut out);
        watch(false);
        calls += CALLS.with(Cell::get);
        frame += P as u64;
        std::thread::sleep(std::time::Duration::from_micros(100));
    }
    drop(h);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(calls, 0, "allocations in the host's period");
}
