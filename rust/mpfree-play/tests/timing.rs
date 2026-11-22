//! The timing contract (Arrange refactor 2.5, design D4): each view says
//! the frame the music had reached when each track's cue (and the mute
//! queue) was decided, and each track's cue revision (`Includes::decided`,
//! `Includes::mutes_decided`, `Includes::cues`). A view
//! held back, as one compiled on another thread can be, still lands every
//! cue in phase: a cue whose line played before its view came starts at
//! once, as if it had started on its line, and is counted. A newer cue
//! takes a waiting one's place, and an older one than a cue carried out
//! is dropped and counted.
//!
//! The control thread here decides at the start of a period, with all the
//! periods before it heard, and its views (and those it compiles after a
//! cue starts) reach the player `hold` periods later.

use mpfree_play::{
    Cue, Decisions, Event, Includes, Launch, MuteCue, Out, PlayView, Player, Sink, Started,
};
use mpfree_seq::{Clip, LaunchGrid, Note, Project, Track, MAX_TRACKS};
use mpfree_transport::{Block, Command, Transport};

const BAR: i64 = 3840;
/// 120 BPM at 48 kHz: 25 frames a tick.
const RATE: u32 = 48_000;
const TICK: u64 = 25;

fn at(tick: i64) -> u64 {
    tick as u64 * TICK
}

/// The first tick played at or after `frame`.
fn tick_of(frame: u64) -> i64 {
    frame.div_ceil(TICK) as i64
}

#[derive(Default)]
struct Log(Vec<(u64, u16, Event)>);
impl Sink for Log {
    fn send(&mut self, o: Out) {
        self.0.push((o.frame, o.track, o.event));
    }
}

/// What the control thread does, at the start of the first period at or
/// after a frame.
#[derive(Clone, Copy, Debug)]
enum Do {
    Cue(Cue),
    /// A mute queue on a track: silent from its line on.
    Mute(usize),
}

#[derive(Clone)]
struct Session {
    project: Project,
    launches: Vec<Launch>,
    commands: Vec<(u64, Command)>,
    decisions: Vec<(u64, Do)>,
    frames: u64,
}

#[derive(Debug)]
struct Run {
    played: Vec<(u64, u16, Event)>,
    started: Vec<Started>,
    late: u64,
    stale: u64,
    lost: u64,
}

impl Run {
    fn ons_from(&self, frame: u64) -> Vec<(u64, u16, u8)> {
        self.played
            .iter()
            .filter(|&&(f, _, _)| f >= frame)
            .filter_map(|&(f, t, e)| match e {
                Event::NoteOn { note, .. } => Some((f, t, note)),
                _ => None,
            })
            .collect()
    }
}

/// The control thread's side: the project's launches, cues and mute
/// queue, and its side of the contract ([`Decisions`]), as the app keeps
/// them.
struct Control {
    project: Project,
    launches: Vec<Launch>,
    cues: Vec<Cue>,
    mutes: Vec<(usize, u32)>,
    decisions: Decisions,
    includes: Includes,
}

impl Control {
    /// The view, compiled having heard the music up to frame `heard`.
    fn view(&mut self, heard: u64) -> PlayView {
        let mut v = PlayView::compile_cued(&self.project, &self.launches, &self.cues).unwrap();
        for &(t, id) in &self.mutes {
            v.set_mute_cue(
                t,
                MuteCue {
                    silent: true,
                    line: LaunchGrid::Bars(1),
                    id,
                },
            );
        }
        self.decisions.stamp(&mut v, heard, &mut self.includes);
        v
    }

    fn cue(&mut self, c: Cue) {
        self.cues.retain(|x| x.track != c.track);
        self.cues.push(c);
    }
}

/// Run `s` in periods of `period` frames, the control thread's views
/// reaching the player `hold` periods after it compiled them.
fn run(s: &Session, period: u32, hold: u64) -> Run {
    run_with(s, period, |k| k + hold)
}

