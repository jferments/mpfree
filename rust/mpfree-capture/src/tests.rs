use super::*;
use mpfree_seq::Track;
use mpfree_transport::{Command, Transport};

const RATE: u32 = 48_000;
const BPM: f64 = 120.0;
/// At 120 BPM and 48 kHz a tick is 25 frames.
const TICK: u64 = 25;
const BAR: Tick = 3840;
const BEAT: Tick = 960;

/// A transport and a capture, run period by period, with pad notes at
/// absolute frames.
struct Rig {
    transport: Box<Transport>,
    block: Box<Block>,
    capture: Box<Capture>,
    period: u32,
    /// (frame, event, kind), in frame order.
    notes: Vec<(u64, Event, Took)>,
    next: usize,
    /// Routed tracks' notes: (frame, track, event), in frame order.
    routed: Vec<(u64, u8, Event)>,
    next_routed: usize,
}

impl Rig {
    fn new(period: u32) -> Rig {
        Rig {
            transport: Box::new(Transport::new(RATE, BPM).unwrap()),
            block: Box::new(Block::new()),
            capture: Box::new(Capture::new()),
            period,
            notes: Vec::new(),
            next: 0,
            routed: Vec::new(),
            next_routed: 0,
        }
    }

    /// A note a route plays on `track` at `frame`, `len` frames long.
    fn note_on(&mut self, track: u8, frame: u64, len: u64, note: u8, velocity: u8) {
        self.routed
            .push((frame, track, Event::NoteOn { note, velocity }));
        self.routed
            .push((frame + len, track, Event::NoteOff { note }));
        self.routed.sort_by_key(|n| n.0);
    }

    /// An event a route plays on `track` at `frame`.
    fn event_on(&mut self, track: u8, frame: u64, event: Event) {
        self.routed.push((frame, track, event));
        self.routed.sort_by_key(|n| n.0);
    }

    /// A pad's note at `frame`, `len` frames long.
    fn note(&mut self, frame: u64, len: u64, note: u8, velocity: u8) {
        self.note_of(Took::PAD, frame, len, note, velocity);
    }

    /// A pad's or a note's note (`took`) at `frame`, `len` frames long.
    fn note_of(&mut self, took: Took, frame: u64, len: u64, note: u8, velocity: u8) {
        self.notes
            .push((frame, Event::NoteOn { note, velocity }, took));
        self.notes
            .push((frame + len, Event::NoteOff { note }, took));
        self.notes.sort_by_key(|n| n.0);
    }

    /// Pad `pad`'s pressure `value` at `frame`, after the notes there.
    fn press(&mut self, frame: u64, pad: u8, value: u8) {
        self.press_of(Took::PAD, frame, pad, value);
    }

    /// [`Rig::press`] on a pad or a note.
    fn press_of(&mut self, took: Took, frame: u64, note: u8, value: u8) {
        self.notes
            .push((frame, Event::Pressure { note, value }, took));
        self.notes.sort_by_key(|n| n.0);
    }

    /// A note on song tick `tick` while playing from bar 1 at frame 0.
    fn at_tick(&mut self, tick: Tick, len: Tick, note: u8) {
        self.note(tick as u64 * TICK, len as u64 * TICK, note, 100);
    }

    fn command(&mut self, frame: u64, cmd: Command) {
        assert!(self.transport.command(frame, cmd));
    }

    /// Run until the transport's frame reaches `frame`.
    fn run_to(&mut self, frame: u64) {
        while self.transport.frame() < frame {
            let start = self.transport.frame();
            let n = (frame - start).min(u64::from(self.period)) as u32;
            while self.next < self.notes.len() && self.notes[self.next].0 < start + u64::from(n) {
                let (f, ev, took) = self.notes[self.next];
                self.capture
                    .input_as(f.saturating_sub(start) as u32, ev, took);
                self.next += 1;
            }
            while self.next_routed < self.routed.len()
                && self.routed[self.next_routed].0 < start + u64::from(n)
            {
                let (f, track, ev) = self.routed[self.next_routed];
                self.capture
                    .input_on(track, f.saturating_sub(start) as u32, ev, Took::PAD);
                self.next_routed += 1;
            }
            assert!(self.transport.process(n, &mut self.block));
            self.capture.process(&self.block);
        }
    }
}

fn ticks(p: &Phrase) -> Vec<(Tick, Tick, u8)> {
    p.played
        .iter()
        .map(|x| (x.note.tick, x.note.len, x.note.note))
        .collect()
}

#[test]
fn a_phrase_in_time_starts_on_its_bar_and_keeps_its_place() {
    let mut r = Rig::new(128);
    r.command(0, Command::Play);
    // Bar 3 beat 2, bar 3 beat 4 and bar 4 beat 1 and a half.
    r.at_tick(2 * BAR + BEAT, 240, 1);
    r.at_tick(2 * BAR + 3 * BEAT, 480, 2);
    r.at_tick(3 * BAR + 1440, 100, 3);
    r.run_to(6 * BAR as u64 * TICK);
    let p = r.capture.phrase().unwrap();
    assert_eq!(
        ticks(&p),
        vec![(BEAT, 240, 1), (3 * BEAT, 480, 2), (BAR + 1440, 100, 3)]
    );
    assert_eq!(
        (p.length, p.bars(), p.song_start),
        (2 * BAR, 2, Some(2 * BAR))
    );
    assert_eq!(p.played[0].song, Some((2 * BAR + BEAT) as f64));
    assert_eq!(
        r.capture.pending(),
        Some(Pending {
            bars: 2,
            timed: true
        })
    );
    assert_eq!(r.capture.dropped(), 0);
}

