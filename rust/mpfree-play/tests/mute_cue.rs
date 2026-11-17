//! The mute queue (sequencer S4): a mute or solo change waits on a track
//! as a [`MuteCue`] and lands on the first line of its launch quantise the
//! playhead plays, frame-exact. Going silent, the track's notes end on the
//! line; heard again, it plays from the line on. Every change queued
//! together lands on the same line, a cue made in a loop's last bar lands
//! on the loop start, and a scene's mute and a mute cue on one track take
//! the later.
//!
//! Every session runs at several period sizes, with the control thread
//! applying a landed queue at once, a few periods later, much later or
//! never. All of them must give the same events.

use mpfree_play::{Cue, Event, Launch, MuteCue, MuteLanded, Out, PlayView, Player, Sink, Started};
use mpfree_seq::{Clip, LaunchGrid, Note, Project, Track};
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

/// A queue the control thread makes at frame `.0`: each track and whether
/// it goes silent.
type Queue = (u64, Vec<(usize, bool)>);

#[derive(Clone)]
struct Session {
    project: Project,
    commands: Vec<(u64, Command)>,
    queues: Vec<Queue>,
    /// Clip cues (a scene's, with their mute) the control thread makes at
    /// their frame.
    cues: Vec<(u64, Cue, Option<bool>)>,
    line: LaunchGrid,
    frames: u64,
}

#[derive(Debug, PartialEq)]
struct Run {
    played: Vec<(u64, u16, Event)>,
    landed: Vec<MuteLanded>,
    started: Vec<Started>,
    /// The tracks' mutes at the end, once every queue was applied.
    mutes: Vec<bool>,
}

/// Run `s` in periods of `period` frames. The control thread applies a
/// landed queue (and a started cue) `apply` periods after the one it
/// landed in (None: never).
fn run(s: &Session, period: u32, apply: Option<usize>) -> Run {
    let mut t = Transport::new(RATE, 120.0).unwrap();
    for &(f, c) in &s.commands {
        assert!(t.command(f, c));
    }
    let mut project = s.project.clone();
    let mut launches: Vec<Launch> = (0..project.tracks.len())
        .map(|track| Launch {
            track,
            slot: 0,
            origin: 0,
            start: 0,
        })
        .collect();
    let mut cues: Vec<Cue> = vec![];
    let mut cue_mutes: Vec<(usize, bool)> = vec![];
    // The queue waiting: its id and changes.
    let mut queue: Option<(u32, Vec<(usize, bool)>)> = None;
    let mut ids = 0;
    let compile = |p: &Project,
                   l: &[Launch],
                   c: &[Cue],
                   cm: &[(usize, bool)],
                   q: &Option<(u32, Vec<(usize, bool)>)>| {
        let mut v = PlayView::compile_cued(p, l, c).unwrap();
        for &(track, m) in cm {
            v.set_cue_mute(track, Some(m));
        }
        if let Some((id, changes)) = q {
            for &(track, silent) in changes {
                v.set_mute_cue(
                    track,
                    MuteCue {
                        silent,
                        line: s.line,
                        id: *id,
                    },
                );
            }
        }
        v
    };
    let mut view = compile(&project, &launches, &cues, &cue_mutes, &queue);
    let (mut player, mut block, mut log) = (Player::new(), Block::new(), Log::default());
    let (mut landed, mut started) = (vec![], vec![]);
    let mut waiting: Vec<(usize, Option<u32>, Option<Started>)> = vec![];
    let (mut next_queue, mut next_cue) = (0, 0);
    while t.frame() < s.frames {
        let f = t.frame();
        let n = period.min((s.frames - f) as u32);
        let mut changed = false;
        for w in &mut waiting {
            if w.0 == 0 {
                if let Some(id) = w.1 {
                    if queue.as_ref().is_some_and(|q| q.0 == id) {
                        let (_, changes) = queue.take().unwrap();
                        for (track, silent) in changes {
                            project.tracks[track].mute = silent;
                        }
                    }
                }
                if let Some(st) = w.2 {
                    if let Some(m) = st.mute {
                        project.tracks[st.track].mute = m;
                    }
                    cue_mutes.retain(|c| c.0 != st.track);
                    st.apply(&mut launches, &mut cues);
                }
                changed = true;
            }
            w.0 = w.0.wrapping_sub(1);
        }
        waiting.retain(|w| w.0 != usize::MAX);
        while next_queue < s.queues.len() && s.queues[next_queue].0 <= f {
            ids += 1;
            queue = Some((ids, s.queues[next_queue].1.clone()));
            next_queue += 1;
            changed = true;
        }
        while next_cue < s.cues.len() && s.cues[next_cue].0 <= f {
            let (_, c, m) = s.cues[next_cue];
            cues.retain(|x| x.track != c.track);
            cues.push(c);
            if let Some(m) = m {
                cue_mutes.push((c.track, m));
            }
            next_cue += 1;
            changed = true;
        }
        if changed {
            view = compile(&project, &launches, &cues, &cue_mutes, &queue);
        }
        assert!(t.process(n, &mut block));
        player.process(&view, &block, &mut log);
        let mut ids_landed: Vec<u32> = vec![];
        for l in player.mutes_landed() {
            landed.push(*l);
            if !ids_landed.contains(&l.id) {
                ids_landed.push(l.id);
            }
        }
        for st in player.started() {
            started.push(*st);
            if let Some(d) = apply {
                waiting.push((d, None, Some(*st)));
            }
        }
        if let Some(d) = apply {
            for id in ids_landed {
                waiting.push((d, Some(id), None));
            }
        }
    }
    assert_eq!(player.stats().dropped + player.stats().late_offs, 0);
    Run {
        played: log.0,
        landed,
        started,
        mutes: project.tracks.iter().map(|t| t.mute).collect(),
    }
}

