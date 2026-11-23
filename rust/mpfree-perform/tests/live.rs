//! Live to song (T11) through the transport and the player: what the
//! launch grid plays (launches, scenes with their mutes, stops, a launch
//! over the song) is recorded into the arrangement, and the song then
//! plays back exactly the same notes on the same frames, at any period
//! size. A loop pass records over the last, and the morph's moves and the
//! XY pad's go into the song's lanes.

use mpfree_perform::{Perform, SongRecorder, When};
use mpfree_play::{Cue, Event, Launch, Out, PlayView, Player, Sink, SONG_TRACK};
use mpfree_seq::arrange::{MAX_PLACEMENTS, MORPH_LANE};
use mpfree_seq::mpfree_moves::{Input, Map, Target};
use mpfree_seq::mpfree_param::{Owner, ParamAddr, Strip};
use mpfree_seq::{Clip, MorphRows, Note, Placement, Project, Track};
use mpfree_transport::time::Tick;
use mpfree_transport::{Block, Command, Transport};

const BAR: i64 = 3840;
/// 120 BPM at 48 kHz: 25 frames a tick.
const RATE: u32 = 48_000;
const TICK: u64 = 25;

fn at(tick: i64) -> u64 {
    tick as u64 * TICK
}

#[derive(Default)]
struct Log(Vec<(u64, u16, Event)>);
impl Sink for Log {
    fn send(&mut self, o: Out) {
        self.0.push((o.frame, o.track, o.event));
    }
}

#[derive(Clone, Copy, Debug)]
enum Act {
    Scene(usize),
    /// A launch (or a stop) waiting for the bar line.
    Cue(usize, Option<usize>),
    /// The morph crossfader, at the tick the transport is at.
    Morph(f32, (usize, usize)),
    /// A track muted or heard again at once (not by a scene).
    Mute(usize, bool),
    /// The XY pad moves a setting to a value, or lets go of it (None).
    Xy(ParamAddr, Option<f32>),
}

/// Notes only: what is heard.
fn notes(log: Vec<(u64, u16, Event)>) -> Vec<(u64, u16, Event)> {
    log.into_iter()
        .filter(|e| matches!(e.2, Event::NoteOn { .. } | Event::NoteOff { .. }))
        .collect()
}

fn compile(p: &Project, l: &[Launch], c: &[Cue], perform: &Perform) -> PlayView {
    let mut v = PlayView::compile_cued(p, l, c).unwrap();
    v.set_song(p, true).unwrap();
    perform.dress(&mut v);
    v
}

/// Play the grid in song mode with the recorder armed from the start,
/// as `mpfree-hello`'s TimeHost does: actions go in before the period
/// that starts at or after their frame, the recorder sees each period's
/// block and started cues, and the passes go into the song at the end.
/// Returns the notes heard and the project with its new song.
fn jam(
    p: &Project,
    launches: &[Launch],
    acts: &[(u64, Act)],
    commands: &[(u64, Command)],
    frames: u64,
    period: u32,
) -> (Vec<(u64, u16, Event)>, Project) {
    let (heard, project, _) = jam_with_room(p, launches, acts, commands, frames, period);
    (heard, project)
}

/// Where recording stopped for room, and the count it warned at.
type Room = (Option<Tick>, Option<usize>);

