//! The real-time calls (`command`, `process`, and the getters a screen
//! reads each frame) must not allocate or free. A counting global
//! allocator checks that across a busy session: recording on several
//! tracks at once, punch in and out, loop recording, locates, a tempo
//! change, comping, every edit, undo and redo, regions following the
//! tempo (read at another speed), a song time edit with its
//! undo and redo, forgetting takes, a full memory, latency changes and
//! stops, a take loaded from a save and takes read for one, at period
//! sizes from 1 to 4096; takes played from their files, by the tracks
//! alone and in the host; and Stretch's copies made and played through
//! the host (phase 5b).
//!
//! This test file is its own program, so its global allocator (the only
//! unsafe code here, a pass-through to the system allocator) does not
//! affect the library, which forbids unsafe code. Only the test's own
//! thread is watched, and counted, so tests running at once never count
//! each other's calls. (The pattern of mpfree-looper's no_alloc test.)

use mpfree_transport::TransactionId;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use mpfree_audiotracks::save::TakeFile;
use mpfree_audiotracks::{
    AudioTracks, Command as A, FollowTempo, Input, Monitor, Piece, Retime, CHUNK, MAX_TRACKS,
};
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
                    // Following the tempo: read at the song's speed once
                    // the tempo changes (bar 10), and at a region's own.
                    A::SetFollow {
                        track: 0,
                        at: 5000,
                        follow: FollowTempo::Repitch,
                        keep_pitch: false,
                    },
                    A::SetFollow {
                        track: 1,
                        at: 500,
                        follow: FollowTempo::Stretch,
                        keep_pitch: true,
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
                        song: TransactionId(1),
                        retime: bar_in,
                    },
                    A::SongUndo(TransactionId(1)),
                    A::SongRedo(TransactionId(1)),
                    A::ForgetSong(TransactionId(1)),
                    A::SetTempo {
                        track: 0,
                        at: 5000,
                        bpm: 180.0,
                    },
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
            varied: false,
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
                    Io::Close { .. } => unreachable!("no take plays from its file"),
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