#[test]
fn a_free_phrase_starts_on_its_first_note() {
    let mut r = Rig::new(128);
    // Stopped: a beat apart from frame 1000, the last held a bar.
    r.note(1000, 240 * TICK, 5, 90);
    r.note(1000 + BEAT as u64 * TICK, 240 * TICK, 6, 91);
    r.note(1000 + 4 * BEAT as u64 * TICK, BAR as u64 * TICK, 7, 92);
    r.run_to(1000 + 10 * BAR as u64 * TICK);
    let p = r.capture.phrase().unwrap();
    assert_eq!(
        ticks(&p),
        vec![(0, 240, 5), (BEAT, 240, 6), (4 * BEAT, BAR, 7)]
    );
    assert_eq!((p.length, p.song_start), (2 * BAR, None));
    assert_eq!(
        p.played.iter().map(|x| x.note.velocity).collect::<Vec<_>>(),
        vec![90, 91, 92]
    );
    assert_eq!(
        r.capture.pending(),
        Some(Pending {
            bars: 2,
            timed: false
        })
    );
}

#[test]
fn a_bar_of_silence_or_a_start_ends_a_phrase() {
    let mut r = Rig::new(128);
    // Free: two notes, then a bar and a beat of silence, then two more.
    r.note(0, 100, 1, 100);
    r.note(BEAT as u64 * TICK, 100, 1, 100);
    let second = (2 * BAR + BEAT) as u64 * TICK;
    r.note(second, 100, 2, 100);
    r.note(second + BEAT as u64 * TICK, 100, 3, 100);
    r.run_to(second + 2 * BAR as u64 * TICK);
    assert_eq!(
        ticks(&r.capture.phrase().unwrap()),
        vec![(0, 4, 2), (BEAT, 4, 3)]
    );
    // Less than a bar is not a gap: held notes and short rests join.
    let mut r = Rig::new(128);
    r.note(0, (BAR / 2) as u64 * TICK, 1, 100);
    r.note((BAR + BAR / 2 - 10) as u64 * TICK, 100, 2, 100);
    r.run_to(3 * BAR as u64 * TICK);
    assert_eq!(r.capture.phrase().unwrap().played.len(), 2);
    // Played free, then the transport starts: only what came after.
    let mut r = Rig::new(128);
    r.note(0, 100, 1, 100);
    let start = 10_000;
    r.command(start, Command::Play);
    r.note(start + BEAT as u64 * TICK, 100, 2, 100);
    r.run_to(start + 2 * BAR as u64 * TICK);
    let p = r.capture.phrase().unwrap();
    assert_eq!((ticks(&p), p.song_start), (vec![(BEAT, 4, 2)], Some(0)));
}

#[test]
fn a_held_note_ends_now() {
    let mut r = Rig::new(128);
    r.notes.push((
        0,
        Event::NoteOn {
            note: 9,
            velocity: 100,
        },
        Took::PAD,
    ));
    r.run_to(1000 * TICK);
    let p = r.capture.phrase().unwrap();
    assert_eq!(ticks(&p), vec![(0, 1000, 9)]);
    // A pad pressed again while held ends the first press.
    r.notes.push((
        2000 * TICK,
        Event::NoteOn {
            note: 9,
            velocity: 50,
        },
        Took::PAD,
    ));
    r.notes
        .push((2100 * TICK, Event::NoteOff { note: 9 }, Took::PAD));
    r.run_to(3000 * TICK);
    assert_eq!(
        ticks(&r.capture.phrase().unwrap()),
        vec![(0, 2000, 9), (2000, 100, 9)]
    );
}

#[test]
fn a_long_phrase_keeps_its_last_bars() {
    let mut r = Rig::new(4096);
    r.command(0, Command::Play);
    // A note on every beat for 40 bars.
    for b in 0..40 * 4 {
        r.at_tick(b * BEAT, 100, (b % 4) as u8);
    }
    r.run_to(41 * BAR as u64 * TICK);
    let p = r.capture.phrase().unwrap();
    assert_eq!((p.bars(), p.song_start), (MAX_BARS, Some(8 * BAR)));
    assert_eq!(p.played.len(), MAX_BARS as usize * 4);
    assert_eq!(ticks(&p)[0], (0, 100, 0));
    assert_eq!(r.capture.pending().map(|x| x.bars), Some(MAX_BARS));
}

#[test]
fn a_phrase_goes_on_across_a_loop_of_whole_bars() {
    let mut r = Rig::new(128);
    r.command(0, Command::SetLoop(Some((0, 2 * BAR))));
    r.command(0, Command::Play);
    // Bar 2 beat 4 of the first pass, then bar 1 beat 1 and 2 of the
    // second: the clock goes on across the jump.
    r.at_tick(BAR + 3 * BEAT, 100, 1);
    r.at_tick(2 * BAR, 100, 2);
    r.at_tick(2 * BAR + BEAT, 100, 3);
    r.run_to(3 * BAR as u64 * TICK);
    let p = r.capture.phrase().unwrap();
    assert_eq!(
        ticks(&p),
        vec![(3 * BEAT, 100, 1), (BAR, 100, 2), (BAR + BEAT, 100, 3)]
    );
    assert_eq!((p.length, p.song_start), (2 * BAR, Some(BAR)));
    // Each note knows where in the song it was played.
    let songs: Vec<f64> = p.played.iter().map(|x| x.song.unwrap()).collect();
    assert_eq!(songs, vec![(BAR + 3 * BEAT) as f64, 0.0, BEAT as f64]);
}

#[test]
fn every_period_size_gives_the_same_phrase() {
    let play = |period: u32, rolling: bool| {
        let mut r = Rig::new(period);
        if rolling {
            r.command(0, Command::Play);
            r.command(3 * BAR as u64 * TICK, Command::SetTempo(97.0));
        }
        for i in 0..64u64 {
            r.note(
                5000 + i * 9_973,
                2_000 + i * 31,
                (i % 16) as u8,
                60 + i as u8,
            );
        }
        r.run_to(8 * BAR as u64 * TICK);
        r.capture.phrase().unwrap()
    };
    for rolling in [false, true] {
        let want = play(128, rolling);
        assert_eq!(want.played.len(), 64);
        for period in [1, 64, 1000, 4096] {
            assert_eq!(
                play(period, rolling),
                want,
                "period {period}, rolling {rolling}"
            );
        }
    }
}