/// [`jam`], also saying whether the recorder stopped for room.
fn jam_with_room(
    p: &Project,
    launches: &[Launch],
    acts: &[(u64, Act)],
    commands: &[(u64, Command)],
    frames: u64,
    period: u32,
) -> (Vec<(u64, u16, Event)>, Project, Room) {
    let mut project = p.clone();
    let mut t = Transport::new(RATE, 120.0).unwrap();
    for &(f, c) in commands {
        assert!(t.command(f, c));
    }
    let mut perform = Perform::new(3);
    let (mut launches, mut cues): (Vec<Launch>, Vec<Cue>) = (launches.to_vec(), vec![]);
    let mut rec = SongRecorder::default();
    rec.arm(true, &project, &launches, 0.0, (0, 1));
    let mut view = compile(&project, &launches, &cues, &perform);
    let (mut player, mut block, mut log) = (Player::new(), Block::new(), Log::default());
    let mut next = 0;
    let (mut full, mut warning) = (None, None);
    while t.frame() < frames {
        let f = t.frame();
        let n = period.min((frames - f) as u32);
        let mut changed = false;
        while next < acts.len() && acts[next].0 <= f {
            match acts[next].1 {
                Act::Scene(row) => {
                    assert!(perform.launch_scene(
                        &mut project,
                        row,
                        When::Cued,
                        &mut launches,
                        &mut cues
                    ));
                }
                Act::Cue(track, slot) => {
                    cues.retain(|c| c.track != track);
                    cues.push(Cue { track, slot });
                }
                Act::Morph(pos, rows) => rec.morph(t.position(), pos, rows),
                Act::Mute(track, on) => {
                    project.tracks[track].mute = on;
                    rec.mute(track, on, t.position());
                }
                Act::Xy(addr, value) => {
                    let input = match value {
                        Some(value) => Input::Move {
                            addr,
                            value,
                            map: Map::linear(0.0, 1.0),
                        },
                        None => Input::End { addr },
                    };
                    rec.moved(t.position(), input);
                }
            }
            next += 1;
            changed = true;
        }
        if changed {
            view = compile(&project, &launches, &cues, &perform);
        }
        assert!(t.process(n, &mut block));
        player.process(&view, &block, &mut log);
        rec.song_blocks(project.arrangement.placements.len());
        rec.period(&block, player.started());
        full = full.or(rec.take_full());
        warning = warning.or(rec.take_warning());
        let mut changed = false;
        for st in player.started() {
            perform.started(&mut project, st);
            st.apply(&mut launches, &mut cues);
            changed = true;
        }
        let bar = t.meter().bar_ticks();
        changed |= perform
            .due(&mut project, &mut launches, &mut cues, t.position(), bar)
            .changed();
        if changed {
            view = compile(&project, &launches, &cues, &perform);
        }
    }
    assert_eq!(player.stats().dropped + player.stats().late_offs, 0);
    rec.arm(false, &project, &launches, 0.0, (0, 1));
    rec.finish(&mut project).unwrap();
    (notes(log.0), project, (full, warning))
}

/// Play the song alone (nothing launched, every track heard) with the
/// same transport commands.
fn replay(
    p: &Project,
    commands: &[(u64, Command)],
    frames: u64,
    period: u32,
) -> Vec<(u64, u16, Event)> {
    let mut p = p.clone();
    for t in &mut p.tracks {
        t.mute = false;
    }
    let mut t = Transport::new(RATE, 120.0).unwrap();
    for &(f, c) in commands {
        assert!(t.command(f, c));
    }
    let view = compile(&p, &[], &[], &Perform::new(3));
    let (mut player, mut block, mut log) = (Player::new(), Block::new(), Log::default());
    while t.frame() < frames {
        let n = period.min((frames - t.frame()) as u32);
        assert!(t.process(n, &mut block));
        player.process(&view, &block, &mut log);
    }
    log.0
}

/// A one-bar clip of `note` on each beat, each held 1.5 beats (so one is
/// always sounding where a switch cuts it).
fn held(note: u8) -> Clip {
    let mut c = Clip::new("held", BAR);
    c.notes = (0..4)
        .map(|i| Note::new(i * 960, 1440, note, 100))
        .collect();
    c
}

/// Tracks 0 to 2; row `r` of track `t` plays note `10 * (t + 1) + r`.
/// Track 2 has rows 1 and 4 only, so scenes 2 and 3 leave it be.
fn project() -> Project {
    let mut p = Project::new();
    for t in 0..3u8 {
        let mut tr = Track::new("t", t.into());
        for r in 0..4 {
            if t < 2 || r == 0 || r == 3 {
                tr.slots[r] = Some(held(10 * (t + 1) + r as u8));
            }
        }
        p.tracks.push(tr);
    }
    p.give_clip_ids();
    p
}

