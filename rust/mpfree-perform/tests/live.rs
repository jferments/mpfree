//! Live to song (T11) through the transport and the player: what the
//! launch grid plays (launches, scenes with their mutes, stops, a launch
//! over the song) is recorded into the arrangement, and the song then
//! plays back exactly the same notes on the same frames, at any period
//! size. A loop pass records over the last, and the morph's moves go into
//! the song's lane.

use mpfree_perform::{Perform, SongRecorder, When};
use mpfree_play::{Cue, Event, Launch, Out, PlayView, Player, Sink, SONG_TRACK};
use mpfree_seq::arrange::MORPH_LANE;
use mpfree_seq::mpfree_param::ParamAddr;
use mpfree_seq::{Clip, MorphRows, Note, Placement, Project, Source, Track};
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
            }
            next += 1;
            changed = true;
        }
        if changed {
            view = compile(&project, &launches, &cues, &perform);
        }
        assert!(t.process(n, &mut block));
        player.process(&view, &block, &mut log);
        rec.period(&block, player.started());
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
    (notes(log.0), project)
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
    p
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
        .place(Placement::linked(2, 3, 0, 7 * BAR))
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
        .map(|p| {
            let Source::Slot(s) = p.source else {
                panic!("linked")
            };
            (p.track, p.start, p.end(), s, p.offset)
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
        .map(|p| (p.start, p.end(), p.slot()))
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

#[test]
fn a_mute_leaves_a_gap_and_the_clip_comes_back_in_phase() {
    let mut p = project();
    // A two-bar clip of eight beats, notes 40 to 47.
    let mut c = Clip::new("long", 2 * BAR);
    c.notes = (0..8)
        .map(|i| Note::new(i * 960, 100, 40 + i as u8, 100))
        .collect();
    p.tracks[0].slots[0] = Some(c);
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