#[test]
fn pending_goes_quiet_once_kept_until_the_next_note() {
    let mut r = Rig::new(128);
    assert_eq!((r.capture.pending(), r.capture.phrase()), (None, None));
    r.note(0, 100, 1, 100);
    r.run_to(1000 * TICK);
    let p = r.capture.phrase().unwrap();
    r.capture.mark_kept(p.id);
    assert_eq!(r.capture.pending(), None);
    // The phrase is still there to read; it is only marked.
    assert_eq!(r.capture.phrase().unwrap().id, p.id);
    r.capture.forget_kept();
    assert!(r.capture.pending().is_some());
    r.capture.mark_kept(p.id);
    r.note(2000 * TICK, 100, 2, 100);
    r.run_to(3000 * TICK);
    assert_eq!(
        r.capture.pending(),
        Some(Pending {
            bars: 1,
            timed: false
        })
    );
    assert_ne!(r.capture.phrase().unwrap().id, p.id);
    r.capture.clear();
    assert_eq!((r.capture.pending(), r.capture.len()), (None, 0));
}

#[test]
fn the_history_is_a_ring() {
    let mut r = Rig::new(4096);
    // More note starts and ends than the ring holds, in one long phrase:
    // the oldest are gone, the phrase is what is left of it.
    let n = MAX_HITS as u64 / 2 + 100;
    for i in 0..n {
        r.note(i * 10 * TICK, 5 * TICK, 1, 100);
    }
    r.run_to(n * 10 * TICK + 100);
    assert_eq!(r.capture.len(), MAX_HITS);
    let p = r.capture.phrase().unwrap();
    // The ring holds the last MAX_HITS / 2 notes, 10 ticks apart: 22 bars
    // from the oldest note left.
    assert_eq!(p.played.len(), MAX_HITS / 2);
    assert_eq!(
        (p.played[0].note.tick, p.played.last().unwrap().note.tick),
        (0, (MAX_HITS as Tick / 2 - 1) * 10)
    );
    assert_eq!(p.bars(), 22);
}

fn runs(p: &Phrase) -> Vec<Vec<(u32, u8)>> {
    p.played
        .iter()
        .map(|x| {
            p.touch_of(&x.note)
                .iter()
                .map(|t| (t.at, t.value))
                .collect()
        })
        .collect()
}

/// Each value goes on the note its pad holds, timed from the note's start
/// and thinned as recording thins it: 2 steps or more kept at once, one
/// step once it has lasted 10 ms (480 frames, 19.2 ticks), a value the
/// same as the last kept never. The same for every period size, rolling
/// or not.
#[test]
fn pressure_goes_on_the_note_its_pad_held_thinned_as_recording_thins_it() {
    let play = |period: u32, rolling: bool| {
        let mut r = Rig::new(period);
        if rolling {
            r.command(0, Command::Play);
        }
        let f = BEAT as u64 * TICK;
        r.note(f, BEAT as u64 * TICK, 1, 100);
        for (at, value) in [
            (0, 10),
            (250, 11),
            (500, 12),
            (1000, 13),
            (2500, 60),
            (3000, 59),
        ] {
            r.press(f + at, 1, value);
        }
        // Let go: no longer its note's.
        r.press(f + BEAT as u64 * TICK + 100, 1, 5);
        let g = 1440 * TICK;
        r.note(g, 480 * TICK, 2, 100);
        for (at, value) in [(1250, 70), (1500, 70), (2000, 71)] {
            r.press(g + at, 2, value);
        }
        // A pad not held, and a note without pressure.
        r.press(g + 10, 4, 99);
        r.note(3 * f, 100 * TICK, 3, 100);
        r.run_to(2 * BAR as u64 * TICK);
        r.capture.phrase().unwrap()
    };
    for rolling in [true, false] {
        let p = play(128, rolling);
        assert_eq!(
            runs(&p),
            vec![
                // 11 gave way to 12 before it lasted; 13 lasted until 60
                // came, so it went in 480 frames after it came; 59 is kept
                // when the pad lets go.
                vec![(0, 10), (20, 12), (59, 13), (100, 60), (139, 59)],
                vec![(50, 70), (99, 71)],
                vec![],
            ],
            "rolling {rolling}"
        );
        assert!(!p.touch_cut);
        assert_eq!(p.touch.len(), 7);
        for period in [1, 64, 1000, 4096] {
            assert_eq!(play(period, rolling), p, "period {period}");
        }
    }
}

/// The pressure ring is apart from the notes': more values than it holds
/// push out the oldest values, never notes, and the phrase says its first
/// notes' pressure is cut short.
#[test]
fn the_pressure_history_is_a_ring_of_its_own() {
    let mut r = Rig::new(4096);
    // 120 beats, each a note with 300 values on it (0 and 10 by turns).
    let beat = BEAT as u64 * TICK;
    for b in 0..120 {
        r.note(b * beat, beat - 10, 1, 100);
        for i in 0..300 {
            let value = if i % 2 == 0 { 10 } else { 0 };
            r.notes.push((
                b * beat + i * 80,
                Event::Pressure { note: 1, value },
                Took::PAD,
            ));
        }
    }
    r.notes.sort_by_key(|n| n.0);
    r.run_to(120 * beat + BAR as u64 * TICK);
    let p = r.capture.phrase().unwrap();
    assert_eq!(p.played.len(), 120);
    assert_eq!(p.touch.len(), MAX_TOUCH_HITS);
    assert!(p.touch_cut);
    let lens: Vec<usize> = runs(&p).iter().map(Vec::len).collect();
    let cut = 120 * 300 - MAX_TOUCH_HITS;
    assert!(lens[..cut / 300].iter().all(|&n| n == 0));
    assert_eq!(lens[cut / 300], 300 - cut % 300);
    assert!(lens[cut / 300 + 1..].iter().all(|&n| n == 300));
    assert_eq!(r.capture.dropped(), 0);
}

// ---------------------------------------------------------------- keep

fn project() -> Project {
    let mut p = Project::new();
    for i in 0..3 {
        p.tracks.push(Track::new(&format!("T{}", i + 1), i));
    }
    p.tracks[1].slots[0] = Some(Clip::new("beat", 2 * BAR));
    p
}

