//! What a clip holds (step 5c of the instrument redesign): a clip of pad
//! hits sends its notes as pad numbers and a clip of notes as notes, each
//! event saying which (`Out::notes`), the note-off, ratchet hits and
//! pressure of a note too, so whoever plays them reads the number right.

use mpfree_play::{Event, Launch, Out, PlayView, Player, Sink};
use mpfree_seq::{Clip, Holds, Note, Project, TouchPoint, Track};
use mpfree_transport::{Block, Command, Transport};

#[derive(Default)]
struct Log(Vec<Out>);
impl Sink for Log {
    fn send(&mut self, out: Out) {
        self.0.push(out);
    }
}

/// Track 0 plays a clip of pad hits, track 1 the same clip as notes, both
/// from tick 0, once, in periods of `size` frames.
fn play(size: u32) -> Vec<Out> {
    let clip = |holds| {
        let mut c = Clip::new("c", 3840);
        c.holds = holds;
        // A plain note with pressure, and a ratchet of three.
        let touch = c.add_touch(&[TouchPoint { at: 60, value: 40 }]).unwrap();
        c.add_note(Note {
            touch,
            ..Note::new(0, 240, 60, 100)
        })
        .unwrap();
        c.add_note(Note {
            ratchet: 3,
            ..Note::new(480, 360, 64, 90)
        })
        .unwrap();
        c
    };
    let mut p = Project::new();
    for (name, holds) in [("hits", Holds::Hits), ("notes", Holds::Notes)] {
        let mut t = Track::new(name, 0);
        t.slots[0] = Some(clip(holds));
        p.tracks.push(t);
    }
    let launches = [0, 1].map(|track| Launch {
        track,
        slot: 0,
        origin: 0,
        start: 0,
    });
    let view = PlayView::compile(&p, &launches).unwrap();
    let mut t = Transport::new(44_100, 120.0).unwrap();
    assert!(t.command(0, Command::Play));
    let mut player = Player::new();
    let mut block = Block::new();
    let mut log = Log::default();
    // Most of one bar at 120 BPM (two seconds), so the clip plays once.
    while t.frame() < 80_000 {
        let n = size.min((80_000 - t.frame()) as u32);
        t.process(n, &mut block);
        player.process(&view, &block, &mut log);
    }
    assert_eq!(player.stats().dropped, 0);
    log.0
}

#[test]
fn a_clip_of_notes_sends_notes_and_one_of_hits_sends_pad_hits() {
    for size in [1, 64, 333, 4096] {
        let played = play(size);
        let of = |track: u16| -> Vec<(Event, bool)> {
            played
                .iter()
                .filter(|o| o.track == track)
                .filter(|o| {
                    matches!(
                        o.event,
                        Event::NoteOn { .. } | Event::NoteOff { .. } | Event::Pressure { .. }
                    )
                })
                .map(|o| (o.event, o.notes))
                .collect()
        };
        let (hits, notes) = (of(0), of(1));
        // The same events from both clips: a note, its pressure and its
        // end, and three ratchet hits, each with its end.
        let events = |v: &[(Event, bool)]| v.iter().map(|&(e, _)| e).collect::<Vec<_>>();
        assert_eq!(events(&hits), events(&notes), "size {size}");
        assert_eq!(hits.len(), 3 + 2 * 3, "size {size}: {hits:?}");
        assert!(
            hits.iter().any(|&(e, _)| e
                == Event::Pressure {
                    note: 60,
                    value: 40
                }),
            "size {size}"
        );
        // Only the kind differs.
        assert!(hits.iter().all(|&(_, n)| !n), "size {size}");
        assert!(notes.iter().all(|&(_, n)| n), "size {size}");
    }
}
