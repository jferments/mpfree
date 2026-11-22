//! The real-time calls (`command`, `process`, and the getters a screen
//! reads each frame) must not allocate or free. A counting global
//! allocator checks that across a busy session: recording on several
//! tracks at once, punch in and out, loop recording, locates, a tempo
//! change, comping, every edit, undo and redo, a song time edit with its
//! undo and redo, forgetting takes, a full memory, latency changes and
//! stops, a take loaded from a save and takes read for one, at period
//! sizes from 1 to 4096.
//!
//! This test file is its own program, so its global allocator (the only
//! unsafe code here, a pass-through to the system allocator) does not
//! affect the library, which forbids unsafe code. Only the test's own
//! thread is watched, and counted, so tests running at once never count
//! each other's calls. (The pattern of mpfree-looper's no_alloc test.)

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use mpfree_audiotracks::save::TakeFile;
use mpfree_audiotracks::{AudioTracks, Command as A, Input, Monitor, Piece, Retime, MAX_TRACKS};
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
        // A bar put in at the start (a song time edit, Arrange 3.4).
        let bar_in = Retime::new(&[Piece {
            from: 0,
            end: i64::MAX,
            to: BT,
        }])
        .unwrap();
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
                    A::Retime {
                        song: 1,
                        retime: bar_in,
                    },
                    A::SongUndo(1),
                    A::SongRedo(1),
                    A::ForgetSong(1),
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
            path: None,
            id: 200,
            // Its own lasting id, kept as it loads.
            rec: mpfree_audiotracks::RecordingId(0x5eed),
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

/// Phase 3a: with recording to files on, arming, recording on two tracks
/// (one stereo, its input changed while armed), a punch out, disarming,
/// the host's side taking the requests, copying every take out as it
/// records and saying each is written, and clearing the tracks: nothing
/// on the audio thread's side allocates.
#[test]
fn recording_to_files_never_allocates_on_the_audio_thread() {
    use mpfree_audiotracks::Io;
    const RATE: u32 = 22_050;
    const BAR: u64 = 22_050; // 240 BPM
    for period in [1usize, 128, 4096] {
        let mut t = Transport::new(RATE, 240.0).unwrap();
        let mut a = AudioTracks::new(RATE, 40);
        assert!(a.files(true));
        let mut b = Block::new();
        let mut input = vec![0.0f32; 2 * period];
        let mut out = vec![0.0f32; 2 * period];
        for (j, s) in input.iter_mut().enumerate() {
            *s = ((j * 31) % 97) as f32 / 97.0 - 0.5;
        }
        // The host's side: what each take has copied out so far, in room
        // made before.
        let mut copied = vec![0usize; mpfree_audiotracks::MAX_TAKES];
        let mut writing: Vec<u16> = Vec::with_capacity(mpfree_audiotracks::MAX_TAKES);
        let mut buf = vec![0.0f32; mpfree_audiotracks::CHUNK];
        let (mut arms, mut disarms, mut starts) = (0, 0, 0);
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
            (5, vec![A::Arm(1, false)], vec![]),
            (8, vec![A::SetInput(0, Input::Stereo)], vec![]),
            (10, vec![A::Arm(0, false)], vec![T::Stop]),
            (11, vec![A::Arm(2, true), A::Arm(2, false)], vec![]),
        ];
        let end = 12 * BAR;
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
            let _ = (
                a.stats(),
                a.memory(),
                a.peaks(0, 0, 10_000),
                a.next_recording(0),
            );
            while let Some(r) = a.take_io() {
                match r {
                    Io::Arm { .. } => arms += 1,
                    Io::Disarm { .. } => disarms += 1,
                    Io::Start { take } => {
                        starts += 1;
                        writing.push(take);
                    }
                }
            }
            let mut k = 0;
            while k < writing.len() {
                let id = writing[k];
                let info = a.take(id).unwrap();
                let at = copied[id as usize];
                let room = mpfree_audiotracks::CHUNK / info.channels * info.channels;
                let n = a.read_take(id, at, &mut buf[..room]).unwrap();
                copied[id as usize] += n;
                if !info.recording && copied[id as usize] == info.frames {
                    a.written(id);
                    writing.swap_remove(k);
                } else {
                    k += 1;
                }
            }
            watch(false);
            calls += CALLS.with(Cell::get);
            frame += period as u64;
        }
        assert_eq!(calls, 0, "allocations at period {period}");
        assert!(writing.is_empty() && a.takes().all(|k| !a.writing(k.id)));
        // Two takes. Files for the two armed tracks, again for track 1's
        // stereo, for each take's next, for track 0's stereo while it
        // records, and for track 2.
        assert_eq!(starts, 2, "period {period}");
        assert_eq!(arms, 2 + 1 + 2 + 1 + 1, "period {period}");
        assert_eq!(arms, disarms + starts, "every file made is used or goes");
        for k in a.takes() {
            assert_eq!(copied[k.id as usize], k.frames, "every frame copied out");
        }
        watch(true);
        let cleared = a.clear();
        watch(false);
        assert!(cleared);
        assert_eq!(CALLS.with(Cell::get), 0);
    }
}

/// The host's period while a track records to its file: the takes are
/// copied out into buffers made once and handed to the worker. Only the
/// channel to the worker may allocate, a block now and then (as in the
/// looper's host): a handful in 20 bars of recording.
#[test]
fn the_host_period_hardly_allocates_while_recording_to_files() {
    use mpfree_audiotracks::files::WriterConfig;
    use mpfree_audiotracks::host::AudioHost;
    const RATE: u32 = 22_050;
    const BAR: u64 = 22_050; // 240 BPM
    const P: usize = 256;
    let dir = std::env::temp_dir().join(format!("mpfree-na-host-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut h = AudioHost::new(RATE, 40);
    assert!(h.record_to(dir.clone(), WriterConfig::default()).is_ok());
    let mut t = Transport::new(RATE, 240.0).unwrap();
    let mut b = Block::new();
    let input: Vec<f32> = (0..2 * P)
        .map(|j| ((j * 31) % 97) as f32 / 97.0 - 0.5)
        .collect();
    let mut out = vec![0.0f32; 2 * P];
    h.command(A::Arm(0, true));
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
    }
    drop(h);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(calls <= 4, "{calls} allocations in the host's period");
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
        A::SetInput(5, Input::Right),
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