fn now(position: Tick) -> Now {
    Now {
        position,
        rolling: true,
        meter: Meter::default(),
        loop_range: None,
    }
}

fn timed(notes: &[(Tick, f64)], length: Tick, song_start: Tick) -> Phrase {
    let played = notes
        .iter()
        .map(|&(tick, song)| Played {
            note: Note {
                tick,
                len: 100,
                note: 4,
                velocity: 100,
                chance: 100,
                cond: mpfree_seq::Cond::Always,
                ratchet: 1,
                touch: mpfree_seq::Touch::NONE,
                on: true,
                ramp: 0,
                join: mpfree_seq::Join::Attack,
                group: 0,
                heard: false,
            },
            song: Some(song),
            track: ARMED,
            holds: Holds::Hits,
            paired: false,
        })
        .collect();
    Phrase {
        played,
        touch: Vec::new(),
        touch_cut: false,
        length,
        bar: BAR,
        song_start: Some(song_start),
        id: 7,
    }
}

#[test]
fn kept_in_time_a_new_clip_plays_on_in_its_place() {
    let mut p = project();
    let mut takes = Takes::default();
    // Played at bars 6 and 7 (song 5 bars on), now at bar 9.
    let phrase = timed(
        &[(BEAT, (5 * BAR + BEAT) as f64), (BAR, (6 * BAR) as f64)],
        2 * BAR,
        5 * BAR,
    );
    let k = keep(
        &phrase,
        &mut p,
        &[],
        &mut takes,
        0,
        now(8 * BAR + 100),
        "Capture 1",
        Holds::Hits,
    )
    .unwrap();
    // From bar 1, an odd number of bars back: bar 6 is still its first.
    assert_eq!(
        k,
        Kept {
            track: 0,
            slot: 0,
            take: 1,
            notes: 2,
            bars: 2,
            launch: Some(Launch {
                track: 0,
                slot: 0,
                origin: -BAR,
                start: 5 * BAR,
            }),
            touch: 0,
            touch_lost: 0,
            holds: Holds::Hits,
            left_out: 0,
        }
    );
    let c = p.clip(0, 0).unwrap();
    assert_eq!(
        (c.name.as_str(), c.length, c.notes.len()),
        ("Capture 1", 2 * BAR, 2)
    );
    // In a loop from bar 3: its start, or before.
    let mut looping = now(8 * BAR);
    looping.loop_range = Some((2 * BAR, 10 * BAR));
    let k = keep(
        &phrase,
        &mut p,
        &[],
        &mut takes,
        0,
        looping,
        "Capture 2",
        Holds::Hits,
    )
    .unwrap();
    assert_eq!((k.slot, k.launch.map(|l| l.origin)), (1, Some(-BAR)));
    // Undo takes it back as a whole.
    assert_eq!(takes.undo(&mut p), Some(2));
    assert!(p.clip(0, 1).is_none() && p.clip(0, 0).is_some());
}

#[test]
fn kept_free_a_new_clip_starts_on_the_next_bar() {
    let mut p = project();
    let mut takes = Takes::default();
    let phrase = Phrase {
        played: vec![Played {
            note: Note {
                tick: 0,
                len: 10,
                note: 1,
                velocity: 9,
                chance: 100,
                cond: mpfree_seq::Cond::Always,
                ratchet: 1,
                touch: mpfree_seq::Touch::NONE,
                on: true,
                ramp: 0,
                join: mpfree_seq::Join::Attack,
                group: 0,
                heard: false,
            },
            song: None,
            track: ARMED,
            holds: Holds::Hits,
            paired: false,
        }],
        touch: Vec::new(),
        touch_cut: false,
        length: BAR,
        bar: BAR,
        song_start: None,
        id: 1,
    };
    let k = keep(
        &phrase,
        &mut p,
        &[],
        &mut takes,
        1,
        now(3 * BAR + 5),
        "c",
        Holds::Hits,
    )
    .unwrap();
    assert_eq!(
        (k.slot, k.launch.map(|l| l.origin)),
        (1, Some(4 * BAR)),
        "slot 1 holds the beat"
    );
    let stopped = Now {
        rolling: false,
        ..now(3 * BAR + 5)
    };
    let k = keep(
        &phrase,
        &mut p,
        &[],
        &mut takes,
        1,
        stopped,
        "c",
        Holds::Hits,
    )
    .unwrap();
    assert_eq!((k.slot, k.launch.map(|l| l.origin)), (2, Some(3 * BAR)));
}

#[test]
fn kept_over_the_playing_clip_the_notes_go_into_it() {
    let mut p = project();
    let mut takes = Takes::default();
    // The 2-bar beat on track 2 plays from bar 3; the phrase was played at
    // bar 5 beat 2 and bar 6 beat 1: clip ticks 960 and 3840.
    let launches = [Launch {
        track: 1,
        slot: 0,
        origin: 2 * BAR,
        start: 2 * BAR,
    }];
    let phrase = timed(
        &[(BEAT, (4 * BAR + BEAT) as f64), (BAR, (5 * BAR) as f64)],
        2 * BAR,
        4 * BAR,
    );
    let k = keep(
        &phrase,
        &mut p,
        &launches,
        &mut takes,
        1,
        now(7 * BAR),
        "unused",
        Holds::Hits,
    )
    .unwrap();
    assert_eq!(
        k,
        Kept {
            track: 1,
            slot: 0,
            take: 1,
            notes: 2,
            bars: 2,
            launch: None,
            touch: 0,
            touch_lost: 0,
            holds: Holds::Hits,
            left_out: 0,
        }
    );
    let ticks: Vec<Tick> = p.clip(1, 0).unwrap().notes.iter().map(|n| n.tick).collect();
    assert_eq!(ticks, vec![BEAT, BAR]);
    takes.undo(&mut p);
    assert!(p.clip(1, 0).unwrap().notes.is_empty());
    // Played before the clip was launched: a new clip instead.
    let late = [Launch {
        track: 1,
        slot: 0,
        origin: 6 * BAR,
        start: 6 * BAR,
    }];
    let k = keep(
        &phrase,
        &mut p,
        &late,
        &mut takes,
        1,
        now(7 * BAR),
        "new",
        Holds::Hits,
    )
    .unwrap();
    assert_eq!((k.slot, k.launch.is_some()), (1, true));
}

