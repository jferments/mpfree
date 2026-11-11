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
    /// (frame, event), in frame order.
    notes: Vec<(u64, Event)>,
    next: usize,
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
        }
    }

    /// A note at `frame`, `len` frames long.
    fn note(&mut self, frame: u64, len: u64, note: u8, velocity: u8) {
        self.notes.push((frame, Event::NoteOn { note, velocity }));
        self.notes.push((frame + len, Event::NoteOff { note }));
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
                let (f, ev) = self.notes[self.next];
                self.capture.input(f.saturating_sub(start) as u32, ev);
                self.next += 1;
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
    ));
    r.notes.push((2100 * TICK, Event::NoteOff { note: 9 }));
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
            },
            song: Some(song),
        })
        .collect();
    Phrase {
        played,
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
                origin: -BAR
            })
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
    let k = keep(&phrase, &mut p, &[], &mut takes, 0, looping, "Capture 2").unwrap();
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
            },
            song: None,
        }],
        length: BAR,
        bar: BAR,
        song_start: None,
        id: 1,
    };
    let k = keep(&phrase, &mut p, &[], &mut takes, 1, now(3 * BAR + 5), "c").unwrap();
    assert_eq!(
        (k.slot, k.launch.map(|l| l.origin)),
        (1, Some(4 * BAR)),
        "slot 1 holds the beat"
    );
    let stopped = Now {
        rolling: false,
        ..now(3 * BAR + 5)
    };
    let k = keep(&phrase, &mut p, &[], &mut takes, 1, stopped, "c").unwrap();
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
            launch: None
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
    }];
    let k = keep(&phrase, &mut p, &late, &mut takes, 1, now(7 * BAR), "new").unwrap();
    assert_eq!((k.slot, k.launch.is_some()), (1, true));
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
        keep(&empty, &mut p, &[], &mut takes, 0, now(0), "c"),
        Err(KeepError::Nothing)
    );
    assert_eq!(
        keep(&phrase, &mut p, &[], &mut takes, 9, now(0), "c"),
        Err(KeepError::NoTrack(9))
    );
    for s in 0..SLOTS {
        p.tracks[2].slots[s] = Some(Clip::new("full", BAR));
    }
    let before = p.clone();
    assert_eq!(
        keep(&phrase, &mut p, &[], &mut takes, 2, now(0), "c"),
        Err(KeepError::NoSlot(2))
    );
    assert_eq!(p, before);
    assert!(takes.done().is_empty());
}