/// The run at every period size and every way of applying, which must
/// all agree (but for the mutes left at the end).
fn same_everywhere(s: &Session) -> Run {
    let first = run(s, 128, Some(0));
    for period in [1, 64, 1000, 4096] {
        for apply in [Some(0), Some(3), Some(40), None] {
            let r = run(s, period, apply);
            let what = format!("period {period}, applied {apply:?}");
            assert_eq!(r.played, first.played, "{what}");
            assert_eq!(r.landed, first.landed, "{what}");
            assert_eq!(r.started, first.started, "{what}");
            if apply == Some(0) {
                assert_eq!(r.mutes, first.mutes, "{what}");
            }
        }
    }
    first
}

/// Two tracks, each playing a bar of quarter notes from 0: track 0 note 1
/// (a whole beat long), track 1 note 2 (short).
fn session(frames: u64) -> Session {
    let mut p = Project::new();
    for (i, (n, len)) in [(1, 960), (2, 100)].into_iter().enumerate() {
        let mut t = Track::new(&format!("T{i}"), i as u16);
        let mut c = Clip::new("beat", BAR);
        c.notes = (0..4).map(|k| Note::new(k * 960, len, n, 100)).collect();
        t.slots[0] = Some(c);
        t.slots[1] = Some(Clip::new("empty", BAR));
        p.tracks.push(t);
    }
    Session {
        project: p,
        commands: vec![(0, Command::Play)],
        queues: vec![],
        cues: vec![],
        line: LaunchGrid::Bars(1),
        frames,
    }
}

/// (song tick, track, note, on) for each note-on and note-off, in a run
/// without jumps.
fn notes(r: &Run) -> Vec<(i64, u16, u8, bool)> {
    r.played
        .iter()
        .filter_map(|&(f, track, e)| match e {
            Event::NoteOn { note, .. } => Some(((f / TICK) as i64, track, note, true)),
            Event::NoteOff { note } => Some(((f / TICK) as i64, track, note, false)),
            _ => None,
        })
        .collect()
}

fn ons_of(r: &Run, track: u16) -> Vec<i64> {
    notes(r)
        .into_iter()
        .filter(|n| n.1 == track && n.3)
        .map(|n| n.0)
        .collect()
}

/// Muted mid-bar, track 0 plays on to the bar line, where its sounding
/// note ends; queued again, it comes back on the next bar line.
#[test]
fn a_mute_lands_on_the_bar_line_and_ends_the_notes_there() {
    let mut s = session(at(4 * BAR));
    s.queues = vec![(at(BAR + 1500), vec![(0, true)])];
    let r = same_everywhere(&s);
    assert_eq!(
        r.landed,
        vec![MuteLanded {
            track: 0,
            silent: true,
            at: 2 * BAR,
            id: 1
        }]
    );
    assert_eq!(
        ons_of(&r, 0),
        vec![0, 960, 1920, 2880, BAR, BAR + 960, BAR + 1920, BAR + 2880]
    );
    assert_eq!(ons_of(&r, 1).len(), 16, "track 1 plays on");
    assert_eq!(r.mutes, vec![true, false]);
    // Heard again from bar 4, queued in bar 3 once the first queue is
    // applied (the control thread applies a queue before it makes the
    // next).
    s.queues.push((at(2 * BAR + 1000), vec![(0, false)]));
    let r = run(&s, 128, Some(0));
    for period in [1, 64, 1000] {
        for apply in [Some(0), Some(3)] {
            let o = run(&s, period, apply);
            assert_eq!(o.played, r.played, "period {period}, {apply:?}");
            assert_eq!(o.landed, r.landed, "period {period}, {apply:?}");
        }
    }
    assert_eq!(r.landed[1].at, 3 * BAR);
    assert_eq!(
        &ons_of(&r, 0)[8..],
        &[3 * BAR, 3 * BAR + 960, 3 * BAR + 1920, 3 * BAR + 2880]
    );
    assert_eq!(r.mutes, vec![false, false]);
}

