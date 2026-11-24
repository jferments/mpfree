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

/// A note that plays as heard (what a live tool played, `Note::heard`)
/// skips the arp on a through track, on its frame and once, while the
/// clip's other notes are arpeggiated; Records sets what REC keeps of the
/// track (You: the pads; mpfree and Both: the arp's notes, as heard).
#[test]
fn a_note_that_plays_as_heard_skips_the_arp_with_the_switch_on() {
    use mpfree_play::Keeps;
    let mut p = Project::new();
    let mut keys = Track::new("keys", 0);
    let mut c = Clip::new("take", 3840);
    c.notes = vec![
        Note::new(0, 3840, 60, 100),
        Note::new(0, 3840, 64, 100),
        Note {
            heard: true,
            ..Note::new(960, 240, 79, 90)
        },
    ];
    keys.slots[0] = Some(c);
    p.tracks = vec![keys];
    let launch = Launch {
        track: 0,
        slot: 0,
        origin: 0,
        start: 0,
    };
    let view = PlayView::compile(&p, &[launch]).unwrap();
    let mut arps = TrackArps::new(44_100, 120.0);
    arps.engine(KEYS).unwrap().set_settings(up_sixteenths());
    arps.set_through(KEYS, true);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    t.command(0, Command::Play);
    let (mut player, mut block, mut log) = (Player::new(), Block::new(), Log::default());
    while t.frame() < 44_100 {
        t.process(128, &mut block);
        player.process(&view, &block, &mut arps.through_sink());
        arps.finish(&block, &mut log);
    }
    let at = |k: u64| (k as f64 * 5512.5).round() as u64;
    let ons = ons(&log.0, KEYS);
    // The held two arpeggiated on the sixteenths; the heard note on beat 2
    // (frame 22 050) as it is, once.
    assert_eq!(&ons[..3], &[(at(0), 60), (at(1), 64), (at(2), 60)]);
    assert_eq!(ons.iter().filter(|o| o.1 == 79).count(), 1);
    assert!(ons.contains(&(22_050, 79)));
    let off = log
        .0
        .iter()
        .find(|o| o.event == Event::NoteOff { note: 79 });
    assert_eq!(
        off.map(|o| o.frame),
        Some(27_563),
        "its own end, a sixteenth on"
    );
    assert_eq!(arps.dropped(), 0);
    // What REC keeps: the switch's say until Records says otherwise.
    assert_eq!(arps.keeps(KEYS), Keeps::Switch);
    assert!(arps.records_pads(KEYS));
    for (k, pads, heard) in [
        (Keeps::Yours, true, false),
        (Keeps::Its, false, true),
        (Keeps::Both, false, true),
    ] {
        arps.set_keeps(KEYS, k);
        assert_eq!(
            (arps.records_pads(KEYS), k.as_heard()),
            (pads, heard),
            "{k:?}"
        );
    }
    arps.set_through(KEYS, false);
    arps.set_keeps(KEYS, Keeps::Yours);
    assert!(
        arps.records_pads(KEYS),
        "You keeps the pads with the switch off too"
    );
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
        notes: false,
        heard: false,
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
        notes: false,
        heard: false,
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

/// A track's MIDI effects as `TrackFx`: `fx` in places 0.., the arp after
/// the first `arp_at`.
fn notes_of(fx: &[mpfree_notefx::Fx], arp_at: u8) -> mpfree_notefx::TrackFx {
    let mut t = mpfree_notefx::TrackFx {
        arp_at,
        ..Default::default()
    };
    for (i, f) in fx.iter().enumerate() {
        t.slots[i] = Some(mpfree_notefx::Slot { fx: *f, on: true });
        t.ids[i] = i as u16 + 1;
    }
    t
}

fn with_chain(arps: &mut TrackArps, track: usize, fx: &mpfree_notefx::TrackFx) {
    assert!(arps
        .give_chain(track, Box::new(mpfree_notefx::Chain::new()))
        .is_none());
    arps.set_notes(track, fx);
}

fn offs(log: &[Out], track: usize) -> Vec<(u64, u8)> {
    log.iter()
        .filter(|o| usize::from(o.track) == track)
        .filter_map(|o| match o.event {
            Event::NoteOff { note } => Some((o.frame, note)),
            _ => None,
        })
        .collect()
}

/// Every note on has its off after it, per track.
fn balanced(log: &[Out]) {
    let mut on = std::collections::HashMap::<(u16, u8), i32>::new();
    for o in log {
        match o.event {
            Event::NoteOn { note, .. } => *on.entry((o.track, note)).or_default() += 1,
            Event::NoteOff { note } => {
                let n = on.entry((o.track, note)).or_default();
                *n -= 1;
                assert!(*n >= 0, "an off before its on: {o:?}");
            }
            _ => {}
        }
    }
    assert!(on.values().all(|&n| n == 0), "notes left on: {on:?}");
}

#[test]
fn midi_effects_above_the_arp_change_what_it_plays_and_those_below_what_it_played() {
    use mpfree_notefx::{Chord, Fx, Shape};
    let mut arps = TrackArps::new(44_100, 120.0);
    arps.engine(KEYS).unwrap().set_settings(up_sixteenths());
    arps.set_through(KEYS, true);
    // Transpose up an octave above the arp; a fifth on each arp note below.
    let fx = notes_of(
        &[
            Fx::Transpose { semitones: 12 },
            Fx::Chord(Chord {
                shape: Shape::Fifth,
                ..Chord::default()
            }),
        ],
        1,
    );
    with_chain(&mut arps, KEYS, &fx);
    let log = run(&mut arps, 44_100, || 128, |_, _| {});
    let at = |k: u64| (k as f64 * 5512.5).round() as u64;
    assert_eq!(
        &ons(&log, KEYS)[..6],
        &[
            (at(0), 72),
            (at(0), 79),
            (at(1), 76),
            (at(1), 83),
            (at(2), 79),
            (at(2), 86)
        ]
    );
    assert!(
        log.windows(2).all(|w| w[0].frame <= w[1].frame),
        "frame order"
    );
}

#[test]
fn a_drum_tracks_clip_skips_its_effects_and_its_pads_go_through_them() {
    use mpfree_notefx::Fx;
    let mut arps = TrackArps::new(44_100, 120.0);
    with_chain(
        &mut arps,
        DRUMS,
        &notes_of(&[Fx::Transpose { semitones: 2 }], 0),
    );
    let log = run(
        &mut arps,
        22_050,
        || 128,
        |a, start| {
            if start == 1024 {
                a.pad(
                    DRUMS,
                    5,
                    Event::NoteOn {
                        note: 38,
                        velocity: 100,
                    },
                );
                a.pad(DRUMS, 50, Event::NoteOff { note: 38 });
            }
        },
    );
    // The kick as recorded; the live snare a tone up.
    assert_eq!(ons(&log, DRUMS), [(0, 36), (1029, 40)]);
    assert_eq!(offs(&log, DRUMS), [(1074, 40), (2297, 36)]);
}

#[test]
fn a_track_without_a_chain_yet_plays_as_it_is() {
    use mpfree_notefx::Fx;
    let mut arps = TrackArps::new(44_100, 120.0);
    arps.set_through(KEYS, true);
    let fx = notes_of(&[Fx::Transpose { semitones: 12 }], 0);
    arps.set_notes(KEYS, &fx);
    assert!(!arps.has_chain(KEYS));
    let sorted = |log: &[Out]| {
        let mut o = ons(log, KEYS);
        o.sort();
        o
    };
    let log = run(&mut arps, 2048, || 128, |_, _| {});
    assert_eq!(sorted(&log), [(0, 60), (0, 64), (0, 67)]);
    // Given one, it runs from the next settings; a second is handed back.
    let mut arps = TrackArps::new(44_100, 120.0);
    arps.set_through(KEYS, true);
    with_chain(&mut arps, KEYS, &fx);
    assert!(arps
        .give_chain(KEYS, Box::new(mpfree_notefx::Chain::new()))
        .is_some());
    let log = run(&mut arps, 2048, || 128, |_, _| {});
    assert_eq!(sorted(&log), [(0, 72), (0, 76), (0, 79)]);
}

#[test]
fn a_mute_cuts_the_echoes_waiting_and_a_panic_ends_what_the_effects_added() {
    use mpfree_notefx::{Chord, Echo, Fx, Shape, Time};
    let echo = Fx::Echo(Echo {
        time: Time::Ms(100),
        repeats: 4,
        feedback: 100,
        pitch: 0,
    });
    let mut arps = TrackArps::new(44_100, 120.0);
    with_chain(&mut arps, DRUMS, &notes_of(&[echo], 0));
    // A pad held 1 000 frames, echoed every 4 410; the mute lands at
    // 10 000, between the second and third repeats.
    let log = run(
        &mut arps,
        44_100,
        || 256,
        |a, start| {
            if start == 1024 {
                a.pad(
                    DRUMS,
                    0,
                    Event::NoteOn {
                        note: 40,
                        velocity: 100,
                    },
                );
            }
            if start == 2048 {
                a.pad(DRUMS, 0, Event::NoteOff { note: 40 });
            }
            if start == 9984 {
                a.cut(DRUMS, 16);
            }
        },
    );
    let pads: Vec<_> = ons(&log, DRUMS).into_iter().filter(|o| o.1 == 40).collect();
    assert_eq!(pads, [(1024, 40), (5434, 40), (9844, 40)]);
    let drums: Vec<Out> = log
        .into_iter()
        .filter(|o| o.track == DRUMS as u16)
        .collect();
    assert_eq!(offs(&drums, DRUMS).last(), Some(&(10_000, 40)));
    balanced(&drums);

    // A panic ends the chord the effects made of an arp's note, and
    // nothing they hold plays after it.
    let mut arps = TrackArps::new(44_100, 120.0);
    let chord = Fx::Chord(Chord {
        shape: Shape::Major,
        strum_ms: 50,
        ..Chord::default()
    });
    with_chain(&mut arps, 7, &notes_of(&[chord], 0));
    let mut t = Transport::new(44_100, 120.0).unwrap();
    t.command(0, Command::Play);
    let mut block = Block::new();
    let mut log = Log::default();
    t.process(256, &mut block);
    arps.pad(
        7,
        0,
        Event::NoteOn {
            note: 50,
            velocity: 90,
        },
    );
    arps.finish(&block, &mut log);
    t.process(2048, &mut block);
    arps.through_sink().send(Out {
        offset: 100,
        frame: 356,
        track: ALL,
        dest: ALL,
        event: Event::AllOff,
        notes: false,
        heard: false,
    });
    arps.finish(&block, &mut log);
    t.process(44_100, &mut block);
    arps.finish(&block, &mut log);
    // The root started before the panic; the third and the fifth,
    // strummed 2 205 frames apart, would have started after it.
    assert_eq!(ons(&log.0, 7), [(0, 50)]);
    assert_eq!(offs(&log.0, 7), [(356, 50)]);
    balanced(&log.0);
}

#[test]
fn midi_effects_give_the_same_notes_for_any_period_size_without_allocating() {
    use mpfree_notefx::{Chord, Echo, Fx, Shape, Time};
    let setup = || {
        let mut a = TrackArps::new(44_100, 120.0);
        a.engine(KEYS).unwrap().set_settings(up_sixteenths());
        a.set_through(KEYS, true);
        let fx = notes_of(
            &[
                Fx::Chord(Chord {
                    shape: Shape::Minor7,
                    strum_ms: 7,
                    ..Chord::default()
                }),
                Fx::Echo(Echo {
                    time: Time::Ticks(320),
                    ..Echo::default()
                }),
                Fx::Humanize {
                    time_ms: 10,
                    velocity: 10,
                },
            ],
            1,
        );
        with_chain(&mut a, KEYS, &fx);
        with_chain(
            &mut a,
            DRUMS,
            &notes_of(&[Fx::Transpose { semitones: -1 }], 0),
        );
        a
    };
    let reference = run(&mut setup(), 60_000, || 128, |_, _| {});
    let mut x = 11u64;
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
    assert!(ons(&reference, KEYS).len() > 20);
    assert_eq!(strip(&reference), strip(&other));
}

/// Turning "Clips through the arp" on or off while the clip's notes sound
/// never leaves one hanging: each note-off goes where its note-on went
/// (refactor A2). The chord starts one way, the switch turns half way
/// through its bar, and it plays again the other way on the bar line.
#[test]
fn turning_the_switch_while_a_clip_note_sounds_leaves_nothing_hanging() {
    const BAR: u64 = 88_200;
    let before_line = |log: &[Out]| -> Vec<Out> {
        log.iter()
            .filter(|o| {
                o.frame < BAR || (o.frame == BAR && matches!(o.event, Event::NoteOff { .. }))
            })
            .copied()
            .collect()
    };
    // Off, then on: the chord went to the sink, so its note-offs do too.
    let mut arps = TrackArps::new(44_100, 120.0);
    arps.engine(KEYS).unwrap().set_settings(up_sixteenths());
    let log = run(
        &mut arps,
        BAR + BAR / 4,
        || 128,
        |a, frame| {
            if frame / 128 == BAR / 2 / 128 {
                a.set_through(KEYS, true);
            }
        },
    );
    let offs_at_line: Vec<_> = offs(&log, KEYS)
        .into_iter()
        .filter(|o| o.0 == BAR)
        .collect();
    assert_eq!(offs_at_line, [(BAR, 60), (BAR, 64), (BAR, 67)]);
    balanced(&before_line(&log));
    assert!(
        ons(&log, KEYS).iter().filter(|o| o.0 > BAR).count() > 2,
        "then arpeggiated"
    );
    // On, then off: the arp let go of the chord when the switch turned, so
    // the clip's note-offs have nothing left to end, and the chord plays
    // as it is from the line.
    let mut arps = TrackArps::new(44_100, 120.0);
    arps.engine(KEYS).unwrap().set_settings(up_sixteenths());
    arps.set_through(KEYS, true);
    let log = run(
        &mut arps,
        BAR + BAR / 4,
        || 128,
        |a, frame| {
            if frame / 128 == BAR / 2 / 128 {
                a.set_through(KEYS, false);
            }
        },
    );
    balanced(&before_line(&log));
    let after: Vec<_> = ons(&log, KEYS).into_iter().filter(|o| o.0 >= BAR).collect();
    assert_eq!(after, [(BAR, 60), (BAR, 64), (BAR, 67)]);
    assert_eq!((arps.lost(), arps.refused()), (0, 0));
}
