//! The looper's real-time calls (`command`, `process`, `status`, `stats`,
//! `layer`) must not allocate or free. A counting global allocator checks
//! that across a busy session: the first pass, overdubs, undo and redo,
//! edits, merges, bounces, a resample and layers copied and moved between
//! loops, clear and its undo, one-pass overdubs, a loop solo, mute, level
//! and latency
//! changes, a full memory, locates, a transport loop, tempo changes, stop
//! and count-in, at period sizes from 1 to 4096 frames; and loops that
//! start when you play (by ear and on a pad, the song stopped, playing and
//! counting in, a start refused); and free first passes that set the
//! tempo (on the bar and from a note, closed too soon, one thrown away by
//! PLAY); and free loops (the free clock set from the song and in your
//! own time, the song stopping under them, STOP and PLAY, their own Stop
//! and Play, a free loop from a note, Clear and Undo of the last one, a
//! free loop's snap and a capture on the free clock's bars).
//!
//! This test file is its own program, so its global allocator (the only
//! unsafe code here, a pass-through to the system allocator) does not
//! affect the library, which forbids unsafe code. Only the test's own
//! thread is watched. (The same pattern as mpfree-transport's no_alloc
//! test.)

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use mpfree_looper::{
    After, Clock, Command as L, Edit as E, LayerCommand as Lc, Looper, NoteStart, Overdubs, Phase,
    SmartEnd, Snap, SnapPlan, Starts, MAX_LOOPS,
};
use mpfree_transport::{Block, Command as T, Transport};

struct Counting;
thread_local! {
    static WATCH: Cell<bool> = const { Cell::new(false) };
    // Each test thread counts its own calls.
    static CALLS: Cell<usize> = const { Cell::new(0) };
}

fn count() {
    if WATCH.with(Cell::get) {
        CALLS.with(|c| c.set(c.get() + 1));
    }
}