/// Run `s` in periods of `period` frames, a view compiled after period `k`
/// (or before the first, 0) reaching the player at the start of period
/// `arrive(k)`.
fn run_with(s: &Session, period: u32, arrive: impl Fn(u64) -> u64) -> Run {
    let mut t = Transport::new(RATE, 120.0).unwrap();
    for &(f, c) in &s.commands {
        assert!(t.command(f, c));
    }
    let mut control = Control {
        project: s.project.clone(),
        launches: s.launches.clone(),
        cues: Vec::new(),
        mutes: Vec::new(),
        decisions: Decisions::default(),
        includes: Includes::default(),
    };
    let mut view = control.view(0);
    let mut on_way: Vec<(u64, PlayView)> = Vec::new();
    let (mut player, mut block, mut log) = (Player::new(), Block::new(), Log::default());
    let mut started = vec![];
    let mut next = 0;
    let mut k = 0u64;
    while t.frame() < s.frames {
        let f = t.frame();
        let n = period.min((s.frames - f) as u32);
        let mut decided = false;
        while next < s.decisions.len() && s.decisions[next].0 <= f {
            match s.decisions[next].1 {
                Do::Cue(c) => control.cue(c),
                // One queue: its cues land together.
                Do::Mute(track) => control.mutes.push((track, 1)),
            }
            next += 1;
            decided = true;
        }
        if decided {
            on_way.push((arrive(k), control.view(f)));
        }
        // The newest view due, as a host swaps in what reached it.
        while on_way.first().is_some_and(|w| w.0 <= k) {
            view = on_way.remove(0).1;
        }
        assert!(t.process(n, &mut block));
        player.process(&view, &block, &mut log);
        let mut changed = false;
        for st in player.started() {
            started.push(*st);
            st.apply(&mut control.launches, &mut control.cues);
            control.decisions.started(st);
            control.includes.switches[st.track] += 1;
            changed = true;
        }
        for l in player.mutes_landed() {
            control.mutes.retain(|m| m.1 != l.id);
            control.project.tracks[l.track].mute = l.silent;
            changed = true;
        }
        k += 1;
        if changed {
            on_way.push((arrive(k), control.view(t.frame())));
        }
    }
    let stats = player.stats();
    assert_eq!(stats.dropped + stats.late_offs, 0);
    Run {
        played: log.0,
        started,
        late: stats.late_cues,
        stale: stats.stale_cues,
        lost: stats.lost_switches,
    }
}

fn note(tick: i64, len: i64, n: u8) -> Note {
    Note::new(tick, len, n, 100)
}

/// Track 0: slot 0 holds A, a bar of beats (note 1), playing from 0; slot
/// 1 holds B, a bar with note 20 on each beat and note 21 on each
/// off-beat; slot 2 holds C, a bar with note 30 on each beat. Track 1:
/// D, a bar of beats (note 40), playing from 0.
fn session(frames: u64) -> Session {
    let mut p = Project::new();
    let mut t = Track::new("drums", 0);
    let mut a = Clip::new("A", BAR);
    a.notes = (0..4).map(|i| note(i * 960, 100, 1)).collect();
    let mut b = Clip::new("B", BAR);
    b.notes = (0..4)
        .flat_map(|i| [note(i * 960, 100, 20), note(i * 960 + 480, 100, 21)])
        .collect();
    let mut c = Clip::new("C", BAR);
    c.notes = (0..4).map(|i| note(i * 960, 100, 30)).collect();
    t.slots[0] = Some(a);
    t.slots[1] = Some(b);
    t.slots[2] = Some(c);
    p.tracks.push(t);
    let mut t1 = Track::new("bass", 1);
    let mut d = Clip::new("D", BAR);
    d.notes = (0..4).map(|i| note(i * 960, 100, 40)).collect();
    t1.slots[0] = Some(d);
    p.tracks.push(t1);
    Session {
        project: p,
        launches: vec![
            Launch {
                track: 0,
                slot: 0,
                origin: 0,
                start: 0,
            },
            Launch {
                track: 1,
                slot: 0,
                origin: 0,
                start: 0,
            },
        ],
        commands: vec![(0, Command::Play)],
        decisions: vec![],
        frames,
    }
}

fn cue(track: usize, slot: Option<usize>) -> Do {
    Do::Cue(Cue { track, slot })
}