/// A whole-beat note sounding over the line ends on it, before anything
/// else there; the other track's note on the line still plays.
#[test]
fn a_note_held_over_the_line_ends_on_it() {
    let mut s = session(at(3 * BAR));
    s.line = LaunchGrid::Ticks(960);
    s.queues = vec![(at(BAR + 1500), vec![(0, true)])];
    let r = same_everywhere(&s);
    assert_eq!(r.landed[0].at, BAR + 1920);
    let n = notes(&r);
    // Track 0's note from BAR + 960 lasts to BAR + 1920: it ends there and
    // no new one starts.
    assert!(n.contains(&(BAR + 1920, 0, 1, false)), "{n:?}");
    assert!(!n.iter().any(|x| x.1 == 0 && x.3 && x.0 >= BAR + 1920));
    assert!(n.contains(&(BAR + 1920, 1, 2, true)));
}

/// Changes queued together land together, on the line they had.
#[test]
fn a_queue_lands_together_on_its_line() {
    let mut s = session(at(4 * BAR));
    s.line = LaunchGrid::Bars(2);
    s.queues = vec![(at(BAR / 2), vec![(0, true), (1, true)])];
    let r = same_everywhere(&s);
    assert_eq!(
        r.landed.iter().map(|l| (l.track, l.at)).collect::<Vec<_>>(),
        vec![(0, 2 * BAR), (1, 2 * BAR)]
    );
    assert!(notes(&r).iter().all(|n| n.0 < 2 * BAR || !n.3));
    assert_eq!(r.mutes, vec![true, true]);
}

/// At once: the first tick of the period the cue is compiled in.
#[test]
fn a_mute_at_once_lands_on_the_next_tick() {
    let mut s = session(at(2 * BAR));
    s.line = LaunchGrid::Now;
    s.queues = vec![(at(1500), vec![(1, true)])];
    for period in [64, 128, 1000] {
        let r = run(&s, period, Some(0));
        let at = r.landed[0].at;
        assert!(
            (1500..=1500 + i64::from(period) / TICK as i64 + 1).contains(&at),
            "period {period}: {at}"
        );
        assert!(ons_of(&r, 1).iter().all(|&t| t < at), "period {period}");
    }
}

/// In a loop, a cue made in its last bar lands on the loop start, right
/// after the jump back.
#[test]
fn in_a_loop_a_mute_lands_on_the_loop_start() {
    let mut s = session(at(5 * BAR));
    s.commands = vec![
        (0, Command::SetLoop(Some((0, 2 * BAR)))),
        (0, Command::Play),
    ];
    s.queues = vec![(at(BAR + 2000), vec![(0, true)])];
    let first = run(&s, 128, Some(0));
    for period in [1, 64, 1000, 4096] {
        for apply in [Some(0), Some(3), Some(40), None] {
            let r = run(&s, period, apply);
            assert_eq!(r.played, first.played, "period {period}, {apply:?}");
            assert_eq!(r.landed, first.landed, "period {period}, {apply:?}");
        }
    }
    assert_eq!(first.landed[0].at, 0);
    // After the jump back track 0 is silent, and track 1 plays on.
    let after_jump = |track: u16| {
        first
            .played
            .iter()
            .filter(|p| p.0 >= at(2 * BAR) && p.1 == track)
            .filter(|p| matches!(p.2, Event::NoteOn { .. }))
            .count()
    };
    assert_eq!(after_jump(0), 0);
    assert_eq!(after_jump(1), 12);
}

/// A scene's mute on a cue and a mute cue on the same track: the one that
/// landed later says what the track is.
#[test]
fn a_scene_mute_and_a_mute_cue_take_the_later() {
    let mut s = session(at(4 * BAR));
    // The mute cue lands on bar 2; the scene's cue (back to slot 0, heard)
    // on bar 3.
    s.queues = vec![(at(BAR / 2), vec![(0, true)])];
    s.cues = vec![(
        at(BAR + 100),
        Cue {
            track: 0,
            slot: Some(0),
        },
        Some(false),
    )];
    let r = same_everywhere(&s);
    assert_eq!(r.landed[0].at, BAR);
    assert_eq!(r.started[0].at, 2 * BAR);
    let on = ons_of(&r, 0);
    assert!(on.iter().all(|&t| !(BAR..2 * BAR).contains(&t)), "{on:?}");
    assert!(on.contains(&(2 * BAR)) && on.contains(&(3 * BAR)));
}