fn calls() -> usize {
    CALLS.with(Cell::get)
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
    const RATE: u32 = 44_100;
    let mut t = Transport::new(RATE, 120.0).unwrap();
    // 13 seconds: a 2-bar loop at 120 BPM is 4 s (and its first pass
    // keeps about half a second around it), so it fills up after two
    // overdubs and the rest are refused.
    let mut l = Looper::new(RATE, 13);
    let mut b = Block::new();
    let mut input = vec![0.0f32; 2 * 4096];
    let mut out = vec![0.0f32; 2 * 4096];

    // The counter itself works: one Vec is one allocation and one free.
    watch(true);
    drop(std::hint::black_box(vec![0u8; 16]));
    watch(false);
    assert_eq!(CALLS.with(|c| c.replace(0)), 2);

    let sizes = [1usize, 7, 128, 333, 4096, 64, 256];
    let script = [
        L::SetFadeOld(0.8),
        L::Overdub,
        L::Undo,
        L::Redo,
        L::Merge,
        L::Overdub,
        L::Overdub,
        L::Mute(true),
        L::Mute(false),
        L::SetLevel(1.5),
        L::Overdub,
        L::Undo,
        L::Undo,
        L::SetLatency(300),
        L::Record,
        L::Record,
        L::Clear,
        // A first pass from the other loops (a Record that resamples).
        L::Resample,
        L::SetBars(None),
        L::Record,
        L::SetBars(Some(1)),
        L::SetAfter(After::Overdub),
        L::Edit(E::Double),
        L::Edit(E::Nudge(1)),
        L::Edit(E::Reverse),
        L::Edit(E::Half),
        L::Edit(E::Trim {
            from: 960,
            to: 2_880,
        }),
        L::Edit(E::SeamFade(512)),
        L::Undo,
        L::Edit(E::Nudge(-3)),
        L::Merge,
        L::Undo,
        L::Redo,
        // One-pass overdubs, a loop solo, a clear Undo brings back, and
        // one it doesn't.
        L::SetOverdubs(Overdubs::Once),
        L::Overdub,
        L::Solo(true),
        L::Clear,
        L::Undo,
        L::Solo(false),
        // A Replace, merged with the layers under it, and undone.
        L::SetOverdubs(Overdubs::Replace),
        L::Overdub,
        L::Overdub,
        L::Merge,
        L::Undo,
        L::SetOverdubs(Overdubs::Add),
        // Half speed, an overdub that waits for its print and is called
        // off, and back.
        L::Edit(E::Speed(-1)),
        L::Overdub,
        L::Overdub,
        L::Edit(E::Speed(0)),
        L::Forget,
        // A capture of the last bars heard, cleared; one called off; and
        // the input changing.
        L::Capture(2),
        L::Forget,
        L::Capture(1),
        L::Capture(1),
        L::NewInput,
    ];
    let mut seen_playing = false;
    let mut layers = 0usize;
    let mut sum = 0.0f64;
    let mut phase = 0.0f32;
    let mut loops_used = 0u32;
    let mut merged = false;
    let mut spare = Some(Vec::with_capacity(2 * 12 * RATE as usize));
    let mut bounced = 0;
    let mut want = false;
    // A calm second session for bounces and a resample, made up front.
    let mut t2 = Transport::new(RATE, 120.0).unwrap();
    let mut l2 = Looper::new(RATE, 12);
    let mut bounced2 = 0;
    let mut copied2 = false;
    let mut played2 = false;
    let mut moved2 = false;
    watch(true);
    t.command(0, T::Play);
    l.command(L::Record);
    for i in 0..6000u64 {
        let f = t.frame();
        // The first pass (2 bars, about 250 periods) and the first
        // overdubs run undisturbed; then the transport moves around.
        match if i < 600 { 0 } else { i % 500 } {
            100 => {
                let _ = t.command(f + 3, T::Locate((i as i64 * 37) % 9000));
            }
            200 => {
                let _ = t.command(
                    f + 1,
                    T::SetLoop(if i % 1000 == 200 {
                        Some((0, 7680))
                    } else {
                        None
                    }),
                );
            }
            300 => {
                let _ = t.command(f + 2, T::SetTempo(100.0 + (i % 7) as f64 * 5.0));
            }
            400 => {
                let _ = t.command(f + 2, T::Stop);
            }
            420 => {
                let _ = t.command(f + 2, T::SetCountIn(1));
                let _ = t.command(f + 2, T::Record);
            }
            _ => {}
        }
        if i >= 290 && i % 37 == 0 {
            let _ = l.command(script[(i / 37 - 8) as usize % script.len()]);
        }
        // The last third moves between three loops (and one past the end).
        if i >= 4000 && i % 211 == 0 {
            let _ = l.select_loop([0, 3, 7, MAX_LOOPS][(i / 211) as usize % 4]);
            let _ = l.command(L::Record);
        }
        // And copies or moves layers between them.
        if i >= 4000 && i.is_multiple_of(263) {
            let j = (i / 263) as usize;
            let _ = l.copy_layer(
                [0, 3, 7][j % 3],
                (j % 3) as u8,
                [3, 7, 1, 0][j % 4],
                j.is_multiple_of(2),
            );
        }
        // Launches, on a bar line ahead and at once (Loop tracks).
        if i >= 290 && i.is_multiple_of(97) {
            let j = (i / 97) as usize;
            let at = j.is_multiple_of(2).then_some((j as i64 % 9) * 3_840);
            let _ = l.launch(j % (MAX_LOOPS + 1), !j.is_multiple_of(3), at);
        }
        // Each layer's controls; deletes only in the second half, so the
        // first half fills the memory and overdubs are refused.
        if i >= 290 && i % 53 == 0 {
            let k = (i / 53) as usize % if i < 3000 { 4 } else { 5 };
            let id = l
                .layer_info(k % l.status().layers.max(1))
                .map_or(9, |x| x.id);
            let c = [
                Lc::Mute(i % 2 == 0),
                Lc::Solo(i % 3 == 0),
                Lc::Pan(0.3),
                Lc::Level(1.4),
                Lc::Delete,
            ][k];
            let _ = l.layer_command(id, c);
        }
        let n = sizes[(i % sizes.len() as u64) as usize];
        for k in 0..n {
            phase += 0.031;
            if phase > std::f32::consts::TAU {
                phase -= std::f32::consts::TAU;
            }
            input[2 * k] = 0.3 * phase.sin();
            input[2 * k + 1] = 0.3 * phase.cos();
        }
        out[..2 * n].fill(0.0);
        assert!(t.process(n as u32, &mut b));
        l.process(&b, &input[..2 * n], &mut out[..2 * n]);
        sum += out[..2 * n].iter().map(|v| v.abs() as f64).sum::<f64>();
        let s = l.status();
        seen_playing |= s.phase == Phase::Playing;
        layers = layers.max(s.layers);
        merged |= s.merging.is_some();
        if let Some(w) = l.layer(0) {
            sum += w[0] as f64 * 0.0;
        }
        let _ = l.stats();
        // The undo history, for the screen.
        sum += l.history().done().len() as f64 * 0.0;
        // A bounce now and then, its buffer going round. (Not too often:
        // a loop's commands wait while it bounces. It is asked for until
        // a loop is ready.)
        want |= i >= 300 && i % 401 == 0;
        bounce(&mut l, i, &mut want, &mut spare, &mut bounced);
        let m = l.memory();
        assert!(m.used <= m.capacity);
        for k in 0..MAX_LOOPS {
            let s = l.loop_status(k).unwrap();
            loops_used |= u32::from(s.phase != Phase::Empty) << k;
            if let Some(w) = l.loop_layer(k, 0) {
                sum += w[w.len() - 1] as f64 * 0.0;
            }
        }
    }

    // The calm session: one bar on loop 0, then loop 1 resamples it, and
    // both are bounced again and again at every period size.
    t2.command(0, T::Play);
    l2.command(L::SetBars(Some(1)));
    l2.command(L::Record);
    for i in 0..3000u64 {
        if i == 400 {
            let _ = l2.select_loop(1);
            let _ = l2.command(L::SetBars(Some(1)));
            let _ = l2.command(L::Resample);
        }
        // Loop 0's bounds moved, resized and turned in the audio its
        // first pass kept, and one taken back; a layer of loop 0 onto
        // loop 1, loop 1's first pass moved into loop 2, and Undo on loop
        // 1, which takes the move back.
        match i {
            200 => {
                let _ = l2.command(L::Edit(E::Turn(1_000)));
            }
            220 => {
                let _ = l2.command(L::Edit(E::Move(500)));
            }
            240 => {
                let _ = l2.command(L::Edit(E::Resize(-700)));
            }
            260 => {
                let _ = l2.command(L::Undo);
            }
            280 => {
                moved2 = l2
                    .status()
                    .bounds
                    .is_some_and(|b| b.start == b.captured.0 + 500)
                    && l2.history().done().len() == 2;
            }
            1500 => {
                let _ = l2.copy_layer(0, 0, 1, true);
            }
            1900 => {
                let _ = l2.copy_layer(1, 0, 2, false);
            }
            2300 => {
                let _ = l2.command(L::Undo);
            }
            _ => {}
        }
        copied2 |= (0..3).any(|k| l2.loop_status(k).unwrap().copying.is_some());
        played2 |= l2.loop_status(2).unwrap().phase == Phase::Playing;
        want |= i % 151 == 0;
        bounce(&mut l2, i, &mut want, &mut spare, &mut bounced2);
        let n = sizes[(i % sizes.len() as u64) as usize];
        out[..2 * n].fill(0.0);
        assert!(t2.process(n as u32, &mut b));
        // Every other period each loop apart, then summed (audio tracks
        // phase 4a): no allocation either.
        if i % 2 == 0 {
            l2.process(&b, &input[..2 * n], &mut out[..2 * n]);
        } else {
            l2.process_each(&b, &input[..2 * n]);
            let outs = l2.outs();
            outs.mix_into(&mut out[..2 * n]);
            sum += outs.loop_out(1).first().map_or(0.0, |v| *v as f64 * 0.0);
        }
        let _ = l2.status();
    }
    watch(false);
    assert_eq!(
        calls(),
        0,
        "allocations or frees in the looper's real-time calls"
    );
    assert!(
        seen_playing && layers >= 3 && sum > 1000.0,
        "{seen_playing} {layers} {sum}"
    );
    assert!(l.stats().refused > 0, "{:?}", l.stats());
    assert!(merged, "no merge rendered");
    assert!(bounced >= 1, "{bounced} bounces in the busy session");
    assert!(bounced2 > 10, "{bounced2} bounces in the calm session");
    assert_eq!(l2.loop_status(1).unwrap().phase, Phase::Playing);
    assert!(copied2 && played2, "no layer copied in the calm session");
    assert!(moved2, "loop 0's bounds didn't move in the calm session");
    // The move into loop 2 was taken back: loop 1 has both layers again.
    assert_eq!(l2.loop_status(2).unwrap().phase, Phase::Empty);
    assert_eq!(l2.loop_status(1).unwrap().layers, 2);
    assert!(loops_used.count_ones() >= 3, "loops used: {loops_used:b}");
}