/// The row of block `b`'s clip on its own lane's track.
fn row(p: &Project, b: &Placement) -> Option<usize> {
    p.tracks[b.track].slots.row_of(b.clip)
}

fn launch(track: usize, slot: usize) -> Launch {
    Launch {
        track,
        slot,
        origin: 0,
        start: 0,
    }
}

#[test]
fn what_the_grid_played_the_song_plays_back_the_same() {
    let mut p = project();
    // Row 2 mutes track 1; row 3 hears it again.
    p.scene_mut(1).unwrap().mutes = Some(0b010);
    p.scene_mut(2).unwrap().mutes = Some(0);
    // Track 2 plays the song: row 4's clip for 7 bars.
    p.arrangement
        .place(Placement::of(&p.tracks, 2, 3, 0, 7 * BAR))
        .unwrap();
    // Each waits for the next bar line.
    let acts = [
        // Bar 3: row 2 (track 1 muted), and track 2's row 1 over the song.
        (at(BAR + 100), Act::Scene(1)),
        (at(BAR + 200), Act::Cue(2, Some(0))),
        // Bar 4: row 3 (track 1 heard; track 2 stops, back to the song).
        (at(2 * BAR + 300), Act::Scene(2)),
        // Bar 6: track 1 row 4. Bar 7: track 2 stops.
        (at(4 * BAR + 50), Act::Cue(0, Some(3))),
        (at(5 * BAR + 10), Act::Cue(1, None)),
    ];
    let commands = [(0, Command::Play)];
    let frames = at(7 * BAR);
    let launches = [launch(0, 0), launch(1, 0)];
    let (heard, song) = jam(&p, &launches, &acts, &commands, frames, 128);
    let placed: Vec<(usize, Tick, Tick, usize, Tick)> = song
        .arrangement
        .placements
        .iter()
        .map(|b| {
            let s = row(&song, b).expect("a block of a cell's clip");
            (b.track, b.start, b.end(), s, b.offset)
        })
        .collect();
    assert_eq!(
        placed,
        [
            (0, 0, 2 * BAR, 0, 0),
            (0, 2 * BAR, 3 * BAR, 1, 0),
            (0, 3 * BAR, 5 * BAR, 2, 0),
            (0, 5 * BAR, 7 * BAR, 3, 0),
            // Muted in bar 3: nothing there.
            (1, 0, 2 * BAR, 0, 0),
            (1, 3 * BAR, 6 * BAR, 2, 0),
            (2, 0, 2 * BAR, 3, 0),
            (2, 2 * BAR, 3 * BAR, 0, 0),
            (2, 3 * BAR, 7 * BAR, 3, 0),
        ]
    );
    for period in [1, 7, 128, 1000, 4096] {
        assert_eq!(
            jam(&p, &launches, &acts, &commands, frames, period),
            (heard.clone(), song.clone()),
            "jam at period {period}"
        );
        assert_eq!(
            notes(replay(&song, &commands, frames, period)),
            heard,
            "the song at period {period}"
        );
    }
}

