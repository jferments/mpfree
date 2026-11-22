//! Convert to notes (step sequencer refactor E1, `mpfree_seq::convert`)
//! plays the same events: a clip of pad hits, its hits read through the
//! pads, and the same clip converted, played for several passes in
//! periods of any size, launched, in a song or saved and opened again,
//! send the same notes, ends, pressure, locks and unlocks on the same
//! frames. Ratchets with a ramp, a tie, a slide, a
//! condition, a chord pad with pressure, two pads that play one note apart
//! in time, a hit that is off, a note past the clip's end and a whole-step
//! lock are all in it.

use mpfree_play::mpfree_param::{OnTrack, RelAddr};
use mpfree_play::{Event, Launch, Out, PlayView, Player, Sink};
use mpfree_seq::{
    save, Clip, Cond, Holds, Join, Lock, Note, Placement, Project, TouchPoint, Track,
};
use mpfree_transport::{Block, Command, Transport};

#[derive(Default)]
struct Log(Vec<Out>);
impl Sink for Log {
    fn send(&mut self, out: Out) {
        self.0.push(out);
    }
}

/// The pads as they play now: pad 1 a chord (root last), pads 0 and 3
/// one note between them, the rest nothing.
fn rows(p: u8) -> &'static [u8] {
    match p {
        0 | 3 => &[48],
        1 => &[64, 67, 60],
        2 => &[50],
        _ => &[],
    }
}

fn inst(param: u16) -> RelAddr {
    RelAddr {
        on: OnTrack::Inst,
        param,
    }
}

/// A bar of pad hits.
fn hits() -> Clip {
    let mut c = Clip::new("riff", 3840);
    let touch = c
        .add_touch(&[
            TouchPoint { at: 30, value: 40 },
            TouchPoint { at: 120, value: 90 },
        ])
        .unwrap();
    // A chord pad's pressure: its three notes share the run.
    let chord_touch = c
        .add_touch(&[
            TouchPoint { at: 20, value: 70 },
            TouchPoint { at: 200, value: 30 },
        ])
        .unwrap();
    let notes = [
        Note {
            touch,
            ..Note::new(0, 240, 0, 100)
        },
        Note {
            ratchet: 3,
            ramp: 2,
            touch: chord_touch,
            ..Note::new(480, 360, 1, 90)
        },
        Note {
            cond: Cond::Every { a: 1, b: 2 },
            ..Note::new(960, 240, 2, 80)
        },
        Note {
            join: Join::Tie,
            ..Note::new(1200, 240, 2, 70)
        },
        Note::new(1920, 240, 3, 110),
        Note::new(2400, 480, 2, 60),
        Note {
            join: Join::Slide,
            ..Note::new(2640, 240, 1, 100)
        },
        Note {
            on: false,
            ..Note::new(3000, 100, 0, 100)
        },
        // Past the end: it sounds on into the next pass.
        Note::new(3600, 480, 2, 50),
    ];
    for n in notes {
        c.add_note(n).unwrap();
    }
    c.locks.push(Lock::new(480, Some(1), inst(3), 0.5));
    c.locks.push(Lock::new(1920, None, inst(4), 0.25));
    c
}

/// How [`play`] plays a clip.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Way {
    /// Launched in its cell.
    Launched,
    /// In a song, a block from bar 1 a bar longer than it plays.
    Song,
    /// Launched, the project saved and opened again first.
    Reopened,
}

/// `clip` on track 1 of a project of its own, from tick 0, for four
/// passes, in periods of `size` frames.
fn play(clip: Clip, size: u32, way: Way) -> Vec<Out> {
    play_bars(clip, size, way, 4)
}

/// [`play`] for `bars` bars.
fn play_bars(clip: Clip, size: u32, way: Way, bars: u64) -> Vec<Out> {
    let mut p = Project::new();
    p.tracks.push(Track::new("synth", 0));
    p.put_new_clip(0, 0, clip).unwrap();
    if way == Way::Reopened {
        p = save::from_text(&save::to_text(&p)).unwrap();
    }
    let launch = Launch {
        track: 0,
        slot: 0,
        origin: 0,
        start: 0,
    };
    let view = if way == Way::Song {
        p.arrangement
            .place(Placement::of(&p.tracks, 0, 0, 0, (bars as i64 + 1) * 3840))
            .unwrap();
        let mut v = PlayView::compile(&p, &[]).unwrap();
        v.set_song(&p, true).unwrap();
        v
    } else {
        PlayView::compile(&p, &[launch]).unwrap()
    };
    let mut t = Transport::new(44_100, 120.0).unwrap();
    assert!(t.command(0, Command::Play));
    let mut player = Player::new();
    let mut block = Block::new();
    let mut log = Log::default();
    // A bar at 120 BPM is two seconds.
    let end = bars * 2 * 44_100;
    while t.frame() < end {
        let n = size.min((end - t.frame()) as u32);
        t.process(n, &mut block);
        player.process(&view, &block, &mut log);
    }
    assert_eq!(player.stats().dropped, 0);
    log.0
}

