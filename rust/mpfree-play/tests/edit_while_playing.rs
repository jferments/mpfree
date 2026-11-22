//! Editing a clip while it plays in several places (Arrange refactor 2.2,
//! design D2): a clip launched on one track and named by song blocks on two
//! others, edited while the song plays, the new view taken between two
//! periods as the app's audio half takes it. Written to pass on the full
//! compile before one compiled copy a clip (2.3), which keeps every rule,
//! and since 2.3 also run with the edit made to the playing view in place
//! (a song-mode tap), which must play the same:
//!
//! - every place the clip plays hears the change from the next note the
//!   player reads;
//! - a note already sounding ends as it was scheduled;
//! - a deleted note that hasn't started doesn't start anywhere;
//! - a note added behind the playhead plays on each place's next pass.
//!
//! Each holds at periods of 1 to 4,096 frames.

use mpfree_play::{Event, InPlace, Launch, Out, PlayView, Player, Sink};
use mpfree_seq::{Clip, Note, Placement, Project, Track};
use mpfree_transport::{Block, Command, Transport};

const BAR: i64 = 3840;
const BEAT: i64 = 960;
/// 120 BPM at 48 kHz: 25 frames a tick.
const RATE: u32 = 48_000;
const TICK: u64 = 25;
/// The edit lands here: bar 2, a beat and a quarter in, while note 61 (on
/// the beat, half a beat long) sounds and before note 62 starts.
const EDIT_AT: i64 = BAR + BEAT + BEAT / 4;
const PERIODS: [u32; 6] = [1, 7, 64, 128, 1000, 4096];
/// Where the clip plays: launched on track 0, two blocks on track 1, one
/// block of four passes on track 2.
const TRACKS: [u16; 3] = [0, 1, 2];

#[derive(Default)]
struct Log(Vec<(u64, u16, Event)>);
impl Sink for Log {
    fn send(&mut self, o: Out) {
        self.0.push((o.frame, o.track, o.event));
    }
}

/// Clip A on track 0: a bar of notes 60 to 63 on the beats, each half a
/// beat long. The song names it on tracks 1 (bars 1 to 2 and 3 to 4) and 2
/// (bars 1 to 4); it is launched on track 0 from the top.
fn project() -> (Project, Vec<Launch>) {
    let mut p = Project::new();
    for (i, name) in ["drums", "bass", "keys"].iter().enumerate() {
        p.tracks.push(Track::new(name, i as u16));
    }
    let mut a = Clip::new("A", BAR);
    a.notes = (0..4)
        .map(|i| Note::new(i * BEAT, BEAT / 2, 60 + i as u8, 100))
        .collect();
    p.tracks[0].slots[0] = Some(a);
    p.give_clip_ids();
    let blocks = [(1, 0, 2 * BAR), (1, 2 * BAR, 2 * BAR), (2, 0, 4 * BAR)];
    for (track, start, len) in blocks {
        let mut b = Placement::of(&p.tracks, 0, 0, start, len);
        b.track = track;
        p.arrangement.place(b).unwrap();
    }
    let launch = Launch {
        track: 0,
        slot: 0,
        origin: 0,
        start: 0,
    };
    (p, vec![launch])
}

fn view(p: &Project, launches: &[Launch]) -> PlayView {
    let mut v = PlayView::compile_cued(p, launches, &[]).unwrap();
    v.set_song(p, true).unwrap();
    v
}

/// The step edit that turns clip A of `p` into clip A of `q`, as
/// `Clip::apply` makes it: the notes erased, then those added.
fn changes(p: &Project, q: &Project) -> Vec<InPlace> {
    let a = &p.tracks[0].slots[0].as_ref().unwrap().notes;
    let b = &q.tracks[0].slots[0].as_ref().unwrap().notes;
    let erased = a.iter().filter(|n| !b.contains(n));
    let added = b.iter().filter(|n| !a.contains(n));
    erased
        .map(|&n| InPlace::EraseNote(n))
        .chain(added.map(|&n| InPlace::AddNote(n)))
        .collect()
}

/// Four bars at `period`, the view of `edited` taken at the first period
/// boundary at or after [`EDIT_AT`]. The events, and the tick the new view
/// came in on.
fn run(edit: impl Fn(&mut Clip), period: u32) -> (Vec<(i64, u16, Event)>, i64) {
    run_with(edit, period, false)
}