#[test]
fn a_loop_pass_records_over_the_last() {
    let p = project();
    // Bars 1 and 2 loop. In the second pass, slot 2 takes over on bar 2.
    let commands = [
        (0, Command::SetLoop(Some((0, 2 * BAR)))),
        (0, Command::Play),
    ];
    let acts = [(at(2 * BAR + 100), Act::Cue(0, Some(1)))];
    let frames = at(4 * BAR - 10);
    let (_, song) = jam(&p, &[launch(0, 0)], &acts, &commands, frames, 256);
    let placed: Vec<(Tick, Tick, Option<usize>)> = song
        .arrangement
        .on_track(0)
        .iter()
        .map(|b| (b.start, b.end(), row(&song, b)))
        .collect();
    // The second pass stops 10 ticks short of its end: the first pass's
    // slot 0 is left there, from as far into its clip.
    assert_eq!(
        placed,
        [
            (0, BAR, Some(0)),
            (BAR, 2 * BAR - 10, Some(1)),
            (2 * BAR - 10, 2 * BAR, Some(0)),
        ]
    );
    // It plays back as the second pass did, up to where it stopped.
    let (heard, _) = jam(&p, &[launch(0, 0)], &acts, &commands, frames, 256);
    let second: Vec<(u64, u16, Event)> = heard
        .into_iter()
        // Not the note-offs of the jump back itself.
        .filter(|e| e.0 > at(2 * BAR) || matches!(e.2, Event::NoteOn { .. }))
        .filter(|e| e.0 >= at(2 * BAR))
        .map(|(f, t, e)| (f - at(2 * BAR), t, e))
        .collect();
    let back = notes(replay(&song, &[(0, Command::Play)], at(2 * BAR - 10), 256));
    assert_eq!(back, second);
}

#[test]
fn morph_moves_go_into_the_songs_lane_and_play_back() {
    let p = project();
    let acts = [
        (at(BAR), Act::Morph(0.25, (0, 1))),
        (at(BAR + 960), Act::Morph(0.5, (0, 1))),
        (at(2 * BAR), Act::Morph(1.0, (2, 3))),
    ];
    let commands = [(0, Command::Play)];
    let (_, song) = jam(&p, &[launch(0, 0)], &acts, &commands, at(3 * BAR), 64);
    let s = &song.arrangement;
    assert_eq!(
        s.morphs,
        [
            MorphRows { at: 0, a: 0, b: 1 },
            MorphRows {
                at: 2 * BAR,
                a: 2,
                b: 3
            }
        ]
    );
    let lane = s.lanes.lane(MORPH_LANE).unwrap();
    // At 0 where the pass started, held until the move to 0.25 at bar 2
    // (the period after the move, 64 frames late at most).
    let near = |tick: f64, want: f32| (lane.value_at(tick).unwrap() - want).abs() < 0.01;
    assert!((lane.value_at(0.0).unwrap()).abs() < 1e-3);
    assert!(near((BAR - 100) as f64, 0.0));
    // Held at 0.25 until the move to 0.5 a beat later, not ramped.
    assert!(near((BAR + 900) as f64, 0.25));
    assert!(near((BAR + 3) as f64, 0.25));
    assert!(near((2 * BAR + 3) as f64, 1.0));
    // The song plays it on its own track.
    let lanes: Vec<(u64, f32)> = replay(&song, &commands, at(3 * BAR), 64)
        .into_iter()
        .filter_map(|(f, t, e)| match e {
            Event::Lane { addr, value } if t == SONG_TRACK => {
                assert_eq!(addr, ParamAddr::morph_pos(0));
                Some((f, value))
            }
            _ => None,
        })
        .collect();
    assert!(lanes.iter().any(|l| (l.1 - 0.5).abs() < 0.01));
    assert!((lanes.last().unwrap().1 - 1.0).abs() < 0.01, "{lanes:?}");
    // A pass with no move leaves the lane as it was.
    let (_, again) = jam(&song, &[launch(0, 0)], &[], &commands, at(BAR), 64);
    assert_eq!(again.arrangement.lanes, song.arrangement.lanes);
}

