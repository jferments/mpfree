//! The looper's real-time calls (`command`, `process`, `status`, `stats`,
//! `layer`) must not allocate or free. A counting global allocator checks
//! that across a busy session: the first pass, overdubs, undo and redo,
//! edits, merges, bounces, a resample and layers copied and moved between
//! loops, clear, mute, level and latency
//! changes, a full memory, locates, a transport loop, tempo changes, stop
//! and count-in, at period sizes from 1 to 4096 frames.
//!
//! This test file is its own program, so its global allocator (the only
//! unsafe code here, a pass-through to the system allocator) does not
//! affect the library, which forbids unsafe code. Only the test's own
//! thread is watched. (The same pattern as mpfree-transport's no_alloc
//! test.)

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use mpfree_looper::{After, Command as L, Edit as E, LayerCommand as Lc, Looper, Phase, MAX_LOOPS};
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
    // 12 seconds: a 2-bar loop at 120 BPM is 4 s, so it fills up after
    // two overdubs and the rest are refused.
    let mut l = Looper::new(RATE, 12);
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
        // A layer of loop 0 onto loop 1, loop 1's first pass moved into
        // loop 2, and Undo on loop 1, which takes the move back.
        match i {
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
        l2.process(&b, &input[..2 * n], &mut out[..2 * n]);
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
    assert_eq!(calls(), 0, "allocated while saving or loading");
}