#[test]
fn a_view_held_back_one_to_four_periods_lands_its_cue_in_phase() {
    for period in [64u32, 128, 1000] {
        let p = u64::from(period);
        // B is cued two periods before bar 2's line.
        let decide = (at(BAR) / p - 2) * p;
        let mut s = session(at(4 * BAR));
        s.decisions = vec![(decide, cue(0, Some(1)))];
        let step = run(&s, period, 0);
        assert_eq!(step.started.len(), 1);
        assert_eq!(step.started[0].at, BAR, "in step it lands on the line");
        assert_eq!(step.late, 0);
        for hold in 1..=4u64 {
            let r = run(&s, period, hold);
            let what = format!("period {period}, held back {hold}");
            let arrived = decide + hold * p;
            assert_eq!(r.started.len(), 1, "{what}");
            let st = r.started[0];
            if arrived <= at(BAR) {
                // In time for its line: as in step.
                assert_eq!(st, step.started[0], "{what}");
                assert_eq!(r.played, step.played, "{what}");
                assert_eq!(r.late, 0, "{what}");
            } else {
                // Late: at once, in phase with the line it missed, and
                // counted. From then on it plays what the step run plays.
                assert_eq!(st.at, tick_of(arrived), "{what}");
                assert_eq!(
                    (st.origin, st.from),
                    (step.started[0].origin, BAR),
                    "{what}"
                );
                assert_eq!(r.ons_from(arrived), step.ons_from(arrived), "{what}");
                assert_eq!(r.late, 1, "{what}");
                // A plays up to it, B nothing before it.
                assert!(r
                    .ons_from(at(BAR))
                    .iter()
                    .all(|&(f, _, n)| (f < arrived) == (n == 1) || n == 40));
            }
            assert_eq!((r.stale, r.lost), (0, 0), "{what}");
            // Track 1 plays on, untouched.
            let track_1 =
                |r: &Run| -> Vec<_> { r.played.iter().filter(|e| e.1 == 1).copied().collect() };
            assert_eq!(track_1(&r), track_1(&step), "{what}");
        }
    }
}

#[test]
fn a_stop_held_back_stops_at_once_and_a_view_on_time_waits_as_before() {
    let mut s = session(at(3 * BAR));
    s.decisions = vec![(at(BAR) - 2 * 128, cue(0, None))];
    let step = run(&s, 128, 0);
    let late = run(&s, 128, 4);
    assert_eq!(step.started[0].at, BAR);
    let arrived = at(BAR) + 2 * 128;
    assert_eq!(late.started[0].at, tick_of(arrived));
    assert_eq!(late.late, 1);
    // Nothing on track 0 after either stop.
    assert!(late.ons_from(arrived).iter().all(|e| e.1 == 1));
    assert!(step.ons_from(at(BAR)).iter().all(|e| e.1 == 1));
}

#[test]
fn a_newer_launch_replaces_a_waiting_one_on_the_same_track() {
    for hold in 0..=4u64 {
        // B, then C a period later, both before bar 2: C starts there and
        // B never does.
        let mut s = session(at(3 * BAR));
        s.decisions = vec![
            (at(BAR) - 8 * 128, cue(0, Some(1))),
            (at(BAR) - 7 * 128, cue(0, Some(2))),
        ];
        let r = run(&s, 128, hold);
        let what = format!("held back {hold}");
        assert_eq!(r.started.len(), 1, "{what}: {:?}", r.started);
        assert_eq!(
            (r.started[0].at, r.started[0].slot),
            (BAR, Some(2)),
            "{what}"
        );
        assert!(
            r.ons_from(0).iter().all(|e| e.2 != 20 && e.2 != 21),
            "{what}"
        );
        assert_eq!((r.late, r.stale, r.lost), (0, 0, 0), "{what}");
    }
    // C decided after B started, but before the player's view showed it:
    // C waits for its own line (bar 3), and B plays until then.
    let mut s = session(at(4 * BAR));
    s.decisions = vec![
        (at(BAR) - 8 * 128, cue(0, Some(1))),
        (at(BAR) + 128, cue(0, Some(2))),
    ];
    for hold in 0..=1u64 {
        let r = run(&s, 128, hold);
        let what = format!("held back {hold}");
        let starts: Vec<_> = r.started.iter().map(|s| (s.at, s.slot)).collect();
        assert_eq!(starts, vec![(BAR, Some(1)), (2 * BAR, Some(2))], "{what}");
        assert_eq!((r.late, r.stale), (0, 0), "{what}");
    }
}