/// The XY pad's moves of a setting go into the song's lane for it while
/// Rec song records, over what the lane had only where they moved, and
/// the song plays them back on its own track. A stop ends a gesture still
/// moving.
#[test]
fn the_xy_pads_moves_go_into_the_songs_lanes_and_play_back() {
    let p = project();
    let wet = ParamAddr::new(Owner::Strip(Strip::Master), 3);
    let drive = ParamAddr::new(Owner::Strip(Strip::Return(0)), 1);
    let acts = [
        (at(BAR), Act::Xy(wet, Some(0.2))),
        (at(BAR + 960), Act::Xy(wet, Some(0.6))),
        (at(BAR + 1920), Act::Xy(wet, None)),
        // Still moving when the transport stops.
        (at(2 * BAR), Act::Xy(drive, Some(0.9))),
    ];
    let commands = [(0, Command::Play), (at(3 * BAR), Command::Stop)];
    let (_, song) = jam(&p, &[launch(0, 0)], &acts, &commands, at(4 * BAR), 64);
    let lane = song.arrangement.lanes.lane(Target::Fixed(wet)).unwrap();
    let near = |tick: i64, want: f32| (lane.value_at(tick as f64).unwrap() - want).abs() < 0.01;
    assert_eq!(
        lane.value_at((BAR - 10) as f64),
        None,
        "free before the move"
    );
    assert!(near(BAR + 3, 0.2));
    assert!(near(BAR + 960 + 3, 0.6));
    assert_eq!(
        lane.value_at((BAR + 1920 + 10) as f64),
        None,
        "free after the lift"
    );
    let drive_lane = song.arrangement.lanes.lane(Target::Fixed(drive)).unwrap();
    assert!((drive_lane.value_at((2 * BAR + 100) as f64).unwrap() - 0.9).abs() < 0.01);
    assert_eq!(
        drive_lane.value_at((3 * BAR + 100) as f64),
        None,
        "the stop ended it"
    );
    // The song plays it on its own track.
    let heard: Vec<f32> = replay(&song, &commands, at(3 * BAR), 64)
        .into_iter()
        .filter_map(|(_, t, e)| match e {
            Event::Lane { addr, value } if t == SONG_TRACK && addr == wet => Some(value),
            _ => None,
        })
        .collect();
    assert!(heard.iter().any(|v| (v - 0.6).abs() < 0.01), "{heard:?}");
    // Another pass moving it in bar 3 leaves bar 2's move as it was.
    let again = [
        (at(2 * BAR + 960), Act::Xy(wet, Some(1.0))),
        (at(2 * BAR + 1920), Act::Xy(wet, None)),
    ];
    let (_, twice) = jam(&song, &[launch(0, 0)], &again, &commands, at(4 * BAR), 64);
    let lane = twice.arrangement.lanes.lane(Target::Fixed(wet)).unwrap();
    assert!((lane.value_at((BAR + 960 + 3) as f64).unwrap() - 0.6).abs() < 0.01);
    assert!((lane.value_at((2 * BAR + 960 + 3) as f64).unwrap() - 1.0).abs() < 0.01);
}

/// A gesture still moving when the transport jumps goes on from where it
/// lands, in the next pass.
#[test]
fn an_xy_gesture_goes_on_across_a_jump() {
    let p = project();
    let wet = ParamAddr::new(Owner::Strip(Strip::Master), 3);
    let acts = [(at(BAR), Act::Xy(wet, Some(0.4)))];
    let commands = [
        (0, Command::Play),
        (at(2 * BAR), Command::Locate(0)),
        (at(3 * BAR), Command::Stop),
    ];
    let (_, song) = jam(&p, &[launch(0, 0)], &acts, &commands, at(4 * BAR), 64);
    let lane = song.arrangement.lanes.lane(Target::Fixed(wet)).unwrap();
    // The second pass (bar 1 again) held it at 0.4 from where it landed.
    assert!((lane.value_at((BAR / 2) as f64).unwrap() - 0.4).abs() < 0.01);
    assert!((lane.value_at((BAR + 100) as f64).unwrap() - 0.4).abs() < 0.01);
}

