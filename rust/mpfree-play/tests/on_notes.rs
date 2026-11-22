//! An arp that works in notes (the instrument redesign's step 5d): on a
//! track whose instrument plays notes, a clip of pad hits through the arp
//! goes in as its pads' notes, a clip of notes as it is, and what the arp
//! plays goes out as notes, so both clips and the live pads meet in one
//! arp, in one number space. Every note ends, whatever changes on the way.

use mpfree_arp::{Mode, Order, Rate, Settings};
use mpfree_play::{Event, Launch, Out, PadRows, PlayView, Player, Sink, TrackArps};
use mpfree_seq::{Clip, Holds, Note, Project, Track};
use mpfree_transport::{Block, Command, Transport};

#[derive(Default)]
struct Log(Vec<Out>);
impl Sink for Log {
    fn send(&mut self, o: Out) {
        self.0.push(o);
    }
}

/// Track 0 plays a bar of pad 3 (a clip of hits); track 1 a bar of note 62
/// (a clip of notes). Both through their arps.
fn view() -> PlayView {
    let mut p = Project::new();
    let mut hits = Track::new("hits", 0);
    let mut c = Clip::new("pad 3", 3840);
    c.notes = vec![Note::new(0, 3840, 2, 100)];
    hits.slots[0] = Some(c);
    let mut notes = Track::new("notes", 1);
    let mut n = Clip::new("D4", 3840);
    n.holds = Holds::Notes;
    n.notes = vec![Note::new(0, 3840, 62, 90)];
    notes.slots[0] = Some(n);
    p.tracks = vec![hits, notes];
    let launch = |track| Launch {
        track,
        slot: 0,
        origin: 0,
        start: 0,
    };
    PlayView::compile(&p, &[launch(0), launch(1)]).unwrap()
}

fn arps() -> TrackArps {
    let mut arps = TrackArps::new(44_100, 120.0);
    let mut pads = PadRows::PADS;
    // Pad 3 plays C major, its root last.
    pads.set(2, &[64, 67, 60]);
    for t in 0..2 {
        arps.set_through(t, true);
        arps.set_on_notes(t, true);
        arps.set_pads(t, &pads);
        arps.engine_at_mut(t).set_settings(Settings {
            mode: Mode::Arp,
            order: Order::Up,
            rate: Rate::Sixteenth,
            gate: 50,
            ..Settings::default()
        });
    }
    arps
}

/// `frames` frames in periods of `size`, `each` before each period's
/// arps run.
fn run(
    arps: &mut TrackArps,
    frames: u64,
    size: u32,
    mut each: impl FnMut(&mut TrackArps, u64),
) -> Vec<Out> {
    let mut t = Transport::new(44_100, 120.0).unwrap();
    t.command(0, Command::Play);
    let view = view();
    let mut player = Player::new();
    let mut block = Block::new();
    let mut log = Log::default();
    while t.frame() < frames {
        let n = size.min((frames - t.frame()) as u32);
        t.process(n, &mut block);
        each(arps, block.start_frame);
        player.process(&view, &block, &mut arps.through_sink());
        arps.finish(&block, &mut log);
    }
    assert_eq!(arps.lost(), 0);
    log.0
}

/// Track `track`'s note-ons: (note, as notes).
fn ons(log: &[Out], track: u16) -> Vec<(u8, bool)> {
    log.iter()
        .filter(|o| o.track == track)
        .filter_map(|o| match o.event {
            Event::NoteOn { note, .. } => Some((note, o.notes)),
            _ => None,
        })
        .collect()
}

/// Whether every note-on on `track` has its note-off, by note and kind.
fn all_end(log: &[Out], track: u16) -> bool {
    let mut held = std::collections::HashMap::<(u8, bool), i32>::new();
    for o in log.iter().filter(|o| o.track == track) {
        match o.event {
            Event::NoteOn { note, .. } => *held.entry((note, o.notes)).or_default() += 1,
            Event::NoteOff { note } => *held.entry((note, o.notes)).or_default() -= 1,
            _ => {}
        }
    }
    held.values().all(|&n| n == 0)
}

/// Most of two bars at 120 BPM: the clips play twice.
const TWO_BARS: u64 = 4 * 44_100 - 2_000;

#[test]
fn a_clip_of_pad_hits_goes_into_an_arp_of_notes_as_its_pads_notes() {
    for size in [64, 333, 4096] {
        let mut a = arps();
        let log = run(&mut a, TWO_BARS, size, |_, _| {});
        let hits = ons(&log, 0);
        assert!(!hits.is_empty(), "size {size}");
        // The chord, up, as notes; never pad 3 itself.
        assert_eq!(
            hits[..3],
            [(60, true), (64, true), (67, true)],
            "size {size}"
        );
        assert!(
            hits.iter().all(|&(n, notes)| notes && n != 2),
            "size {size}"
        );
        // A clip of notes goes in as it is.
        let notes = ons(&log, 1);
        assert!(!notes.is_empty());
        assert!(notes.iter().all(|&o| o == (62, true)), "size {size}");
        assert!(all_end(&log, 0) && all_end(&log, 1), "size {size}");
    }
}

#[test]
fn an_arp_of_pad_hits_plays_the_pad_as_before() {
    let mut a = arps();
    a.set_on_notes(0, false);
    let log = run(&mut a, TWO_BARS, 256, |_, _| {});
    let hits = ons(&log, 0);
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|&o| o == (2, false)), "{hits:?}");
    assert!(all_end(&log, 0));
}

#[test]
fn a_new_layout_reaches_new_hits_and_each_hit_ends_what_it_pressed() {
    let mut a = arps();
    // Halfway through the first bar pad 3 plays D minor: the hit sounding
    // keeps C major until it ends; the next bar's hit plays D minor.
    let log = run(&mut a, TWO_BARS, 256, |a, frame| {
        if (44_100..44_100 + 256).contains(&frame) {
            let mut pads = PadRows::PADS;
            pads.set(2, &[65, 69, 62]);
            a.set_pads(0, &pads);
        }
    });
    let before: Vec<u8> = log
        .iter()
        .filter(|o| o.track == 0 && o.frame < 88_200)
        .filter_map(|o| match o.event {
            Event::NoteOn { note, .. } => Some(note),
            _ => None,
        })
        .collect();
    let after: Vec<u8> = log
        .iter()
        .filter(|o| o.track == 0 && o.frame >= 88_200 + 2_000)
        .filter_map(|o| match o.event {
            Event::NoteOn { note, .. } => Some(note),
            _ => None,
        })
        .collect();
    assert!(
        before.iter().all(|n| [60, 64, 67].contains(n)),
        "{before:?}"
    );
    assert!(!after.is_empty());
    assert!(after.iter().all(|n| [62, 65, 69].contains(n)), "{after:?}");
    assert!(all_end(&log, 0));
}

#[test]
fn switching_the_arp_to_pad_hits_ends_its_notes_as_notes() {
    let mut a = arps();
    let log = run(&mut a, TWO_BARS, 256, |a, frame| {
        if (44_100..44_100 + 256).contains(&frame) {
            a.set_on_notes(0, false);
        }
    });
    // Every note it played as notes ends as notes; then it plays pad 3.
    assert!(all_end(&log, 0));
    let hits = ons(&log, 0);
    assert!(hits.contains(&(60, true)));
    assert!(hits.contains(&(2, false)));
}