/// Phase 3b step 2: with streaming on, the host's period hardly allocates
/// either (the worker's and the stream thread's channels now and then):
/// takes recorded to their files and moved to them, the song played from
/// them (the regions' starts held in the cache as it goes), the tracks
/// emptied (the files closed) and a project opened as its files, then
/// played.
#[test]
fn the_host_period_hardly_allocates_while_takes_play_from_their_files() {
    use mpfree_audiotracks::files::WriterConfig;
    use mpfree_audiotracks::host::AudioHost;
    use mpfree_audiotracks::streaming::StreamConfig;
    use mpfree_stream::player::Config;
    use std::time::{Duration, Instant};
    const RATE: u32 = 22_050;
    const BAR: u64 = 22_050; // 240 BPM
    const P: usize = 256;
    let dir = std::env::temp_dir().join(format!("mpfree-na-hstream-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut h = AudioHost::new(RATE, 40);
    let cache = Config {
        slots: 256,
        ..Config::default()
    };
    assert!(h.stream(StreamConfig {
        cache,
        ..StreamConfig::default()
    }));
    let quick = WriterConfig {
        interval_s: 0.05,
        max_interval_s: 0.2,
        commit_s: 0.01,
        ..WriterConfig::default()
    };
    assert!(h.record_to(dir.clone(), quick).is_ok());
    let mut t = Transport::new(RATE, 240.0).unwrap();
    let mut b = Block::new();
    let input: Vec<f32> = (0..2 * P)
        .map(|j| ((j * 31) % 97) as f32 / 97.0 - 0.5)
        .collect();
    let mut out = vec![0.0f32; 2 * P];
    let (mut frame, mut calls) = (0u64, 0usize);
    // Periods until `done`, the threads given time between them; only
    // the host's period is watched.
    let mut run =
        |h: &mut AudioHost, t: &mut Transport, done: &mut dyn FnMut(&AudioHost, u64) -> bool| {
            let t0 = Instant::now();
            while !done(h, frame) {
                assert!(t0.elapsed() < Duration::from_secs(30), "timed out");
                t.process(P as u32, &mut b);
                let map = *t.tempo_map();
                CALLS.with(|c| c.set(0));
                watch(true);
                h.period(&b, &map, &input, &mut out);
                watch(false);
                calls += CALLS.with(Cell::get);
                frame += P as u64;
                std::thread::sleep(Duration::from_micros(200));
            }
            frame
        };
    run(&mut h, &mut t, &mut |h, _| h.tracks.player().is_some());
    h.command(A::Arm(0, true));
    h.command(A::Arm(1, true));
    h.command(A::SetInput(1, Input::Stereo));
    let now = run(&mut h, &mut t, &mut |h, _| {
        h.armed_file(0) == Some(true) && h.armed_file(1) == Some(true)
    });
    let at = now + BAR;
    t.command(at, T::Record);
    t.command(at + 4 * BAR, T::Stop);
    run(&mut h, &mut t, &mut |_, f| f > at + 4 * BAR + 4 * P as u64);
    h.command(A::Arm(0, false));
    h.command(A::Arm(1, false));
    let on_files = |h: &AudioHost| {
        h.tracks.takes().count() == 2 && h.tracks.takes().all(|k| h.tracks.on_file(k.id))
    };
    let now = run(&mut h, &mut t, &mut |h, _| on_files(h));
    // The song from its files.
    t.command(now, T::Locate(0));
    t.command(now, T::PlayStart);
    let now = run(&mut h, &mut t, &mut |_, f| f > now + 6 * BAR);
    t.command(now, T::StopAndRewind);
    run(&mut h, &mut t, &mut |_, f| f > now + 4 * P as u64);
    // Saved, emptied, and opened again as its files, then played.
    assert!(h.save());
    run(&mut h, &mut t, &mut |h, _| h.saving().is_none());
    let saved = h.take_saved().expect("saved");
    assert!(saved.takes.iter().all(|k| k.path.is_some()));
    assert!(h.clear());
    let now = run(&mut h, &mut t, &mut |_, f| f > now + 8 * P as u64);
    assert!(h.load(saved).is_ok());
    let now = run(&mut h, &mut t, &mut |h, f| h.opening() == 0 && f > now);
    assert!(on_files(&h));
    t.command(now, T::Locate(0));
    t.command(now, T::PlayStart);
    let now = run(&mut h, &mut t, &mut |_, f| f > now + 6 * BAR);
    t.command(now, T::StopAndRewind);
    run(&mut h, &mut t, &mut |_, f| f > now + 4 * P as u64);
    drop(h);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(calls <= 4, "{calls} allocations in the host's period");
}

/// Phase 3b: playing takes from their files allocates nothing on the
/// audio thread either: a take moving from the pool to its file while the
/// song plays, a take opened as its file, frames the stick is late with,
/// the player's messages both ways, a take forgotten (its file closed),
/// and the player taken away and given back.
#[test]
fn playing_from_files_never_allocates() {
    use mpfree_audiotracks::save::wav;
    use mpfree_stream::coord::{Coordinator, Inline, Plan};
    use mpfree_stream::player::{Config, Player};
    const RATE: u32 = 22_050;
    const BAR: u64 = 22_050; // 240 BPM
    const P: usize = 128;
    let dir = std::env::temp_dir().join(format!("mpfree-na-stream-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = Config {
        slots: 64,
        ..Config::default()
    };
    let (p, rx, tx) = Player::new(cfg);
    let plan = Plan {
        rate: RATE,
        ..Plan::default()
    };
    let mut c = Coordinator::new(cfg, plan, rx, tx, Box::<Inline>::default());
    let mut a = AudioTracks::new(RATE, 10);
    assert!(a.stream(Some(Box::new(p))).is_none());
    let mut t = Transport::new(RATE, 240.0).unwrap();
    let mut b = Block::new();
    let input: Vec<f32> = (0..2 * P)
        .map(|j| ((j * 31) % 97) as f32 / 97.0 - 0.5)
        .collect();
    let mut out = vec![0.0f32; 2 * P];
    let mut calls = 0;
    let mut frame = 0u64;
    let mut run = |a: &mut AudioTracks, c: &mut Coordinator, t: &mut Transport, to: u64| {
        while frame < to {
            t.process(P as u32, &mut b);
            let map = *t.tempo_map();
            CALLS.with(|k| k.set(0));
            watch(true);
            a.process(&b, &map, &input, &mut out);
            let _ = (a.stats(), a.missed(0), a.on_file(0), a.take_io());
            watch(false);
            calls += CALLS.with(Cell::get);
            c.step();
            frame += P as u64;
        }
        frame
    };
    // Two takes, two bars each, on tracks 0 and 1.
    a.command(A::Arm(0, true));
    a.command(A::Arm(1, true));
    t.command(100, T::Record);
    t.command(100 + 2 * BAR, T::Stop);
    let f = run(&mut a, &mut c, &mut t, 3 * BAR);
    a.command(A::Arm(0, false));
    a.command(A::Arm(1, false));
    let takes: Vec<_> = a.takes().collect();
    assert_eq!(takes.len(), 2);
    let mut paths = Vec::new();
    for k in &takes {
        let mut audio = vec![0.0f32; k.frames * k.channels];
        assert_eq!(a.read_take(k.id, 0, &mut audio), Some(k.frames));
        let path = dir.join(format!("take_{}.wav", k.id));
        std::fs::write(&path, wav(&audio, k.channels, RATE)).unwrap();
        paths.push(path);
    }
    // The first moves to its file while the song plays.
    t.command(f, T::PlayStart);
    let f = run(&mut a, &mut c, &mut t, f + BAR / 4);
    let tag = c.open(takes[0].id, &paths[0]).unwrap();
    watch(true);
    a.command(A::OnFile {
        take: takes[0].id,
        rec: takes[0].rec,
        gen: tag.gen,
        growing: false,
    });
    watch(false);
    let f = run(&mut a, &mut c, &mut t, f + BAR / 2);
    assert!(a.on_file(takes[0].id));
    // The second's file holds only its first chunk and a little more so
    // far, as while it records: that chunk goes, and it plays partly from
    // its file.
    let per = CHUNK / takes[1].channels;
    let n = per + 100;
    let part = dir.join("part.wav");
    let mut bytes = std::fs::read(&paths[1]).unwrap();
    bytes.truncate(44 + n * takes[1].channels * 4);
    std::fs::write(&part, bytes).unwrap();
    let tag = c.open_growing(takes[1].id, &part, n as u64).unwrap();
    watch(true);
    a.command(A::OnFile {
        take: takes[1].id,
        rec: takes[1].rec,
        gen: tag.gen,
        growing: true,
    });
    watch(false);
    let f = run(&mut a, &mut c, &mut t, f + BAR / 2);
    assert_eq!(a.file_frames(takes[1].id), Some(per));
    // The player away and back: the file take misses meanwhile.
    watch(true);
    let p = a.stream(None);
    watch(false);
    let f = run(&mut a, &mut c, &mut t, f + BAR / 4);
    watch(true);
    let none = a.stream(p);
    watch(false);
    assert!(none.is_none());
    assert!(a.stats().missed > 0);
    let f = run(&mut a, &mut c, &mut t, f + 2 * BAR);
    // Stopped, the tracks cleared (the file closed), and the second take
    // opened as its file and played.
    t.command(f, T::StopAndRewind);
    let f = run(&mut a, &mut c, &mut t, f + 4 * P as u64);
    let mut s = a.save_file().unwrap();
    watch(true);
    let cleared = a.clear();
    watch(false);
    assert!(cleared);
    c.close(takes[0].id);
    let tag = c.open(takes[1].id, &paths[1]).unwrap();
    let k = s.takes.remove(1);
    let tr = s.tracks.iter().find(|tr| tr.index == 1).unwrap().clone();
    watch(true);
    let added = a.add_file_take(&k, tag.gen);
    let loaded = a.load_track(&tr);
    watch(false);
    assert!(added && loaded);
    t.command(f, T::PlayStart);
    run(&mut a, &mut c, &mut t, f + 3 * BAR);
    assert_eq!(calls, 0, "allocations playing from files");
    let _ = std::fs::remove_dir_all(&dir);
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

#[test]
fn clips_never_allocate() {
    // Launches closer than a fade (a fifth voice), an edit, a mute, song
    // mode, a ring and loop jumps, on every track, at periods from 1 to
    // 4096: the answers are built before each period, as the arranger's
    // player builds them from its view.
    use mpfree_audiotracks::{Answer, Change, ClipPlay, ClipRegion, Plays, RecordingId};
    const RATE: u32 = 22_050;
    const BT: i64 = 3_840;
    let regions: Vec<[ClipRegion; 2]> = (0..4u16)
        .map(|k| {
            let r = ClipRegion {
                start: 0,
                end: 960,
                rec: RecordingId(100 + u128::from(k)),
                src: 0,
                phase: 0.0,
                gain: 1.0,
                fade_in: 100,
                fade_out: 100,
                into: 0,
                left: 0,
                bpm: 0.0,
                follow: Default::default(),
                keep_pitch: false,
            };
            [
                r,
                ClipRegion {
                    start: 1280,
                    end: BT,
                    src: 3000,
                    ..r
                },
            ]
        })
        .collect();
    let clip = |k: usize, origin: f64, stamp: u64| ClipPlay {
        regions: &regions[k],
        origin,
        len: BT,
        last_frame: 30_000,
        gain: 1.0,
        clip: k as u32 + 1,
        stamp,
    };
    for period in [1usize, 64, 333, 4096] {
        let mut t = Transport::new(RATE, 240.0).unwrap();
        let mut a = AudioTracks::new(RATE, 30);
        let mut b = Block::new();
        for k in 0..4u16 {
            let audio = (0..2 * 44_100)
                .map(|j| ((j * 31 + k as usize) % 97) as f32 / 97.0 - 0.5)
                .collect();
            let f = TakeFile {
                id: k,
                rec: RecordingId(100 + u128::from(k)),
                track: 0,
                group: k,
                pass: 0,
                bpm: 240.0,
                anchor: 0.0,
                frames: 44_100,
                channels: 2,
                file: String::new(),
                path: None,
                audio,
                varied: false,
            };
            assert!(a.load_take(f).is_ok());
        }
        let input = vec![0.0f32; 2 * period];
        let mut out = vec![0.0f32; 2 * period];
        let mut frame = 0u64;
        while a.loading() > 0 {
            assert!(t.process(period as u32, &mut b));
            let map = *t.tempo_map();
            a.process(&b, &map, &input, &mut out);
            frame += period as u64;
        }
        let map = *t.tempo_map();
        assert!(a.place(3, &[], &map));
        assert!(t.command(frame, T::SetLoop(Some((0, 2 * BT)))));
        assert!(t.command(frame, T::PlayStart));
        let start = frame;
        let at = |tick: i64| start + tick as u64 * 735 / 128;
        // At frame `at`, track `tr` plays clip `k` (or its song) from
        // `origin`, at `stamp`, `muted`, ringing what played.
        #[derive(Clone, Copy)]
        struct Ev {
            at: u64,
            tr: usize,
            k: Option<usize>,
            origin: f64,
            stamp: u64,
            muted: bool,
            ring: bool,
        }
        let e = |at, tr, k, origin, stamp| Ev {
            at,
            tr,
            k,
            origin,
            stamp,
            muted: false,
            ring: false,
        };
        let mut evs: Vec<Ev> = Vec::new();
        for tr in 0..MAX_TRACKS {
            evs.push(e(at(0), tr, Some(tr % 3), 0.0, 1));
            evs.push(e(at(0) + 4000, tr, Some(tr % 3), 0.0, 2));
            for k in 0..4 {
                evs.push(e(at(BT) + 9 * k as u64, tr, Some(k % 3), BT as f64, 1));
            }
            evs.push(Ev {
                muted: true,
                ..e(at(BT) + 3000, tr, Some(0), BT as f64, 1)
            });
            evs.push(Ev {
                ring: true,
                ..e(at(BT) + 6000, tr, None, 0.0, 0)
            });
        }
        evs.sort_by_key(|e| e.at);
        let mut changes: Vec<Vec<Change>> =
            (0..MAX_TRACKS).map(|_| Vec::with_capacity(16)).collect();
        let end = at(6 * BT);
        CALLS.with(|c| c.set(0));
        let mut calls = 0;
        while frame < end {
            let n = period as u64;
            for (tr, cs) in changes.iter_mut().enumerate() {
                cs.clear();
                for e in evs
                    .iter()
                    .filter(|e| e.tr == tr && (frame..frame + n).contains(&e.at))
                {
                    if cs.len() == 4 {
                        cs.remove(0);
                    }
                    cs.push(Change {
                        frame: (e.at - frame) as u32,
                        // The whole tick it lands on, as the player gives.
                        tick: ((e.at - start) as f64 * 128.0 / 735.0).round(),
                        plays: e
                            .k
                            .map_or(Plays::Song, |k| Plays::Clip(clip(k, e.origin, e.stamp))),
                        muted: e.muted,
                        ring: e.ring,
                    });
                }
            }
            let mut answers = [Answer::default(); MAX_TRACKS];
            for (tr, ans) in answers.iter_mut().enumerate() {
                let before = evs.iter().rev().find(|e| e.tr == tr && e.at < frame);
                *ans = Answer {
                    start: before.map_or(Plays::Song, |e| {
                        e.k.map_or(Plays::Song, |k| Plays::Clip(clip(k, e.origin, e.stamp)))
                    }),
                    muted: before.is_some_and(|e| e.muted),
                    changes: &changes[tr],
                    superseded: 0,
                };
            }
            watch(true);
            assert!(t.process(n as u32, &mut b));
            let map = *t.tempo_map();
            a.process_with(&b, &map, &input, &mut out, &answers);
            let _ = a.stats();
            watch(false);
            calls += CALLS.with(Cell::get);
            CALLS.with(|c| c.set(0));
            frame += n;
        }
        assert!(a.stats().declicked > 0);
        assert_eq!(calls, 0, "period {period}");
    }
}

#[test]
fn loops_through_the_track_player_never_allocate() {
    // Phase 4, step 4b: on every track, a loop, a second crossing over, a
    // third inside the fade (a voice taken), a mute, a take clip, the
    // same loop from another origin, and loop jumps, at periods from 1 to
    // 4096: the plan, the loops' runs and the mix never allocate.
    use mpfree_audiotracks::{
        Answer, Change, ClipPlay, ClipRegion, LoopPlay, LoopPlayer, Plays, RecordingId,
    };
    const RATE: u32 = 22_050;
    const BT: i64 = 3_840;
    let region = [ClipRegion {
        start: 0,
        end: BT,
        rec: RecordingId(100),
        src: 0,
        phase: 0.0,
        gain: 1.0,
        fade_in: 0,
        fade_out: 0,
        into: 0,
        left: 0,
        bpm: 0.0,
        follow: Default::default(),
        keep_pitch: false,
    }];
    let what = |k: u8, origin: f64| match k {
        0 => Plays::Clip(ClipPlay {
            regions: &region,
            origin,
            len: BT,
            last_frame: 20_000,
            gain: 1.0,
            clip: 9,
            stamp: 1,
        }),
        id => Plays::Loop(LoopPlay {
            id: u128::from(id),
            origin,
            len: BT,
            clip: u32::from(id),
            stamp: 1,
        }),
    };
    let bufs: Vec<Vec<f32>> = (0..4).map(|k| vec![k as f32 * 0.1; 2 * 4096]).collect();
    for period in [1usize, 64, 333, 4096] {
        let mut t = Transport::new(RATE, 240.0).unwrap();
        let mut a = AudioTracks::new(RATE, 30);
        let mut loops = LoopPlayer::new();
        let mut b = Block::new();
        let audio = vec![0.25f32; 2 * 30_000];
        let f = TakeFile {
            id: 0,
            rec: RecordingId(100),
            track: 0,
            group: 0,
            pass: 0,
            bpm: 240.0,
            anchor: 0.0,
            frames: 30_000,
            channels: 2,
            file: String::new(),
            path: None,
            audio,
            varied: false,
        };
        assert!(a.load_take(f).is_ok());
        let input = vec![0.0f32; 2 * period];
        let mut out = vec![0.0f32; 2 * period];
        let mut frame = 0u64;
        while a.loading() > 0 {
            assert!(t.process(period as u32, &mut b));
            let map = *t.tempo_map();
            a.process(&b, &map, &input, &mut out);
            frame += period as u64;
        }
        assert!(t.command(frame, T::SetLoop(Some((0, 4 * BT)))));
        assert!(t.command(frame, T::PlayStart));
        let start = frame;
        let at = |tick: i64| start + tick as u64 * 735 / 128;
        // (frame, track, what, origin, muted)
        let mut evs: Vec<(u64, usize, u8, f64, bool)> = Vec::new();
        for tr in 0..MAX_TRACKS {
            evs.push((at(0), tr, 1, 0.0, false));
            evs.push((at(BT), tr, 2, BT as f64, false));
            evs.push((at(BT) + 7, tr, 3, BT as f64, false));
            evs.push((at(BT) + 3000, tr, 3, BT as f64, true));
            evs.push((at(2 * BT), tr, 0, (2 * BT) as f64, false));
            evs.push((at(3 * BT), tr, 1, (3 * BT) as f64, false));
            evs.push((at(3 * BT) + 5000, tr, 1, 0.0, false));
        }
        evs.sort_by_key(|e| e.0);
        let mut changes: Vec<Vec<Change>> =
            (0..MAX_TRACKS).map(|_| Vec::with_capacity(16)).collect();
        let end = at(9 * BT);
        let mut calls = 0;
        while frame < end {
            let n = period as u64;
            for (tr, cs) in changes.iter_mut().enumerate() {
                cs.clear();
                for e in evs
                    .iter()
                    .filter(|e| e.1 == tr && (frame..frame + n).contains(&e.0))
                {
                    if cs.len() == 4 {
                        cs.remove(0);
                    }
                    cs.push(Change {
                        frame: (e.0 - frame) as u32,
                        tick: ((e.0 - start) as f64 * 128.0 / 735.0).round(),
                        plays: what(e.2, e.3),
                        muted: e.4,
                        ring: false,
                    });
                }
            }
            let mut answers = [Answer::default(); MAX_TRACKS];
            for (tr, ans) in answers.iter_mut().enumerate() {
                let before = evs.iter().rev().find(|e| e.1 == tr && e.0 < frame);
                *ans = Answer {
                    start: before.map_or(Plays::Nothing, |e| what(e.2, e.3)),
                    muted: before.is_some_and(|e| e.4),
                    changes: &changes[tr],
                    superseded: 0,
                };
            }
            let src = |id: u128| bufs.get(id as usize).map(|v| &v[..2 * period]);
            CALLS.with(|c| c.set(0));
            watch(true);
            assert!(t.process(n as u32, &mut b));
            let map = *t.tempo_map();
            loops.plan(&b, &answers);
            let _ = (0..MAX_TRACKS)
                .map(|tr| loops.runs(tr).len())
                .sum::<usize>();
            a.process_with_loops(&b, &map, &input, &mut out, &answers, &loops, &src);
            watch(false);
            calls += CALLS.with(Cell::get);
            frame += n;
        }
        assert!(loops.stolen() > 0, "period {period}");
        assert_eq!(calls, 0, "period {period}");
    }
}

/// Phase 5b.2, Stretch's copies: the host's period hardly allocates (the
/// worker's channel now and then) while it waits for the tempo to hold,
/// works out what is wanted, reads a song region's and a clip's sources,
/// takes their copies in, plays from one, plays Repitch to the region's
/// end after a tempo change while the new copies are made, and lets them
/// go at Clear copies, which doesn't allocate either. The scheduler,
/// stepped between periods, isn't watched: it has its own thread.
#[test]
fn the_host_period_hardly_allocates_while_copies_are_made_and_played() {
    copies_made_and_played(false);
}

/// Phase 5b.4, Keep pitch: the same with the song region on Keep pitch,
/// so after the tempo change it drifts on its copy for 90 and rejoins the
/// bars on the copy for 100: no more allocation.
#[test]
fn a_region_drifting_on_keep_pitch_and_rejoining_never_allocates() {
    copies_made_and_played(true);
}

fn copies_made_and_played(keep_pitch: bool) {
    use mpfree_audiotracks::copying::ClipWant;
    use mpfree_audiotracks::host::AudioHost;
    use mpfree_audiotracks::save::TrackFile;
    use mpfree_audiotracks::{ClipRegion, RecordingId, Region, MAX_CLIP_REGIONS};
    use mpfree_stretch::sched::{Config, Priority, Sched};
    use std::time::{Duration, Instant};
    const RATE: u32 = 22_050;
    const P: usize = 256;
    let bar = 4 * i64::from(mpfree_transport::time::PPQ);
    let rec = RecordingId(900);
    let frames = (8.0 * 60.0 * f64::from(RATE) / 120.0) as usize;
    let take = TakeFile {
        id: 0,
        rec,
        track: 0,
        group: 0,
        pass: 0,
        bpm: 120.0,
        varied: false,
        anchor: 0.0,
        frames,
        channels: 1,
        file: String::new(),
        path: None,
        audio: (0..frames)
            .map(|j| ((j * 31) % 97) as f32 / 97.0 - 0.5)
            .collect(),
    };
    let s = Sched::manual(Config::default());
    let mut h = AudioHost::new(RATE, 20);
    h.copies_on(s.clone());
    assert!(h.tracks.load_take(take).is_ok());
    let region = Region {
        follow: FollowTempo::Stretch,
        keep_pitch,
        ..Region::of(0, 2 * bar, 0, (0, 0.0), 120.0)
    };
    let track = TrackFile {
        index: 0,
        id: 0,
        input: Input::Left,
        monitor: Monitor::Auto,
        from: String::new(),
        regions: vec![region],
    };
    assert!(h.tracks.load_track(&track));
    let clip = ClipRegion {
        start: 0,
        end: 2 * bar,
        rec,
        src: 0,
        phase: 0.0,
        gain: 1.0,
        fade_in: 0,
        fade_out: 0,
        into: 0,
        left: 0,
        follow: FollowTempo::Stretch,
        keep_pitch: false,
        bpm: 0.0,
    };
    h.clip_copies(vec![ClipWant {
        regions: [clip; MAX_CLIP_REGIONS],
        n: 1,
        len: 2 * bar,
        last_frame: frames as i64,
        priority: Priority::Prepared,
        bpm: 0.0,
    }]);
    let mut t = Transport::new(RATE, 90.0).unwrap();
    let mut b = Block::new();
    let input = vec![0.0f32; 2 * P];
    let mut out = vec![0.0f32; 2 * P];
    let mut frame = 0u64;
    let mut calls = 0;
    let t0 = Instant::now();
    // 0: the first two copies made; 1: playing; 2: the tempo turned, the
    // new two made; 3: played on.
    let mut stage = 0;
    let mut at = 0u64;
    let mut turned = 0u64;
    while stage < 4 {
        assert!(
            t0.elapsed() < Duration::from_secs(60),
            "stage {stage}: {:?}",
            h.copy_counts()
        );
        match stage {
            0 if h.copy_counts().made == 2 => {
                assert!(t.command(frame, T::PlayStart));
                (stage, at) = (1, frame);
            }
            1 if frame > at + u64::from(RATE) => {
                assert!(t.command(frame, T::TempoNow(100.0)));
                (stage, turned) = (2, frame);
            }
            2 if h.copy_counts().made == 4 => {
                (stage, at) = (3, frame);
            }
            3 if frame > at + u64::from(RATE) => stage = 4,
            _ => {}
        }
        t.process(P as u32, &mut b);
        let map = *t.tempo_map();
        CALLS.with(|c| c.set(0));
        watch(true);
        h.period(&b, &map, &input, &mut out);
        watch(false);
        calls += CALLS.with(Cell::get);
        frame += P as u64;
        s.step();
        std::thread::sleep(Duration::from_micros(100));
    }
    let st = h.tracks.stats();
    assert!(st.copied > 0, "{st:?}");
    if keep_pitch {
        assert!(st.drifted > 0 && st.for_now == 0, "{st:?}");
        // It rejoined: it stopped drifting before the end.
        assert!(st.drifted + 4 * (P as u64) < frame - turned, "{st:?}");
        // Stopped, nothing plays a copy.
        assert!(t.command(frame, T::Stop));
        t.process(P as u32, &mut b);
        let map = *t.tempo_map();
        CALLS.with(|c| c.set(0));
        watch(true);
        h.period(&b, &map, &input, &mut out);
        watch(false);
        calls += CALLS.with(Cell::get);
    }
    CALLS.with(|c| c.set(0));
    watch(true);
    let gone = h.clear_copies();
    watch(false);
    let cleared = CALLS.with(Cell::get);
    assert_eq!(gone, 4);
    // Nothing of its own; sending the four to the worker may take a block
    // of the channel (one in 31 sends), as the period may.
    assert!(cleared <= 1, "{cleared} allocations clearing the copies");
    // None in practice; the channel to the worker may take a block now
    // and then.
    assert!(calls <= 4, "{calls} allocations in the host's period");
}