#[test]
fn a_mute_leaves_a_gap_and_the_clip_comes_back_in_phase() {
    let mut p = project();
    // A two-bar clip of eight beats, notes 40 to 47.
    let mut c = Clip::new("long", 2 * BAR);
    c.notes = (0..8)
        .map(|i| Note::new(i * 960, 100, 40 + i as u8, 100))
        .collect();
    p.tracks[0].slots[0] = Some(c);
    p.give_clip_ids();
    // Muted from beat 2 of bar 1 to beat 3 of bar 2 (a 64-frame period
    // lands exactly on both).
    let acts = [
        (at(960), Act::Mute(0, true)),
        (at(BAR + 1920), Act::Mute(0, false)),
    ];
    let commands = [(0, Command::Play)];
    let frames = at(4 * BAR);
    let (heard, song) = jam(&p, &[launch(0, 0)], &acts, &commands, frames, 64);
    let placed: Vec<(Tick, Tick, Tick)> = song
        .arrangement
        .on_track(0)
        .iter()
        .map(|p| (p.start, p.end(), p.offset))
        .collect();
    assert_eq!(placed, [(0, 960, 0), (BAR + 1920, 4 * BAR, BAR + 1920)]);
    let ons = |v: &[(u64, u16, Event)]| -> Vec<(u64, u8)> {
        v.iter()
            .filter_map(|&(f, _, e)| match e {
                Event::NoteOn { note, .. } => Some((f, note)),
                _ => None,
            })
            .collect()
    };
    let want = ons(&heard);
    let heard_notes: Vec<u8> = want.iter().map(|o| o.1).collect();
    assert_eq!(heard_notes, [40, 46, 47, 40, 41, 42, 43, 44, 45, 46, 47]);
    assert_eq!(ons(&replay(&song, &commands, frames, 64)), want);
}

/// `p` with its song filled to `MAX_PLACEMENTS - room` blocks on `track`,
/// one bar each from bar 1001, far past anything a test plays.
fn nearly_full(mut p: Project, track: usize, room: usize) -> Project {
    for k in 0..MAX_PLACEMENTS - room {
        p.arrangement.placements.push(Placement::of(
            &p.tracks,
            track,
            0,
            (1000 + k as i64) * BAR,
            BAR,
        ));
    }
    p.check().unwrap();
    p
}

/// What `song` holds besides the filler: (track, start, end, slot).
fn recorded(song: &Project) -> Vec<(usize, Tick, Tick, Option<usize>)> {
    song.arrangement
        .placements
        .iter()
        .filter(|b| b.start < 1000 * BAR)
        .map(|b| (b.track, b.start, b.end(), row(song, b)))
        .collect()
}

/// Sixteen tracks switch together on one line with the song nearly full:
/// with room for the line, it all goes in; without, recording stops on
/// every track at that line, none of its cues half in, and what was
/// recorded goes in, opens and plays. The count warns at 90 %.
#[test]
fn sixteen_tracks_on_one_line_at_the_limit_go_in_together_or_stop_together() {
    let mut p = Project::new();
    for t in 0..16u8 {
        let mut tr = Track::new("t", t.into());
        tr.slots[0] = Some(held(20 + t));
        tr.slots[1] = Some(held(60 + t));
        p.tracks.push(tr);
    }
    p.give_clip_ids();
    let launches: Vec<Launch> = (0..16).map(|t| launch(t, 0)).collect();
    // Row 2 on bar 3, for every track.
    let acts = [(at(BAR + 100), Act::Scene(1))];
    let commands = [(0, Command::Play)];
    let frames = at(4 * BAR);
    // The pass takes 32 (a block and a stretch on each track); the line
    // 16 more (a block each).
    for (room, fits) in [(48, true), (47, false), (40, false)] {
        let full = nearly_full(p.clone(), 0, room);
        for period in [128, 1000] {
            let (heard, song, (stopped, warned)) =
                jam_with_room(&full, &launches, &acts, &commands, frames, period);
            assert!(warned.is_some(), "room {room}: warned at 90 %");
            assert!(song.arrangement.placements.len() <= MAX_PLACEMENTS);
            let mut want: Vec<_> = (0..16)
                .flat_map(|t| {
                    let first = (t, 0, 2 * BAR, Some(0));
                    let second = (t, 2 * BAR, 4 * BAR, Some(1));
                    if fits {
                        vec![first, second]
                    } else {
                        vec![first]
                    }
                })
                .collect();
            want.sort();
            let mut got = recorded(&song);
            got.sort();
            assert_eq!(got, want, "room {room}, period {period}");
            assert_eq!(stopped, (!fits).then_some(2 * BAR), "room {room}");
            // What's kept opens again and plays what was heard up to the
            // stop.
            let opened = mpfree_seq::save::from_text(&mpfree_seq::save::to_text(&song)).unwrap();
            assert_eq!(
                opened.arrangement.placements.len(),
                song.arrangement.placements.len()
            );
            let until = if fits { frames } else { at(2 * BAR) };
            let before = |log: Vec<(u64, u16, Event)>| -> Vec<_> {
                log.into_iter().filter(|e| e.0 < until).collect()
            };
            assert_eq!(
                before(notes(replay(&opened, &commands, frames, period))),
                before(heard),
                "room {room}, period {period}"
            );
        }
    }
}