#[test]
fn a_newer_cue_over_a_late_switch_takes_its_place_in_phase() {
    // B and then C are cued before bar 2, and each view comes after the
    // line: B starts late, then C, newer, takes its place, in phase with
    // the same line.
    let mut s = session(at(3 * BAR));
    s.decisions = vec![
        (at(BAR) - 2 * 128, cue(0, Some(1))),
        (at(BAR) - 128, cue(0, Some(2))),
    ];
    let r = run(&s, 128, 4);
    let starts: Vec<_> = r.started.iter().map(|s| (s.at, s.slot, s.from)).collect();
    let b = tick_of(at(BAR) + 2 * 128);
    let c = tick_of(at(BAR) + 3 * 128);
    assert_eq!(starts, vec![(b, Some(1), BAR), (c, Some(2), BAR)]);
    assert_eq!((r.late, r.stale, r.lost), (2, 0, 0));
    let mut only_c = session(at(3 * BAR));
    only_c.decisions = vec![(at(BAR) - 128, cue(0, Some(2)))];
    let step = run(&only_c, 128, 0);
    assert_eq!(
        r.ons_from(at(BAR) + 3 * 128),
        step.ons_from(at(BAR) + 3 * 128)
    );
}

#[test]
fn a_cue_older_than_one_carried_out_is_dropped_and_counted() {
    let s = session(at(3 * BAR));
    let mut t = Transport::new(RATE, 120.0).unwrap();
    assert!(t.command(0, Command::Play));
    let view = |cues: &[Cue], rev: u32| {
        let mut v = PlayView::compile_cued(&s.project, &s.launches, cues).unwrap();
        let mut inc = Includes {
            decided: [1; MAX_TRACKS],
            ..Includes::default()
        };
        inc.cues[0] = rev;
        v.set_includes(inc);
        v
    };
    let b = Cue {
        track: 0,
        slot: Some(1),
    };
    let c = Cue {
        track: 0,
        slot: Some(2),
    };
    let (mut player, mut block, mut log) = (Player::new(), Block::new(), Log::default());
    let mut v = view(&[], 0);
    let mut started = vec![];
    while t.frame() < at(3 * BAR) {
        if t.frame() == 1000 {
            v = view(&[b], 2);
        }
        if t.frame() == at(BAR) + 4000 {
            // A view from before B's (C, revision 1) after B started.
            v = view(&[c], 1);
        }
        assert!(t.process(1000, &mut block));
        player.process(&v, &block, &mut log);
        started.extend(player.started().iter().map(|s| (s.at, s.slot)));
    }
    assert_eq!(started, vec![(BAR, Some(1))]);
    assert_eq!(player.stats().stale_cues, 1, "counted once");
}

#[test]
fn a_mute_cue_held_back_lands_at_once_with_the_cues() {
    // Track 1 goes silent on bar 2, and track 0 switches to B there: held
    // back past the line, both land on the same tick at once.
    let mut s = session(at(3 * BAR));
    s.decisions = vec![
        (at(BAR) - 2 * 128, cue(0, Some(1))),
        (at(BAR) - 2 * 128, Do::Mute(1)),
    ];
    let step = run(&s, 128, 0);
    assert!(step.ons_from(at(BAR)).iter().all(|e| e.1 == 0));
    let r = run(&s, 128, 3);
    let arrived = at(BAR) + 128;
    assert_eq!(r.started[0].at, tick_of(arrived));
    assert!(
        r.ons_from(arrived).iter().all(|e| e.1 == 0),
        "track 1 silent"
    );
    assert_eq!(r.ons_from(arrived), step.ons_from(arrived));
    assert_eq!(r.late, 2, "the cue and the mute cue");
}

