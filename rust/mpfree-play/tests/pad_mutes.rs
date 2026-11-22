//! Pad mutes on one track (the instrument redesign's step 10, Jay's card
//! of 22 Nov): a track's muted pads, on any bank, are left out of its
//! clips, launched, cued or placed in the song, while another track
//! playing the same pads still plays them; a pad solo leaves out the
//! track's other pads; a clip of notes plays every note. The same at any
//! period size.

use mpfree_play::{Cue, Event, Launch, Out, PlayView, Player, Sink};
use mpfree_seq::{Clip, Holds, Note, Placement, Project, Track};
use mpfree_transport::{Block, Command, Transport};

const BAR: i64 = 3840;

#[derive(Default)]
struct Log(Vec<(u16, u8)>);
impl Sink for Log {
    fn send(&mut self, o: Out) {
        if let Event::NoteOn { note, .. } = o.event {
            self.0.push((o.track, note));
        }
    }
}

/// A bar with a hit on pads A3 (2), A6 (5), B5 (20) and H16 (127).
fn clip(holds: Holds) -> Clip {
    let mut c = Clip::new("beat", BAR);
    c.holds = holds;
    for (i, pad) in [2u8, 5, 20, 127].into_iter().enumerate() {
        c.add_note(Note::new(i as i64 * 960, 100, pad, 100))
            .unwrap();
    }
    c
}

/// Plays `p` for most of two bars (120 BPM at 48 kHz) in periods of
/// `size` frames, with `launches` from the start and `cues` waiting for
/// the first bar line, in song mode with `song`: each note-on's (track,
/// note), in order.
fn play(p: &Project, launches: &[Launch], cues: &[Cue], song: bool, size: u32) -> Vec<(u16, u8)> {
    let mut view = PlayView::compile_cued(p, launches, cues).unwrap();
    if song {
        view.set_song(p, true).unwrap();
    }
    let mut t = Transport::new(48_000, 120.0).unwrap();
    assert!(t.command(0, Command::Play));
    let (mut player, mut block, mut log) = (Player::new(), Block::new(), Log::default());
    let end = 2 * 96_000 - 1_000;
    while t.frame() < end {
        let n = size.min((end - t.frame()) as u32);
        t.process(n, &mut block);
        player.process(&view, &block, &mut log);
    }
    assert_eq!(player.stats().dropped, 0);
    log.0
}

fn same_at_any_period(
    p: &Project,
    launches: &[Launch],
    cues: &[Cue],
    song: bool,
) -> Vec<(u16, u8)> {
    let first = play(p, launches, cues, song, 128);
    for size in [1, 64, 333, 4096] {
        assert_eq!(play(p, launches, cues, song, size), first, "period {size}");
    }
    first
}

fn of(played: &[(u16, u8)], track: u16) -> Vec<u8> {
    played
        .iter()
        .filter(|o| o.0 == track)
        .map(|o| o.1)
        .collect()
}

fn launch(track: usize) -> Launch {
    Launch {
        track,
        slot: 0,
        origin: 0,
        start: 0,
    }
}

/// Track 0 plays the beat as it is; track 1, on the same pads, mutes A3
/// and B5; track 2 solos A6 and is cued (it starts on the first line).
#[test]
fn a_pad_muted_on_one_track_is_left_out_there_alone_on_any_bank() {
    let mut p = Project::new();
    for name in ["main", "variation", "solo"] {
        let mut t = Track::new(name, 0);
        t.slots[0] = Some(clip(Holds::Hits));
        p.tracks.push(t);
    }
    p.tracks[1].pad_mutes = 1 << 2 | 1 << 20;
    p.tracks[2].pad_solos = 1 << 5;
    let cues = [Cue {
        track: 2,
        slot: Some(0),
    }];
    let played = same_at_any_period(&p, &[launch(0), launch(1)], &cues, false);
    assert_eq!(of(&played, 0), [2, 5, 20, 127, 2, 5, 20, 127], "untouched");
    assert_eq!(of(&played, 1), [5, 127, 5, 127], "A3 and B5 muted");
    assert_eq!(
        of(&played, 2),
        [5, 5],
        "cued, from the first line: A6 alone"
    );
    // A muted pad stays silent soloed.
    p.tracks[2].pad_mutes = 1 << 5;
    let played = same_at_any_period(&p, &[launch(0), launch(1)], &cues, false);
    assert!(of(&played, 2).is_empty());
}

/// In song mode a placement on the track leaves its muted pads out too.
#[test]
fn a_placement_leaves_its_tracks_muted_pads_out() {
    let mut p = Project::new();
    let mut t = Track::new("drums", 0);
    t.slots[0] = Some(clip(Holds::Hits));
    t.pad_mutes = 1 << 127;
    p.tracks.push(t);
    p.arrangement
        .place(Placement::linked(0, 0, 0, 2 * BAR))
        .unwrap();
    let played = same_at_any_period(&p, &[], &[], true);
    assert_eq!(of(&played, 0), [2, 5, 20, 2, 5, 20], "H16 muted");
}

/// A clip of notes plays every note: its numbers are notes, not pads.
#[test]
fn a_clip_of_notes_plays_every_note_whatever_the_pad_mutes() {
    let mut p = Project::new();
    let mut t = Track::new("keys", 0);
    t.slots[0] = Some(clip(Holds::Notes));
    t.pad_mutes = 1 << 2 | 1 << 20;
    p.tracks.push(t);
    let played = same_at_any_period(&p, &[launch(0)], &[], false);
    assert_eq!(of(&played, 0), [2, 5, 20, 127, 2, 5, 20, 127]);
}