/// [`run`], or with `in_place` the edit made there to the view playing
/// (`PlayView::edit_in_place`, as the audio half makes a song-mode tap
/// since 2.3), which must then be what a compile gives.
fn run_with(
    edit: impl Fn(&mut Clip),
    period: u32,
    in_place: bool,
) -> (Vec<(i64, u16, Event)>, i64) {
    let (p, launches) = project();
    let mut q = p.clone();
    edit(q.tracks[0].slots[0].as_mut().unwrap());
    let mut before = view(&p, &launches);
    before.make_room_on(0, 64, 64);
    let after = view(&q, &launches);
    let edit = changes(&p, &q);
    let mut t = Transport::new(RATE, 120.0).unwrap();
    assert!(t.command(0, Command::Play));
    let (mut player, mut block, mut log) = (Player::new(), Block::new(), Log::default());
    let end = 4 * BAR as u64 * TICK;
    let swap_frame = EDIT_AT as u64 * TICK;
    let mut swapped = None;
    while t.frame() < end {
        let f = t.frame();
        if swapped.is_none() && f >= swap_frame {
            swapped = Some(f);
            if in_place {
                assert!(before.edit_in_place(0, 0, &edit), "in place");
                assert!(before == after, "in place, as a compile makes it");
            }
        }
        let n = period.min((end - f) as u32);
        assert!(t.process(n, &mut block));
        let v = if swapped.is_some() && !in_place {
            &after
        } else {
            &before
        };
        player.process(v, &block, &mut log);
    }
    assert_eq!(player.stats().dropped + player.stats().late_offs, 0);
    let at = (swapped.unwrap() / TICK) as i64;
    let played = log
        .0
        .into_iter()
        .map(|(f, track, e)| ((f / TICK) as i64, track, e))
        .collect();
    (played, at)
}

/// (tick, track, velocity) of each note-on of `note`.
fn ons(played: &[(i64, u16, Event)], note: u8) -> Vec<(i64, u16, u8)> {
    played
        .iter()
        .filter_map(|&(at, track, e)| match e {
            Event::NoteOn { note: n, velocity } if n == note => Some((at, track, velocity)),
            _ => None,
        })
        .collect()
}

/// (tick, track) of each note-off of `note`.
fn offs(played: &[(i64, u16, Event)], note: u8) -> Vec<(i64, u16)> {
    played
        .iter()
        .filter_map(|&(at, track, e)| match e {
            Event::NoteOff { note: n } if n == note => Some((at, track)),
            _ => None,
        })
        .collect()
}

/// Each of the three places on `tick`, in track order.
fn everywhere(tick: i64) -> Vec<(i64, u16)> {
    TRACKS.iter().map(|&t| (tick, t)).collect()
}

fn sorted<T: Ord>(mut v: Vec<T>) -> Vec<T> {
    v.sort();
    v
}

#[test]
fn every_place_a_clip_plays_hears_a_change_from_the_next_note() {
    for period in PERIODS {
        let (played, at) = run(|c| c.notes[2].velocity = 50, period);
        assert!(
            at <= BAR + 2 * BEAT,
            "period {period}: in before bar 2's note 62"
        );
        // Note 62, on beat 3 of every bar, everywhere: as it was before the
        // edit, then with the new velocity on every track.
        let want: Vec<(i64, u16, u8)> = (0..4)
            .flat_map(|bar| {
                let tick = bar * BAR + 2 * BEAT;
                let v = if tick < at { 100 } else { 50 };
                TRACKS.map(|t| (tick, t, v))
            })
            .collect();
        assert_eq!(sorted(ons(&played, 62)), want, "period {period}");
    }
}

#[test]
fn a_note_sounding_when_its_clip_changes_ends_as_it_was_scheduled() {
    for period in PERIODS {
        // Note 61 sounds on every track when it goes, and is gone after.
        let (played, at) = run(|c| c.notes.retain(|n| n.note != 61), period);
        let start = BAR + BEAT;
        assert!(at > start && at < start + BEAT / 2, "period {period}");
        let on: Vec<(i64, u16)> = ons(&played, 61).iter().map(|&(t, tr, _)| (t, tr)).collect();
        let want_on: Vec<(i64, u16)> = [BEAT, start].iter().flat_map(|&t| everywhere(t)).collect();
        assert_eq!(sorted(on), want_on, "period {period}");
        // Each ends half a beat after it started, as scheduled: once, and
        // not where the edit came in.
        let want_off: Vec<(i64, u16)> = [BEAT, start]
            .iter()
            .flat_map(|&t| everywhere(t + BEAT / 2))
            .collect();
        assert_eq!(sorted(offs(&played, 61)), want_off, "period {period}");
    }
}

#[test]
fn a_deleted_note_that_hasnt_started_doesnt_start_anywhere() {
    for period in PERIODS {
        let (played, at) = run(|c| c.notes.retain(|n| n.note != 63), period);
        assert!(at < BAR + 3 * BEAT, "period {period}");
        // Note 63 played in bar 1 on every track, then nowhere.
        let on: Vec<(i64, u16)> = ons(&played, 63).iter().map(|&(t, tr, _)| (t, tr)).collect();
        assert_eq!(sorted(on), everywhere(3 * BEAT), "period {period}");
        assert_eq!(
            sorted(offs(&played, 63)),
            everywhere(3 * BEAT + BEAT / 2),
            "period {period}"
        );
    }
}