/// A loop pass records over the last at the limit: each pass can cut a
/// block of the song in two, so a pass that wouldn't fit isn't begun:
/// recording stops at the jump and the first pass goes in.
#[test]
fn a_loop_pass_that_would_not_fit_stops_recording_at_the_jump() {
    let commands = [
        (0, Command::SetLoop(Some((0, 2 * BAR)))),
        (0, Command::Play),
    ];
    let launches = [launch(0, 0), launch(1, 0)];
    let frames = at(3 * BAR);
    // A pass on two tracks takes 4; the second pass 4 more.
    for (room, passes) in [(8, 2), (7, 1)] {
        let full = nearly_full(project(), 2, room);
        let (_, song, (stopped, _)) = jam_with_room(&full, &launches, &[], &commands, frames, 256);
        // The second pass (stopped a bar in) records over the first's
        // first bar, cutting its block in two.
        let want = if passes == 2 {
            vec![
                (0, 0, BAR, Some(0)),
                (0, BAR, 2 * BAR, Some(0)),
                (1, 0, BAR, Some(0)),
                (1, BAR, 2 * BAR, Some(0)),
            ]
        } else {
            vec![(0, 0, 2 * BAR, Some(0)), (1, 0, 2 * BAR, Some(0))]
        };
        assert_eq!(recorded(&song), want, "room {room}");
        assert_eq!(stopped, (passes == 1).then_some(2 * BAR), "room {room}");
        assert!(song.arrangement.placements.len() <= MAX_PLACEMENTS);
    }
}

/// The transport stopping on a scene's line, or just after it: every
/// track's block ends at the stop, the line's clips all in or all out.
#[test]
fn the_transport_stopping_during_a_scene_change_keeps_every_track_alike() {
    let p = project();
    let launches = [launch(0, 0), launch(1, 0)];
    let acts = [(at(BAR + 100), Act::Scene(1))];
    for (stop, switched) in [(at(2 * BAR), false), (at(2 * BAR) + 50, true)] {
        let commands = [(0, Command::Play), (stop, Command::Stop)];
        for period in [7, 128, 1000] {
            let (_, song) = jam(&p, &launches, &acts, &commands, at(3 * BAR), period);
            let got = recorded(&song);
            let end = (stop / TICK) as Tick;
            let want = if switched {
                vec![
                    (0, 0, 2 * BAR, Some(0)),
                    (0, 2 * BAR, end, Some(1)),
                    (1, 0, 2 * BAR, Some(0)),
                    (1, 2 * BAR, end, Some(1)),
                ]
            } else {
                vec![(0, 0, 2 * BAR, Some(0)), (1, 0, 2 * BAR, Some(0))]
            };
            assert_eq!(got, want, "stop at {stop}, period {period}");
            song.check().unwrap();
        }
    }
}