#[test]
fn in_a_loop_a_late_cue_is_in_phase_with_the_loop_start() {
    // A two-bar loop. B is cued just before the loop jumps back; its line
    // is the loop's start, and its view comes after the jump.
    let mut s = session(at(6 * BAR));
    s.commands
        .insert(0, (0, Command::SetLoop(Some((0, 2 * BAR)))));
    s.decisions = vec![(at(2 * BAR) - 2 * 128, cue(0, Some(1)))];
    let step = run(&s, 128, 0);
    assert_eq!(step.started[0].at, 0, "the loop start");
    let r = run(&s, 128, 4);
    let arrived = at(2 * BAR) + 2 * 128;
    assert_eq!(r.late, 1);
    assert_eq!(r.started[0].at, tick_of(2 * 128));
    assert_eq!(r.started[0].origin, step.started[0].origin);
    assert_eq!(r.ons_from(arrived), step.ons_from(arrived));
}

#[test]
fn a_cue_made_while_stopped_lands_on_the_first_line_played() {
    // Decided before the music starts (200 ticks before bar 2, at frame
    // 40,000), its view comes after the start: before the bar line it
    // lands there, as in step; after it, at once, in phase with it.
    let (from, play) = (BAR - 200, 40_000u64);
    for (period, late) in [(1000u32, false), (4096, true)] {
        let mut s = session(at(4 * BAR));
        s.commands = vec![(0, Command::Locate(from)), (play, Command::Play)];
        s.decisions = vec![(36_000, cue(0, Some(1)))];
        let step = run(&s, period, 0);
        let r = run(&s, period, 4);
        assert_eq!(step.started[0].at, BAR);
        assert_eq!(step.late, 0);
        let p = u64::from(period);
        let arrived = 36_000u64.div_ceil(p) * p + 4 * p;
        let line = play + at(BAR - from);
        assert_eq!(arrived > line, late);
        if late {
            assert_eq!(r.started[0].at, from + tick_of(arrived - play));
            assert_eq!(r.started[0].origin, step.started[0].origin);
            assert_eq!(r.ons_from(arrived), step.ons_from(arrived));
            assert_eq!(r.late, 1);
        } else {
            assert_eq!(r.started, step.started);
            assert_eq!(r.played, step.played);
            assert_eq!(r.late, 0);
        }
    }
}

#[test]
fn a_newer_cue_before_the_view_shows_a_switch_is_counted_until_it_does() {
    // B starts on bar 2. C, which lands on 2-bar lines (bar 3), is decided
    // as that period starts, before the control thread hears of B, so its
    // view does not show B's switch: B's clip is not in it, and the track
    // is silent (counted) until the next view shows the switch. C still
    // lands on bar 3.
    let mut s = session(at(4 * BAR));
    if let Some(c) = s.project.tracks[0].slots[2].as_mut() {
        c.launch = Some(LaunchGrid::Bars(2));
    }
    s.decisions = vec![
        (at(BAR) - 8 * 128, cue(0, Some(1))),
        (at(BAR), cue(0, Some(2))),
    ];
    let r = run(&s, 128, 1);
    let starts: Vec<_> = r.started.iter().map(|s| (s.at, s.slot)).collect();
    assert_eq!(starts, vec![(BAR, Some(1)), (2 * BAR, Some(2))]);
    assert_eq!((r.late, r.stale, r.lost), (0, 0, 1));
    // Silent from the period after the switch until the view shows it,
    // then B again, then C.
    let quiet = (at(BAR) + 128)..(at(BAR) + 2 * 128);
    assert!(r
        .ons_from(0)
        .iter()
        .all(|&(f, t, _)| t != 0 || !quiet.contains(&f)));
    assert!(r.ons_from(at(BAR) + 960 * TICK).iter().any(|e| e.2 == 20));
    // In step the control thread hears first: C simply follows B.
    let step = run(&s, 128, 0);
    assert_eq!(step.lost, 0);
}