#[test]
fn a_note_added_behind_the_playhead_plays_on_each_places_next_pass() {
    for period in PERIODS {
        // Note 70, half a beat into the bar: bar 2 has passed it.
        let (played, at) = run(
            |c| {
                c.notes.push(Note::new(BEAT / 2, BEAT / 4, 70, 90));
                c.notes.sort_by_key(|n| n.tick);
            },
            period,
        );
        assert!(at > BAR + BEAT / 2, "period {period}");
        let on: Vec<(i64, u16)> = ons(&played, 70).iter().map(|&(t, tr, _)| (t, tr)).collect();
        let want: Vec<(i64, u16)> = [2, 3]
            .iter()
            .flat_map(|&bar| everywhere(bar * BAR + BEAT / 2))
            .collect();
        assert_eq!(sorted(on), want, "period {period}");
        let want_off: Vec<(i64, u16)> = want.iter().map(|&(t, tr)| (t + BEAT / 4, tr)).collect();
        assert_eq!(sorted(offs(&played, 70)), want_off, "period {period}");
    }
}

#[test]
fn an_edit_made_in_place_plays_as_the_new_view_does() {
    type Edit = fn(&mut Clip);
    let edits: [(&str, Edit); 4] = [
        ("a velocity", |c| c.notes[2].velocity = 50),
        ("a note sounding, deleted", |c| {
            c.notes.retain(|n| n.note != 61)
        }),
        ("a note to come, deleted", |c| {
            c.notes.retain(|n| n.note != 63)
        }),
        ("a note behind the playhead, added", |c| {
            c.notes.push(Note::new(BEAT / 2, BEAT / 4, 70, 90));
            c.notes.sort_by_key(|n| n.tick);
        }),
    ];
    for (what, edit) in edits {
        for period in PERIODS {
            let (in_place, at) = run_with(edit, period, true);
            let (swapped, at_swap) = run_with(edit, period, false);
            assert_eq!(at, at_swap);
            assert_eq!(in_place, swapped, "{what}, period {period}");
        }
    }
}

#[test]
fn a_clip_on_two_tracks_swings_with_each_tracks_swing_through_an_edit() {
    use mpfree_transport::time::Swing;
    // Track 1 swings sixteenths at 66; track 2 plays straight. A note on
    // the second sixteenth, added by the edit, lands late on track 1 only.
    let swing = Swing::new(240, 66).unwrap();
    let late = swing.apply(240.0).round() as i64;
    assert!(late > 240);
    for (period, in_place) in PERIODS.iter().flat_map(|&p| [(p, false), (p, true)]) {
        let (p, launches) = project();
        let mut q = p.clone();
        q.tracks[1].swing = Some(swing);
        let mut p = p;
        p.tracks[1].swing = Some(swing);
        let c = q.tracks[0].slots[0].as_mut().unwrap();
        c.notes.push(Note::new(240, 60, 72, 90));
        c.notes.sort_by_key(|n| n.tick);
        let (mut before, after) = (view(&p, &launches), view(&q, &launches));
        // Tracks 1 and 2 swing apart, so the song keeps a copy for each.
        before.make_room_on(0, 64, 64);
        let edit = changes(&p, &q);
        let mut edited = false;
        let mut t = Transport::new(RATE, 120.0).unwrap();
        assert!(t.command(0, Command::Play));
        let (mut player, mut block, mut log) = (Player::new(), Block::new(), Log::default());
        let end = 4 * BAR as u64 * TICK;
        while t.frame() < end {
            let f = t.frame();
            let n = period.min((end - f) as u32);
            assert!(t.process(n, &mut block));
            let due = f >= EDIT_AT as u64 * TICK;
            if due && in_place && !edited {
                edited = true;
                assert!(before.edit_in_place(0, 0, &edit), "in place");
                assert!(before == after, "both copies, as a compile makes them");
            }
            let v = if due && !in_place { &after } else { &before };
            player.process(v, &block, &mut log);
        }
        let played: Vec<(i64, u16, Event)> = log
            .0
            .into_iter()
            .map(|(f, track, e)| ((f / TICK) as i64, track, e))
            .collect();
        // Bars 3 and 4: straight on tracks 0 and 2, late on track 1.
        let on: Vec<(i64, u16)> = ons(&played, 72).iter().map(|&(t, tr, _)| (t, tr)).collect();
        let want: Vec<(i64, u16)> = [2i64, 3]
            .iter()
            .flat_map(|&bar| {
                let at = bar * BAR;
                [(at + 240, 0), (at + late, 1), (at + 240, 2)]
            })
            .collect();
        assert_eq!(
            sorted(on),
            sorted(want),
            "period {period}, in place {in_place}"
        );
    }
}