/// Asks for a bounce from whichever loop is ready, and takes a finished
/// one back, its buffer going round.
fn bounce(l: &mut Looper, i: u64, want: &mut bool, spare: &mut Option<Vec<f32>>, n: &mut usize) {
    if *want {
        if let Some(buf) = spare.take() {
            match l.bounce(i as usize % MAX_LOOPS, buf) {
                Ok(()) => *want = false,
                Err(buf) => *spare = Some(buf),
            }
        }
    }
    let _ = l.bounce_progress();
    if let Some(b) = l.take_bounce() {
        *n += 1;
        *spare = Some(b.audio);
    }
}

/// Saving reads a loop's layers and loading copies a saved loop in, both
/// without allocating (only [`Looper::loop_file`], which makes the list of
/// layers, does, and the host drops what [`Looper::take_loaded`] gives
/// back away from the audio thread). Clear calls a second load off.
#[test]
fn saving_and_loading_never_allocate() {
    const RATE: u32 = 44_100;
    let mut t = Transport::new(RATE, 120.0).unwrap();
    let mut l = Looper::new(RATE, 30);
    let mut b = Block::new();
    let input: Vec<f32> = (0..2 * 4096)
        .map(|i| ((i % 97) as f32 - 48.0) / 200.0)
        .collect();
    let mut out = vec![0.0f32; 2 * 4096];
    l.command(L::SetBars(Some(1)));
    l.command(L::Record);
    t.command(0, T::Play);
    let mut period = |t: &mut Transport, l: &mut Looper, n: usize| {
        assert!(t.process(n as u32, &mut b));
        l.process(&b, &input[..2 * n], &mut out[..2 * n]);
    };
    while l.status().phase != Phase::Playing {
        period(&mut t, &mut l, 128);
    }
    period(&mut t, &mut l, 512);
    let mut f = l.loop_file(0).expect("savable");
    let frames = f.frames;
    for y in &mut f.layers {
        y.audio = vec![0.0; 2 * frames];
    }
    let again = f.clone();
    watch(true);
    for (k, y) in f.layers.iter_mut().enumerate() {
        for (i, piece) in y.audio.chunks_mut(2 * 777).enumerate() {
            assert!(l.read_layer(0, k, 777 * i, piece).is_some());
        }
    }
    l.load(4, f).unwrap();
    for (i, &n) in [1usize, 7, 128, 333, 4096, 64]
        .iter()
        .cycle()
        .take(400)
        .enumerate()
    {
        period(&mut t, &mut l, n);
        if i == 200 {
            assert_eq!(l.loop_status(4).unwrap().phase, Phase::Playing);
        }
    }
    watch(false);
    let loaded = l.take_loaded(4).expect("loaded");
    watch(true);
    l.select_loop(5);
    l.load(5, again).unwrap();
    period(&mut t, &mut l, 64);
    l.command(L::Clear);
    for _ in 0..20 {
        period(&mut t, &mut l, 128);
    }
    watch(false);
    assert_eq!(l.loop_status(5).unwrap().phase, Phase::Empty);
    assert!(l.take_loaded(5).is_some());
    drop(loaded);
    // Kept as the looper holds it (audio tracks phase 4c): raw_layer
    // reads, and the load copies in and puts the views back, without
    // allocating.
    let mut raw = l.loop_file_raw(0).expect("savable");
    for y in &mut raw.layers {
        y.audio = vec![0.0; 2 * y.raw.as_ref().unwrap().size];
    }
    let born: Vec<u64> = (0..raw.layers.len())
        .map(|k| l.loop_layer_info(0, k).unwrap().born)
        .collect();
    let mut piece = vec![0.0f32; 2 * 777];
    watch(true);
    for (k, y) in raw.layers.iter_mut().enumerate() {
        for (i, piece) in y.audio.chunks_mut(2 * 777).enumerate() {
            assert!(l.raw_layer(0, k, 777 * i, piece).is_some());
        }
    }
    // And by a layer's number once it is closed (phase 4c.3).
    for &b in &born {
        assert!(l.raw_of(0, b, 777, &mut piece).is_some());
    }
    l.select_loop(6);
    l.load(6, raw).unwrap();
    for &n in [1usize, 7, 128, 333, 4096, 64].iter().cycle().take(400) {
        period(&mut t, &mut l, n);
    }
    watch(false);
    assert_eq!(l.loop_status(6).unwrap().phase, Phase::Playing);
    assert!(l.take_loaded(6).is_some());
    assert_eq!(calls(), 0, "allocated while saving or loading");
}