/// The kept notes keep their pressure when the track records it: as it
/// was in a new clip, on the playing clip's own ticks (twice as many at
/// double speed) after its own pressure, and left off, counted, when the
/// playing clip has no room for it.
#[test]
fn kept_notes_keep_their_pressure_when_the_track_records_it() {
    let point = |at, value| TouchPoint { at, value };
    let mut phrase = timed(
        &[(BEAT, (4 * BAR + BEAT) as f64), (BAR, (5 * BAR) as f64)],
        2 * BAR,
        4 * BAR,
    );
    phrase.touch = vec![point(0, 10), point(50, 90)];
    phrase.played[0].note.touch = Touch { first: 0, len: 2 };
    let values = |c: &Clip| -> Vec<Vec<(u32, u8)>> {
        c.notes
            .iter()
            .map(|n| c.touch_of(n).iter().map(|t| (t.at, t.value)).collect())
            .collect()
    };
    let mut p = project();
    let mut takes = Takes::default();
    let k = keep(
        &phrase,
        &mut p,
        &[],
        &mut takes,
        0,
        now(8 * BAR),
        "c",
        Holds::Hits,
    )
    .unwrap();
    assert_eq!((k.touch, k.touch_lost), (2, 0));
    assert_eq!(
        values(p.clip(0, 0).unwrap()),
        vec![vec![(0, 10), (50, 90)], vec![]]
    );
    // A track that leaves pressure out.
    p.tracks[2].record_touch = false;
    let k = keep(
        &phrase,
        &mut p,
        &[],
        &mut takes,
        2,
        now(8 * BAR),
        "c",
        Holds::Hits,
    )
    .unwrap();
    let c = p.clip(2, k.slot).unwrap();
    assert_eq!((k.touch, c.touch.len()), (0, 0));
    assert!(c.notes.iter().all(|n| n.touch.is_none()));
    // Over the playing clip, at double speed, after its own pressure.
    let launches = [Launch {
        track: 1,
        slot: 0,
        origin: 2 * BAR,
        start: 2 * BAR,
    }];
    let beat = p.tracks[1].slots[0].as_mut().unwrap();
    beat.speed = mpfree_seq::Speed::new(2, 1).unwrap();
    beat.touch.push(point(0, 1));
    let k = keep(
        &phrase,
        &mut p,
        &launches,
        &mut takes,
        1,
        now(7 * BAR),
        "-",
        Holds::Hits,
    )
    .unwrap();
    assert_eq!((k.launch, k.touch, k.touch_lost), (None, 2, 0));
    let c = p.clip(1, 0).unwrap();
    assert_eq!(c.touch.len(), 3);
    assert_eq!(values(c), vec![vec![(0, 10), (100, 90)], vec![]]);
    // Undo takes the notes out; the clip keeps the values it was given.
    takes.undo(&mut p);
    assert!(p.clip(1, 0).unwrap().notes.is_empty());
    // A playing clip with no room: the notes go in without it.
    let beat = p.tracks[1].slots[0].as_mut().unwrap();
    beat.touch
        .resize(mpfree_seq::MAX_TOUCH_POINTS - 1, point(0, 0));
    let k = keep(
        &phrase,
        &mut p,
        &launches,
        &mut takes,
        1,
        now(7 * BAR),
        "-",
        Holds::Hits,
    )
    .unwrap();
    assert_eq!((k.notes, k.touch, k.touch_lost), (2, 0, 2));
    let c = p.clip(1, 0).unwrap();
    assert!(c.notes.iter().all(|n| n.touch.is_none()));
}

#[test]
fn keep_refuses_and_changes_nothing() {
    let mut p = project();
    let mut takes = Takes::default();
    let phrase = timed(&[(0, 0.0)], BAR, 0);
    let empty = Phrase {
        played: vec![],
        ..phrase.clone()
    };
    assert_eq!(
        keep(&empty, &mut p, &[], &mut takes, 0, now(0), "c", Holds::Hits),
        Err(KeepError::Nothing)
    );
    assert_eq!(
        keep(
            &phrase,
            &mut p,
            &[],
            &mut takes,
            9,
            now(0),
            "c",
            Holds::Hits
        ),
        Err(KeepError::NoTrack(9))
    );
    for s in 0..SLOTS {
        p.tracks[2].slots[s] = Some(Clip::new("full", BAR));
    }
    let before = p.clone();
    assert_eq!(
        keep(
            &phrase,
            &mut p,
            &[],
            &mut takes,
            2,
            now(0),
            "c",
            Holds::Hits
        ),
        Err(KeepError::NoSlot(2))
    );
    assert_eq!(p, before);
    assert!(takes.done().is_empty());
}

/// (track, tick, len, note) of each note, in the order played.
fn on_tracks(p: &Phrase) -> Vec<(u8, Tick, Tick, u8)> {
    p.played
        .iter()
        .map(|x| (x.track, x.note.tick, x.note.len, x.note.note))
        .collect()
}