/// Each frame's events, sorted, a pad's read through the pads.
fn by_frame(outs: &[Out]) -> Vec<(u64, Vec<String>)> {
    let mut frames: Vec<(u64, Vec<String>)> = Vec::new();
    for o in outs {
        let events: Vec<Event> = match o.event {
            Event::NoteOn { note, velocity } if !o.notes => rows(note)
                .iter()
                .map(|&note| Event::NoteOn { note, velocity })
                .collect(),
            Event::NoteOff { note } if !o.notes => rows(note)
                .iter()
                .map(|&note| Event::NoteOff { note })
                .collect(),
            Event::Pressure { note, value } if !o.notes => rows(note)
                .iter()
                .map(|&note| Event::Pressure { note, value })
                .collect(),
            e => vec![e],
        };
        if frames.last().is_none_or(|f| f.0 != o.frame) {
            frames.push((o.frame, Vec::new()));
        }
        let at = frames.last_mut().expect("pushed");
        at.1.extend(events.iter().map(|e| format!("{e:?}")));
    }
    for f in &mut frames {
        f.1.sort();
    }
    frames
}

#[test]
fn a_converted_clip_plays_the_same_events_as_its_pad_hits() {
    let hits = hits();
    let made = hits.convert_to_notes(rows).expect("it converts");
    assert_eq!(made.clip.holds, Holds::Notes);
    assert_eq!((made.hits, made.notes), (9, 13));
    let ways = [Way::Launched, Way::Song, Way::Reopened];
    for (way, size) in ways
        .into_iter()
        .flat_map(|w| [64u32, 333, 4096].map(|s| (w, s)))
    {
        let size_way = format!("{way:?}, size {size}");
        let before = play(hits.clone(), size, way);
        let after = play(made.clip.clone(), size, way);
        assert!(before.iter().all(|o| !o.notes), "{size_way}");
        assert!(
            after
                .iter()
                .filter(|o| matches!(o.event, Event::NoteOn { .. }))
                .all(|o| o.notes),
            "{size_way}"
        );
        let (a, b) = (by_frame(&before), by_frame(&after));
        assert_eq!(a, b, "{size_way}");
        // Four passes of it, with pressure and locks.
        let ons = |v: &[(u64, Vec<String>)]| {
            v.iter()
                .flat_map(|f| &f.1)
                .filter(|e| e.starts_with("NoteOn"))
                .count()
        };
        assert!(ons(&a) > 4 * 8, "{size_way}: {}", ons(&a));
        assert!(a
            .iter()
            .flat_map(|f| &f.1)
            .any(|e| e.starts_with("Pressure")));
        assert!(a.iter().flat_map(|f| &f.1).any(|e| e.starts_with("Lock")));
    }
}

/// A bar of pad hits that roll: the chord pad at even odds, a pad after
/// it on PRE and one on NOT PRE, a one-note pad hit twice on one step
/// alike, and two pads that play one note on their own odds.
fn rolls() -> Clip {
    let mut c = Clip::new("rolls", 3840);
    let notes = [
        Note {
            chance: 50,
            ..Note::new(0, 240, 1, 100)
        },
        Note {
            cond: Cond::Pre,
            ..Note::new(480, 240, 2, 90)
        },
        Note {
            cond: Cond::NotPre,
            ..Note::new(960, 240, 2, 80)
        },
        Note {
            chance: 30,
            ..Note::new(1440, 120, 0, 100)
        },
        Note {
            chance: 30,
            ..Note::new(1440, 60, 0, 70)
        },
        Note {
            chance: 70,
            ..Note::new(1920, 240, 3, 100)
        },
        Note {
            chance: 25,
            cond: Cond::Every { a: 1, b: 2 },
            ..Note::new(2880, 240, 1, 60)
        },
    ];
    for n in notes {
        c.add_note(n).unwrap();
    }
    c
}

/// Trigger groups (the trigger groups design): a clip whose hits roll
/// plays the same after Convert, pass for pass, so a chord pad's chord
/// still plays whole or not at all, and PRE still follows it.
#[test]
fn a_converted_clip_rolls_the_same_dice_as_its_pad_hits() {
    let hits = rolls();
    let made = hits.convert_to_notes(rows).expect("it converts");
    assert!(made.clip.groups_ok());
    for (way, size) in [Way::Launched, Way::Reopened]
        .into_iter()
        .flat_map(|w| [64u32, 4096].map(|s| (w, s)))
    {
        let size_way = format!("{way:?}, size {size}");
        let before = by_frame(&play_bars(hits.clone(), size, way, 32));
        let after = by_frame(&play_bars(made.clip.clone(), size, way, 32));
        assert_eq!(before, after, "{size_way}");
        // Each pass's chord on beat 1: all three notes or none, and both
        // on some passes.
        let bar = 2 * 44_100;
        let chords: Vec<usize> = (0..32u64)
            .map(|b| {
                after
                    .iter()
                    .filter(|f| f.0 == b * bar)
                    .flat_map(|f| &f.1)
                    .filter(|e| e.starts_with("NoteOn"))
                    .count()
            })
            .collect();
        assert!(
            chords.iter().all(|&n| n == 0 || n == 3),
            "{size_way}: {chords:?}"
        );
        assert!(
            chords.contains(&0) && chords.contains(&3),
            "{size_way}: {chords:?}"
        );
    }
}