/// Starts: When I play: listening by ear and for pads, a note starting
/// the song, one refused, a late note copied in from what was heard, a
/// pickup and a count-in, at every period size, without allocating.
#[test]
fn starting_when_you_play_never_allocates() {
    const RATE: u32 = 44_100;
    let mut t = Transport::new(RATE, 120.0).unwrap();
    let mut l = Looper::new(RATE, 30);
    let mut b = Block::new();
    let mut input = vec![0.0f32; 2 * 4096];
    let mut out = vec![0.0f32; 2 * 4096];
    let sizes = [1usize, 7, 128, 333, 4096, 64, 256];
    let mut starts = 0;
    let mut refused = false;
    let mut leads = 0;
    let mut playing = 0u32;
    watch(true);
    for k in 0..4 {
        let _ = l.select_loop(k);
        let _ = l.command(L::SetStarts(Starts::Play));
        let _ = l.command(L::SetBars(Some(1)));
    }
    let _ = l.select_loop(0);
    let _ = l.command(L::StartsByEar(true));
    let _ = l.command(L::Record);
    for i in 0..4000u64 {
        let f = t.frame();
        match i {
            // Loop 1 armed for a pad hit while loop 0 plays; loop 2 hears
            // a note late in a bar (a pickup); the song stops, and loop 3
            // hears one counting in.
            1200 => {
                let _ = l.command(L::StartsByEar(false));
                let _ = l.select_loop(1);
                let _ = l.command(L::Record);
            }
            1500 => {
                let _ = l.command(L::Played(f));
            }
            2000 => {
                let _ = l.command(L::StartsByEar(true));
                let _ = l.select_loop(2);
                let _ = l.command(L::Record);
            }
            2900 => {
                let _ = t.command(f, T::Stop);
                let _ = l.select_loop(3);
                let _ = l.command(L::Record);
            }
            3000 => {
                let _ = t.command(f, T::SetCountIn(1));
                let _ = t.command(f, T::Record);
            }
            _ => {}
        }
        let n = sizes[(i % sizes.len() as u64) as usize];
        // Silence, then a note now and then.
        let loud = matches!(i % 700, 300..=320);
        for k in 0..n {
            let x = if loud {
                0.2 * ((k % 17) as f32 - 8.0) / 8.0
            } else {
                0.0
            };
            input[2 * k] = x;
            input[2 * k + 1] = -x;
        }
        out[..2 * n].fill(0.0);
        assert!(t.process(n as u32, &mut b));
        l.process(&b, &input[..2 * n], &mut out[..2 * n]);
        // The host: the first start asked for is refused, the rest start
        // the song.
        if l.take_start().is_some() {
            if refused {
                let _ = t.command(t.frame(), T::Play);
                starts += 1;
            } else {
                l.start_refused();
                refused = true;
            }
        }
        for k in 0..4 {
            let s = l.loop_status(k).unwrap();
            playing |= u32::from(s.phase == Phase::Playing) << k;
            leads = leads.max(s.lead);
        }
        let _ = l.stats();
    }
    watch(false);
    assert_eq!(calls(), 0, "allocated while starting when you play");
    assert!(refused && starts >= 1, "{refused} {starts}");
    assert_eq!(playing, 0b1111, "loops playing: {playing:b}");
    assert!(leads > 0, "no pickup");
}

#[test]
fn a_free_pass_setting_the_tempo_never_allocates() {
    // Smart loop end in each mode, with the pads' notes going in.
    for mode in [SmartEnd::Off, SmartEnd::Suggest, SmartEnd::Auto] {
        free_pass_without_allocating(mode);
    }
}

fn free_pass_without_allocating(mode: SmartEnd) {
    const RATE: u32 = 44_100;
    let mut t = Transport::new(RATE, 100.0).unwrap();
    let mut l = Looper::new(RATE, 30);
    let mut b = Block::new();
    let mut input = vec![0.0f32; 2 * 4096];
    let mut out = vec![0.0f32; 2 * 4096];
    let sizes = [1usize, 7, 128, 333, 4096, 64, 256];
    let mut tempos = 0;
    let mut said = 0;
    watch(true);
    let _ = l.command(L::TempoFromLoop(true));
    let _ = l.command(L::SetBars(None));
    let _ = l.command(L::SetSmartEnd(mode));
    // Auto hands what was played after the end to the overdub.
    if mode == SmartEnd::Auto {
        let _ = l.command(L::SetAfter(After::Overdub));
    }
    for i in 0..3000u64 {
        let f = t.frame();
        // A pad every few periods, more than the note log holds.
        if i % 3 == 0 {
            l.note(NoteStart {
                frame: f,
                source: 1,
                key: (i % 16) as u8,
                velocity: 100,
            });
        }
        match i {
            // Loop 0 records free from Record; Record again a moment
            // later waits to be a bar at 160 BPM, then sets the tempo.
            10 => {
                let _ = l.command(L::Record);
            }
            40 => {
                let _ = l.command(L::Record);
            }
            // Cleared and stopped: loop 0 again, from a note heard; PLAY
            // under it throws it away; then from a pad, closed.
            900 => {
                let _ = t.command(f, T::Stop);
                let _ = l.command(L::Forget);
            }
            950 => {
                let _ = l.command(L::SetStarts(Starts::Play));
                let _ = l.command(L::StartsByEar(true));
                let _ = l.command(L::Record);
            }
            1300 => {
                let _ = t.command(f, T::Play);
            }
            1400 => {
                let _ = t.command(f, T::Stop);
            }
            1500 => {
                let _ = l.command(L::StartsByEar(false));
                let _ = l.command(L::Record);
            }
            1600 => {
                let _ = l.command(L::Played(f));
            }
            1900 => {
                let _ = l.command(L::Record);
            }
            _ => {}
        }
        let n = sizes[(i % sizes.len() as u64) as usize];
        let loud = matches!(i % 300, 100..=200);
        for k in 0..n {
            let x = if loud {
                0.2 * ((k % 17) as f32 - 8.0) / 8.0
            } else {
                0.0
            };
            input[2 * k] = x;
            input[2 * k + 1] = -x;
        }
        out[..2 * n].fill(0.0);
        assert!(t.process(n as u32, &mut b));
        l.process(&b, &input[..2 * n], &mut out[..2 * n]);
        // The host: the tempo, then PLAY, on the frame asked.
        if let Some((_, r, at, _)) = l.take_tempo() {
            let _ = t.command(at, T::SetTempo(r.bpm));
            let _ = t.command(at, T::Play);
            tempos += 1;
        }
        if l.take_smart().is_some() {
            said += 1;
        }
        let _ = l.status();
        let _ = l.stats();
    }
    watch(false);
    assert_eq!(
        calls(),
        0,
        "allocated while a free pass set the tempo ({mode:?})"
    );
    assert_eq!(said > 0, mode != SmartEnd::Off, "{mode:?}");
    assert_eq!(tempos, 2, "two passes set the tempo");
    if mode == SmartEnd::Auto {
        assert!(l.stats().handoffs >= 1, "{:?}", l.stats());
    }
    assert!(l.stats().aborted >= 1, "PLAY threw one away");
}