/// Keys a route plays on other tracks go into the same phrase, each held
/// apart by its track (the same note on two tracks is two notes), and the
/// phrase parts by track: the armed track's first, then the others in
/// track order, every part as long as the phrase and on its bar.
#[test]
fn routed_tracks_notes_are_held_apart_and_part_by_track() {
    let mut r = Rig::new(128);
    r.command(0, Command::Play);
    let f = |t: Tick| t as u64 * TICK;
    // Bar 2: the pads and track 3 both play note 1; track 3's is longer.
    r.at_tick(BAR + BEAT, 240, 1);
    r.note_on(2, f(BAR + BEAT), f(960), 1, 80);
    // Track 1 joins in bar 3; a key routed to the armed track (2) itself.
    r.note_on(0, f(2 * BAR), f(100), 5, 70);
    r.note_on(1, f(2 * BAR + BEAT), f(100), 6, 60);
    r.run_to(f(5 * BAR));
    let p = r.capture.phrase().unwrap();
    assert_eq!(
        on_tracks(&p),
        vec![
            (ARMED, BEAT, 240, 1),
            (2, BEAT, 960, 1),
            (0, BAR, 100, 5),
            (1, BAR + BEAT, 100, 6),
        ]
    );
    assert_eq!((p.length, p.song_start), (2 * BAR, Some(BAR)));
    assert_eq!(r.capture.pending().map(|x| x.bars), Some(2));
    let parts = p.split(1);
    let tracks: Vec<usize> = parts.iter().map(|x| x.0).collect();
    assert_eq!(tracks, vec![1, 0, 2]);
    assert_eq!(
        ticks(&parts[0].1),
        vec![(BEAT, 240, 1), (BAR + BEAT, 100, 6)]
    );
    assert_eq!(ticks(&parts[1].1), vec![(BAR, 100, 5)]);
    assert_eq!(ticks(&parts[2].1), vec![(BEAT, 960, 1)]);
    for (_, x) in &parts {
        assert_eq!(
            (x.length, x.bar, x.song_start, x.id),
            (p.length, p.bar, p.song_start, p.id)
        );
    }
    // Only the pads played: one part, the armed track's.
    let mut r = Rig::new(64);
    r.at_tick(0, 100, 3);
    r.run_to(f(BAR));
    let parts = r.capture.phrase().unwrap().split(4);
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].0, 4);
    assert_eq!(r.capture.dropped(), 0);
}

/// A phrase goes on while any track holds a note, and a bar of silence on
/// them all ends it.
#[test]
fn a_phrase_ends_on_a_bar_of_silence_on_every_track() {
    let mut r = Rig::new(128);
    let f = |t: Tick| t as u64 * TICK;
    r.command(0, Command::Play);
    r.at_tick(0, 100, 1);
    // Track 6 holds a note across bars 1 to 3, the pads play in bar 4.
    r.note_on(5, f(BEAT), f(2 * BAR), 2, 90);
    r.at_tick(3 * BAR, 100, 3);
    // Then two bars of silence, and track 6 starts a phrase of its own.
    r.note_on(5, f(6 * BAR), f(100), 4, 90);
    r.run_to(f(4 * BAR));
    let p = r.capture.phrase().unwrap();
    assert_eq!(p.played.len(), 3);
    assert_eq!(p.bars(), 4);
    r.run_to(f(8 * BAR));
    let p = r.capture.phrase().unwrap();
    assert_eq!(on_tracks(&p), vec![(5, 0, 100, 4)]);
    assert_eq!(p.song_start, Some(6 * BAR));
}

/// A track's All off ends its own notes, not another track's.
#[test]
fn a_tracks_all_off_ends_only_its_own_notes() {
    let mut r = Rig::new(128);
    let f = |t: Tick| t as u64 * TICK;
    r.command(0, Command::Play);
    r.notes.push((
        0,
        Event::NoteOn {
            note: 1,
            velocity: 100,
        },
        Took::PAD,
    ));
    r.event_on(
        3,
        0,
        Event::NoteOn {
            note: 1,
            velocity: 100,
        },
    );
    r.event_on(
        7,
        0,
        Event::NoteOn {
            note: 2,
            velocity: 100,
        },
    );
    r.notes.push((f(100), Event::AllOff, Took::PAD));
    r.event_on(3, f(200), Event::AllOff);
    r.event_on(7, f(300), Event::NoteOff { note: 2 });
    r.run_to(f(BAR));
    let p = r.capture.phrase().unwrap();
    assert_eq!(
        on_tracks(&p),
        vec![(ARMED, 0, 100, 1), (3, 0, 200, 1), (7, 0, 300, 2)]
    );
}

/// A routed note whose end never comes holds its phrase open; the
/// track's All off ends it, so the quiet after it ends the phrase and the
/// next note starts a new one.
#[test]
fn a_routed_tracks_all_off_lets_a_new_phrase_start_after_the_gap() {
    let f = |t: Tick| t as u64 * TICK;
    let held = Event::NoteOn {
        note: 2,
        velocity: 90,
    };
    let phrase = |all_off: bool| {
        let mut r = Rig::new(128);
        r.command(0, Command::Play);
        r.event_on(4, f(BEAT), held);
        if all_off {
            r.event_on(4, f(BAR), Event::AllOff);
        }
        r.note_on(4, f(4 * BAR), f(100), 3, 90);
        r.run_to(f(6 * BAR));
        r.capture.phrase().unwrap()
    };
    let stuck = phrase(false);
    assert_eq!(stuck.song_start, Some(0), "held open from bar 1");
    assert_eq!(stuck.played.len(), 2);
    let p = phrase(true);
    assert_eq!(on_tracks(&p), vec![(4, 0, 100, 3)]);
    assert_eq!(p.song_start, Some(4 * BAR));
}

/// A key's pressure goes on its own track's note, and stays with it when
/// the phrase parts.
#[test]
fn routed_pressure_goes_on_its_own_tracks_note() {
    let mut r = Rig::new(128);
    let f = |t: Tick| t as u64 * TICK;
    r.note(0, f(BEAT), 3, 100);
    r.note_on(4, 0, f(BEAT), 3, 100);
    r.press(f(10), 3, 40);
    r.event_on(4, f(20), Event::Pressure { note: 3, value: 90 });
    // A track holding nothing takes none.
    r.event_on(5, f(30), Event::Pressure { note: 3, value: 50 });
    r.run_to(f(BAR));
    let p = r.capture.phrase().unwrap();
    assert_eq!(runs(&p), vec![vec![(10, 40)], vec![(20, 90)]]);
    let parts = p.split(0);
    assert_eq!(parts.iter().map(|x| x.0).collect::<Vec<_>>(), vec![0, 4]);
    assert_eq!(runs(&parts[0].1), vec![vec![(10, 40)]]);
    assert_eq!(runs(&parts[1].1), vec![vec![(20, 90)]]);
    assert_eq!(parts[1].1.touch.len(), 1);
    assert_eq!(parts[1].1.played[0].note.touch.first, 0);
}