#[test]
fn each_cue_is_late_or_not_by_when_it_was_decided_not_by_its_view() {
    // B is cued on track 0 two periods before bar 2, and D's stop on track
    // 1 two periods after it. Both views are held back until four periods
    // after the line, so only the second comes: it carries B's cue as
    // decided before the line (late, in phase) and the stop as decided
    // after it (on bar 3's line, on time).
    let mut s = session(at(4 * BAR));
    s.decisions = vec![
        (at(BAR) - 2 * 128, cue(0, Some(1))),
        (at(BAR) + 2 * 128, cue(1, None)),
    ];
    let k = (at(BAR) + 4 * 128) / 128;
    let r = run_with(&s, 128, |j| j.max(k));
    let starts: Vec<_> = r.started.iter().map(|s| (s.track, s.at, s.from)).collect();
    assert_eq!(
        starts,
        vec![(0, tick_of(k * 128), BAR), (1, 2 * BAR, 2 * BAR)]
    );
    assert_eq!((r.late, r.stale, r.lost), (1, 0, 0));
}

#[test]
fn the_host_gives_each_new_decision_a_revision_and_the_frame_it_heard() {
    use mpfree_play::SceneProgram;
    use mpfree_seq::ClipProgram;
    let s = session(at(BAR));
    let b = Cue {
        track: 0,
        slot: Some(1),
    };
    let c = Cue {
        track: 0,
        slot: Some(2),
    };
    let (mut d, mut inc) = (Decisions::default(), Includes::default());
    let s = &s;
    let mut stamp = |d: &mut Decisions, cues: &[Cue], heard: u64, edit: &dyn Fn(&mut PlayView)| {
        let mut v = PlayView::compile_cued(&s.project, &s.launches, cues).unwrap();
        edit(&mut v);
        d.stamp(&mut v, heard, &mut inc);
        assert_eq!(v.includes(), Some(inc), "the view takes them");
        (inc.cues[0], inc.decided[0], inc.cues[1], inc.mutes_decided)
    };
    let none = |_: &mut PlayView| {};
    // A cue: revision 1, decided at 100. Compiled again, it is the same.
    assert_eq!(stamp(&mut d, &[b], 100, &none), (1, 100, 0, 0));
    assert_eq!(stamp(&mut d, &[b], 200, &none), (1, 100, 0, 0));
    // Another clip, or the same on another line, is a new decision.
    assert_eq!(stamp(&mut d, &[c], 300, &none), (2, 300, 0, 0));
    let line = |v: &mut PlayView| assert!(v.set_cue_line(0, LaunchGrid::Bars(2)));
    assert_eq!(stamp(&mut d, &[c], 400, &line), (3, 400, 0, 0));
    // The player carried it out, and the host cued the same clip again
    // before compiling: a new decision too.
    let st = Started {
        track: 0,
        slot: Some(2),
        at: BAR,
        origin: BAR,
        mute: None,
        from: BAR,
        rev: 3,
    };
    d.started(&st);
    assert_eq!(stamp(&mut d, &[c], 500, &line), (4, 500, 0, 0));
    // One older than the cue the view has, carried out late, is not.
    d.started(&Started { rev: 2, ..st });
    assert_eq!(stamp(&mut d, &[c], 600, &line), (4, 500, 0, 0));
    // No cue: nothing new. A scene's program change alone is a decision.
    assert_eq!(stamp(&mut d, &[], 700, &none), (4, 500, 0, 0));
    let program = |v: &mut PlayView| {
        let p = SceneProgram {
            program: ClipProgram {
                program: 5,
                bank: None,
            },
            line: Some(LaunchGrid::Bars(1)),
            id: 1,
        };
        assert!(v.set_scene_program(1, Some(p)));
    };
    assert_eq!(stamp(&mut d, &[], 800, &program), (4, 500, 5, 0));
    // The mute queue: the frame it began, until a new queue.
    let queue = |id: u32| {
        move |v: &mut PlayView| {
            for t in 0..2 {
                let m = MuteCue {
                    silent: true,
                    line: LaunchGrid::Bars(1),
                    id,
                };
                assert!(v.set_mute_cue(t, m));
            }
        }
    };
    assert_eq!(stamp(&mut d, &[], 900, &queue(7)), (4, 500, 5, 900));
    assert_eq!(stamp(&mut d, &[], 1000, &queue(7)).3, 900);
    assert_eq!(stamp(&mut d, &[], 1100, &queue(8)).3, 1100);
}