/// Free loops ([`Clock::Free`]): one recorded with the song sets the free
/// clock from it and plays on when the song stops; STOP again stops it
/// and PLAY starts it; their own Stop and Play, with the song playing (on
/// its bar line) and stopped; a second free loop on the free clock's bars;
/// both cleared (the clock goes) and one brought back with Undo (and the
/// clock with it); one in your own time setting a new clock; one started
/// by a note on the free clock's bar line: at every period size, without
/// allocating.
#[test]
fn free_loops_never_allocate() {
    const RATE: u32 = 44_100;
    let mut t = Transport::new(RATE, 120.0).unwrap();
    let mut l = Looper::new(RATE, 40);
    let mut b = Block::new();
    let mut input = vec![0.0f32; 2 * 4096];
    let mut out = vec![0.0f32; 2 * 4096];
    let sizes = [1usize, 7, 128, 333, 4096, 64, 256];
    // What was seen: the free clock set from the song (at its tempo), the
    // free loops playing on with the song stopped, the clock stopped,
    // gone, back, and set in your own time; and which loops played.
    let mut from_song = false;
    let mut on_alone = false;
    let mut stopped = false;
    let mut gone = false;
    let mut back = false;
    let mut own_time = false;
    let mut playing = 0u32;
    let mut overdubbed = false;
    let mut undone = false;
    watch(true);
    for k in 0..4 {
        let _ = l.select_loop(k);
        let _ = l.command(L::SetClock(Clock::Free));
        let _ = l.command(L::SetBars(Some(1)));
    }
    let _ = l.select_loop(0);
    let _ = l.command(L::Record);
    let _ = t.command(0, T::Play);
    for i in 0..4200u64 {
        let f = t.frame();
        match i {
            // The song stops: loop 0 plays on. STOP again stops it; PLAY
            // starts the song and it.
            500 => {
                let _ = t.command(f, T::Stop);
            }
            700 => {
                let _ = t.command(f, T::Stop);
            }
            800 => {
                let _ = t.command(f, T::Play);
            }
            // Stop and Play for the free loops while the song plays: Play
            // waits for the song's bar line.
            1000 => {
                let _ = l.command(L::FreeStop);
            }
            1050 => {
                let _ = l.command(L::FreePlay);
            }
            // Loop 1 on the free clock's bars, until a tap.
            1300 => {
                let _ = l.select_loop(1);
                let _ = l.command(L::SetBars(None));
                let _ = l.command(L::Record);
            }
            1500 => {
                let _ = l.command(L::Record);
            }
            // The song stops; both cleared (the free clock goes), and loop
            // 1 back with Undo (the clock with it); STOP stops them.
            1700 => {
                let _ = t.command(f, T::Stop);
                let _ = l.command(L::Clear);
                let _ = l.select_loop(0);
                let _ = l.command(L::Clear);
            }
            1800 => {
                let _ = l.select_loop(1);
                let _ = l.command(L::Undo);
            }
            2000 => {
                let _ = t.command(f, T::Stop);
            }
            // Cleared again; loop 2 in your own time, which sets a new
            // free clock.
            2100 => {
                let _ = l.command(L::Clear);
            }
            2200 => {
                let _ = l.select_loop(2);
                let _ = l.command(L::SetBars(None));
                let _ = l.command(L::Record);
            }
            2400 => {
                let _ = l.command(L::Record);
            }
            // Loop 3 waits for a note, and starts on the free clock's bar
            // line nearest it.
            2600 => {
                let _ = l.select_loop(3);
                let _ = l.command(L::SetStarts(Starts::Play));
                let _ = l.command(L::Record);
            }
            2700 => {
                let _ = l.command(L::Played(f));
            }
            // Their own Stop and Play with the song stopped, at once; then
            // the song plays beside them, and loop 3 takes an overdub,
            // undone, and is cleared.
            3000 => {
                let _ = l.command(L::FreeStop);
            }
            3100 => {
                let _ = l.command(L::FreePlay);
            }
            3300 => {
                let _ = t.command(f, T::Play);
            }
            3400 | 3500 => {
                let _ = l.command(L::Overdub);
            }
            3600 => {
                let _ = l.command(L::Undo);
            }
            3800 => {
                let _ = l.command(L::Clear);
            }
            _ => {}
        }
        let n = sizes[(i % sizes.len() as u64) as usize];
        for k in 0..n {
            let x = 0.2 * (((i as usize + k) % 23) as f32 - 11.0) / 11.0;
            input[2 * k] = x;
            input[2 * k + 1] = -x;
        }
        out[..2 * n].fill(0.0);
        assert!(t.process(n as u32, &mut b));
        l.process(&b, &input[..2 * n], &mut out[..2 * n]);
        let song_stopped = t.state() == mpfree_transport::State::Stopped;
        match l.free_clock() {
            Some(c) => {
                from_song |= c.bpm == 120.0 && c.running && !song_stopped;
                on_alone |= c.running && song_stopped && i < 1700;
                stopped |= !c.running;
                back |= gone && i < 2100;
                own_time |= c.bpm != 120.0 && c.running;
            }
            None => gone |= i >= 1700,
        }
        for k in 0..4 {
            let s = l.loop_status(k).unwrap();
            playing |= u32::from(s.phase == Phase::Playing) << k;
            if k == 3 {
                overdubbed |= s.layers == 2;
                undone |= overdubbed && s.layers == 1 && s.redo == 1;
            }
        }
        let _ = l.take_tempo();
        let _ = l.stats();
    }
    watch(false);
    assert_eq!(calls(), 0, "allocated with free loops");
    assert!(
        from_song && on_alone && stopped && gone && back && own_time,
        "{from_song} {on_alone} {stopped} {gone} {back} {own_time}"
    );
    assert_eq!(playing, 0b1111, "loops playing: {playing:b}");
    assert!(overdubbed && undone, "loop 3 overdubbed and undone");
    assert_eq!(l.loop_status(3).unwrap().phase, Phase::Empty, "and cleared");
    assert_eq!(l.loop_status(2).unwrap().phase, Phase::Playing);
    assert!(l.free_clock().is_some_and(|c| c.running));
    assert_eq!(l.stats().refused, 0, "{:?}", l.stats());
}