/// Up to MAX_VOICES - 1 routed tracks hold notes at once; a note on one
/// more is left out and counted, until a track lets go of its notes.
#[test]
fn a_note_past_the_tracks_that_can_hold_is_left_out() {
    let mut r = Rig::new(128);
    let f = |t: Tick| t as u64 * TICK;
    let routed = (MAX_VOICES - 1) as u8;
    for t in 0..routed {
        r.note_on(t, 0, f(BEAT), 1, 100);
    }
    r.at_tick(0, BEAT, 1);
    // One track too many while they all hold; after they let go, it plays.
    r.note_on(routed, f(10), f(10), 2, 100);
    r.note_on(routed, f(2 * BEAT), f(10), 3, 100);
    r.run_to(f(BAR));
    assert_eq!(r.capture.dropped(), 1);
    let p = r.capture.phrase().unwrap();
    assert_eq!(p.played.len(), MAX_VOICES + 1);
    assert_eq!(on_tracks(&p).last(), Some(&(routed, 2 * BEAT, 10, 3)));
}

/// The parts kept with `Takes::joining` go into their own tracks as one
/// take: one UNDO takes every track's clip back.
#[test]
fn parts_kept_into_their_tracks_are_one_take() {
    let mut p = project();
    let mut takes = Takes::default();
    let mut phrase = timed(&[(0, 0.0), (BEAT, BEAT as f64)], BAR, 0);
    phrase.played[1].track = 2;
    let parts = phrase.split(0);
    let (t0, first) = &parts[0];
    let k = keep(
        first,
        &mut p,
        &[],
        &mut takes,
        *t0,
        now(BAR),
        "Capture 1",
        Holds::Hits,
    )
    .unwrap();
    let (t1, second) = &parts[1];
    let k2 = takes
        .joining(k.take, |takes| {
            keep(
                second,
                &mut p,
                &[],
                takes,
                *t1,
                now(BAR),
                "Capture 1",
                Holds::Hits,
            )
        })
        .unwrap();
    assert_eq!((k.track, k2.track, k2.take), (0, 2, k.take));
    assert_eq!(takes.done().len(), 1);
    assert!(p.clip(0, 0).is_some() && p.clip(2, 0).is_some());
    assert_eq!(takes.undo(&mut p), Some(1));
    assert!(p.clip(0, 0).is_none() && p.clip(2, 0).is_none());
}

// ------------------------------------------------- pads and notes (E4)

const PAD_OF_NOTES: Took = Took {
    holds: Holds::Hits,
    paired: true,
    group: 0,
    heard: false,
};
const NOTE_OF_PAD: Took = Took {
    holds: Holds::Notes,
    paired: true,
    group: 0,
    heard: false,
};
const NOTE_ALONE: Took = Took {
    holds: Holds::Notes,
    paired: false,
    group: 0,
    heard: false,
};

#[test]
fn a_pad_and_a_note_of_one_number_are_two_notes() {
    let mut r = Rig::new(128);
    r.command(0, Command::Play);
    // Pad 3 held a beat, and note 3 held two beats from the same tick:
    // each ends on its own let-go, and each keeps its own pressure.
    r.note_of(PAD_OF_NOTES, 0, BEAT as u64 * TICK, 3, 100);
    r.note_of(NOTE_OF_PAD, 0, 2 * BEAT as u64 * TICK, 3, 90);
    r.press_of(NOTE_OF_PAD, 10 * TICK, 3, 64);
    r.run_to(3 * BAR as u64 * TICK);
    let p = r.capture.phrase().unwrap();
    assert_eq!(ticks(&p), vec![(0, BEAT, 3), (0, 2 * BEAT, 3)]);
    let kinds: Vec<(Holds, bool)> = p.played.iter().map(|x| (x.holds, x.paired)).collect();
    assert_eq!(kinds, vec![(Holds::Hits, true), (Holds::Notes, true)]);
    assert_eq!(runs(&p), vec![vec![], vec![(10, 64)]]);
}

/// Pad 4 played on `notes` (tick, song tick), each touch kept as the pad
/// and as the note 60 it played, as [`timed`].
fn timed_both(notes: &[(Tick, f64)], length: Tick, song_start: Tick) -> Phrase {
    let mut p = timed(notes, length, song_start);
    let pads = p.played.clone();
    p.played.clear();
    for x in pads {
        p.played.push(Played { paired: true, ..x });
        let mut n = Played {
            holds: Holds::Notes,
            paired: true,
            ..x
        };
        n.note.note = 60;
        p.played.push(n);
    }
    p
}

#[test]
fn capture_over_a_playing_clip_keeps_that_clips_kind() {
    let mut p = project();
    let mut takes = Takes::default();
    // Track 2's beat plays from bar 3, and two touches were played over
    // it in time, kept as pads and as notes.
    let launches = [Launch {
        track: 1,
        slot: 0,
        origin: 2 * BAR,
        start: 2 * BAR,
    }];
    let phrase = timed_both(
        &[(BEAT, (4 * BAR + BEAT) as f64), (BAR, (5 * BAR) as f64)],
        2 * BAR,
        4 * BAR,
    );
    let pitches = |p: &Project, slot: usize| -> Vec<u8> {
        p.clip(1, slot)
            .unwrap()
            .notes
            .iter()
            .map(|n| n.note)
            .collect()
    };
    // A clip of pads takes the pads, as an overdub would, whatever a new
    // clip there would hold.
    let k = keep(
        &phrase,
        &mut p,
        &launches,
        &mut takes,
        1,
        now(7 * BAR),
        "c",
        Holds::Notes,
    )
    .unwrap();
    assert_eq!(
        (k.slot, k.launch, k.holds, k.notes, k.left_out),
        (0, None, Holds::Hits, 2, 0)
    );
    assert_eq!(pitches(&p, 0), vec![4, 4]);
    // A clip of notes takes the notes.
    takes.undo(&mut p);
    p.tracks[1].slots[0].as_mut().unwrap().holds = Holds::Notes;
    let k = keep(
        &phrase,
        &mut p,
        &launches,
        &mut takes,
        1,
        now(7 * BAR),
        "c",
        Holds::Hits,
    )
    .unwrap();
    assert_eq!((k.slot, k.holds, k.notes), (0, Holds::Notes, 2));
    assert_eq!(pitches(&p, 0), vec![60, 60]);
    // No clip playing: a new clip of the kind the host says.
    takes.undo(&mut p);
    let k = keep(
        &phrase,
        &mut p,
        &[],
        &mut takes,
        1,
        now(7 * BAR),
        "c",
        Holds::Notes,
    )
    .unwrap();
    assert_eq!((k.slot, k.holds), (1, Holds::Notes));
    assert!(k.launch.is_some());
    assert_eq!(p.clip(1, 1).unwrap().holds, Holds::Notes);
    assert_eq!(pitches(&p, 1), vec![60, 60]);
}

