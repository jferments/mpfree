//! Scenes and follow actions played through the transport and the player:
//! a scene's clips start together on the bar line and its mutes switch
//! exactly there; follow actions chain clips and scenes on their bar lines;
//! all of it the same at any period size.

use mpfree_perform::{Perform, When};
use mpfree_play::{Cue, Event, Launch, Out, PlayView, Player, Sink, Started};
use mpfree_seq::{Clip, Follow, FollowAction, Note, Project, Track};
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
    StopAll,
}

#[derive(Debug, PartialEq)]
struct Run {
    played: Vec<(u64, u16, Event)>,
    started: Vec<Started>,
    mutes: Vec<bool>,
}

/// The host's part, as `mpfree-app`'s TimeHost does it: actions go in
/// before the period that starts at or after their frame, started cues are
/// applied at once, and follow actions are checked after every period.
fn run(
    project: &Project,
    launches: &[Launch],
    acts: &[(u64, Act)],
    frames: u64,
    period: u32,
) -> Run {
    let mut project = project.clone();
    let mut t = Transport::new(RATE, 120.0).unwrap();
    assert!(t.command(0, Command::Play));
    let mut perform = Perform::new(11);
    let (mut launches, mut cues): (Vec<Launch>, Vec<Cue>) = (launches.to_vec(), vec![]);
    let compile = |p: &Project, l: &[Launch], c: &[Cue], perform: &Perform| {
        let mut v = PlayView::compile_cued(p, l, c).unwrap();
        perform.dress(&mut v);
        v
    };
    let mut view = compile(&project, &launches, &cues, &perform);
    let (mut player, mut block, mut log) = (Player::new(), Block::new(), Log::default());
    let mut started = vec![];
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
                Act::StopAll => perform.stop_all(When::Cued, &mut launches, &mut cues),
            }
            next += 1;
            changed = true;
        }
        if changed {
            view = compile(&project, &launches, &cues, &perform);
        }
        assert!(t.process(n, &mut block));
        player.process(&view, &block, &mut log);
        let mut changed = false;
        for st in player.started() {
            perform.started(&mut project, st);
            st.apply(&mut launches, &mut cues);
            started.push(*st);
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
    Run {
        played: log.0,
        started,
        mutes: project.tracks.iter().map(|t| t.mute).collect(),
    }
}

fn same_everywhere(p: &Project, launches: &[Launch], acts: &[(u64, Act)], frames: u64) -> Run {
    let first = run(p, launches, acts, frames, 128);
    for period in [1, 64, 1000, 4096] {
        assert_eq!(
            run(p, launches, acts, frames, period),
            first,
            "period {period}"
        );
    }
    first
}

/// A one-bar clip of four quarter notes of `note`.
fn beats(note: u8) -> Clip {
    let mut c = Clip::new("beats", BAR);
    c.notes = (0..4).map(|i| Note::new(i * 960, 100, note, 100)).collect();
    c
}

/// Tracks 0 and 1, row `r` of track `t` playing note `10 * (t + 1) + r`.
fn project(rows: usize) -> Project {
    let mut p = Project::new();
    for t in 0..2u8 {
        let mut tr = Track::new("t", t.into());
        for r in 0..rows {
            tr.slots[r] = Some(beats(10 * (t + 1) + r as u8));
        }
        p.tracks.push(tr);
    }
    p
}

fn row0() -> Vec<Launch> {
    (0..2)
        .map(|track| Launch {
            track,
            slot: 0,
            origin: 0,
            start: 0,
        })
        .collect()
}

/// Note-ons as (bar, track, note) on bar downbeats only.
fn downbeats(r: &Run) -> Vec<(i64, u16, u8)> {
    r.played
        .iter()
        .filter_map(|&(f, track, e)| match e {
            Event::NoteOn { note, .. } if (f / TICK) as i64 % BAR == 0 => {
                Some(((f / TICK) as i64 / BAR, track, note))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_scene_starts_together_on_the_bar_line_and_switches_its_mutes_there() {
    let mut p = project(2);
    p.scene_mut(1).unwrap().mutes = Some(0b10);
    let r = same_everywhere(&p, &row0(), &[(at(1500), Act::Scene(1))], at(3 * BAR));
    assert_eq!(
        downbeats(&r),
        vec![(0, 0, 10), (0, 1, 20), (1, 0, 11), (2, 0, 11)],
        "track 1 is muted from bar 1 exactly"
    );
    // Track 1 played its last beat of bar 0 and nothing after.
    let last1 = r
        .played
        .iter()
        .filter(|x| x.1 == 1)
        .filter_map(|x| match x.2 {
            Event::NoteOn { .. } => Some(x.0 / TICK),
            _ => None,
        });
    assert_eq!(last1.max(), Some(3 * 960));
    assert_eq!(r.mutes, vec![false, true]);
    assert!(r.started.iter().all(|s| s.at == BAR));
    assert_eq!(r.started[1].mute, Some(true));
}

#[test]
fn clip_follow_actions_chain_on_their_bar_lines() {
    let mut p = project(3);
    let f = |bars, action| Some(Follow::after(bars, action));
    p.tracks[0].slots[0].as_mut().unwrap().follow = f(1, FollowAction::Next);
    p.tracks[0].slots[1].as_mut().unwrap().follow = f(2, FollowAction::Next);
    p.tracks[0].slots[2].as_mut().unwrap().follow = f(1, FollowAction::First);
    let launches = &row0()[..1];
    let r = same_everywhere(&p, launches, &[], at(6 * BAR));
    let starts: Vec<_> = r.started.iter().map(|s| (s.at / BAR, s.slot)).collect();
    assert_eq!(
        starts,
        vec![(1, Some(1)), (3, Some(2)), (4, Some(0)), (5, Some(1))]
    );
    assert_eq!(
        downbeats(&r).iter().map(|d| d.2).collect::<Vec<_>>(),
        vec![10, 11, 11, 12, 10, 11]
    );
}

#[test]
fn scene_follow_actions_chain_and_carry_their_mutes() {
    let mut p = project(2);
    let s0 = p.scene_mut(0).unwrap();
    s0.mutes = Some(0);
    s0.follow = Some(Follow::after(2, FollowAction::Next));
    let s1 = p.scene_mut(1).unwrap();
    s1.mutes = Some(0b01);
    s1.follow = Some(Follow::after(1, FollowAction::Previous));
    let r = same_everywhere(&p, &row0(), &[(at(100), Act::Scene(0))], at(7 * BAR));
    // Scene 0 from bar 1 (two bars), scene 1 from bar 3 (one bar, track 0
    // muted), scene 0 from bar 4, scene 1 from bar 6.
    assert_eq!(
        downbeats(&r),
        vec![
            (0, 0, 10),
            (0, 1, 20),
            (1, 0, 10),
            (1, 1, 20),
            (2, 0, 10),
            (2, 1, 20),
            (3, 1, 21),
            (4, 0, 10),
            (4, 1, 20),
            (5, 0, 10),
            (5, 1, 20),
            (6, 1, 21),
        ]
    );
}

#[test]
fn stop_all_stops_every_track_on_the_bar_line() {
    let p = project(1);
    let r = same_everywhere(&p, &row0(), &[(at(BAR + 7), Act::StopAll)], at(4 * BAR));
    assert_eq!(
        downbeats(&r),
        vec![(0, 0, 10), (0, 1, 20), (1, 0, 10), (1, 1, 20)]
    );
    assert!(r
        .started
        .iter()
        .all(|s| s.at == 2 * BAR && s.slot.is_none()));
}