/// Make song ([`Looper::make_song`]): two free loops playing with the
/// song stopped become song loops on their bar line, where the song
/// starts at their tempo, as the host starts it; Undo
/// ([`Looper::make_free`]) makes them free again and the song stops; one
/// given with the song never started is refused on the bar line; and with
/// the free loops stopped, the song's place and tempo set at once, it
/// lands at once and PLAY starts them with the song: at every period size,
/// without allocating.
#[test]
fn make_song_never_allocates() {
    const RATE: u32 = 44_100;
    let mut t = Transport::new(RATE, 120.0).unwrap();
    let mut l = Looper::new(RATE, 40);
    let mut b = Block::new();
    let mut input = vec![0.0f32; 2 * 4096];
    let mut out = vec![0.0f32; 2 * 4096];
    let sizes = [1usize, 7, 128, 333, 4096, 64, 256];
    let mut plan = None;
    let (mut made, mut refused, mut freed) = (0u32, 0u32, false);
    let mut song_loops_played = false;
    watch(true);
    for k in 0..2 {
        let _ = l.select_loop(k);
        let _ = l.command(L::SetClock(Clock::Free));
        let _ = l.command(L::SetBars(Some(1)));
    }
    let _ = l.select_loop(0);
    let _ = l.command(L::Record);
    let _ = t.command(0, T::Play);
    for i in 0..2600u64 {
        let f = t.frame();
        match i {
            // The song stops: loop 0 plays on, free; loop 1 records on the
            // free clock's bars.
            400 => {
                let _ = t.command(f, T::Stop);
            }
            500 => {
                let _ = l.select_loop(1);
                let _ = l.command(L::Record);
            }
            // Make song with them playing: the song starts on their bar
            // line, back at the start of its bar, at their tempo.
            1000 => {
                let p = l.song_plan(f).unwrap();
                let at = p.at.unwrap();
                if t.position() != p.tick {
                    assert!(t.command(at, T::Locate(p.tick)));
                }
                assert!(t.command(at, T::SetTempo(p.bpm)));
                assert!(t.command(at, T::Play));
                assert!(l.make_song(p));
                plan = Some(p);
            }
            // The host's Undo: free again, and the song it started stops.
            1400 => {
                freed = l.make_free(plan.unwrap()).is_ok();
                let _ = t.command(f, T::Stop);
            }
            // Given, but the song never starts: refused on the bar line.
            1500 => {
                let p = l.song_plan(f).unwrap();
                assert!(l.make_song(p));
            }
            // STOP again stops the free loops; then Make song lands at once
            // and PLAY starts them with the song.
            1700 => {
                let _ = t.command(f, T::Stop);
            }
            1800 => {
                let p = l.song_plan(f).unwrap();
                assert!(p.at.is_none());
                if t.position() != p.tick {
                    assert!(t.command(f, T::Locate(p.tick)));
                }
                assert!(t.command(f, T::SetTempo(p.bpm)));
                assert!(l.make_song(p));
            }
            2000 => {
                let _ = t.command(f, T::Play);
            }
            _ => {}
        }
        let n = sizes[(i % sizes.len() as u64) as usize];
        for k in 0..n {
            let x = 0.2 * (((i as usize + k) % 23) as f32 - 11.0) / 11.0;
            input[2 * k] = x;
            input[2 * k + 1] = -x;
        }
        out[..2 * n].fill(0.0);
        assert!(t.process(n as u32, &mut b));
        l.process(&b, &input[..2 * n], &mut out[..2 * n]);
        if let Some(d) = l.take_song() {
            match d.refused {
                None => made += 1,
                Some(_) => refused += 1,
            }
        }
        song_loops_played |= i > 2000
            && (0..2).all(|k| {
                let s = l.loop_status(k).unwrap();
                s.clock == Clock::Song && s.phase == Phase::Playing
            });
        let _ = l.take_tempo();
        let _ = l.stats();
    }
    watch(false);
    assert_eq!(calls(), 0, "allocated making a song");
    assert!(freed, "Undo made them free");
    assert_eq!((made, refused), (2, 1), "two made, one refused");
    assert!(song_loops_played, "song loops, playing with the song");
    assert!(l.free_clock().is_none(), "the free clock went with them");
}