/// Trigger groups: a chord pad's notes come with its group, the phrase
/// keeps it, and they go into a clip of notes as one trigger: a new
/// clip's, or the playing clip's, joining the trigger there with their
/// settings and beside one with others.
#[test]
fn a_chord_pads_notes_are_kept_as_one_trigger() {
    let mut r = Rig::new(128);
    r.command(0, Command::Play);
    let chord = Took {
        group: 5,
        ..NOTE_OF_PAD
    };
    for note in [60, 64, 67] {
        r.note_of(chord, 0, BEAT as u64 * TICK, note, 100);
    }
    r.run_to(3 * BAR as u64 * TICK);
    let p = r.capture.phrase().unwrap();
    let groups: Vec<u8> = p.played.iter().map(|x| x.note.group).collect();
    assert_eq!(groups, vec![5, 5, 5]);

    let mut p = project();
    let mut takes = Takes::default();
    let mut phrase = timed_both(&[(BEAT, (4 * BAR + BEAT) as f64)], BAR, 4 * BAR);
    for x in phrase.played.iter_mut().filter(|x| x.holds == Holds::Notes) {
        x.note.group = 5;
    }
    // The playing clip of notes holds pad 5's trigger on that beat at
    // half chance, and another on the next beat as recorded.
    let c = p.tracks[1].slots[0].as_mut().unwrap();
    c.holds = Holds::Notes;
    c.notes = vec![
        mpfree_seq::Note {
            chance: 50,
            group: 5,
            ..mpfree_seq::Note::new(BEAT, 120, 64, 90)
        },
        mpfree_seq::Note {
            group: 5,
            ..mpfree_seq::Note::new(2 * BEAT, 120, 64, 90)
        },
    ];
    let launches = [Launch {
        track: 1,
        slot: 0,
        origin: 4 * BAR,
        start: 4 * BAR,
    }];
    let kept = |p: &Project, slot: usize| -> Vec<(Tick, u8, u8)> {
        p.clip(1, slot)
            .unwrap()
            .notes
            .iter()
            .map(|n| (n.tick, n.group, n.chance))
            .collect()
    };
    keep(
        &phrase,
        &mut p,
        &launches,
        &mut takes,
        1,
        now(7 * BAR),
        "c",
        Holds::Notes,
    )
    .unwrap();
    assert_eq!(
        kept(&p, 0),
        vec![(BEAT, 5, 50), (2 * BEAT, 5, 100), (BEAT, 129, 100)],
        "beside the trigger with other settings"
    );
    assert!(p.clip(1, 0).unwrap().groups_ok());
    // Over the trigger with its settings: it joins.
    takes.undo(&mut p);
    let mut later = phrase.clone();
    for x in &mut later.played {
        x.song = x.song.map(|s| s + BEAT as f64);
    }
    keep(
        &later,
        &mut p,
        &launches,
        &mut takes,
        1,
        now(7 * BAR),
        "c",
        Holds::Notes,
    )
    .unwrap();
    assert_eq!(kept(&p, 0)[2], (2 * BEAT, 5, 100));
    assert!(p.clip(1, 0).unwrap().groups_ok());
    // No clip playing: a new clip, its trigger as played.
    takes.undo(&mut p);
    keep(
        &phrase,
        &mut p,
        &[],
        &mut takes,
        1,
        now(7 * BAR),
        "c",
        Holds::Notes,
    )
    .unwrap();
    assert_eq!(kept(&p, 1), vec![(BEAT, 5, 100)]);
}

#[test]
fn a_touch_kept_only_as_the_other_kind_is_left_out_and_counted() {
    let mut p = project();
    let mut takes = Takes::default();
    // Two kit pads that play no note, then a note the arp played, which
    // comes with no pad.
    let mut phrase = timed(
        &[
            (0, (4 * BAR) as f64),
            (BEAT, (4 * BAR + BEAT) as f64),
            (2 * BEAT, (4 * BAR + 2 * BEAT) as f64),
        ],
        BAR,
        4 * BAR,
    );
    phrase.played[2].holds = NOTE_ALONE.holds;
    phrase.played[2].note.note = 60;
    let k = keep(
        &phrase,
        &mut p,
        &[],
        &mut takes,
        0,
        now(7 * BAR),
        "c",
        Holds::Notes,
    )
    .unwrap();
    assert_eq!((k.notes, k.left_out, k.holds), (1, 2, Holds::Notes));
    let clip = p.clip(0, k.slot).unwrap();
    assert_eq!(clip.holds, Holds::Notes);
    assert_eq!(
        clip.notes.iter().map(|n| n.note).collect::<Vec<_>>(),
        vec![60]
    );
    // Only the pads, into a clip of notes: nothing it can hold, and
    // nothing changes.
    phrase.played.truncate(2);
    let before = p.clone();
    assert_eq!(
        keep(
            &phrase,
            &mut p,
            &[],
            &mut takes,
            2,
            now(7 * BAR),
            "c",
            Holds::Notes
        ),
        Err(KeepError::OtherKind(Holds::Notes))
    );
    assert_eq!(p, before);
}
