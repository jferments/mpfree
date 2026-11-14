//! One arp per track and the "Clips through the arp" switch (owner
//! decisions 2 and 3, 2026-11-11): what reaches the sink, in frame order,
//! the same for any period size, without allocating on the audio thread.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use mpfree_arp::{LatchMode, Mode, Order, Rate, Settings};
use mpfree_play::{Event, Launch, Out, PlayView, Player, Sink, TrackArps, ALL};
use mpfree_seq::{Clip, Note, Project, Track};
use mpfree_transport::{Block, Command, Transport};

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
static A: Counting = Counting;

const KEYS: usize = 0;
const DRUMS: usize = 1;

/// Track 0 ("keys", dest 0): a one-bar C major chord. Track 1 ("drums",
/// dest 1): a kick on beat 1.
fn view() -> PlayView {
    let mut p = Project::new();
    let mut keys = Track::new("keys", 0);
    let mut c = Clip::new("chord", 3840);
    c.notes = [60u8, 64, 67]
        .iter()
        .map(|&n| Note::new(0, 3840, n, 100))
        .collect();
    keys.slots[0] = Some(c);
    let mut drums = Track::new("drums", 1);
    let mut k = Clip::new("kick", 3840);
    k.notes = vec![Note::new(0, 100, 36, 120)];
    drums.slots[0] = Some(k);
    p.tracks = vec![keys, drums];
    let launch = |track| Launch {
        track,
        slot: 0,
        origin: 0,
        start: 0,
    };
    PlayView::compile(&p, &[launch(0), launch(1)]).unwrap()
}

fn up_sixteenths() -> Settings {
    Settings {
        mode: Mode::Arp,
        order: Order::Up,
        rate: Rate::Sixteenth,
        gate: 50,
        ..Settings::default()
    }
}

#[derive(Default)]
struct Log(Vec<Out>);
impl Sink for Log {
    fn send(&mut self, o: Out) {
        self.0.push(o);
    }
}

/// Runs `frames` frames in periods from `next`, calling `each` with the
/// arps and the period's start frame before each period's arps run.
fn run(
    arps: &mut TrackArps,
    frames: u64,
    mut next: impl FnMut() -> u32,
    mut each: impl FnMut(&mut TrackArps, u64),
) -> Vec<Out> {
    let mut t = Transport::new(44_100, 120.0).unwrap();
    t.command(0, Command::Play);
    let view = view();
    let mut player = Player::new();
    let mut block = Block::new();
    let mut log = Log(Vec::with_capacity(4096));
    while t.frame() < frames {
        let n = next().min((frames - t.frame()) as u32);
        t.process(n, &mut block);
        each(arps, block.start_frame);
        WATCH.with(|w| w.set(true));
        player.process(&view, &block, &mut arps.through_sink());
        arps.finish(&block, &mut log);
        WATCH.with(|w| w.set(false));
    }
    assert_eq!(arps.dropped(), 0);
    log.0
}