/// A free loop read as other bars (a snap that changes the free clock's
/// tempo on its downbeat), then a capture into another free loop on the
/// free clock's bars, asked about first as What you played asks, at odd
/// period sizes: nothing allocates.
#[test]
fn free_snaps_and_captures_never_allocate() {
    const RATE: u32 = 44_100;
    let mut t = Transport::new(RATE, 120.0).unwrap();
    let mut l = Looper::new(RATE, 40);
    let mut b = Block::new();
    let mut input = vec![0.0f32; 2 * 4096];
    let mut out = vec![0.0f32; 2 * 4096];
    let mut lines = [(0i64, 0u64); 16];
    let sizes = [1usize, 7, 128, 333, 4096, 64, 256];
    let (mut planned, mut landed, mut took, mut captured) = (false, false, false, false);
    watch(true);
    // The song stopped: a free pass in your own time sets the free clock.
    let _ = l.command(L::SetClock(Clock::Free));
    let _ = l.command(L::SetBars(None));
    let _ = l.command(L::Record);
    for i in 0..2400u64 {
        match i {
            300 => {
                let _ = l.command(L::Record);
            }
            600 => {
                let s = l.loop_status(0).unwrap();
                let bar = l.free_clock().unwrap().meter.bar_ticks();
                let bars = (s.ticks / bar) as u32;
                if let Ok(p) = l.snap_plan(0, Snap::Bars(bars + 1), t.frame()) {
                    planned = l.snap_at(p);
                }
            }
            1500 => {
                took = l.capture_take(Clock::Free, 1).is_some();
                let _ = l.heard_lines(Clock::Free, &mut lines);
                let _ = l.capture_waits(Clock::Free);
                let _ = l.select_loop(1);
                let _ = l.command(L::SetClock(Clock::Free));
                let _ = l.command(L::Capture(1));
            }
            _ => {}
        }
        let n = sizes[(i % sizes.len() as u64) as usize];
        for k in 0..n {
            let x = 0.2 * (((i as usize + k) % 23) as f32 - 11.0) / 11.0;
            input[2 * k] = x;
            input[2 * k + 1] = -x;
        }
        out[..2 * n].fill(0.0);
        assert!(t.process(n as u32, &mut b));
        l.process(&b, &input[..2 * n], &mut out[..2 * n]);
        if let Some(d) = l.take_snap() {
            landed |= d.landed;
        }
        let s = l.loop_status(1).unwrap();
        captured |= s.phase == Phase::Playing && s.clock == Clock::Free;
        let _ = l.take_tempo();
        let _ = l.stats();
    }
    watch(false);
    assert_eq!(calls(), 0, "allocated");
    assert!(
        planned && landed && took && captured,
        "{planned} {landed} {took} {captured}"
    );
    assert_eq!(l.stats().refused, 0, "{:?}", l.stats());
}

/// Smart loop end by ear, in Suggest and Auto: the hit track listening to
/// a free pass of the kit's beat on the line input, the finder's job
/// filled, answered and taken, at odd period sizes, allocate nothing.
#[test]
fn a_free_pass_by_ear_never_allocates() {
    use mpfree_looper::find::{find, FindJob};
    use mpfree_looper::hits::kit::Kit;
    const RATE: u32 = 22_050;
    for mode in [SmartEnd::Suggest, SmartEnd::Auto] {
        let mut t = Transport::new(RATE, 100.0).unwrap();
        let mut l = Looper::new(RATE, 30);
        let mut b = Block::new();
        let mut out = vec![0.0f32; 2 * 4096];
        let mut audio = vec![0.0f32; 2 * 6 * RATE as usize];
        let rec_from = 10_000usize;
        Kit::new(RATE).beat(&mut audio, rec_from + 1_764, 120.0, 2, &|_| 0.0);
        let mut job = FindJob::new(mpfree_looper::ends::reach(RATE, f64::NAN));
        let sizes = [1usize, 7, 128, 333, 64, 256];
        let mut said = 0;
        let _ = l.command(L::TempoFromLoop(true));
        let _ = l.command(L::SetBars(None));
        let _ = l.command(L::SetSmartEnd(mode));
        let _ = l.command(L::SmartByEar(true));
        let mut closed = false;
        let mut i = 0u64;
        watch(true);
        while (t.frame() as usize) < audio.len() / 2 - 4096 {
            let f = t.frame() as usize;
            let n = sizes[(i % sizes.len() as u64) as usize];
            i += 1;
            if f >= rec_from
                && t.frame() < rec_from as u64 + 512
                && l.status().phase == Phase::Empty
            {
                let _ = l.command(L::Record);
            }
            if !closed && f >= rec_from + 1_764 + 88_200 + 1_323 {
                let _ = l.command(L::Record);
                closed = true;
            }
            out[..2 * n].fill(0.0);
            assert!(t.process(n as u32, &mut b));
            l.process(&b, &audio[2 * f..2 * (f + n)], &mut out[..2 * n]);
            if l.find_job(&mut job) {
                find(&mut job);
                l.found(&job);
            }
            if let Some((_, r, at, _)) = l.take_tempo() {
                let _ = t.command(at, T::SetTempo(r.bpm));
                let _ = t.command(at, T::Play);
            }
            if l.take_smart().is_some() {
                said += 1;
            }
            let _ = l.stats();
        }
        watch(false);
        assert_eq!(calls(), 0, "allocated by ear ({mode:?})");
        assert_eq!(said, 1, "{mode:?}");
        let st = l.stats();
        assert_eq!((st.finds_asked, st.finds_answered), (1, 1), "{st:?}");
        assert!(st.ends_moved + st.ends_offered == 1, "{st:?}");
    }
}

/// Smart loop end's snaps: Suggest offers the downbeat played before a
/// late tap, and a snap to it, its Undo and Redo, Use my tap, and Read as
/// 4 bars and its Undo, each worked out, given and landed at odd period
/// sizes, allocate nothing.
#[test]
fn snaps_never_allocate() {
    const RATE: u32 = 22_050;
    let mut t = Transport::new(RATE, 100.0).unwrap();
    let mut l = Looper::new(RATE, 30);
    let mut b = Block::new();
    let mut input = vec![0.0f32; 2 * 4096];
    let mut out = vec![0.0f32; 2 * 4096];
    // Short enough that the tap lands within the reach.
    let sizes = [1usize, 7, 128, 333, 64, 256];
    let mut snap: Option<SnapPlan> = None;
    let mut landed = 0;
    // The loop point view's windows, read as the host reads them.
    let mut kept = vec![[0.0f32; 2]; 6_615];
    let mut read = 0;
    let mut want = [
        Snap::Found,
        Snap::Undo,
        Snap::Redo,
        Snap::Tap,
        Snap::Bars(4),
        Snap::Undo,
    ]
    .into_iter();
    let _ = l.command(L::TempoFromLoop(true));
    let _ = l.command(L::SetBars(None));
    let _ = l.command(L::SetSmartEnd(SmartEnd::Suggest));
    // Two bars at 120 BPM on the pads from Record, Record again 60 ms
    // after the downbeat that ends them.
    let mut rec_from = None;
    let mut closed = false;
    watch(true);
    for i in 0..12_000u64 {
        let f = t.frame();
        let n = sizes[(i % sizes.len() as u64) as usize];
        if i == 10 {
            let _ = l.command(L::Record);
            rec_from = Some(f);
        }
        if let Some(r) = rec_from {
            for k in 0..=8u64 {
                let at = r + k * 11_025;
                if at >= f && at < f + n as u64 {
                    l.note(NoteStart {
                        frame: at,
                        source: 0,
                        key: if k % 2 == 0 { 36 } else { 38 },
                        velocity: if k % 2 == 0 { 120 } else { 100 },
                    });
                }
            }
            if !closed && f >= r + 88_200 + 1_323 {
                let _ = l.command(L::Record);
                closed = true;
            }
        }
        for k in 0..n {
            let x = 0.1 * ((k % 13) as f32 - 6.0) / 6.0;
            input[2 * k] = x;
            input[2 * k + 1] = -x;
        }
        out[..2 * n].fill(0.0);
        if let Some(p) = snap.filter(|p| p.at < f + n as u64) {
            let _ = t.command(p.at, T::TempoNow(p.bpm));
            let _ = l.snap_at(p);
            snap = None;
        }
        assert!(t.process(n as u32, &mut b));
        l.process(&b, &input[..2 * n], &mut out[..2 * n]);
        if let Some((_, r, at, _)) = l.take_tempo() {
            let _ = t.command(at, T::SetTempo(r.bpm));
            let _ = t.command(at, T::Play);
        }
        let _ = l.take_smart();
        if l.take_snap().is_some_and(|d| d.landed) {
            landed += 1;
        }
        if i % 7 == 0 && l.read_kept(0, (i % 900) as i64 - 300, &mut kept).is_some() {
            read += 1;
        }
        // The next snap once the last has landed and the offer is in.
        let st = l.status();
        if snap.is_none() && st.snapping.is_none() && (st.offer.is_some() || st.moved.is_some()) {
            if let Some(s) = want.next() {
                snap = Some(l.snap_plan(0, s, t.frame()).expect("a snap"));
            }
        }
    }
    watch(false);
    assert_eq!(calls(), 0, "allocated while snapping");
    assert!(read > 0, "the kept audio was read");
    assert_eq!(
        landed, 6,
        "Snap, Undo, Redo, Use my tap, Read as and its Undo landed"
    );
}

/// A loop on an audio track (`Looper::play`): told where it sounds each
/// period, from two origins and after silence, with an overdub, a merge
/// and refused launches, at every period size, without allocating.
#[test]
fn a_loop_on_a_track_never_allocates() {
    use mpfree_looper::PlayRun;
    const RATE: u32 = 44_100;
    let mut t = Transport::new(RATE, 120.0).unwrap();
    let mut l = Looper::new(RATE, 30);
    let mut b = Block::new();
    let input: Vec<f32> = (0..2 * 4096)
        .map(|i| ((i % 89) as f32 - 44.0) / 200.0)
        .collect();
    l.command(L::SetBars(Some(1)));
    l.command(L::Record);
    t.command(0, T::Play);
    while l.status().phase != Phase::Playing {
        assert!(t.process(128, &mut b));
        l.process_each(&b, &input[..256]);
    }
    let origin = l.loop_status(0).unwrap().origin as f64;
    watch(true);
    for (i, &n) in [1usize, 7, 128, 333, 4096, 64]
        .iter()
        .cycle()
        .take(600)
        .enumerate()
    {
        let n32 = n as u32;
        let runs = match i % 5 {
            // Silent.
            0 => [None, None],
            // All of it, from its own origin.
            1 | 2 => [
                Some(PlayRun {
                    from: 0,
                    to: n32,
                    origin,
                }),
                None,
            ],
            // A new origin halfway.
            3 if n > 1 => [
                Some(PlayRun {
                    from: 0,
                    to: n32 / 2,
                    origin,
                }),
                Some(PlayRun {
                    from: n32 / 2,
                    to: n32,
                    origin: origin + 960.0,
                }),
            ],
            _ => [
                Some(PlayRun {
                    from: 0,
                    to: n32,
                    origin: origin + 960.0,
                }),
                None,
            ],
        };
        let mut told = [PlayRun::default(); 2];
        let mut k = 0;
        for r in runs.into_iter().flatten() {
            told[k] = r;
            k += 1;
        }
        assert!(l.play(0, &told[..k]));
        match i {
            100 => assert!(l.command(L::Overdub)),
            160 => assert!(l.command(L::Overdub)),
            200 => assert!(l.launch(0, false, None)),
            210 => assert!(l.command(L::Mute(true))),
            300 => assert!(l.command(L::Merge)),
            _ => {}
        }
        assert!(t.process(n32, &mut b));
        l.process_each(&b, &input[..2 * n]);
        std::hint::black_box(l.outs().loop_out(0));
    }
    watch(false);
    assert!(l.on_track(0));
    assert_eq!(calls(), 0, "allocated with a loop on a track");
}