fn ons(log: &[Out], track: usize) -> Vec<(u64, u8)> {
    log.iter()
        .filter(|o| usize::from(o.track) == track)
        .filter_map(|o| match o.event {
            Event::NoteOn { note, .. } => Some((o.frame, note)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_through_track_arpeggiates_its_clip_and_a_drum_track_plays_as_recorded() {
    let mut arps = TrackArps::new(44_100, 120.0);
    arps.engine(KEYS).unwrap().set_settings(up_sixteenths());
    arps.set_through(KEYS, true);
    // The drum track's arp is in Arp mode too, but its switch is off.
    arps.engine(DRUMS).unwrap().set_settings(up_sixteenths());
    let log = run(&mut arps, 44_100, || 128, |_, _| {});
    let at = |k: u64| (k as f64 * 5512.5).round() as u64;
    assert_eq!(
        &ons(&log, KEYS)[..4],
        &[(at(0), 60), (at(1), 64), (at(2), 67), (at(3), 60)]
    );
    assert_eq!(
        ons(&log, DRUMS),
        [(0, 36)],
        "the kick once, not arpeggiated"
    );
    assert!(log
        .iter()
        .filter(|o| o.track == KEYS as u16)
        .all(|o| o.dest == 0));
    assert!(
        log.windows(2).all(|w| w[0].frame <= w[1].frame),
        "frame order"
    );
    assert!(arps.records_pads(KEYS) && !arps.records_pads(DRUMS));
}

#[test]
fn switch_off_the_clip_plays_its_notes_as_they_are() {
    let mut arps = TrackArps::new(44_100, 120.0);
    arps.engine(KEYS).unwrap().set_settings(up_sixteenths());
    let log = run(&mut arps, 22_050, || 128, |_, _| {});
    assert_eq!(
        ons(&log, KEYS),
        [(0, 60), (0, 64), (0, 67)],
        "the chord, once"
    );
}

#[test]
fn a_latched_arp_keeps_running_while_the_pads_play_another_track() {
    let mut arps = TrackArps::new(44_100, 120.0);
    let bass = 5;
    arps.engine(bass).unwrap().set_settings(Settings {
        mode: Mode::Repeat,
        ..up_sixteenths()
    });
    arps.engine(bass).unwrap().set_latch(LatchMode::Replace);
    arps.set_dest(bass, 9);
    let log = run(
        &mut arps,
        44_100,
        || 256,
        |a, start| {
            if start == 0 {
                a.pad(
                    bass,
                    0,
                    Event::NoteOn {
                        note: 40,
                        velocity: 90,
                    },
                );
                a.pad(bass, 10, Event::NoteOff { note: 40 });
            } else if (11_000..11_256).contains(&start) {
                // Focus moves to the drums: a live hit through track 1's arp (Direct).
                a.pad(
                    DRUMS,
                    0,
                    Event::NoteOn {
                        note: 38,
                        velocity: 100,
                    },
                );
            }
        },
    );
    let bass_ons = ons(&log, bass);
    assert!(bass_ons.len() >= 7, "a bar of 1/16 repeats: {bass_ons:?}");
    assert!(
        bass_ons.iter().any(|&(f, _)| f > 22_000),
        "still running after the drum hit"
    );
    assert!(log
        .iter()
        .filter(|o| usize::from(o.track) == bass)
        .all(|o| o.dest == 9));
    assert!(ons(&log, DRUMS).iter().any(|&(_, n)| n == 38));
}

#[test]
fn the_same_notes_for_any_period_size_and_no_allocation() {
    let setup = || {
        let mut a = TrackArps::new(44_100, 120.0);
        a.engine(KEYS).unwrap().set_settings(up_sixteenths());
        a.set_through(KEYS, true);
        a
    };
    let reference = run(&mut setup(), 60_000, || 128, |_, _| {});
    let mut x = 7u64;
    let random = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x % 1500) as u32 + 1
    };
    CALLS.store(0, Ordering::Relaxed);
    let other = run(&mut setup(), 60_000, random, |_, _| {});
    assert_eq!(
        CALLS.load(Ordering::Relaxed),
        0,
        "the audio-thread calls allocated"
    );
    let strip = |l: &[Out]| {
        l.iter()
            .map(|o| (o.frame, o.track, o.dest, o.event))
            .collect::<Vec<_>>()
    };
    assert_eq!(strip(&reference), strip(&other));
}

#[test]
fn a_panic_clears_every_arp_and_passes_through() {
    let mut arps = TrackArps::new(44_100, 120.0);
    arps.engine(KEYS).unwrap().set_settings(up_sixteenths());
    arps.set_through(KEYS, true);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    t.command(0, Command::Play);
    let mut block = Block::new();
    t.process(256, &mut block);
    arps.pad(
        KEYS,
        0,
        Event::NoteOn {
            note: 50,
            velocity: 90,
        },
    );
    let mut sink = arps.through_sink();
    sink.send(Out {
        offset: 100,
        frame: 100,
        track: ALL,
        dest: ALL,
        event: Event::AllOff,
    });
    let mut log = Log::default();
    arps.finish(&block, &mut log);
    assert!(log
        .0
        .iter()
        .any(|o| o.event == Event::AllOff && o.dest == ALL));
    t.process(44_100, &mut block);
    let mut later = Log::default();
    arps.finish(&block, &mut later);
    assert!(
        ons(&later.0, KEYS).is_empty(),
        "nothing held after the panic"
    );
}

/// A through track's parameter locks skip its arp (which has no use for
/// them) and reach the sink as the player sent them, ahead of the arp's
/// notes on the same frame; its notes still go into the arp.
#[test]
fn a_through_tracks_locks_go_around_its_arp() {
    use mpfree_play::mpfree_param::{OnTrack, RelAddr};
    let mut arps = TrackArps::new(44_100, 120.0);
    arps.engine(KEYS).unwrap().set_settings(up_sixteenths());
    arps.set_through(KEYS, true);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    t.command(0, Command::Play);
    let mut block = Block::new();
    t.process(256, &mut block);
    let addr = RelAddr::new(OnTrack::Inst, 1).on(KEYS as u8);
    let out = |offset, event| Out {
        offset,
        frame: u64::from(offset),
        track: KEYS as u16,
        dest: 0,
        event,
    };
    let mut sink = arps.through_sink();
    sink.send(out(0, Event::Lock { addr, value: 0.25 }));
    sink.send(out(
        0,
        Event::NoteOn {
            note: 60,
            velocity: 100,
        },
    ));
    sink.send(out(200, Event::Unlock { addr }));
    let mut log = Log::default();
    arps.finish(&block, &mut log);
    let events: Vec<_> = log.0.iter().map(|o| (o.offset, o.event)).collect();
    assert_eq!(
        events,
        [
            (0, Event::Lock { addr, value: 0.25 }),
            (
                0,
                Event::NoteOn {
                    note: 60,
                    velocity: 100
                }
            ),
            (200, Event::Unlock { addr }),
        ],
        "the lock first, then the arp's first step, then the unlock"
    );
    assert_eq!(arps.dropped(), 0);
}

/// An idle arp skips its periods; the notes it plays once a pad wakes it are
/// the same as from an arp run every period, with the transport rolling or
/// stopped.
#[test]
fn an_idle_arp_that_skips_its_periods_plays_the_same_when_woken() {
    for rolling in [true, false] {
        let mut t = Transport::new(44_100, 120.0).unwrap();
        if rolling {
            t.command(0, Command::Play);
        }
        let mut arps = TrackArps::new(44_100, 120.0);
        let bass = 3;
        arps.engine(bass).unwrap().set_settings(up_sixteenths());
        let mut reference = mpfree_play::ArpHost::new(mpfree_arp::Engine::new(44_100, 120.0));
        reference.engine.set_settings(up_sixteenths());
        let (mut block, mut got, mut want) = (Block::new(), Log::default(), Vec::new());
        let mut periods = 0u32;
        while t.frame() < 66_150 {
            let n = 100 + (periods * 37) % 300;
            periods += 1;
            t.process(n, &mut block);
            let start = block.start_frame;
            // Pressed 0.7 s in (mid-period), let go 0.4 s later.
            for (at, e) in [
                (
                    30_870u64,
                    Event::NoteOn {
                        note: 45,
                        velocity: 100,
                    },
                ),
                (48_510, Event::NoteOff { note: 45 }),
            ] {
                if (start..start + u64::from(n)).contains(&at) {
                    arps.pad(bass, (at - start) as u32, e);
                    reference.input((at - start) as u32, e);
                }
            }
            arps.through_sink();
            arps.finish(&block, &mut got);
            reference.process(&block, &mut |offset, kind| {
                want.push((start + u64::from(offset), kind))
            });
        }
        let got: Vec<_> = got
            .0
            .iter()
            .filter(|o| usize::from(o.track) == bass)
            .map(|o| (o.frame, o.event))
            .collect();
        let want: Vec<_> = want
            .iter()
            .map(|&(f, k)| {
                (
                    f,
                    match k {
                        mpfree_arp::EventKind::On { note, velocity } => {
                            Event::NoteOn { note, velocity }
                        }
                        mpfree_arp::EventKind::Off { note } => Event::NoteOff { note },
                    },
                )
            })
            .collect();
        assert!(want.len() >= 6, "rolling {rolling}: {want:?}");
        assert_eq!(got, want, "rolling {rolling}");
    }
}

/// `finish_with` says which notes an arp played (the pads through it, or a
/// through track's clip) and which the player passed on as recorded.
#[test]
fn finish_with_marks_what_the_arps_played() {
    let mut arps = TrackArps::new(44_100, 120.0);
    arps.engine(KEYS).unwrap().set_settings(up_sixteenths());
    arps.set_through(KEYS, true);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    t.command(0, Command::Play);
    let view = view();
    let mut player = Player::new();
    let mut block = Block::new();
    let mut seen: Vec<(u16, u8, bool)> = Vec::new();
    while t.frame() < 11_025 {
        t.process(128, &mut block);
        if block.start_frame == 0 {
            // A live hit on track 3, through its (Direct) arp.
            arps.pad(
                3,
                5,
                Event::NoteOn {
                    note: 7,
                    velocity: 80,
                },
            );
        }
        player.process(&view, &block, &mut arps.through_sink());
        arps.finish_with(&block, |o, arp| {
            if let Event::NoteOn { note, .. } = o.event {
                seen.push((o.track, note, arp));
            }
        });
    }
    // The keys' clip through its arp, the kick as recorded, the pad live.
    assert!(seen.contains(&(KEYS as u16, 60, true)));
    assert!(seen.contains(&(KEYS as u16, 64, true)));
    assert!(seen.contains(&(DRUMS as u16, 36, false)));
    assert!(seen.contains(&(3, 7, true)));
    assert_eq!(seen.iter().filter(|s| !s.2).count(), 1, "{seen:?}");
}

#[test]
fn only_the_tracks_in_use_are_visited_and_the_128th_track_plays() {
    let mut arps = TrackArps::new(44_100, 120.0);
    assert_eq!(arps.used(), 0);
    // A setting on every engine (the tempo, a project's arps) wakes none.
    for e in arps.engines_mut() {
        e.set_tempo(100.0);
    }
    assert_eq!(arps.used(), 0);
    // The last of 128 tracks, latched in Repeat, plays from a pad.
    let last = mpfree_seq::MAX_TRACKS - 1;
    arps.engine(last).unwrap().set_settings(Settings {
        mode: Mode::Repeat,
        ..up_sixteenths()
    });
    arps.engine(last).unwrap().set_latch(LatchMode::Replace);
    assert_eq!(arps.used(), 128);
    assert!(arps.engine(128).is_none(), "no 129th track");
    let log = run(
        &mut arps,
        22_050,
        || 128,
        |a, start| {
            if start == 0 {
                a.pad(
                    last,
                    0,
                    Event::NoteOn {
                        note: 50,
                        velocity: 90,
                    },
                );
                a.pad(last, 10, Event::NoteOff { note: 50 });
            }
        },
    );
    assert!(ons(&log, last).len() >= 3, "{:?}", ons(&log, last));
    assert_eq!(CALLS.load(Ordering::Relaxed), 0, "no allocation");
}
